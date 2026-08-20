//! Windows scheme aliases for timer cues.
//!
//! `PlaySound` honours the user's sound scheme and "mute system sounds". There
//! is no per-app volume for aliases, so loudness stays under Windows.

use windows::core::PCWSTR;
use windows::Win32::Media::Audio::{PlaySoundW, SND_ALIAS, SND_ASYNC, SND_NODEFAULT};

use crate::os::registry::wide;

/// Play the single timer cue. Never blocks the UI thread.
pub fn play() {
    let w = wide("SystemAsterisk");
    unsafe {
        let _ = PlaySoundW(
            PCWSTR(w.as_ptr()),
            None,
            SND_ALIAS | SND_ASYNC | SND_NODEFAULT,
        );
    }
}
