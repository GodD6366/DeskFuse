//! Cross-platform collaboration core.
//!
//! Hosts inject [`deskfuse_platform::ports`] implementations. Product extensions
//! subscribe to [`api::CollaborationEvent`]. This crate must not depend on axum,
//! image, zip, workflow, content-library, or a concrete OS adapter.
#![forbid(unsafe_code)]

pub mod api;
pub mod clipboard_sync;
pub mod content;
mod core;
pub mod error;
pub mod file_transfer;
pub mod input_coordinator;
pub mod session;
pub mod transport;

pub use api::{
    ClipboardStatus, CollaborationCommand, CollaborationEvent, CollaborationSnapshot,
    ConnectionState, DeviceInfo, DeviceState, DisconnectReason, InputStatus, OperationId,
    SessionInfo, TransferState,
};
pub use content::ContentHandle;
pub use core::{CollaborationCore, CoreConfig};
pub use deskfuse_transfer::TransferId;
pub use error::{CollaborationError, Result};
pub use transport::{CollaborationTransport, MemoryTransport, OutboundMessage, RemoteContent};
