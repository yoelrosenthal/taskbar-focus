//! The settings window.
//!
//! Plain Win32 controls created in code - no resource script, no dialog
//! template - so the whole layout is readable in one place and needs no build
//! tooling. It is modeless: the timer keeps ticking while it is open, and the
//! status line at the top updates live.
//!
//! Layout is built by a small cursor abstraction ([`Col`]) that stacks controls
//! down a column and wraps them in group boxes, so adding a setting is one line
//! rather than a pile of pixel arithmetic.

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateFontIndirectW, DeleteObject, COLOR_WINDOW, HFONT, LOGFONTW,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{
    BST_CHECKED, TOOLTIPS_CLASSW, TTF_IDISHWND, TTF_SUBCLASS, TTM_ADDTOOLW, TTM_SETMAXTIPWIDTH,
    TTS_ALWAYSTIP, TTTOOLINFOW, WC_BUTTONW, WC_COMBOBOXW, WC_EDITW, WC_STATICW,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::config::{Config, Preset, WakePolicy};
use crate::ui::app::App;
use crate::ui::wide;

const CLASS: &str = "TaskbarFocusSettings";

const ID_STATUS: usize = 90;
const ID_PRESET: usize = 100;
const ID_NAME: usize = 101;
const ID_FOCUS: usize = 102;
const ID_SHORT: usize = 103;
const ID_LONG: usize = 104;
const ID_SESSIONS: usize = 105;

const ID_SEQUENCE: usize = 110;
const ID_AUTO_BREAK: usize = 111;
const ID_AUTO_FOCUS: usize = 112;
const ID_STRICT: usize = 113;
const ID_RESTORE_SESSION: usize = 114;
const ID_WAKE: usize = 115;

const ID_DND: usize = 120;
const ID_MINI: usize = 131;
const ID_TOPMOST: usize = 132;

const ID_NOTIFY: usize = 140;
const ID_N_FS: usize = 141;
const ID_N_FE: usize = 142;
const ID_N_BS: usize = 143;
const ID_N_BE: usize = 144;

const ID_MUTE: usize = 150;
const ID_S_FS: usize = 151;
const ID_S_FE: usize = 152;
const ID_S_BS: usize = 153;
const ID_S_BE: usize = 154;

const ID_HOTKEYS: usize = 160;
const ID_HK_TOGGLE: usize = 161;
const ID_HK_SKIP: usize = 162;

const ID_SAVE: usize = 1;
const ID_CANCEL: usize = 2;
const ID_SAVE_AS: usize = 3;
const ID_DEFAULTS: usize = 4;

struct State {
    app: *mut App,
    font: HFONT,
    bold: HFONT,
    tooltip: HWND,
    /// Presets as currently edited, so switching the combo keeps changes.
    presets: Vec<Preset>,
    current: usize,
}

/// Open the settings window. Returns its handle, or `None` if creation failed.
pub fn open(owner: HWND, app: *mut App) -> Option<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        let class = wide(CLASS);
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: class.as_pcwstr(),
            hCursor: LoadCursorW(None, IDC_ARROW).ok()?,
            hIcon: crate::ui::app::shared_icon(),
            hbrBackground: windows::Win32::Graphics::Gdi::HBRUSH(
                (COLOR_WINDOW.0 + 1) as isize as *mut _,
            ),
            ..Default::default()
        };
        RegisterClassW(&wc);

        let title = wide("taskbar-focus settings");
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class.as_pcwstr(),
            title.as_pcwstr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            760,
            640,
            Some(owner),
            None,
            Some(instance.into()),
            None,
        )
        .ok()?;

        let config = (*app).config().clone();
        let presets = config.presets.clone();
        let current = presets
            .iter()
            .position(|p| p.name == config.preset().name)
            .unwrap_or(0);

        let (font, bold) = gui_fonts();
        let state = Box::new(State {
            app,
            font,
            bold,
            tooltip: make_tooltip(hwnd),
            presets,
            current,
        });
        let ptr = Box::into_raw(state);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, ptr as isize);

        build(hwnd, &*ptr, &config);
        let _ = SetForegroundWindow(hwnd);
        Some(hwnd)
    }
}

