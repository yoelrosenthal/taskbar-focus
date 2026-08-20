//! The settings window.
//!
//! Plain Win32 controls created in code - no resource script, no dialog
//! template - so the whole layout is readable in one place and needs no build
//! tooling. It is modeless: the timer keeps ticking while it is open, and the
//! status line at the bottom updates live.
//!
//! The window is a navigation list on the left and one page of settings on the
//! right. Every control is created once, hidden, and described by an [`Item`];
//! [`relayout`] then decides which items belong on screen - the selected page,
//! or everything matching the search box - and stacks them down the pane. That
//! is what lets the search box reach across pages without rebuilding anything.
//!
//! The pane paints the cards, and the icon at the left of each one, itself.
//! Windows has no control for either, and grouping the rows this way is what
//! keeps a long page readable.

use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontIndirectW, CreatePen, CreateSolidBrush, DeleteObject, DrawTextW,
    EndPaint, FillRect, GetDC, GetSysColor, GetSysColorBrush, GetTextFaceW, InvalidateRect,
    ReleaseDC, RoundRect, SelectObject, SetBkMode, SetTextColor, COLOR_GRAYTEXT, COLOR_HIGHLIGHT,
    COLOR_WINDOW, COLOR_WINDOWTEXT, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX,
    DT_SINGLELINE, DT_VCENTER, HBRUSH, HDC, HFONT, HPEN, LOGFONTW, PAINTSTRUCT, PS_SOLID,
    TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemServices::{SS_ENDELLIPSIS, SS_ETCHEDHORZ};
use windows::Win32::UI::Controls::{
    SetScrollInfo, BST_CHECKED, DRAWITEMSTRUCT, EM_SETCUEBANNER, ODS_SELECTED, TOOLTIPS_CLASSW,
    TTF_IDISHWND, TTF_SUBCLASS, TTM_ADDTOOLW, TTM_SETMAXTIPWIDTH, TTS_ALWAYSTIP, TTTOOLINFOW,
    WC_BUTTONW, WC_COMBOBOXW, WC_EDITW, WC_LISTBOXW, WC_STATICW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, GetFocus, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::config::{Config, LengthPreset, Preset, WakePolicy};
use crate::ui::app::App;
use crate::ui::wide;

const CLASS: &str = "TaskbarFocusSettings";
const PANE_CLASS: &str = "TaskbarFocusSettingsPane";

const ID_SAVE: usize = 1;
const ID_CANCEL: usize = 2;
const ID_SAVE_AS: usize = 3;
const ID_DEFAULTS: usize = 4;

const ID_SEARCH: usize = 10;
const ID_NAV: usize = 11;
const ID_TITLE: usize = 12;
const ID_EMPTY: usize = 13;
const ID_STATUS: usize = 14;

const ID_PRESET: usize = 100;
const ID_NAME: usize = 101;
const ID_FOCUS: usize = 102;
const ID_SHORT: usize = 103;
const ID_LONG: usize = 104;
const ID_SESSIONS: usize = 105;

const ID_SEQUENCE: usize = 110;
const ID_AUTO_BREAK: usize = 111;
const ID_AUTO_FOCUS: usize = 112;
const ID_REPEAT_CYCLES: usize = 116;
const ID_STRICT: usize = 113;
const ID_RESTORE_SESSION: usize = 114;
const ID_WAKE: usize = 115;

const ID_DND: usize = 120;
const ID_DND_SHORT_BREAK: usize = 121;
const ID_MUTE_TRAY: usize = 122;
const ID_MUTE_WINDOW: usize = 123;
const ID_MINI: usize = 131;
const ID_TOPMOST: usize = 132;

const ID_NOTIFY: usize = 140;
const ID_N_FS: usize = 141;
const ID_N_FE: usize = 142;
const ID_N_BS: usize = 143;
const ID_N_BE: usize = 144;
const ID_ALERT_TOAST: usize = 145;
const ID_REQUIRE_DISMISS: usize = 146;
const ID_ALERT_OVERLAY: usize = 147;
const ID_ALERT_FLASH: usize = 148;
const ID_FLASH_LENGTH: usize = 149;

const ID_MUTE: usize = 150;
const ID_S_FS: usize = 151;
const ID_S_FE: usize = 152;
const ID_S_BS: usize = 153;
const ID_S_BE: usize = 154;

const ID_HOTKEYS: usize = 160;
const ID_HK_TOGGLE: usize = 161;
const ID_HK_SKIP: usize = 162;

/// The pages, in the order they appear in the navigation list.
const PAGES: [&str; 7] = [
    "Timer",
    "Sequence",
    "Notifications",
    "Do Not Disturb",
    "Sounds",
    "Timer window",
    "Hotkeys",
];

const PAGE_TIMER: usize = 0;
const PAGE_SEQUENCE: usize = 1;
const PAGE_NOTIFICATIONS: usize = 2;
const PAGE_DND: usize = 3;
const PAGE_SOUNDS: usize = 4;
const PAGE_WINDOW: usize = 5;
const PAGE_HOTKEYS: usize = 6;

/// The surfaces the window paints for itself.
///
/// The rest of the window already assumes the classic light system colours -
/// its background is `COLOR_WINDOW` - so these are fixed values chosen to sit
/// against them rather than system colours, which carry no notion of a card.
/// `COLORREF` is `0x00BBGGRR`.
const SIDEBAR_BG: u32 = 0x00F3F3F3;
const PAGE_BG: u32 = 0x00F7F7F7;
const CARD_BG: u32 = 0x00FFFFFF;
const CARD_EDGE: u32 = 0x00E3E3E3;
const NAV_SELECTED: u32 = 0x00E8E8E8;

const CLIENT_W: i32 = 660;
const CLIENT_H: i32 = 496;
const PAD: i32 = 12;
const SIDEBAR_W: i32 = 168;
const NAV_ROW: i32 = 28;
const CONTENT_X: i32 = SIDEBAR_W + 16;
const PANE_Y: i32 = 48;
const FOOTER_H: i32 = 78;
const FOOTER_Y: i32 = CLIENT_H - FOOTER_H;
const PANE_W: i32 = CLIENT_W - CONTENT_X - PAD;
const PANE_H: i32 = FOOTER_Y - PANE_Y - PAD;
/// Usable width inside the pane, leaving room for the scroll bar.
const ITEM_W: i32 = PANE_W - 26;
/// Width of the caption to the left of an edit box or a drop-down.
const CAP_W: i32 = 150;
/// Inset of a control from the edge of its card.
const CARD_X: i32 = 12;
/// Space above and below the contents of a card.
const CARD_Y: i32 = 11;
/// The column an icon occupies at the left of a card, gap included.
const ICON_W: i32 = 30;
const H: i32 = 22;

/// The icon for each page, in the same order as [`PAGES`]: stopwatch, repeat,
/// bell, moon, speaker, monitor, keyboard.
const PAGE_ICONS: [&str; 7] = [
    "\u{E916}", "\u{E8EE}", "\u{EA8F}", "\u{E708}", "\u{E767}", "\u{E7F4}", "\u{E765}",
];

/// What a row in the settings pane looks like, which decides how it is laid
/// out and whether the search box can match it.
#[derive(PartialEq)]
enum Kind {
    /// A bold caption introducing the cards below it. Not itself on a card.
    Header,
    /// A checkbox filling the width of its card.
    Check,
    /// A caption on the left of its card and an edit box or drop-down on the
    /// right.
    Field,
    /// A grey paragraph explaining a whole group. Not on a card.
    Note,
}

/// How a static control wants to be painted: which surface it sits on, and
/// whether its text is secondary.
mod paint_as {
    /// Normal text on a card.
    pub const CARD: isize = 0;
    /// Secondary text on a card.
    pub const CARD_GREY: isize = 1;
    /// A section caption, on the page background.
    pub const PAGE: isize = 2;
    /// Secondary text on the page background.
    pub const PAGE_GREY: isize = 3;
    /// Secondary text on the window background, outside the pane.
    pub const WINDOW_GREY: isize = 4;
}

/// One row of the settings pane.
struct Item {
    page: usize,
    /// The header this row belongs to; a header points at itself. A header is
    /// shown during a search when any of its rows matched.
    section: usize,
    kind: Kind,
    hwnd: HWND,
    /// The caption of a [`Kind::Field`], otherwise invalid.
    caption: HWND,
    /// The grey one-liner under the control, otherwise invalid.
    desc: HWND,
    /// The glyph drawn in the card's icon column. Empty for rows with no card.
    icon: &'static str,
    /// Extra left inset, used to nest options under the switch that governs
    /// them.
    indent: i32,
    /// Height the control itself is created and laid out with. A drop-down is
    /// told the height of its *open* list; Windows shrinks the closed control
    /// to one row itself.
    ctl_h: i32,
    /// Height of a [`Kind::Note`] in text lines.
    lines: i32,
    /// Everything a search matches against, already lowercased.
    search: String,
}

/// A card to paint behind a row, in content coordinates, with the glyph that
/// belongs in its icon column.
struct Card {
    rect: RECT,
    icon: &'static str,
}

struct State {
    app: *mut App,
    font: HFONT,
    bold: HFONT,
    title: HFONT,
    /// Segoe UI Emoji, for the glyph at the left of every card and page.
    icons: HFONT,
    tooltip: HWND,
    /// The scrolling container holding every setting.
    pane: HWND,
    sidebar: HBRUSH,
    page_bg: HBRUSH,
    card: HBRUSH,
    card_pen: HPEN,
    nav_selected: HBRUSH,
    nav_selected_pen: HPEN,
    accent: HBRUSH,
    accent_pen: HPEN,
    items: Vec<Item>,
    /// The card behind each visible row, in content coordinates. Rows that are
    /// hidden, or that have no card, are not in here.
    cards: Vec<Card>,
    page: usize,
    scroll: i32,
    /// Height of the currently laid out page, for the scroll range.
    content_h: i32,
    /// Presets as currently edited, so switching the combo keeps changes.
    presets: Vec<Preset>,
    current: usize,
}

/// Open the settings window. Returns its handle, or `None` if creation failed.
pub fn open(owner: HWND, app: *mut App) -> Option<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;

        let class = wide(CLASS);
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: class.as_pcwstr(),
            hCursor: LoadCursorW(None, IDC_ARROW).ok()?,
            hIcon: crate::ui::app::shared_icon(),
            hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as isize as *mut _),
            ..Default::default()
        });

        let pane_class = wide(PANE_CLASS);
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(pane_wndproc),
            hInstance: instance.into(),
            lpszClassName: pane_class.as_pcwstr(),
            hCursor: LoadCursorW(None, IDC_ARROW).ok()?,
            ..Default::default()
        });

        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN;
        let mut rc = RECT {
            left: 0,
            top: 0,
            right: CLIENT_W,
            bottom: CLIENT_H,
        };
        let _ = AdjustWindowRect(&mut rc, style, false);

        let title = wide("Taskbar-Focus Settings");
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class.as_pcwstr(),
            title.as_pcwstr(),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            rc.right - rc.left,
            rc.bottom - rc.top,
            Some(owner),
            None,
            Some(instance.into()),
            None,
        )
        .ok()?;

        let pane = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            pane_class.as_pcwstr(),
            PCWSTR::null(),
            WS_CHILD | WS_VISIBLE | WS_VSCROLL | WS_CLIPCHILDREN,
            CONTENT_X,
            PANE_Y,
            PANE_W,
            PANE_H,
            Some(hwnd),
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

        let (font, bold, title_font, icon_font) = gui_fonts();
        let accent = COLORREF(GetSysColor(COLOR_HIGHLIGHT));
        let ptr = Box::into_raw(Box::new(State {
            app,
            font,
            bold,
            title: title_font,
            icons: icon_font,
            tooltip: make_tooltip(hwnd),
            pane,
            sidebar: CreateSolidBrush(COLORREF(SIDEBAR_BG)),
            page_bg: CreateSolidBrush(COLORREF(PAGE_BG)),
            card: CreateSolidBrush(COLORREF(CARD_BG)),
            card_pen: CreatePen(PS_SOLID, 1, COLORREF(CARD_EDGE)),
            nav_selected: CreateSolidBrush(COLORREF(NAV_SELECTED)),
            nav_selected_pen: CreatePen(PS_SOLID, 1, COLORREF(NAV_SELECTED)),
            accent: CreateSolidBrush(accent),
            accent_pen: CreatePen(PS_SOLID, 1, accent),
            items: Vec::new(),
            cards: Vec::new(),
            page: 0,
            scroll: 0,
            content_h: 0,
            presets,
            current,
        }));
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, ptr as isize);
        SetWindowLongPtrW(pane, GWLP_USERDATA, ptr as isize);

        build(hwnd, &mut *ptr, &config);
        relayout(hwnd, &mut *ptr);
        sync_dependencies((*ptr).pane);
        refresh_status(hwnd);

        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        Some(hwnd)
    }
}

