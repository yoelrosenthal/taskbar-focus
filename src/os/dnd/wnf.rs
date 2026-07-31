//! Reading the *effective* quiet-hours state that the shell is actually using.
//!
//! ⚠ **UNDOCUMENTED.** This uses `NtQueryWnfStateData` from `ntdll` against a
//! well-known WNF (Windows Notification Facility) state name. It is read-only
//! and failure-tolerant: if the state name ever changes, every function here
//! returns `None` and the app simply loses its ability to *verify* that a DND
//! change took effect. Nothing else breaks.
//!
//! Why not the documented `Windows.UI.Shell.FocusSessionManager`? Because it
//! reports **Focus sessions** (the timed "Focus" feature driven by the Clock
//! app), which is a *different* feature from the "Do not disturb" notification
//! silencer. `IsFocusActive` stays `false` while DND is on, so it is the wrong
//! oracle for this job - a trap worth documenting.
//!
//! Observed on Windows 11 Pro 26200: toggling DND moves this value between 0
//! and 1 exactly as described below.

use windows::core::PCSTR;
use windows::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};

/// `WNF_SHEL_QUIETHOURS_ACTIVE_PROFILE_CHANGED` - the restrictive level of the
/// currently active quiet-hours profile.
const WNF_QUIETHOURS_ACTIVE_PROFILE: u64 = 0x0D83063EA3BF1C75;

type NtQueryWnfStateDataFn =
    unsafe extern "system" fn(*const u64, *const u8, *const u8, *mut u32, *mut u8, *mut u32) -> i32;

/// The effective notification-suppression level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DndState {
    /// Notifications flow normally.
    Off,
    /// Suppressed, except apps/contacts the user marked as priority.
    PriorityOnly,
    /// Suppressed, except alarms.
    AlarmsOnly,
    /// A level this build reports that we do not have a name for.
    Unknown(u32),
}

impl DndState {
    pub fn is_on(self) -> bool {
        !matches!(self, DndState::Off)
    }

    fn from_raw(v: u32) -> Self {
        match v {
            0 => DndState::Off,
            1 => DndState::PriorityOnly,
            2 => DndState::AlarmsOnly,
            other => DndState::Unknown(other),
        }
    }
}

/// Query the live state. `None` means the query failed (wrong build, state name
/// changed, access denied) - treat that as "cannot verify", not as "off".
pub fn query() -> Option<DndState> {
    unsafe {
        let ntdll = GetModuleHandleA(PCSTR(c"ntdll.dll".as_ptr() as *const u8)).ok()?;
        let proc = GetProcAddress(ntdll, PCSTR(c"NtQueryWnfStateData".as_ptr() as *const u8))?;
        let f: NtQueryWnfStateDataFn = std::mem::transmute(proc);

        let mut buf = [0u8; 16];
        let mut size = buf.len() as u32;
        let mut stamp = 0u32;
        let status = f(
            &WNF_QUIETHOURS_ACTIVE_PROFILE,
            std::ptr::null(),
            std::ptr::null(),
            &mut stamp,
            buf.as_mut_ptr(),
            &mut size,
        );
        if status < 0 || size < 4 {
            return None;
        }
        Some(DndState::from_raw(u32::from_le_bytes([
            buf[0], buf[1], buf[2], buf[3],
        ])))
    }
}
