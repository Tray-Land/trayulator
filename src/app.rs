//! Tray-only application: no window at launch, just a notification-area icon that toggles the
//! flyout. Windows exist only while they're in use; the idle app is an icon and little else.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, LPARAM, LRESULT, RECT, WAIT_OBJECT_0, WPARAM};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, EVENT_MODIFY_STATE, GetCurrentProcess, INFINITE, OpenEventW, ReleaseMutex, SetEvent,
    SetProcessWorkingSetSize,
};
use windows::Win32::UI::WindowsAndMessaging::{
    ASFW_ANY, AllowSetForegroundWindow, AppendMenuW, CreatePopupMenu, DestroyMenu, DestroyWindow, DispatchMessageW, HCURSOR,
    KillTimer, MF_SEPARATOR, MF_STRING, MSG, MWMO_INPUTAVAILABLE, MsgWaitForMultipleObjectsEx, PM_REMOVE, PeekMessageW,
    PostMessageW, PostQuitMessage, QS_ALLINPUT, RegisterWindowMessageW, SetForegroundWindow, SetTimer, TPM_BOTTOMALIGN,
    TPM_LEFTALIGN, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTALIGN, TPM_RIGHTBUTTON, TPM_TOPALIGN, TrackPopupMenuEx,
    TranslateMessage, WINDOW_EX_STYLE, WM_DESTROY, WM_NCCREATE, WM_NULL, WM_QUIT, WM_SETTINGCHANGE, WM_TIMER, WNDCLASS_STYLES,
    WS_OVERLAPPED,
};
use windows::core::{HSTRING, PCWSTR, Result, w};

use crate::calc::history::CalculationHistory;
use crate::placement::{self, Edge};
use crate::tray::{self, TrayEvent, TrayIcon};
use crate::ui::flyout::{self, FlyoutWindow};
use crate::{settings, theme, win};

pub const DISPLAY_NAME: &str = "Trayulator";

const MUTEX_NAME: PCWSTR = w!("Local\\Trayulator_SingleInstance");
const SHOW_EVENT_NAME: PCWSTR = w!("Local\\Trayulator_ShowFlyout");
const CLASS_NAME: PCWSTR = w!("Trayulator.App");

/// Startup leaves loader and first-paint state behind that the idle app never touches again.
const STARTUP_TRIM_DELAY_MS: u32 = 30_000;
const TIMER_TRIM: usize = 1;

const MENU_OPEN: usize = 1;
const MENU_SETTINGS: usize = 2;
const MENU_EXIT: usize = 3;

/// Runs the app until the Exit menu item, or hands off to the instance already running.
pub fn run() -> Result<()> {
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()? };

    let mutex = unsafe { CreateMutexW(None, true, MUTEX_NAME)? };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        // Another instance owns the tray icon: ask it to open its flyout, then quit.
        signal_running_instance();
        unsafe {
            let _ = CloseHandle(mutex);
        }
        return Ok(());
    }

    let show_event = unsafe { CreateEventW(None, false, false, SHOW_EVENT_NAME)? };
    let app = App::new()?;

    // First run: show where the app lives instead of launching into silence.
    if !settings::has_launched_before() {
        settings::set_has_launched_before(true);
        app.show_flyout();
    } else {
        app.schedule_trim(STARTUP_TRIM_DELAY_MS);
    }

    message_loop(&app, show_event);

    unsafe {
        let _ = CloseHandle(show_event);
        let _ = ReleaseMutex(mutex);
        let _ = CloseHandle(mutex);
    }
    Ok(())
}

fn signal_running_instance() {
    unsafe {
        // Let the running instance take the foreground when it shows its flyout.
        let _ = AllowSetForegroundWindow(ASFW_ANY);
        if let Ok(event) = OpenEventW(EVENT_MODIFY_STATE, false, SHOW_EVENT_NAME) {
            let _ = SetEvent(event);
            let _ = CloseHandle(event);
        }
        // Otherwise the other instance is shutting down; nothing to do.
    }
}

