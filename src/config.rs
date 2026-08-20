//! Human-readable configuration, persisted as TOML.
//!
//! Location: `%APPDATA%\taskbar-focus\config.toml` (resolved via
//! `SHGetKnownFolderPath(FOLDERID_RoamingAppData)`, never hardcoded), so the
//! file is easy to back up, diff or sync.
//!
//! Every field carries a `#[serde(default)]`, which means a hand-edited file
//! that is missing keys - or was written by an older version - still loads.
//! Unknown keys are preserved-by-ignoring rather than being an error, so a
//! newer config does not break an older binary.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

use crate::timer::{Phase, Plan};

pub const APP_DIR: &str = "taskbar-focus";
pub const CONFIG_FILE: &str = "config.toml";

/// A named set of durations the user can switch between.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Preset {
    pub name: String,
    pub focus_minutes: f64,
    pub short_break_minutes: f64,
    pub long_break_minutes: f64,
    #[serde(default = "default_sessions")]
    pub sessions_before_long_break: u32,
}

fn default_sessions() -> u32 {
    4
}

impl Preset {
    pub fn duration_of(&self, phase: Phase) -> Duration {
        let mins = match phase {
            Phase::Focus => self.focus_minutes,
            Phase::ShortBreak => self.short_break_minutes,
            Phase::LongBreak => self.long_break_minutes,
        };

        Duration::from_secs_f64(if mins.is_finite() && mins > 0.0 {
            mins * 60.0
        } else {
            60.0
        })
    }
}

fn default_presets() -> Vec<Preset> {
    vec![
        Preset {
            name: "Pomodoro 25/5".into(),
            focus_minutes: 25.0,
            short_break_minutes: 5.0,
            long_break_minutes: 15.0,
            sessions_before_long_break: 4,
        },
        Preset {
            name: "Deep Work 90/15".into(),
            focus_minutes: 90.0,
            short_break_minutes: 15.0,
            long_break_minutes: 30.0,
            sessions_before_long_break: 2,
        },
        Preset {
            name: "Short 15/3".into(),
            focus_minutes: 15.0,
            short_break_minutes: 3.0,
            long_break_minutes: 10.0,
            sessions_before_long_break: 4,
        },
    ]
}

/// What to do with a running timer when the machine wakes from sleep.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum WakePolicy {
    /// Subtract the time spent asleep - a 25 minute session started before a
    /// 2 hour nap is simply over.
    #[default]
    CountSleep,
    /// Pretend the machine never slept and keep the remaining time.
    IgnoreSleep,
    /// Pause and let the user decide.
    Pause,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Behavior {
    pub sequence_enabled: bool,
    pub auto_start_break: bool,
    pub auto_start_focus: bool,
    /// After a long break ends, start a new focus session (another cycle).
    pub repeat_cycles: bool,
    /// Require confirmation before stopping or skipping a *focus* session.
    pub strict_focus: bool,
    /// Re-arm an interrupted session when the app restarts.
    pub restore_session_on_restart: bool,
    pub wake_policy: WakePolicy,
}

impl Default for Behavior {
    fn default() -> Self {
        Self {
            sequence_enabled: true,
            auto_start_break: true,
            auto_start_focus: true,
            repeat_cycles: true,
            strict_focus: false,
            restore_session_on_restart: false,
            wake_policy: WakePolicy::IgnoreSleep,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct DndSettings {
    /// Turn Do Not Disturb on for focus and off again for breaks.
    ///
    /// "On" means the `PriorityOnly` quiet-hours profile - exactly what the
    /// Windows UI calls "Do not disturb" - so the user's own priority apps and
    /// contacts still get through. There is deliberately no profile choice
    /// here; see `os::dnd::profile` for why.
    /// Restoring the previous state afterwards, and restarting the notification
    /// service so Windows notices the change, are not choices: the first is
    /// simply correct, and the second is what makes the feature work at all.
    /// Offering them as switches invited people to turn the feature off without
    /// realising it.
    pub enabled: bool,

    /// Leave Do Not Disturb on through short breaks; only a long break releases
    /// it. Off by default, so any break unmutes.
    pub keep_on_short_break: bool,

    /// Show the mark as a separate notification-area icon while muted, as
    /// close to Windows' own indicator as an ordinary program can get: its own
    /// icon beside the clock, present only while muted.
    ///
    /// The timer tray icon remains dedicated to progress because it cannot show
    /// both clearly at sixteen pixels; see `ui::icon`.
    pub mute_tray_icon: bool,

    /// Draw the mark beside the countdown in the compact timer window.
    pub mute_window: bool,
}

impl Default for DndSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            keep_on_short_break: false,
            mute_tray_icon: true,
            mute_window: true,
        }
    }
}

