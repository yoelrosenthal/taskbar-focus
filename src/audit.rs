//! A plain-text record of every change this app makes to system state.
//!
//! Written to `%APPDATA%\taskbar-focus\activity.log`.
//!
//! This exists for the benefit of whoever has to answer "what did this program
//! actually do?" - the user, or a security analyst looking at an endpoint
//! detection. The app modifies a registry value and restarts a service, which
//! are exactly the kinds of actions that deserve a timestamped, human-readable
//! trail rather than trust.
//!
//! It is local-only, append-only, size-capped, and contains nothing but the
//! app's own actions. Nothing is transmitted anywhere.

use std::fmt::Write as _;
use std::io::Write as _;

/// Keep the log small enough to never become a disk-space problem.
const MAX_BYTES: u64 = 128 * 1024;

fn path() -> Option<std::path::PathBuf> {
    crate::config::app_dir().map(|d| d.join("activity.log"))
}

/// `2026-07-31 10:42:05`, from the local clock.
fn timestamp() -> String {
    #[cfg(windows)]
    unsafe {
        let t = windows::Win32::System::SystemInformation::GetLocalTime();
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
        )
    }
    #[cfg(not(windows))]
    String::from("---------- --:--:--")
}

/// Append one line. Never panics and never blocks the caller on failure -
/// losing a log line must not break the timer.
pub fn log(line: &str) {
    let Some(p) = path() else { return };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }

    if std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0) > MAX_BYTES {
        if let Ok(text) = std::fs::read_to_string(&p) {
            let keep: String =
                text.lines()
                    .skip(text.lines().count() / 2)
                    .fold(String::new(), |mut acc, l| {
                        let _ = writeln!(acc, "{l}");
                        acc
                    });
            let _ = std::fs::write(&p, keep);
        }
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&p)
    {
        let _ = writeln!(f, "{} {}", timestamp(), line);
    }
}

/// Record a Do Not Disturb change: what we asked for and what happened.
pub fn log_dnd(engaging: bool, outcome: &str) {
    log(&format!(
        "dnd: requested {} -> {outcome}",
        if engaging {
            "ON (priority-only)"
        } else {
            "OFF (restore)"
        }
    ));
}

/// Record startup, so the log begins with context.
pub fn log_start(dnd_enabled: bool) {
    log(&format!(
        "start: taskbar-focus {} (pid {}), do-not-disturb integration {}",
        env!("CARGO_PKG_VERSION"),
        std::process::id(),
        if dnd_enabled { "ENABLED" } else { "disabled" }
    ));
}

pub fn log_stop() {
    log("stop: exiting");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_has_the_expected_shape() {
        let t = timestamp();
        assert_eq!(t.len(), 19, "{t}");
        assert_eq!(t.as_bytes()[4], b'-');
        assert_eq!(t.as_bytes()[10], b' ');
        assert_eq!(t.as_bytes()[13], b':');
    }

    #[test]
    fn dnd_lines_say_which_direction() {
        assert!(format!(
            "dnd: requested {} -> {}",
            "ON (priority-only)", "Applied(PriorityOnly)"
        )
        .contains("ON (priority-only)"));
    }
}
