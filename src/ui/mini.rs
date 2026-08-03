//! The compact timer window.
//!
//! A small, resizable, optionally always-on-top readout of the countdown. It is
//! the dependable alternative to the taskbar-title trick: a real window, so it
//! shows what it is told to show, and it can be shrunk to roughly the height of
//! a taskbar button if all you want is the number.
//!
//! Everything is drawn by hand into an off-screen bitmap and blitted in one go.
//! Painting directly to the window would flicker badly on a surface that
//! repaints twice a second.

use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::timer::{Phase, State};
use crate::ui::app::App;
use crate::ui::wide;

const CLASS: &str = "TaskbarFocusMini";

/// Small enough to sit beside taskbar buttons, large enough to stay readable.
/// There is no caption bar, so the whole height is usable.
const MIN_W: i32 = 96;
const MIN_H: i32 = 34;

/// Default size: a strip that sits comfortably alongside taskbar buttons.
///
/// Deliberately below [`LABEL_MIN_H`], so out of the box it uses the
/// side-by-side layout - phase on the left, clock on the right - which is what
/// reads best at this height. Whatever the user resizes it to is remembered.
const DEFAULT_W: i32 = 210;
const DEFAULT_H: i32 = 44;

/// Below this height the phase label is dropped so the countdown gets the
/// entire window.
const LABEL_MIN_H: i32 = 58;

/// How far the pointer may travel before a press counts as a drag rather than
/// a click.
const DRAG_SLOP: i32 = 4;

/// Width of the invisible band along each edge that resizes the window.
const RESIZE_MARGIN: i32 = 6;

const BG: u32 = 0x1C1A18;
const BG_IDLE: u32 = 0x24211F;
const TEXT: u32 = 0xF2F2F0;
const TEXT_DIM: u32 = 0x9A9490;
const TRACK: u32 = 0x38332F;
const BORDER: u32 = 0x4A4440;

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF((b as u32) << 16 | (g as u32) << 8 | r as u32)
}

fn phase_accent(p: Phase) -> COLORREF {
    match p {
        Phase::Focus => rgb(0xE8, 0x56, 0x3F),
        Phase::ShortBreak | Phase::LongBreak => rgb(0x35, 0xC4, 0x6A),
    }
}

struct State_ {
    app: *mut App,
    /// Cached fonts, rebuilt only when the window shape or the string changes.
    font: HFONT,
    font_small: HFONT,
    /// `(width, height, label shown, character count)` the fonts were fitted to.
    fitted_for: (i32, i32, bool, i32),
}

/// Where the window sits the first time it opens: on the taskbar, at its left
/// end, vertically centred in the strip.
///
/// Measured from the live taskbar rather than guessed, so it lands correctly
/// whatever the taskbar's height, and wherever the taskbar is. `CW_USEDEFAULT`
/// is no use here - for a popup window it simply means the top-left corner of
/// the screen, which is where this used to end up.
unsafe fn default_placement(w: i32, h: i32) -> (i32, i32) {
    let tray_class = wide("Shell_TrayWnd");
    if let Ok(taskbar) = FindWindowW(tray_class.as_pcwstr(), PCWSTR::null()) {
        if !taskbar.is_invalid() {
            let mut tb = RECT::default();
            if GetWindowRect(taskbar, &mut tb).is_ok() && tb.right > tb.left {
                let x = (tb.left + 12).min(tb.right - w);
                let y = tb.top + ((tb.bottom - tb.top) - h) / 2;
                return (x, y);
            }
        }
    }

    let mut work = RECT::default();
    let _ = SystemParametersInfoW(
        SPI_GETWORKAREA,
        0,
        Some(&mut work as *mut _ as *mut _),
        SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
    );
    if work.right > work.left {
        (work.left + 12, (work.bottom - h - 16).max(0))
    } else {
        (100, 100)
    }
}

