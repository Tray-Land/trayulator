//! Positions the flyout next to the tray icon, falling back to the cursor and the taskbar edge
//! (pattern from Traydio, tray-radar, tray-times).

use std::cmp::Reverse;

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

/// The primary taskbar's rectangle and the screen edge it's docked to. The shell doesn't report
/// the taskbars on other displays; see `monitor_taskbar`.
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
    if let Some(primary) = taskbar() {
        let (bar, edge) = monitor_taskbar(info.rcMonitor, work, anchor, primary);
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

/// The taskbar on the monitor with bounds `monitor` and work area `work`: `primary`, the primary
/// taskbar's rectangle and edge, when it's on this monitor. Otherwise the edge is the side the
/// work area is inset from most, or for an auto-hide taskbar, which reserves no space, the side
/// nearest `anchor`; the rectangle is the part of the monitor outside the work area on that side.
fn monitor_taskbar(monitor: RECT, work: RECT, anchor: POINT, primary: (RECT, Edge)) -> (RECT, Edge) {
    let (bar, edge) = primary;
    if is_on(bar, edge, monitor) {
        return primary;
    }

    // Bottom comes first so it wins ties (`min_by_key` keeps the first): it's where taskbars
    // usually are.
    const EDGES: [Edge; 4] = [Edge::Bottom, Edge::Left, Edge::Top, Edge::Right];
    let point = RECT { left: anchor.x, top: anchor.y, right: anchor.x, bottom: anchor.y };
    let edge = EDGES
        .into_iter()
        .filter(|&edge| inset(monitor, work, edge) > 0)
        .min_by_key(|&edge| Reverse(inset(monitor, work, edge)))
        .or_else(|| EDGES.into_iter().min_by_key(|&edge| inset(monitor, point, edge)))
        .unwrap_or(Edge::Bottom);
    let bar = match edge {
        Edge::Left => RECT { right: work.left, ..monitor },
        Edge::Top => RECT { bottom: work.top, ..monitor },
        Edge::Right => RECT { left: work.right, ..monitor },
        Edge::Bottom => RECT { top: work.bottom, ..monitor },
    };
    (bar, edge)
}

/// Whether the taskbar `bar`, docked to `edge`, is on `monitor`. This tests the bar's inner side,
/// the one facing the middle of the screen: an auto-hidden bar's rectangle can lie mostly past
/// its monitor's edge, even over the next monitor.
fn is_on(bar: RECT, edge: Edge, monitor: RECT) -> bool {
    let (middle_x, middle_y) = ((bar.left + bar.right) / 2, (bar.top + bar.bottom) / 2);
    let (x, y) = match edge {
        Edge::Left => (bar.right - 1, middle_y),
        Edge::Top => (middle_x, bar.bottom - 1),
        Edge::Right => (bar.left, middle_y),
        Edge::Bottom => (middle_x, bar.top),
    };
    (monitor.left..monitor.right).contains(&x) && (monitor.top..monitor.bottom).contains(&y)
}

/// How far `inner` is inset from `outer` on the `edge` side.
fn inset(outer: RECT, inner: RECT, edge: Edge) -> i32 {
    match edge {
        Edge::Left => inner.left - outer.left,
        Edge::Top => inner.top - outer.top,
        Edge::Right => outer.right - inner.right,
        Edge::Bottom => outer.bottom - inner.bottom,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT { left, top, right, bottom }
    }

    /// A 1920 × 1080 primary monitor with its taskbar along the bottom, and another monitor of the
    /// same size to its right.
    const PRIMARY: RECT = rect(0, 0, 1920, 1080);
    const PRIMARY_BAR: (RECT, Edge) = (rect(0, 1032, 1920, 1080), Edge::Bottom);
    const OTHER: RECT = rect(1920, 0, 3840, 1080);
    const OTHER_MIDDLE: POINT = POINT { x: 2880, y: 540 };

    #[test]
    fn monitor_taskbar_primary_monitor_returns_primary_bar() {
        let work = rect(0, 0, 1920, 1032);
        assert_eq!(monitor_taskbar(PRIMARY, work, POINT { x: 1800, y: 1056 }, PRIMARY_BAR), PRIMARY_BAR);
    }

    #[test]
    fn monitor_taskbar_other_monitor_inset_bottom_returns_bottom() {
        let work = rect(1920, 0, 3840, 1032);
        assert_eq!(monitor_taskbar(OTHER, work, OTHER_MIDDLE, PRIMARY_BAR), (rect(1920, 1032, 3840, 1080), Edge::Bottom));
    }

    #[test]
    fn monitor_taskbar_other_monitor_inset_top_returns_top() {
        let work = rect(1920, 48, 3840, 1080);
        assert_eq!(monitor_taskbar(OTHER, work, OTHER_MIDDLE, PRIMARY_BAR), (rect(1920, 0, 3840, 48), Edge::Top));
    }

    #[test]
    fn monitor_taskbar_other_monitor_inset_left_returns_left() {
        let work = rect(1968, 0, 3840, 1080);
        assert_eq!(monitor_taskbar(OTHER, work, OTHER_MIDDLE, PRIMARY_BAR), (rect(1920, 0, 1968, 1080), Edge::Left));
    }

    #[test]
    fn monitor_taskbar_other_monitor_inset_right_returns_right() {
        let work = rect(1920, 0, 3792, 1080);
        assert_eq!(monitor_taskbar(OTHER, work, OTHER_MIDDLE, PRIMARY_BAR), (rect(3792, 0, 3840, 1080), Edge::Right));
    }

    #[test]
    fn monitor_taskbar_other_monitor_auto_hide_returns_edge_nearest_anchor() {
        let anchor = POINT { x: 3830, y: 600 };
        assert_eq!(monitor_taskbar(OTHER, OTHER, anchor, PRIMARY_BAR), (rect(3840, 0, 3840, 1080), Edge::Right));
    }

    #[test]
    fn monitor_taskbar_hidden_primary_bar_returns_it_on_primary_only() {
        // Auto-hidden, the primary bar reaches past the primary monitor over the one below it.
        let hidden = (rect(0, 1078, 1920, 1126), Edge::Bottom);
        let below = rect(0, 1080, 1920, 2160);
        let below_work = rect(0, 1080, 1920, 2112);
        assert_eq!(monitor_taskbar(PRIMARY, PRIMARY, POINT { x: 960, y: 1079 }, hidden), hidden);
        assert_eq!(monitor_taskbar(below, below_work, POINT { x: 960, y: 1620 }, hidden), (rect(0, 2112, 1920, 2160), Edge::Bottom));
    }
}
