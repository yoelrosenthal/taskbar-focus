# Changelog

All notable changes to taskbar-focus are recorded here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
the project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Sections below the marker are written by the release workflow when a pull
request is merged into `main`; there is no need to edit them by hand.

<!-- next-release -->

## [0.4.0] - 2026-08-20

### Added

- A centre overlay when a focus or break ends, with an optional wait-for-dismiss
  that pauses the timer until you continue.
- A screen flash on every connected monitor, with a short / normal / long /
  extra-long length.
- Tray toast, overlay, and flash as independent options under
  **Settings → Notifications**.
- An option to start another cycle after a long break.
- An option to keep Do Not Disturb on through short breaks, unmuting only on a
  long break.
- A dedicated Strict-focus confirmation instead of a system MessageBox.

### Changed

- New installs, and **Restore defaults**, loop the Pomodoro sequence, show the
  compact timer, ignore time spent asleep, and do not restore a session on
  restart. An existing `config.toml` is left as-is.
- Completion cues use the Windows Asterisk sound, the same as session starts.
- Overlay and flash colour follow the phase being entered: red for focus, green
  for a break.

### Fixed

- Toasts no longer turn off Do Not Disturb during a short break when you asked
  to keep it on.
- Saving Settings no longer clears dependent checkboxes just because their
  parent switch is off.
- The overlay appears on the monitor you are using when the compact timer is
  hidden.
- Strict focus cannot open a second confirmation on top of the first.

**Full changelog**: https://github.com/yoelrosenthal/taskbar-focus/compare/v0.3.4...v0.4.0

## [0.3.4] - 2026-08-04

### Changed

- docs: keep badges on one row
- docs: split badges across two rows
- docs: add project badges
- docs: refresh user guide and screenshots

**Full changelog**: https://github.com/yoelrosenthal/taskbar-focus/compare/v0.3.3...v0.3.4

## [0.3.3] - 2026-08-04

### Fixed

- shorten the notification app header

**Full changelog**: https://github.com/yoelrosenthal/taskbar-focus/compare/v0.3.2...v0.3.3

## [0.3.2] - 2026-08-04

### Fixed

- let focus-start notifications render before DND

**Full changelog**: https://github.com/yoelrosenthal/taskbar-focus/compare/v0.3.1...v0.3.2

## [0.3.1] - 2026-08-04

### Fixed

- keep notifications queued until DND releases
- show timer notifications around DND transitions

**Full changelog**: https://github.com/yoelrosenthal/taskbar-focus/compare/v0.3.0...v0.3.1

## [0.3.0] - 2026-08-04

### Added

- draw the icons from the system icon font
- give every setting and page an icon
- group settings into pages with a search box

**Full changelog**: https://github.com/yoelrosenthal/taskbar-focus/compare/v0.2.0...v0.3.0

## [0.2.0] - 2026-08-03

### Changed

- Harden the release workflow against partial failures
- Drop the muted bell on the application icon
- Update the application icon
- Release automatically when a pull request is merged
- Trim the Windows-only note back to one line
- Say plainly in the README that this is Windows-only
- Show a muted-bell indicator while Do Not Disturb is on

**Full changelog**: https://github.com/yoelrosenthal/taskbar-focus/compare/v0.1.0...v0.2.0

## [0.1.0] - 2026-07-31

### Added

- Initial release: a tray app that toggles Windows "Do not disturb" on a timer,
  with a settings window, a compact window and a `--explain` flag.
- Scoop manifest, so the repository doubles as a Scoop bucket.

### Changed

- Pin the docs.rs build target to Windows.