impl DndSettings {
    /// Whether any muted-state indicator is enabled.
    pub fn wants_indicator(&self) -> bool {
        self.mute_tray_icon || self.mute_window
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Display {
    /// Show the compact timer window: a small, resizable, always-visible
    /// readout of the countdown. More reliable than the taskbar title trick,
    /// and it can be shrunk to roughly the height of a taskbar button.
    pub mini_window: bool,

    /// Keep the compact window above other windows.
    pub always_on_top: bool,

    /// Saved position and size of the compact window: `[x, y, width, height]`.
    /// Absent until the user has moved or resized it.
    pub mini_geometry: Option<[i32; 4]>,
}

impl Default for Display {
    fn default() -> Self {
        Self {
            mini_window: true,
            // On by default. The Windows taskbar is itself a top-most window,
            // so without this the compact window sinks behind it and cannot be
            // parked over the taskbar at all.
            always_on_top: true,
            mini_geometry: None,
        }
    }
}

/// Per-event toggles shared by notifications and sounds.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct EventToggles {
    pub focus_start: bool,
    pub focus_end: bool,
    pub break_start: bool,
    pub break_end: bool,
}

impl Default for EventToggles {
    fn default() -> Self {
        Self {
            focus_start: true,
            focus_end: true,
            break_start: true,
            break_end: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Notifications {
    pub enabled: bool,
    pub events: EventToggles,
}

impl Default for Notifications {
    fn default() -> Self {
        Self {
            enabled: true,
            events: EventToggles::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Sounds {
    /// Global mute, independent of the per-event switches.
    pub muted: bool,
    pub events: EventToggles,
}

/// Duration presets for the screen flash.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum LengthPreset {
    Short,
    #[default]
    Normal,
    Long,
    ExtraLong,
}

impl LengthPreset {
    pub fn flash_pulses(self) -> u32 {
        self.flash_timing().0
    }

    pub fn flash_ms(self) -> u32 {
        self.flash_timing().1
    }

    fn flash_timing(self) -> (u32, u32) {
        match self {
            Self::Short => (2, 110),
            Self::Normal => (3, 160),
            Self::Long => (5, 240),
            Self::ExtraLong => (8, 320),
        }
    }

    pub fn index(self) -> usize {
        match self {
            Self::Short => 0,
            Self::Normal => 1,
            Self::Long => 2,
            Self::ExtraLong => 3,
        }
    }

    pub fn from_index(index: usize) -> Self {
        match index {
            0 => Self::Short,
            2 => Self::Long,
            3 => Self::ExtraLong,
            _ => Self::Normal,
        }
    }
}

/// How to draw attention when a focus or break interval completes.
///
/// Each channel is independent, so toast, overlay and flash can be combined.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Alerts {
    /// Show an Action Centre tray toast.
    pub toast: bool,
    /// Show a large centred card on the active monitor.
    pub overlay: bool,
    /// Pulse a brief colour flash across every monitor.
    pub flash: bool,
    /// Keep the overlay until the user clicks Dismiss, and pause the timer
    /// while it is open.
    pub require_dismiss: bool,
    /// Seconds before the overlay closes itself when `require_dismiss` is off.
    pub auto_dismiss_secs: u32,
    /// How many pulses, and how long each one lasts.
    pub flash_length: LengthPreset,
}

impl Default for Alerts {
    fn default() -> Self {
        Self {
            toast: true,
            overlay: false,
            flash: false,
            require_dismiss: false,
            auto_dismiss_secs: 8,
            flash_length: LengthPreset::default(),
        }
    }
}

impl Alerts {
    pub fn wants_visual(&self) -> bool {
        self.overlay || self.flash
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Hotkeys {
    pub enabled: bool,
    /// Accelerator strings like "Ctrl+Alt+F"; empty disables that binding.
    pub toggle: String,
    pub skip: String,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            enabled: true,
            toggle: "Ctrl+Alt+F".into(),
            skip: "Ctrl+Alt+B".into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub active_preset: String,
    pub presets: Vec<Preset>,
    pub behavior: Behavior,
    pub dnd: DndSettings,
    pub display: Display,
    pub notifications: Notifications,
    pub alerts: Alerts,
    pub sounds: Sounds,
    pub hotkeys: Hotkeys,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            active_preset: "Pomodoro 25/5".into(),
            presets: default_presets(),
            behavior: Behavior::default(),
            dnd: DndSettings::default(),
            display: Display::default(),
            notifications: Notifications::default(),
            alerts: Alerts::default(),
            sounds: Sounds::default(),
            hotkeys: Hotkeys::default(),
        }
    }
}

impl Config {
    /// The preset named by `active_preset`, falling back to the first one and
    /// finally to a built-in, so a bad name in the file can never panic.
    pub fn preset(&self) -> Preset {
        self.presets
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(&self.active_preset))
            .or_else(|| self.presets.first())
            .cloned()
            .unwrap_or_else(|| default_presets().remove(0))
    }

    /// Project the config into the pure timer's view of the world.
    pub fn plan(&self) -> Plan {
        let p = self.preset();
        Plan {
            focus: p.duration_of(Phase::Focus),
            short_break: p.duration_of(Phase::ShortBreak),
            long_break: p.duration_of(Phase::LongBreak),
            sessions_before_long_break: p.sessions_before_long_break.max(1),
            sequence_enabled: self.behavior.sequence_enabled,
            auto_start_break: self.behavior.auto_start_break,
            auto_start_focus: self.behavior.auto_start_focus,
            repeat_cycles: self.behavior.repeat_cycles,
        }
    }

    pub fn select_preset(&mut self, name: &str) -> bool {
        if let Some(p) = self
            .presets
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(name))
        {
            self.active_preset = p.name.clone();
            true
        } else {
            false
        }
    }

    /// Load from disk, falling back to defaults when the file is absent or
    /// unparseable. A corrupt file is renamed rather than deleted so the user
    /// can recover whatever was in it.
    pub fn load() -> Self {
        let path = match config_path() {
            Some(p) => p,
            None => return Config::default(),
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => return Config::default(),
        };
        match toml::from_str::<Config>(&text) {
            Ok(mut c) => {
                if c.presets.is_empty() {
                    c.presets = default_presets();
                }
                c
            }
            Err(e) => {
                eprintln!(
                    "taskbar-focus: {} is invalid ({e}); using defaults",
                    path.display()
                );
                let _ = std::fs::rename(&path, path.with_extension("toml.bad"));
                Config::default()
            }
        }
    }

    /// Save atomically: write a sibling temp file then rename over the target,
    /// so a crash mid-write cannot leave a truncated config behind.
    pub fn save(&self) -> std::io::Result<()> {
        let path = config_path().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "no config directory")
        })?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let body = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let text = format!(
            "# taskbar-focus configuration.\n\
             # Edit freely - unknown or missing keys fall back to defaults.\n\
             # Durations are in minutes and may be fractional.\n\n{body}"
        );
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &path)
    }
}

