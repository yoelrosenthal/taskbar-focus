//! `taskbar-focus --explain`: a self-report of everything the program touches.
//!
//! Intended for the user, and for whoever reviews an endpoint-security
//! detection. It inspects the *actual machine* rather than printing a canned
//! blurb, so the registry paths shown are the real ones that would be written.
//!
//! It is read-only: running `--explain` changes nothing.

use crate::config;
use crate::os::dnd::{cloudstore, profile, wnf};

pub fn report() -> String {
    let mut s = String::new();
    let v = env!("CARGO_PKG_VERSION");
    s.push_str(&format!(
        "taskbar-focus {v} - what this program does to your system\n\
         =========================================================\n\n"
    ));

    s.push_str(
        "SUMMARY\n  \
         A Pomodoro/focus timer that lives in the notification area. It can turn\n  \
         Windows \"Do not disturb\" on during focus sessions and off during breaks.\n  \
         It needs no administrator rights, makes no network connections, and\n  \
         collects no telemetry.\n\n",
    );

    s.push_str("FILES IT WRITES (only under your own profile)\n");
    match config::app_dir() {
        Some(d) => {
            s.push_str(&format!(
                "  {}\\config.toml     settings and presets\n",
                d.display()
            ));
            s.push_str(&format!(
                "  {}\\session.toml    the interval in progress\n",
                d.display()
            ));
            s.push_str(&format!(
                "  {}\\activity.log    log of every change it makes\n",
                d.display()
            ));
        }
        None => s.push_str("  (could not resolve %APPDATA%)\n"),
    }
    s.push('\n');

    let cfg = config::Config::load();
    s.push_str(&format!(
        "DO NOT DISTURB INTEGRATION: currently {}\n",
        if cfg.dnd.enabled {
            "ENABLED"
        } else {
            "DISABLED"
        }
    ));
    if !cfg.dnd.enabled {
        s.push_str(
            "  With this disabled the program makes none of the registry writes\n  \
             below and controls no services. It is then just a timer with a tray\n  \
             icon - apart from the read-only state query described under MUTED\n  \
             INDICATOR, which only happens if you switched an indicator on.\n\n",
        );
    } else {
        s.push_str(
            "  Windows has no supported API to set Do Not Disturb (the documented\n  \
             one is a Limited Access Feature needing a Microsoft-issued token), so\n  \
             this uses the mechanism admin scripts use. Three steps:\n\n",
        );

        s.push_str("  1. REGISTRY WRITE - swaps the active quiet-hours profile name\n");
        let targets = cloudstore::discover();
        if targets.is_empty() {
            s.push_str("     (no quiet-hours settings found on this machine)\n");
        } else {
            for t in &targets {
                s.push_str(&format!("     HKCU\\{}\n", t.path));
                s.push_str(&format!(
                    "       value \"Data\", currently {:?}\n",
                    profile::read(&t.blob).unwrap_or_else(|| "?".into())
                ));
            }
            s.push_str(&format!(
                "     The only edit made is replacing that name with\n       {:?}\n     \
                 and back again. Both names are the same length, so no other byte\n     \
                 of the value changes. The original bytes are restored afterwards.\n",
                profile::PRIORITY_ONLY
            ));
        }
        s.push('\n');

        s.push_str(
            "  2. SERVICE RESTART - WpnUserService_<id> (Windows Push Notifications\n     \
             User Service). This is a PER-USER service instance owned by you, and\n     \
             restarting it needs no elevation. Without this step the registry\n     \
             change has no effect, because the service caches the setting.\n\n",
        );

        s.push_str(
            "  3. VERIFICATION - reads the resulting state back via\n     \
             NtQueryWnfStateData in ntdll.dll, to confirm the change actually\n     \
             applied rather than assuming it did. This is a READ-ONLY query and\n     \
             is the only undocumented API the program calls.\n\n",
        );

        s.push_str(
            "  Note: notifications are genuinely suppressed, but Windows' own\n  \
             indicator beside the clock does not appear, because the taskbar is\n  \
             drawn by Explorer and it caches that state. Cosmetic only, and the\n  \
             mark this app draws for itself covers it; see below.\n\n",
        );
        s.push_str(&format!(
            "  Live state right now: {}\n\n",
            match wnf::query() {
                Some(st) => format!("{st:?}"),
                None => "could not be read on this build".into(),
            }
        ));
    }

    let on_off = |enabled| if enabled { "ON" } else { "OFF" };
    s.push_str(&format!(
        "MUTED INDICATOR\n  \
         Separate bell beside the clock:   {}\n  \
         Bell in the compact timer window: {}\n",
        on_off(cfg.dnd.mute_tray_icon),
        on_off(cfg.dnd.mute_window)
    ));
    if cfg.dnd.wants_indicator() {
        s.push_str(
            "  Because Windows will not show an indicator for a change it did not\n  \
             make itself, the program draws one: the same crossed-out bell the\n  \
             shell uses. To know when to draw it, it reads the effective state\n  \
             every two seconds using the same READ-ONLY NtQueryWnfStateData call\n  \
             described above. Nothing is written, and the mark follows Do Not\n  \
             Disturb however it was switched on - including by you, in Windows'\n  \
             own settings. The timer's own tray icon is left alone: at sixteen\n  \
             pixels it can show progress or a bell, not both.\n\n",
        );
    } else {
        s.push_str("  Both are off, so no state is read at all for this.\n\n");
    }

    s.push_str(
        "WHAT IT NEVER DOES\n  \
         - No network access of any kind. No telemetry, opt-in or otherwise.\n  \
         - No elevation; the manifest requests asInvoker.\n  \
         - No installation, no services created, no scheduled tasks, no autostart\n    \
           entries, no changes outside HKCU and your own %APPDATA%.\n  \
         - Does not read, modify or transmit your notification content, nor your\n    \
           Windows priority-app or priority-contact list.\n  \
         - No code injection, no hooking, no driver, no child processes.\n\n",
    );

    s.push_str(
        "OTHER SYSTEM INTERACTION\n  \
         - Registers global hotkeys (default Ctrl+Alt+F / Ctrl+Alt+B) via\n    \
           RegisterHotKey. It does not install a keyboard hook and cannot see\n    \
           any other keystrokes.\n  \
         - Adds a notification-area icon and shows notifications through it,\n    \
           plus a second one while notifications are muted.\n\n",
    );

    s.push_str(
        "SOURCE\n  \
         Every action above is in src/os/dnd/ and documented there.\n  \
         Build it yourself with: cargo build --release\n",
    );

    s
}
