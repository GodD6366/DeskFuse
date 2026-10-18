//! Command, event and snapshot surface used by hosts and product extensions.
use crate::content::ContentHandle;
use deskfuse_core::DeviceId;
use deskfuse_transfer::TransferId;
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, time::Duration};
use uuid::Uuid;

/// Stable identifier for one clipboard apply/publish attempt.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct OperationId(Uuid);

impl OperationId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    #[must_use]
    pub const fn from_uuid(id: Uuid) -> Self {
        Self(id)
    }

    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for OperationId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for OperationId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

/// User-originated operations. Hosts enqueue these; collaboration applies them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CollaborationCommand {
    StartPairing { timeout: Duration },
    AcceptDevice { device_id: DeviceId },
    RejectDevice { device_id: DeviceId },
    RemoveTrust { device_id: DeviceId },
    PublishClipboard { content: ContentHandle },
    CancelClipboard,
    SendFiles { files: Vec<PathBuf>, to: DeviceId },
    CancelTransfer { transfer_id: TransferId },
    RequestInputControl { device_id: DeviceId },
    ReleaseInputControl,
}

/// Why an authenticated session ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DisconnectReason {
    User,
    Peer,
    Transport(String),
    LeaseExpired,
}

/// Notifications for UI and product extensions. Bodies stay behind [`ContentHandle`].
#[derive(Clone, Debug, PartialEq)]
pub enum CollaborationEvent {
    DeviceDiscovered {
        device: DeviceInfo,
    },
    DeviceLost {
        device_id: DeviceId,
    },
    Connected {
        device_id: DeviceId,
        session: SessionInfo,
    },
    Disconnected {
        device_id: DeviceId,
        reason: DisconnectReason,
    },
    ClipboardReceived {
        content: ContentHandle,
        from: DeviceId,
    },
    ClipboardApplied {
        operation_id: OperationId,
    },
    ClipboardFailed {
        operation_id: OperationId,
        error: String,
    },
    ClipboardSuperseded {
        operation_id: OperationId,
    },
    TransferStarted {
        transfer_id: TransferId,
    },
    TransferProgress {
        transfer_id: TransferId,
        progress: f64,
    },
    TransferCompleted {
        transfer_id: TransferId,
    },
    TransferFailed {
        transfer_id: TransferId,
        error: String,
    },
    InputControlGranted {
        device_id: DeviceId,
    },
    InputControlRevoked {
        reason: String,
    },
}

/// Full rebuild surface. Hosts render this on subscribe; they do not cache a second copy.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CollaborationSnapshot {
    pub devices: Vec<DeviceState>,
    pub active_connections: Vec<ConnectionState>,
    pub clipboard_status: ClipboardStatus,
    pub pending_transfers: Vec<TransferState>,
    pub input_status: InputStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: DeviceId,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub device_id: DeviceId,
    pub generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceState {
    pub id: DeviceId,
    pub name: String,
    pub trusted: bool,
    pub connected: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectionState {
    pub device_id: DeviceId,
    pub generation: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClipboardStatus {
    pub local_generation: u64,
    pub last_operation: Option<OperationId>,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TransferState {
    pub transfer_id: TransferId,
    pub peer: DeviceId,
    pub name: String,
    pub progress: f64,
    pub inbound: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InputStatus {
    pub controller: Option<DeviceId>,
    pub lease_active: bool,
}
