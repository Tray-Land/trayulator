@AGENTS.md

## Tray app conventions

This is a tray-only Win32 app in Rust, ported from the `winui-tray-app` skill's C# scaffold. That skill's behavior rules (lifetime, flyout, light-dismiss, idle close, theme, storage) still apply; its C#/XAML code doesn't.

- Launch with `.\run.ps1` (cargo build, then `winapp run` on a staged layout); never register the package by hand. Use `.\run.ps1 -Clean` to test first-run behavior.
- `src/app.rs` owns the tray icon, single instance, and the only exit path (the Exit menu). There is no main window.
- The flyout (`src/ui/flyout.rs`) is hidden on dismiss and closed after a minute hidden. Anything it starts, it must stop in `close` / `WM_DESTROY`.
- Trayulator is offline: no network capability, no background work. Nothing runs while the flyout is hidden.
- User preferences (`IsPinned`, `CopyResultOnEvaluate`) live in `src/settings.rs`. Pinned skips light-dismiss in `FlyoutWindow::hide_if_focus_left`.
- Expression evaluation lives in `src/calc/engine.rs`, which has no Windows dependency; run `cargo test`.
