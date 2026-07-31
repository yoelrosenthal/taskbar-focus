//! Sound cues.
//!
//! We deliberately use the *system* sound aliases rather than shipping audio
//! files: they respect the user's sound scheme, honour "mute system sounds",
//! and cost nothing in binary size. `SND_NODEFAULT` means a user who has set an
//! alias to "(None)" gets silence rather than the generic beep.

use crate::orchestrator::Event;
use crate::os::registry::wide;
use windows::core::PCWSTR;
use windows::Win32::Media::Audio::{PlaySoundW, SND_ALIAS, SND_ASYNC, SND_NODEFAULT};

fn alias_for(event: Event) -> &'static str {
    match event {
        Event::FocusStart | Event::BreakStart => "SystemAsterisk",

        Event::FocusEnd | Event::BreakEnd => "SystemExclamation",
    }
}

/// Play the cue for `event`. Never blocks and never fails loudly.
pub fn play(event: Event) {
    let w = wide(alias_for(event));
    unsafe {
        let _ = PlaySoundW(
            PCWSTR(w.as_ptr()),
            None,
            SND_ALIAS | SND_ASYNC | SND_NODEFAULT,
        );
    }
}
