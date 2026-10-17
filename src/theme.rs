//! The Windows (taskbar and Start) theme, which can differ from the app theme. The tray icon, its
//! menu and the flyout follow this one, like the shell's own flyouts.

use windows::UI::ViewManagement::{UIColorType, UISettings};
use windows::Win32::Foundation::{ERROR_SUCCESS, LPARAM};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::core::{PCSTR, PCWSTR, w};

const PERSONALIZE_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");

/// True when the Windows theme is light; false when dark or unknown.
pub fn is_light() -> bool {
    read_flag(w!("SystemUsesLightTheme"))
}

/// True when "Show accent color on Start and taskbar" is on, which Windows only allows in dark
/// mode. Shell flyouts take an accent tint then.
pub fn shows_accent_color() -> bool {
    !is_light() && read_flag(w!("ColorPrevalence"))
}

/// True for the WM_SETTINGCHANGE broadcast Windows sends when the theme or accent color changes.
pub fn is_theme_change(lparam: LPARAM) -> bool {
    let name = PCWSTR(lparam.0 as *const u16);
    !name.is_null() && unsafe { name.to_string() }.is_ok_and(|n| n == "ImmersiveColorSet")
}

/// A shade of the user's accent color as 0xRRGGBB, or `None` when it can't be read.
pub fn accent(shade: UIColorType) -> Option<u32> {
    let color = UISettings::new().and_then(|settings| settings.GetColorValue(shade)).ok()?;
    Some(u32::from(color.R) << 16 | u32::from(color.G) << 8 | u32::from(color.B))
}

/// Makes classic popup menus (the tray menu) follow the taskbar theme. Windows has no documented
/// switch for this; uxtheme's unnamed SetPreferredAppMode (ordinal 135, Windows 10 1903 and
/// later) and FlushMenuThemes (ordinal 136) are what the shell uses. If they're missing, menus
/// just stay light.
pub fn set_menu_theme(dark: bool) {
    const FORCE_DARK: i32 = 2;
    const FORCE_LIGHT: i32 = 3;
    type SetPreferredAppMode = unsafe extern "system" fn(i32) -> i32;
    type FlushMenuThemes = unsafe extern "system" fn();

    unsafe {
        let Ok(uxtheme) = LoadLibraryExW(w!("uxtheme.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32) else {
            return;
        };
        let ordinal = |n: usize| GetProcAddress(uxtheme, PCSTR(n as *const u8));
        if let (Some(set_mode), Some(flush)) = (ordinal(135), ordinal(136)) {
            let set_mode: SetPreferredAppMode = std::mem::transmute(set_mode);
            let flush: FlushMenuThemes = std::mem::transmute(flush);
            set_mode(if dark { FORCE_DARK } else { FORCE_LIGHT });
            flush();
        }
    }
}

fn read_flag(name: PCWSTR) -> bool {
    let mut value = 0u32;
    let mut size = size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PERSONALIZE_KEY,
            name,
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut value).cast()),
            Some(&mut size),
        )
    };
    status == ERROR_SUCCESS && value != 0
}