/// The standard UI font, plus a bold variant for group captions.
fn gui_fonts() -> (HFONT, HFONT) {
    unsafe {
        let mut ncm = NONCLIENTMETRICSW {
            cbSize: std::mem::size_of::<NONCLIENTMETRICSW>() as u32,
            ..Default::default()
        };
        let ok = SystemParametersInfoW(
            SPI_GETNONCLIENTMETRICS,
            ncm.cbSize,
            Some(&mut ncm as *mut _ as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_ok();
        let lf: LOGFONTW = if ok {
            ncm.lfMessageFont
        } else {
            LOGFONTW::default()
        };
        let mut bold_lf = lf;
        bold_lf.lfWeight = 700;
        (CreateFontIndirectW(&lf), CreateFontIndirectW(&bold_lf))
    }
}

unsafe fn make_tooltip(parent: HWND) -> HWND {
    let h = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        TOOLTIPS_CLASSW,
        PCWSTR::null(),
        WINDOW_STYLE(TTS_ALWAYSTIP),
        0,
        0,
        0,
        0,
        Some(parent),
        None,
        None,
        None,
    )
    .unwrap_or_default();
    if !h.is_invalid() {
        SendMessageW(h, TTM_SETMAXTIPWIDTH, None, Some(LPARAM(320)));
    }
    h
}

std::thread_local! {
    /// A tooltip control stores a *pointer* to its text rather than copying it,
    /// so these buffers have to outlive the `TTM_ADDTOOLW` call. They are owned
    /// here for the life of the settings window and cleared when it closes.
    static TIP_TEXT: std::cell::RefCell<Vec<Vec<u16>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Attach hover help to a control. This is where most of the "easy to
/// understand" comes from - the labels stay short, the explanation is one
/// hover away.
unsafe fn tip(st: &State, control: HWND, text: &str) {
    if st.tooltip.is_invalid() || control.is_invalid() {
        return;
    }
    let ptr = TIP_TEXT.with(|c| {
        let mut v = c.borrow_mut();
        v.push(text.encode_utf16().chain(std::iter::once(0)).collect());

        v.last_mut().unwrap().as_mut_ptr()
    });
    let ti = TTTOOLINFOW {
        cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
        uFlags: TTF_IDISHWND | TTF_SUBCLASS,
        hwnd: GetParent(control).unwrap_or_default(),
        uId: control.0 as usize,
        lpszText: windows::core::PWSTR(ptr),
        ..Default::default()
    };
    SendMessageW(
        st.tooltip,
        TTM_ADDTOOLW,
        None,
        Some(LPARAM(&ti as *const _ as isize)),
    );
}

const PAD: i32 = 16;
const COL_W: i32 = 348;
const ROW: i32 = 26;
const H: i32 = 22;
/// Inset of controls inside a group box.
const GX: i32 = 12;

/// A vertical cursor that stacks controls and wraps them in group boxes.
struct Col {
    x: i32,
    y: i32,
    /// Group box being filled: (hwnd, top y).
    group: Option<(HWND, i32)>,
}

impl Col {
    fn new(x: i32, y: i32) -> Self {
        Col { x, y, group: None }
    }

    fn inner_x(&self) -> i32 {
        if self.group.is_some() {
            self.x + GX
        } else {
            self.x
        }
    }

    fn inner_w(&self) -> i32 {
        COL_W - if self.group.is_some() { GX * 2 } else { 0 }
    }

    fn take(&mut self, dy: i32) -> i32 {
        let y = self.y;
        self.y += dy;
        y
    }

    /// Start a group box. Created now (so it sits behind its contents in
    /// z-order) at a placeholder height, resized in `end`.
    unsafe fn begin(&mut self, parent: HWND, st: &State, title: &str) {
        let top = self.y;
        let gb = ctl(
            parent,
            WC_BUTTONW,
            title,
            WINDOW_STYLE(BS_GROUPBOX as u32),
            self.x,
            top,
            COL_W,
            40,
            0,
            st.bold,
        );
        self.group = Some((gb, top));
        self.y += 24;
    }

    unsafe fn end(&mut self) {
        if let Some((gb, top)) = self.group.take() {
            self.y += 10;
            let _ = SetWindowPos(
                gb,
                None,
                0,
                0,
                COL_W,
                self.y - top,
                SWP_NOMOVE | SWP_NOZORDER,
            );
            self.y += 12;
        }
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn ctl(
    parent: HWND,
    class: PCWSTR,
    text: &str,
    style: WINDOW_STYLE,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    id: usize,
    font: HFONT,
) -> HWND {
    let t = wide(text);
    let hwnd = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        class,
        t.as_pcwstr(),
        WS_CHILD | WS_VISIBLE | style,
        x,
        y,
        w,
        h,
        Some(parent),
        Some(HMENU(id as *mut _)),
        None,
        None,
    )
    .unwrap_or_default();
    SendMessageW(
        hwnd,
        WM_SETFONT,
        Some(WPARAM(font.0 as usize)),
        Some(LPARAM(1)),
    );
    hwnd
}

unsafe fn check(
    parent: HWND,
    c: &mut Col,
    st: &State,
    text: &str,
    id: usize,
    on: bool,
    help: &str,
) {
    let y = c.take(ROW);
    let h = ctl(
        parent,
        WC_BUTTONW,
        text,
        WINDOW_STYLE(BS_AUTOCHECKBOX as u32 | WS_TABSTOP.0),
        c.inner_x(),
        y,
        c.inner_w(),
        H,
        id,
        st.font,
    );
    SendMessageW(
        h,
        BM_SETCHECK,
        Some(WPARAM(if on { BST_CHECKED.0 as usize } else { 0 })),
        None,
    );
    tip(st, h, help);
}

unsafe fn field(
    parent: HWND,
    c: &mut Col,
    st: &State,
    label: &str,
    id: usize,
    value: &str,
    help: &str,
) {
    let y = c.take(28);
    ctl(
        parent,
        WC_STATICW,
        label,
        WINDOW_STYLE(0),
        c.inner_x(),
        y + 4,
        170,
        H,
        0,
        st.font,
    );
    let e = ctl(
        parent,
        WC_EDITW,
        value,
        WINDOW_STYLE(WS_BORDER.0 | WS_TABSTOP.0),
        c.inner_x() + 176,
        y,
        c.inner_w() - 176,
        H,
        id,
        st.font,
    );
    tip(st, e, help);
}

/// Small grey explanatory paragraph inside a group.
unsafe fn note(parent: HWND, c: &mut Col, st: &State, text: &str, lines: i32) {
    let y = c.take(15 * lines + 6);
    ctl(
        parent,
        WC_STATICW,
        text,
        WINDOW_STYLE(0),
        c.inner_x(),
        y,
        c.inner_w(),
        15 * lines,
        0,
        st.font,
    );
}

unsafe fn build(hwnd: HWND, st: &State, cfg: &Config) {
    let f = st.font;

    ctl(
        hwnd,
        WC_STATICW,
        "",
        WINDOW_STYLE(0),
        PAD,
        PAD - 4,
        COL_W * 2 + PAD,
        20,
        ID_STATUS,
        st.bold,
    );
    refresh_status(hwnd);

    let top = PAD + 24;
    let mut left = Col::new(PAD, top);

    left.begin(hwnd, st, " Timer lengths ");
    let y = left.take(30);
    let combo = ctl(
        hwnd,
        WC_COMBOBOXW,
        "",
        WINDOW_STYLE(CBS_DROPDOWNLIST as u32 | WS_TABSTOP.0 | WS_VSCROLL.0),
        left.inner_x(),
        y,
        left.inner_w(),
        220,
        ID_PRESET,
        f,
    );
    for p in &st.presets {
        let t = wide(&p.name);
        SendMessageW(
            combo,
            CB_ADDSTRING,
            None,
            Some(LPARAM(t.as_pcwstr().0 as isize)),
        );
    }
    SendMessageW(combo, CB_SETCURSEL, Some(WPARAM(st.current)), None);
    tip(
        st,
        combo,
        "Switch between saved presets. Edit the boxes below and press Save to \
         change this preset, or Save as new preset to keep both.",
    );

    let p = &st.presets[st.current.min(st.presets.len() - 1)];
    field(
        hwnd,
        &mut left,
        st,
        "Preset name",
        ID_NAME,
        &p.name,
        "The name shown in the tray menu and usable with --preset.",
    );
    field(
        hwnd,
        &mut left,
        st,
        "Focus (minutes)",
        ID_FOCUS,
        &num(p.focus_minutes),
        "How long one focus session lasts. Fractions are allowed, e.g. 0.5 for 30 seconds.",
    );
    field(
        hwnd,
        &mut left,
        st,
        "Short break (minutes)",
        ID_SHORT,
        &num(p.short_break_minutes),
        "The break taken after most focus sessions.",
    );
    field(
        hwnd,
        &mut left,
        st,
        "Long break (minutes)",
        ID_LONG,
        &num(p.long_break_minutes),
        "The longer break taken after every Nth focus session.",
    );
    field(
        hwnd,
        &mut left,
        st,
        "Sessions per long break",
        ID_SESSIONS,
        &p.sessions_before_long_break.to_string(),
        "How many focus sessions to complete before earning a long break. 4 is the classic Pomodoro cadence.",
    );
    left.end();

    left.begin(hwnd, st, " Automatic sequence ");
    check(
        hwnd,
        &mut left,
        st,
        "Run the Pomodoro sequence",
        ID_SEQUENCE,
        cfg.behavior.sequence_enabled,
        "Master switch. Turn this off to use the app as a plain manual timer.",
    );
    check(
        hwnd,
        &mut left,
        st,
        "Start a break when focus ends",
        ID_AUTO_BREAK,
        cfg.behavior.auto_start_break,
        "When a focus session completes, begin the break automatically.",
    );
    check(hwnd, &mut left, st, "Start focus when a break ends", ID_AUTO_FOCUS,
        cfg.behavior.auto_start_focus,
        "When a break completes, begin the next focus session automatically. Off by default, so breaks end with you in control.");
    check(hwnd, &mut left, st, "Strict focus (confirm before stopping)", ID_STRICT,
        cfg.behavior.strict_focus,
        "Ask for confirmation before stopping or skipping a running focus session, so a stray click cannot end it. Breaks are never guarded.");
    check(hwnd, &mut left, st, "Restore an interrupted session on restart", ID_RESTORE_SESSION,
        cfg.behavior.restore_session_on_restart,
        "If the app closes mid-session, bring the remaining time back next time it starts. It always returns paused, never running.");

    let y = left.take(30);
    ctl(
        hwnd,
        WC_STATICW,
        "After sleep:",
        WINDOW_STYLE(0),
        left.inner_x(),
        y + 4,
        90,
        H,
        0,
        f,
    );
    let wake = ctl(
        hwnd,
        WC_COMBOBOXW,
        "",
        WINDOW_STYLE(CBS_DROPDOWNLIST as u32 | WS_TABSTOP.0),
        left.inner_x() + 96,
        y,
        left.inner_w() - 96,
        200,
        ID_WAKE,
        f,
    );
    for s in [
        "Count the time asleep",
        "Ignore the time asleep",
        "Pause the timer",
    ] {
        let t = wide(s);
        SendMessageW(
            wake,
            CB_ADDSTRING,
            None,
            Some(LPARAM(t.as_pcwstr().0 as isize)),
        );
    }
    SendMessageW(
        wake,
        CB_SETCURSEL,
        Some(WPARAM(match cfg.behavior.wake_policy {
            WakePolicy::CountSleep => 0,
            WakePolicy::IgnoreSleep => 1,
            WakePolicy::Pause => 2,
        })),
        None,
    );
    tip(st, wake, "What to do with a running timer when the machine wakes up. \
        Counting the time asleep means a 25 minute session started before a two hour nap is simply over.");
    left.end();

    left.begin(hwnd, st, " Global hotkeys ");
    check(hwnd, &mut left, st, "Enable global hotkeys", ID_HOTKEYS, cfg.hotkeys.enabled,
        "Hotkeys work from any application. If another program already owns a combination, you will be told.");
    field(
        hwnd,
        &mut left,
        st,
        "Start / pause",
        ID_HK_TOGGLE,
        &cfg.hotkeys.toggle,
        "For example Ctrl+Alt+F. A modifier is required. Leave blank to disable this one.",
    );
    field(
        hwnd,
        &mut left,
        st,
        "Skip to next",
        ID_HK_SKIP,
        &cfg.hotkeys.skip,
        "For example Ctrl+Alt+B. A modifier is required. Leave blank to disable this one.",
    );
    left.end();

    let mut right = Col::new(PAD + COL_W + PAD, top);

    right.begin(hwnd, st, " Do Not Disturb ");
    check(
        hwnd,
        &mut right,
        st,
        "Mute notifications during focus",
        ID_DND,
        cfg.dnd.enabled,
        "Switch Windows Do Not Disturb on when focus starts and off when a break starts.",
    );
    note(
        hwnd,
        &mut right,
        st,
        "Your priority apps and contacts still get through - Windows keeps \
         applying your own priority list, which this app never touches.",
        3,
    );
    right.end();

    right.begin(hwnd, st, " Timer window ");
    check(hwnd, &mut right, st, "Show the compact timer window", ID_MINI,
        cfg.display.mini_window,
        "A small resizable window showing the countdown. Drag its edges to shrink it to roughly the height of a taskbar button, or make it large. Click it to start or pause.");
    check(hwnd, &mut right, st, "Keep it above other windows", ID_TOPMOST,
        cfg.display.always_on_top,
        "Keep the compact timer window on top, so it stays visible while you work in other applications.");
    note(
        hwnd,
        &mut right,
        st,
        "The tray icon always stays visible; these are extra readouts.",
        2,
    );
    right.end();

    right.begin(hwnd, st, " Notifications ");
    check(
        hwnd,
        &mut right,
        st,
        "Show notifications",
        ID_NOTIFY,
        cfg.notifications.enabled,
        "Master switch for all notifications.",
    );
    check(
        hwnd,
        &mut right,
        st,
        "Focus started",
        ID_N_FS,
        cfg.notifications.events.focus_start,
        "Notify when a focus session begins.",
    );
    check(
        hwnd,
        &mut right,
        st,
        "Focus ended",
        ID_N_FE,
        cfg.notifications.events.focus_end,
        "Notify when a focus session completes.",
    );
    check(
        hwnd,
        &mut right,
        st,
        "Break started",
        ID_N_BS,
        cfg.notifications.events.break_start,
        "Notify when a break begins.",
    );
    check(
        hwnd,
        &mut right,
        st,
        "Break ended",
        ID_N_BE,
        cfg.notifications.events.break_end,
        "Notify when a break completes.",
    );
    right.end();

    right.begin(hwnd, st, " Sounds ");
    check(
        hwnd,
        &mut right,
        st,
        "Mute all sounds",
        ID_MUTE,
        cfg.sounds.muted,
        "Global mute, independent of the individual switches below.",
    );
    check(
        hwnd,
        &mut right,
        st,
        "Focus started",
        ID_S_FS,
        cfg.sounds.events.focus_start,
        "Play a cue when a focus session begins.",
    );
    check(
        hwnd,
        &mut right,
        st,
        "Focus ended",
        ID_S_FE,
        cfg.sounds.events.focus_end,
        "Play a cue when a focus session completes.",
    );
    check(
        hwnd,
        &mut right,
        st,
        "Break started",
        ID_S_BS,
        cfg.sounds.events.break_start,
        "Play a cue when a break begins.",
    );
    check(
        hwnd,
        &mut right,
        st,
        "Break ended",
        ID_S_BE,
        cfg.sounds.events.break_end,
        "Play a cue when a break completes.",
    );
    right.end();

    let by = left.y.max(right.y) + 6;
    let right_edge = PAD + COL_W + PAD + COL_W;
    let b = |x: i32, w: i32, text: &str, id: usize, help: &str| {
        let h = ctl(
            hwnd,
            WC_BUTTONW,
            text,
            WINDOW_STYLE(WS_TABSTOP.0),
            x,
            by,
            w,
            30,
            id,
            f,
        );
        tip(st, h, help);
    };
    b(PAD, 120, "Save", ID_SAVE, "Apply these settings and close.");
    b(
        PAD + 128,
        168,
        "Save as new preset",
        ID_SAVE_AS,
        "Keep the current preset untouched and store these timer lengths under the name above.",
    );
    b(PAD + 304, 140, "Restore defaults", ID_DEFAULTS,
        "Reset every setting on this window to its original value. Nothing is saved until you press Save.");
    b(
        right_edge - 110,
        110,
        "Cancel",
        ID_CANCEL,
        "Close without saving.",
    );

    let (mut wr, mut cr) = (RECT::default(), RECT::default());
    let _ = GetWindowRect(hwnd, &mut wr);
    let _ = GetClientRect(hwnd, &mut cr);
    let chrome_w = (wr.right - wr.left) - cr.right;
    let chrome_h = (wr.bottom - wr.top) - cr.bottom;
    let _ = SetWindowPos(
        hwnd,
        None,
        0,
        0,
        right_edge + PAD + chrome_w,
        by + 30 + PAD + chrome_h,
        SWP_NOMOVE | SWP_NOZORDER,
    );
}

/// Update the live status line. Called from the app on every tick.
pub fn refresh_status(hwnd: HWND) {
    unsafe {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
        if ptr.is_null() {
            return;
        }
        let app = &*(*ptr).app;
        let dnd = app.orch.dnd_status();
        let text = format!("{}   \u{2022}   {}", app.orch.tooltip(), dnd);
        if let Ok(h) = GetDlgItem(Some(hwnd), ID_STATUS as i32) {
            let w = wide(&text);
            let _ = SetWindowTextW(h, w.as_pcwstr());
        }
    }
}

/// Format a duration without a pointless trailing `.0`.
fn num(v: f64) -> String {
    if (v.fract()).abs() < 1e-9 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

unsafe fn text_of(hwnd: HWND, id: usize) -> String {
    let h = GetDlgItem(Some(hwnd), id as i32).unwrap_or_default();
    if h.is_invalid() {
        return String::new();
    }
    let mut buf = [0u16; 256];
    let n = GetWindowTextW(h, &mut buf);
    String::from_utf16_lossy(&buf[..n as usize])
}

unsafe fn checked(hwnd: HWND, id: usize) -> bool {
    let h = GetDlgItem(Some(hwnd), id as i32).unwrap_or_default();
    !h.is_invalid() && SendMessageW(h, BM_GETCHECK, None, None).0 == BST_CHECKED.0 as isize
}

unsafe fn set_checked(hwnd: HWND, id: usize, on: bool) {
    if let Ok(h) = GetDlgItem(Some(hwnd), id as i32) {
        SendMessageW(
            h,
            BM_SETCHECK,
            Some(WPARAM(if on { BST_CHECKED.0 as usize } else { 0 })),
            None,
        );
    }
}

unsafe fn combo_index(hwnd: HWND, id: usize) -> usize {
    let h = GetDlgItem(Some(hwnd), id as i32).unwrap_or_default();
    if h.is_invalid() {
        return 0;
    }
    let i = SendMessageW(h, CB_GETCURSEL, None, None).0;
    if i < 0 {
        0
    } else {
        i as usize
    }
}

/// A duration field: `Ok(minutes)`, or `Err(())` if the text is unusable.
fn parse_minutes(s: &str) -> Result<f64, ()> {
    s.trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && *v > 0.0)
        .ok_or(())
}

/// Read the preset fields, reporting the first unusable one by name rather than
/// silently reverting it behind the user's back.
unsafe fn read_preset(hwnd: HWND, base: &Preset) -> Result<Preset, &'static str> {
    let name = text_of(hwnd, ID_NAME).trim().to_string();
    Ok(Preset {
        name: if name.is_empty() {
            base.name.clone()
        } else {
            name
        },
        focus_minutes: parse_minutes(&text_of(hwnd, ID_FOCUS)).map_err(|_| "Focus (minutes)")?,
        short_break_minutes: parse_minutes(&text_of(hwnd, ID_SHORT))
            .map_err(|_| "Short break (minutes)")?,
        long_break_minutes: parse_minutes(&text_of(hwnd, ID_LONG))
            .map_err(|_| "Long break (minutes)")?,
        sessions_before_long_break: text_of(hwnd, ID_SESSIONS)
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|n| *n > 0)
            .ok_or("Sessions per long break")?,
    })
}

unsafe fn collect(hwnd: HWND, st: &mut State, save_as_new: bool) -> Result<Config, &'static str> {
    let app = &mut *st.app;
    let mut cfg = app.config().clone();

    let idx = st.current.min(st.presets.len().saturating_sub(1));
    let edited = read_preset(hwnd, &st.presets[idx])?;
    if save_as_new && !st.presets.iter().any(|p| p.name == edited.name) {
        st.presets.push(edited.clone());
    } else {
        st.presets[idx] = edited.clone();
    }
    cfg.presets = st.presets.clone();
    cfg.active_preset = edited.name;

    cfg.behavior.sequence_enabled = checked(hwnd, ID_SEQUENCE);
    cfg.behavior.auto_start_break = checked(hwnd, ID_AUTO_BREAK);
    cfg.behavior.auto_start_focus = checked(hwnd, ID_AUTO_FOCUS);
    cfg.behavior.strict_focus = checked(hwnd, ID_STRICT);
    cfg.behavior.restore_session_on_restart = checked(hwnd, ID_RESTORE_SESSION);
    cfg.behavior.wake_policy = match combo_index(hwnd, ID_WAKE) {
        1 => WakePolicy::IgnoreSleep,
        2 => WakePolicy::Pause,
        _ => WakePolicy::CountSleep,
    };

    cfg.dnd.enabled = checked(hwnd, ID_DND);
    cfg.display.mini_window = checked(hwnd, ID_MINI);
    cfg.display.always_on_top = checked(hwnd, ID_TOPMOST);

    cfg.notifications.enabled = checked(hwnd, ID_NOTIFY);
    cfg.notifications.events.focus_start = checked(hwnd, ID_N_FS);
    cfg.notifications.events.focus_end = checked(hwnd, ID_N_FE);
    cfg.notifications.events.break_start = checked(hwnd, ID_N_BS);
    cfg.notifications.events.break_end = checked(hwnd, ID_N_BE);

    cfg.sounds.muted = checked(hwnd, ID_MUTE);
    cfg.sounds.events.focus_start = checked(hwnd, ID_S_FS);
    cfg.sounds.events.focus_end = checked(hwnd, ID_S_FE);
    cfg.sounds.events.break_start = checked(hwnd, ID_S_BS);
    cfg.sounds.events.break_end = checked(hwnd, ID_S_BE);

    cfg.hotkeys.enabled = checked(hwnd, ID_HOTKEYS);
    cfg.hotkeys.toggle = text_of(hwnd, ID_HK_TOGGLE).trim().to_string();
    cfg.hotkeys.skip = text_of(hwnd, ID_HK_SKIP).trim().to_string();

    Ok(cfg)
}

