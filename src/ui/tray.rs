//! The notification-area (tray) icon.
//!
//! Notifications are delivered as tray balloons via `Shell_NotifyIcon`. On
//! Windows 10/11 the shell renders these as ordinary toasts in the Action
//! Centre, which means they follow the user's notification settings - including
//! being suppressed by Do Not Disturb - without us needing to register an
//! AppUserModelID or install a Start Menu shortcut.

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_NOSOUND, NIM_ADD, NIM_DELETE,
    NIM_MODIFY, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::HICON;

/// Copy `s` into a fixed-size wide buffer, truncating safely.
fn fill(buf: &mut [u16], s: &str) {
    let src: Vec<u16> = s.encode_utf16().take(buf.len().saturating_sub(1)).collect();
    buf[..src.len()].copy_from_slice(&src);
    buf[src.len()] = 0;
    for c in buf.iter_mut().skip(src.len() + 1) {
        *c = 0;
    }
}

pub struct Tray {
    hwnd: HWND,
    id: u32,
    callback: u32,
    added: bool,
}

impl Tray {
    pub fn new(hwnd: HWND, id: u32, callback: u32) -> Self {
        Tray {
            hwnd,
            id,
            callback,
            added: false,
        }
    }

    fn base(&self) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: self.id,
            uCallbackMessage: self.callback,
            ..Default::default()
        }
    }

    /// Add or update the icon and tooltip.
    pub fn set(&mut self, icon: HICON, tooltip: &str) {
        let mut data = self.base();
        data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        data.hIcon = icon;
        fill(&mut data.szTip, tooltip);
        let msg = if self.added { NIM_MODIFY } else { NIM_ADD };
        let ok = unsafe { Shell_NotifyIconW(msg, &data) }.as_bool();
        if ok {
            self.added = true;
        } else if self.added {
            self.added = false;
        }
    }

    /// Show a notification. `NIIF_NOSOUND` because we play our own cue, and
    /// letting Windows add a second one is grating.
    pub fn notify(&self, title: &str, body: &str) {
        if !self.added {
            return;
        }
        let mut data = self.base();
        data.uFlags = NIF_INFO;
        data.dwInfoFlags = NIIF_NOSOUND;
        fill(&mut data.szInfoTitle, title);
        fill(&mut data.szInfo, body);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
        }
    }

    /// Called after Explorer restarts, which silently drops every tray icon.
    pub fn forget(&mut self) {
        self.added = false;
    }

    pub fn remove(&mut self) {
        if self.added {
            let data = self.base();
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &data);
            }
            self.added = false;
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.remove();
    }
}
