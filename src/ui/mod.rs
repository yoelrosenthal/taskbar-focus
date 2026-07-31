//! The Win32 user interface: tray icon, taskbar title window, settings.
//!
//! This layer owns no policy. It turns clicks, hotkeys and timer ticks into
//! [`crate::cli::Command`]s for the orchestrator, and renders whatever
//! [`crate::orchestrator::UiEvent`]s come back.

pub mod app;
pub mod hotkeys;
pub mod icon;
pub mod mini;
pub mod settings;
pub mod tray;

use windows::core::PCWSTR;

/// A NUL-terminated UTF-16 string that stays alive as long as you hold it.
pub struct WideStr(Vec<u16>);

impl WideStr {
    pub fn new(s: &str) -> Self {
        WideStr(s.encode_utf16().chain(std::iter::once(0)).collect())
    }

    pub fn as_pcwstr(&self) -> PCWSTR {
        PCWSTR(self.0.as_ptr())
    }
}

pub fn wide(s: &str) -> WideStr {
    WideStr::new(s)
}
