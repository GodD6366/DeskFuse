use std::{
    net::{SocketAddr, UdpSocket},
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};
use deskfuse_platform::ports::{
    EnginePlatformPort, FinderDeliveryItem, FinderDeliveryResult, FinderDestinationPort,
    FinderPort, HostPorts, HostRuntimePort, LanInterface, LanWebPort, WakeObserverPort,
    WorkflowExecutionPort,
};
use deskfuse_platform::{
    InputEnvironmentPort, Pasteboard, PermissionService, PlatformResult, SharedInputCapture,
    SharedInputInjector,
};
use deskfuse_platform_macos::{MacOSPasteboard, MacOSWorkflowExecutionPort};

struct FinderDestination(deskfuse_platform_macos::FinderDestination);
impl FinderDestinationPort for FinderDestination {
    fn identity(&self) -> (u64, u64) {
        self.0.identity()
    }
    fn deliver(
        &self,
        paths: &[PathBuf],
        cancel: &AtomicBool,
        progress: &dyn Fn(u64, u64),
    ) -> PlatformResult<FinderDeliveryResult> {
        deskfuse_platform_macos::deliver_finder_files(&self.0, paths, cancel, |saved, total| {
            progress(saved, total)
        })
        .map(|r| FinderDeliveryResult {
            items: r
                .items
                .into_iter()
                .map(|i| FinderDeliveryItem {
                    source: i.source,
                    destination: i.destination,
                })
                .collect(),
            cancelled: r.cancelled,
            error: r.error,
        })
        .map_err(|e| deskfuse_platform::PlatformError::InvalidRequest {
            message: format!("Finder delivery: {e}"),
        })
    }
}
struct Finder;
impl FinderPort for Finder {
    fn is_finder_pid(&self, pid: i32) -> bool {
        deskfuse_platform_macos::is_finder_pid(pid)
    }
    fn capture_destination(
        &self,
        pid: i32,
    ) -> PlatformResult<Option<Arc<dyn FinderDestinationPort>>> {
        deskfuse_platform_macos::capture_finder_destination(pid)
            .map(|v| v.map(|d| Arc::new(FinderDestination(d)) as Arc<dyn FinderDestinationPort>))
            .map_err(|e| deskfuse_platform::PlatformError::InvalidRequest {
                message: format!("Finder destination: {e}"),
            })
    }
}
struct Lan;
impl LanWebPort for Lan {
    fn interfaces(&self) -> PlatformResult<Vec<LanInterface>> {
        deskfuse_platform_macos::lan_web::interfaces()
            .map(|v| {
                v.into_iter()
                    .map(|i| LanInterface {
                        name: i.name,
                        address: i.address,
                        label: i.label,
                    })
                    .collect()
            })
            .map_err(|e| deskfuse_platform::PlatformError::InvalidRequest {
                message: format!("LAN interfaces: {e}"),
            })
    }
    fn ipv4_addresses(&self) -> PlatformResult<Vec<String>> {
        deskfuse_platform_macos::lan_web::all_ipv4_interfaces()
            .map(|v| v.into_iter().map(|i| i.address).collect())
            .map_err(|e| deskfuse_platform::PlatformError::InvalidRequest {
                message: format!("LAN addresses: {e}"),
            })
    }
    fn image_png(&self, path: &Path, size: u32) -> PlatformResult<Vec<u8>> {
        deskfuse_platform_macos::lan_web::image_png(path, size).map_err(|e| {
            deskfuse_platform::PlatformError::InvalidRequest {
                message: format!("LAN image: {e}"),
            }
        })
    }
    fn data_image_png(&self, data: &[u8], size: u32) -> PlatformResult<Vec<u8>> {
        deskfuse_platform_macos::lan_web::data_image_png(data, size).map_err(|e| {
            deskfuse_platform::PlatformError::InvalidRequest {
                message: format!("LAN image: {e}"),
            }
        })
    }
    fn transport_context(
        &self,
        session_id: deskfuse_core::TransportSessionId,
        local_addr: SocketAddr,
        remote_addr: SocketAddr,
    ) -> PlatformResult<deskfuse_core::ConnectionContext> {
        deskfuse_platform_macos::lan_web::transport_context(session_id, local_addr, remote_addr)
            .map_err(|message| deskfuse_platform::PlatformError::InvalidRequest { message })
    }
    fn transport_listener_sockets(
        &self,
        port: u16,
    ) -> PlatformResult<Vec<(String, UdpSocket)>> {
        deskfuse_platform_macos::lan_web::transport_listener_sockets(port)
            .map_err(|message| deskfuse_platform::PlatformError::InvalidRequest { message })
    }
}
struct Wake(deskfuse_platform_macos::MacOSWakeObserver);
impl WakeObserverPort for Wake {}
struct MacRuntime;
impl HostRuntimePort for MacRuntime {
    fn arguments(&self) -> Vec<String> { std::env::args().collect() }
    fn process_id(&self) -> u32 { std::process::id() }
    fn environment(&self, name: &str) -> Option<String> { std::env::var(name).ok() }
    fn hostname(&self) -> Option<String> {
        std::process::Command::new("/bin/hostname").output().ok().and_then(|output| {
            output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        })
    }
    fn current_executable(&self) -> PlatformResult<PathBuf> {
        std::env::current_exe().map_err(|error| deskfuse_platform::PlatformError::InvalidRequest {
            message: format!("current executable: {error}"),
        })
    }
    fn write_file(&self, path: &Path, contents: &[u8]) -> PlatformResult<()> {
        std::fs::write(path, contents).map_err(|error| deskfuse_platform::PlatformError::InvalidRequest { message: error.to_string() })
    }
    fn create_dir_all(&self, path: &Path) -> PlatformResult<()> {
        std::fs::create_dir_all(path).map_err(|error| deskfuse_platform::PlatformError::InvalidRequest { message: error.to_string() })
    }
    fn remove_dir_all(&self, path: &Path) -> PlatformResult<()> {
        std::fs::remove_dir_all(path).map_err(|error| deskfuse_platform::PlatformError::InvalidRequest { message: error.to_string() })
    }
}
pub struct MacPlatform;
impl EnginePlatformPort for MacPlatform {
    fn pasteboard(&self) -> Arc<dyn Pasteboard + Sync> {
        Arc::new(MacOSPasteboard::new())
    }
    fn finder(&self) -> Arc<dyn FinderPort> {
        Arc::new(Finder)
    }
    fn lan_web(&self) -> Arc<dyn LanWebPort> {
        Arc::new(Lan)
    }
    fn permission_service(&self) -> Box<dyn PermissionService> {
        deskfuse_platform_macos::create_permission_service()
    }
    fn display_topology(&self) -> PlatformResult<deskfuse_core::DisplayTopology> {
        deskfuse_platform_macos::display_topology()
    }
    fn input_environment(&self) -> Box<dyn InputEnvironmentPort> {
        Box::new(deskfuse_platform_macos::sharing::MacOSInputEnvironment)
    }
    fn input_capture(&self) -> Box<dyn SharedInputCapture> {
        Box::new(deskfuse_platform_macos::sharing::MacOSSharingCapture::new())
    }
    fn input_injector(&self) -> PlatformResult<Box<dyn SharedInputInjector>> {
        Ok(Box::new(
            deskfuse_platform_macos::sharing::MacOSSharingInjector::new()?,
        ))
    }
    fn session_is_active(&self) -> bool {
        deskfuse_platform_macos::sharing::session_is_active()
    }
    fn can_post_input(&self) -> bool {
        deskfuse_platform_macos::sharing::can_post_input()
    }
    fn frontmost_pid(&self) -> i32 {
        deskfuse_platform_macos::sharing::frontmost_pid()
    }
    fn restore_pointer_after_service_exit(&self) -> PlatformResult<()> {
        if deskfuse_platform_macos::sharing::restore_pointer_after_service_exit() {
            Ok(())
        } else {
            Err(deskfuse_platform::PlatformError::InputCaptureUnavailable)
        }
    }
    fn warp_pointer(&self, x: f64, y: f64) -> PlatformResult<()> {
        deskfuse_platform_macos::sharing::warp_pointer(x, y)
    }
    fn current_ssid(&self) -> Option<String> {
        deskfuse_platform_macos::wifi_environment::current_ssid()
    }
    fn clipboard_source_application(&self) -> (Option<String>, Option<String>) {
        deskfuse_platform_macos::clipboard_source_application()
    }
    fn wake_observer(
        &self,
        callback: Box<dyn Fn() + Send + Sync>,
    ) -> PlatformResult<Box<dyn WakeObserverPort>> {
        Ok(Box::new(Wake(
            deskfuse_platform_macos::MacOSWakeObserver::new(callback),
        )))
    }
    fn runtime(&self) -> Arc<dyn HostRuntimePort> {
        Arc::new(MacRuntime)
    }
    fn workflow_execution(&self) -> Arc<dyn WorkflowExecutionPort> {
        Arc::new(MacOSWorkflowExecutionPort)
    }
}
pub fn platform() -> Arc<dyn EnginePlatformPort> {
    Arc::new(MacPlatform)
}
pub fn host_ports(data_dir: &Path) -> PlatformResult<HostPorts> {
    let platform = platform();
    let clipboard = Arc::new(deskfuse_platform::ports::PasteboardPort::new(
        MacOSPasteboard::new(),
    ));
    Ok(HostPorts {
        clipboard_capture: clipboard.clone(),
        clipboard_apply: clipboard,
        file_authorization: Arc::new(deskfuse_platform::ports::MemoryFileAuthorization::new(
            data_dir.join("inbox"),
        )),
        input_execution: Arc::new(deskfuse_platform::ports::MemoryInputExecution::new()),
        credential_store: Arc::new(deskfuse_platform::ports::MemoryCredentialStore::new(
            data_dir.join("identity.key"),
        )),
        permission_observer: Arc::new(deskfuse_platform::ports::PermissionServicePort::new(
            platform.permission_service(),
        )),
        // The output port is a real macOS adapter; keep it typed even though
        // the legacy workflow worker still receives the execution port above.
        workflow_output: Arc::new(deskfuse_platform_macos::MacOSWorkflowOutputPort),
    })
}
