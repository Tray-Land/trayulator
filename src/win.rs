//! Small Win32 helpers shared by the app's windows.

use std::rc::Rc;

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{ClientToScreen, MONITOR_DEFAULTTONEAREST, MonitorFromPoint};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, GetDpiForWindow, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CreateWindowExW, DefWindowProcW, GWLP_USERDATA, GetWindowLongPtrW, HCURSOR, RegisterClassExW,
    SetWindowLongPtrW, WINDOW_EX_STYLE, WINDOW_STYLE, WM_NCCREATE, WM_NCDESTROY, WNDCLASS_STYLES, WNDCLASSEXW,
};
use windows::core::{PCWSTR, Result};

/// Logical pixels per inch: the DPI at 100% scaling.
pub const BASE_DPI: u32 = 96;

/// Handles messages for one window. `handle` returns `None` to fall back to `DefWindowProcW`.
pub trait WindowProc: 'static {
    fn handle(self: &Rc<Self>, hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT>;
}

pub fn instance() -> HINSTANCE {
    unsafe { GetModuleHandleW(None).map(Into::into).unwrap_or_default() }
}

/// Registers a window class whose messages go to `T::handle`.
pub fn register_class<T: WindowProc>(name: PCWSTR, style: WNDCLASS_STYLES, cursor: HCURSOR) -> Result<()> {
    let class = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        style,
        lpfnWndProc: Some(window_proc::<T>),
        hInstance: instance(),
        hCursor: cursor,
        lpszClassName: name,
        ..Default::default()
    };

    if unsafe { RegisterClassExW(&class) } == 0 {
        return Err(windows::core::Error::from_thread());
    }
    Ok(())
}

/// Creates a window whose procedure is `this`. The window holds a strong reference until
/// `WM_NCDESTROY`, so the handler stays alive for every message it receives.
#[allow(clippy::too_many_arguments)]
pub fn create_window<T: WindowProc>(
    this: &Rc<T>,
    class: PCWSTR,
    title: PCWSTR,
    style: WINDOW_STYLE,
    ex_style: WINDOW_EX_STYLE,
    rect: RECT,
    parent: Option<HWND>,
) -> Result<HWND> {
    let raw = Rc::into_raw(Rc::clone(this));
    unsafe {
        CreateWindowExW(
            ex_style,
            class,
            title,
            style,
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            parent,
            None,
            Some(instance()),
            Some(raw.cast()),
        )
    }
}

unsafe extern "system" fn window_proc<T: WindowProc>(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let create = &*(lparam.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }

        let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const T;
        if raw.is_null() {
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }

        // A temporary strong reference: the handler may drop every other one (closing the window).
        Rc::increment_strong_count(raw);
        let this = Rc::from_raw(raw);
        let result = this.handle(hwnd, msg, wparam, lparam);

        if msg == WM_NCDESTROY {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            drop(Rc::from_raw(raw));
        }

        result.unwrap_or_else(|| DefWindowProcW(hwnd, msg, wparam, lparam))
    }
}

pub fn loword(value: usize) -> u16 {
    (value & 0xFFFF) as u16
}

/// The signed x coordinate packed in the low word (GET_X_LPARAM).
pub fn x_param(value: isize) -> i32 {
    (value & 0xFFFF) as i16 as i32
}

/// The signed y coordinate packed in the high word (GET_Y_LPARAM).
pub fn y_param(value: isize) -> i32 {
    ((value >> 16) & 0xFFFF) as i16 as i32
}

pub fn window_dpi(hwnd: HWND) -> u32 {
    match unsafe { GetDpiForWindow(hwnd) } {
        0 => BASE_DPI,
        dpi => dpi,
    }
}

pub fn dpi_for_point(point: POINT) -> u32 {
    let monitor = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) };
    let (mut x, mut y) = (0, 0);
    match unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y) } {
        Ok(()) if x != 0 => x,
        _ => BASE_DPI,
    }
}

/// Logical (96 DPI) pixels to physical pixels, rounding up so content never clips.
pub fn to_physical(logical: f32, dpi: u32) -> i32 {
    (logical * dpi as f32 / BASE_DPI as f32).ceil() as i32
}

pub fn client_to_screen(hwnd: HWND, x: i32, y: i32) -> POINT {
    let mut point = POINT { x, y };
    unsafe {
        let _ = ClientToScreen(hwnd, &mut point);
    }
    point
}
