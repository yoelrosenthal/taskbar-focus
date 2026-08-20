//! Centre overlay and per-monitor screen flash for completed focus / break
//! intervals.
//!
//! These are ordinary top-most windows owned by the app, so they remain visible
//! even while Windows Do Not Disturb is suppressing Action Centre toasts.

use std::sync::Once;

use windows::core::{BOOL, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{VK_ESCAPE, VK_RETURN};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::config::Alerts;
use crate::orchestrator::Event;
use crate::ui::wide;

const OVERLAY_CLASS: &str = "TaskbarFocusAlertOverlay";
const DIM_CLASS: &str = "TaskbarFocusAlertDim";
const FLASH_CLASS: &str = "TaskbarFocusAlertFlash";

/// Posted to the owner when an overlay or flash destroys itself.
/// `wparam` is the HWND, `lparam` is the generation from [`show_overlay`].
pub const WM_ALERT_CLOSED: u32 = WM_APP + 3;

const OVERLAY_W: i32 = 460;
const OVERLAY_H: i32 = 248;
const TIMER_DISMISS: usize = 1;
const TIMER_FLASH: usize = 1;

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF((b as u32) << 16 | (g as u32) << 8 | r as u32)
}

fn darken(c: COLORREF) -> COLORREF {
    let r = ((c.0 & 0xFF) as u8 as u16 * 3 / 4) as u8;
    let g = (((c.0 >> 8) & 0xFF) as u8 as u16 * 3 / 4) as u8;
    let b = (((c.0 >> 16) & 0xFF) as u8 as u16 * 3 / 4) as u8;
    rgb(r, g, b)
}

fn register_classes() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let Ok(instance) = GetModuleHandleW(None) else {
            return;
        };
        let cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        let icon = crate::ui::app::shared_icon();
        let bg = HBRUSH(GetStockObject(BLACK_BRUSH).0);
        let one = |name: &str, proc, icon: HICON| {
            let class = wide(name);
            let wc = WNDCLASSW {
                lpfnWndProc: Some(proc),
                hInstance: instance.into(),
                lpszClassName: class.as_pcwstr(),
                hCursor: cursor,
                hIcon: icon,
                hbrBackground: bg,
                ..Default::default()
            };
            RegisterClassW(&wc);
        };
        one(OVERLAY_CLASS, overlay_wndproc, icon);
        one(DIM_CLASS, dim_wndproc, HICON::default());
        one(FLASH_CLASS, flash_wndproc, HICON::default());
    });
}

fn accent_for(event: Event) -> COLORREF {
    match event {
        Event::FocusStart | Event::BreakEnd => rgb(0xE8, 0x56, 0x3F),
        Event::BreakStart | Event::FocusEnd => rgb(0x35, 0xC4, 0x6A),
    }
}

fn eyebrow_for(event: Event) -> &'static str {
    match event {
        Event::FocusStart | Event::BreakEnd => "FOCUS",
        Event::BreakStart | Event::FocusEnd => "BREAK",
    }
}

fn button_rect() -> RECT {
    RECT {
        left: OVERLAY_W - 148,
        top: OVERLAY_H - 58,
        right: OVERLAY_W - 24,
        bottom: OVERLAY_H - 22,
    }
}

/// Monitor work area nearest to `anchor`, or the primary work area.
pub fn monitor_work(anchor: HWND) -> RECT {
    unsafe {
        let mon = MonitorFromWindow(anchor, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(mon, &mut mi).as_bool() {
            return mi.rcWork;
        }
        let mut work = RECT::default();
        let _ = SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut work as *mut _ as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
        work
    }
}

/// Full bounds of every connected display, in virtual-screen coordinates.
fn monitor_bounds(fallback: HWND) -> Vec<RECT> {
    let mut rects = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(collect_monitor),
            LPARAM(&mut rects as *mut Vec<RECT> as isize),
        );
    }
    if rects.is_empty() {
        rects.push(monitor_work(fallback));
    }
    rects
}

