//! Borderless, always-on-top popup shown against the tray icon, styled like the shell's own
//! flyouts. Slides and fades in, light-dismisses when focus leaves unless pinned. Dismissing only
//! hides it, so reopening soon after is instant; after a minute hidden it closes for real and the
//! next open builds a new window, so the idle app holds no graphics resources.
//!
//! Re-entrancy: Win32 calls such as `ShowWindow` deliver messages to this window before they
//! return. Methods here never hold a `RefCell` borrow across such calls, and message handlers
//! use `try_borrow` so a nested message can't panic.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMSBT_TRANSIENTWINDOW, DWMWA_CAPTION_COLOR, DWMWA_COLOR_NONE, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE,
    DWMWCP_ROUND, DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, InvalidateRect, PAINTSTRUCT, ScreenToClient};
use windows::Win32::UI::Accessibility::{IRawElementProviderSimple, UiaDisconnectProvider, UiaRect, UiaReturnRawElementProvider, UiaRootObjectId};
use windows::Win32::UI::Controls::{MARGINS, WM_MOUSELEAVE};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VIRTUAL_KEY, VK_CONTROL, VK_ESCAPE,
    VK_H, VK_LEFT, VK_SHIFT, VK_TAB,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, DestroyWindow, GA_ROOT, GetAncestor, GetCaretBlinkTime, GetClassNameW, GetCursorPos, GetForegroundWindow,
    HTCLIENT, HWND_TOPMOST, IDC_ARROW, IDC_IBEAM, KillTimer, LWA_ALPHA, LoadCursorW, PostMessageW, SW_HIDE, SW_SHOW,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetCursor, SetForegroundWindow, SetLayeredWindowAttributes,
    SetTimer, SetWindowPos, ShowWindow, WA_INACTIVE, WM_ACTIVATE, WM_APP, WM_CHAR, WM_CLOSE, WM_DESTROY, WM_DPICHANGED,
    WM_DWMCOLORIZATIONCOLORCHANGED, WM_ERASEBKGND, WM_GETOBJECT, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCACTIVATE, WM_NCCALCSIZE, WM_NCCREATE, WM_NCDESTROY, WM_NCHITTEST, WM_PAINT, WM_SETCURSOR, WM_SETTINGCHANGE, WM_SYSCHAR, WM_SYSKEYDOWN, WM_TIMER, WM_XBUTTONUP, WS_EX_LAYERED,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_THICKFRAME, CS_DBLCLKS,
};
use windows::ApplicationModel::StartupTaskState;
use windows::core::{BOOL, PCWSTR, Result, w};

use super::a11y::{self, Host};
use super::anim::Ease;
use super::graphics::Graphics;
use super::page::{self, Command, Page};
use super::palette::{Accents, Color, Palette};
use super::tooltip::Tooltip;
use super::{Element, Id, Rect};
use crate::app::DISPLAY_NAME;
use crate::calc::history::CalculationHistory;
use crate::{placement, startup, theme, win};

/// Posted to the owner when the window has closed for good.
pub const CLOSED_MESSAGE: u32 = WM_APP + 2;
/// Posted to the flyout with the start-with-Windows state: 0 when unknown, else the state + 1.
/// LPARAM is the request it answers.
const STARTUP_MESSAGE: u32 = WM_APP + 3;

const CLASS_NAME: PCWSTR = w!("Trayulator.Flyout");

const TIMER_HIDE: usize = 1;
const TIMER_CLOSE: usize = 2;
const TIMER_ANIMATION: usize = 3;
const TIMER_CARET: usize = 4;
const TIMER_TOOLTIP: usize = 5;

/// Focus bounces briefly while menus open; wait this long before light-dismissing.
const HIDE_DEBOUNCE_MS: u32 = 150;
const CLOSE_DELAY_MS: u32 = 60_000;
const FRAME_MS: u32 = 15;
const TOOLTIP_DELAY_MS: u32 = 600;

/// A tray click right after a light-dismiss is the same click that dismissed the popup (focus
/// left to the taskbar); treat it as "close" instead of re-showing.
const RECENT_DISMISS: Duration = Duration::from_millis(300);

// The backdrop paints behind the content, so the fade uses per-window layered alpha, alongside a
// small slide.
const SHOW_SLIDE: f32 = 24.0;
const HIDE_SLIDE: f32 = 12.0;
const SHOW_DURATION: Duration = Duration::from_millis(250);
const HIDE_DURATION: Duration = Duration::from_millis(180);

thread_local! {
    static REGISTERED: Cell<bool> = const { Cell::new(false) };
}

/// The window sliding and fading in or out. Offsets are physical pixels below the resting place.
#[derive(Clone, Copy)]
struct WindowMotion {
    start: Instant,
    hiding: bool,
    from_offset: i32,
    to_offset: i32,
    from_opacity: f32,
    to_opacity: f32,
}

