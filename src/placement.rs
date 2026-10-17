//! Positions the flyout next to the tray icon, falling back to the cursor and the taskbar edge
//! (pattern from Traydio, tray-radar, tray-times).

use windows::Win32::Foundation::{POINT, RECT};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint};
use windows::Win32::UI::Shell::{ABE_BOTTOM, ABE_LEFT, ABE_RIGHT, ABE_TOP, ABM_GETTASKBARPOS, APPBARDATA, SHAppBarMessage};
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

use crate::win;

/// Gap between the flyout and the taskbar or screen edge, in logical pixels.
const WINDOW_MARGIN: f32 = 12.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Left,
    Top,
    Right,
    Bottom,
}

/// The taskbar's rectangle and the screen edge it's docked to.
pub fn taskbar() -> Option<(RECT, Edge)> {
    let mut data = APPBARDATA { cbSize: size_of::<APPBARDATA>() as u32, ..Default::default() };
    if unsafe { SHAppBarMessage(ABM_GETTASKBARPOS, &mut data) } == 0 {
        return None;
    }

    let edge = match data.uEdge {
        ABE_LEFT => Edge::Left,
        ABE_TOP => Edge::Top,
        ABE_RIGHT => Edge::Right,
        ABE_BOTTOM => Edge::Bottom,
        _ => return None,
    };
    Some((data.rc, edge))
}

/// The point a popup opens from: the middle of the tray icon at `icon`, or the cursor when the
/// icon's position is unknown (it's in the overflow).
pub fn anchor(icon: Option<RECT>) -> POINT {
    match icon {
        Some(r) => POINT { x: (r.left + r.right) / 2, y: (r.top + r.bottom) / 2 },
        None => {
            let mut cursor = POINT::default();
            unsafe {
                let _ = GetCursorPos(&mut cursor);
            }
            cursor
        }
    }
}

/// The physical rectangle for a popup of `width` × `height` logical pixels next to the tray at
/// `anchor`, sized for that monitor's DPI.
pub fn near_tray(anchor: POINT, width: f32, height: f32) -> RECT {
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe {
        let _ = GetMonitorInfoW(MonitorFromPoint(anchor, MONITOR_DEFAULTTOPRIMARY), &mut info);
    }
    let work = info.rcWork;
    let dpi = win::dpi_for_point(anchor);
    let w = win::to_physical(width, dpi);
    let h = win::to_physical(height, dpi);
    let margin = win::to_physical(WINDOW_MARGIN, dpi);

    let mut x = anchor.x - w / 2;
    let mut y = work.bottom - h - margin;
    if let Some((bar, edge)) = taskbar() {
        match edge {
            Edge::Bottom => y = bar.top.min(work.bottom) - h - margin,
            Edge::Top => y = bar.bottom.max(work.top) + margin,
            Edge::Left => {
                x = bar.right.max(work.left) + margin;
                y = anchor.y - h + margin;
            }
            Edge::Right => {
                x = bar.left.min(work.right) - w - margin;
                y = anchor.y - h + margin;
            }
        }
    }

    let x = x.clamp(work.left + margin, (work.right - w - margin).max(work.left + margin));
    let y = y.clamp(work.top + margin, (work.bottom - h - margin).max(work.top + margin));
    RECT { left: x, top: y, right: x + w, bottom: y + h }
}
