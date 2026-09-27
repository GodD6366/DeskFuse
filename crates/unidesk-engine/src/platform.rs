//! Platform composition. Business workers use this factory rather than select
//! AppKit/Win32 themselves. Tests never read or replace the user's clipboard.
use std::sync::Arc;
use unidesk_platform::PlatformResult;

#[cfg(test)]
pub(crate) use unidesk_platform::MemoryPasteboard as WorkerPasteboard;
#[cfg(all(not(test), target_os = "macos"))]
pub(crate) use unidesk_platform_macos::MacOSPasteboard as WorkerPasteboard;
#[cfg(all(not(test), target_os = "windows"))]
pub(crate) use unidesk_platform_windows::WindowsPasteboard as WorkerPasteboard;

pub(crate) use unidesk_platform_macos::{
    CapturedFilePaste, DeliveryResult, FilePasteReference, FinderDestination,
    capture_finder_destination, deliver_finder_files, is_finder_pid,
};

pub fn host_ports(
    data_dir: &std::path::Path,
) -> PlatformResult<unidesk_platform::ports::HostPorts> {
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
        workflow_output: create_workflow_output(),
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

/// Composition boundary: all concrete workflow adapter choices live here.
pub fn create_workflow_execution_port() -> Arc<dyn unidesk_platform::WorkflowExecutionPort> {
    #[cfg(target_os = "macos")]
    {
        Arc::new(unidesk_platform_macos::MacOSWorkflowExecutionPort)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Arc::new(UnsupportedWorkflowExecutionPort)
    }
}

#[cfg(not(target_os = "macos"))]
struct UnsupportedWorkflowExecutionPort;
#[cfg(not(target_os = "macos"))]
#[async_trait::async_trait]
impl unidesk_platform::WorkflowExecutionPort for UnsupportedWorkflowExecutionPort {
    async fn execute(
        &self,
        _plan: &unidesk_platform::AuthorizedWorkflowPlan,
        _payload: unidesk_platform::WorkflowPayload,
    ) -> unidesk_platform::PlatformResult<unidesk_platform::WorkflowExecutionResult> {
        Err(unidesk_platform::PlatformError::Unsupported {
            operation: "workflow execution",
        })
    }
    async fn capabilities(
        &self,
    ) -> unidesk_platform::PlatformResult<unidesk_platform::PlatformCapabilities> {
        Ok(unidesk_platform::PlatformCapabilities::default())
    }
    fn workflow_capabilities(&self) -> unidesk_platform::WorkflowCapabilities {
        Default::default()
    }
    fn clipboard_revision(
        &self,
    ) -> unidesk_platform::PlatformResult<unidesk_platform::NativeRevision> {
        Err(unidesk_platform::PlatformError::Unsupported {
            operation: "workflow clipboard",
        })
    }
    fn modifiers_released(&self) -> bool {
        false
    }
    fn paste_if_current(
        &self,
        _target_pid: i32,
        _revision: unidesk_platform::NativeRevision,
    ) -> unidesk_platform::PlatformResult<()> {
        Err(unidesk_platform::PlatformError::Unsupported {
            operation: "workflow paste",
        })
    }
    fn create_workspace(&self, _path: &std::path::Path) -> unidesk_platform::PlatformResult<()> {
        Err(unidesk_platform::PlatformError::Unsupported {
            operation: "workflow workspace",
        })
    }
    fn copy_workspace(
        &self,
        _request: &unidesk_platform::WorkflowWorkspaceRequest,
        _cancel: &std::sync::atomic::AtomicBool,
    ) -> unidesk_platform::PlatformResult<u64> {
        Err(unidesk_platform::PlatformError::Unsupported {
            operation: "workflow workspace copy",
        })
    }
    fn rename_path(
        &self,
        _source: &std::path::Path,
        _target: &std::path::Path,
    ) -> unidesk_platform::PlatformResult<()> {
        Err(unidesk_platform::PlatformError::Unsupported {
            operation: "workflow rename",
        })
    }
    fn move_path(
        &self,
        _source: &std::path::Path,
        _target: &std::path::Path,
    ) -> unidesk_platform::PlatformResult<()> {
        Err(unidesk_platform::PlatformError::Unsupported {
            operation: "workflow move",
        })
    }
    fn remove_path(&self, _path: &std::path::Path) -> unidesk_platform::PlatformResult<()> {
        Err(unidesk_platform::PlatformError::Unsupported {
            operation: "workflow remove",
        })
    }
    fn archive_zip(
        &self,
        _request: &unidesk_platform::WorkflowArchiveRequest,
        _cancel: &std::sync::atomic::AtomicBool,
    ) -> unidesk_platform::PlatformResult<()> {
        Err(unidesk_platform::PlatformError::Unsupported {
            operation: "workflow zip",
        })
    }
    fn list_shortcuts(
        &self,
    ) -> unidesk_platform::PlatformResult<Vec<unidesk_platform::WorkflowShortcut>> {
        Err(unidesk_platform::PlatformError::Unsupported {
            operation: "system shortcuts",
        })
    }
    fn run_shortcut(
        &self,
        _request: &unidesk_platform::WorkflowShortcutRequest,
        _cancel: &std::sync::atomic::AtomicBool,
    ) -> unidesk_platform::PlatformResult<()> {
        Err(unidesk_platform::PlatformError::Unsupported {
            operation: "system shortcut",
        })
    }
}

// Composition boundary: all concrete adapter choices live here.
pub use unidesk_platform::{PermissionService, PermissionSnapshot, PermissionStatus};

#[cfg(target_os = "macos")]
pub use unidesk_platform_macos::MacOSWakeObserver as WakeObserver;
#[cfg(target_os = "macos")]
pub use unidesk_platform_macos::sharing::{
    can_post_input, frontmost_pid, restore_pointer_after_service_exit, session_is_active,
    warp_pointer,
};
#[cfg(target_os = "macos")]
pub use unidesk_platform_macos::wifi_environment::current_ssid;
#[cfg(target_os = "macos")]
pub(crate) use unidesk_platform_macos::{clipboard_source_application, image_processor, lan_web};
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
pub fn create_workflow_output() -> std::sync::Arc<dyn unidesk_platform::WorkflowOutputPort> {
    std::sync::Arc::new(unidesk_platform_macos::MacOSWorkflowOutputPort)
}

#[cfg(not(target_os = "macos"))]
pub fn create_workflow_output() -> std::sync::Arc<dyn unidesk_platform::WorkflowOutputPort> {
    std::sync::Arc::new(unidesk_platform::UnavailableWorkflowOutputPort)
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
