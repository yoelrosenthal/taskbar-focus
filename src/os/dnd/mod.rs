//! Turning Windows "Do not disturb" on and off.
//!
//! # ⚠ Read this before changing anything in this directory
//!
//! Windows has **no supported API for setting** Do Not Disturb. The documented
//! one, [`Windows.UI.Shell.FocusSessionManager`], exposes `TryStartFocusSession`
//! and `DeactivateFocus`, but both are *Limited Access Features*: calling them
//! requires an unlock token issued by Microsoft per-app, which an open-source
//! project cannot ship. Its read-only members are free to use, but they report
//! **Focus sessions**, not DND (see `wnf.rs`).
//!
//! So the write path here is the mechanism the admin/community tooling settled
//! on, and it is undocumented and fragile by nature:
//!
//! 1. `cloudstore` - rewrite the active profile name inside a serialised blob
//!    in the registry. The key path is discovered, not hardcoded, because it
//!    was renamed between Windows versions.
//! 2. `service` - restart `WpnUserService_*`, without which the change is
//!    invisible: the service caches the state and nothing re-reads the registry.
//! 3. `wnf` - read back the *effective* state to confirm it actually applied.
//!
//! Step 3 is what keeps this honest. Every change is verified, and an
//! unverifiable or failed change is reported to the caller rather than being
//! silently assumed to have worked.
//!
//! ## What this means for priority notifications
//!
//! We switch to `Microsoft.QuietHoursProfile.PriorityOnly`, which is precisely
//! the profile the Windows UI calls "Do not disturb". Windows keeps applying
//! the user's own priority list, so calls, reminders and any apps they marked
//! as priority still come through. We never touch that list.
//!
//! ## Known cosmetic limitation
//!
//! Muting is enforced, but Windows' own indicator beside the clock usually does
//! not appear, and the Settings toggle can still read "off" until reopened.
//! Enforcement lives in the notification service, which is restarted here and
//! does pick the change up; the taskbar chrome is drawn by Explorer, which
//! caches the state and only refreshes when Explorer itself performs the
//! toggle. Nothing short of restarting Explorer changes that, which is not a
//! reasonable thing to do to someone's desktop for the sake of an icon.
//!
//! ## If a future Windows build breaks this
//!
//! The failure mode is designed to be loud but harmless: `engage` returns
//! [`DndOutcome::Failed`] or [`DndOutcome::Unverified`], the UI surfaces it, and
//! the timer keeps working as a plain Pomodoro timer. Fixing it should only
//! require updating `profile.rs` (blob layout) or `wnf.rs` (state name).

pub mod cloudstore;
pub mod profile;
pub mod service;
pub mod wnf;
pub mod worker;

pub use wnf::DndState;

/// Crash recovery
/// --------------
///
/// If the process is killed rather than exited - Task Manager, a force-kill, a
/// power loss - `Drop` never runs and Do Not Disturb stays on. Worse, the app
/// deliberately leaves a pre-existing DND alone on the next start (so it never
/// overrides a setting the user chose themselves), which means it would never
/// heal by itself.
///
/// So while DND is engaged, the original bytes are also written to disk. If
/// that file is still there at startup, a previous instance died holding the
/// user's notifications hostage, and we put them back.
const RECOVERY_FILE: &str = "dnd-recovery.toml";

#[derive(serde::Serialize, serde::Deserialize)]
struct Recovery {
    entries: Vec<RecoveryEntry>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct RecoveryEntry {
    path: String,
    /// Original blob, hex encoded so the file stays human-readable text.
    blob_hex: String,
}

/// Seconds since the Unix epoch, for the blob's timestamp field.
fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn recovery_path() -> Option<std::path::PathBuf> {
    crate::config::app_dir().map(|d| d.join(RECOVERY_FILE))
}

fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn from_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

fn persist_recovery(saved: &[(String, Vec<u8>)]) {
    let Some(p) = recovery_path() else { return };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let rec = Recovery {
        entries: saved
            .iter()
            .map(|(path, blob)| RecoveryEntry {
                path: path.clone(),
                blob_hex: to_hex(blob),
            })
            .collect(),
    };
    if let Ok(text) = toml::to_string_pretty(&rec) {
        let _ = std::fs::write(
            &p,
            format!(
                "# Written while this app has Do Not Disturb switched on.\n\
                 # If it is still here at startup, the app was killed and these\n\
                 # original values are restored. Safe to delete when not running.\n\n{text}"
            ),
        );
    }
}

fn clear_recovery() {
    if let Some(p) = recovery_path() {
        let _ = std::fs::remove_file(p);
    }
}

/// Called once at startup. Returns `Some` if a previous instance left DND on.
pub fn recover_if_needed() -> Option<DndOutcome> {
    let p = recovery_path()?;
    let text = std::fs::read_to_string(&p).ok()?;
    let rec: Recovery = toml::from_str(&text).ok()?;
    let mut wrote = 0;
    let mut expect = DndState::Off;
    for e in &rec.entries {
        if let Some(blob) = from_hex(&e.blob_hex) {
            let mut restored = blob.clone();
            if let Some(name) = profile::read(&blob) {
                expect = state_for_profile(&name);
                if let Ok(fresh) = profile::write(&blob, &name, now_unix()) {
                    restored = fresh;
                }
            }
            if cloudstore::write_blob(&e.path, &restored) {
                wrote += 1;
            }
        }
    }
    let _ = std::fs::remove_file(&p);
    if wrote == 0 {
        return Some(DndOutcome::Failed(
            "could not restore after an unclean exit",
        ));
    }
    Some(DndController::settle(Some(expect)))
}

/// Map a Windows profile identifier onto the state the OS should report.
fn state_for_profile(profile_id: &str) -> DndState {
    match profile_id {
        id if id == profile::UNRESTRICTED => DndState::Off,
        id if id == profile::PRIORITY_ONLY => DndState::PriorityOnly,
        _ => DndState::Off,
    }
}

/// What happened when we tried to change DND.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DndOutcome {
    /// Changed, and the OS confirms the new state.
    Applied(DndState),
    /// Registry write succeeded but the state could not be read back. The
    /// change has probably worked; we just cannot prove it on this build.
    Unverified,
    /// Nothing to do - already in the requested state.
    AlreadyCorrect,
    /// Could not change it. Carries a short reason for the UI/log.
    Failed(&'static str),
}

/// Owns our changes to the system DND state so they can be undone exactly.
///
/// The controller remembers the *original bytes* of every blob it modified.
/// Restoring writes those bytes back verbatim rather than reconstructing an
/// "off" blob, so anything else stored alongside the profile name survives.
#[derive(Default)]
pub struct DndController {
    /// Original blobs captured the first time we engaged, by registry path.
    saved: Vec<(String, Vec<u8>)>,
    /// True while we believe we are the reason DND is on.
    engaged: bool,
}

impl DndController {
    pub fn new() -> Self {
        Self::default()
    }

