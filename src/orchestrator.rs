//! The seam between the pure timer and the operating system.
//!
//! The UI layer never talks to the timer or to Do Not Disturb directly. It
//! sends [`Command`]s in and gets [`UiEvent`]s back, which keeps every policy
//! decision ("should this event make a sound?", "does focus mute the machine?")
//! in one readable place instead of scattered through window procedures.

use crate::cli::Command;
use crate::clock::Clock;
use crate::config::{Config, WakePolicy};
use crate::os::dnd::worker::DndWorker;
use crate::os::dnd::DndOutcome;
use crate::session;
use crate::timer::{Cmd, Effect, Phase, State, Timer};

/// Which sound/notification an event maps to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    FocusStart,
    FocusEnd,
    BreakStart,
    BreakEnd,
}

/// Something the UI should do.
#[derive(Clone, Debug, PartialEq)]
pub enum UiEvent {
    /// Show a Windows notification.
    Notify { title: String, body: String },
    /// Play the sound associated with an event.
    Sound(Event),
    /// Tray icon / taskbar title / settings need repainting.
    Refresh,
    /// A warning worth showing the user (a DND change that did not stick).
    Warn(String),
}

pub struct Orchestrator {
    pub config: Config,
    pub timer: Timer,
    clock: Clock,
    dnd: DndWorker,
    /// Last DND outcome, for display in the tray menu.
    dnd_note: Option<String>,
    /// True while this app is the reason notifications are muted.
    ///
    /// Reported instead of the system state, which is not the same question:
    /// Do Not Disturb can be on because the user switched it on, and the
    /// shell's published value can lag a change by a second or two. Claiming
    /// "Do Not Disturb is on" when the app had not touched anything was simply
    /// misleading.
    dnd_engaged: bool,
    /// Whether notifications are being suppressed right now, for the muted
    /// indicator. Cached because it is read from the system, not from us.
    dnd_active: bool,
    /// Ticks to wait before reading the system state again.
    dnd_poll_in: u32,
}

/// How often the live Do Not Disturb state is re-read, in ticks - every two
/// seconds at the UI's twice-a-second tick. The mark has no reason to be more
/// responsive than that, and the state can also change behind our back when the
/// user toggles Do Not Disturb themselves.
const DND_POLL_TICKS: u32 = 4;

impl Orchestrator {
    pub fn new(config: Config, timer: Timer, dnd: DndWorker) -> Self {
        Orchestrator {
            config,
            timer,
            clock: Clock::new(),
            dnd,
            dnd_note: None,
            dnd_engaged: false,
            dnd_active: false,
            dnd_poll_in: 0,
        }
    }