/// Dispatches messages, and shows the flyout whenever a second launch signals `show_event`.
fn message_loop(app: &App, show_event: HANDLE) {
    let mut msg = MSG::default();
    loop {
        let wait = unsafe { MsgWaitForMultipleObjectsEx(Some(&[show_event]), INFINITE, QS_ALLINPUT, MWMO_INPUTAVAILABLE) };
        if wait == WAIT_OBJECT_0 {
            app.show_flyout();
            continue;
        }

        while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
            if msg.message == WM_QUIT {
                return;
            }
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

struct App {
    me: Weak<App>,
    hwnd: Cell<HWND>,
    tray: RefCell<Option<TrayIcon>>,
    flyout: RefCell<Option<Rc<FlyoutWindow>>>,
    /// This session's equations. The flyout closes when idle; the history stays.
    history: Rc<RefCell<CalculationHistory>>,
    taskbar_created: u32,
    exiting: Cell<bool>,
}

impl App {
    fn new() -> Result<Rc<Self>> {
        win::register_class::<Self>(CLASS_NAME, WNDCLASS_STYLES(0), HCURSOR::default())?;
        let app = Rc::new_cyclic(|me| Self {
            me: me.clone(),
            hwnd: Cell::default(),
            tray: RefCell::new(None),
            flyout: RefCell::new(None),
            history: Rc::default(),
            taskbar_created: unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) },
            exiting: Cell::new(false),
        });

        // A hidden top-level window rather than a message-only one: those miss the TaskbarCreated
        // and theme-change broadcasts.
        let hwnd = win::create_window(&app, CLASS_NAME, w!("Trayulator"), WS_OVERLAPPED, WINDOW_EX_STYLE(0), RECT::default(), None)?;
        *app.tray.borrow_mut() = Some(TrayIcon::new(hwnd, DISPLAY_NAME));
        Ok(app)
    }

    fn show_flyout(&self) {
        if let Some(flyout) = self.flyout() {
            flyout.show(self.tray_rect());
        }
    }

    /// Opens the flyout on its settings page.
    fn show_settings(&self) {
        self.show_flyout();
        if let Some(flyout) = self.flyout() {
            flyout.show_settings();
        }
    }

    fn toggle_flyout(&self) {
        if let Some(flyout) = self.flyout() {
            flyout.toggle(self.tray_rect());
        }
    }

    /// Sets the tray tooltip under the app name, e.g. "12+3 = 15", or just the name for `""`.
    fn set_tray_status(&self, status: &str) {
        let tooltip = if status.trim().is_empty() { DISPLAY_NAME.to_owned() } else { format!("{DISPLAY_NAME}\n{status}") };
        if let Some(tray) = self.tray.borrow().as_ref() {
            tray.set_tooltip(&tooltip);
        }
    }

    fn tray_rect(&self) -> Option<RECT> {
        self.tray.borrow().as_ref().and_then(TrayIcon::rect)
    }

    /// The flyout, built on demand: it closes itself after staying hidden for a while.
    fn flyout(&self) -> Option<Rc<FlyoutWindow>> {
        if let Some(flyout) = self.flyout.borrow().as_ref().filter(|f| !f.is_closed()) {
            return Some(Rc::clone(flyout));
        }

        let me = self.me.clone();
        let on_status = Box::new(move |status: &str| {
            if let Some(app) = me.upgrade() {
                app.set_tray_status(status);
            }
        });
        let flyout = FlyoutWindow::new(self.hwnd.get(), Rc::clone(&self.history), on_status).ok()?;
        *self.flyout.borrow_mut() = Some(Rc::clone(&flyout));
        Some(flyout)
    }

    fn show_menu(&self, x: i32, y: i32) {
        let hwnd = self.hwnd.get();
        let Ok(menu) = (unsafe { CreatePopupMenu() }) else {
            return;
        };

        let open = HSTRING::from(format!("Open {DISPLAY_NAME}"));
        let command = unsafe {
            let _ = AppendMenuW(menu, MF_STRING, MENU_OPEN, &open);
            let _ = AppendMenuW(menu, MF_STRING, MENU_SETTINGS, w!("Settings"));
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            let _ = AppendMenuW(menu, MF_STRING, MENU_EXIT, w!("Exit"));

            theme::set_menu_theme(!theme::is_light());

            // The menu only closes on an outside click when its owner is the foreground window.
            let _ = SetForegroundWindow(hwnd);
            let command = TrackPopupMenuEx(menu, menu_alignment() | (TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON).0, x, y, hwnd, None);
            let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
            let _ = DestroyMenu(menu);
            command.0 as usize
        };

        match command {
            MENU_OPEN => self.show_flyout(),
            MENU_SETTINGS => self.show_settings(),
            MENU_EXIT => self.exit(),
            _ => {}
        }
    }

    /// The only way out: closes the flyout, removes the icon and ends the message loop.
    fn exit(&self) {
        self.exiting.set(true);
        let flyout = self.flyout.borrow_mut().take();
        if let Some(flyout) = flyout {
            flyout.close();
        }
        self.tray.borrow_mut().take();
        unsafe {
            let _ = DestroyWindow(self.hwnd.get());
        }
    }

    fn schedule_trim(&self, delay_ms: u32) {
        unsafe {
            SetTimer(Some(self.hwnd.get()), TIMER_TRIM, delay_ms, None);
        }
    }
}