/// The standard UI font, a bold variant for section captions, a larger one for
/// the page title, and the system icon font.
fn gui_fonts() -> (HFONT, HFONT, HFONT, HFONT) {
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
        bold_lf.lfWeight = 600;
        let mut title_lf = lf;
        title_lf.lfWeight = 600;
        title_lf.lfHeight = title_lf.lfHeight * 3 / 2;
        let mut icon_lf = lf;
        icon_lf.lfWeight = 400;
        icon_lf.lfHeight = icon_lf.lfHeight * 7 / 5;
        (
            CreateFontIndirectW(&lf),
            CreateFontIndirectW(&bold_lf),
            CreateFontIndirectW(&title_lf),
            icon_font(&icon_lf),
        )
    }
}

/// The system icon font, which is where the glyphs in this window come from.
///
/// Windows 11 ships Segoe Fluent Icons and Windows 10 shipped Segoe MDL2
/// Assets; the two use the same code points for the glyphs used here. GDI
/// substitutes silently when a face is missing, drawing empty boxes, so each
/// candidate is asked for by name and then checked against what was actually
/// created.
unsafe fn icon_font(base: &LOGFONTW) -> HFONT {
    for face in ["Segoe Fluent Icons", "Segoe MDL2 Assets", "Segoe UI Symbol"] {
        let mut lf = *base;
        set_face(&mut lf, face);
        let font = CreateFontIndirectW(&lf);
        if font_is(font, face) {
            return font;
        }
        let _ = DeleteObject(font.into());
    }
    CreateFontIndirectW(base)
}

/// Whether `font` really is the typeface it asked for.
unsafe fn font_is(font: HFONT, face: &str) -> bool {
    let hdc = GetDC(None);
    if hdc.is_invalid() {
        return false;
    }
    let old = SelectObject(hdc, font.into());
    let mut name = [0u16; 32];
    let n = GetTextFaceW(hdc, Some(&mut name));
    SelectObject(hdc, old);
    ReleaseDC(None, hdc);
    n > 1 && String::from_utf16_lossy(&name[..n as usize - 1]) == face
}