/// `%APPDATA%\taskbar-focus`, via the documented known-folder API.
pub fn app_dir() -> Option<PathBuf> {
    roaming_appdata().map(|d| d.join(APP_DIR))
}

pub fn config_path() -> Option<PathBuf> {
    app_dir().map(|d| d.join(CONFIG_FILE))
}

/// Has this user ever run the app before?
///
/// Used to make the first launch visible: a tray-only app that opens no window
/// is indistinguishable from one that failed to start, especially on Windows 11
/// where new tray icons are hidden in the overflow by default.
pub fn is_first_run() -> bool {
    !config_path().is_some_and(|p| p.exists())
}

#[cfg(windows)]
fn roaming_appdata() -> Option<PathBuf> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{
        FOLDERID_RoamingAppData, SHGetKnownFolderPath, KF_FLAG_DEFAULT,
    };
    unsafe {
        let pw = SHGetKnownFolderPath(&FOLDERID_RoamingAppData, KF_FLAG_DEFAULT, None).ok()?;
        let s = pw.to_string().ok()?;
        CoTaskMemFree(Some(pw.0 as *const _));
        Some(PathBuf::from(s))
    }
}

#[cfg(not(windows))]
fn roaming_appdata() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_file_fills_in_defaults() {
        let text = r#"
            active_preset = "Deep Work 90/15"
            [behavior]
            strict_focus = true
        "#;
        let c: Config = toml::from_str(text).unwrap();
        assert!(c.behavior.strict_focus);
        assert!(
            c.behavior.sequence_enabled,
            "unspecified keys keep defaults"
        );
        assert!(c.dnd.enabled);

        assert_eq!(c.presets, default_presets());
        assert_eq!(c.plan().focus, Duration::from_secs(90 * 60));
    }

    #[test]
    fn nonsense_durations_are_clamped() {
        let p = Preset {
            name: "bad".into(),
            focus_minutes: -5.0,
            short_break_minutes: f64::NAN,
            long_break_minutes: 0.0,
            sessions_before_long_break: 0,
        };
        for ph in [Phase::Focus, Phase::ShortBreak, Phase::LongBreak] {
            assert_eq!(p.duration_of(ph), Duration::from_secs(60));
        }
    }
}
