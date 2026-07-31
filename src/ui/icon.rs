//! Drawing the tray icon.
//!
//! The icon is generated at runtime rather than shipped as a resource, because
//! it has to show progress through the current interval. It is rasterised by
//! hand into a 32-bit DIB - no GDI+ or image crates - which keeps the binary
//! tiny and gives us proper antialiasing via supersampling.
//!
//! The icon encodes three things at a glance:
//!   * **colour**  - focus (warm red), break (green), idle (grey)
//!   * **the ring** - how much of the interval has elapsed
//!   * **opacity**  - faded to 45% while paused
//!
//! Everything here is tuned for **16 pixels**, which is what the notification
//! area actually renders. Designs that look good enlarged tend not to survive
//! that: the first version used a thick ring over a dark track with notches cut
//! out for "paused", and at real size it was an indistinct blob that broke into
//! fragments when paused.

use windows::Win32::Graphics::Gdi::{
    CreateBitmap, DeleteObject, GetDC, GetDeviceCaps, ReleaseDC, LOGPIXELSX,
};
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, DestroyIcon, HICON, ICONINFO};

use crate::timer::{Phase, State};

/// Supersampling factor per axis; 4 means 16 samples per pixel.
const SS: i32 = 4;

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Visual {
    pub rgb: (u8, u8, u8),
    pub progress: f32,
    pub paused: bool,
    pub idle: bool,
    /// Overall opacity, used to fade the whole icon when paused.
    ///
    /// Fading with alpha rather than mixing the colour towards grey matters:
    /// the taskbar can be light or dark and we cannot know which, so a blend
    /// towards a fixed grey looks muddy on one of them. Alpha fades correctly
    /// against whatever is behind it.
    pub alpha: f32,
}

/// The ring is only ~16 pixels across, so there is no visible difference
/// between 41.2% and 41.9%. Snapping progress to a fixed number of steps means
/// the icon is rebuilt roughly once every `1/STEPS` of the interval instead of
/// twice a second, which removes a constant, faintly shimmering redraw of the
/// tray icon - and a lot of pointless GDI work.
const PROGRESS_STEPS: f32 = 60.0;

fn quantise(progress: f32) -> f32 {
    (progress.clamp(0.0, 1.0) * PROGRESS_STEPS).round() / PROGRESS_STEPS
}

impl Visual {
    /// Map timer state onto how the icon should look.
    pub fn from_state(state: &State) -> Visual {
        match state {
            State::Idle => Visual {
                rgb: (0x8A, 0x8E, 0x94),
                progress: 1.0,
                paused: false,
                idle: true,
                alpha: 1.0,
            },
            State::Running { phase, .. } => Visual {
                rgb: phase_color(*phase),
                progress: quantise(state.progress()),
                paused: false,
                idle: false,
                alpha: 1.0,
            },

            State::Paused { phase, .. } => Visual {
                rgb: phase_color(*phase),
                progress: quantise(state.progress()),
                paused: true,
                idle: false,
                alpha: 0.45,
            },
        }
    }
}

fn phase_color(p: Phase) -> (u8, u8, u8) {
    match p {
        Phase::Focus => (0xE8, 0x56, 0x3F),
        Phase::ShortBreak | Phase::LongBreak => (0x35, 0xC4, 0x6A),
    }
}

/// An owned `HICON` that destroys itself, so swapping the tray icon every
/// second cannot leak GDI handles.
pub struct OwnedIcon(HICON);

impl OwnedIcon {
    pub fn handle(&self) -> HICON {
        self.0
    }
}

impl Drop for OwnedIcon {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = DestroyIcon(self.0);
            }
        }
    }
}

/// The tray icon size Windows wants at the current DPI.
pub fn tray_icon_size() -> i32 {
    unsafe {
        let dc = GetDC(None);
        let dpi = if dc.is_invalid() {
            96
        } else {
            let d = GetDeviceCaps(Some(dc), LOGPIXELSX);
            ReleaseDC(None, dc);
            if d <= 0 {
                96
            } else {
                d
            }
        };

        ((16 * dpi) / 96).clamp(16, 64)
    }
}