unsafe extern "system" fn collect_monitor(
    hmon: HMONITOR,
    _hdc: HDC,
    lprc: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    let rects = &mut *(lparam.0 as *mut Vec<RECT>);
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if GetMonitorInfoW(hmon, &mut mi).as_bool() {
        rects.push(mi.rcMonitor);
    } else if !lprc.is_null() {
        rects.push(*lprc);
    }
    true.into()
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

struct OverlayState {
    title: String,
    body: String,
    eyebrow: String,
    button: String,
    accent: COLORREF,
    font_eyebrow: HFONT,
    font_title: HFONT,
    font_body: HFONT,
    dim: Option<HWND>,
    pressed: bool,
    require_dismiss: bool,
    generation: u32,
}

/// Show the centred alert card (with a dimmed backdrop). Returns the card HWND.
pub fn show_overlay(
    owner: HWND,
    anchor: HWND,
    title: &str,
    body: &str,
    event: Event,
    settings: &Alerts,
    generation: u32,
) -> Option<HWND> {
    unsafe {
        register_classes();
        let instance = GetModuleHandleW(None).ok()?;
        let work = monitor_work(anchor);
        let dim = show_dim(owner, &work, instance.into());
        let class = wide(OVERLAY_CLASS);

        let x = work.left + ((work.right - work.left) - OVERLAY_W) / 2;
        let y = work.top + ((work.bottom - work.top) - OVERLAY_H) / 2;

        let hwnd = match CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class.as_pcwstr(),
            wide(title).as_pcwstr(),
            WS_POPUP | WS_VISIBLE,
            x,
            y,
            OVERLAY_W,
            OVERLAY_H,
            Some(owner),
            None,
            Some(instance.into()),
            None,
        ) {
            Ok(h) => h,
            Err(_) => {
                if let Some(d) = dim {
                    let _ = DestroyWindow(d);
                }
                return None;
            }
        };

        let pref = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &pref as *const _ as *const _,
            std::mem::size_of_val(&pref) as u32,
        );

        let font_eyebrow = make_font(13, FW_SEMIBOLD.0 as i32);
        let font_title = make_font(30, FW_SEMIBOLD.0 as i32);
        let font_body = make_font(17, FW_NORMAL.0 as i32);

        let button = if settings.require_dismiss {
            "Continue"
        } else {
            "Dismiss"
        };

        let st = Box::new(OverlayState {
            title: title.to_string(),
            body: body.to_string(),
            eyebrow: eyebrow_for(event).to_string(),
            button: button.to_string(),
            accent: accent_for(event),
            font_eyebrow,
            font_title,
            font_body,
            dim,
            pressed: false,
            require_dismiss: settings.require_dismiss,
            generation,
        });
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(st) as isize);
        if let Some(d) = dim {
            SetWindowLongPtrW(d, GWLP_USERDATA, hwnd.0 as isize);
        }

        if !settings.require_dismiss {
            let secs = settings.auto_dismiss_secs.max(1);
            SetTimer(Some(hwnd), TIMER_DISMISS, secs.saturating_mul(1000), None);
        }

        Some(hwnd)
    }
}

unsafe fn show_dim(
    owner: HWND,
    work: &RECT,
    instance: windows::Win32::Foundation::HINSTANCE,
) -> Option<HWND> {
    let class = wide(DIM_CLASS);
    let hwnd = CreateWindowExW(
        WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_NOACTIVATE,
        class.as_pcwstr(),
        PCWSTR::null(),
        WS_POPUP | WS_VISIBLE,
        work.left,
        work.top,
        work.right - work.left,
        work.bottom - work.top,
        Some(owner),
        None,
        Some(instance),
        None,
    )
    .ok()?;
    let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 120, LWA_ALPHA);
    Some(hwnd)
}

pub fn close(hwnd: HWND) {
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
}

unsafe fn paint_button(hdc: HDC, st: &OverlayState) {
    let rc = button_rect();
    let fill = if st.pressed {
        darken(st.accent)
    } else {
        st.accent
    };
    let brush = CreateSolidBrush(fill);
    let pen = CreatePen(PS_SOLID, 1, fill);
    let old_brush = SelectObject(hdc, brush.into());
    let old_pen = SelectObject(hdc, pen.into());
    let _ = RoundRect(hdc, rc.left, rc.top, rc.right, rc.bottom, 10, 10);
    SelectObject(hdc, old_brush);
    SelectObject(hdc, old_pen);
    let _ = DeleteObject(brush.into());
    let _ = DeleteObject(pen.into());

    SetBkMode(hdc, TRANSPARENT);
    SetTextColor(hdc, rgb(0xFF, 0xFF, 0xFF));
    let old = SelectObject(hdc, st.font_body.into());
    let mut label: Vec<u16> = st.button.encode_utf16().collect();
    let mut text_rc = rc;
    DrawTextW(
        hdc,
        &mut label,
        &mut text_rc,
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );
    SelectObject(hdc, old);
}

