//! Platform composition. Business workers use this factory rather than select
//! AppKit/Win32 themselves. Tests never read or replace the user's clipboard.
use unidesk_platform::PlatformResult;

#[cfg(test)]
pub(crate) use unidesk_platform::MemoryPasteboard as WorkerPasteboard;
#[cfg(all(not(test), target_os = "macos"))]
pub(crate) use unidesk_platform_macos::MacOSPasteboard as WorkerPasteboard;
#[cfg(all(not(test), target_os = "windows"))]
pub(crate) use unidesk_platform_windows::WindowsPasteboard as WorkerPasteboard;

pub(crate) fn create_worker_pasteboard() -> PlatformResult<WorkerPasteboard> {
    #[cfg(any(test, target_os = "macos"))]
    {
        Ok(WorkerPasteboard::new())
    }
    #[cfg(all(not(test), target_os = "windows"))]
    {
        WorkerPasteboard::new()
    }
}

// Composition boundary: all concrete adapter choices live here.
pub use unidesk_platform::{PermissionService, PermissionSnapshot, PermissionStatus};

#[cfg(target_os = "macos")]
pub use unidesk_platform_macos::MacOSWakeObserver as WakeObserver;
#[cfg(target_os = "macos")]
pub use unidesk_platform_macos::sharing::{session_is_active, warp_pointer};
#[cfg(target_os = "macos")]
pub use unidesk_platform_macos::wifi_environment::current_ssid;
#[cfg(target_os = "macos")]
pub use unidesk_platform_macos::{create_permission_service, display_topology};

#[cfg(target_os = "windows")]
pub use unidesk_platform_windows::{
    create_permission_service, display_topology, session_is_active, warp_pointer,
};

#[cfg(target_os = "macos")]
pub fn create_capture() -> Box<dyn unidesk_platform::SharedInputCapture> {
    Box::new(unidesk_platform_macos::sharing::MacOSSharingCapture::new())
}
#[cfg(target_os = "windows")]
pub fn create_capture() -> Box<dyn unidesk_platform::SharedInputCapture> {
    Box::new(unidesk_platform_windows::WindowsSharingCapture::new())
}

#[cfg(target_os = "macos")]
pub fn create_injector() -> PlatformResult<Box<dyn unidesk_platform::SharedInputInjector>> {
    Ok(Box::new(
        unidesk_platform_macos::sharing::MacOSSharingInjector::new()?,
    ))
}
#[cfg(target_os = "windows")]
pub fn create_injector() -> PlatformResult<Box<dyn unidesk_platform::SharedInputInjector>> {
    Ok(Box::new(
        unidesk_platform_windows::WindowsSharingInjector::new()?,
    ))
}

#[cfg(target_os = "windows")]
pub fn current_ssid() -> Option<String> {
    None
}

#[cfg(target_os = "windows")]
pub struct WakeObserver;
#[cfg(target_os = "windows")]
impl WakeObserver {
    pub fn new<F>(_callback: F) -> Self
    where
        F: Fn() + Send + Sync + 'static,
    {
        Self
    }
}
