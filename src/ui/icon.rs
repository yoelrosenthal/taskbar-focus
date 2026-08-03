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
//! This module also draws the **muted mark** - the struck-through bell Windows
//! uses for "notifications off" - but never on the timer icon. Windows will not
//! light its own indicator for a change the shell did not make (see
//! [`crate::os::dnd`]), so the app draws that mark itself, as an icon of its
//! own beside the clock and on the application icon. It is not squeezed into
//! the timer icon as well: sixteen pixels hold a progress ring or a bell, and
//! every attempt at both - filled, hollow, struck through the mark or through
//! the whole icon - came out a smudge.
//!
//! Everything here is tuned for **16 pixels**, which is what the notification
//! area actually renders. Designs that look good enlarged tend not to survive
//! that: the first version used a thick ring over a dark track with notches cut
//! out for "paused", and at real size it was an indistinct blob that broke into
//! fragments when paused.

use windows::core::PCWSTR;
use windows::Win32::Graphics::Gdi::{
    CreateBitmap, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GdiFlush, GetDC,
    GetDeviceCaps, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    LOGPIXELSX,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, DestroyIcon, DrawIconEx, LoadImageW, DI_NORMAL, HICON, ICONINFO,
    IMAGE_ICON, LR_DEFAULTCOLOR,
};

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
    icon_from_pixels(&rasterise(v, size), size)
}

/// The muted mark on its own, for the separate notification-area indicator.
///
/// Coloured for the current system theme rather than the app's palette: this
/// icon sits among the shell's own glyphs beside the clock, and a fixed colour
/// is invisible on one theme or the other.
pub fn render_mute_icon(size: i32) -> Option<OwnedIcon> {
    let size = size.max(8);
    icon_from_pixels(&mute_mark_pixels(size, taskbar_ink()), size)
}

/// The muted mark alone, as premultiplied BGRA pixels, top-down.
///
/// Shared with the compact window so that the mark is drawn by this one
/// rasteriser everywhere it appears. The window used to build it out of GDI
/// regions instead, which have no antialiasing and made a visibly jagged
/// version of the same shape.
///
/// The bell is hollow here, as the shell draws its own. A filled bell at this
/// size is a heavy blob; the outline is what makes it read as a bell rather
/// than a wedge.
pub fn mute_mark_pixels(size: i32, rgb: (u8, u8, u8)) -> Vec<u8> {
    let c = (size as f32 - 1.0) / 2.0;
    let mark = MuteMark::new(c, c, size as f32 / 2.0 - 0.5 - size as f32 * MARK_PADDING);
    let mut pixels = vec![0u8; (size * size * 4) as usize];
    draw_shape(&mut pixels, size, rgb, 1.0, |px, py| mark.covers(px, py));
    pixels
}

/// Breathing room around a mark that has a whole icon to itself, as a fraction
/// of the icon. The shell's own glyphs do not touch their edges either.
const MARK_PADDING: f32 = 0.02;

/// The application icon from the executable, with the muted mark burned in.
///
/// Composited onto the artwork that ships in the binary rather than a redrawn
/// substitute, so a muted window still wears the application's own icon.
pub fn app_icon_with_mute_mark(size: i32) -> Option<OwnedIcon> {
    let size = size.max(16);
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        let loaded = LoadImageW(
            Some(instance.into()),
            PCWSTR(1 as *const u16),
            IMAGE_ICON,
            size,
            size,
            LR_DEFAULTCOLOR,
        )
        .ok()?;
        let base = HICON(loaded.0);
        let pixels = icon_pixels(base, size);
        let _ = DestroyIcon(base);

        let mut pixels = pixels?;
        let s = size as f32;
        let (cx, cy) = (s * 0.70, s * 0.70);
        draw_shape(&mut pixels, size, BADGE_PAD, 0.92, |px, py| {
            let (dx, dy) = (px - cx, py - cy);
            dx * dx + dy * dy <= (s * 0.29) * (s * 0.29)
        });
        let mark = MuteMark::new(cx, cy, s * 0.24);
        draw_shape(&mut pixels, size, BADGE_INK, 1.0, |px, py| {
            mark.covers(px, py)
        });
        icon_from_pixels(&pixels, size)
    }
}

/// Dark pad drawn under the badge so the mark reads whatever it sits on.
const BADGE_PAD: (u8, u8, u8) = (0x10, 0x12, 0x1E);
const BADGE_INK: (u8, u8, u8) = (0xF2, 0xF4, 0xFF);

