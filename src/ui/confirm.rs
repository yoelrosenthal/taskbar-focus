//! Modal Strict-focus confirmation, drawn like the session overlay.

use std::sync::Once;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{VK_ESCAPE, VK_RETURN};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::ui::wide;

const CLASS: &str = "TaskbarFocusConfirm";
const W: i32 = 440;
const H: i32 = 228;

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF((b as u32) << 16 | (g as u32) << 8 | r as u32)
}

fn darken(c: COLORREF) -> COLORREF {
    let r = ((c.0 & 0xFF) as u8 as u16 * 3 / 4) as u8;
    let g = (((c.0 >> 8) & 0xFF) as u8 as u16 * 3 / 4) as u8;
    let b = (((c.0 >> 16) & 0xFF) as u8 as u16 * 3 / 4) as u8;
    rgb(r, g, b)
}

fn accent() -> COLORREF {
    rgb(0xE8, 0x56, 0x3F)
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

fn keep_rect() -> RECT {
    RECT {
        left: W - 168,
        top: H - 58,
        right: W - 24,
        bottom: H - 22,
    }
}

fn action_rect() -> RECT {
    RECT {
        left: 24,
        top: H - 58,
        right: 24 + 128,
        bottom: H - 22,
    }
}

fn hit(rc: RECT, x: i32, y: i32) -> bool {
    x >= rc.left && x < rc.right && y >= rc.top && y < rc.bottom
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Which {
    Keep,
    Action,
}

struct State {
    title: String,
    body: String,
    action: String,
    font_eyebrow: HFONT,
    font_title: HFONT,
    font_body: HFONT,
    pressed: Option<Which>,
    accepted: *mut bool,
    finished: *mut bool,
}

fn register() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let Ok(instance) = GetModuleHandleW(None) else {
            return;
        };
        let class = wide(CLASS);
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: class.as_pcwstr(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hIcon: crate::ui::app::shared_icon(),
            hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
            ..Default::default()
        };
        RegisterClassW(&wc);
    });
}

/// Ask whether to go through with Stop/Skip. `true` means the user chose the
/// destructive action. Escape and Enter keep the session running.
pub fn ask(owner: HWND, title: &str, body: &str, action: &str) -> bool {
    register();
    unsafe {
        let Ok(instance) = GetModuleHandleW(None) else {
            return false;
        };
        let work = crate::ui::alert::monitor_work(owner);
        let x = work.left + ((work.right - work.left) - W) / 2;
        let y = work.top + ((work.bottom - work.top) - H) / 2;

        let mut accepted = false;
        let mut finished = false;

        let hwnd = match CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            wide(CLASS).as_pcwstr(),
            wide("Strict focus").as_pcwstr(),
            WS_POPUP | WS_VISIBLE,
            x,
            y,
            W,
            H,
            Some(owner),
            None,
            Some(instance.into()),
            None,
        ) {
            Ok(h) => h,
            Err(_) => return false,
        };

        let pref = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &pref as *const _ as *const _,
            std::mem::size_of_val(&pref) as u32,
        );

        let st = Box::new(State {
            title: title.to_string(),
            body: body.to_string(),
            action: action.to_string(),
            font_eyebrow: make_font(13, FW_SEMIBOLD.0 as i32),
            font_title: make_font(26, FW_SEMIBOLD.0 as i32),
            font_body: make_font(16, FW_NORMAL.0 as i32),
            pressed: None,
            accepted: &mut accepted,
            finished: &mut finished,
        });
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(st) as isize);
        let _ = SetForegroundWindow(hwnd);

        let mut msg = MSG::default();
        while !finished && GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
            if !IsWindow(Some(hwnd)).as_bool() {
                break;
            }
        }
        accepted
    }
}