/// Where startup workers post their replies: the flyout until it's destroyed, then nowhere, so a
/// worker that outlives the window can't post to a dead (or reused) handle.
#[derive(Clone, Default)]
struct ReplyTarget(Arc<Mutex<Option<isize>>>);

impl ReplyTarget {
    fn set(&self, hwnd: Option<HWND>) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = hwnd.map(|hwnd| hwnd.0 as isize);
    }

    /// Posts unless the window is gone. The lock is held while posting (which doesn't wait), so
    /// the window can't be cleared between the check and the post.
    fn post(&self, msg: u32, wparam: WPARAM, lparam: LPARAM) {
        let target = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(hwnd) = *target {
            unsafe {
                let _ = PostMessageW(Some(HWND(hwnd as *mut _)), msg, wparam, lparam);
            }
        }
    }
}

pub struct FlyoutWindow {
    me: Weak<FlyoutWindow>,
    hwnd: Cell<HWND>,
    owner: HWND,
    on_status: Box<dyn Fn(&str)>,
    page: RefCell<Page>,
    graphics: RefCell<Option<Graphics>>,
    palette: RefCell<Palette>,
    /// The system acrylic backdrop is available (Windows 11 22H2 and later).
    backdrop: Cell<bool>,
    active: Cell<bool>,
    visible: Cell<bool>,
    /// Between showing and the window's first activation, deactivation isn't a dismissal.
    showing: Cell<bool>,
    allow_close: Cell<bool>,
    closed: Cell<bool>,
    last_dismissed: Cell<Option<Instant>>,
    /// Where the window was last opened from, to place it again if its DPI changes on screen.
    anchor: Cell<POINT>,
    base: Cell<(i32, i32)>,
    frame: Cell<(i32, f32)>,
    motion: Cell<Option<WindowMotion>>,
    animating: Cell<bool>,
    tracking_mouse: Cell<bool>,
    tooltip: RefCell<Option<Rc<Tooltip>>>,
    tooltip_target: Cell<Option<Id>>,
    accessibility: RefCell<Option<IRawElementProviderSimple>>,
    /// The latest start-with-Windows request; replies to earlier ones are stale.
    startup_request: Cell<usize>,
    /// The latest request's worker, which the next one waits for.
    startup_worker: Cell<Option<JoinHandle<()>>>,
    startup_reply: ReplyTarget,
}

impl FlyoutWindow {
    /// Creates the (hidden) flyout. `on_status` receives a short status for the tray tooltip;
    /// `owner` gets `CLOSED_MESSAGE` when the window closes for good.
    pub fn new(owner: HWND, history: Rc<RefCell<CalculationHistory>>, on_status: Box<dyn Fn(&str)>) -> Result<Rc<Self>> {
        if !REGISTERED.get() {
            let cursor = unsafe { LoadCursorW(None, IDC_ARROW)? };
            win::register_class::<Self>(CLASS_NAME, CS_DBLCLKS, cursor)?;
            REGISTERED.set(true);
        }

        let this = Rc::new_cyclic(|me| Self {
            me: me.clone(),
            hwnd: Cell::default(),
            owner,
            on_status,
            page: RefCell::new(Page::new(history)),
            graphics: RefCell::new(None),
            palette: RefCell::new(Palette::new(theme::is_light(), &Accents::read(), false)),
            backdrop: Cell::new(false),
            active: Cell::new(false),
            visible: Cell::new(false),
            showing: Cell::new(false),
            allow_close: Cell::new(false),
            closed: Cell::new(false),
            last_dismissed: Cell::new(None),
            anchor: Cell::new(POINT::default()),
            base: Cell::new((0, 0)),
            frame: Cell::new((0, 0.0)),
            motion: Cell::new(None),
            animating: Cell::new(false),
            tracking_mouse: Cell::new(false),
            tooltip: RefCell::new(None),
            tooltip_target: Cell::new(None),
            accessibility: RefCell::new(None),
            startup_request: Cell::new(0),
            startup_worker: Cell::new(None),
            startup_reply: ReplyTarget::default(),
        });

        // DWM only draws a system backdrop for windows with a frame, so this one has a sizing
        // frame that WM_NCCALCSIZE gives to the client area. WS_EX_LAYERED enables the fade;
        // WS_EX_TOOLWINDOW keeps it off the taskbar and Alt+Tab.
        let style = WS_POPUP | WS_THICKFRAME;
        let ex_style = WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_LAYERED;
        let title = windows::core::HSTRING::from(DISPLAY_NAME);
        let hwnd = win::create_window(&this, CLASS_NAME, PCWSTR(title.as_ptr()), style, ex_style, RECT::default(), None)?;
        this.configure_chrome(hwnd);
        Ok(this)
    }

    pub fn is_closed(&self) -> bool {
        self.closed.get()
    }

