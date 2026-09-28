//! Platform-neutral fallback adapters. Production composition is injected by the host.
use std::{
    path::Path,
    sync::{Arc, atomic::AtomicBool},
};
use unidesk_platform::ports::{
    EnginePlatformPort, FinderDestinationPort, FinderPort, LanInterface, LanWebPort,
    WakeObserverPort, WorkflowExecutionPort,
};
use unidesk_platform::{
    InputEnvironmentPort, InputEnvironmentSnapshot, Pasteboard, PermissionService,
    PermissionSnapshot, PlatformError, PlatformResult, SharedInputCapture, SharedInputInjector,
};

pub fn memory_platform() -> Arc<dyn EnginePlatformPort> {
    Arc::new(MemoryPlatform::default())
}

#[derive(Default)]
struct MemoryPlatform {
    pasteboard: Arc<unidesk_platform::MemoryPasteboard>,
    finder: Arc<MemoryFinder>,
    lan: Arc<MemoryLan>,
}
impl EnginePlatformPort for MemoryPlatform {
    fn pasteboard(&self) -> Arc<dyn Pasteboard + Sync> {
        self.pasteboard.clone()
    }
    fn finder(&self) -> Arc<dyn FinderPort> {
        self.finder.clone()
    }
    fn lan_web(&self) -> Arc<dyn LanWebPort> {
        self.lan.clone()
    }
    fn permission_service(&self) -> Box<dyn PermissionService> {
        Box::new(MemoryPermission)
    }
    fn display_topology(&self) -> PlatformResult<unidesk_core::DisplayTopology> {
        Err(PlatformError::Unsupported {
            operation: "display topology",
        })
    }
    fn input_environment(&self) -> Box<dyn InputEnvironmentPort> {
        Box::new(MemoryInputEnvironment)
    }
    fn input_capture(&self) -> Box<dyn SharedInputCapture> {
        Box::new(MemoryCapture)
    }
    fn input_injector(&self) -> PlatformResult<Box<dyn SharedInputInjector>> {
        Ok(Box::new(MemoryInjector))
    }
    fn session_is_active(&self) -> bool {
        false
    }
    fn can_post_input(&self) -> bool {
        false
    }
    fn frontmost_pid(&self) -> i32 {
        0
    }
    fn restore_pointer_after_service_exit(&self) -> PlatformResult<()> {
        Ok(())
    }
    fn warp_pointer(&self, _x: f64, _y: f64) -> PlatformResult<()> {
        Ok(())
    }
    fn current_ssid(&self) -> Option<String> {
        None
    }
    fn clipboard_source_application(&self) -> (Option<String>, Option<String>) {
        (None, None)
    }
    fn wake_observer(
        &self,
        _callback: Box<dyn Fn() + Send + Sync>,
    ) -> PlatformResult<Box<dyn WakeObserverPort>> {
        Ok(Box::new(NoopWakeObserver))
    }
    fn workflow_execution(&self) -> Arc<dyn WorkflowExecutionPort> {
        Arc::new(UnsupportedWorkflowExecutionPort)
    }
}

