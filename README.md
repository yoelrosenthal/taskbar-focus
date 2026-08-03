<div align="center">

<img src="focus-app-icons/icon-256.png" width="120" alt="taskbar-focus">

# taskbar-focus

**A tiny focus timer for Windows that actually silences your notifications.**

Work in 25-minute stretches. While you're focused, Windows goes quiet.
When you take a break, everything comes back.

**Windows 10 and 11 only, 64-bit.**

</div>

---

## Install

This is a Windows program and only a Windows program. It is built on the Windows
notification area, the Windows notification service and the Windows quiet-hours
settings, none of which have an equivalent elsewhere — so there is no macOS or
Linux version, and there is not going to be one. Everything below is PowerShell.

**With [Scoop](https://scoop.sh)** — recommended, and updates cleanly:

```powershell
scoop bucket add taskbar-focus https://github.com/yoelrosenthal/taskbar-focus
scoop install taskbar-focus
```

Later: `scoop update taskbar-focus`

**With Cargo**, if you have Rust on Windows:

```powershell
cargo install taskbar-focus
```

**Just the executable:** download `taskbar-focus.exe` from the
[latest release](../../releases) and run it. Nothing to configure — put it
wherever you like.

> **Windows 11 hides new tray icons.** If you don't see it after starting,
> click the `^` arrow near the clock and drag **taskbar-focus** onto the
> taskbar.

## What you get

### A timer that sits on your taskbar

A small window showing the countdown, sized to sit on the taskbar out of the box:

```
▌ Focus                    23:16
▬▬▬▬▬▬▬▬▬▬▬▬▬▬▬▬▬▬▬▬▬▬▬▬▬▬▬▬▬▬▬
```

- **Drag it** anywhere by its middle. **Resize it** from any edge.
- Make it big and it switches to a stacked layout with a large clock.
- **Click it** to pause or resume.
- It stays on top so you can always see it, and remembers where you put it.

Turn it on in Settings, or from the tray menu. *Reset window size* puts it back.

### Quiet while you work

When focus starts, Windows **Do Not Disturb** switches on. When a break starts,
it switches off. That's the whole point of the app.

Your **priority** apps and contacts still get through — it uses exactly the mode
Windows' own "Do not disturb" button uses, and never touches your priority list.

### A muted bell, because Windows will not draw one

Windows only shows its own muted icon when the taskbar itself switches Do Not
Disturb on, so muting from another program leaves no visible trace at all. This
app draws that trace instead — the same crossed-out bell — and it follows Do Not
Disturb however it was switched on, including by you.

- a **bell on the application icon** — its title bar and taskbar button
- a **separate bell beside the clock**, present only while you are muted
- a **bell in the compact timer window**, next to the countdown

All three indicators are on by default and can be switched independently under
*Do Not Disturb* in Settings. Windows 11 files new tray icons behind the `^`
arrow — drag the bell out once and it stays where you put it.

The timer's tray icon is left alone: sixteen pixels hold a progress ring or a
bell, not both.

### A tray icon that shows progress

| | |
|---|---|
| 🔴 Red ring filling up | Focused |
| 🟢 Green ring | On a break |
| ⚪ Grey ring | Nothing running |
| Faded | Paused |

**Left-click** starts, pauses and resumes. **Right-click** for everything else.

### Shortcuts

| | |
|---|---|
| <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>F</kbd> | Start, pause or resume |
| <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>B</kbd> | Skip to the next interval |

They work from any app, and you can change them in Settings.

## How a session flows

Finish a focus session → a short break starts by itself → after four sessions
you get a long break instead. Breaks end by going idle, so *you* decide when to
start working again.

Don't like that? Turn the automatic sequence off and drive it by hand.

**Strict focus** makes stopping or skipping a running focus session ask first,
so a stray click can't end it.

## Presets

Three to start with — **Pomodoro 25/5**, **Deep Work 90/15**, **Short 15/3** —
and you can edit them or add your own with *Save as new preset*. Switch between
them from the tray menu.

## From the command line

Handy for shortcuts, stream decks, or scripts. Commands go to the running app,
so you never end up with two timers.

```powershell
taskbar-focus --start                     # start focusing
taskbar-focus --break                     # take a break
taskbar-focus --toggle                    # start / pause / resume
taskbar-focus --skip                      # next interval
taskbar-focus --stop                      # stop
taskbar-focus --preset "Deep Work 90/15"  # switch preset
taskbar-focus --quit
```

## Good to know

**If your antivirus complains.** It might. Muting notifications and restarting a
Windows service look, from the outside, like something suspicious — and a
brand-new unsigned program has no reputation yet. Run
`taskbar-focus --explain` to see precisely what it touches; it's read-only and
works without the app running. Everything it does is logged to
`%APPDATA%\taskbar-focus\activity.log`. On a work machine, ask IT to allow it
rather than turning off your protection. Or switch off **Mute notifications
during focus** and use it as a plain timer.

**Windows' own muted icon will not appear.** Notifications really are muted, but
Explorer caches the indicator beside the clock, so it does not update without
restarting Explorer. Restarting the desktop for an icon is unreasonable, so the
app draws [its own mark](#a-muted-bell-because-windows-will-not-draw-one)
instead.

**Your settings** live in `%APPDATA%\taskbar-focus\config.toml` — plain text you
can back up, sync or edit by hand.

**It handles the awkward cases.** Computer sleeps mid-session? Counts the time
asleep by default (or ignores it, or pauses — your choice). Changed the system
clock? Ignored. Closed the app mid-session? It comes back **paused**, with the
time you were away deducted. Explorer restarted? The tray icon reappears.

**Privacy.** No network access at all — there's no networking code in it. No
telemetry, no accounts, no auto-update. It never needs administrator rights.

**Needs** Windows 10 or 11, 64-bit — and nothing else: no runtime to install, no
framework. It does not run on macOS or Linux.

## Uninstall

`scoop uninstall taskbar-focus`, or `cargo uninstall taskbar-focus`, or just
delete the `.exe`.

Either way, settings stay in `%APPDATA%\taskbar-focus` until you delete that
folder. Do Not Disturb is switched back when the app exits.

## For developers

Build it on Windows, with the MSVC toolchain. Every source file below `src/os/`
and `src/ui/` calls Win32 directly, so a build on any other platform does not
compile and is not meant to.

```powershell
cargo build --release        # target\release\taskbar-focus.exe
cargo test
cargo install --path .       # install your local build
```

Push a tag (`git tag v0.1.0 && git push origin v0.1.0`) and GitHub Actions
builds the release, attaches a build provenance attestation, updates the Scoop
manifest and publishes to crates.io. The tag must match the version in
`Cargo.toml` — the workflow checks this first and stops if it doesn't.

Publishing to crates.io needs a `CARGO_REGISTRY_TOKEN` repository secret (from
[crates.io/settings/tokens](https://crates.io/settings/tokens)). Without it that
step is skipped, and everything else still runs.

The interesting part is [`src/os/dnd/`](src/os/dnd/) — Windows has no supported
API for setting Do Not Disturb, so that module explains what it does instead,
and why.

This repository doubles as its own Scoop bucket: the manifest in
[`bucket/`](bucket/) is updated automatically on each release.

## License

MIT — see [LICENSE](LICENSE).