    /// Shows the popup, or hides it if already visible (tray-icon click behavior).
    pub fn toggle(&self, icon: Option<RECT>) {
        if self.visible.get() {
            self.hide();
            return;
        }
        if self.last_dismissed.get().is_some_and(|t| t.elapsed() < RECENT_DISMISS) {
            return;
        }
        self.show(icon);
    }

    /// Shows the popup next to the tray icon at `icon` (or the cursor), on the calculator.
    pub fn show(&self, icon: Option<RECT>) {
        let hwnd = self.hwnd.get();
        // Position before showing so the window never flashes at a stale location.
        self.anchor.set(placement::anchor(icon));
        let rect = placement::near_tray(self.anchor.get(), page::WIDTH, page::HEIGHT);
        self.base.set((rect.left, rect.top));
        unsafe {
            let _ = KillTimer(Some(hwnd), TIMER_HIDE);
            let _ = KillTimer(Some(hwnd), TIMER_CLOSE);
            let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), rect.left, rect.top, rect.right - rect.left, rect.bottom - rect.top, SWP_NOACTIVATE);
        }

        self.showing.set(true);
        self.visible.set(true);
        // Re-read here too: not every taskbar setting change is broadcast.
        self.apply_theme();
        if let Ok(mut page) = self.page.try_borrow_mut() {
            page.on_shown();
        }

        let from = self.scale(SHOW_SLIDE);
        self.motion.set(Some(WindowMotion {
            start: Instant::now(),
            hiding: false,
            from_offset: from,
            to_offset: 0,
            from_opacity: 0.0,
            to_opacity: 1.0,
        }));
        self.apply_frame(from, 0.0);

        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
        }
        self.start_animation();
        self.invalidate();
    }

    /// Switches the (visible) flyout to its settings page.
    pub fn show_settings(&self) {
        self.with_page(|page, _, out| page.show_settings(out));
        self.after_input();
    }

    pub fn hide(&self) {
        let hwnd = self.hwnd.get();
        unsafe {
            let _ = KillTimer(Some(hwnd), TIMER_HIDE);
        }
        self.showing.set(false);
        if !self.visible.get() {
            return;
        }

        self.visible.set(false);
        self.last_dismissed.set(Some(Instant::now()));
        self.hide_tooltip();

        let (offset, opacity) = self.frame.get();
        self.motion.set(Some(WindowMotion {
            start: Instant::now(),
            hiding: true,
            from_offset: offset,
            to_offset: self.scale(HIDE_SLIDE),
            from_opacity: opacity,
            to_opacity: 0.0,
        }));
        self.start_animation();

        unsafe {
            SetTimer(Some(hwnd), TIMER_CLOSE, CLOSE_DELAY_MS, None);
        }
    }

    /// Closes the window for real: stops its timers and destroys it, releasing everything it
    /// holds. The owner hears about it through `CLOSED_MESSAGE`.
    pub fn close(&self) {
        if self.closed.replace(true) {
            return;
        }

        let hwnd = self.hwnd.get();
        unsafe {
            for timer in [TIMER_HIDE, TIMER_CLOSE, TIMER_ANIMATION, TIMER_CARET, TIMER_TOOLTIP] {
                let _ = KillTimer(Some(hwnd), timer);
            }
        }
        self.allow_close.set(true);
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }

    fn configure_chrome(&self, hwnd: HWND) {
        unsafe {
            let corner = DWMWCP_ROUND;
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, (&raw const corner).cast(), size_of_val(&corner) as u32);

            // The whole window is "frame", so the backdrop shows wherever the content is clear.
            let margins = MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 };
            let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);

            // No caption color: with accent-colored title bars on, DWM would paint one across the top.
            let none = DWMWA_COLOR_NONE;
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_CAPTION_COLOR, (&raw const none).cast(), size_of_val(&none) as u32);

            let backdrop = DWMSBT_TRANSIENTWINDOW;
            let supported = DwmSetWindowAttribute(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, (&raw const backdrop).cast(), size_of_val(&backdrop) as u32);
            self.backdrop.set(supported.is_ok());

            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 0, LWA_ALPHA);
        }
    }

    /// Matches the taskbar's mode and accent tint, like the shell's own flyouts.
    fn apply_theme(&self) {
        let light = theme::is_light();
        let palette = Palette::new(light, &Accents::read(), theme::shows_accent_color());
        if let Ok(mut current) = self.palette.try_borrow_mut() {
            *current = palette;
        }

        let dark = BOOL::from(!light);
        unsafe {
            let _ = DwmSetWindowAttribute(self.hwnd.get(), DWMWA_USE_IMMERSIVE_DARK_MODE, (&raw const dark).cast(), size_of::<BOOL>() as u32);
        }
        self.invalidate();
    }

    fn apply_frame(&self, offset: i32, opacity: f32) {
        self.frame.set((offset, opacity));
        let (x, y) = self.base.get();
        let hwnd = self.hwnd.get();
        unsafe {
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), (opacity * 255.0).round().clamp(0.0, 255.0) as u8, LWA_ALPHA);
            let _ = SetWindowPos(hwnd, None, x, y + offset, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }

    fn start_animation(&self) {
        if !self.animating.replace(true) {
            unsafe {
                SetTimer(Some(self.hwnd.get()), TIMER_ANIMATION, FRAME_MS, None);
            }
        }
    }

    /// One animation frame: the window's slide and fade, and a repaint for the page's motion.
    fn animation_tick(&self) {
        let now = Instant::now();
        if let Some(motion) = self.motion.get() {
            let duration = if motion.hiding { HIDE_DURATION } else { SHOW_DURATION };
            let progress = (now.saturating_duration_since(motion.start).as_secs_f32() / duration.as_secs_f32()).min(1.0);

            // Cubic ease-out on the way in, ease-in on the way out; linear fade.
            let eased = if motion.hiding { Ease::CubicIn } else { Ease::CubicOut }.apply(progress);
            let offset = motion.from_offset + ((motion.to_offset - motion.from_offset) as f32 * eased).round() as i32;
            let opacity = motion.from_opacity + (motion.to_opacity - motion.from_opacity) * progress;
            self.apply_frame(offset, opacity);

            if progress >= 1.0 {
                self.motion.set(None);
                if motion.hiding {
                    unsafe {
                        let _ = ShowWindow(self.hwnd.get(), SW_HIDE);
                    }
                }
            }
        }

        let page_animating = self.page.try_borrow().map_or(true, |page| page.is_animating(now));
        self.invalidate();
        if self.motion.get().is_none() && !page_animating {
            self.animating.set(false);
            unsafe {
                let _ = KillTimer(Some(self.hwnd.get()), TIMER_ANIMATION);
            }
        }
    }

    fn on_activate(&self, active: bool) {
        self.active.set(active);
        if let Ok(mut page) = self.page.try_borrow_mut() {
            page.set_active(active);
        }

        let hwnd = self.hwnd.get();
        if active {
            self.showing.set(false);
            unsafe {
                let _ = KillTimer(Some(hwnd), TIMER_HIDE);
            }
            if let Some(focus) = self.page.try_borrow().ok().map(|p| p.focus()) {
                self.raise_focus_changed(focus);
            }
        } else {
            self.hide_tooltip();
            if !self.showing.get() && self.visible.get() {
                unsafe {
                    SetTimer(Some(hwnd), TIMER_HIDE, HIDE_DEBOUNCE_MS, None);
                }
            }
        }

        self.update_caret();
        self.invalidate();
    }

    fn hide_if_focus_left(&self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd.get()), TIMER_HIDE);
        }

        // Pinned: stay on screen (always on top) until hidden explicitly with Escape or the tray icon.
        if self.page.try_borrow().is_ok_and(|page| page.is_pinned()) {
            return;
        }
        if self.visible.get() && !self.should_remain_visible(unsafe { GetForegroundWindow() }) {
            self.hide();
        }
    }

    fn should_remain_visible(&self, foreground: HWND) -> bool {
        let hwnd = self.hwnd.get();
        if foreground.is_invalid() {
            return false;
        }
        if foreground == hwnd || unsafe { GetAncestor(foreground, GA_ROOT) } == hwnd {
            return true;
        }

        // A classic menu (#32768) opening, such as the tray icon's.
        let mut class = [0u16; 64];
        let length = unsafe { GetClassNameW(foreground, &mut class) } as usize;
        String::from_utf16_lossy(&class[..length]) == "#32768"
    }

    /// Runs `f` against the page and its text system, then carries out the commands it issued
    /// and announces any focus change. `None` when the page is busy (a nested message).
    fn with_page<R>(&self, f: impl FnOnce(&mut Page, &super::graphics::TextSystem, &mut Vec<Command>) -> R) -> Option<R> {
        let mut commands = Vec::new();
        let (result, focus_before, focus_after) = {
            let mut graphics = self.graphics.try_borrow_mut().ok()?;
            if graphics.is_none() {
                *graphics = Graphics::new().ok();
            }
            let graphics = graphics.as_ref()?;
            let mut page = self.page.try_borrow_mut().ok()?;
            let before = page.focus();
            let result = f(&mut page, &graphics.text, &mut commands);
            (result, before, page.focus())
        };

        // Commands, and so toggles, go first: screen readers speak a toggle on the element they
        // have focus on, so the Pin button's flip is raised before focus moves on to the entry.
        for command in commands {
            self.run(command);
        }
        if focus_after != focus_before {
            self.raise_focus_changed(focus_after);
        }
        Some(result)
    }

    /// Repaints, restarts the caret blink and keeps any new motion running after input.
    fn after_input(&self) {
        if let Ok(mut page) = self.page.try_borrow_mut() {
            page.show_caret();
        }
        self.update_caret();
        self.start_animation();
        self.invalidate();
    }

    fn run(&self, command: Command) {
        match command {
            Command::SetTrayStatus(status) => (self.on_status)(&status),
            Command::QueryStartup => self.update_startup(None),
            Command::SetStartup(on) => self.update_startup(Some(on)),
            Command::Toggled { id, on } => self.raise_toggled(id, on),
        }
    }

    /// Reads or changes the startup task on a worker thread (the calls block), then posts the
    /// resulting state back. Each worker waits for the one before it, so a read never runs
    /// alongside a change, and the reply names its request so only the latest one is used.
    fn update_startup(&self, enable: Option<bool>) {
        let request = self.startup_request.get().wrapping_add(1);
        self.startup_request.set(request);
        let reply = self.startup_reply.clone();
        let worker = spawn_after(self.startup_worker.take(), move || {
            let state = match enable {
                Some(on) => startup::set_enabled(on),
                None => startup::state(),
            };
            let code = state.map_or(0, |s| s.0 as usize + 1);
            reply.post(STARTUP_MESSAGE, WPARAM(code), LPARAM(request as isize));
        });
        self.startup_worker.set(Some(worker));
    }

    fn update_caret(&self) {
        let hwnd = self.hwnd.get();
        let wants_caret = self.page.try_borrow().is_ok_and(|page| page.wants_caret());
        let blink = unsafe { GetCaretBlinkTime() };
        unsafe {
            // INFINITE (u32::MAX) means the user turned blinking off.
            if wants_caret && blink != u32::MAX && blink != 0 {
                SetTimer(Some(hwnd), TIMER_CARET, blink, None);
            } else {
                let _ = KillTimer(Some(hwnd), TIMER_CARET);
            }
        }
    }

    fn on_key(&self, key: VIRTUAL_KEY) {
        let pressed = |k: VIRTUAL_KEY| unsafe { GetKeyState(i32::from(k.0)) } < 0;
        let (ctrl, shift) = (pressed(VK_CONTROL), pressed(VK_SHIFT));
        self.hide_tooltip();

        match key {
            VK_ESCAPE => {
                // Escape steps back out of settings or history first, then hides.
                if self.with_page(|page, _, out| page.go_back(out)) == Some(false) {
                    self.hide();
                }
            }
            VK_TAB => {
                self.with_page(|page, text, _| page.move_focus(text, !shift));
            }
            VK_H if ctrl => {
                self.with_page(|page, _, out| page.toggle_history(out));
            }
            _ => {
                self.with_page(|page, _, out| page.key_down(key, ctrl, shift, out));
            }
        }
        self.after_input();
    }

    fn go_back(&self) {
        self.with_page(|page, _, out| page.go_back(out));
        self.after_input();
    }

    fn on_mouse_move(&self, lparam: LPARAM) {
        if !self.tracking_mouse.replace(true) {
            let mut track = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: self.hwnd.get(), dwHoverTime: 0 };
            unsafe {
                let _ = TrackMouseEvent(&mut track);
            }
        }

        let (x, y) = self.to_dips(win::x_param(lparam.0), win::y_param(lparam.0));
        let changed = self.with_page(|page, text, _| {
            let before = page.hover();
            page.pointer_move(text, x, y);
            page.hover() != before || page.is_selecting()
        });
        if changed == Some(true) {
            self.invalidate();
            self.update_tooltip_target();
        }
    }

    fn on_mouse_leave(&self) {
        self.tracking_mouse.set(false);
        self.with_page(|page, _, _| page.pointer_leave());
        self.update_tooltip_target();
        self.invalidate();
    }

    fn on_button_down(&self, lparam: LPARAM, double: bool) {
        self.hide_tooltip();
        let (x, y) = self.to_dips(win::x_param(lparam.0), win::y_param(lparam.0));
        let shift = unsafe { GetKeyState(i32::from(VK_SHIFT.0)) } < 0;
        if self.with_page(|page, text, _| page.pointer_down(text, x, y, shift, double)) == Some(true) {
            unsafe { SetCapture(self.hwnd.get()) };
        }
        self.after_input();
    }

    fn on_button_up(&self, lparam: LPARAM) {
        unsafe {
            let _ = ReleaseCapture();
        }
        let (x, y) = self.to_dips(win::x_param(lparam.0), win::y_param(lparam.0));
        self.with_page(|page, text, out| page.pointer_up(text, x, y, out));
        self.after_input();
    }

    fn on_wheel(&self, wparam: WPARAM, lparam: LPARAM) {
        let mut point = POINT { x: win::x_param(lparam.0), y: win::y_param(lparam.0) };
        unsafe {
            let _ = ScreenToClient(self.hwnd.get(), &mut point);
        }
        let (x, y) = self.to_dips(point.x, point.y);
        let notches = f32::from(((wparam.0 >> 16) & 0xFFFF) as u16 as i16) / 120.0;
        self.with_page(|page, text, _| page.wheel(text, x, y, notches));
        self.hide_tooltip();
        self.invalidate();
    }

    fn on_set_cursor(&self) {
        let mut point = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut point);
            let _ = ScreenToClient(self.hwnd.get(), &mut point);
        }
        let (x, y) = self.to_dips(point.x, point.y);
        let over_entry = self.with_page(|page, text, _| page.hit(text, x, y).is_some_and(|e| e.id == Id::Entry));
        let cursor = if over_entry == Some(true) { IDC_IBEAM } else { IDC_ARROW };
        unsafe {
            if let Ok(cursor) = LoadCursorW(None, cursor) {
                SetCursor(Some(cursor));
            }
        }
    }

    /// Starts the tooltip delay when the pointer moves onto a new element.
    fn update_tooltip_target(&self) {
        let hover = self.page.try_borrow().ok().and_then(|page| page.hover());
        if hover == self.tooltip_target.get() {
            return;
        }

        self.tooltip_target.set(hover);
        self.hide_tooltip();
        if hover.is_some() {
            unsafe {
                SetTimer(Some(self.hwnd.get()), TIMER_TOOLTIP, TOOLTIP_DELAY_MS, None);
            }
        }
    }

    fn show_tooltip(&self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd.get()), TIMER_TOOLTIP);
        }
        let Some(target) = self.tooltip_target.get() else {
            return;
        };
        let Some(element) = self.elements().and_then(|elements| elements.into_iter().find(|e| e.id == target)) else {
            return;
        };
        let Some(help) = element.help else {
            return;
        };

        if self.tooltip.borrow().is_none() {
            *self.tooltip.borrow_mut() = Tooltip::new(self.hwnd.get()).ok();
        }
        let tooltip = self.tooltip.borrow().clone();
        let palette = self.palette.borrow().clone();
        if let Some(tooltip) = tooltip {
            tooltip.show(&help, self.to_screen_rect(element.rect), &palette);
        }
    }

    fn hide_tooltip(&self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd.get()), TIMER_TOOLTIP);
        }
        let tooltip = self.tooltip.borrow().clone();
        if let Some(tooltip) = tooltip {
            tooltip.hide();
        }
    }

    fn paint(&self) {
        let hwnd = self.hwnd.get();
        let mut paint = PAINTSTRUCT::default();
        unsafe { BeginPaint(hwnd, &mut paint) };

        let now = Instant::now();
        let animating = (|| {
            let mut graphics = self.graphics.try_borrow_mut().ok()?;
            if graphics.is_none() {
                *graphics = Graphics::new().ok();
            }
            let graphics = graphics.as_mut()?;
            let mut page = self.page.try_borrow_mut().ok()?;
            let palette = self.palette.try_borrow().ok()?;

            let background = if self.backdrop.get() { Color::TRANSPARENT } else { palette.solid_background };
            graphics.draw(hwnd, win::window_dpi(hwnd), &palette, background, |canvas| {
                if let Some(tint) = palette.tint {
                    canvas.fill(Rect::new(0.0, 0.0, 4.0 * page::WIDTH, 4.0 * page::HEIGHT), 0.0, tint);
                }
                page.paint(canvas, now);
            });
            Some(page.is_animating(now))
        })();

        unsafe {
            let _ = EndPaint(hwnd, &paint);
        }
        if animating == Some(true) {
            self.start_animation();
        }
    }

    fn invalidate(&self) {
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd.get()), None, false);
        }
    }

    /// The window moved to a monitor with another scale, or its monitor's scale changed. The page
    /// is a fixed size in DIPs, so the size follows from `dpi` alone; Windows' suggested size is
    /// no use, since when `show` places the window on another monitor it's already sized for that
    /// monitor, and the suggestion scales it a second time. On screen, the flyout goes back
    /// against the tray at its new size: keeping its top-left would leave it overlapping the
    /// taskbar or floating above it. Drawing picks up the new DPI on the next paint.
    fn on_dpi_changed(&self, dpi: u32) {
        let hwnd = self.hwnd.get();
        unsafe {
            if self.visible.get() {
                let rect = placement::near_tray(self.anchor.get(), page::WIDTH, page::HEIGHT);
                self.base.set((rect.left, rect.top));
                let (offset, _) = self.frame.get();
                let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
                let _ = SetWindowPos(hwnd, None, rect.left, rect.top + offset, width, height, SWP_NOZORDER | SWP_NOACTIVATE);
            } else {
                let (width, height) = (win::to_physical(page::WIDTH, dpi), win::to_physical(page::HEIGHT, dpi));
                let _ = SetWindowPos(hwnd, None, 0, 0, width, height, SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOMOVE);
            }
        }
        self.invalidate();
    }

    fn scale(&self, logical: f32) -> i32 {
        (logical * win::window_dpi(self.hwnd.get()) as f32 / win::BASE_DPI as f32).round() as i32
    }

    fn to_dips(&self, x: i32, y: i32) -> (f32, f32) {
        let scale = win::window_dpi(self.hwnd.get()) as f32 / win::BASE_DPI as f32;
        (x as f32 / scale, y as f32 / scale)
    }

    fn to_screen_rect(&self, rect: Rect) -> RECT {
        let scale = win::window_dpi(self.hwnd.get()) as f32 / win::BASE_DPI as f32;
        let origin = win::client_to_screen(self.hwnd.get(), (rect.x * scale).round() as i32, (rect.y * scale).round() as i32);
        RECT { left: origin.x, top: origin.y, right: origin.x + (rect.w * scale).round() as i32, bottom: origin.y + (rect.h * scale).round() as i32 }
    }

    fn host(&self) -> Weak<dyn Host> {
        self.me.clone()
    }

    // The raise functions take the root out of its RefCell first: raising an event can bring a
    // nested WM_GETOBJECT, which needs to borrow it.
    fn raise_focus_changed(&self, focus: Id) {
        let root = self.accessibility.borrow().clone();
        if let Some(root) = root {
            a11y::raise_focus_changed(&root, self.host(), focus);
        }
    }

    fn raise_toggled(&self, id: Id, on: bool) {
        let root = self.accessibility.borrow().clone();
        if let Some(root) = root {
            a11y::raise_toggled(&root, self.host(), id, on);
        }
    }

    fn get_object(&self, hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        if lparam.0 as i32 != UiaRootObjectId {
            return None;
        }
        let root = self.accessibility.try_borrow_mut().ok()?.get_or_insert_with(|| a11y::root(self.host())).clone();
        Some(unsafe { UiaReturnRawElementProvider(hwnd, wparam, lparam, &root) })
    }

    fn on_destroy(&self, hwnd: HWND) {
        let root = self.accessibility.borrow_mut().take();
        if let Some(root) = root {
            unsafe {
                UiaReturnRawElementProvider(hwnd, WPARAM(0), LPARAM(0), None);
                let _ = UiaDisconnectProvider(&root);
            }
        }
        self.tooltip.borrow_mut().take();
        self.graphics.borrow_mut().take();

        // Startup workers aren't joined: one can be blocked in a WinRT call, and waiting would
        // hang the UI thread. With the target cleared, one that finishes later posts nothing, and
        // at Exit the process ends it.
        self.startup_reply.set(None);
    }
}