    /// Turn DND on.
    ///
    /// If the user already has *any* form of Do Not Disturb switched on, we
    /// leave their setting completely alone and report `AlreadyCorrect`. They
    /// have asked for at least this much quiet; it is not our place to weaken
    /// it, and `release` will then correctly leave it on afterwards.
    pub fn engage(&mut self) -> DndOutcome {
        if self.engaged {
            return DndOutcome::AlreadyCorrect;
        }
        if wnf::query().is_some_and(DndState::is_on) {
            return DndOutcome::AlreadyCorrect;
        }
        self.apply(profile::PRIORITY_ONLY, Some(DndState::PriorityOnly), true)
    }

    /// Turn DND back off.
    ///
    /// With `restore_previous`, the exact bytes captured by [`engage`] are
    /// written back, so a user who was already in "Alarms only" before a focus
    /// session ends up back in "Alarms only" rather than being forced to "off".
    pub fn release(&mut self) -> DndOutcome {
        if !self.engaged {
            return DndOutcome::AlreadyCorrect;
        }
        let outcome = if !self.saved.is_empty() {
            let saved = std::mem::take(&mut self.saved);
            let mut wrote = 0;
            for (path, blob) in &saved {
                let restored = profile::read(blob)
                    .and_then(|name| profile::write(blob, &name, now_unix()).ok())
                    .unwrap_or_else(|| blob.clone());
                if cloudstore::write_blob(path, &restored) {
                    wrote += 1;
                }
            }
            if wrote == 0 {
                DndOutcome::Failed("could not restore the previous DND state")
            } else {
                let expect = profile::read(&saved[0].1)
                    .as_deref()
                    .map(state_for_profile)
                    .unwrap_or(DndState::Off);
                Self::settle(Some(expect))
            }
        } else {
            self.apply(profile::UNRESTRICTED, Some(DndState::Off), false)
        };
        self.engaged = false;
        self.saved.clear();
        clear_recovery();
        outcome
    }

    /// Core write path shared by engage/release.
    fn apply(&mut self, profile_id: &str, expect: Option<DndState>, remember: bool) -> DndOutcome {
        let targets = cloudstore::discover();
        if targets.is_empty() {
            return DndOutcome::Failed("quiet-hours settings not found in the registry");
        }

        let mut wrote = 0usize;
        for t in &targets {
            let new_blob = match profile::write(&t.blob, profile_id, now_unix()) {
                Ok(b) => b,

                Err(_) => continue,
            };
            if cloudstore::write_blob(&t.path, &new_blob) {
                if remember && !self.saved.iter().any(|(p, _)| p == &t.path) {
                    self.saved.push((t.path.clone(), t.blob.clone()));
                }
                wrote += 1;
            }
        }

        if wrote == 0 {
            return DndOutcome::Failed("could not write the quiet-hours setting");
        }
        if remember {
            self.engaged = true;

            persist_recovery(&self.saved);
        }

        let outcome = Self::settle(expect);

        if let (DndOutcome::Failed(_), true) = (&outcome, remember) {
            self.rollback();
        }
        outcome
    }

    /// Restart the notification service and confirm the new state.
    ///
    /// `expect` is mandatory in spirit: right after the service restarts, the
    /// WNF value is still the *old* one for a short while, so accepting the
    /// first reading we get would report success for a change that never
    /// happened. We poll until it matches or the deadline passes.
    fn settle(expect: Option<DndState>) -> DndOutcome {
        if service::restart_notification_service() == 0 {
            return DndOutcome::Failed("could not restart the notification service");
        }

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut last = None;
        loop {
            last = wnf::query().or(last);
            if let (Some(got), Some(want)) = (last, expect) {
                if got == want {
                    return DndOutcome::Applied(got);
                }
            }
            if std::time::Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        match (last, expect) {
            (None, _) => DndOutcome::Unverified,

            (Some(_), Some(_)) => DndOutcome::Failed("Windows did not accept the DND change"),
            (Some(got), None) => DndOutcome::Applied(got),
        }
    }

    /// Put back whatever we saved, ignoring further errors.
    fn rollback(&mut self) {
        for (path, blob) in std::mem::take(&mut self.saved) {
            let _ = cloudstore::write_blob(&path, &blob);
        }
        self.engaged = false;
    }
}

impl Drop for DndController {
    /// Never leave the machine muted because our process went away.
    ///
    /// This covers an orderly exit. A *killed* process never runs `Drop` at
    /// all, which is what the on-disk recovery file above is for.
    fn drop(&mut self) {
        if self.engaged {
            let _ = self.release();
        }
    }
}