/// Create the compact window. Returns `None` if creation fails.
pub fn open(app: *mut App, geometry: Option<[i32; 4]>, topmost: bool) -> Option<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        let class = wide(CLASS);
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: class.as_pcwstr(),
            hCursor: LoadCursorW(None, IDC_ARROW).ok()?,
            hIcon: crate::ui::app::shared_icon(),
            ..Default::default()
        };
        RegisterClassW(&wc);

        let (x, y, w, h) = match geometry {
            Some([x, y, w, h]) => (x, y, w.max(MIN_W), h.max(MIN_H)),
            None => {
                let (x, y) = default_placement(DEFAULT_W, DEFAULT_H);
                (x, y, DEFAULT_W, DEFAULT_H)
            }
        };

        // WS_EX_TOOLWINDOW keeps this floating readout out of the taskbar and
        // out of Alt+Tab; it is a widget, not a document window.
        let mut ex = WS_EX_TOOLWINDOW;
        if topmost {
            ex |= WS_EX_TOPMOST;
        }
        let title = wide("Focus");
        let hwnd = CreateWindowExW(
            ex,
            class.as_pcwstr(),
            title.as_pcwstr(),
            WS_POPUP | WS_THICKFRAME | WS_VISIBLE,
            x,
            y,
            w,
            h,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .ok()?;

        let st = Box::new(State_ {
            app,
            font: HFONT::default(),
            font_small: HFONT::default(),
            fitted_for: (0, 0, false, 0),
        });
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(st) as isize);

        // Rounded corners to match Windows 11. Ignored on Windows 10, which
        // simply keeps square corners.
        let pref = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &pref as *const _ as *const _,
            std::mem::size_of_val(&pref) as u32,
        );

        Some(hwnd)
    }
}

