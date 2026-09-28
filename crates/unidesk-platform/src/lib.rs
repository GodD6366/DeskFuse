//! Platform-neutral native boundaries for uniDesk. No OS or GPUI dependency.
#![forbid(unsafe_code)]
mod session;
pub use session::{DesktopState, SessionBlocker, SessionCapabilities};
mod display;
mod error;
pub use display::MainDisplayGeometry;
mod pasteboard;
mod permissions;
pub use error::{BoundaryErrorCode, PlatformError, PlatformResult};
pub use pasteboard::{
    MemoryPasteboard, Pasteboard, PasteboardItem, PasteboardRepresentation, PasteboardSnapshot,
    REMOTE_ORIGIN_MARKER_TYPE, UNIVERSAL_CLIPBOARD_TYPE,
};
pub use permissions::{PermissionKind, PermissionService, PermissionSnapshot, PermissionStatus};

mod shared_input;
pub use shared_input::*;
mod input_environment;
pub use input_environment::{
    InputBlocker, InputBlockerKind, InputCapability, InputEnvironmentPort, InputEnvironmentSnapshot,
};
pub mod timing;

/// Host Ports injected by the application host. Collaboration depends only on these traits.
pub mod ports;
pub use ports::{
    AuthorizedWorkflowPlan, CapturedFilePaste, ClipboardState, EnginePlatformPort,
    FileDataObjectHandle, FileDataObjectPort, FilePasteCapture, FilePasteReference,
    FileTransferState, FinderDeliveryItem, FinderDeliveryResult, FinderDestinationPort, FinderPort,
    HostPorts, LanInterface, LanWebPort, NativeCancellation, NativeGeneration, NativeRevision,
    PlatformCapabilities, PlatformCapability, PlatformCommand, PlatformEvent,
    UnavailableWorkflowOutputPort, WakeObserverPort, WorkflowArchiveRequest, WorkflowCapabilities,
    WorkflowExecutionPlan, WorkflowExecutionPort, WorkflowExecutionResult, WorkflowInput, WorkflowOutputMode,
    WorkflowOutputPort, WorkflowPayload, WorkflowShortcut, WorkflowShortcutRequest,
    WorkflowWorkspaceRequest,
};
