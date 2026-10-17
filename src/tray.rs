//! The notification-area icon. Its glyph follows the taskbar theme: a white glyph disappears on a
//! light taskbar, and Windows won't recolor it.

use std::cell::Cell;

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::Controls::{LIM_SMALL, LoadIconMetric};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NIN_SELECT,
    NOTIFY_ICON_DATA_FLAGS, NOTIFYICON_VERSION_4, NOTIFYICONDATAW, NOTIFYICONIDENTIFIER, Shell_NotifyIconGetRect,
    Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, HICON, WM_APP};
use windows::core::PCWSTR;

use crate::{theme, win};

/// Sent to the owner window for clicks on the icon; see `TrayEvent::from_lparam`.
pub const CALLBACK_MESSAGE: u32 = WM_APP + 1;

const ICON_ID: u32 = 1;

/// Icon resource IDs in trayulator.rc.
const WHITE_GLYPH: usize = 1;
const BLACK_GLYPH: usize = 2;

/// NOTIFYICONDATA.szTip holds 127 characters and a terminator.
const MAX_TOOLTIP_LENGTH: usize = 127;

/// What happened to the icon, decoded from a `CALLBACK_MESSAGE` (NOTIFYICON_VERSION_4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayEvent {
    /// A left click, or Enter/Space while the icon has keyboard focus.
    Select,
    /// A right click or the menu key; the point is where to show the menu.
    ContextMenu { x: i32, y: i32 },
}

impl TrayEvent {
    pub fn from_message(wparam: usize, lparam: isize) -> Option<Self> {
        const NIN_KEYSELECT: u32 = NIN_SELECT | 1;
        const WM_CONTEXTMENU: u32 = 0x007B;

        match u32::from(win::loword(lparam as usize)) {
            NIN_SELECT | NIN_KEYSELECT => Some(TrayEvent::Select),
            WM_CONTEXTMENU => Some(TrayEvent::ContextMenu { x: win::x_param(wparam as isize), y: win::y_param(wparam as isize) }),
            _ => None,
        }
    }
}

pub struct TrayIcon {
    hwnd: HWND,
    icon: Cell<HICON>,
    tooltip: Cell<[u16; 128]>,
}

impl TrayIcon {
    /// Shows the icon, with its messages going to `hwnd`.
    pub fn new(hwnd: HWND, tooltip: &str) -> Self {
        let this = Self { hwnd, icon: Cell::new(load_icon()), tooltip: Cell::new([0; 128]) };
        this.set_tooltip_text(tooltip);
        this.add();
        this
    }

    /// Adds the icon to the notification area; call again after Explorer restarts.
    pub fn add(&self) {
        let mut data = self.data(NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_ADD, &data);
            data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            let _ = Shell_NotifyIconW(NIM_SETVERSION, &data);
        }
    }

    /// Reloads the glyph for the current taskbar theme.
    pub fn update_icon(&self) {
        let previous = self.icon.replace(load_icon());
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &self.data(NIF_ICON));
            // The shell keeps its own copy, so the old handle can go.
            let _ = DestroyIcon(previous);
        }
    }

    /// Sets the hover text, truncated to what the shell can hold.
    pub fn set_tooltip(&self, text: &str) {
        self.set_tooltip_text(text);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &self.data(NIF_TIP | NIF_SHOWTIP));
        }
    }

    /// The icon's screen rectangle, or `None` when it's hidden in the overflow area.
    pub fn rect(&self) -> Option<RECT> {
        let id = NOTIFYICONIDENTIFIER {
            cbSize: size_of::<NOTIFYICONIDENTIFIER>() as u32,
            hWnd: self.hwnd,
            uID: ICON_ID,
            ..Default::default()
        };
        unsafe { Shell_NotifyIconGetRect(&id) }.ok().filter(|r| r.right > r.left)
    }

    fn set_tooltip_text(&self, text: &str) {
        let mut tip = [0u16; 128];
        for (slot, unit) in tip.iter_mut().zip(text.encode_utf16().take(MAX_TOOLTIP_LENGTH)) {
            *slot = unit;
        }
        self.tooltip.set(tip);
    }

    fn data(&self, flags: NOTIFY_ICON_DATA_FLAGS) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: ICON_ID,
            uFlags: flags,
            uCallbackMessage: CALLBACK_MESSAGE,
            hIcon: self.icon.get(),
            szTip: self.tooltip.get(),
            ..Default::default()
        }
    }
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &self.data(NOTIFY_ICON_DATA_FLAGS(0)));
            let _ = DestroyIcon(self.icon.get());
        }
    }
}

/// The glyph that contrasts with the taskbar, at the small-icon size for the system DPI.
fn load_icon() -> HICON {
    let id = if theme::is_light() { BLACK_GLYPH } else { WHITE_GLYPH };
    unsafe { LoadIconMetric(Some(win::instance()), PCWSTR(id as *const u16), LIM_SMALL) }.unwrap_or_default()
}
