//! Platform composition. Business workers use this factory rather than select
//! AppKit/Win32 themselves. Tests never read or replace the user's clipboard.
use unidesk_platform::PlatformResult;

#[cfg(test)]
pub(crate) use unidesk_platform::MemoryPasteboard as WorkerPasteboard;
#[cfg(all(not(test), target_os = "macos"))]
pub(crate) use unidesk_platform_macos::MacOSPasteboard as WorkerPasteboard;
#[cfg(all(not(test), target_os = "windows"))]
pub(crate) use unidesk_platform_windows::WindowsPasteboard as WorkerPasteboard;

pub fn host_ports(
    data_dir: &std::path::Path,
) -> PlatformResult<unidesk_platform::ports::HostPorts> {
    use std::sync::Arc;
    use unidesk_platform::ports::{
        HostPorts, MemoryCredentialStore, MemoryFileAuthorization, MemoryInputExecution,
        PasteboardPort, PermissionServicePort,
    };
    let clipboard = Arc::new(PasteboardPort::new(create_worker_pasteboard()?));
    Ok(HostPorts {
        clipboard_capture: clipboard.clone(),
        clipboard_apply: clipboard,
        file_authorization: Arc::new(MemoryFileAuthorization::new(data_dir.join("inbox"))),
        input_execution: Arc::new(MemoryInputExecution::new()),
        credential_store: Arc::new(MemoryCredentialStore::new(data_dir.join("identity.key"))),
        permission_observer: Arc::new(PermissionServicePort::new(create_permission_service())),
    })
}

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

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn create_permission_service() -> Box<dyn PermissionService> {
    struct Unknown;
    impl PermissionService for Unknown {
        fn snapshot(&self) -> PermissionSnapshot {
            PermissionSnapshot::all_unknown()
        }
        fn open_system_settings(
            &self,
            _kind: unidesk_platform::PermissionKind,
        ) -> PlatformResult<()> {
            Ok(())
        }
    }
    Box::new(Unknown)
}

#[cfg(target_os = "macos")]
pub fn create_input_environment() -> Box<dyn unidesk_platform::InputEnvironmentPort> {
    Box::new(unidesk_platform_macos::sharing::MacOSInputEnvironment)
}
#[cfg(target_os = "windows")]
pub fn create_input_environment() -> Box<dyn unidesk_platform::InputEnvironmentPort> {
    Box::new(unidesk_platform_windows::WindowsInputEnvironment)
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn create_input_environment() -> Box<dyn unidesk_platform::InputEnvironmentPort> {
    struct Unknown;
    impl unidesk_platform::InputEnvironmentPort for Unknown {
        fn snapshot(&self) -> unidesk_platform::InputEnvironmentSnapshot {
            unidesk_platform::InputEnvironmentSnapshot::blocked(
                unidesk_platform::DesktopState::Unavailable,
                unidesk_platform::InputBlockerKind::Unavailable,
                "input platform unavailable",
            )
        }
    }
    Box::new(Unknown)
}

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
