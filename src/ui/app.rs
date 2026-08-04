//! The application window, tray icon and message pump.

use std::ffi::c_void;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::cli::Command;
use crate::config::Config;
use crate::orchestrator::{Orchestrator, UiEvent};
use crate::os::dnd::worker::DndWorker;
use crate::os::sound;
use crate::session;
use crate::timer::{Phase, Timer};
use crate::ui::hotkeys::{self, Hotkeys};
use crate::ui::icon::{self, OwnedIcon, Visual};
use crate::ui::tray::Tray;
use crate::ui::wide;

pub const CLASS_MAIN: &str = "TaskbarFocusMainWindow";
pub const MUTEX_NAME: &str = "Local\\TaskbarFocusSingleInstance";

const WM_TRAY: u32 = WM_APP + 1;
const WM_DND_REPORT: u32 = WM_APP + 2;

/// Notification-area icon ids. The muted mark is a second icon rather than
/// part of the first so it can sit beside the clock on its own, which is where
/// Windows would have put its own indicator.
///
/// It is an indicator, not a control, so any click on it opens the menu rather
/// than starting or pausing a session the way the timer icon does.
const TRAY_ID: u32 = 1;
const MUTE_TRAY_ID: u32 = 2;

const TIMER_ID: usize = 1;
/// Twice a second: the countdown stays honest without burning power.
const TICK_MS: u32 = 500;

const HOTKEY_TOGGLE: i32 = 1;
const HOTKEY_SKIP: i32 = 2;

const ID_TOGGLE: usize = 1001;
const ID_START_FOCUS: usize = 1002;
const ID_START_BREAK: usize = 1003;
const ID_STOP: usize = 1004;
const ID_SKIP: usize = 1005;
const ID_SETTINGS: usize = 1006;
const ID_EXIT: usize = 1007;
const ID_HIDE_MINI: usize = 1008;
const ID_RESET_MINI: usize = 1009;
const ID_PRESET_BASE: usize = 2000;

/// The application icon, as embedded in the executable by `build.rs`.
///
/// Loaded once and never destroyed: it lives as long as the window classes do,
/// which is the life of the process. Falls back to a drawn ring if the resource
/// is missing, so the app never shows the generic Windows placeholder.
pub fn shared_icon() -> HICON {
    use std::sync::OnceLock;
    static ICON: OnceLock<isize> = OnceLock::new();
    let raw = *ICON.get_or_init(|| unsafe {
        let instance = GetModuleHandleW(None).unwrap_or_default();
        if let Ok(h) = LoadIconW(Some(instance.into()), PCWSTR(1 as *const u16)) {
            if !h.is_invalid() {
                return h.0 as isize;
            }
        }
        let v = Visual {
            rgb: (0xE8, 0x56, 0x3F),
            progress: 1.0,
            paused: false,
            idle: false,
            alpha: 1.0,
        };
        match icon::render(v, 32) {
            Some(ic) => {
                let h = ic.handle().0 as isize;
                std::mem::forget(ic);
                h
            }
            None => 0,
        }
    });
    HICON(raw as *mut c_void)
}

pub struct App {
    hwnd: HWND,
    mini_hwnd: Option<HWND>,
    pub orch: Orchestrator,
    tray: Tray,
    /// Kept alive because the shell keeps using the handle we hand it.
    current_icon: Option<OwnedIcon>,
    last_visual: Option<Visual>,
    last_tooltip: String,
    hotkeys: Hotkeys,
    taskbar_created_msg: u32,
    settings_hwnd: Option<HWND>,
    /// The standalone muted mark beside the clock, shown only while muted.
    mute_tray: Tray,
    mute_icon: Option<OwnedIcon>,
}

