# Trayulator

A small calculator that lives in the Windows notification area. Click the tray icon, type an equation, press Enter.

<img src="docs/screenshot.png" alt="Trayulator flyout showing (1250*1.08)/12 = 112.5 above a calculator keypad" width="328" />

## Features

- **Type or tap.** The display is a real text box, so you can type whole equations such as `(1250*1.08)/12` or use the keypad. Keys insert at the cursor.
- **Evaluate in place.** Enter, typing `=`, or the **=** key replaces the equation with its result, and the equation moves to the line above, like Windows Calculator.
- **Keep going.** After a result, an operator continues from it and a number starts over. `ans` is the previous result, and **Up** brings back the last equation to edit.
- **Pin it open.** The pin button keeps the flyout on screen when you click elsewhere. Escape or the tray icon still hides it.
- **Copy on evaluate.** An optional setting puts every result on the clipboard.
- **Light on resources.** It does no work while hidden, needs no network access, and follows the taskbar's light or dark theme and accent color.

## What you can type

| Input | Meaning |
|---|---|
| `+ - * /` or `× − ÷` | Arithmetic |
| `^` or `**` | Power: `2^10` = 1024 |
| `%` | Remainder: `10 % 3` = 1 |
| `( )` | Grouping |
| `pi` or `π`, `e` | Constants. `2π` works too |
| `ans` | The previous result |
| `sqrt(x)`, `abs(x)`, `round(x, n)`, `pow(x, y)`, `max(a, b)`, `min(a, b)`, `log(x, base)`, `sin(x)`… | [NCalc built-in functions](https://github.com/ncalc/ncalc), case-insensitive |

Numbers use `.` for decimals and `,` between function arguments. Results are shown to 15 significant digits, so `0.1+0.2` is `0.3`. Whole numbers that overflow switch to floating point instead of wrapping around.

## Keyboard

| Key | Action |
|---|---|
| Enter or `=` | Evaluate |
| Up | Recall the last equation |
| Escape | Leave settings, or hide the flyout |

## Build and run

Requires Windows 10 1809 or later, the [.NET 10 SDK](https://dotnet.microsoft.com/download), and the [winapp CLI](https://github.com/microsoft/winappCli).

```powershell
winapp run            # build, register with package identity, and launch
winapp run --clean    # same, starting from first-run state
dotnet test tests/Trayulator.Tests
```

Right-click the tray icon for **Settings** and **Exit**.

## How it's built

- WinUI 3 and the Windows App SDK, with [WinUIEx](https://github.com/dotMorten/WinUIEx) for the tray icon.
- Expressions are evaluated by [NCalcAsync](https://github.com/ncalc/ncalc) in `Core/CalculatorEngine.cs`. It has no WinRT dependency, so the test project compiles it directly.
- The flyout (`Views/TrayFlyoutWindow`) hides when dismissed, so reopening it is instant. After a minute hidden it closes completely and gives its memory back.
