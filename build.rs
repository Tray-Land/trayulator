//! Compiles the Windows resources into trayulator.exe: the tray icon glyphs, the application
//! manifest (per-monitor DPI awareness) and the version shown in Task Manager and Explorer.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let white_icon = root.join("Assets").join("AppIcon.ico");
    let black_icon = root.join("Assets").join("AppIcon-dark.ico");
    let manifest = root.join("app.manifest");
    for input in [&white_icon, &black_icon, &manifest] {
        println!("cargo:rerun-if-changed={}", input.display());
    }

    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let mut parts = version.split(['.', '-']).map(|p| p.parse::<u16>().unwrap_or(0));
    let (major, minor, patch) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0), parts.next().unwrap_or(0));

    // Icon 1 is the exe's icon and the tray glyph for dark taskbars; icon 2 is for light ones.
    let script = format!(
        r#"1 ICON "{white}"
2 ICON "{black}"
1 24 "{manifest}"

1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "FileDescription", "Trayulator"
      VALUE "FileVersion", "{version}"
      VALUE "OriginalFilename", "trayulator.exe"
      VALUE "ProductName", "Trayulator"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        white = rc_path(&white_icon),
        black = rc_path(&black_icon),
        manifest = rc_path(&manifest),
    );

    let script_path = PathBuf::from(env::var("OUT_DIR").unwrap()).join("trayulator.rc");
    fs::write(&script_path, script).unwrap();
    embed_resource::compile(&script_path, embed_resource::NONE).manifest_required().unwrap();
}

/// A path the resource compiler reads as-is: forward slashes need no escaping.
fn rc_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}