/// Apply the always-on-top setting to an existing window.
pub fn set_topmost(hwnd: HWND, topmost: bool) {
    unsafe {
        let after = if topmost {
            HWND_TOPMOST
        } else {
            HWND_NOTOPMOST
        };
        let _ = SetWindowPos(
            hwnd,
            Some(after),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

/// Ask the window to repaint; called on every tick.
///
/// When `topmost` is set this also re-asserts the position in the z-order. The
/// Windows taskbar is itself a top-most window, so clicking it raises it above
/// anything else in that band and the compact window vanishes behind it.
/// Re-applying `HWND_TOPMOST` puts it back; `SWP_NOACTIVATE` means focus is
/// never stolen from whatever the user is actually working in.
pub fn refresh(hwnd: HWND, topmost: bool) {
    unsafe {
        if topmost {
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

/// Current position and size, for persisting to the config.
pub fn geometry(hwnd: HWND) -> Option<[i32; 4]> {
    unsafe {
        let mut r = RECT::default();
        GetWindowRect(hwnd, &mut r).ok()?;
        Some([r.left, r.top, r.right - r.left, r.bottom - r.top])
    }
}

unsafe fn make_font(height: i32, weight: i32) -> HFONT {
    let face = wide("Segoe UI");
    let mut lf = LOGFONTW {
        lfHeight: -height,
        lfWeight: weight,
        lfQuality: CLEARTYPE_QUALITY,
        ..Default::default()
    };
    let name: Vec<u16> = face.as_pcwstr().as_wide().to_vec();
    for (i, c) in name.iter().take(31).enumerate() {
        lf.lfFaceName[i] = *c;
    }
    CreateFontIndirectW(&lf)
}

/// Largest font that renders `text` within both `max_h` and `max_w`.
///
/// Sizing on height alone was the original bug: a tall narrow window produced a
/// number too wide for the window and the last digit was clipped. Text width
/// scales almost linearly with font height, so one measure-and-correct pass
/// lands within a pixel or two; a second pass covers rounding.
unsafe fn fit_font(dc: HDC, text: &str, max_h: i32, max_w: i32, weight: i32) -> HFONT {
    let wide_text: Vec<u16> = text.encode_utf16().collect();
    let mut size = max_h.clamp(8, 200);
    let mut font = make_font(size, weight);

    for _ in 0..2 {
        let old = SelectObject(dc, font.into());
        let mut ext = SIZE::default();
        let ok = GetTextExtentPoint32W(dc, &wide_text, &mut ext).as_bool();
        SelectObject(dc, old);
        if !ok || ext.cx <= max_w || ext.cx == 0 {
            break;
        }
        let scaled = (size as i64 * max_w as i64 / ext.cx as i64) as i32;
        let next = scaled.clamp(8, 200);
        if next >= size {
            break;
        }
        let _ = DeleteObject(font.into());
        size = next;
        font = make_font(size, weight);
    }
    font
}

/// Width of `text` in the currently selected font.
unsafe fn text_width(dc: HDC, font: HFONT, text: &str) -> i32 {
    let units: Vec<u16> = text.encode_utf16().collect();
    let old = SelectObject(dc, font.into());
    let mut ext = SIZE::default();
    let ok = GetTextExtentPoint32W(dc, &units, &mut ext).as_bool();
    SelectObject(dc, old);
    if ok {
        ext.cx
    } else {
        0
    }
}

/// Descriptions of the current phase, longest first.
///
/// The window can be narrower than the word, so the caller picks the longest
/// one that fits rather than letting it ellipsize into "Focus paus...". Being
/// paused is *not* said here: it is drawn as an icon beside the clock, which
/// survives at sizes where no text does.
fn label_variants(state: &State) -> Vec<String> {
    match state {
        State::Idle => vec!["Idle".into()],
        State::Running { phase, .. } | State::Paused { phase, .. } => match phase {
            Phase::Focus => vec!["Focus".into()],
            _ => vec![phase.label().to_string(), "Break".into()],
        },
    }
}

/// Two upright bars: the pause icon, drawn beside the clock.
unsafe fn pause_glyph(dc: HDC, x: i32, cy: i32, size: i32, colour: COLORREF) {
    let bar = (size as f32 * 0.30).round().max(2.0) as i32;
    let gap = (size as f32 * 0.26).round().max(2.0) as i32;
    let top = cy - size / 2;
    let brush = CreateSolidBrush(colour);
    for i in 0..2 {
        let left = x + i * (bar + gap);
        let r = RECT {
            left,
            top,
            right: left + bar,
            bottom: top + size,
        };
        FillRect(dc, &r, brush);
    }
    let _ = DeleteObject(brush.into());
}

/// Width the pause icon needs, including the gap before the clock.
fn glyph_advance(size: i32) -> i32 {
    let bar = (size as f32 * 0.30).round().max(2.0) as i32;
    let gap = (size as f32 * 0.26).round().max(2.0) as i32;
    bar * 2 + gap + (size as f32 * 0.45).round() as i32
}

/// Draw the shared antialiased muted mark beside the clock.
unsafe fn mute_glyph(dc: HDC, x: i32, cy: i32, size: i32, colour: COLORREF) {
    let rgb = (
        (colour.0 & 0xFF) as u8,
        ((colour.0 >> 8) & 0xFF) as u8,
        ((colour.0 >> 16) & 0xFF) as u8,
    );
    let pixels = crate::ui::icon::mute_mark_pixels(size, rgb);

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
    let Ok(bmp) = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) else {
        return;
    };
    if bits.is_null() {
        let _ = DeleteObject(bmp.into());
        return;
    }

    std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits as *mut u8, pixels.len());
    let src = CreateCompatibleDC(Some(dc));
    let old = SelectObject(src, bmp.into());
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: 255,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    let _ = AlphaBlend(
        dc,
        x,
        cy - size / 2,
        size,
        size,
        src,
        0,
        0,
        size,
        size,
        blend,
    );
    SelectObject(src, old);
    let _ = DeleteDC(src);
    let _ = DeleteObject(bmp.into());
}

/// Width the muted mark needs, including the gap after it.
fn mute_advance(size: i32) -> i32 {
    size + (size as f32 * 0.38).round() as i32
}

/// The small marks that share the clock's line: muted, and paused.
#[derive(Clone, Copy)]
struct Marks {
    muted: bool,
    paused: bool,
}

impl Marks {
    /// Width both marks take before the clock starts.
    fn advance(self, glyph_size: i32) -> i32 {
        let muted = if self.muted {
            mute_advance(glyph_size)
        } else {
            0
        };
        let paused = if self.paused {
            glyph_advance(glyph_size)
        } else {
            0
        };
        muted + paused
    }
}

/// Draw the muted mark, the pause icon (when paused) and the clock as one
/// aligned group.
#[allow(clippy::too_many_arguments)]
unsafe fn draw_clock_group(
    dc: HDC,
    font: HFONT,
    time: &str,
    marks: Marks,
    text_colour: COLORREF,
    accent: COLORREF,
    area: RECT,
    align: DRAW_TEXT_FORMAT,
) {
    let clock_w = text_width(dc, font, time);
    let cy = (area.top + area.bottom) / 2;
    let glyph_size = ((area.bottom - area.top) as f32 * 0.42).round() as i32;
    let glyph_size = glyph_size.clamp(6, 40);
    let advance = marks.advance(glyph_size);
    let total = advance + clock_w;

    let x = if align == DT_CENTER {
        area.left + ((area.right - area.left) - total) / 2
    } else if align == DT_RIGHT {
        area.right - total
    } else {
        area.left
    }
    .max(area.left);

    if marks.muted {
        mute_glyph(dc, x, cy, glyph_size, COLORREF(TEXT));
    }
    if marks.paused {
        let after_mark = if marks.muted {
            mute_advance(glyph_size)
        } else {
            0
        };
        pause_glyph(dc, x + after_mark, cy, glyph_size, accent);
    }

    let old = SelectObject(dc, font.into());
    SetTextColor(dc, text_colour);
    let mut tr = RECT {
        left: x + advance,
        top: area.top,
        right: area.right,
        bottom: area.bottom,
    };
    let tw = wide(time);
    DrawTextW(
        dc,
        &mut tw.as_pcwstr().as_wide().to_vec(),
        &mut tr,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    SelectObject(dc, old);
}

/// The longest variant that fits `max_w`, or `None` if even the shortest does
/// not - in which case the coloured stripe carries the meaning on its own.
unsafe fn pick_label(dc: HDC, font: HFONT, state: &State, max_w: i32) -> Option<String> {
    if max_w < 16 {
        return None;
    }
    label_variants(state)
        .into_iter()
        .find(|c| text_width(dc, font, c) <= max_w)
}

unsafe fn paint(hwnd: HWND, st: &mut State_) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);

    let mut rc = RECT::default();
    let _ = GetClientRect(hwnd, &mut rc);
    let (w, h) = (rc.right, rc.bottom);
    if w <= 0 || h <= 0 {
        let _ = EndPaint(hwnd, &ps);
        return;
    }

    let mem = CreateCompatibleDC(Some(hdc));
    let bmp = CreateCompatibleBitmap(hdc, w, h);
    let old = SelectObject(mem, bmp.into());

    let app = &*st.app;
    let state = app.orch.timer.state();
    let idle = matches!(state, State::Idle);
    let accent = state
        .phase()
        .map(phase_accent)
        .unwrap_or(rgb(0x7A, 0x74, 0x70));

    let bg = CreateSolidBrush(COLORREF(if idle { BG_IDLE } else { BG }));
    FillRect(mem, &rc, bg);
    let _ = DeleteObject(bg.into());

    let stacked = h >= LABEL_MIN_H;
    let pad_x = (w / 16).clamp(6, 14);
    let bar_h = (h / 10).clamp(3, 8);
    let paused = matches!(state, State::Paused { .. });
    let marks = Marks {
        muted: app.orch.dnd_active() && app.config().dnd.mute_window,
        paused,
    };

    let time = match state.remaining() {
        Some(d) => crate::orchestrator::mmss(d),
        None => "--:--".into(),
    };
    let label = match state {
        State::Idle => "Idle".to_string(),
        State::Running { phase, .. } => phase.label().to_string(),
        State::Paused { phase, .. } => format!("{} paused", phase.label()),
    };

    let stripe_w = if stacked { 0 } else { (w / 40).clamp(3, 5) };
    let text_left = pad_x + stripe_w;

    // Rebuild the fonts only when the shape of the window or the length of the
    // string changes; measuring on every tick would be wasteful.
    let key = (w, h, stacked, (time.len() + label.len()) as i32);
    if st.fitted_for != key {
        if !st.font.is_invalid() {
            let _ = DeleteObject(st.font.into());
            let _ = DeleteObject(st.font_small.into());
        }
        if stacked {
            let avail_h = (h - bar_h - (h as f32 * 0.22) as i32).max(8);
            st.font = fit_font(mem, &time, avail_h, (w - pad_x * 2).max(8), 600);
            st.font_small = make_font(((h as f32 * 0.15) as i32).clamp(10, 20), 400);
        } else {
            // Side-by-side: the clock takes the right half so the phase stays
            // readable next to it.
            let avail_h = (h - bar_h - 4).max(8);
            let avail_w = ((w - text_left - pad_x) as f32 * 0.52) as i32;
            st.font = fit_font(mem, &time, avail_h, avail_w.max(8), 600);
            st.font_small = make_font((avail_h as f32 * 0.44) as i32, 500);
        }
        st.fitted_for = key;
    }

    SetBkMode(mem, TRANSPARENT);
    let label_colour = if paused { accent } else { COLORREF(TEXT_DIM) };

    if stacked {
        if let Some(text) = pick_label(mem, st.font_small, state, w - pad_x * 2) {
            let old_font = SelectObject(mem, st.font_small.into());
            SetTextColor(mem, label_colour);
            let mut lr = RECT {
                left: pad_x,
                top: (h as f32 * 0.06) as i32,
                right: w - pad_x,
                bottom: h,
            };
            let lw = wide(&text);
            DrawTextW(
                mem,
                &mut lw.as_pcwstr().as_wide().to_vec(),
                &mut lr,
                DT_CENTER | DT_TOP | DT_SINGLELINE,
            );
            SelectObject(mem, old_font);
        }

        draw_clock_group(
            mem,
            st.font,
            &time,
            marks,
            COLORREF(if idle { TEXT_DIM } else { TEXT }),
            accent,
            RECT {
                left: pad_x,
                top: (h as f32 * 0.24) as i32,
                right: w - pad_x,
                bottom: h - bar_h,
            },
            DT_CENTER,
        );
    } else {
        // A coloured stripe down the left edge: at taskbar height it is the
        // fastest way to see focus vs break vs paused without reading anything.
        let sr = RECT {
            left: 0,
            top: 0,
            right: stripe_w,
            bottom: h - bar_h,
        };
        let sb = CreateSolidBrush(accent);
        FillRect(mem, &sr, sb);
        let _ = DeleteObject(sb.into());

        // Reserve the clock's own width first, then offer the label whatever is
        // left. Letting both spans cover the full width made them overlap.
        let glyph_size = (((h - bar_h) as f32 * 0.42).round() as i32).clamp(6, 40);
        let group_w = text_width(mem, st.font, &time) + marks.advance(glyph_size);
        let clock_left = (w - pad_x - group_w).max(text_left);
        let label_left = text_left + 4;
        let chosen = pick_label(mem, st.font_small, state, clock_left - 8 - label_left);

        draw_clock_group(
            mem,
            st.font,
            &time,
            marks,
            COLORREF(if idle { TEXT_DIM } else { TEXT }),
            accent,
            RECT {
                left: text_left,
                top: 0,
                right: w - pad_x,
                bottom: h - bar_h,
            },
            if chosen.is_some() {
                DT_RIGHT
            } else {
                DT_CENTER
            },
        );

        if let Some(text) = chosen {
            let old_font = SelectObject(mem, st.font_small.into());
            SetTextColor(mem, label_colour);
            let mut lr = RECT {
                left: label_left,
                top: 0,
                right: clock_left - 8,
                bottom: h - bar_h,
            };
            let lw = wide(&text);
            DrawTextW(
                mem,
                &mut lw.as_pcwstr().as_wide().to_vec(),
                &mut lr,
                DT_LEFT | DT_VCENTER | DT_SINGLELINE,
            );
            SelectObject(mem, old_font);
        }
    }

    let bar_top = h - bar_h;
    let track_rect = RECT {
        left: 0,
        top: bar_top,
        right: w,
        bottom: h,
    };
    let tb = CreateSolidBrush(COLORREF(TRACK));
    FillRect(mem, &track_rect, tb);
    let _ = DeleteObject(tb.into());

    if !idle {
        let filled = (w as f32 * state.progress()).round() as i32;
        if filled > 0 {
            let fr = RECT {
                left: 0,
                top: bar_top,
                right: filled.min(w),
                bottom: h,
            };
            let fb = CreateSolidBrush(accent);
            FillRect(mem, &fr, fb);
            let _ = DeleteObject(fb.into());
        }
    }

    let border = CreateSolidBrush(COLORREF(BORDER));
    FrameRect(mem, &rc, border);
    let _ = DeleteObject(border.into());

    let _ = BitBlt(hdc, 0, 0, w, h, Some(mem), 0, 0, SRCCOPY);
    SelectObject(mem, old);
    let _ = DeleteObject(bmp.into());
    let _ = DeleteDC(mem);
    let _ = EndPaint(hwnd, &ps);
}

/// Window procedure for the compact window.
///
/// The window has no caption, which means Windows provides nothing to drag it
/// by and no usable sizing border, so the frame is handled here:
///
/// * `WM_NCCALCSIZE` claims the whole window as client area. `WS_THICKFRAME` is
///   still set, because Windows only allows resizing for windows that have it,
///   but its border would otherwise be drawn around our own painting.
/// * `WM_NCHITTEST` reports the outer few pixels as resize handles and
///   everything else as client. Without it neither resizing nor moving works.
/// * `WM_LBUTTONDOWN` hands the press to the system move loop with the real
///   cursor position, then decides after the fact whether it was a drag or a
///   click. Passing zero coordinates leaves Windows thinking the press happened
///   at the top-left corner, and the drag silently never starts.
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State_;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let st = &mut *ptr;

    match msg {
        WM_ERASEBKGND => LRESULT(1),

        WM_NCCALCSIZE if wparam.0 != 0 => LRESULT(0),

        WM_NCHITTEST => {
            let sx = (lparam.0 & 0xFFFF) as i16 as i32;
            let sy = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            let mut r = RECT::default();
            let _ = GetWindowRect(hwnd, &mut r);

            let left = sx < r.left + RESIZE_MARGIN;
            let right = sx >= r.right - RESIZE_MARGIN;
            let top = sy < r.top + RESIZE_MARGIN;
            let bottom = sy >= r.bottom - RESIZE_MARGIN;

            let ht = match (left, right, top, bottom) {
                (true, _, true, _) => HTTOPLEFT,
                (_, true, true, _) => HTTOPRIGHT,
                (true, _, _, true) => HTBOTTOMLEFT,
                (_, true, _, true) => HTBOTTOMRIGHT,
                (true, ..) => HTLEFT,
                (_, true, ..) => HTRIGHT,
                (_, _, true, _) => HTTOP,
                (_, _, _, true) => HTBOTTOM,
                _ => HTCLIENT,
            };
            LRESULT(ht as isize)
        }

        WM_PAINT => {
            paint(hwnd, st);
            LRESULT(0)
        }

        WM_GETMINMAXINFO => {
            let mmi = lparam.0 as *mut MINMAXINFO;
            if !mmi.is_null() {
                (*mmi).ptMinTrackSize = windows::Win32::Foundation::POINT { x: MIN_W, y: MIN_H };
            }
            LRESULT(0)
        }

        // With no caption there is nothing to drag by, so the whole body drags
        // the window. Handing the press straight to WM_NCLBUTTONDOWN starts
        // Windows' own move loop, which blocks until the button is released;
        // if the pointer never really moved, treat it as a click instead.
        WM_LBUTTONDOWN => {
            let mut before = windows::Win32::Foundation::POINT::default();
            let _ = GetCursorPos(&mut before);
            let packed = LPARAM(((before.y as isize) << 16) | (before.x as isize & 0xFFFF));
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture();
            SendMessageW(
                hwnd,
                WM_NCLBUTTONDOWN,
                Some(WPARAM(HTCAPTION as usize)),
                Some(packed),
            );

            let mut after = windows::Win32::Foundation::POINT::default();
            let _ = GetCursorPos(&mut after);
            if (after.x - before.x).abs() <= DRAG_SLOP && (after.y - before.y).abs() <= DRAG_SLOP {
                (*st.app).mini_clicked();
            } else {
                (*st.app).mini_geometry_changed();
            }
            LRESULT(0)
        }

        WM_RBUTTONUP | WM_CONTEXTMENU => {
            (*st.app).show_menu();
            LRESULT(0)
        }

        WM_EXITSIZEMOVE => {
            (*st.app).mini_geometry_changed();
            LRESULT(0)
        }

        WM_SIZE => {
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }

        WM_CLOSE => {
            (*st.app).mini_closed_by_user();
            LRESULT(0)
        }

        WM_DESTROY => {
            let st = Box::from_raw(ptr);
            if !st.font.is_invalid() {
                let _ = DeleteObject(st.font.into());
                let _ = DeleteObject(st.font_small.into());
            }
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            LRESULT(0)
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
