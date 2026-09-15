//! Platform-neutral native boundaries for uniDesk. No OS or GPUI dependency.
#![forbid(unsafe_code)]
mod session;
pub use session::{DesktopState, SessionBlocker, SessionCapabilities};
mod display;
mod error;
pub use display::MainDisplayGeometry;
mod pasteboard;
mod permissions;
pub use error::{PlatformError, PlatformResult};
pub use pasteboard::{
    MemoryPasteboard, Pasteboard, PasteboardItem, PasteboardRepresentation, PasteboardSnapshot,
    REMOTE_ORIGIN_MARKER_TYPE,
};
pub use permissions::{PermissionKind, PermissionService, PermissionSnapshot, PermissionStatus};

mod shared_input;
pub use shared_input::*;
pub mod timing;