/// Rasterise the icon. Returns `None` if Windows refuses to make the bitmaps.
pub fn render(v: Visual, size: i32) -> Option<OwnedIcon> {
    let size = size.max(8);
    let pixels = rasterise(v, size);

    unsafe {
        let color = CreateBitmap(size, size, 1, 32, Some(pixels.as_ptr() as *const _));
        if color.is_invalid() {
            return None;
        }

        let mask = CreateBitmap(size, size, 1, 1, None);
        if mask.is_invalid() {
            let _ = DeleteObject(color.into());
            return None;
        }

        let info = ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        };
        let icon = CreateIconIndirect(&info);
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon.ok().map(OwnedIcon)
    }
}

/// Tunable proportions of the ring. Split out so the tests can render
/// alternatives side by side at the real 16px tray size - what looks good
/// blown up to 64px is not the same thing at all.
#[derive(Clone, Copy)]
struct Style {
    /// Ring thickness as a fraction of the icon size.
    thickness: f32,
    /// Opacity of the unfilled part of the ring.
    track_alpha: f32,
    track: (u8, u8, u8),
}

impl Default for Style {
    fn default() -> Self {
        Style {
            thickness: 0.22,
            track_alpha: 0.22,
            track: (0x9A, 0x9F, 0xA6),
        }
    }
}

/// Produce premultiplied BGRA pixels, top-down.
fn rasterise(v: Visual, size: i32) -> Vec<u8> {
    rasterise_with(v, size, Style::default())
}

fn rasterise_with(v: Visual, size: i32, style: Style) -> Vec<u8> {
    let n = size as usize;
    let mut out = vec![0u8; n * n * 4];

    let c = (size as f32 - 1.0) / 2.0;
    let r_out = size as f32 / 2.0 - 0.5;
    let thickness = (size as f32 * style.thickness).max(2.0);
    let r_in = (r_out - thickness).max(1.0);

    let track = style.track;
    let track_alpha = if v.idle { 0.0 } else { style.track_alpha };

    let sweep = v.progress.clamp(0.0, 1.0) * std::f32::consts::TAU;
    let fade = v.alpha.clamp(0.0, 1.0);

    for y in 0..size {
        for x in 0..size {
            let mut acc = [0f32; 4];
            for sy in 0..SS {
                for sx in 0..SS {
                    let px = x as f32 + (sx as f32 + 0.5) / SS as f32 - 0.5;
                    let py = y as f32 + (sy as f32 + 0.5) / SS as f32 - 0.5;
                    let dx = px - c;
                    let dy = py - c;
                    let dist = (dx * dx + dy * dy).sqrt();
                    if dist > r_out || dist < r_in {
                        continue;
                    }

                    let mut ang = dx.atan2(-dy);
                    if ang < 0.0 {
                        ang += std::f32::consts::TAU;
                    }
                    let filled = v.idle || ang <= sweep;
                    let (rgb, a) = if filled {
                        (v.rgb, fade)
                    } else {
                        (track, track_alpha * fade)
                    };
                    acc[0] += rgb.2 as f32 * a;
                    acc[1] += rgb.1 as f32 * a;
                    acc[2] += rgb.0 as f32 * a;
                    acc[3] += a;
                }
            }
            let samples = (SS * SS) as f32;
            let idx = ((y as usize) * n + x as usize) * 4;

            out[idx] = (acc[0] / samples) as u8;
            out[idx + 1] = (acc[1] / samples) as u8;
            out[idx + 2] = (acc[2] / samples) as u8;
            out[idx + 3] = (acc[3] / samples * 255.0) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn paused_keeps_its_colour_but_fades() {
        let p = Visual::from_state(&State::Paused {
            phase: Phase::Focus,
            total: Duration::from_secs(60),
            remaining: Duration::from_secs(30),
        });
        assert!(p.paused);

        assert_eq!(p.rgb, phase_color(Phase::Focus));
        assert!(p.alpha < 1.0, "paused must be faded");

        let running = Visual::from_state(&State::Running {
            phase: Phase::Focus,
            total: Duration::from_secs(60),
            remaining: Duration::from_secs(30),
        });
        assert_eq!(running.alpha, 1.0);
        assert_ne!(p, running, "paused and running must be distinguishable");
    }

    #[test]
    fn progress_is_quantised_to_avoid_constant_redraws() {
        assert_eq!(quantise(0.412), quantise(0.419));
        assert_eq!(quantise(0.0), 0.0);
        assert_eq!(quantise(1.0), 1.0);
        assert!(quantise(0.10) < quantise(0.90));
    }
}
