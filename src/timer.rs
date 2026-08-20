//! The focus/break timer state machine.
//!
//! This module is deliberately **pure**: it knows nothing about Windows, the
//! tray, notifications or Do Not Disturb. It takes commands in and returns a
//! list of [`Effect`]s describing what just happened. The caller (see
//! `orchestrator`) decides what those effects mean for the OS.
//!
//! Keeping it pure buys two things:
//!   * it is exhaustively unit-testable without a desktop session, and
//!   * time is injected, so sleep/wake and wall-clock changes are handled by
//!     the caller feeding a different elapsed duration rather than by sprinkling
//!     clock logic through the state machine.

use std::time::Duration;

/// Which kind of interval is running.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Focus,
    ShortBreak,
    LongBreak,
}

impl Phase {
    pub fn is_break(self) -> bool {
        matches!(self, Phase::ShortBreak | Phase::LongBreak)
    }

    pub fn label(self) -> &'static str {
        match self {
            Phase::Focus => "Focus",
            Phase::ShortBreak => "Short break",
            Phase::LongBreak => "Long break",
        }
    }
}

/// The complete timer state. `Idle` means nothing is scheduled.
#[derive(Clone, PartialEq, Debug)]
pub enum State {
    Idle,
    Running {
        phase: Phase,
        total: Duration,
        remaining: Duration,
    },
    Paused {
        phase: Phase,
        total: Duration,
        remaining: Duration,
    },
}

impl State {
    pub fn phase(&self) -> Option<Phase> {
        match self {
            State::Idle => None,
            State::Running { phase, .. } | State::Paused { phase, .. } => Some(*phase),
        }
    }

    pub fn remaining(&self) -> Option<Duration> {
        match self {
            State::Idle => None,
            State::Running { remaining, .. } | State::Paused { remaining, .. } => Some(*remaining),
        }
    }

    /// Fraction of the interval already elapsed, in `0.0..=1.0`.
    /// Used to draw the tray progress ring.
    pub fn progress(&self) -> f32 {
        match self {
            State::Idle => 0.0,
            State::Running {
                total, remaining, ..
            }
            | State::Paused {
                total, remaining, ..
            } => {
                let t = total.as_secs_f32();
                if t <= 0.0 {
                    return 1.0;
                }
                (1.0 - remaining.as_secs_f32() / t).clamp(0.0, 1.0)
            }
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(self, State::Running { .. })
    }
}

/// Durations and sequencing rules, supplied by the caller on every update so
/// that settings changes take effect immediately without restarting the timer.
#[derive(Clone, Copy, Debug)]
pub struct Plan {
    pub focus: Duration,
    pub short_break: Duration,
    pub long_break: Duration,
    /// How many focus sessions before a long break (`0`/`1` => every break long).
    pub sessions_before_long_break: u32,
    /// Master switch for automatic Pomodoro sequencing.
    pub sequence_enabled: bool,
    /// After a focus session ends, start the break automatically.
    pub auto_start_break: bool,
    /// After a break ends, start the next focus session automatically.
    pub auto_start_focus: bool,
    /// After a long break ends, start a new cycle even if `auto_start_focus` is off.
    pub repeat_cycles: bool,
}

impl Plan {
    pub fn duration_of(&self, phase: Phase) -> Duration {
        match phase {
            Phase::Focus => self.focus,
            Phase::ShortBreak => self.short_break,
            Phase::LongBreak => self.long_break,
        }
    }
}

/// Commands fed into the machine.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cmd {
    /// Start a specific phase, replacing whatever is running.
    Start(Phase),
    /// Left-click / hotkey behaviour: idle -> start focus, running -> pause,
    /// paused -> resume.
    Toggle,
    Pause,
    Resume,
    /// Abandon the current interval and go idle.
    Stop,
    /// End the current interval early and move to whatever comes next.
    Skip,
    /// Advance time. The caller supplies elapsed time from a *monotonic* clock.
    Tick(Duration),
}

/// Things that happened as a result of a command, for the caller to act on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Effect {
    Started(Phase),
    /// `completed` is false when the interval was skipped or stopped early.
    Ended {
        phase: Phase,
        completed: bool,
    },
    Paused(Phase),
    Resumed(Phase),
    /// Emitted when the machine settles in `Idle` after an interval finished.
    WentIdle,
}

/// The timer itself.
#[derive(Clone, Debug)]
pub struct Timer {
    state: State,
    /// Completed focus sessions since the last long break; drives the cadence.
    completed_focus: u32,
}