struct MemoryPermission;
impl PermissionService for MemoryPermission {
    fn snapshot(&self) -> PermissionSnapshot {
        PermissionSnapshot::all_unknown()
    }
    fn open_system_settings(&self, _kind: unidesk_platform::PermissionKind) -> PlatformResult<()> {
        Ok(())
    }
}
struct MemoryInputEnvironment;
impl InputEnvironmentPort for MemoryInputEnvironment {
    fn snapshot(&self) -> InputEnvironmentSnapshot {
        InputEnvironmentSnapshot::blocked(
            unidesk_platform::DesktopState::Unavailable,
            unidesk_platform::InputBlockerKind::Unavailable,
            "input platform unavailable",
        )
    }
}
struct MemoryCapture;
impl SharedInputCapture for MemoryCapture {
    fn start(&mut self) -> PlatformResult<()> {
        Ok(())
    }
    fn stop(&mut self) -> PlatformResult<()> {
        Ok(())
    }
    fn set_mode(&self, _: unidesk_platform::CaptureMode) {}
    fn set_hotkeys(&self, _: unidesk_input::sharing::HotkeyBindings) {}
    fn arm_session(&self, _: unidesk_core::InputSessionId, _: std::time::Duration) -> bool {
        false
    }
    fn renew_deadline(&self, _: std::time::Duration) -> bool {
        false
    }
    fn lease_expired(&self) -> bool {
        true
    }
    fn set_controller_hotkeys_enabled(&self, _: bool) {}
    fn enter_remote(&self, _: f64, _: f64) -> PlatformResult<()> {
        Ok(())
    }
    fn exit_remote(&self, _: Option<(f64, f64)>) -> PlatformResult<()> {
        Ok(())
    }
    fn event_notification(&self) -> Arc<tokio::sync::Notify> {
        Arc::new(tokio::sync::Notify::new())
    }
    fn next_event(&mut self) -> PlatformResult<Option<unidesk_platform::CaptureEvent>> {
        Ok(None)
    }
}
struct MemoryInjector;
impl SharedInputInjector for MemoryInjector {
    fn arm_session(&self, _: unidesk_core::InputSessionId, _: std::time::Duration) -> bool {
        false
    }
    fn renew_deadline(&self, _: std::time::Duration) -> bool {
        false
    }
    fn lease_expired(&self) -> bool {
        true
    }
    fn enqueue_batch(
        &self,
        _: unidesk_core::InputSessionId,
        _: &unidesk_core::DisplayDescriptor,
        _: u64,
        _: &[unidesk_core::SharedInputEvent],
    ) -> PlatformResult<()> {
        Ok(())
    }
    fn error_notification(&self) -> Arc<tokio::sync::Notify> {
        Arc::new(tokio::sync::Notify::new())
    }
    fn take_error(&self) -> Option<PlatformError> {
        None
    }
    fn release_all(&self) -> PlatformResult<()> {
        Ok(())
    }
}
#[derive(Default)]
struct MemoryFinder;
impl FinderPort for MemoryFinder {
    fn is_finder_pid(&self, _: i32) -> bool {
        false
    }
    fn capture_destination(
        &self,
        _: i32,
    ) -> PlatformResult<Option<Arc<dyn FinderDestinationPort>>> {
        Ok(None)
    }
}
#[derive(Default)]
struct MemoryLan;
impl LanWebPort for MemoryLan {
    fn interfaces(&self) -> PlatformResult<Vec<LanInterface>> {
        Ok(Vec::new())
    }
    fn ipv4_addresses(&self) -> PlatformResult<Vec<String>> {
        Ok(Vec::new())
    }
    fn image_png(&self, _: &Path, _: u32) -> PlatformResult<Vec<u8>> {
        Err(PlatformError::Unsupported {
            operation: "LAN image preview",
        })
    }
    fn data_image_png(&self, _: &[u8], _: u32) -> PlatformResult<Vec<u8>> {
        Err(PlatformError::Unsupported {
            operation: "LAN image preview",
        })
    }
}
struct NoopWakeObserver;
impl WakeObserverPort for NoopWakeObserver {}

#[derive(Clone, Debug, Default)]
struct UnsupportedWorkflowExecutionPort;
#[async_trait::async_trait]
impl WorkflowExecutionPort for UnsupportedWorkflowExecutionPort {
    async fn execute(
        &self,
        _: &unidesk_platform::AuthorizedWorkflowPlan,
        _: unidesk_platform::WorkflowPayload,
    ) -> PlatformResult<unidesk_platform::WorkflowExecutionResult> {
        Err(PlatformError::Unsupported {
            operation: "workflow execution",
        })
    }
    async fn capabilities(&self) -> PlatformResult<unidesk_platform::PlatformCapabilities> {
        Ok(Default::default())
    }
    fn workflow_capabilities(&self) -> unidesk_platform::WorkflowCapabilities {
        Default::default()
    }
    fn clipboard_revision(&self) -> PlatformResult<unidesk_platform::NativeRevision> {
        Err(PlatformError::Unsupported {
            operation: "workflow clipboard",
        })
    }
    fn modifiers_released(&self) -> bool {
        false
    }
    fn paste_if_current(&self, _: i32, _: unidesk_platform::NativeRevision) -> PlatformResult<()> {
        Err(PlatformError::Unsupported {
            operation: "workflow paste",
        })
    }
    fn create_workspace(&self, _: &Path) -> PlatformResult<()> {
        Err(PlatformError::Unsupported {
            operation: "workflow workspace",
        })
    }
    fn copy_workspace(
        &self,
        _: &unidesk_platform::WorkflowWorkspaceRequest,
        _: &AtomicBool,
    ) -> PlatformResult<u64> {
        Err(PlatformError::Unsupported {
            operation: "workflow workspace copy",
        })
    }
    fn rename_path(&self, _: &Path, _: &Path) -> PlatformResult<()> {
        Err(PlatformError::Unsupported {
            operation: "workflow rename",
        })
    }
    fn move_path(&self, _: &Path, _: &Path) -> PlatformResult<()> {
        Err(PlatformError::Unsupported {
            operation: "workflow move",
        })
    }
    fn remove_path(&self, _: &Path) -> PlatformResult<()> {
        Err(PlatformError::Unsupported {
            operation: "workflow remove",
        })
    }
    fn archive_zip(
        &self,
        _: &unidesk_platform::WorkflowArchiveRequest,
        _: &AtomicBool,
    ) -> PlatformResult<()> {
        Err(PlatformError::Unsupported {
            operation: "workflow zip",
        })
    }
    fn list_shortcuts(&self) -> PlatformResult<Vec<unidesk_platform::WorkflowShortcut>> {
        Err(PlatformError::Unsupported {
            operation: "system shortcuts",
        })
    }
    fn run_shortcut(
        &self,
        _: &unidesk_platform::WorkflowShortcutRequest,
        _: &AtomicBool,
    ) -> PlatformResult<()> {
        Err(PlatformError::Unsupported {
            operation: "system shortcut",
        })
    }
}
