//! Trayulator: a small calculator that lives in the Windows notification area.

#![windows_subsystem = "windows"]

mod app;
mod calc;
mod clipboard;
mod placement;
mod settings;
mod startup;
mod theme;
mod tray;
mod ui;
mod win;

use std::io::Write;

use windows::Win32::System::SystemInformation::GetLocalTime;

fn main() {
    // A panic inside a window procedure aborts the process; leave a note behind first.
    std::panic::set_hook(Box::new(|info| {
        log_crash(&format!("{info}\n{}", std::backtrace::Backtrace::force_capture()));
    }));

    if let Err(error) = app::run() {
        log_crash(&format!("Startup failed: {error}"));
    }
}

/// Appends to %TEMP%\Trayulator-crash.log. Never fails: there's nowhere left to report to.
fn log_crash(message: &str) {
    let path = std::env::temp_dir().join("Trayulator-crash.log");
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let t = unsafe { GetLocalTime() };
        let _ = writeln!(
            file,
            "[{:04}-{:02}-{:02}T{:02}:{:02}:{:02}] {message}",
            t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
        );
    }
}
