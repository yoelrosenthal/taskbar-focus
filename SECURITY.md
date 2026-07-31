# Security Policy

## Reporting a vulnerability

Please report privately via GitHub's **Report a vulnerability** button on the
Security tab, rather than opening a public issue.

## Verifying a download

Release binaries are built by GitHub Actions and carry a build provenance
attestation:

```
gh attestation verify taskbar-focus.exe --repo yoelrosenthal/taskbar-focus
```

This proves the binary was produced from the tagged commit by this
repository's workflow. Checksums are in `SHA256SUMS.txt`.

Releases are **not** Authenticode-signed, so Windows will warn about an unknown
publisher. That is expected. **This project will never ask you to install a
certificate in order to run it** — treat any such instruction, from any source,
as a red flag.

## What this application does to your system

It toggles Windows "Do not disturb" during focus sessions, which requires an
`HKCU` registry write and restarting the per-user `WpnUserService_*`. Some
endpoint security products flag that pattern.

To see exactly what it touches on your machine:

```
taskbar-focus.exe --explain
```

It is read-only and works without the app running. Full detail, including the
complete Windows API inventory, is in `--explain`.

The behaviour is entirely opt-in: set `enabled = false` under `[dnd]` in the
config and no registry write, service control or `ntdll` lookup occurs at all.

## Scope

- No network access, no telemetry, no auto-update.
- Runs as the invoking user; never requests elevation.
- Writes only to `%APPDATA%\taskbar-focus\` and the `HKCU` value described above.