/// Name the typeface a font description asks for.
fn set_face(lf: &mut LOGFONTW, name: &str) {
    lf.lfFaceName = [0; 32];
    for (slot, ch) in lf.lfFaceName.iter_mut().zip(name.encode_utf16()) {
        *slot = ch;
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

/// Attach hover help to a control. The visible description stays one line; the
/// full explanation is one hover away.
unsafe fn tip(tooltip: HWND, control: HWND, text: &str) {
    if tooltip.is_invalid() || control.is_invalid() || text.is_empty() {
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
        tooltip,
        TTM_ADDTOOLW,
        None,
        Some(LPARAM(&ti as *const _ as isize)),
    );
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
        WS_CHILD | style,
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

/// Record how a static control wants to be painted; see [`paint_as`].
unsafe fn painted_as(h: HWND, how: isize) {
    if !h.is_invalid() {
        SetWindowLongPtrW(h, GWLP_USERDATA, how);
    }
}

/// Collects the settings rows as they are created.
struct Rows {
    pane: HWND,
    font: HFONT,
    bold: HFONT,
    tooltip: HWND,
    page: usize,
    section: usize,
    items: Vec<Item>,
}

impl Rows {
    unsafe fn header(&mut self, page: usize, title: &str) {
        self.page = page;
        self.section = self.items.len();
        let h = ctl(
            self.pane,
            WC_STATICW,
            title,
            WINDOW_STYLE(0),
            0,
            0,
            ITEM_W,
            20,
            0,
            self.bold,
        );
        painted_as(h, paint_as::PAGE);
        self.push(
            Kind::Header,
            h,
            HWND::default(),
            HWND::default(),
            "",
            0,
            20,
            0,
            title,
            "",
            "",
        );
    }

    unsafe fn check(
        &mut self,
        icon: &'static str,
        id: usize,
        label: &str,
        on: bool,
        desc: &str,
        help: &str,
    ) {
        self.check_at(icon, 0, id, label, on, desc, help);
    }

    /// A checkbox nested under the switch above it.
    unsafe fn sub_check(
        &mut self,
        icon: &'static str,
        id: usize,
        label: &str,
        on: bool,
        help: &str,
    ) {
        self.check_at(icon, 22, id, label, on, "", help);
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn check_at(
        &mut self,
        icon: &'static str,
        indent: i32,
        id: usize,
        label: &str,
        on: bool,
        desc: &str,
        help: &str,
    ) {
        let h = ctl(
            self.pane,
            WC_BUTTONW,
            label,
            WINDOW_STYLE(BS_AUTOCHECKBOX as u32 | WS_TABSTOP.0),
            0,
            0,
            ITEM_W,
            20,
            id,
            self.font,
        );
        SendMessageW(
            h,
            BM_SETCHECK,
            Some(WPARAM(if on { BST_CHECKED.0 as usize } else { 0 })),
            None,
        );
        tip(self.tooltip, h, help);
        let d = self.description(desc);
        self.push(
            Kind::Check,
            h,
            HWND::default(),
            d,
            icon,
            indent,
            20,
            0,
            label,
            desc,
            help,
        );
    }

    unsafe fn field(
        &mut self,
        icon: &'static str,
        id: usize,
        label: &str,
        value: &str,
        desc: &str,
        help: &str,
    ) {
        let e = ctl(
            self.pane,
            WC_EDITW,
            value,
            WINDOW_STYLE(WS_BORDER.0 | WS_TABSTOP.0 | ES_AUTOHSCROLL as u32),
            0,
            0,
            100,
            H,
            id,
            self.font,
        );
        tip(self.tooltip, e, help);
        let c = self.caption(label);
        let d = self.description(desc);
        self.push(Kind::Field, e, c, d, icon, 0, H, 0, label, desc, help);
    }

    /// A drop-down. `options` are added in order and `selected` is preselected.
    #[allow(clippy::too_many_arguments)]
    unsafe fn combo(
        &mut self,
        icon: &'static str,
        id: usize,
        label: &str,
        options: &[&str],
        selected: usize,
        desc: &str,
        help: &str,
    ) {
        let h = ctl(
            self.pane,
            WC_COMBOBOXW,
            "",
            WINDOW_STYLE(CBS_DROPDOWNLIST as u32 | WS_TABSTOP.0 | WS_VSCROLL.0),
            0,
            0,
            100,
            200,
            id,
            self.font,
        );
        for o in options {
            let t = wide(o);
            SendMessageW(
                h,
                CB_ADDSTRING,
                None,
                Some(LPARAM(t.as_pcwstr().0 as isize)),
            );
        }
        SendMessageW(h, CB_SETCURSEL, Some(WPARAM(selected)), None);
        tip(self.tooltip, h, help);
        let c = self.caption(label);
        let d = self.description(desc);
        self.push(Kind::Field, h, c, d, icon, 0, 200, 0, label, desc, help);
    }

    unsafe fn note(&mut self, text: &str, lines: i32) {
        let h = ctl(
            self.pane,
            WC_STATICW,
            text,
            WINDOW_STYLE(0),
            0,
            0,
            ITEM_W,
            15 * lines,
            0,
            self.font,
        );
        painted_as(h, paint_as::PAGE_GREY);
        self.push(
            Kind::Note,
            h,
            HWND::default(),
            HWND::default(),
            "",
            0,
            15 * lines,
            lines,
            text,
            "",
            "",
        );
    }

    unsafe fn caption(&self, label: &str) -> HWND {
        let h = ctl(
            self.pane,
            WC_STATICW,
            label,
            WINDOW_STYLE(0),
            0,
            0,
            CAP_W,
            H,
            0,
            self.font,
        );
        painted_as(h, paint_as::CARD);
        h
    }

    unsafe fn description(&self, text: &str) -> HWND {
        if text.is_empty() {
            return HWND::default();
        }
        let h = ctl(
            self.pane,
            WC_STATICW,
            text,
            WINDOW_STYLE(0),
            0,
            0,
            ITEM_W,
            16,
            0,
            self.font,
        );
        painted_as(h, paint_as::CARD_GREY);
        h
    }

    #[allow(clippy::too_many_arguments)]
    fn push(
        &mut self,
        kind: Kind,
        hwnd: HWND,
        caption: HWND,
        desc: HWND,
        icon: &'static str,
        indent: i32,
        ctl_h: i32,
        lines: i32,
        label: &str,
        description: &str,
        help: &str,
    ) {
        let section = if kind == Kind::Header {
            self.items.len()
        } else {
            self.section
        };
        let heading = self
            .items
            .get(section)
            .map(|h| h.search.clone())
            .unwrap_or_default();
        self.items.push(Item {
            page: self.page,
            section,
            kind,
            hwnd,
            caption,
            desc,
            icon,
            indent,
            ctl_h,
            lines,
            search: format!(
                "{} {} {} {} {}",
                PAGES[self.page], heading, label, description, help
            )
            .to_lowercase(),
        });
    }
}

unsafe fn build(hwnd: HWND, st: &mut State, cfg: &Config) {
    let f = st.font;

    let search = ctl(
        hwnd,
        WC_EDITW,
        "",
        WINDOW_STYLE(WS_VISIBLE.0 | WS_BORDER.0 | WS_TABSTOP.0 | ES_AUTOHSCROLL as u32),
        PAD,
        PAD + 2,
        SIDEBAR_W - PAD * 2,
        24,
        ID_SEARCH,
        f,
    );
    let cue = wide("Search settings");
    SendMessageW(
        search,
        EM_SETCUEBANNER,
        Some(WPARAM(1)),
        Some(LPARAM(cue.as_pcwstr().0 as isize)),
    );
    tip(
        st.tooltip,
        search,
        "Type a word to see every matching setting from all pages at once.",
    );

    let nav = ctl(
        hwnd,
        WC_LISTBOXW,
        "",
        WINDOW_STYLE(
            WS_VISIBLE.0
                | WS_TABSTOP.0
                | LBS_NOTIFY as u32
                | LBS_HASSTRINGS as u32
                | LBS_OWNERDRAWFIXED as u32
                | LBS_NOINTEGRALHEIGHT as u32,
        ),
        8,
        PANE_Y,
        SIDEBAR_W - 16,
        NAV_ROW * PAGES.len() as i32,
        ID_NAV,
        f,
    );
    SendMessageW(
        nav,
        LB_SETITEMHEIGHT,
        Some(WPARAM(0)),
        Some(LPARAM(NAV_ROW as isize)),
    );
    for p in PAGES {
        let t = wide(p);
        SendMessageW(
            nav,
            LB_ADDSTRING,
            None,
            Some(LPARAM(t.as_pcwstr().0 as isize)),
        );
    }
    SendMessageW(nav, LB_SETCURSEL, Some(WPARAM(0)), None);

    let title = ctl(
        hwnd,
        WC_STATICW,
        PAGES[0],
        WINDOW_STYLE(WS_VISIBLE.0),
        CONTENT_X + 34,
        PAD + 2,
        PANE_W - 34,
        26,
        ID_TITLE,
        st.title,
    );
    painted_as(title, paint_as::PAGE);

    ctl(
        hwnd,
        WC_STATICW,
        "",
        WINDOW_STYLE(WS_VISIBLE.0 | SS_ETCHEDHORZ.0),
        0,
        FOOTER_Y,
        CLIENT_W,
        2,
        0,
        f,
    );

    let status = ctl(
        hwnd,
        WC_STATICW,
        "",
        WINDOW_STYLE(WS_VISIBLE.0 | SS_ENDELLIPSIS.0),
        PAD,
        FOOTER_Y + 14,
        CLIENT_W - PAD * 2,
        18,
        ID_STATUS,
        f,
    );
    painted_as(status, paint_as::WINDOW_GREY);

    let tooltip = st.tooltip;
    let by = CLIENT_H - PAD - 30;
    let b = |x: i32, w: i32, text: &str, id: usize, help: &str| {
        let h = ctl(
            hwnd,
            WC_BUTTONW,
            text,
            WINDOW_STYLE(WS_VISIBLE.0 | WS_TABSTOP.0),
            x,
            by,
            w,
            30,
            id,
            f,
        );
        tip(tooltip, h, help);
    };
    b(PAD, 124, "Restore defaults", ID_DEFAULTS,
        "Reset every setting on this window to its original value. Nothing is saved until you press Save.");
    b(
        CLIENT_W - PAD - 88 - 8 - 88 - 8 - 150,
        150,
        "Save as new preset",
        ID_SAVE_AS,
        "Keep the current preset untouched and store these timer lengths under the name above.",
    );
    b(
        CLIENT_W - PAD - 88 - 8 - 88,
        88,
        "Save",
        ID_SAVE,
        "Apply these settings and close.",
    );
    b(
        CLIENT_W - PAD - 88,
        88,
        "Cancel",
        ID_CANCEL,
        "Close without saving.",
    );

    let empty = ctl(
        st.pane,
        WC_STATICW,
        "Nothing matches that. Try a shorter word.",
        WINDOW_STYLE(0),
        0,
        0,
        ITEM_W,
        20,
        ID_EMPTY,
        f,
    );
    painted_as(empty, paint_as::PAGE_GREY);

    let mut rows = Rows {
        pane: st.pane,
        font: st.font,
        bold: st.bold,
        tooltip: st.tooltip,
        page: 0,
        section: 0,
        items: Vec::new(),
    };
    build_rows(&mut rows, st, cfg);
    st.items = rows.items;
}

unsafe fn build_rows(r: &mut Rows, st: &State, cfg: &Config) {
    let p = st.presets[st.current.min(st.presets.len() - 1)].clone();
    let names: Vec<&str> = st.presets.iter().map(|p| p.name.as_str()).collect();

    r.header(PAGE_TIMER, "Preset");
    r.combo(
        "\u{E8FD}",
        ID_PRESET,
        "Preset",
        &names,
        st.current,
        "The set of lengths the timer runs with.",
        "Switch between saved presets. Edit the boxes below and press Save to \
         change this preset, or Save as new preset to keep both.",
    );
    r.field(
        "\u{E8EC}",
        ID_NAME,
        "Preset name",
        &p.name,
        "Shown in the tray menu and usable with --preset.",
        "Renaming here and pressing Save renames the preset.",
    );

    r.header(PAGE_TIMER, "Lengths");
    r.field(
        "\u{E916}",
        ID_FOCUS,
        "Focus (minutes)",
        &num(p.focus_minutes),
        "Fractions are allowed, e.g. 0.5 for 30 seconds.",
        "How long one focus session lasts.",
    );
    r.field(
        "\u{EC32}",
        ID_SHORT,
        "Short break (minutes)",
        &num(p.short_break_minutes),
        "Taken after most focus sessions.",
        "The break taken after most focus sessions.",
    );
    r.field(
        "\u{E823}",
        ID_LONG,
        "Long break (minutes)",
        &num(p.long_break_minutes),
        "Taken after every Nth focus session.",
        "The longer break taken after every Nth focus session.",
    );
    r.field(
        "\u{E9D5}",
        ID_SESSIONS,
        "Sessions per long break",
        &p.sessions_before_long_break.to_string(),
        "4 is the classic Pomodoro cadence.",
        "How many focus sessions to complete before earning a long break.",
    );

    r.header(PAGE_SEQUENCE, "Automatic sequence");
    r.check(
        "\u{E8EE}",
        ID_SEQUENCE,
        "Run the Pomodoro sequence",
        cfg.behavior.sequence_enabled,
        "Turn this off to use the app as a plain manual timer.",
        "Master switch for focus, break and long break running one after another.",
    );
    r.sub_check(
        "\u{EC32}",
        ID_AUTO_BREAK,
        "Start a break when focus ends",
        cfg.behavior.auto_start_break,
        "When a focus session completes, begin the break automatically.",
    );
    r.sub_check(
        "\u{E916}",
        ID_AUTO_FOCUS,
        "Start focus when a break ends",
        cfg.behavior.auto_start_focus,
        "When a break completes, begin the next focus session automatically.",
    );
    r.sub_check(
        "\u{E8EE}",
        ID_REPEAT_CYCLES,
        "Start another cycle after a long break",
        cfg.behavior.repeat_cycles,
        "When the long break ends, begin a new focus session even if \
         “Start focus when a break ends” is off.",
    );

    r.header(PAGE_SEQUENCE, "Interruptions");
    r.check(
        "\u{E72E}",
        ID_STRICT,
        "Strict focus",
        cfg.behavior.strict_focus,
        "Confirm before stopping or skipping a running focus session.",
        "Ask for confirmation before stopping or skipping a running focus session, \
         so a stray click cannot end it. Breaks are never guarded.",
    );
    r.check(
        "\u{E81C}",
        ID_RESTORE_SESSION,
        "Restore an interrupted session on restart",
        cfg.behavior.restore_session_on_restart,
        "It always comes back paused, never running.",
        "If the app closes mid-session, bring the remaining time back next time it starts.",
    );
    r.combo(
        "\u{E7E8}",
        ID_WAKE,
        "After sleep",
        &[
            "Count the time asleep",
            "Ignore the time asleep",
            "Pause the timer",
        ],
        match cfg.behavior.wake_policy {
            WakePolicy::CountSleep => 0,
            WakePolicy::IgnoreSleep => 1,
            WakePolicy::Pause => 2,
        },
        "What a running timer does when the machine wakes up.",
        "Counting the time asleep means a 25 minute session started before a two \
         hour nap is simply over.",
    );

    r.header(PAGE_NOTIFICATIONS, "Windows notifications");
    r.check(
        "\u{EA8F}",
        ID_NOTIFY,
        "Show notifications",
        cfg.notifications.enabled,
        "Master switch for all notifications.",
        "Turn this off to make the app silent in the notification centre.",
    );
    r.sub_check(
        "\u{E916}",
        ID_N_FS,
        "Focus started",
        cfg.notifications.events.focus_start,
        "Notify when a focus session begins.",
    );
    r.sub_check(
        "\u{E7C1}",
        ID_N_FE,
        "Focus ended",
        cfg.notifications.events.focus_end,
        "Notify when a focus session completes.",
    );
    r.sub_check(
        "\u{EC32}",
        ID_N_BS,
        "Break started",
        cfg.notifications.events.break_start,
        "Notify when a break begins.",
    );
    r.sub_check(
        "\u{E823}",
        ID_N_BE,
        "Break ended",
        cfg.notifications.events.break_end,
        "Notify when a break completes.",
    );

    r.header(PAGE_NOTIFICATIONS, "When a session ends");
    r.note(
        "Pick any combination. Options that need another switch stay dimmed \
         until that switch is on. Overlay and flash still work under Do Not \
         Disturb.",
        3,
    );
    r.check(
        "\u{EA8F}",
        ID_ALERT_TOAST,
        "Tray toast",
        cfg.alerts.toast,
        "Needs “Show notifications” above.",
        "Posts the usual Windows toast when a focus or break ends. Disabled \
         while Show notifications is off.",
    );
    r.check(
        "\u{E7F4}",
        ID_ALERT_OVERLAY,
        "Centre overlay",
        cfg.alerts.overlay,
        "A large card in the middle of the screen.",
        "A dimmed backdrop and centred card that is hard to miss.",
    );
    r.sub_check(
        "\u{E769}",
        ID_REQUIRE_DISMISS,
        "Wait for dismiss — pauses the timer",
        cfg.alerts.require_dismiss,
        "Needs Centre overlay. Keeps the card open and pauses until Continue.",
    );
    r.check(
        "\u{E706}",
        ID_ALERT_FLASH,
        "Screen flash",
        cfg.alerts.flash,
        "A colour pulse across every screen.",
        "A colour pulse across every screen. Short is two quick pulses; Extra long is eight slower ones.",
    );
    r.combo(
        "\u{E706}",
        ID_FLASH_LENGTH,
        "Flash length",
        &["Short", "Normal", "Long", "Extra long"],
        cfg.alerts.flash_length.index(),
        "How long the screen flash runs.",
        "Needs Screen flash. Short is two quick pulses; Extra long is eight slower ones.",
    );

    r.header(PAGE_DND, "Muting");
    r.check(
        "\u{E708}",
        ID_DND,
        "Mute notifications during focus",
        cfg.dnd.enabled,
        "Do Not Disturb goes on with focus and off with a break.",
        "Switch Windows Do Not Disturb on when focus starts and off when a break starts.",
    );
    r.sub_check(
        "\u{E708}",
        ID_DND_SHORT_BREAK,
        "Keep it on during short breaks",
        cfg.dnd.keep_on_short_break,
        "Only a long break turns Do Not Disturb off. Needs muting during focus.",
    );
    r.note(
        "Your priority apps and contacts still get through - Windows keeps \
         applying your own priority list, which this app never touches.",
        3,
    );

    r.header(PAGE_DND, "Muted indicator");
    r.note("Only used while muting during focus is switched on.", 1);
    r.sub_check(
        "\u{EA8F}",
        ID_MUTE_TRAY,
        "Add a separate bell beside the clock",
        cfg.dnd.mute_tray_icon,
        "A second tray icon, present only while muted. Needs muting during focus.",
    );
    r.sub_check(
        "\u{E7ED}",
        ID_MUTE_WINDOW,
        "Show a muted bell in the timer window",
        cfg.dnd.mute_window,
        "Next to the countdown. Needs muting during focus.",
    );
    r.note(
        "Windows only shows its own muted icon for changes made from the taskbar. \
         These draw the same crossed-out bell instead. The timer icon stays \
         dedicated to progress because it cannot show both legibly.",
        4,
    );

    r.header(PAGE_SOUNDS, "Alert sounds");
    r.note(
        "Plays the Windows Asterisk sound from your scheme. Mute system sounds \
         in Windows if you want silence.",
        2,
    );
    r.check(
        "\u{E74F}",
        ID_MUTE,
        "Mute all sounds",
        cfg.sounds.muted,
        "When on, the switches below do nothing.",
        "Silences every cue without forgetting which ones you had chosen.",
    );
    r.sub_check(
        "\u{E916}",
        ID_S_FS,
        "Focus started",
        cfg.sounds.events.focus_start,
        "Play a cue when a focus session begins.",
    );
    r.sub_check(
        "\u{E7C1}",
        ID_S_FE,
        "Focus ended",
        cfg.sounds.events.focus_end,
        "Play a cue when a focus session completes.",
    );
    r.sub_check(
        "\u{EC32}",
        ID_S_BS,
        "Break started",
        cfg.sounds.events.break_start,
        "Play a cue when a break begins.",
    );
    r.sub_check(
        "\u{E823}",
        ID_S_BE,
        "Break ended",
        cfg.sounds.events.break_end,
        "Play a cue when a break completes.",
    );

    r.header(PAGE_WINDOW, "Compact timer");
    r.check(
        "\u{E7F4}",
        ID_MINI,
        "Show the compact timer window",
        cfg.display.mini_window,
        "A small resizable countdown you can click to start or pause.",
        "Drag its edges to shrink it to roughly the height of a taskbar button, \
         or make it large. Click it to start or pause.",
    );
    r.sub_check(
        "\u{E718}",
        ID_TOPMOST,
        "Keep it above other windows",
        cfg.display.always_on_top,
        "Needs the compact timer window. Stays visible over other apps.",
    );
    r.note(
        "The tray icon always stays visible; these are extra readouts.",
        2,
    );

    r.header(PAGE_HOTKEYS, "Global hotkeys");
    r.check(
        "\u{E765}",
        ID_HOTKEYS,
        "Enable global hotkeys",
        cfg.hotkeys.enabled,
        "Hotkeys work from any application.",
        "If another program already owns a combination, you will be told.",
    );
    r.field(
        "\u{E768}",
        ID_HK_TOGGLE,
        "Start / pause",
        &cfg.hotkeys.toggle,
        "For example Ctrl+Alt+F. Leave blank to disable.",
        "A modifier is required.",
    );
    r.field(
        "\u{E893}",
        ID_HK_SKIP,
        "Skip to next",
        &cfg.hotkeys.skip,
        "For example Ctrl+Alt+B. Leave blank to disable.",
        "A modifier is required.",
    );
}

/// The text currently in the search box, lowercased and trimmed.
unsafe fn query(hwnd: HWND) -> String {
    text_of(hwnd, ID_SEARCH).trim().to_lowercase()
}

/// Show the rows that belong on screen - the selected page, or everything the
/// search box matches - and stack them down the pane.
unsafe fn relayout(hwnd: HWND, st: &mut State) {
    let q = query(hwnd);
    let mut visible = vec![false; st.items.len()];
    if q.is_empty() {
        for (i, it) in st.items.iter().enumerate() {
            visible[i] = it.page == st.page;
        }
    } else {
        for i in 0..st.items.len() {
            if st.items[i].kind != Kind::Header && st.items[i].search.contains(&q) {
                visible[i] = true;
                visible[st.items[i].section] = true;
            }
        }
    }

    let focus = GetFocus();
    let mut cards = Vec::new();
    let mut y = PAD;
    for (i, it) in st.items.iter().enumerate() {
        if !visible[i] {
            hide(it);
            continue;
        }
        let (next, card) = place(it, y);
        y = next;
        if let Some(rect) = card {
            cards.push(Card {
                rect,
                icon: it.icon,
            });
        }
    }
    st.cards = cards;
    st.content_h = y;

    let none = !visible.iter().any(|v| *v);
    if let Ok(h) = GetDlgItem(Some(st.pane), ID_EMPTY as i32) {
        let _ = SetWindowPos(h, None, 0, PAD, ITEM_W, 20, SWP_NOZORDER);
        let _ = ShowWindow(h, if none { SW_SHOW } else { SW_HIDE });
    }

    if !focus.is_invalid() && !IsWindowVisible(focus).as_bool() {
        let fallback = if q.is_empty() { ID_NAV } else { ID_SEARCH };
        if let Ok(h) = GetDlgItem(Some(hwnd), fallback as i32) {
            let _ = SetFocus(Some(h));
        }
    }

    let heading = if q.is_empty() {
        PAGES[st.page]
    } else {
        "Search results"
    };
    if let Ok(nav) = GetDlgItem(Some(hwnd), ID_NAV as i32) {
        let sel = if q.is_empty() { st.page as isize } else { -1 };
        SendMessageW(nav, LB_SETCURSEL, Some(WPARAM(sel as usize)), None);
    }
    if let Ok(h) = GetDlgItem(Some(hwnd), ID_TITLE as i32) {
        let t = wide(heading);
        let _ = SetWindowTextW(h, t.as_pcwstr());
    }
    let strip = RECT {
        left: SIDEBAR_W,
        top: 0,
        right: CLIENT_W,
        bottom: PANE_Y,
    };
    let _ = InvalidateRect(Some(hwnd), Some(&strip), true);

    st.scroll = 0;
    set_scroll_range(st);
    let _ = InvalidateRect(Some(st.pane), None, true);
}

unsafe fn hide(it: &Item) {
    for h in [it.hwnd, it.caption, it.desc] {
        if !h.is_invalid() {
            let _ = ShowWindow(h, SW_HIDE);
        }
    }
}

/// Position one row at `y`, returning the top of the next row and the card to
/// paint behind this one.
unsafe fn place(it: &Item, y: i32) -> (i32, Option<RECT>) {
    let show = |h: HWND, x: i32, y: i32, w: i32, hh: i32| {
        if !h.is_invalid() {
            let _ = SetWindowPos(h, None, x, y, w, hh, SWP_NOZORDER);
            let _ = ShowWindow(h, SW_SHOW);
        }
    };
    let left = it.indent;
    let width = ITEM_W - it.indent;
    let inner_x = left + CARD_X + ICON_W;
    let inner_w = width - CARD_X * 2 - ICON_W;
    let card = |top: i32, bottom: i32| RECT {
        left,
        top,
        right: left + width,
        bottom,
    };

    match it.kind {
        Kind::Header => {
            let y = if y > PAD { y + 20 } else { y };
            show(it.hwnd, 0, y, ITEM_W, 20);
            (y + 26, None)
        }
        Kind::Check => {
            let mut inner = y + CARD_Y;
            let row_h = it.ctl_h.max(20);
            show(it.hwnd, inner_x, inner, inner_w, row_h);
            inner += row_h;
            if !it.desc.is_invalid() {
                show(it.desc, inner_x + 20, inner + 2, inner_w - 20, 16);
                inner += 18;
            }
            let bottom = inner + CARD_Y;
            (bottom + 6, Some(card(y, bottom)))
        }
        Kind::Field => {
            let mut inner = y + CARD_Y;
            show(it.caption, inner_x, inner + 4, CAP_W, H);
            show(
                it.hwnd,
                inner_x + CAP_W + 8,
                inner,
                inner_w - CAP_W - 8,
                it.ctl_h,
            );
            inner += H;
            if !it.desc.is_invalid() {
                show(it.desc, inner_x, inner + 4, inner_w, 16);
                inner += 20;
            }
            let bottom = inner + CARD_Y;
            (bottom + 6, Some(card(y, bottom)))
        }
        Kind::Note => {
            let x = left + CARD_X;
            show(it.hwnd, x, y + 2, ITEM_W - x - CARD_X, 15 * it.lines);
            (y + 15 * it.lines + 10, None)
        }
    }
}

unsafe fn set_scroll_range(st: &State) {
    let mut rc = RECT::default();
    let _ = GetClientRect(st.pane, &mut rc);
    let si = SCROLLINFO {
        cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
        fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
        nMin: 0,
        nMax: (st.content_h + PAD - 1).max(0),
        nPage: rc.bottom as u32,
        nPos: st.scroll,
        ..Default::default()
    };
    SetScrollInfo(st.pane, SB_VERT, &si, true);
}

/// Scroll the pane so that `pos` pixels of content are above the top edge.
unsafe fn scroll_to(st: &mut State, pos: i32) {
    let mut rc = RECT::default();
    let _ = GetClientRect(st.pane, &mut rc);
    let max = (st.content_h + PAD - rc.bottom).max(0);
    let pos = pos.clamp(0, max);
    let dy = st.scroll - pos;
    if dy == 0 {
        return;
    }
    st.scroll = pos;
    ScrollWindowEx(
        st.pane,
        0,
        dy,
        None,
        None,
        None,
        None,
        SW_SCROLLCHILDREN | SW_INVALIDATE | SW_ERASE,
    );
    set_scroll_range(st);
}

/// Paint the cards behind the rows currently on screen, each with its icon.
unsafe fn paint_cards(st: &State, hdc: HDC) {
    let old_pen = SelectObject(hdc, st.card_pen.into());
    let old_brush = SelectObject(hdc, st.card.into());
    for c in &st.cards {
        let _ = RoundRect(
            hdc,
            c.rect.left,
            c.rect.top - st.scroll,
            c.rect.right,
            c.rect.bottom - st.scroll,
            10,
            10,
        );
    }
    SelectObject(hdc, old_pen);
    SelectObject(hdc, old_brush);

    let old_font = SelectObject(hdc, st.icons.into());
    SetBkMode(hdc, TRANSPARENT);
    SetTextColor(hdc, COLORREF(GetSysColor(COLOR_WINDOWTEXT)));
    for c in &st.cards {
        let top = c.rect.top - st.scroll + CARD_Y;
        draw_icon(hdc, c.icon, c.rect.left + CARD_X, top, ICON_W - 8, 22);
    }
    SelectObject(hdc, old_font);
}

/// Draw one icon glyph, centred in the box it is given.
unsafe fn draw_icon(hdc: HDC, icon: &str, x: i32, y: i32, w: i32, h: i32) {
    if icon.is_empty() {
        return;
    }
    let mut glyph: Vec<u16> = icon.encode_utf16().collect();
    let mut rc = RECT {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    };
    DrawTextW(
        hdc,
        &mut glyph,
        &mut rc,
        DT_CENTER | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
    );
}

/// Draw one navigation entry: a rounded highlight and an accent bar for the
/// page being shown, plain text for the rest.
unsafe fn paint_nav_item(st: &State, dis: &DRAWITEMSTRUCT) {
    let rc = dis.rcItem;
    FillRect(dis.hDC, &rc, st.sidebar);

    if dis.itemState.0 & ODS_SELECTED.0 != 0 {
        let old_pen = SelectObject(dis.hDC, st.nav_selected_pen.into());
        let old_brush = SelectObject(dis.hDC, st.nav_selected.into());
        let _ = RoundRect(dis.hDC, rc.left, rc.top + 1, rc.right, rc.bottom - 1, 8, 8);
        SelectObject(dis.hDC, st.accent_pen.into());
        SelectObject(dis.hDC, st.accent.into());
        let _ = RoundRect(
            dis.hDC,
            rc.left + 2,
            rc.top + 8,
            rc.left + 6,
            rc.bottom - 8,
            4,
            4,
        );
        SelectObject(dis.hDC, old_pen);
        SelectObject(dis.hDC, old_brush);
    }

    let mut text = [0u16; 64];
    let n = SendMessageW(
        dis.hwndItem,
        LB_GETTEXT,
        Some(WPARAM(dis.itemID as usize)),
        Some(LPARAM(text.as_mut_ptr() as isize)),
    )
    .0;
    if n <= 0 {
        return;
    }
    SetBkMode(dis.hDC, TRANSPARENT);
    SetTextColor(dis.hDC, COLORREF(GetSysColor(COLOR_WINDOWTEXT)));

    let old_font = SelectObject(dis.hDC, st.icons.into());
    draw_icon(
        dis.hDC,
        PAGE_ICONS
            .get(dis.itemID as usize)
            .copied()
            .unwrap_or_default(),
        rc.left + 12,
        rc.top,
        20,
        rc.bottom - rc.top,
    );
    SelectObject(dis.hDC, st.font.into());
    let mut text_rc = RECT {
        left: rc.left + 40,
        top: rc.top,
        right: rc.right - 8,
        bottom: rc.bottom,
    };
    DrawTextW(
        dis.hDC,
        &mut text[..n as usize],
        &mut text_rc,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
    );
    SelectObject(dis.hDC, old_font);
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

unsafe fn text_of(parent: HWND, id: usize) -> String {
    let h = GetDlgItem(Some(parent), id as i32).unwrap_or_default();
    if h.is_invalid() {
        return String::new();
    }
    let mut buf = [0u16; 256];
    let n = GetWindowTextW(h, &mut buf);
    String::from_utf16_lossy(&buf[..n as usize])
}

/// Enable or disable dependent controls so irrelevant options cannot be
/// toggled while their parent switch is off.
unsafe fn sync_dependencies(pane: HWND) {
    let set_enabled = |id: usize, enabled: bool| {
        if let Ok(control) = GetDlgItem(Some(pane), id as i32) {
            let _ = EnableWindow(control, enabled);
        }
    };
    let sequence = checked(pane, ID_SEQUENCE);
    set_enabled(ID_AUTO_BREAK, sequence);
    set_enabled(ID_AUTO_FOCUS, sequence);
    set_enabled(ID_REPEAT_CYCLES, sequence);

    let notifications = checked(pane, ID_NOTIFY);
    for id in [ID_N_FS, ID_N_FE, ID_N_BS, ID_N_BE, ID_ALERT_TOAST] {
        set_enabled(id, notifications);
    }

    set_enabled(ID_REQUIRE_DISMISS, checked(pane, ID_ALERT_OVERLAY));

    let dnd = checked(pane, ID_DND);
    set_enabled(ID_DND_SHORT_BREAK, dnd);
    set_enabled(ID_MUTE_TRAY, dnd);
    set_enabled(ID_MUTE_WINDOW, dnd);

    let sounds_unmuted = !checked(pane, ID_MUTE);
    for id in [ID_S_FS, ID_S_FE, ID_S_BS, ID_S_BE] {
        set_enabled(id, sounds_unmuted);
    }
    set_enabled(ID_FLASH_LENGTH, checked(pane, ID_ALERT_FLASH));

    set_enabled(ID_TOPMOST, checked(pane, ID_MINI));

    let hotkeys = checked(pane, ID_HOTKEYS);
    set_enabled(ID_HK_TOGGLE, hotkeys);
    set_enabled(ID_HK_SKIP, hotkeys);
}

unsafe fn checked(parent: HWND, id: usize) -> bool {
    let h = GetDlgItem(Some(parent), id as i32).unwrap_or_default();
    !h.is_invalid() && SendMessageW(h, BM_GETCHECK, None, None).0 == BST_CHECKED.0 as isize
}

unsafe fn set_checked(parent: HWND, id: usize, on: bool) {
    if let Ok(h) = GetDlgItem(Some(parent), id as i32) {
        SendMessageW(
            h,
            BM_SETCHECK,
            Some(WPARAM(if on { BST_CHECKED.0 as usize } else { 0 })),
            None,
        );
    }
}

unsafe fn combo_index(parent: HWND, id: usize) -> usize {
    let h = GetDlgItem(Some(parent), id as i32).unwrap_or_default();
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
unsafe fn read_preset(pane: HWND, base: &Preset) -> Result<Preset, &'static str> {
    let name = text_of(pane, ID_NAME).trim().to_string();
    Ok(Preset {
        name: if name.is_empty() {
            base.name.clone()
        } else {
            name
        },
        focus_minutes: parse_minutes(&text_of(pane, ID_FOCUS)).map_err(|_| "Focus (minutes)")?,
        short_break_minutes: parse_minutes(&text_of(pane, ID_SHORT))
            .map_err(|_| "Short break (minutes)")?,
        long_break_minutes: parse_minutes(&text_of(pane, ID_LONG))
            .map_err(|_| "Long break (minutes)")?,
        sessions_before_long_break: text_of(pane, ID_SESSIONS)
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|n| *n > 0)
            .ok_or("Sessions per long break")?,
    })
}

unsafe fn collect(st: &mut State, save_as_new: bool) -> Result<Config, &'static str> {
    let pane = st.pane;
    let app = &mut *st.app;
    let mut cfg = app.config().clone();

    let idx = st.current.min(st.presets.len().saturating_sub(1));
    let edited = read_preset(pane, &st.presets[idx])?;
    if save_as_new && !st.presets.iter().any(|p| p.name == edited.name) {
        st.presets.push(edited.clone());
    } else {
        st.presets[idx] = edited.clone();
    }
    cfg.presets = st.presets.clone();
    cfg.active_preset = edited.name;

    let sequence = checked(pane, ID_SEQUENCE);
    cfg.behavior.sequence_enabled = sequence;
    cfg.behavior.auto_start_break = sequence && checked(pane, ID_AUTO_BREAK);
    cfg.behavior.auto_start_focus = sequence && checked(pane, ID_AUTO_FOCUS);
    cfg.behavior.repeat_cycles = sequence && checked(pane, ID_REPEAT_CYCLES);
    cfg.behavior.strict_focus = checked(pane, ID_STRICT);
    cfg.behavior.restore_session_on_restart = checked(pane, ID_RESTORE_SESSION);
    cfg.behavior.wake_policy = match combo_index(pane, ID_WAKE) {
        1 => WakePolicy::IgnoreSleep,
        2 => WakePolicy::Pause,
        _ => WakePolicy::CountSleep,
    };

    let dnd = checked(pane, ID_DND);
    cfg.dnd.enabled = dnd;
    cfg.dnd.keep_on_short_break = dnd && checked(pane, ID_DND_SHORT_BREAK);
    cfg.dnd.mute_tray_icon = dnd && checked(pane, ID_MUTE_TRAY);
    cfg.dnd.mute_window = dnd && checked(pane, ID_MUTE_WINDOW);
    let mini = checked(pane, ID_MINI);
    cfg.display.mini_window = mini;
    cfg.display.always_on_top = mini && checked(pane, ID_TOPMOST);

    let notify = checked(pane, ID_NOTIFY);
    cfg.notifications.enabled = notify;
    cfg.notifications.events.focus_start = checked(pane, ID_N_FS);
    cfg.notifications.events.focus_end = checked(pane, ID_N_FE);
    cfg.notifications.events.break_start = checked(pane, ID_N_BS);
    cfg.notifications.events.break_end = checked(pane, ID_N_BE);

    let overlay = checked(pane, ID_ALERT_OVERLAY);
    cfg.alerts.toast = notify && checked(pane, ID_ALERT_TOAST);
    cfg.alerts.overlay = overlay;
    cfg.alerts.flash = checked(pane, ID_ALERT_FLASH);
    cfg.alerts.require_dismiss = overlay && checked(pane, ID_REQUIRE_DISMISS);

    cfg.sounds.muted = checked(pane, ID_MUTE);
    cfg.sounds.events.focus_start = checked(pane, ID_S_FS);
    cfg.sounds.events.focus_end = checked(pane, ID_S_FE);
    cfg.sounds.events.break_start = checked(pane, ID_S_BS);
    cfg.sounds.events.break_end = checked(pane, ID_S_BE);
    cfg.alerts.flash_length = LengthPreset::from_index(combo_index(pane, ID_FLASH_LENGTH));

    cfg.hotkeys.enabled = checked(pane, ID_HOTKEYS);
    cfg.hotkeys.toggle = text_of(pane, ID_HK_TOGGLE).trim().to_string();
    cfg.hotkeys.skip = text_of(pane, ID_HK_SKIP).trim().to_string();

    Ok(cfg)
}

/// Load the selected preset's values into the edit fields.
unsafe fn show_preset(st: &State) {
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
        if let Ok(h) = GetDlgItem(Some(st.pane), id as i32) {
            let t = wide(&val);
            let _ = SetWindowTextW(h, t.as_pcwstr());
        }
    }
}

/// Put every control back to the built-in defaults, without saving.
unsafe fn restore_defaults(st: &mut State) {
    let pane = st.pane;
    let d = Config::default();
    st.presets = d.presets.clone();
    st.current = 0;
    if let Ok(h) = GetDlgItem(Some(pane), ID_PRESET as i32) {
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
    show_preset(st);

    for (id, v) in [
        (ID_SEQUENCE, d.behavior.sequence_enabled),
        (ID_AUTO_BREAK, d.behavior.auto_start_break),
        (ID_AUTO_FOCUS, d.behavior.auto_start_focus),
        (ID_REPEAT_CYCLES, d.behavior.repeat_cycles),
        (ID_STRICT, d.behavior.strict_focus),
        (ID_RESTORE_SESSION, d.behavior.restore_session_on_restart),
        (ID_DND, d.dnd.enabled),
        (ID_DND_SHORT_BREAK, d.dnd.keep_on_short_break),
        (ID_MUTE_TRAY, d.dnd.mute_tray_icon),
        (ID_MUTE_WINDOW, d.dnd.mute_window),
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
        (ID_ALERT_TOAST, d.alerts.toast),
        (ID_ALERT_OVERLAY, d.alerts.overlay),
        (ID_ALERT_FLASH, d.alerts.flash),
        (ID_REQUIRE_DISMISS, d.alerts.require_dismiss),
        (ID_HOTKEYS, d.hotkeys.enabled),
    ] {
        set_checked(pane, id, v);
    }
    if let Ok(h) = GetDlgItem(Some(pane), ID_WAKE as i32) {
        let sel = match d.behavior.wake_policy {
            WakePolicy::CountSleep => 0,
            WakePolicy::IgnoreSleep => 1,
            WakePolicy::Pause => 2,
        };
        SendMessageW(h, CB_SETCURSEL, Some(WPARAM(sel)), None);
    }
    if let Ok(h) = GetDlgItem(Some(pane), ID_FLASH_LENGTH as i32) {
        SendMessageW(
            h,
            CB_SETCURSEL,
            Some(WPARAM(d.alerts.flash_length.index())),
            None,
        );
    }
    for (id, v) in [
        (ID_HK_TOGGLE, d.hotkeys.toggle),
        (ID_HK_SKIP, d.hotkeys.skip),
    ] {
        if let Ok(h) = GetDlgItem(Some(pane), id as i32) {
            let t = wide(&v);
            let _ = SetWindowTextW(h, t.as_pcwstr());
        }
    }
}

/// Give a static control the surface it was built for, so labels do not sit in
/// a grey box of their own on a white card.
unsafe fn paint_static(st: &State, in_pane: bool, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let child = HWND(lparam.0 as *mut _);
    let how = GetWindowLongPtrW(child, GWLP_USERDATA);
    let hdc = HDC(wparam.0 as *mut _);
    let grey = matches!(
        how,
        paint_as::CARD_GREY | paint_as::PAGE_GREY | paint_as::WINDOW_GREY
    );
    SetTextColor(
        hdc,
        COLORREF(GetSysColor(if grey {
            COLOR_GRAYTEXT
        } else {
            COLOR_WINDOWTEXT
        })),
    );
    SetBkMode(hdc, TRANSPARENT);
    let brush = match how {
        paint_as::PAGE | paint_as::PAGE_GREY => st.page_bg,
        paint_as::CARD | paint_as::CARD_GREY if in_pane => st.card,
        _ if in_pane => st.page_bg,
        _ => GetSysColorBrush(COLOR_WINDOW),
    };
    LRESULT(brush.0 as isize)
}

unsafe extern "system" fn pane_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let st = &mut *ptr;

    match msg {
        WM_COMMAND => {
            if let Ok(parent) = GetParent(hwnd) {
                SendMessageW(parent, WM_COMMAND, Some(wparam), Some(lparam));
            }
            LRESULT(0)
        }
        WM_ERASEBKGND => {
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);
            FillRect(HDC(wparam.0 as *mut _), &rc, st.page_bg);
            LRESULT(1)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            paint_cards(st, hdc);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_CTLCOLORSTATIC => paint_static(st, true, wparam, lparam),
        WM_VSCROLL => {
            let mut si = SCROLLINFO {
                cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                fMask: SIF_ALL,
                ..Default::default()
            };
            let _ = GetScrollInfo(hwnd, SB_VERT, &mut si);
            let pos = match SCROLLBAR_COMMAND((wparam.0 & 0xFFFF) as i32) {
                SB_LINEUP => st.scroll - 28,
                SB_LINEDOWN => st.scroll + 28,
                SB_PAGEUP => st.scroll - si.nPage as i32,
                SB_PAGEDOWN => st.scroll + si.nPage as i32,
                SB_THUMBTRACK | SB_THUMBPOSITION => si.nTrackPos,
                SB_TOP => 0,
                SB_BOTTOM => st.content_h,
                _ => st.scroll,
            };
            scroll_to(st, pos);
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam.0 >> 16) & 0xFFFF) as i16;
            scroll_to(st, st.scroll - delta as i32 * 48 / 120);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }

    match msg {
        WM_PAINT => {
            let st = &*ptr;
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            FillRect(
                hdc,
                &RECT {
                    left: 0,
                    top: 0,
                    right: SIDEBAR_W,
                    bottom: FOOTER_Y,
                },
                st.sidebar,
            );
            FillRect(
                hdc,
                &RECT {
                    left: SIDEBAR_W,
                    top: 0,
                    right: CLIENT_W,
                    bottom: FOOTER_Y,
                },
                st.page_bg,
            );
            let old_font = SelectObject(hdc, st.icons.into());
            SetBkMode(hdc, TRANSPARENT);
            SetTextColor(hdc, COLORREF(GetSysColor(COLOR_WINDOWTEXT)));
            let icon = if query(hwnd).is_empty() {
                PAGE_ICONS[st.page]
            } else {
                "\u{E721}"
            };
            draw_icon(hdc, icon, CONTENT_X, PAD, 28, 28);
            SelectObject(hdc, old_font);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_CTLCOLORSTATIC => paint_static(&*ptr, false, wparam, lparam),
        WM_CTLCOLORLISTBOX => {
            SetBkMode(HDC(wparam.0 as *mut _), TRANSPARENT);
            LRESULT((*ptr).sidebar.0 as isize)
        }
        WM_DRAWITEM => {
            let dis = &*(lparam.0 as *const DRAWITEMSTRUCT);
            if dis.CtlID == ID_NAV as u32 && dis.itemID != u32::MAX {
                paint_nav_item(&*ptr, dis);
            }
            LRESULT(1)
        }
        WM_COMMAND => {
            let id = wparam.0 & 0xFFFF;
            let code = ((wparam.0 >> 16) & 0xFFFF) as u32;
            let st = &mut *ptr;
            match (id, code) {
                (ID_SEARCH, EN_CHANGE) => relayout(hwnd, st),
                (ID_NAV, LBN_SELCHANGE) => {
                    let sel = SendMessageW(
                        GetDlgItem(Some(hwnd), ID_NAV as i32).unwrap_or_default(),
                        LB_GETCURSEL,
                        None,
                        None,
                    )
                    .0;
                    if sel >= 0 {
                        st.page = sel as usize;
                        relayout(hwnd, st);
                    }
                }
                (ID_PRESET, CBN_SELCHANGE) => {
                    let idx = st.current.min(st.presets.len().saturating_sub(1));
                    if let Ok(p) = read_preset(st.pane, &st.presets[idx]) {
                        st.presets[idx] = p;
                    }
                    st.current = combo_index(st.pane, ID_PRESET);
                    show_preset(st);
                }
                (ID_DEFAULTS, _) => {
                    restore_defaults(st);
                    sync_dependencies(st.pane);
                }
                (
                    ID_SEQUENCE | ID_NOTIFY | ID_ALERT_OVERLAY | ID_ALERT_FLASH | ID_DND | ID_MUTE
                    | ID_MINI | ID_HOTKEYS | ID_S_FS | ID_S_FE | ID_S_BS | ID_S_BE,
                    BN_CLICKED,
                ) => sync_dependencies(st.pane),
                (ID_SAVE, _) | (ID_SAVE_AS, _) => match collect(st, id == ID_SAVE_AS) {
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
            let _ = DeleteObject(st.title.into());
            let _ = DeleteObject(st.icons.into());
            let _ = DeleteObject(st.sidebar.into());
            let _ = DeleteObject(st.page_bg.into());
            let _ = DeleteObject(st.card.into());
            let _ = DeleteObject(st.card_pen.into());
            let _ = DeleteObject(st.nav_selected.into());
            let _ = DeleteObject(st.nav_selected_pen.into());
            let _ = DeleteObject(st.accent.into());
            let _ = DeleteObject(st.accent_pen.into());
            (*st.app).on_settings_closed();
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