unsafe fn paint_pill(
    hdc: HDC,
    rc: RECT,
    fill: COLORREF,
    text: &str,
    text_color: COLORREF,
    font: HFONT,
) {
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
    SetTextColor(hdc, text_color);
    let old = SelectObject(hdc, font.into());
    let mut label: Vec<u16> = text.encode_utf16().collect();
    let mut text_rc = rc;
    DrawTextW(
        hdc,
        &mut label,
        &mut text_rc,
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );
    SelectObject(hdc, old);
}

unsafe fn finish(hwnd: HWND, accepted: bool) {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
    if !ptr.is_null() {
        *(*ptr).accepted = accepted;
        *(*ptr).finished = true;
    }
    let _ = DestroyWindow(hwnd);
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
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
            let stripe_brush = CreateSolidBrush(accent());
            FillRect(hdc, &stripe, stripe_brush);
            let badge = RECT {
                left: 24,
                top: 26,
                right: 30,
                bottom: 50,
            };
            FillRect(hdc, &badge, stripe_brush);
            let _ = DeleteObject(stripe_brush.into());

            SetBkMode(hdc, TRANSPARENT);
            let old = SelectObject(hdc, st.font_eyebrow.into());
            SetTextColor(hdc, rgb(0xD2, 0xCB, 0xC4));
            let mut eyebrow: Vec<u16> = "STRICT FOCUS".encode_utf16().collect();
            let mut eyebrow_rc = RECT {
                left: 44,
                top: 26,
                right: rc.right - 24,
                bottom: 50,
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
                left: 24,
                top: 60,
                right: rc.right - 24,
                bottom: 104,
            };
            DrawTextW(hdc, &mut title, &mut title_rc, DT_LEFT | DT_WORDBREAK);

            SelectObject(hdc, st.font_body.into());
            SetTextColor(hdc, rgb(0xA8, 0xA2, 0x9C));
            let mut body: Vec<u16> = st.body.encode_utf16().collect();
            let mut body_rc = RECT {
                left: 24,
                top: 108,
                right: rc.right - 24,
                bottom: rc.bottom - 74,
            };
            DrawTextW(hdc, &mut body, &mut body_rc, DT_LEFT | DT_WORDBREAK);

            let keep_fill = if st.pressed == Some(Which::Keep) {
                rgb(0xE0, 0xDC, 0xD6)
            } else {
                rgb(0xF7, 0xF5, 0xF2)
            };
            paint_pill(
                hdc,
                keep_rect(),
                keep_fill,
                "Keep going",
                rgb(0x1C, 0x1A, 0x18),
                st.font_body,
            );
            let action_fill = if st.pressed == Some(Which::Action) {
                darken(accent())
            } else {
                accent()
            };
            paint_pill(
                hdc,
                action_rect(),
                action_fill,
                &st.action,
                rgb(0xFF, 0xFF, 0xFF),
                st.font_body,
            );
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
            let which = if hit(keep_rect(), x, y) {
                Some(Which::Keep)
            } else if hit(action_rect(), x, y) {
                Some(Which::Action)
            } else {
                None
            };
            (*ptr).pressed = which;
            if which.is_some() {
                let _ = InvalidateRect(Some(hwnd), None, true);
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
            (*ptr).pressed = None;
            let _ = InvalidateRect(Some(hwnd), None, true);
            match was {
                Some(Which::Keep) if hit(keep_rect(), x, y) => finish(hwnd, false),
                Some(Which::Action) if hit(action_rect(), x, y) => finish(hwnd, true),
                _ => {}
            }
            LRESULT(0)
        }
        WM_KEYDOWN if wparam.0 == VK_ESCAPE.0 as usize || wparam.0 == VK_RETURN.0 as usize => {
            finish(hwnd, false);
            LRESULT(0)
        }
        WM_DESTROY => {
            if !ptr.is_null() {
                let st = Box::from_raw(ptr);
                *st.finished = true;
                let _ = DeleteObject(st.font_eyebrow.into());
                let _ = DeleteObject(st.font_title.into());
                let _ = DeleteObject(st.font_body.into());
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
