//! Reading and writing the quiet-hours state in the CloudStore registry area.
//!
//! ⚠ **UNDOCUMENTED.** See the warning block in `super` (`os::dnd`).
//!
//! The key path is *discovered*, never hardcoded, because the leaf name has
//! changed across Windows versions:
//!
//! ```text
//! Windows 10 / early 11 : ...$windows.data.notifications.quiethourssettings
//! Windows 11 22H2+      : ...$windows.data.donotdisturb.quiethourssettings
//! ```
//!
//! There is also normally more than one container - a `default$...` one and a
//! per-account `{GUID}$...` one - and they must be kept in agreement, so we
//! write every container we find.

use super::profile;
use crate::os::registry::Key;
use windows::Win32::System::Registry::{KEY_READ, KEY_SET_VALUE};

const BASE: &str =
    r"Software\Microsoft\Windows\CurrentVersion\CloudStore\Store\DefaultAccount\Current";
const VALUE: &str = "Data";

/// One writable location holding a quiet-hours blob.
#[derive(Clone, Debug)]
pub struct Target {
    pub path: String,
    pub blob: Vec<u8>,
}

/// Find every quiet-hours settings blob for the current user.
pub fn discover() -> Vec<Target> {
    let mut out = Vec::new();
    let Some(base) = Key::open_read(BASE) else {
        return out;
    };
    for container in base.subkeys() {
        if !container
            .to_ascii_lowercase()
            .contains("quiethourssettings")
        {
            continue;
        }

        let container_path = format!("{BASE}\\{container}");
        let Some(ck) = Key::open_read(&container_path) else {
            continue;
        };
        for child in ck.subkeys() {
            let path = format!("{container_path}\\{child}");
            if let Some(blob) = Key::open_read(&path).and_then(|k| k.get_binary(VALUE)) {
                if profile::read(&blob).is_some() {
                    out.push(Target { path, blob });
                }
            }
        }
    }
    out
}

/// The profile name currently recorded in the registry, if all targets agree.
///
/// This is what is *stored*; [`super::wnf::query`] is what is *in effect*. They
/// disagree between a write and the notification service restarting, which is
/// exactly the situation the verification step exists to catch.
#[allow(dead_code)]
pub fn current_profile() -> Option<String> {
    let targets = discover();
    let first = profile::read(&targets.first()?.blob)?;
    targets
        .iter()
        .all(|t| profile::read(&t.blob).as_deref() == Some(first.as_str()))
        .then_some(first)
}

/// Overwrite the `Data` value at `path`. Returns false if the key could not be
/// opened for writing or the write failed.
pub fn write_blob(path: &str, blob: &[u8]) -> bool {
    Key::open(path, KEY_READ | KEY_SET_VALUE)
        .map(|k| k.set_binary(VALUE, blob))
        .unwrap_or(false)
}
