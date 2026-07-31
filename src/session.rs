//! Remembering an in-flight session across an app restart or a crash.
//!
//! Stored next to the config as `session.toml`. The file records wall-clock
//! time so we can tell how stale it is; a session that has been sitting around
//! for longer than the interval had left is simply dropped rather than
//! resurrected into something nonsensical.

use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::timer::{Phase, State, Timer};

const FILE: &str = "session.toml";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SavedPhase {
    Focus,
    ShortBreak,
    LongBreak,
}

impl From<Phase> for SavedPhase {
    fn from(p: Phase) -> Self {
        match p {
            Phase::Focus => SavedPhase::Focus,
            Phase::ShortBreak => SavedPhase::ShortBreak,
            Phase::LongBreak => SavedPhase::LongBreak,
        }
    }
}

impl From<SavedPhase> for Phase {
    fn from(p: SavedPhase) -> Self {
        match p {
            SavedPhase::Focus => Phase::Focus,
            SavedPhase::ShortBreak => Phase::ShortBreak,
            SavedPhase::LongBreak => Phase::LongBreak,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedSession {
    pub phase: SavedPhase,
    pub total_secs: f64,
    pub remaining_secs: f64,
    pub running: bool,
    pub completed_focus: u32,
    /// Unix seconds at the moment of saving.
    pub saved_at: u64,
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn path() -> Option<std::path::PathBuf> {
    crate::config::app_dir().map(|d| d.join(FILE))
}

/// Persist the current timer, or clear the file when idle.
pub fn save(timer: &Timer) {
    let Some(p) = path() else { return };
    let saved = match timer.state() {
        State::Idle => {
            let _ = std::fs::remove_file(&p);
            return;
        }
        State::Running {
            phase,
            total,
            remaining,
        } => SavedSession {
            phase: (*phase).into(),
            total_secs: total.as_secs_f64(),
            remaining_secs: remaining.as_secs_f64(),
            running: true,
            completed_focus: timer.completed_focus(),
            saved_at: now_unix(),
        },
        State::Paused {
            phase,
            total,
            remaining,
        } => SavedSession {
            phase: (*phase).into(),
            total_secs: total.as_secs_f64(),
            remaining_secs: remaining.as_secs_f64(),
            running: false,
            completed_focus: timer.completed_focus(),
            saved_at: now_unix(),
        },
    };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = toml::to_string_pretty(&saved) {
        let _ = std::fs::write(&p, text);
    }
}

pub fn clear() {
    if let Some(p) = path() {
        let _ = std::fs::remove_file(p);
    }
}

/// Rebuild a timer from disk, if there is a session worth restoring.
///
/// A restored session always comes back **paused**, whatever it was doing when
/// the app stopped: silently resuming a focus session (and re-muting the
/// machine) behind the user's back would be the wrong default.
pub fn restore() -> Option<Timer> {
    let raw = std::fs::read_to_string(path()?).ok()?;
    let s: SavedSession = toml::from_str(&raw).ok()?;
    Some(rebuild(&s, now_unix()))
}

/// Testable core of [`restore`].
pub fn rebuild(s: &SavedSession, now: u64) -> Timer {
    let away = Duration::from_secs(now.saturating_sub(s.saved_at));
    let remaining = Duration::from_secs_f64(s.remaining_secs.max(0.0));

    let left = if s.running {
        remaining.saturating_sub(away)
    } else {
        remaining
    };
    if left.is_zero() {
        return Timer::restore(State::Idle, s.completed_focus);
    }
    Timer::restore(
        State::Paused {
            phase: s.phase.into(),
            total: Duration::from_secs_f64(s.total_secs.max(1.0)),
            remaining: left,
        },
        s.completed_focus,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved(running: bool, remaining: f64, saved_at: u64) -> SavedSession {
        SavedSession {
            phase: SavedPhase::Focus,
            total_secs: 1500.0,
            remaining_secs: remaining,
            running,
            completed_focus: 3,
            saved_at,
        }
    }

    #[test]
    fn running_session_loses_the_time_the_app_was_closed() {
        let t = rebuild(&saved(true, 600.0, 1000), 1300);
        assert_eq!(t.state().remaining(), Some(Duration::from_secs(300)));
        assert!(
            matches!(t.state(), State::Paused { .. }),
            "never auto-resumes"
        );
        assert_eq!(t.completed_focus(), 3);
    }

    #[test]
    fn stale_session_is_dropped_but_cadence_survives() {
        let t = rebuild(&saved(true, 600.0, 1000), 100_000);
        assert_eq!(*t.state(), State::Idle);
        assert_eq!(t.completed_focus(), 3, "long-break cadence is preserved");
    }
}
