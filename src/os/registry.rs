//! Thin, safe-ish wrappers over the handful of registry calls this app needs.

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::System::Registry::*;

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// RAII wrapper so early returns cannot leak an `HKEY`.
pub struct Key(HKEY);

impl Key {
    pub fn open(path: &str, access: REG_SAM_FLAGS) -> Option<Key> {
        let w = wide(path);
        let mut h = HKEY::default();
        unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(w.as_ptr()), None, access, &mut h) }
            .is_ok()
            .then_some(Key(h))
    }

    pub fn open_read(path: &str) -> Option<Key> {
        Key::open(path, KEY_READ)
    }

    /// Names of the immediate child keys.
    pub fn subkeys(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut i = 0u32;
        loop {
            let mut buf = [0u16; 512];
            let mut len = buf.len() as u32;
            let rc = unsafe {
                RegEnumKeyExW(
                    self.0,
                    i,
                    Some(PWSTR(buf.as_mut_ptr())),
                    &mut len,
                    None,
                    None,
                    None,
                    None,
                )
            };
            if rc.is_err() {
                break;
            }
            out.push(String::from_utf16_lossy(&buf[..len as usize]));
            i += 1;
        }
        out
    }

    pub fn get_binary(&self, value: &str) -> Option<Vec<u8>> {
        let v = wide(value);
        let mut size = 0u32;
        let mut ty = REG_VALUE_TYPE::default();
        unsafe {
            RegQueryValueExW(
                self.0,
                PCWSTR(v.as_ptr()),
                None,
                Some(&mut ty),
                None,
                Some(&mut size),
            )
            .ok()
            .ok()?;
            let mut buf = vec![0u8; size as usize];
            RegQueryValueExW(
                self.0,
                PCWSTR(v.as_ptr()),
                None,
                Some(&mut ty),
                Some(buf.as_mut_ptr()),
                Some(&mut size),
            )
            .ok()
            .ok()?;
            buf.truncate(size as usize);
            Some(buf)
        }
    }

    pub fn set_binary(&self, value: &str, data: &[u8]) -> bool {
        let v = wide(value);
        unsafe { RegSetValueExW(self.0, PCWSTR(v.as_ptr()), None, REG_BINARY, Some(data)) }.is_ok()
    }
}

impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}
