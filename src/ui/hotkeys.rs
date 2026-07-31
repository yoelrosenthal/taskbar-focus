//! Global hotkeys.
//!
//! Parsing lives apart from registration so the accelerator grammar can be
//! unit-tested without a window handle.

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Accelerator {
    pub modifiers: u32,
    pub vk: u32,
}

/// Parse strings like `Ctrl+Alt+F`, `Ctrl+Shift+F9`, `Win+Alt+P`.
///
/// Returns `None` for an empty string (meaning "no binding") or anything we do
/// not understand, so a typo in the config disables one hotkey rather than
/// breaking startup.
pub fn parse(s: &str) -> Option<Accelerator> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut modifiers = 0u32;
    let mut key = None;
    for part in s.split('+') {
        let p = part.trim();
        if p.is_empty() {
            return None;
        }
        match p.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => modifiers |= MOD_CONTROL.0,
            "alt" => modifiers |= MOD_ALT.0,
            "shift" => modifiers |= MOD_SHIFT.0,
            "win" | "super" | "meta" => modifiers |= MOD_WIN.0,
            _ => {
                if key.is_some() {
                    return None;
                }
                key = Some(vk_from(p)?);
            }
        }
    }
    let vk = key?;

    if modifiers == 0 {
        return None;
    }
    Some(Accelerator { modifiers, vk })
}

fn vk_from(name: &str) -> Option<u32> {
    let upper = name.to_ascii_uppercase();
    let b = upper.as_bytes();
    if b.len() == 1 && (b[0].is_ascii_alphanumeric()) {
        return Some(b[0] as u32);
    }
    if let Some(n) = upper.strip_prefix('F') {
        if let Ok(n) = n.parse::<u32>() {
            if (1..=24).contains(&n) {
                return Some(VK_F1.0 as u32 + n - 1);
            }
        }
    }
    Some(match upper.as_str() {
        "SPACE" => VK_SPACE.0 as u32,
        "ENTER" | "RETURN" => VK_RETURN.0 as u32,
        "ESC" | "ESCAPE" => VK_ESCAPE.0 as u32,
        "TAB" => VK_TAB.0 as u32,
        "PAUSE" => VK_PAUSE.0 as u32,
        "INSERT" => VK_INSERT.0 as u32,
        "DELETE" | "DEL" => VK_DELETE.0 as u32,
        "HOME" => VK_HOME.0 as u32,
        "END" => VK_END.0 as u32,
        "PAGEUP" | "PGUP" => VK_PRIOR.0 as u32,
        "PAGEDOWN" | "PGDN" => VK_NEXT.0 as u32,
        _ => return None,
    })
}

/// Registers hotkeys and unregisters them on drop.
pub struct Hotkeys {
    hwnd: HWND,
    ids: Vec<i32>,
}

impl Hotkeys {
    pub fn new(hwnd: HWND) -> Self {
        Hotkeys {
            hwnd,
            ids: Vec::new(),
        }
    }

    /// Returns false if Windows refused the binding, which normally means
    /// another application already owns that combination.
    pub fn register(&mut self, id: i32, accel: Accelerator) -> bool {
        let ok = unsafe {
            RegisterHotKey(
                Some(self.hwnd),
                id,
                HOT_KEY_MODIFIERS(accel.modifiers | MOD_NOREPEAT.0),
                accel.vk,
            )
        }
        .is_ok();
        if ok {
            self.ids.push(id);
        }
        ok
    }

    pub fn unregister_all(&mut self) {
        for id in self.ids.drain(..) {
            unsafe {
                let _ = UnregisterHotKey(Some(self.hwnd), id);
            }
        }
    }
}

impl Drop for Hotkeys {
    fn drop(&mut self) {
        self.unregister_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_combinations() {
        let a = parse("Ctrl+Alt+F").unwrap();
        assert_eq!(a.modifiers, MOD_CONTROL.0 | MOD_ALT.0);
        assert_eq!(a.vk, b'F' as u32);

        let b = parse("ctrl+shift+f9").unwrap();
        assert_eq!(b.modifiers, MOD_CONTROL.0 | MOD_SHIFT.0);
        assert_eq!(b.vk, VK_F9.0 as u32);

        assert_eq!(parse("Win+Alt+P").unwrap().modifiers, MOD_WIN.0 | MOD_ALT.0);
    }

    #[test]
    fn a_modifier_is_required() {
        assert_eq!(parse("F"), None);
        assert_eq!(parse("F5"), None);
    }
}
