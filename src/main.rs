//! taskbar-focus - a focus/Pomodoro timer that lives in the Windows tray.
//!
//! Module map:
//!
//! ```text
//!   timer         pure state machine (Idle / Focus / Break / Paused)
//!   config        TOML settings and presets in %APPDATA%
//!   session       restoring an interrupted interval after a restart
//!   clock         elapsed-time source, immune to clock changes and sleep
//!   cli           argument parsing, shared with the single-instance IPC
//!   orchestrator  the seam: timer effects -> OS actions
//!   os::dnd       Do Not Disturb control (see its module docs first!)
//!   os::sound     Windows scheme aliases for timer cues
//!   ui            tray icon, alerts, settings window, message pump
//! ```
//!
//! The dependency direction is strictly `ui -> orchestrator -> {timer, os}`;
//! `timer` and `config` never reach into Windows, which is what makes them
//! testable on any machine.

#![windows_subsystem = "windows"]

mod audit;
mod cli;
mod clock;
mod config;
mod explain;
mod orchestrator;
mod os;
mod session;
mod timer;
mod ui;

use cli::Command;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, HANDLE, HWND, LPARAM, WPARAM};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, SendMessageW, WM_COPYDATA};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = match cli::parse(&args) {
        Ok(c) => c,
        Err(e) => {
            report(&format!("taskbar-focus: {}\n\n{}", e.0, cli::HELP));
            std::process::exit(2);
        }
    };

    match command {
        Command::Help => {
            report(cli::HELP);
            return;
        }
        Command::Version => {
            report(&format!("taskbar-focus {}", env!("CARGO_PKG_VERSION")));
            return;
        }

        Command::Explain => {
            report(&explain::report());
            return;
        }
        _ => {}
    }

    let (already_running, _mutex) = acquire_single_instance();
    if already_running {
        forward_to_running_instance(&command);
        return;
    }

    let first_run = config::is_first_run();
    let config = config::Config::load();
    let timer = if config.behavior.restore_session_on_restart {
        session::restore().unwrap_or_default()
    } else {
        session::clear();
        timer::Timer::new()
    };

    audit::log_start(config.dnd.enabled);

    if let Some(outcome) = os::dnd::recover_if_needed() {
        audit::log(&format!(
            "recovery: previous run exited uncleanly with DND on -> {outcome:?}"
        ));
    }
    if let Err(e) = ui::app::run(config, timer, command, first_run) {
        report(&format!("taskbar-focus failed to start: {e}"));
        std::process::exit(1);
    }
}

/// Holds the single-instance mutex open for the lifetime of the process.
/// The handle is never read - owning it *is* the point, and Windows releases
/// it when we exit.
struct MutexGuard(#[allow(dead_code)] HANDLE);
unsafe impl Send for MutexGuard {}

fn acquire_single_instance() -> (bool, Option<MutexGuard>) {
    let name = ui::wide(ui::app::MUTEX_NAME);
    unsafe {
        match CreateMutexW(None, true, name.as_pcwstr()) {
            Ok(h) => {
                let existed = windows::Win32::Foundation::GetLastError() == ERROR_ALREADY_EXISTS;
                (existed, Some(MutexGuard(h)))
            }

            Err(_) => (false, None),
        }
    }
}

/// Hand the command to the instance that already owns the tray icon.
fn forward_to_running_instance(command: &Command) {
    let class = ui::wide(ui::app::CLASS_MAIN);
    let hwnd = unsafe { FindWindowW(class.as_pcwstr(), PCWSTR::null()) };
    let Ok(hwnd) = hwnd else {
        report("taskbar-focus is already running, but its window could not be found.");
        return;
    };
    if hwnd.is_invalid() {
        return;
    }
    if !command.is_remote_control() {
        return;
    }
    let payload = command.encode();
    let cds = COPYDATASTRUCT {
        dwData: 0,
        cbData: payload.len() as u32,
        lpData: payload.as_ptr() as *mut _,
    };
    unsafe {
        SendMessageW(
            hwnd,
            WM_COPYDATA,
            Some(WPARAM(0)),
            Some(LPARAM(&cds as *const _ as isize)),
        );
    }
}

/// Print to the terminal that launched us when there is one, otherwise show a
/// message box.
///
/// A release build is a GUI subsystem binary, so it has no console of its own
/// and `println!` would go nowhere. Attaching to the *parent* console makes
/// `--help` and error messages behave like a normal command-line tool when run
/// from a shell, while a double-clicked shortcut still gets a visible dialog.
fn report(text: &str) {
    if write_to_parent_console(text) {
        return;
    }
    let body = ui::wide(text);
    let title = ui::wide("taskbar-focus");
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::MessageBoxW(
            Some(HWND::default()),
            body.as_pcwstr(),
            title.as_pcwstr(),
            windows::Win32::UI::WindowsAndMessaging::MB_OK,
        );
    }
}

fn write_to_parent_console(text: &str) -> bool {
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_GENERIC_WRITE, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows::Win32::System::Console::{
        AttachConsole, FreeConsole, GetConsoleWindow, WriteConsoleW, ATTACH_PARENT_PROCESS,
    };
    unsafe {
        if !GetConsoleWindow().is_invalid() {
            println!("{text}");
            return true;
        }
        if AttachConsole(ATTACH_PARENT_PROCESS).is_err() {
            return false;
        }
        let name = ui::wide("CONOUT$");
        let handle = CreateFileW(
            name.as_pcwstr(),
            FILE_GENERIC_WRITE.0,
            FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            Default::default(),
            None,
        );
        let ok = match handle {
            Ok(h) if !h.is_invalid() => {
                let wide: Vec<u16> = format!("{text}\r\n").encode_utf16().collect();
                let mut written = 0u32;
                let r = WriteConsoleW(h, &wide, Some(&mut written), None).is_ok();
                let _ = windows::Win32::Foundation::CloseHandle(h);
                r
            }
            _ => false,
        };
        let _ = FreeConsole();
        ok
    }
}
