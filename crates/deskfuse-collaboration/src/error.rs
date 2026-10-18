use deskfuse_core::DeviceId;
use deskfuse_platform::PlatformError;
use deskfuse_transfer::TransferId;

/// Recoverable collaboration failure. Native clipboard bodies are never included.
#[derive(Debug, thiserror::Error)]
pub enum CollaborationError {
    #[error("{0}")]
    Platform(#[from] PlatformError),
    #[error("input lease has expired")]
    LeaseExpired,
    #[error("no active input lease")]
    NoLease,
    #[error("transfer {0} was not found")]
    TransferNotFound(TransferId),
    #[error("device {0} is not in an active session")]
    SessionNotFound(DeviceId),
    #[error("{0}")]
    Failed(String),
}

impl CollaborationError {
    pub fn failed(message: impl Into<String>) -> Self {
        Self::Failed(message.into())
    }
}

pub type Result<T> = std::result::Result<T, CollaborationError>;