impl Default for Timer {
    fn default() -> Self {
        Self::new()
    }
}

impl Timer {
    pub fn new() -> Self {
        Timer {
            state: State::Idle,
            completed_focus: 0,
        }
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn completed_focus(&self) -> u32 {
        self.completed_focus
    }

    /// Restore a previously persisted state (used when the app restarts
    /// mid-session). Kept separate from `new` so the normal path stays obvious.
    pub fn restore(state: State, completed_focus: u32) -> Self {
        Timer {
            state,
            completed_focus,
        }
    }

    /// Which break follows the focus session that just finished?
    fn break_after_focus(&self, plan: &Plan) -> Phase {
        let n = plan.sessions_before_long_break.max(1);
        if self.completed_focus % n == 0 {
            Phase::LongBreak
        } else {
            Phase::ShortBreak
        }
    }

    fn begin(&mut self, phase: Phase, plan: &Plan, out: &mut Vec<Effect>) {
        let total = plan.duration_of(phase);
        self.state = State::Running {
            phase,
            total,
            remaining: total,
        };
        out.push(Effect::Started(phase));
    }

    /// Feed a command in, get the resulting effects out.
    pub fn update(&mut self, cmd: Cmd, plan: &Plan) -> Vec<Effect> {
        let mut out = Vec::new();
        match cmd {
            Cmd::Start(phase) => {
                if let Some(cur) = self.state.phase() {
                    out.push(Effect::Ended {
                        phase: cur,
                        completed: false,
                    });
                }
                self.begin(phase, plan, &mut out);
            }

            Cmd::Toggle => match self.state.clone() {
                State::Idle => self.begin(Phase::Focus, plan, &mut out),
                State::Running {
                    phase,
                    total,
                    remaining,
                } => {
                    self.state = State::Paused {
                        phase,
                        total,
                        remaining,
                    };
                    out.push(Effect::Paused(phase));
                }
                State::Paused {
                    phase,
                    total,
                    remaining,
                } => {
                    self.state = State::Running {
                        phase,
                        total,
                        remaining,
                    };
                    out.push(Effect::Resumed(phase));
                }
            },

            Cmd::Pause => {
                if let State::Running {
                    phase,
                    total,
                    remaining,
                } = self.state.clone()
                {
                    self.state = State::Paused {
                        phase,
                        total,
                        remaining,
                    };
                    out.push(Effect::Paused(phase));
                }
            }

            Cmd::Resume => {
                if let State::Paused {
                    phase,
                    total,
                    remaining,
                } = self.state.clone()
                {
                    self.state = State::Running {
                        phase,
                        total,
                        remaining,
                    };
                    out.push(Effect::Resumed(phase));
                }
            }

            Cmd::Stop => {
                if let Some(phase) = self.state.phase() {
                    self.state = State::Idle;
                    out.push(Effect::Ended {
                        phase,
                        completed: false,
                    });
                    out.push(Effect::WentIdle);
                }
            }

            Cmd::Skip => {
                if let Some(phase) = self.state.phase() {
                    out.push(Effect::Ended {
                        phase,
                        completed: false,
                    });

                    let next = if phase.is_break() {
                        Some(Phase::Focus)
                    } else if plan.sequence_enabled {
                        Some(Phase::ShortBreak)
                    } else {
                        None
                    };
                    match next {
                        Some(p) => self.begin(p, plan, &mut out),
                        None => {
                            self.state = State::Idle;
                            out.push(Effect::WentIdle);
                        }
                    }
                }
            }

            Cmd::Tick(dt) => {
                if let State::Running {
                    phase,
                    total,
                    remaining,
                } = self.state.clone()
                {
                    let left = remaining.saturating_sub(dt);
                    if !left.is_zero() {
                        self.state = State::Running {
                            phase,
                            total,
                            remaining: left,
                        };
                    } else {
                        out.push(Effect::Ended {
                            phase,
                            completed: true,
                        });
                        if phase == Phase::Focus {
                            self.completed_focus = self.completed_focus.saturating_add(1);
                        }
                        let next = self.next_after_completed(phase, plan);
                        match next {
                            Some(p) => self.begin(p, plan, &mut out),
                            None => {
                                self.state = State::Idle;
                                out.push(Effect::WentIdle);
                            }
                        }
                    }
                }
            }
        }
        out
    }

    /// Sequencing decision after an interval ran to completion.
    fn next_after_completed(&self, finished: Phase, plan: &Plan) -> Option<Phase> {
        if !plan.sequence_enabled {
            return None;
        }
        if finished.is_break() {
            (plan.auto_start_focus || (finished == Phase::LongBreak && plan.repeat_cycles))
                .then_some(Phase::Focus)
        } else {
            plan.auto_start_break.then(|| self.break_after_focus(plan))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> Plan {
        Plan {
            focus: Duration::from_secs(60),
            short_break: Duration::from_secs(20),
            long_break: Duration::from_secs(40),
            sessions_before_long_break: 4,
            sequence_enabled: true,
            auto_start_break: true,
            auto_start_focus: false,
            repeat_cycles: false,
        }
    }

    #[test]
    fn toggle_from_idle_starts_focus() {
        let mut t = Timer::new();
        let fx = t.update(Cmd::Toggle, &plan());
        assert_eq!(fx, vec![Effect::Started(Phase::Focus)]);
        assert_eq!(t.state().phase(), Some(Phase::Focus));
    }

    #[test]
    fn toggle_pauses_and_resumes_preserving_remaining() {
        let p = plan();
        let mut t = Timer::new();
        t.update(Cmd::Toggle, &p);
        t.update(Cmd::Tick(Duration::from_secs(10)), &p);
        t.update(Cmd::Toggle, &p);
        assert!(matches!(t.state(), State::Paused { .. }));

        t.update(Cmd::Tick(Duration::from_secs(30)), &p);
        assert_eq!(t.state().remaining(), Some(Duration::from_secs(50)));
        t.update(Cmd::Toggle, &p);
        assert!(t.state().is_running());
        assert_eq!(t.state().remaining(), Some(Duration::from_secs(50)));
    }

    #[test]
    fn focus_completion_auto_starts_short_break() {
        let p = plan();
        let mut t = Timer::new();
        t.update(Cmd::Start(Phase::Focus), &p);
        let fx = t.update(Cmd::Tick(Duration::from_secs(60)), &p);
        assert_eq!(
            fx,
            vec![
                Effect::Ended {
                    phase: Phase::Focus,
                    completed: true
                },
                Effect::Started(Phase::ShortBreak),
            ]
        );
        assert_eq!(t.completed_focus(), 1);
    }

    #[test]
    fn fourth_focus_session_leads_to_long_break() {
        let p = plan();
        let mut t = Timer::new();
        for i in 1..=4 {
            t.update(Cmd::Start(Phase::Focus), &p);
            let fx = t.update(Cmd::Tick(Duration::from_secs(60)), &p);
            let started = fx
                .iter()
                .find_map(|e| match e {
                    Effect::Started(ph) => Some(*ph),
                    _ => None,
                })
                .unwrap();
            if i == 4 {
                assert_eq!(started, Phase::LongBreak, "4th session -> long break");
            } else {
                assert_eq!(started, Phase::ShortBreak, "session {i} -> short break");
            }
        }
    }

    #[test]
    fn long_break_repeat_cycles_starts_focus() {
        let mut p = plan();
        p.repeat_cycles = true;
        let mut t = Timer::new();
        t.update(Cmd::Start(Phase::LongBreak), &p);
        let fx = t.update(Cmd::Tick(Duration::from_secs(40)), &p);
        assert!(
            fx.iter()
                .any(|e| matches!(e, Effect::Started(Phase::Focus))),
            "long break should start a new cycle"
        );
    }

    #[test]
    fn skip_does_not_count_as_completed_focus() {
        let p = plan();
        let mut t = Timer::new();
        t.update(Cmd::Start(Phase::Focus), &p);
        let fx = t.update(Cmd::Skip, &p);
        assert_eq!(
            fx[0],
            Effect::Ended {
                phase: Phase::Focus,
                completed: false
            }
        );
        assert_eq!(t.completed_focus(), 0, "skipped focus must not count");

        assert_eq!(t.state().phase(), Some(Phase::ShortBreak));
    }

    #[test]
    fn oversized_tick_from_sleep_ends_exactly_once() {
        let p = plan();
        let mut t = Timer::new();
        t.update(Cmd::Start(Phase::Focus), &p);
        let fx = t.update(Cmd::Tick(Duration::from_secs(9999)), &p);
        assert_eq!(
            fx.iter()
                .filter(|e| matches!(e, Effect::Ended { .. }))
                .count(),
            1
        );
    }
}