/// Build an `HICON` from premultiplied BGRA pixels, top-down.
fn icon_from_pixels(pixels: &[u8], size: i32) -> Option<OwnedIcon> {
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

/// A bell with a stroke through it: the mark Windows itself uses for
/// "notifications off", and the one people already read as muted.
///
/// The bell is hollow, matching the shell's own glyph and avoiding the heavy
/// blob produced by a filled bell at tray size.
#[derive(Clone, Copy)]
struct MuteMark {
    cx: f32,
    cy: f32,
    /// Half the width of the square the mark is drawn in.
    r: f32,
}

impl MuteMark {
    fn new(cx: f32, cy: f32, r: f32) -> Self {
        Self { cx, cy, r }
    }

    /// Is this sample point on the mark?
    ///
    /// Everything is expressed in a square running from -1 to 1 about the
    /// centre, so the same proportions hold at every size.
    fn covers(&self, px: f32, py: f32) -> bool {
        let u = (px - self.cx) / self.r;
        let v = (py - self.cy) / self.r;
        let detail = self.r >= DETAIL_PX;
        let distance = bell(u, v, detail);
        let body = distance <= 0.0 && distance >= -self.outline_width();
        if !detail {
            return body;
        }
        (body && stroke(u, v, self.gap()) > 0.0) || stroke(u, v, self.line()) <= 0.0
    }

    /// Line width of the outline, in normalised units, with a pixel floor for
    /// the same reason the stroke has one.
    fn outline_width(&self) -> f32 {
        OUTLINE_WIDTH.max(MIN_OUTLINE_PX / self.r)
    }

    /// Half-thickness of the stroke, never thinner than [`MIN_STROKE_PX`].
    ///
    /// The proportional width alone is what made the small icon look scratchy:
    /// at tray size it works out under a pixel, and a sub-pixel diagonal does
    /// not render as a line, it dithers into a grey smear.
    fn line(&self) -> f32 {
        STROKE_WIDTH.max(MIN_STROKE_PX / self.r)
    }

    /// Half-thickness of the clearance cut around the stroke, kept the same
    /// distance outside it however thick the stroke had to become.
    fn gap(&self) -> f32 {
        self.line() + (STROKE_GAP - STROKE_WIDTH)
    }
}

/// Half-thickness of the stroke, and of the clearance cut around it so the
/// stroke stays visible where it crosses the bell.
///
/// Both are as narrow as they can be: at 16 pixels a wide clearance saws the
/// bell into two unrecognisable fragments, which is what the first attempt did.
const STROKE_WIDTH: f32 = 0.075;
const STROKE_GAP: f32 = 0.155;
/// Radius, in pixels, below which the mark drops its stroke and its small
/// parts. Above it the mark is drawn in full; below, both would be sub-pixel
/// features that dither into static rather than reading as anything.
const DETAIL_PX: f32 = 5.6;
/// Half-thickness floor for the stroke, in pixels.
const MIN_STROKE_PX: f32 = 0.75;
/// Line width of the hollow bell, and its floor in pixels.
const OUTLINE_WIDTH: f32 = 0.17;
const MIN_OUTLINE_PX: f32 = 1.25;

/// The bell silhouette: a domed body on a base bar, with a handle above and a
/// clapper below.
///
/// Written as a signed distance - negative inside - because that is what lets
/// the same description be filled or outlined without describing the shape
/// twice, and unions are then just a minimum.
///
/// `detail` adds the handle above and the clapper below. They are dropped when
/// the mark is drawn smaller than [`DETAIL_PX`], where each would be a speck a
/// pixel or two across: too small to be recognised, big enough to fill the
/// shape with half-lit pixels and make the whole mark look like static.
fn bell(u: f32, v: f32, detail: bool) -> f32 {
    let disc = |cy: f32, r: f32| (u * u + (v - cy) * (v - cy)).sqrt() - r;
    let box_ = |cy: f32, hu: f32, hv: f32| {
        let (du, dv) = (u.abs() - hu, (v - cy).abs() - hv);
        du.max(dv).min(0.0) + (du.max(0.0).powi(2) + dv.max(0.0).powi(2)).sqrt()
    };

    let far = 9.0f32;
    let handle = if detail { disc(-0.88, 0.11) } else { far };
    let clapper = if detail { disc(0.82, 0.15) } else { far };
    let dome = disc(-0.22, 0.52);
    let base = box_(0.54, 0.74, if detail { 0.08 } else { 0.13 });

    let skirt = {
        const NU: f32 = 0.989;
        const NV: f32 = -0.146;
        let side = NU * (u.abs() - 0.52) + NV * (v + 0.22);
        side.max(-0.22 - v).max(v - 0.46)
    };

    handle.min(dome).min(skirt).min(base).min(clapper)
}

/// A stroke of half-thickness `half` running from the lower left to the upper
/// right, the way the system's own muted mark is struck through.
fn stroke(u: f32, v: f32, half: f32) -> f32 {
    const DIAGONAL: f32 = std::f32::consts::FRAC_1_SQRT_2;
    let across = ((u + v) * DIAGONAL).abs() - half;
    let along = ((u - v) * DIAGONAL).abs() - 0.98;
    across.max(along)
}

/// Paint a shape into a premultiplied BGRA buffer, antialiased by the same
/// supersampling the ring uses.
fn draw_shape(
    out: &mut [u8],
    size: i32,
    rgb: (u8, u8, u8),
    alpha: f32,
    inside: impl Fn(f32, f32) -> bool,
) {
    let n = size as usize;
    let samples = (SS * SS) as f32;
    for y in 0..size {
        for x in 0..size {
            let mut hits = 0;
            for sy in 0..SS {
                for sx in 0..SS {
                    let px = x as f32 + (sx as f32 + 0.5) / SS as f32 - 0.5;
                    let py = y as f32 + (sy as f32 + 0.5) / SS as f32 - 0.5;
                    if inside(px, py) {
                        hits += 1;
                    }
                }
            }
            if hits > 0 {
                let idx = ((y as usize) * n + x as usize) * 4;
                blend_over(out, idx, rgb, alpha * hits as f32 / samples);
            }
        }
    }
}

/// Composite one premultiplied sample over the pixel at `idx`.
fn blend_over(out: &mut [u8], idx: usize, rgb: (u8, u8, u8), a: f32) {
    let a = a.clamp(0.0, 1.0);
    if a <= 0.0 {
        return;
    }
    let inv = 1.0 - a;
    for (i, channel) in [rgb.2, rgb.1, rgb.0, 0xFF].into_iter().enumerate() {
        let src = channel as f32 * a;
        out[idx + i] = (src + out[idx + i] as f32 * inv).min(255.0) as u8;
    }
}

/// The 32-bit pixels of an existing icon, premultiplied and top-down.
///
/// Drawing the icon into a transparent DIB section is what makes this
/// premultiplied: `DrawIconEx` composites 32-bit icons with `AlphaBlend`, which
/// works in premultiplied space, so the result drops straight into the same
/// pipeline as everything else here.
unsafe fn icon_pixels(icon: HICON, size: i32) -> Option<Vec<u8>> {
    let screen = GetDC(None);
    let dc = CreateCompatibleDC(Some(screen));
    let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size,
            biHeight: -size,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let bmp = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0);

    let out = (|| {
        let bmp = bmp.ok()?;
        if bits.is_null() {
            let _ = DeleteObject(bmp.into());
            return None;
        }
        let old = SelectObject(dc, bmp.into());
        let drawn = DrawIconEx(dc, 0, 0, icon, size, size, 0, None, DI_NORMAL).is_ok();
        let _ = GdiFlush();
        SelectObject(dc, old);

        let out = drawn.then(|| {
            std::slice::from_raw_parts(bits as *const u8, (size * size * 4) as usize).to_vec()
        });
        let _ = DeleteObject(bmp.into());
        out
    })();

    let _ = DeleteDC(dc);
    ReleaseDC(None, screen);
    out
}

/// Ink for a glyph that has to stay legible on this user's taskbar.
///
/// The notification area follows the *system* theme, which is a documented
/// registry value; guessing produces a mark that is invisible on one theme.
fn taskbar_ink() -> (u8, u8, u8) {
    let light = crate::os::registry::Key::open_read(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize",
    )
    .and_then(|k| k.get_u32("SystemUsesLightTheme"))
    .is_some_and(|v| v != 0);
    if light {
        (0x2B, 0x30, 0x42)
    } else {
        (0xE8, 0xEC, 0xFA)
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
