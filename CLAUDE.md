@AGENTS.md

## Tray app conventions

This is a tray-only WinUI 3 app built from the `winui-tray-app` skill scaffold on top of `winapp new`. Load that skill before changing app lifetime, the flyout, the tray icon, polling, settings, or secrets.

- Launch with `winapp run` (or `dotnet run`); never register the package by hand. Use `winapp run --clean` to test first-run behavior.
- `App.xaml.cs` owns the tray icon, single instance, and the only exit path (the Exit menu). There is no main window.
- The flyout (`Views/TrayFlyoutWindow`) is hidden on dismiss and closed after a minute hidden. Anything it starts, it must stop in `FlyoutPage.OnHidden` / `Dispose`.
- Trayulator is offline: no network capability, no background work. Nothing runs while the flyout is hidden.
- User preferences (`IsPinned`, `CopyResultOnEvaluate`) live in `Services/SettingsService`. Pinned skips light-dismiss in `TrayFlyoutWindow.HideIfFocusLeftWindow`.
- Expression evaluation (NCalcAsync) lives in `Core/CalculatorEngine.cs`, which has no WinRT dependency. `tests/Trayulator.Tests` links `Core/**` directly; run `dotnet test tests/Trayulator.Tests`.
