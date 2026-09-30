//! Operating-system updater adapters.
//!
//! `deskfuse-update` contains only signed release policy and transaction state.
//! This crate is the transitional Rust platform boundary for filesystem,
//! process, elevation, replacement, backup, and recovery operations.  Native
//! Swift/C# implementations can replace these modules without changing the
//! public manifest or transaction protocol.
#![forbid(unsafe_code)]

#[cfg(target_os = "macos")]
pub mod installer {
    pub use crate::macos::*;
}
#[cfg(windows)]
pub mod installer {
    pub use crate::windows::*;
}

#[cfg(not(any(target_os = "macos", windows)))]
pub mod installer {
    use deskfuse_update::ReleaseVersion;
    use std::path::{Path, PathBuf};
    use std::process::Child;

    pub fn current_bundle() -> Result<PathBuf, String> {
        Err("DeskFuse automatic updates are unavailable on this platform".into())
    }
    pub fn installed_release(_bundle: &Path) -> Result<ReleaseVersion, String> {
        Err("DeskFuse automatic updates are unavailable on this platform".into())
    }
    pub fn launch_helper() -> Result<Child, String> {
        Err("DeskFuse update helper is unavailable on this platform".into())
    }
}

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

/// Entry point for the packaged helper.  It remains a separate platform
/// binary so the common updater library cannot be used as an installer by
/// accident.
pub fn run_helper() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        return macos::run_helper();
    }
    #[cfg(windows)]
    {
        return windows::run_helper();
    }
    #[allow(unreachable_code)]
    Err("DeskFuse update helper is unavailable on this platform".into())
}
