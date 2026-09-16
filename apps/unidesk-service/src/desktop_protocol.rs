//! Desktop session-bound IPC. See `docs/WINDOWS_DESKTOP_IPC.md`.
use serde::{Deserialize, Serialize};
pub const VERSION: u32 = 2;
pub const MAX_FRAME: usize = 24 * 1024 * 1024;
pub const MAX_CONTENT: usize = 16 * 1024 * 1024;
pub const PIPE_NAME: &str = r"\\.\pipe\com.unidesk.app.desktop.v2";
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u32,
    pub request_id: u64,
    pub command: Command,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Command {
    Snapshot,
    Poll {
        session_epoch: u64,
        after_sequence: u64,
    },
    RequestPairing {
        peer_id: String,
    },
    RequestPairingAt {
        ipv4: String,
    },
    AcceptPairing {
        request_id: String,
    },
    RejectPairing {
        request_id: String,
    },
    Disconnect {
        peer_id: String,
    },
    Revoke {
        peer_id: String,
    },
    Rename {
        name: String,
    },
    InvalidateClipboard {
        session_epoch: u64,
        sequence: u64,
    },
    PublishClipboard {
        session_epoch: u64,
        sequence: u64,
        formats: Vec<Format>,
    },
    CommitClipboard {
        session_epoch: u64,
        sequence: u64,
        applied: bool,
    },
    RegisterInput {
        topology: unidesk_core::DisplayTopology,
    },
    PollInput {
        after_sequence: u64,
    },
    InputFault {
        message: String,
    },
    ReclaimInput,
}
// Intentionally no Debug: user clipboard body is not diagnostic data.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Format {
    pub mime: String,
    pub data_base64: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Clipboard {
    pub sequence: u64,
    pub peer_id: String,
    pub formats: Vec<Format>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Response {
    pub version: u32,
    pub request_id: u64,
    pub result: Reply,
}
#[derive(Clone, Serialize, Deserialize)]
pub enum Reply {
    Snapshot(Snapshot),
    Poll {
        snapshot: Snapshot,
        clipboard: Option<Clipboard>,
        clipboard_sequence: u64,
    },
    InputPoll {
        state: InputState,
        deliveries: Vec<InputDelivery>,
        sequence: u64,
    },
    Queued {
        message: String,
    },
    Rejected {
        code: String,
        message: String,
    },
    UnsupportedVersion,
}
impl Reply {
    pub fn queued() -> Self {
        Self::Queued {
            message: "queued; observe snapshot for completion".into(),
        }
    }
    pub fn rejected(code: &str, message: &str) -> Self {
        Self::Rejected {
            code: code.into(),
            message: message.into(),
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub package_version: String,
    pub session_epoch: u64,
    pub client_session_id: u32,
    pub sequence: u64,
    pub local_device_id: String,
    pub local_name: String,
    pub network_running: bool,
    pub control_port: u16,
    pub capabilities: Capabilities,
    pub input: InputState,
    pub peers: Vec<Peer>,
    pub pending: Vec<Pending>,
    pub pairing_status: Option<String>,
    pub last_error: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Capabilities {
    pub clipboard: bool,
    pub files: bool,
    pub input: bool,
    pub updater: bool,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct InputState {
    pub helper_connected: bool,
    pub managed: bool,
    pub status: String,
    pub reason: Option<String>,
    pub controller_id: Option<String>,
    pub display_count: usize,
}
#[derive(Clone, Serialize, Deserialize)]
pub enum InputDelivery {
    Lease {
        sequence: u64,
        session_id: String,
        lease_revision: u64,
    },
    Batch {
        sequence: u64,
        session_id: String,
        target_display: String,
        state_epoch: u64,
        events: Vec<unidesk_core::SharedInputEvent>,
    },
    Release {
        sequence: u64,
        reason: String,
    },
}
impl InputDelivery {
    #[must_use]
    pub fn sequence(&self) -> u64 {
        match self {
            Self::Lease { sequence, .. }
            | Self::Batch { sequence, .. }
            | Self::Release { sequence, .. } => *sequence,
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Peer {
    pub peer_id: String,
    pub display_name: String,
    pub trust: String,
    pub connection: String,
    pub discovered_only: bool,
    pub selected_route: Option<String>,
    pub routes: Vec<serde_json::Value>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Pending {
    pub request_id: String,
    pub peer_id: String,
    pub display_name: String,
    pub incoming: bool,
    pub expires_at_unix_ms: u64,
    pub owner_name: String,
    pub member_names: Vec<String>,
}
