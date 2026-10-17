//! The MSIX "start with Windows" task declared in Package.appxmanifest, plus the package version.
//! The task calls block, so callers run them off the UI thread.

use windows::ApplicationModel::{Package, StartupTask, StartupTaskState};
use windows::core::{HSTRING, Result};

const TASK_ID: &str = "TrayulatorStartup";

/// The current state, or `None` when the app runs unpackaged.
pub fn state() -> Option<StartupTaskState> {
    task().and_then(|task| task.State()).ok()
}

/// Turns the task on or off and returns the state Windows settled on, which can differ: the user
/// or a policy has the final say.
pub fn set_enabled(enabled: bool) -> Option<StartupTaskState> {
    let apply = || -> Result<StartupTaskState> {
        let task = task()?;
        if enabled {
            return task.RequestEnableAsync()?.join();
        }
        task.Disable()?;
        task.State()
    };
    apply().ok()
}

/// "1.0.0" from the package identity, or `None` when unpackaged.
pub fn package_version() -> Option<String> {
    let version = Package::Current().and_then(|p| p.Id()).and_then(|id| id.Version()).ok()?;
    Some(format!("{}.{}.{}", version.Major, version.Minor, version.Build))
}

fn task() -> Result<StartupTask> {
    StartupTask::GetAsync(&HSTRING::from(TASK_ID))?.join()
}
