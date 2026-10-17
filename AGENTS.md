# Agent instructions: Trayulator (Rust + windows-rs)

Trayulator is a tray-only Windows calculator written in Rust with [windows-rs](https://github.com/microsoft/windows-rs). The UI is one Win32 popup drawn with Direct2D and DirectWrite over the shell's acrylic backdrop (DWM system backdrop), with UI Automation providers so it's accessible. It's packaged as MSIX through the [winapp CLI](https://github.com/microsoft/WinAppCli).

`Cargo.toml` is the source of truth for the windows-rs version and the enabled `Win32_*` / WinRT features. Add a feature there when an API is missing; don't add other Windows binding crates.

## Layout

| Path | What |
|---|---|
| `src/main.rs` | Entry point, crash log (`%TEMP%\Trayulator-crash.log`) |
| `src/app.rs` | Single instance, hidden owner window, tray icon events, context menu, exit |
| `src/tray.rs`, `placement.rs`, `theme.rs` | Notification icon, flyout positioning, taskbar theme |
| `src/settings.rs`, `startup.rs` | Package LocalSettings, StartupTask (MSIX only) |
| `src/win.rs`, `clipboard.rs` | Win32 helpers (window classes and procedures, DPI, coordinates), plain-text clipboard |
| `src/calc/` | Expression engine and history: no Windows dependency |
| `src/ui/mod.rs` | Types the UI shares: `Rect`, and the element list's `Element`, `Id` and `Role` |
| `src/ui/flyout.rs` | The popup window: show/hide motion, light-dismiss, idle close, input routing |
| `src/ui/page.rs` | Calculator and settings content: layout, drawing, input, element list |
| `src/ui/entry.rs`, `anim.rs` | Expression entry editing (text, caret, selection), tweens and easing: no Windows dependency |
| `src/ui/a11y.rs`, `tooltip.rs` | UI Automation providers built from the page's element list, tooltip popup for help text |
| `src/ui/graphics.rs`, `widgets.rs`, `palette.rs` | Direct2D/DirectWrite wrapper, WinUI-style controls, Fluent colors |
| `build.rs` | Embeds icons, `app.manifest` and version info |

## Commands

```powershell
cargo build                     # debug build
cargo test                      # unit tests (engine, history, entry editing, tweens, page layout/input/toggles, tray messages, taskbar edge)
cargo clippy --all-targets      # keep this warning-free
.\run.ps1                       # build, stage, register with package identity, launch (winapp run)
.\run.ps1 -Clean                # same, from first-run state
.\run.ps1 -Release              # release profile
```

`cargo run` launches unpackaged: settings don't persist and the startup task is unavailable. Use `.\run.ps1` to verify anything that depends on package identity.

## Rules

- **Re-entrancy.** Win32 calls like `ShowWindow`, `SetWindowPos` and `DestroyWindow` deliver messages before they return. Never hold a `RefCell` borrow across them; message handlers use `try_borrow`. Page logic returns `Command`s that the window runs after releasing its borrows.
- **One element list.** `Page::elements` drives hit testing, Tab order, tooltips and UI Automation. Every interactive element needs a name, and icon-only ones need help text.
- **Tray app lifetime.** No work while hidden. The flyout hides on dismiss and is destroyed after a minute hidden; whatever it starts (timers, worker threads posting back) must end with it.
- **Follow the taskbar theme** (`theme::is_light`), not the app theme.
- **Keep pure logic testable.** Parsing, formatting and editing logic go in modules without Windows calls, with `#[cfg(test)]` tests named `function_scenario_expected`.
- **Finding APIs.** Check exact signatures in the windows-rs source under `~/.cargo/registry/src/*/windows-0.62*/src/Windows/` or at https://microsoft.github.io/windows-docs-rs/. `winapp find-api` searches WinRT types.
- **Verify UI changes in the running app**: `.\run.ps1`, then `winapp ui inspect -a trayulator` and `winapp ui screenshot -a trayulator --capture-screen`. A second launch shows the flyout of the running instance.
