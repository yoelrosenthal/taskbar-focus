//! Command-line interface.
//!
//! Every verb is also what a second instance forwards to the running one, so
//! `taskbar-focus.exe --start` from a script controls the timer already in the
//! tray instead of launching a rival copy.

/// A single instruction, from the command line or from another instance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// No arguments: run the app (or surface the existing instance).
    Run,
    StartFocus,
    StartBreak,
    Stop,
    Pause,
    Resume,
    Toggle,
    Skip,
    Preset(String),
    Quit,
    Help,
    Version,
    /// Print exactly what the program does to the system, then exit.
    Explain,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ParseError(pub String);

impl Command {
    /// Wire format for [`WM_COPYDATA`] between instances.
    pub fn encode(&self) -> String {
        match self {
            Command::Run => "run".into(),
            Command::StartFocus => "start".into(),
            Command::StartBreak => "break".into(),
            Command::Stop => "stop".into(),
            Command::Pause => "pause".into(),
            Command::Resume => "resume".into(),
            Command::Toggle => "toggle".into(),
            Command::Skip => "skip".into(),
            Command::Preset(n) => format!("preset:{n}"),
            Command::Quit => "quit".into(),
            Command::Help => "help".into(),
            Command::Version => "version".into(),
            Command::Explain => "explain".into(),
        }
    }

    pub fn decode(s: &str) -> Option<Command> {
        Some(match s {
            "run" => Command::Run,
            "start" => Command::StartFocus,
            "break" => Command::StartBreak,
            "stop" => Command::Stop,
            "pause" => Command::Pause,
            "resume" => Command::Resume,
            "toggle" => Command::Toggle,
            "skip" => Command::Skip,
            "quit" => Command::Quit,
            "help" => Command::Help,
            "version" => Command::Version,
            "explain" => Command::Explain,
            other => Command::Preset(other.strip_prefix("preset:")?.to_string()),
        })
    }

    /// Commands that only make sense to hand to a running instance.
    pub fn is_remote_control(&self) -> bool {
        !matches!(
            self,
            Command::Run | Command::Help | Command::Version | Command::Explain
        )
    }
}

pub const HELP: &str = concat!(
    "taskbar-focus ",
    env!("CARGO_PKG_VERSION"),
    r#" - a focus/Pomodoro timer for the Windows tray.

USAGE:
    taskbar-focus.exe [COMMAND]

With no command, starts the app (or brings the running one to the front).
Any command is forwarded to the already-running instance, so these are safe
to call from scripts and shortcuts.

COMMANDS:
    --start             Start (or restart) a focus session
    --break             Start a break now
    --stop              Stop the current session and go idle
    --pause             Pause the current session
    --resume            Resume a paused session
    --toggle            Start, pause or resume, depending on state
    --skip              End the current interval and move to the next
    --preset <NAME>     Switch to a named preset, e.g. --preset "Deep Work 90/15"
    --quit              Exit the running instance
    --explain           Report exactly what this program does to your system,
                        including the real registry paths it would write.
                        Read-only: running it changes nothing.
    -h, --help          Show this help
    -V, --version       Show the version
"#
);

/// Parse process arguments (excluding argv[0]).
pub fn parse<I, S>(args: I) -> Result<Command, ParseError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut it = args.into_iter().peekable();
    let Some(first) = it.next() else {
        return Ok(Command::Run);
    };
    let arg = first.as_ref();
    let cmd = match arg {
        "--start" | "--focus" => Command::StartFocus,
        "--break" => Command::StartBreak,
        "--stop" => Command::Stop,
        "--pause" => Command::Pause,
        "--resume" => Command::Resume,
        "--toggle" => Command::Toggle,
        "--skip" | "--next" => Command::Skip,
        "--quit" | "--exit" => Command::Quit,
        "-h" | "--help" | "/?" => Command::Help,
        "-V" | "--version" => Command::Version,
        "--explain" => Command::Explain,
        "--preset" => {
            let name = it
                .next()
                .ok_or_else(|| ParseError("--preset needs a preset name".into()))?;
            let name = name.as_ref().trim().to_string();
            if name.is_empty() {
                return Err(ParseError("--preset needs a preset name".into()));
            }
            Command::Preset(name)
        }
        other => {
            return Err(ParseError(format!(
                "unrecognised argument `{other}` (try --help)"
            )))
        }
    };
    if let Some(extra) = it.next() {
        return Err(ParseError(format!(
            "unexpected extra argument `{}`",
            extra.as_ref()
        )));
    }
    Ok(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_survives_the_ipc_round_trip() {
        let all = [
            Command::Run,
            Command::StartFocus,
            Command::StartBreak,
            Command::Stop,
            Command::Pause,
            Command::Resume,
            Command::Toggle,
            Command::Skip,
            Command::Preset("Deep Work 90/15".into()),
            Command::Quit,
            Command::Help,
            Command::Version,
            Command::Explain,
        ];
        for c in all {
            assert_eq!(Command::decode(&c.encode()).as_ref(), Some(&c), "{c:?}");
        }
    }
}
