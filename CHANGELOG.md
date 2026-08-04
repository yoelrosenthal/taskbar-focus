# Changelog

All notable changes to taskbar-focus are recorded here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
the project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Sections below the marker are written by the release workflow when a pull
request is merged into `main`; there is no need to edit them by hand.

<!-- next-release -->

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
