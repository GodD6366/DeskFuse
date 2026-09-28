//! Typed native services injected by an application host.
use super::workflow::WorkflowExecutionPort;
use crate::{
    InputEnvironmentPort, Pasteboard, PermissionService, PlatformResult, SharedInputCapture,
    SharedInputInjector,
};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64},
    },
};

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
}

pub trait WakeObserverPort {}

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
    fn workflow_execution(&self) -> Arc<dyn WorkflowExecutionPort>;
}

/// A cancellation token type shared by host implementations that perform
/// bounded native copies. Kept here to make the contract discoverable.
pub type NativeCancellation = Arc<AtomicBool>;

/// Monotonic counter used by host implementations for native generations.
pub type NativeGeneration = AtomicU64;