/// Entry point for the UI. Blocks until the user exits.
///
/// `first_run` makes the initial launch visibly announce itself; see
/// [`crate::config::is_first_run`].
pub fn run(
    config: Config,
    timer: Timer,
    initial: Command,
    first_run: bool,
) -> windows::core::Result<()> {
    unsafe {
        let icc = windows::Win32::UI::Controls::INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<windows::Win32::UI::Controls::INITCOMMONCONTROLSEX>()
                as u32,
            dwICC: windows::Win32::UI::Controls::ICC_STANDARD_CLASSES
                | windows::Win32::UI::Controls::ICC_TAB_CLASSES,
        };
        let _ = windows::Win32::UI::Controls::InitCommonControlsEx(&icc);

        let instance = GetModuleHandleW(None)?;
        let class = wide(CLASS_MAIN);

        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: class.as_pcwstr(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hIcon: shared_icon(),
            ..Default::default()
        };
        RegisterClassW(&wc);

        let title = wide("taskbar-focus");

        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class.as_pcwstr(),
            title.as_pcwstr(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )?;

        let hwnd_bits = hwnd.0 as isize;
        let dnd = DndWorker::spawn(move || {
            let _ = PostMessageW(
                Some(HWND(hwnd_bits as *mut c_void)),
                WM_DND_REPORT,
                WPARAM(0),
                LPARAM(0),
            );
        });

        let taskbar_created_msg = RegisterWindowMessageW(wide("TaskbarCreated").as_pcwstr());

        let mut app = Box::new(App {
            hwnd,
            mini_hwnd: None,
            orch: Orchestrator::new(config, timer, dnd),
            tray: Tray::new(hwnd, TRAY_ID, WM_TRAY),
            current_icon: None,
            last_visual: None,
            last_tooltip: String::new(),
            hotkeys: Hotkeys::new(hwnd),
            taskbar_created_msg,
            settings_hwnd: None,
            mute_tray: Tray::new(hwnd, MUTE_TRAY_ID, WM_TRAY),
            mute_icon: None,
        });

        app.register_hotkeys();
        let ptr = Box::into_raw(app);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, ptr as isize);

        let app = &mut *ptr;

        app.sync_mini_window();
        app.refresh();

        if first_run {
            app.tray.notify(
                "taskbar-focus is running",
                "Look for the ring icon in the notification area - click the ^ arrow \
                 and drag it onto the taskbar to keep it visible. Left-click starts a \
                 focus session; right-click opens the menu.",
            );
            let _ = app.orch.config.save();
            app.open_settings();
        }

        if initial.is_remote_control() {
            app.handle_command(&initial);
        }

        SetTimer(Some(hwnd), TIMER_ID, TICK_MS, None);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let settings = (*ptr).settings_hwnd;
            if let Some(s) = settings {
                if IsDialogMessageW(s, &msg).as_bool() {
                    continue;
                }
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        drop(Box::from_raw(ptr));
        Ok(())
    }
}

impl App {
    fn register_hotkeys(&mut self) {
        self.hotkeys.unregister_all();
        if !self.orch.config.hotkeys.enabled {
            return;
        }
        let mut failed = Vec::new();
        if let Some(a) = hotkeys::parse(&self.orch.config.hotkeys.toggle) {
            if !self.hotkeys.register(HOTKEY_TOGGLE, a) {
                failed.push(self.orch.config.hotkeys.toggle.clone());
            }
        }
        if let Some(a) = hotkeys::parse(&self.orch.config.hotkeys.skip) {
            if !self.hotkeys.register(HOTKEY_SKIP, a) {
                failed.push(self.orch.config.hotkeys.skip.clone());
            }
        }
        if !failed.is_empty() {
            self.tray.notify(
                "Hotkey unavailable",
                &format!("Another application already uses {}.", failed.join(" and ")),
            );
        }
    }

    /// Create or destroy the compact timer window to match settings.
    fn sync_mini_window(&mut self) {
        let want = self.orch.config.display.mini_window;
        let topmost = self.orch.config.display.always_on_top;
        match (want, self.mini_hwnd) {
            (true, None) => {
                let geo = self.orch.config.display.mini_geometry;
                self.mini_hwnd = crate::ui::mini::open(self as *mut App, geo, topmost);
            }
            (true, Some(h)) => crate::ui::mini::set_topmost(h, topmost),
            (false, Some(h)) => {
                unsafe {
                    let _ = DestroyWindow(h);
                }
                self.mini_hwnd = None;
            }
            (false, None) => {}
        }
    }

    /// Clicking the compact window body behaves like the tray icon.
    pub fn mini_clicked(&mut self) {
        self.handle_command(&Command::Toggle);
    }

    /// Forget the saved size and position, putting the window back to the
    /// default strip. Recreated rather than resized so it also returns to the
    /// default placement.
    fn reset_mini_window(&mut self) {
        self.orch.config.display.mini_geometry = None;
        let _ = self.orch.config.save();
        if let Some(h) = self.mini_hwnd.take() {
            unsafe {
                let _ = DestroyWindow(h);
            }
        }
        self.sync_mini_window();
        self.refresh();
    }

    /// The user moved or resized it; remember where.
    pub fn mini_geometry_changed(&mut self) {
        if let Some(h) = self.mini_hwnd {
            if let Some(g) = crate::ui::mini::geometry(h) {
                self.orch.config.display.mini_geometry = Some(g);
                let _ = self.orch.config.save();
            }
        }
    }