impl Host for FlyoutWindow {
    fn hwnd(&self) -> HWND {
        self.hwnd.get()
    }

    fn elements(&self) -> Option<Vec<Element>> {
        let mut graphics = self.graphics.try_borrow_mut().ok()?;
        if graphics.is_none() {
            *graphics = Graphics::new().ok();
        }
        let page = self.page.try_borrow().ok()?;
        Some(page.elements(&graphics.as_ref()?.text))
    }

    fn focused(&self) -> Option<Id> {
        if !self.active.get() {
            return None;
        }
        self.page.try_borrow().ok().map(|page| page.focus())
    }

    fn value(&self, id: Id) -> Option<String> {
        (id == Id::Entry).then(|| self.page.try_borrow().ok().map(|page| page.entry_text())).flatten()
    }

    fn screen_rect(&self, rect: Rect) -> UiaRect {
        let r = self.to_screen_rect(rect);
        UiaRect { left: r.left.into(), top: r.top.into(), width: (r.right - r.left).into(), height: (r.bottom - r.top).into() }
    }

    fn client_point(&self, x: f64, y: f64) -> (f32, f32) {
        let mut point = POINT { x: x as i32, y: y as i32 };
        unsafe {
            let _ = ScreenToClient(self.hwnd.get(), &mut point);
        }
        self.to_dips(point.x, point.y)
    }

