//! Deciding how much time really passed between two ticks.
//!
//! Three things can go wrong with "just subtract the timestamps":
//!
//! * **The user changes the system clock.** Wall-clock time jumps, but no time
//!   actually passed. Using a monotonic clock makes us immune.
//! * **The machine sleeps.** On Windows the monotonic clock does not advance
//!   across a true suspend, so a 25 minute timer would survive an 8 hour nap
//!   with its remaining time untouched - surprising, and rarely wanted.
//! * **Modern standby**, where the monotonic clock *does* keep running.
//!
//! So we track both clocks and reconcile them: the monotonic delta is the
//! truth, and a wall-clock delta that is meaningfully *larger* is taken as
//! evidence of a suspend and handled per the user's [`WakePolicy`].

use crate::config::WakePolicy;
use std::time::{Duration, Instant, SystemTime};

/// A wall-clock/monotonic gap smaller than this is just scheduling jitter.
const SLEEP_DETECTION_SLACK: Duration = Duration::from_secs(3);

pub struct Clock {
    last_mono: Instant,
    last_wall: SystemTime,
    /// Set by `WM_TIMECHANGE`: the next wall-clock delta is meaningless.
    wall_is_suspect: bool,
}

/// How much time to charge against the running interval, and why.
#[derive(Debug, PartialEq)]
pub struct Elapsed {
    pub delta: Duration,
    /// The machine appears to have been suspended for this long.
    pub slept: Option<Duration>,
}

impl Default for Clock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock {
    pub fn new() -> Self {
        Clock {
            last_mono: Instant::now(),
            last_wall: SystemTime::now(),
            wall_is_suspect: false,
        }
    }

    /// Called on `WM_TIMECHANGE`, so a deliberate clock change is not mistaken
    /// for a suspend.
    pub fn note_time_change(&mut self) {
        self.wall_is_suspect = true;
    }

    /// Advance to now and report the elapsed time under `policy`.
    pub fn tick(&mut self, policy: WakePolicy) -> Elapsed {
        self.tick_at(Instant::now(), SystemTime::now(), policy)
    }

    /// Testable core of [`tick`].
    pub fn tick_at(&mut self, mono: Instant, wall: SystemTime, policy: WakePolicy) -> Elapsed {
        let mono_delta = mono.saturating_duration_since(self.last_mono);

        let wall_delta = wall
            .duration_since(self.last_wall)
            .unwrap_or(Duration::ZERO);

        self.last_mono = mono;
        self.last_wall = wall;

        if std::mem::take(&mut self.wall_is_suspect) {
            return Elapsed {
                delta: mono_delta,
                slept: None,
            };
        }

        let gap = wall_delta.saturating_sub(mono_delta);
        if gap <= SLEEP_DETECTION_SLACK {
            return Elapsed {
                delta: mono_delta,
                slept: None,
            };
        }

        let delta = match policy {
            WakePolicy::CountSleep => wall_delta,
            WakePolicy::IgnoreSleep => mono_delta,

            WakePolicy::Pause => mono_delta,
        };
        Elapsed {
            delta,
            slept: Some(gap),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock_at(mono: Instant, wall: SystemTime) -> Clock {
        Clock {
            last_mono: mono,
            last_wall: wall,
            wall_is_suspect: false,
        }
    }

    #[test]
    fn clock_moved_forward_does_not_steal_time() {
        let m = Instant::now();
        let w = SystemTime::now();
        let mut c = clock_at(m, w);
        c.note_time_change();
        let e = c.tick_at(
            m + Duration::from_secs(1),
            w + Duration::from_secs(3601),
            WakePolicy::CountSleep,
        );
        assert_eq!(e.delta, Duration::from_secs(1));
        assert_eq!(e.slept, None);
    }

    #[test]
    fn suspend_is_detected_and_policy_respected() {
        let m = Instant::now();
        let w = SystemTime::now();

        let next_m = m + Duration::from_secs(1);
        let next_w = w + Duration::from_secs(7200);

        let mut c = clock_at(m, w);
        let e = c.tick_at(next_m, next_w, WakePolicy::CountSleep);
        assert_eq!(e.delta, Duration::from_secs(7200));
        assert!(e.slept.is_some());

        let mut c = clock_at(m, w);
        let e = c.tick_at(next_m, next_w, WakePolicy::IgnoreSleep);
        assert_eq!(e.delta, Duration::from_secs(1));
        assert!(e.slept.is_some(), "still reported, just not charged");

        let mut c = clock_at(m, w);
        let e = c.tick_at(next_m, next_w, WakePolicy::Pause);
        assert_eq!(e.delta, Duration::from_secs(1));
        assert!(e.slept.is_some());
    }
}