    /// Closing it switches the setting off, rather than leaving a checkbox
    /// insisting the window is shown when it plainly is not.
    pub fn mini_closed_by_user(&mut self) {
        self.mini_geometry_changed();
        self.orch.config.display.mini_window = false;
        let _ = self.orch.config.save();
        if let Some(h) = self.mini_hwnd.take() {
            unsafe {
                let _ = DestroyWindow(h);
            }
        }
    }

    /// Repaint the tray icon, tooltip and taskbar title if anything changed.
    ///
    /// Everything here is guarded on "did it actually change?". Pushing an
    /// unchanged icon or tooltip to the shell twice a second is what makes a
    /// tray icon shimmer.
    fn refresh(&mut self) {
        let muted = self.orch.dnd_active();
        let visual = Visual::from_state(self.orch.timer.state());
        let tooltip = self.orch.tooltip();
        let icon_changed = self.last_visual != Some(visual) || self.current_icon.is_none();
        let tip_changed = tooltip != self.last_tooltip;

        if icon_changed {
            if let Some(ic) = icon::render(visual, icon::tray_icon_size()) {
                self.tray.set(ic.handle(), &tooltip);

                self.current_icon = Some(ic);
                self.last_visual = Some(visual);
                self.last_tooltip = tooltip.clone();
            }
        } else if tip_changed {
            if let Some(ic) = &self.current_icon {
                self.tray.set(ic.handle(), &tooltip);
            }
            self.last_tooltip = tooltip;
        }

        self.sync_mute_tray(muted);

        if let Some(s) = self.settings_hwnd {
            crate::ui::settings::refresh_status(s);
        }
        if let Some(m) = self.mini_hwnd {
            crate::ui::mini::refresh(m, self.orch.config.display.always_on_top);
        }
    }

    /// Add or remove the standalone muted mark in the notification area.
    ///
    /// It only exists while notifications are muted: an indicator that is
    /// always there says nothing. Windows 11 files new tray icons into the
    /// overflow, so the first time it appears the user may have to drag it out
    /// - which is also true of the timer icon itself.
    fn sync_mute_tray(&mut self, muted: bool) {
        let want = muted && self.orch.config.dnd.mute_tray_icon;
        if want == self.mute_icon.is_some() {
            return;
        }
        if !want {
            self.mute_tray.remove();
            self.mute_icon = None;
            return;
        }
        if let Some(icon) = icon::render_mute_icon(icon::tray_icon_size()) {
            self.mute_tray
                .set(icon.handle(), "Notifications are muted - taskbar-focus");
            self.mute_icon = Some(icon);
        }
    }

    fn apply_events(&mut self, events: Vec<UiEvent>) {
        for e in events {
            match e {
                UiEvent::Notify { title, body } => self.tray.notify(&title, &body),
                UiEvent::Sound(ev) => sound::play(ev),
                UiEvent::Refresh => {}
                UiEvent::Warn(msg) => self.tray.notify("taskbar-focus", &msg),
            }
        }
        self.refresh();
        self.orch.ui_events_applied();
    }

    /// Run a command, applying the "strict focus" guard where it applies.
    fn handle_command(&mut self, cmd: &Command) {
        if self.needs_confirmation(cmd) && !self.confirm(cmd) {
            return;
        }
        let events = self.orch.dispatch(cmd);
        self.apply_events(events);
    }

    /// In strict mode, abandoning a *running* focus session needs a deliberate
    /// answer - the whole point is to defeat an absent-minded click.
    fn needs_confirmation(&self, cmd: &Command) -> bool {
        if !self.orch.config.behavior.strict_focus {
            return false;
        }
        if !matches!(cmd, Command::Stop | Command::Skip) {
            return false;
        }
        self.orch.timer.state().is_running()
            && self.orch.timer.state().phase() == Some(Phase::Focus)
    }

    fn confirm(&self, cmd: &Command) -> bool {
        let what = if matches!(cmd, Command::Stop) {
            "Stop this focus session?"
        } else {
            "Skip the rest of this focus session?"
        };
        let text = wide(what);
        let caption = wide("Strict focus");
        unsafe {
            MessageBoxW(
                Some(self.hwnd),
                text.as_pcwstr(),
                caption.as_pcwstr(),
                MB_YESNO | MB_ICONQUESTION | MB_SETFOREGROUND,
            ) == IDYES
        }
    }