unsafe extern "system" fn dim_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);
            let brush = CreateSolidBrush(rgb(0x00, 0x00, 0x00));
            FillRect(hdc, &rc, brush);
            let _ = DeleteObject(brush.into());
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let card = HWND(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut _);
            if !card.is_invalid() {
                let ptr = GetWindowLongPtrW(card, GWLP_USERDATA) as *mut OverlayState;
                if ptr.is_null() || !(*ptr).require_dismiss {
                    let _ = DestroyWindow(card);
                }
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe extern "system" fn overlay_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut OverlayState;
    match msg {
        WM_PAINT => {
            if ptr.is_null() {
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            }
            let st = &*ptr;
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);

            let bg = CreateSolidBrush(rgb(0x22, 0x20, 0x1E));
            FillRect(hdc, &rc, bg);
            let _ = DeleteObject(bg.into());

            let stripe = RECT {
                left: 0,
                top: 0,
                right: rc.right,
                bottom: 5,
            };
            let accent_brush = CreateSolidBrush(st.accent);
            FillRect(hdc, &stripe, accent_brush);

            let badge = RECT {
                left: 28,
                top: 28,
                right: 34,
                bottom: 52,
            };
            FillRect(hdc, &badge, accent_brush);
            let _ = DeleteObject(accent_brush.into());

            SetBkMode(hdc, TRANSPARENT);
            let old = SelectObject(hdc, st.font_eyebrow.into());
            SetTextColor(hdc, rgb(0xD2, 0xCB, 0xC4));
            let mut eyebrow: Vec<u16> = st.eyebrow.encode_utf16().collect();
            let mut eyebrow_rc = RECT {
                left: 48,
                top: 28,
                right: rc.right - 28,
                bottom: 52,
            };
            DrawTextW(
                hdc,
                &mut eyebrow,
                &mut eyebrow_rc,
                DT_LEFT | DT_SINGLELINE | DT_VCENTER,
            );

            SelectObject(hdc, st.font_title.into());
            SetTextColor(hdc, rgb(0xF7, 0xF5, 0xF2));
            let mut title: Vec<u16> = st.title.encode_utf16().collect();
            let mut title_rc = RECT {
                left: 28,
                top: 64,
                right: rc.right - 28,
                bottom: 108,
            };
            DrawTextW(
                hdc,
                &mut title,
                &mut title_rc,
                DT_LEFT | DT_SINGLELINE | DT_END_ELLIPSIS,
            );

            SelectObject(hdc, st.font_body.into());
            SetTextColor(hdc, rgb(0xA8, 0xA2, 0x9C));
            let mut body: Vec<u16> = st.body.encode_utf16().collect();
            let mut body_rc = RECT {
                left: 28,
                top: 118,
                right: rc.right - 28,
                bottom: rc.bottom - 78,
            };
            DrawTextW(hdc, &mut body, &mut body_rc, DT_LEFT | DT_WORDBREAK);

            let rule = RECT {
                left: 28,
                top: rc.bottom - 74,
                right: rc.right - 28,
                bottom: rc.bottom - 73,
            };
            let rule_brush = CreateSolidBrush(rgb(0x3A, 0x36, 0x32));
            FillRect(hdc, &rule, rule_brush);
            let _ = DeleteObject(rule_brush.into());

            paint_button(hdc, st);
            SelectObject(hdc, old);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            if ptr.is_null() {
                return LRESULT(0);
            }
            let x = (lparam.0 as i32) & 0xFFFF;
            let y = ((lparam.0 as i32) >> 16) & 0xFFFF;
            let rc = button_rect();
            if x >= rc.left && x < rc.right && y >= rc.top && y < rc.bottom {
                (*ptr).pressed = true;
                let _ = InvalidateRect(Some(hwnd), Some(&rc), true);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            if ptr.is_null() {
                return LRESULT(0);
            }
            let x = (lparam.0 as i32) & 0xFFFF;
            let y = ((lparam.0 as i32) >> 16) & 0xFFFF;
            let was = (*ptr).pressed;
            (*ptr).pressed = false;
            let rc = button_rect();
            let _ = InvalidateRect(Some(hwnd), Some(&rc), true);
            if was && x >= rc.left && x < rc.right && y >= rc.top && y < rc.bottom {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_DISMISS => {
            let _ = KillTimer(Some(hwnd), TIMER_DISMISS);
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_KEYDOWN if wparam.0 == VK_ESCAPE.0 as usize || wparam.0 == VK_RETURN.0 as usize => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            let _ = KillTimer(Some(hwnd), TIMER_DISMISS);
            if !ptr.is_null() {
                let st = Box::from_raw(ptr);
                notify_owner_closed(hwnd, st.generation);
                if let Some(dim) = st.dim {
                    let _ = DestroyWindow(dim);
                }
                let _ = DeleteObject(st.font_eyebrow.into());
                let _ = DeleteObject(st.font_title.into());
                let _ = DeleteObject(st.font_body.into());
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn notify_owner_closed(hwnd: HWND, generation: u32) {
    unsafe {
        let owner = GetWindow(hwnd, GW_OWNER).unwrap_or_default();
        if !owner.is_invalid() {
            let _ = PostMessageW(
                Some(owner),
                WM_ALERT_CLOSED,
                WPARAM(hwnd.0 as usize),
                LPARAM(generation as isize),
            );
        }
    }
}

struct FlashState {
    accent: COLORREF,
    pulses_left: u32,
    on: bool,
    generation: u32,
}

/// Pulse a translucent colour flash across every connected monitor, then
/// destroy the windows.
pub fn show_flash(
    owner: HWND,
    anchor: HWND,
    event: Event,
    generation: u32,
    length: crate::config::LengthPreset,
) -> Vec<HWND> {
    unsafe {
        register_classes();
        let Ok(instance) = GetModuleHandleW(None) else {
            return Vec::new();
        };
        let class = wide(FLASH_CLASS);
        let instance = instance.into();
        let mut hwnds = Vec::new();
        for bounds in monitor_bounds(anchor) {
            if let Some(hwnd) = create_flash_window(
                owner,
                instance,
                class.as_pcwstr(),
                bounds,
                event,
                generation,
                length,
            ) {
                hwnds.push(hwnd);
            }
        }
        hwnds
    }
}

unsafe fn create_flash_window(
    owner: HWND,
    instance: windows::Win32::Foundation::HINSTANCE,
    class: PCWSTR,
    bounds: RECT,
    event: Event,
    generation: u32,
    length: crate::config::LengthPreset,
) -> Option<HWND> {
    let hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE,
        class,
        PCWSTR::null(),
        WS_POPUP | WS_VISIBLE,
        bounds.left,
        bounds.top,
        bounds.right - bounds.left,
        bounds.bottom - bounds.top,
        Some(owner),
        None,
        Some(instance),
        None,
    )
    .ok()?;

    let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 100, LWA_ALPHA);

    let st = Box::new(FlashState {
        accent: accent_for(event),
        pulses_left: length.flash_pulses(),
        on: true,
        generation,
    });
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(st) as isize);
    SetTimer(Some(hwnd), TIMER_FLASH, length.flash_ms(), None);
    Some(hwnd)
}

unsafe extern "system" fn flash_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut FlashState;
    match msg {
        WM_PAINT => {
            if ptr.is_null() {
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            }
            let st = &*ptr;
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);
            let brush = CreateSolidBrush(st.accent);
            FillRect(hdc, &rc, brush);
            let _ = DeleteObject(brush.into());
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_FLASH => {
            if ptr.is_null() {
                let _ = DestroyWindow(hwnd);
                return LRESULT(0);
            }
            let st = &mut *ptr;
            if st.on {
                st.on = false;
                let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 0, LWA_ALPHA);
                st.pulses_left = st.pulses_left.saturating_sub(1);
                if st.pulses_left == 0 {
                    let _ = KillTimer(Some(hwnd), TIMER_FLASH);
                    let _ = DestroyWindow(hwnd);
                }
            } else {
                st.on = true;
                let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 100, LWA_ALPHA);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            let _ = KillTimer(Some(hwnd), TIMER_FLASH);
            if !ptr.is_null() {
                let st = Box::from_raw(ptr);
                notify_owner_closed(hwnd, st.generation);
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
