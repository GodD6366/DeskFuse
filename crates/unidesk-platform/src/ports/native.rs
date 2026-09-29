//! Typed native services injected by an application host.
use super::workflow::WorkflowExecutionPort;
use crate::{
    InputEnvironmentPort, Pasteboard, PermissionService, PlatformResult, SharedInputCapture,
    SharedInputInjector,
};
use std::{
    net::{SocketAddr, UdpSocket},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64},
    },
};
use unidesk_core::{ConnectionContext, TransportSessionId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinderDeliveryItem {
    pub source: PathBuf,
    pub destination: PathBuf,
}

#[derive(Clone, Debug, Default)]
pub struct FinderDeliveryResult {
    pub items: Vec<FinderDeliveryItem>,
    pub cancelled: bool,
    pub error: Option<String>,
}

pub trait FinderDestinationPort: Send + Sync {
    fn identity(&self) -> (u64, u64);
    fn deliver(
        &self,
        paths: &[PathBuf],
        cancel: &AtomicBool,
        progress: &dyn Fn(u64, u64),
    ) -> PlatformResult<FinderDeliveryResult>;
}

pub trait FinderPort: Send + Sync {
    fn is_finder_pid(&self, pid: i32) -> bool;
    fn capture_destination(
        &self,
        pid: i32,
    ) -> PlatformResult<Option<Arc<dyn FinderDestinationPort>>>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LanInterface {
    pub name: String,
    pub address: String,
    pub label: String,
}

pub trait LanWebPort: Send + Sync {
    fn interfaces(&self) -> PlatformResult<Vec<LanInterface>>;
    fn ipv4_addresses(&self) -> PlatformResult<Vec<String>>;
    fn image_png(&self, path: &Path, max_pixel_size: u32) -> PlatformResult<Vec<u8>>;
    fn data_image_png(&self, data: &[u8], max_pixel_size: u32) -> PlatformResult<Vec<u8>>;

    /// Inspect a transport route using host-owned kernel/network evidence.
    /// Implementations must return `Unknown` rather than infer a route from
    /// interface names or address classes when evidence is unavailable.
    fn transport_context(
        &self,
        session_id: TransportSessionId,
        local_addr: SocketAddr,
        remote_addr: SocketAddr,
    ) -> PlatformResult<ConnectionContext> {
        Ok(ConnectionContext {
            session_id,
            local_addr,
            remote_addr,
            interface_name: None,
            interface_kind: unidesk_core::InterfaceKind::Other,
            scope: unidesk_core::LinkScope::Unknown,
        })
    }

    /// Open listeners bound to host-verified physical interfaces.
    fn transport_listener_sockets(
        &self,
        _port: u16,
    ) -> PlatformResult<Vec<(String, UdpSocket)>> {
        Ok(Vec::new())
    }
}

/// Explicit fail-closed LAN adapter for hosts without route/listener support.
/// Construction is required at each transport composition site so missing
/// evidence cannot be mistaken for an implicit physical route.
pub struct FailClosedLanPort;
impl LanWebPort for FailClosedLanPort {
    fn interfaces(&self) -> PlatformResult<Vec<LanInterface>> { Ok(Vec::new()) }
    fn ipv4_addresses(&self) -> PlatformResult<Vec<String>> { Ok(Vec::new()) }
    fn image_png(&self, _path: &Path, _max_pixel_size: u32) -> PlatformResult<Vec<u8>> {
        Err(crate::PlatformError::Unsupported { operation: "LAN image preview" })
    }
    fn data_image_png(&self, _data: &[u8], _max_pixel_size: u32) -> PlatformResult<Vec<u8>> {
        Err(crate::PlatformError::Unsupported { operation: "LAN image preview" })
    }
}

pub trait WakeObserverPort {}

/// Minimal process, environment, executable and filesystem boundary used by
/// engine workers. Cancellation and timeout policy remain in the engine; this
/// port only performs the explicit host operation.
pub trait HostRuntimePort: Send + Sync {
    fn arguments(&self) -> Vec<String>;
    fn process_id(&self) -> u32;
    fn environment(&self, name: &str) -> Option<String>;
    fn hostname(&self) -> Option<String>;
    fn current_executable(&self) -> PlatformResult<PathBuf>;
    fn write_file(&self, path: &Path, contents: &[u8]) -> PlatformResult<()>;
    fn create_dir_all(&self, path: &Path) -> PlatformResult<()>;
    fn remove_dir_all(&self, path: &Path) -> PlatformResult<()>;
}

/// Safe fallback for non-native hosts. It never supplies network evidence.
pub struct SystemHostRuntime;
impl HostRuntimePort for SystemHostRuntime {
    fn arguments(&self) -> Vec<String> { std::env::args().collect() }
    fn process_id(&self) -> u32 { std::process::id() }
    fn environment(&self, name: &str) -> Option<String> { std::env::var(name).ok() }
    fn hostname(&self) -> Option<String> { None }
    fn current_executable(&self) -> PlatformResult<PathBuf> {
        std::env::current_exe().map_err(|error| crate::PlatformError::InvalidRequest {
            message: format!("current executable: {error}"),
        })
    }
    fn write_file(&self, path: &Path, contents: &[u8]) -> PlatformResult<()> {
        std::fs::write(path, contents).map_err(|error| crate::PlatformError::InvalidRequest {
            message: format!("write file: {error}"),
        })
    }
    fn create_dir_all(&self, path: &Path) -> PlatformResult<()> {
        std::fs::create_dir_all(path).map_err(|error| crate::PlatformError::InvalidRequest {
            message: format!("create directory: {error}"),
        })
    }
    fn remove_dir_all(&self, path: &Path) -> PlatformResult<()> {
        std::fs::remove_dir_all(path).map_err(|error| crate::PlatformError::InvalidRequest {
            message: format!("remove directory: {error}"),
        })
    }
}

/// All concrete native effects used by the legacy application worker.
/// Collaboration and workflow policy stay independent of this port.
pub trait EnginePlatformPort: Send + Sync {
    fn pasteboard(&self) -> Arc<dyn Pasteboard + Sync>;
    fn finder(&self) -> Arc<dyn FinderPort>;
    fn lan_web(&self) -> Arc<dyn LanWebPort>;
    fn permission_service(&self) -> Box<dyn PermissionService>;
    fn display_topology(&self) -> PlatformResult<unidesk_core::DisplayTopology>;
    fn input_environment(&self) -> Box<dyn InputEnvironmentPort>;
    fn input_capture(&self) -> Box<dyn SharedInputCapture>;
    fn input_injector(&self) -> PlatformResult<Box<dyn SharedInputInjector>>;
    fn session_is_active(&self) -> bool;
    fn can_post_input(&self) -> bool;
    fn frontmost_pid(&self) -> i32;
    fn restore_pointer_after_service_exit(&self) -> PlatformResult<()>;
    fn warp_pointer(&self, x: f64, y: f64) -> PlatformResult<()>;
    fn current_ssid(&self) -> Option<String>;
    fn clipboard_source_application(&self) -> (Option<String>, Option<String>);
    fn wake_observer(
        &self,
        callback: Box<dyn Fn() + Send + Sync>,
    ) -> PlatformResult<Box<dyn WakeObserverPort>>;
    fn runtime(&self) -> Arc<dyn HostRuntimePort>;
    fn workflow_execution(&self) -> Arc<dyn WorkflowExecutionPort>;
}

/// A cancellation token type shared by host implementations that perform
/// bounded native copies. Kept here to make the contract discoverable.
pub type NativeCancellation = Arc<AtomicBool>;

/// Monotonic counter used by host implementations for native generations.
pub type NativeGeneration = AtomicU64;