impl win::WindowProc for App {
    fn handle(self: &Rc<Self>, hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_NCCREATE => {
                self.hwnd.set(hwnd);
                None
            }
            tray::CALLBACK_MESSAGE => {
                match TrayEvent::from_message(wparam.0, lparam.0) {
                    Some(TrayEvent::Select) => self.toggle_flyout(),
                    Some(TrayEvent::ContextMenu { x, y }) => self.show_menu(x, y),
                    None => {}
                }
                Some(LRESULT(0))
            }
            // Explorer restarted and dropped every tray icon.
            m if m == self.taskbar_created => {
                if let Some(tray) = self.tray.borrow().as_ref() {
                    tray.add();
                }
                Some(LRESULT(0))
            }
            // A white glyph disappears on a light taskbar, which follows the Windows theme.
            WM_SETTINGCHANGE if theme::is_theme_change(lparam) => {
                if let Some(tray) = self.tray.borrow().as_ref() {
                    tray.update_icon();
                }
                None
            }
            flyout::CLOSED_MESSAGE => {
                let closed = self.flyout.borrow().as_ref().is_some_and(|f| f.is_closed());
                if closed {
                    self.flyout.borrow_mut().take();
                }
                if !self.exiting.get() {
                    trim_working_set();
                }
                Some(LRESULT(0))
            }
            WM_TIMER if wparam.0 == TIMER_TRIM => {
                unsafe {
                    let _ = KillTimer(Some(hwnd), TIMER_TRIM);
                }
                trim_working_set();
                Some(LRESULT(0))
            }
            WM_DESTROY => {
                unsafe { PostQuitMessage(0) };
                Some(LRESULT(0))
            }
            _ => None,
        }
    }
}

/// Opens the menu away from the taskbar, wherever it's docked.
fn menu_alignment() -> u32 {
    let flags = match placement::taskbar().map(|(_, edge)| edge) {
        Some(Edge::Top) => TPM_TOPALIGN | TPM_LEFTALIGN,
        Some(Edge::Left) => TPM_LEFTALIGN | TPM_BOTTOMALIGN,
        Some(Edge::Right) => TPM_RIGHTALIGN | TPM_BOTTOMALIGN,
        _ => TPM_BOTTOMALIGN | TPM_LEFTALIGN,
    };
    flags.0
}

/// Gives memory back to Windows once idle, the state a tray app spends nearly all its time in.
/// What the flyout used (graphics drivers, fonts) pages back in cheaply on the next open.
fn trim_working_set() {
    unsafe {
        let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
    }
}
