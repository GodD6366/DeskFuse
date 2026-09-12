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
pub use unidesk_platform_macos::MacOSWakeObserver as WakeObserver;
pub use unidesk_platform_macos::sharing::{session_is_active, warp_pointer};
pub use unidesk_platform_macos::wifi_environment::current_ssid;
pub use unidesk_platform_macos::{create_permission_service, display_topology};
pub fn create_capture() -> Box<dyn unidesk_platform::SharedInputCapture> {
    Box::new(unidesk_platform_macos::sharing::MacOSSharingCapture::new())
}
pub fn create_injector() -> PlatformResult<Box<dyn unidesk_platform::SharedInputInjector>> {
    Ok(Box::new(
        unidesk_platform_macos::sharing::MacOSSharingInjector::new()?,
    ))
}