/// Load the selected preset's values into the edit fields.
unsafe fn show_preset(hwnd: HWND, st: &State) {
    let Some(p) = st.presets.get(st.current) else {
        return;
    };
    for (id, val) in [
        (ID_NAME, p.name.clone()),
        (ID_FOCUS, num(p.focus_minutes)),
        (ID_SHORT, num(p.short_break_minutes)),
        (ID_LONG, num(p.long_break_minutes)),
        (ID_SESSIONS, p.sessions_before_long_break.to_string()),
    ] {
        if let Ok(h) = GetDlgItem(Some(hwnd), id as i32) {
            let t = wide(&val);
            let _ = SetWindowTextW(h, t.as_pcwstr());
        }
    }
}

/// Put every control back to the built-in defaults, without saving.
unsafe fn restore_defaults(hwnd: HWND, st: &mut State) {
    let d = Config::default();
    st.presets = d.presets.clone();
    st.current = 0;
    if let Ok(h) = GetDlgItem(Some(hwnd), ID_PRESET as i32) {
        SendMessageW(h, CB_RESETCONTENT, None, None);
        for p in &st.presets {
            let t = wide(&p.name);
            SendMessageW(
                h,
                CB_ADDSTRING,
                None,
                Some(LPARAM(t.as_pcwstr().0 as isize)),
            );
        }
        SendMessageW(h, CB_SETCURSEL, Some(WPARAM(0)), None);
    }
    show_preset(hwnd, st);

    for (id, v) in [
        (ID_SEQUENCE, d.behavior.sequence_enabled),
        (ID_AUTO_BREAK, d.behavior.auto_start_break),
        (ID_AUTO_FOCUS, d.behavior.auto_start_focus),
        (ID_STRICT, d.behavior.strict_focus),
        (ID_RESTORE_SESSION, d.behavior.restore_session_on_restart),
        (ID_DND, d.dnd.enabled),
        (ID_MINI, d.display.mini_window),
        (ID_TOPMOST, d.display.always_on_top),
        (ID_NOTIFY, d.notifications.enabled),
        (ID_N_FS, d.notifications.events.focus_start),
        (ID_N_FE, d.notifications.events.focus_end),
        (ID_N_BS, d.notifications.events.break_start),
        (ID_N_BE, d.notifications.events.break_end),
        (ID_MUTE, d.sounds.muted),
        (ID_S_FS, d.sounds.events.focus_start),
        (ID_S_FE, d.sounds.events.focus_end),
        (ID_S_BS, d.sounds.events.break_start),
        (ID_S_BE, d.sounds.events.break_end),
        (ID_HOTKEYS, d.hotkeys.enabled),
    ] {
        set_checked(hwnd, id, v);
    }
    if let Ok(h) = GetDlgItem(Some(hwnd), ID_WAKE as i32) {
        SendMessageW(h, CB_SETCURSEL, Some(WPARAM(0)), None);
    }
    for (id, v) in [
        (ID_HK_TOGGLE, d.hotkeys.toggle),
        (ID_HK_SKIP, d.hotkeys.skip),
    ] {
        if let Ok(h) = GetDlgItem(Some(hwnd), id as i32) {
            let t = wide(&v);
            let _ = SetWindowTextW(h, t.as_pcwstr());
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }

    match msg {
        WM_COMMAND => {
            let id = wparam.0 & 0xFFFF;
            let code = ((wparam.0 >> 16) & 0xFFFF) as u32;
            let st = &mut *ptr;
            match (id, code) {
                (ID_PRESET, CBN_SELCHANGE) => {
                    let idx = st.current.min(st.presets.len().saturating_sub(1));
                    if let Ok(p) = read_preset(hwnd, &st.presets[idx]) {
                        st.presets[idx] = p;
                    }
                    st.current = combo_index(hwnd, ID_PRESET);
                    show_preset(hwnd, st);
                }
                (ID_DEFAULTS, _) => restore_defaults(hwnd, st),
                (ID_SAVE, _) | (ID_SAVE_AS, _) => match collect(hwnd, st, id == ID_SAVE_AS) {
                    Ok(cfg) => {
                        (*st.app).apply_config(cfg);
                        let _ = DestroyWindow(hwnd);
                    }
                    Err(field) => {
                        let msg = wide(&format!(
                            "\"{field}\" needs a number greater than zero.\n\n\
                             Nothing has been saved."
                        ));
                        let cap = wide("Check that value");
                        MessageBoxW(
                            Some(hwnd),
                            msg.as_pcwstr(),
                            cap.as_pcwstr(),
                            MB_OK | MB_ICONWARNING,
                        );
                    }
                },
                (ID_CANCEL, _) => {
                    let _ = DestroyWindow(hwnd);
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            let st = Box::from_raw(ptr);
            TIP_TEXT.with(|c| c.borrow_mut().clear());
            let _ = DeleteObject(st.font.into());
            let _ = DeleteObject(st.bold.into());
            (*st.app).on_settings_closed();
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