    fn invoke(&self, id: Id) {
        self.with_page(|page, _, out| page.activate(id, out));
        self.after_input();
    }

    fn focus(&self, id: Id) {
        self.with_page(|page, _, _| page.set_focus(id, true));
        self.after_input();
    }

    fn set_value(&self, id: Id, value: &str) {
        if id == Id::Entry {
            self.with_page(|page, _, _| page.set_entry_value(value));
            self.after_input();
        }
    }
}

impl win::WindowProc for FlyoutWindow {
    fn handle(self: &Rc<Self>, hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_NCCREATE => {
                self.hwnd.set(hwnd);
                self.startup_reply.set(Some(hwnd));
                None
            }
            // The frame belongs to the client area, and it isn't for resizing.
            WM_NCCALCSIZE if wparam.0 != 0 => Some(LRESULT(0)),
            WM_NCHITTEST => Some(LRESULT(HTCLIENT as isize)),
            // Keep the active look, so the acrylic stays while a pinned flyout is in the background.
            WM_NCACTIVATE => Some(unsafe { DefWindowProcW(hwnd, msg, WPARAM(1), lparam) }),
            WM_ERASEBKGND => Some(LRESULT(1)),
            WM_PAINT => {
                self.paint();
                Some(LRESULT(0))
            }
            WM_ACTIVATE => {
                self.on_activate(win::loword(wparam.0) != WA_INACTIVE as u16);
                Some(LRESULT(0))
            }
            WM_TIMER => {
                match wparam.0 {
                    TIMER_HIDE => self.hide_if_focus_left(),
                    TIMER_CLOSE if !self.visible.get() => self.close(),
                    TIMER_ANIMATION => self.animation_tick(),
                    TIMER_CARET => {
                        if let Ok(mut page) = self.page.try_borrow_mut() {
                            page.blink();
                        }
                        self.invalidate();
                    }
                    TIMER_TOOLTIP => self.show_tooltip(),
                    _ => {}
                }
                Some(LRESULT(0))
            }
            WM_KEYDOWN => {
                self.on_key(VIRTUAL_KEY(wparam.0 as u16));
                Some(LRESULT(0))
            }
            // Alt+Left steps back, like a browser.
            WM_SYSKEYDOWN if wparam.0 == usize::from(VK_LEFT.0) => {
                self.go_back();
                Some(LRESULT(0))
            }
            // No menus or mnemonics here; don't beep at Alt+letter.
            WM_SYSCHAR => Some(LRESULT(0)),
            WM_CHAR => {
                if let Some(c) = char::from_u32(wparam.0 as u32) {
                    self.with_page(|page, _, out| page.char_input(c, out));
                    self.after_input();
                }
                Some(LRESULT(0))
            }
            WM_MOUSEMOVE => {
                self.on_mouse_move(lparam);
                Some(LRESULT(0))
            }
            WM_MOUSELEAVE => {
                self.on_mouse_leave();
                Some(LRESULT(0))
            }
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
                self.on_button_down(lparam, msg == WM_LBUTTONDBLCLK);
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                self.on_button_up(lparam);
                Some(LRESULT(0))
            }
            WM_MOUSEWHEEL => {
                self.on_wheel(wparam, lparam);
                Some(LRESULT(0))
            }
            // The mouse's back button steps back too.
            WM_XBUTTONUP => {
                const XBUTTON1: u16 = 1;
                if ((wparam.0 >> 16) & 0xFFFF) as u16 == XBUTTON1 {
                    self.go_back();
                }
                Some(LRESULT(1))
            }
            WM_SETCURSOR if u32::from(win::loword(lparam.0 as usize)) == HTCLIENT => {
                self.on_set_cursor();
                Some(LRESULT(1))
            }
            WM_GETOBJECT => self.get_object(hwnd, wparam, lparam),
            WM_SETTINGCHANGE | WM_DWMCOLORIZATIONCOLORCHANGED => {
                let relevant = msg == WM_DWMCOLORIZATIONCOLORCHANGED || theme::is_theme_change(lparam);
                if relevant && self.visible.get() {
                    self.apply_theme();
                }
                None
            }
            WM_DPICHANGED => {
                // X and Y DPI are always the same.
                self.on_dpi_changed(u32::from(win::loword(wparam.0)));
                Some(LRESULT(0))
            }
            // Only the latest request's reply: an earlier one would overwrite a newer state.
            STARTUP_MESSAGE if lparam.0 as usize == self.startup_request.get() => {
                let state = wparam.0.checked_sub(1).map(|s| StartupTaskState(s as i32));
                self.with_page(|page, _, out| page.startup_state(state, out));
                self.invalidate();
                Some(LRESULT(0))
            }
            // Alt+F4 hides like any other dismissal.
            WM_CLOSE if !self.allow_close.get() => {
                self.hide();
                Some(LRESULT(0))
            }
            WM_DESTROY => {
                self.on_destroy(hwnd);
                Some(LRESULT(0))
            }
            WM_NCDESTROY => {
                self.closed.set(true);
                unsafe {
                    let _ = PostMessageW(Some(self.owner), CLOSED_MESSAGE, WPARAM(0), LPARAM(0));
                }
                None
            }
            _ => None,
        }
    }
}

/// Runs `work` on a new thread once `previous` has finished, so work started in order runs in
/// order.
fn spawn_after(previous: Option<JoinHandle<()>>, work: impl FnOnce() + Send + 'static) -> JoinHandle<()> {
    std::thread::spawn(move || {
        if let Some(previous) = previous {
            let _ = previous.join();
        }
        work();
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn spawn_after_previous_running_waits_for_it() {
        let (release, released) = mpsc::channel();
        let (done, finished) = mpsc::channel();
        let first_done = done.clone();
        let first = spawn_after(None, move || {
            let _ = released.recv();
            let _ = first_done.send(1);
        });
        let second = spawn_after(Some(first), move || {
            let _ = done.send(2);
        });

        assert!(finished.recv_timeout(Duration::from_millis(100)).is_err());
        release.send(()).unwrap();
        second.join().unwrap();
        assert_eq!(finished.try_iter().collect::<Vec<_>>(), [1, 2]);
    }
}
