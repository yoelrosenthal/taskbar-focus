# Conventions for this repository

## Comments

**Do not write inline comments.** No `//` line comments, no `/* */` blocks.

Only **doc comments** are allowed:

- `///` on items (functions, structs, fields, enum variants)
- `//!` at the top of a module, for what the module is and why it exists

If something needs explaining, put it in the doc comment of the thing it
explains, or in the module header. If a piece of code needs an inline comment to
be understood, rename it or extract it into a named function instead.

```rust
/// Restart the per-user notification service so Windows re-reads the setting.
fn restart_notification_service() -> usize { ... }   // ✅

fn restart_notification_service() -> usize {
    // Windows caches the setting, so bounce the service   ← ❌ delete this
    ...
}
```

## Tests

Write **few** tests. Only for logic that would genuinely hurt if it broke:

- the timer state machine
- the registry blob rewriting in `os::dnd::profile`
- clock/sleep reconciliation

Do not write tests for string formatting, buffer helpers, enum mappings,
argument parsing or other low-risk code. Prefer running the real application and
checking the result over adding assertions.

## Verifying UI work

The settings and compact windows cannot be judged from code. When a change
affects how something looks, **ask the user for a screenshot** rather than
guessing or inferring it from window metrics. Past attempts to verify layout by
enumerating control rectangles produced false conclusions.

For the tray icon specifically, render it at its real **16px** size and inspect
it magnified. Designs reviewed at larger sizes have been misleading.

## Documentation

- `README.md` is for **end users**. Keep it task-oriented and free of internal
  or organisation-specific material. End users do not need to know about code
  signing, certificates or endpoint-security policy.
- `bucket/taskbar-focus.json` is the Scoop manifest, and `bucket/` is what makes
  this repository usable as a Scoop bucket - Scoop looks for manifests there, so
  the directory name is not arbitrary. Its `version` and `hash` are rewritten
  automatically by the release workflow; do not hand-edit them.
- Deployment notes for any particular organisation, and signing procedures and
  certificates, live **outside this repository** and must not be committed.
  The user-facing note that releases are unsigned, in `SECURITY.md`, stays.
- `CHANGELOG.md` is generated. Sections below the `<!-- next-release -->` marker
  are written by the release workflow; only the header above it is hand-edited.

## Pull requests

After committing and opening a pull request, watch its checks and review
comments until settled. Evaluate comments rather than following them blindly;
apply valid feedback that fits the user's request and repository rules, and ask
the user when feedback is unclear or conflicting. Push required fixes and watch
again. Report completion only when checks are green and valid comments are
resolved; merge only when explicitly requested.

## Releases

**Merging a pull request into `main` publishes a release.** There is no separate
step. `release-on-merge.yml` bumps the version, writes the `CHANGELOG.md`
section, tags the commit and calls `release.yml`, which builds, attests,
publishes the GitHub Release, updates the Scoop manifest and pushes to
crates.io.

How big the bump is comes from a label on the pull request:

| Label            | Effect                          |
| ---------------- | ------------------------------- |
| `release:major`  | `0.4.1` → `1.0.0`               |
| `release:minor`  | `0.4.1` → `0.5.0`               |
| `release:patch`  | `0.4.1` → `0.4.2`               |
| `release:skip`   | merge without releasing         |

With no label, `.github/scripts/suggest-bump.sh` reads the commit subjects and
bodies plus the pull request title: a `!:` prefix or a line starting
`BREAKING CHANGE` (or `BREAKING-CHANGE`) is major, `feat:` is minor, anything
else is patch. That fallback exists so a merge is never blocked, not so the label can
be skipped - decide the label when opening the pull request.

Conventional `feat:` and `fix:` prefixes also sort commits into the changelog's
Added and Fixed sections. Plain subjects land under Changed, which is fine.

Never bump `version` in `Cargo.toml` by hand, and never tag by hand unless a
release has to be re-cut - `release.yml` still accepts a pushed tag, and a
`workflow_dispatch` with an existing tag, for exactly that case.

## Things not to break

- Never commit signing material: every pattern under the signing block in
  `.gitignore` (`*.pfx`, `*.p12`, `*.pvk`, `*.spc`, `*.snk`, `*.key`, `*.pem`,
  `*.cer`, `*.crt`).
- The app must never require administrator rights.
- No network access, no telemetry.
- `os::dnd` touches undocumented Windows behaviour. Every change there must be
  verified against a live system and must fail loudly, never silently.

## Agent skills

### Issue tracker

Issues are tracked in GitHub Issues via the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Uses the five default triage labels: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `GLOSSARY.md` and `docs/adr/` live at the repo root once created. See `docs/agents/domain.md`.