    /// One phrase describing what is happening to notifications, and why.
    ///
    /// The reason matters: "notifications not muted" next to a ticked "mute
    /// during focus" box reads as a contradiction, when in fact both are true
    /// during a break. Saying which it is removes the apparent conflict.
    pub fn dnd_status(&self) -> &'static str {
        if !self.config.dnd.enabled {
            return "muting is switched off";
        }
        if self.dnd_engaged {
            return "notifications muted";
        }
        match self.timer.state().phase() {
            Some(p) if p.is_break() => "notifications on during breaks",
            Some(_) => "notifications not muted",
            None => "notifications on - nothing running",
        }
    }

    pub fn dnd_note(&self) -> Option<&str> {
        self.dnd_note.as_deref()
    }

    /// Are notifications actually being suppressed at this moment?
    ///
    /// This is what the muted mark draws, and it deliberately reports the
    /// *system* state rather than [`Self::dnd_status`]'s "did we do it": a user
    /// who switched Do Not Disturb on themselves is just as muted, and Windows
    /// will not show them an indicator for it either while this app is running
    /// its own. Falls back to our own bookkeeping when the state cannot be read.
    pub fn dnd_active(&self) -> bool {
        self.dnd_active
    }

    /// Re-read the live Do Not Disturb state.
    ///
    /// Skipped entirely when no indicator is switched on, so a user who wants
    /// nothing to do with the undocumented read simply does not get one.
    fn poll_dnd(&mut self) {
        self.dnd_active = if self.config.dnd.wants_indicator() {
            crate::os::dnd::wnf::query().map_or(self.dnd_engaged, |state| state.is_on())
        } else {
            self.dnd_engaged
        };
    }

    /// Translate a CLI/UI command into timer input.
    pub fn dispatch(&mut self, command: &Command) -> Vec<UiEvent> {
        let cmd = match command {
            Command::StartFocus => Cmd::Start(Phase::Focus),
            Command::StartBreak => Cmd::Start(Phase::ShortBreak),
            Command::Stop => Cmd::Stop,
            Command::Pause => Cmd::Pause,
            Command::Resume => Cmd::Resume,
            Command::Toggle => Cmd::Toggle,
            Command::Skip => Cmd::Skip,
            Command::Preset(name) => {
                let mut out = Vec::new();
                if self.config.select_preset(name) {
                    let _ = self.config.save();
                    out.push(UiEvent::Refresh);
                } else {
                    out.push(UiEvent::Warn(format!("No preset named \"{name}\"")));
                }
                return out;
            }

            Command::Run | Command::Quit | Command::Help | Command::Version | Command::Explain => {
                return Vec::new()
            }
        };
        self.apply(cmd)
    }

    /// Advance the timer. Called from the UI timer tick.
    pub fn tick(&mut self) -> Vec<UiEvent> {
        let elapsed = self.clock.tick(self.config.behavior.wake_policy);
        let mut out = Vec::new();

        if self.dnd_poll_in == 0 {
            self.poll_dnd();
            self.dnd_poll_in = DND_POLL_TICKS;
        } else {
            self.dnd_poll_in -= 1;
        }

        if let Some(slept) = elapsed.slept {
            if self.config.behavior.wake_policy == WakePolicy::Pause
                && self.timer.state().is_running()
            {
                out.extend(self.apply(Cmd::Pause));
                out.push(UiEvent::Warn(format!(
                    "Paused: the machine was asleep for {}.",
                    human_duration(slept)
                )));
                return out;
            }
        }

        if !elapsed.delta.is_zero() {
            out.extend(self.apply(Cmd::Tick(elapsed.delta)));
        }
        out
    }

    /// The user changed the system clock.
    pub fn note_time_change(&mut self) {
        self.clock.note_time_change();
    }

    /// Drain finished DND changes and turn failures into warnings.
    pub fn collect_dnd_reports(&mut self) -> Vec<UiEvent> {
        let mut out = Vec::new();
        for r in self.dnd.reports().collect::<Vec<_>>() {
            crate::audit::log_dnd(r.engaging, &format!("{:?}", r.outcome));
            match &r.outcome {
                DndOutcome::Applied(_) => {
                    self.dnd_engaged = r.engaging;
                    self.dnd_note = None;
                }
                DndOutcome::AlreadyCorrect => {
                    if !r.engaging {
                        self.dnd_engaged = false;
                    }
                }
                DndOutcome::Unverified => {
                    self.dnd_engaged = r.engaging;
                    self.dnd_note = Some("Could not confirm the change with Windows".into());
                }
                DndOutcome::Failed(why) => {
                    self.dnd_note = Some(format!("Do Not Disturb failed: {why}"));
                    out.push(UiEvent::Warn(format!(
                        "Could not {} Do Not Disturb: {why}",
                        if r.engaging { "turn on" } else { "turn off" }
                    )));
                }
            }
            out.push(UiEvent::Refresh);
        }
        if !out.is_empty() {
            self.poll_dnd();
        }
        out
    }

    /// Feed the state machine and turn its effects into OS actions.
    fn apply(&mut self, cmd: Cmd) -> Vec<UiEvent> {
        let plan = self.config.plan();
        let effects = self.timer.update(cmd, &plan);
        if effects.is_empty() {
            return Vec::new();
        }

        let mut out = Vec::new();
        for e in &effects {
            match *e {
                Effect::Started(Phase::Focus) => {
                    self.set_dnd(true);
                    self.emit(
                        &mut out,
                        Event::FocusStart,
                        "Focus started",
                        &self.focus_body(),
                    );
                }
                Effect::Started(phase) => {
                    self.set_dnd(false);
                    self.emit(
                        &mut out,
                        Event::BreakStart,
                        &format!("{} started", phase.label()),
                        &format!("{} to relax.", self.remaining_text()),
                    );
                }
                Effect::Ended { phase, completed } => {
                    if completed {
                        if phase == Phase::Focus {
                            self.emit(
                                &mut out,
                                Event::FocusEnd,
                                "Focus session complete",
                                "Nice work - time for a break.",
                            );
                        } else {
                            self.emit(
                                &mut out,
                                Event::BreakEnd,
                                "Break over",
                                "Ready for the next focus session?",
                            );
                        }
                    }
                }
                Effect::WentIdle => self.set_dnd(false),
                Effect::Paused(_) | Effect::Resumed(_) => {}
            }
        }

        session::save(&self.timer);
        out.push(UiEvent::Refresh);
        out
    }

    fn focus_body(&self) -> String {
        if self.config.dnd.enabled {
            format!(
                "{} of focus. Notifications are muted.",
                self.remaining_text()
            )
        } else {
            format!("{} of focus.", self.remaining_text())
        }
    }

    fn remaining_text(&self) -> String {
        self.timer
            .state()
            .remaining()
            .map(human_duration)
            .unwrap_or_default()
    }

    /// Ask the background worker to change DND, if the user enabled that.
    fn set_dnd(&mut self, on: bool) {
        if !self.config.dnd.enabled {
            return;
        }
        if on {
            self.dnd.engage();
        } else {
            self.dnd.release();
        }
    }

    /// Queue a notification and/or sound, honouring the per-event toggles.
    fn emit(&self, out: &mut Vec<UiEvent>, event: Event, title: &str, body: &str) {
        let n = &self.config.notifications;
        let s = &self.config.sounds;
        let enabled = |t: &crate::config::EventToggles| match event {
            Event::FocusStart => t.focus_start,
            Event::FocusEnd => t.focus_end,
            Event::BreakStart => t.break_start,
            Event::BreakEnd => t.break_end,
        };
        if n.enabled && enabled(&n.events) {
            out.push(UiEvent::Notify {
                title: title.to_string(),
                body: body.to_string(),
            });
        }
        if !s.muted && enabled(&s.events) {
            out.push(UiEvent::Sound(event));
        }
    }

    /// Longer form for the tray tooltip.
    pub fn tooltip(&self) -> String {
        match self.timer.state() {
            State::Idle => "taskbar-focus - idle".into(),
            State::Running {
                phase, remaining, ..
            } => format!("{} - {} left", phase.label(), human_duration(*remaining)),
            State::Paused {
                phase, remaining, ..
            } => format!(
                "{} - paused, {} left",
                phase.label(),
                human_duration(*remaining)
            ),
        }
    }
}

/// `MM:SS`, or `H:MM:SS` for intervals of an hour or more.
pub fn mmss(d: std::time::Duration) -> String {
    let total = d.as_secs();
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

pub fn human_duration(d: std::time::Duration) -> String {
    let total = d.as_secs();
    match (total / 3600, (total % 3600) / 60, total % 60) {
        (0, 0, s) => format!("{s} second{}", plural(s)),
        (0, m, _) => format!("{m} minute{}", plural(m)),
        (h, 0, _) => format!("{h} hour{}", plural(h)),
        (h, m, _) => format!("{h}h {m}m"),
    }
}

fn plural(n: u64) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}