    pub fn show_menu(&mut self) {
        unsafe {
            let Ok(menu) = CreatePopupMenu() else { return };
            let running = self.orch.timer.state().is_running();
            let active = self.orch.timer.state().phase().is_some();

            let _ = AppendMenuW(
                menu,
                MF_STRING | MF_DISABLED,
                0,
                wide(&self.orch.tooltip()).as_pcwstr(),
            );
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());

            // Menus put anything after a tab in the accelerator column, which
            // is where a shortcut belongs. Only shown when the binding is
            // actually enabled and parses, so the menu never advertises a
            // shortcut that does nothing.
            let shortcut = |configured: &str| -> String {
                if self.orch.config.hotkeys.enabled && hotkeys::parse(configured).is_some() {
                    configured.trim().to_string()
                } else {
                    String::new()
                }
            };
            let toggle_key = shortcut(&self.orch.config.hotkeys.toggle);
            let skip_key = shortcut(&self.orch.config.hotkeys.skip);

            let toggle_verb = match (active, running) {
                (false, _) => "Start focus",
                (true, true) => "Pause",
                (true, false) => "Resume",
            };
            let toggle_label = if toggle_key.is_empty() {
                format!("{toggle_verb}\tClick")
            } else {
                format!("{toggle_verb}\tClick  \u{2022}  {toggle_key}")
            };
            let _ = AppendMenuW(menu, MF_STRING, ID_TOGGLE, wide(&toggle_label).as_pcwstr());
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            let _ = AppendMenuW(
                menu,
                MF_STRING,
                ID_START_FOCUS,
                wide("Start focus").as_pcwstr(),
            );
            let _ = AppendMenuW(
                menu,
                MF_STRING,
                ID_START_BREAK,
                wide("Start break").as_pcwstr(),
            );
            let disabled = if active {
                MF_STRING
            } else {
                MF_STRING | MF_GRAYED
            };
            let skip_label = if skip_key.is_empty() {
                "Skip to next".to_string()
            } else {
                format!("Skip to next\t{skip_key}")
            };
            let _ = AppendMenuW(menu, disabled, ID_SKIP, wide(&skip_label).as_pcwstr());
            let _ = AppendMenuW(menu, disabled, ID_STOP, wide("Stop").as_pcwstr());

            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            if let Ok(sub) = CreatePopupMenu() {
                let active_name = self.orch.config.preset().name;
                for (i, p) in self.orch.config.presets.iter().enumerate() {
                    let mut flags = MF_STRING;
                    if p.name == active_name {
                        flags |= MF_CHECKED;
                    }
                    let _ = AppendMenuW(sub, flags, ID_PRESET_BASE + i, wide(&p.name).as_pcwstr());
                }
                let _ = AppendMenuW(menu, MF_POPUP, sub.0 as usize, wide("Preset").as_pcwstr());
            }

            let status = match self.orch.dnd_note() {
                Some(note) => note.to_string(),
                None => {
                    let s = self.orch.dnd_status();
                    let mut c = s.chars();
                    match c.next() {
                        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                        None => s.to_string(),
                    }
                }
            };
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            let _ = AppendMenuW(menu, MF_STRING | MF_DISABLED, 0, wide(&status).as_pcwstr());

            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            let mini_label = if self.mini_hwnd.is_some() {
                "Hide timer window"
            } else {
                "Show timer window"
            };
            let _ = AppendMenuW(menu, MF_STRING, ID_HIDE_MINI, wide(mini_label).as_pcwstr());
            if self.mini_hwnd.is_some() {
                let _ = AppendMenuW(
                    menu,
                    MF_STRING,
                    ID_RESET_MINI,
                    wide("Reset window size").as_pcwstr(),
                );
            }
            let _ = AppendMenuW(
                menu,
                MF_STRING,
                ID_SETTINGS,
                wide("Settings...").as_pcwstr(),
            );
            let _ = AppendMenuW(menu, MF_STRING, ID_EXIT, wide("Exit").as_pcwstr());

            let mut pt = Default::default();
            let _ = GetCursorPos(&mut pt);

            let _ = SetForegroundWindow(self.hwnd);
            let _ = TrackPopupMenu(
                menu,
                TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
                pt.x,
                pt.y,
                None,
                self.hwnd,
                None,
            );
            let _ = PostMessageW(Some(self.hwnd), WM_NULL, WPARAM(0), LPARAM(0));
            let _ = DestroyMenu(menu);
        }
    }

    fn on_menu(&mut self, id: usize) {
        match id {
            ID_TOGGLE => self.handle_command(&Command::Toggle),
            ID_START_FOCUS => self.handle_command(&Command::StartFocus),
            ID_START_BREAK => self.handle_command(&Command::StartBreak),
            ID_STOP => self.handle_command(&Command::Stop),
            ID_SKIP => self.handle_command(&Command::Skip),
            ID_HIDE_MINI => {
                self.orch.config.display.mini_window = !self.orch.config.display.mini_window;
                let _ = self.orch.config.save();
                self.sync_mini_window();
            }
            ID_RESET_MINI => self.reset_mini_window(),
            ID_SETTINGS => self.open_settings(),
            ID_EXIT => unsafe {
                let _ = DestroyWindow(self.hwnd);
            },
            n if n >= ID_PRESET_BASE => {
                let i = n - ID_PRESET_BASE;
                if let Some(p) = self.orch.config.presets.get(i) {
                    let name = p.name.clone();
                    self.handle_command(&Command::Preset(name));
                }
            }
            _ => {}
        }
    }

    fn open_settings(&mut self) {
        if let Some(h) = self.settings_hwnd {
            unsafe {
                let _ = SetForegroundWindow(h);
            }
            return;
        }
        self.settings_hwnd = crate::ui::settings::open(self.hwnd, self as *mut App);
        self.refresh();
    }

    /// Called by the settings window when the user saves.
    pub fn apply_config(&mut self, new: Config) {
        self.orch.replace_config(new);
        let _ = self.orch.config.save();
        self.register_hotkeys();
        self.sync_mini_window();
        self.last_visual = None;
        self.refresh();
    }

    pub fn config(&self) -> &Config {
        &self.orch.config
    }

    pub fn on_settings_closed(&mut self) {
        self.settings_hwnd = None;
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let app = &mut *ptr;

    if msg == app.taskbar_created_msg {
        app.tray.forget();
        app.mute_tray.forget();
        app.mute_icon = None;
        app.last_visual = None;
        app.refresh();
        return LRESULT(0);
    }

    match msg {
        WM_TRAY => {
            let indicator = wparam.0 as u32 == MUTE_TRAY_ID;
            match lparam.0 as u32 {
                WM_LBUTTONUP if indicator => app.show_menu(),
                WM_LBUTTONUP => app.handle_command(&Command::Toggle),
                WM_RBUTTONUP | WM_CONTEXTMENU => app.show_menu(),
                WM_LBUTTONDBLCLK => app.open_settings(),
                _ => {}
            }
            LRESULT(0)
        }

        WM_COMMAND => {
            app.on_menu(wparam.0 & 0xFFFF);
            LRESULT(0)
        }

        WM_TIMER => {
            if wparam.0 == TIMER_ID {
                let events = app.orch.tick();
                app.apply_events(events);
            }
            LRESULT(0)
        }

        WM_HOTKEY => {
            match wparam.0 as i32 {
                HOTKEY_TOGGLE => app.handle_command(&Command::Toggle),
                HOTKEY_SKIP => app.handle_command(&Command::Skip),
                _ => {}
            }
            LRESULT(0)
        }

        WM_DND_REPORT => {
            let events = app.orch.collect_dnd_reports();
            app.apply_events(events);
            LRESULT(0)
        }

        WM_TIMECHANGE => {
            app.orch.note_time_change();
            LRESULT(0)
        }

        WM_POWERBROADCAST => {
            let ev = wparam.0 as u32;
            if ev == PBT_APMRESUMEAUTOMATIC || ev == PBT_APMRESUMESUSPEND {
                let events = app.orch.tick();
                app.apply_events(events);
            }
            LRESULT(1)
        }

        WM_COPYDATA => {
            let cds = lparam.0 as *const COPYDATASTRUCT;
            if !cds.is_null() && !(*cds).lpData.is_null() {
                let bytes =
                    std::slice::from_raw_parts((*cds).lpData as *const u8, (*cds).cbData as usize);
                if let Ok(text) = std::str::from_utf8(bytes) {
                    if let Some(cmd) = Command::decode(text) {
                        if matches!(cmd, Command::Quit) {
                            let _ = DestroyWindow(hwnd);
                        } else {
                            app.handle_command(&cmd);
                        }
                    }
                }
            }
            LRESULT(1)
        }

        WM_DESTROY => {
            app.mini_geometry_changed();
            session::save(&app.orch.timer);
            crate::audit::log_stop();
            app.tray.remove();
            app.mute_tray.remove();
            PostQuitMessage(0);
            LRESULT(0)
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
