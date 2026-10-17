//! User preferences in the package's `ApplicationData.LocalSettings`. Every read falls back to a
//! default, so a missing or corrupt value (or running unpackaged) never fails. Not for secrets.

use windows::Foundation::{IReference, PropertyValue};
use windows::Storage::ApplicationData;
use windows::core::{HSTRING, Interface, Result};

/// False until the first launch has shown the flyout.
pub fn has_launched_before() -> bool {
    get("HasLaunchedBefore")
}

pub fn set_has_launched_before(value: bool) {
    set("HasLaunchedBefore", value);
}

/// When on, the flyout ignores light-dismiss and stays open until hidden explicitly.
pub fn is_pinned() -> bool {
    get("IsPinned")
}

pub fn set_is_pinned(value: bool) {
    set("IsPinned", value);
}

/// When on, every successful evaluation also puts the result on the clipboard.
pub fn copy_result_on_evaluate() -> bool {
    get("CopyResultOnEvaluate")
}

pub fn set_copy_result_on_evaluate(value: bool) {
    set("CopyResultOnEvaluate", value);
}

fn get(key: &str) -> bool {
    let read = || -> Result<bool> {
        let values = ApplicationData::Current()?.LocalSettings()?.Values()?;
        values.Lookup(&HSTRING::from(key))?.cast::<IReference<bool>>()?.Value()
    };
    read().unwrap_or(false)
}

fn set(key: &str, value: bool) {
    let write = || -> Result<()> {
        let values = ApplicationData::Current()?.LocalSettings()?.Values()?;
        values.Insert(&HSTRING::from(key), &PropertyValue::CreateBoolean(value)?)?;
        Ok(())
    };
    // Settings are best-effort; ignore storage failures.
    let _ = write();
}
