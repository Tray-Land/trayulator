//! The flyout's tooltips: a small popup that never takes focus or clicks, drawn like WinUI's.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUNDSMALL, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, GetMonitorInfoW, InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, PAINTSTRUCT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, HCURSOR, HTTRANSPARENT, HWND_TOPMOST, MA_NOACTIVATE, SW_HIDE, SWP_NOACTIVATE, SWP_SHOWWINDOW, SetWindowPos, ShowWindow,
    WM_ERASEBKGND, WM_MOUSEACTIVATE, WM_NCCREATE, WM_NCHITTEST, WM_PAINT, WNDCLASS_STYLES, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{PCWSTR, Result, w};

use super::Rect;
use super::graphics::{Graphics, TextStyle};
use super::palette::Palette;
use crate::win::{self, WindowProc};

const CLASS_NAME: PCWSTR = w!("Trayulator.Tooltip");
const PADDING: (f32, f32) = (9.0, 6.0);
const MAX_WIDTH: f32 = 320.0;
/// Gap between the tooltip and what it describes.
const OFFSET: f32 = 6.0;

thread_local! {
    static REGISTERED: Cell<bool> = const { Cell::new(false) };
}

pub struct Tooltip {
    hwnd: Cell<HWND>,
    graphics: RefCell<Option<Graphics>>,
    text: RefCell<String>,
    palette: RefCell<Option<Palette>>,
}

impl Tooltip {
    /// Creates the (hidden) tooltip window. It's owned by `owner`, so it goes when the owner does.
    pub fn new(owner: HWND) -> Result<Rc<Self>> {
        if !REGISTERED.get() {
            win::register_class::<Self>(CLASS_NAME, WNDCLASS_STYLES(0), HCURSOR::default())?;
            REGISTERED.set(true);
        }

        let this = Rc::new(Self {
            hwnd: Cell::default(),
            graphics: RefCell::new(None),
            text: RefCell::default(),
            palette: RefCell::new(None),
        });
        let ex_style = WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT;
        let hwnd = win::create_window(&this, CLASS_NAME, w!(""), WS_POPUP, ex_style, RECT::default(), Some(owner))?;
        unsafe {
            let corner = DWMWCP_ROUNDSMALL;
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, (&raw const corner).cast(), size_of_val(&corner) as u32);
        }
        Ok(this)
    }

    /// Shows `text` centered above `anchor` (a screen rectangle), or below it near the top of
    /// the screen.
    pub fn show(&self, text: &str, anchor: RECT, palette: &Palette) {
        *self.text.borrow_mut() = text.to_owned();
        *self.palette.borrow_mut() = Some(palette.clone());

        let center = POINT { x: (anchor.left + anchor.right) / 2, y: (anchor.top + anchor.bottom) / 2 };
        let dpi = win::dpi_for_point(center);
        let Some((width, height)) = self.measure(text) else {
            return;
        };
        let (w, h) = (win::to_physical(width, dpi), win::to_physical(height, dpi));
        let offset = win::to_physical(OFFSET, dpi);

        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        unsafe {
            let _ = GetMonitorInfoW(MonitorFromPoint(center, MONITOR_DEFAULTTONEAREST), &mut info);
        }
        let work = info.rcWork;
        let x = (center.x - w / 2).clamp(work.left, (work.right - w).max(work.left));
        let above = anchor.top - offset - h;
        let y = if above >= work.top { above } else { anchor.bottom + offset };

        let hwnd = self.hwnd.get();
        unsafe {
            let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }

    pub fn hide(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd.get(), SW_HIDE);
        }
    }

    /// The tooltip's size in DIPs for `text`, wrapping long text.
    fn measure(&self, text: &str) -> Option<(f32, f32)> {
        let mut graphics = self.graphics.borrow_mut();
        if graphics.is_none() {
            *graphics = Graphics::new().ok();
        }
        let (width, height) = graphics.as_ref()?.text.measure(text, TextStyle::CAPTION, MAX_WIDTH - 2.0 * PADDING.0, true);
        Some((width.ceil() + 2.0 * PADDING.0, height.ceil() + 2.0 * PADDING.1))
    }

    fn paint(&self, hwnd: HWND) {
        let mut paint = PAINTSTRUCT::default();
        unsafe { BeginPaint(hwnd, &mut paint) };

        let (mut graphics, text, palette) = (self.graphics.borrow_mut(), self.text.borrow(), self.palette.borrow());
        if let (Some(graphics), Some(palette)) = (graphics.as_mut(), palette.as_ref()) {
            let bounds = client_bounds(hwnd);
            graphics.draw(hwnd, win::window_dpi(hwnd), palette, palette.solid_background, |canvas| {
                canvas.stroke(bounds, 4.0, 1.0, palette.surface_stroke_flyout);
                let content = bounds.inset(PADDING.0, PADDING.1, PADDING.0, PADDING.1);
                canvas.wrapped_text(&text, TextStyle::CAPTION, content, palette.text_primary);
            });
        }

        unsafe {
            let _ = EndPaint(hwnd, &paint);
        }
    }
}

/// The window's client area in DIPs.
fn client_bounds(hwnd: HWND) -> Rect {
    let mut rect = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut rect);
    }
    let scale = win::window_dpi(hwnd) as f32 / win::BASE_DPI as f32;
    Rect::new(0.0, 0.0, (rect.right - rect.left) as f32 / scale, (rect.bottom - rect.top) as f32 / scale)
}

impl WindowProc for Tooltip {
    fn handle(self: &Rc<Self>, hwnd: HWND, msg: u32, _wparam: WPARAM, _lparam: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_NCCREATE => {
                self.hwnd.set(hwnd);
                None
            }
            WM_PAINT => {
                self.paint(hwnd);
                Some(LRESULT(0))
            }
            WM_ERASEBKGND => Some(LRESULT(1)),
            // Clicks and the pointer pass through to whatever is underneath.
            WM_NCHITTEST => Some(LRESULT(HTTRANSPARENT as isize)),
            WM_MOUSEACTIVATE => Some(LRESULT(MA_NOACTIVATE as isize)),
            _ => None,
        }
    }
}
