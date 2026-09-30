//! Desktop session-bound IPC. See `docs/WINDOWS_DESKTOP_IPC.md`.
use deskfuse_platform::{BoundaryErrorCode, PlatformCommand, PlatformEvent};
use serde::{Deserialize, Serialize};

#[cfg(windows)]
pub use deskfuse_transfer::{DeferredEntryKind, DeferredFileEntry, FileTransferView};
#[cfg(not(windows))]
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct DeferredFileEntry {
    pub path: String,
    pub kind: String,
    pub size: u64,
    pub lindex: u32,
}
// The desktop pipe is Windows-only; non-Windows builds keep the crate green.
#[cfg(not(windows))]
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct FileTransferView {
    pub transfer_id: String,
    pub peer_id: String,
    pub direction: String,
    pub names: Vec<String>,
    #[serde(default)]
    pub entries: Vec<DeferredFileEntry>,
    pub total_bytes: u64,
    pub transferred_bytes: u64,
    pub state: String,
    pub detail: Option<String>,
    pub staged_paths: Vec<String>,
    pub created_unix_ms: u64,
    pub updated_unix_ms: u64,
}
pub const VERSION: u32 = 2;
pub const MAX_FRAME: usize = 24 * 1024 * 1024;
pub const MAX_CONTENT: usize = 16 * 1024 * 1024;
pub const PIPE_NAME: &str = r"\\.\pipe\com.deskfuse.app.desktop.v2";
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
    PublishFileClipboard {
        session_epoch: u64,
        sequence: u64,
        paths: Vec<String>,
    },
    /// Typed platform boundary. Legacy clipboard/file commands remain
    /// wire-compatible; native host payloads stay on this authenticated pipe.
    Platform {
        session_epoch: u64,
        sequence: u64,
        command: PlatformCommand,
    },
    RequestFileClipboard {
        session_epoch: u64,
        transfer_id: String,
    },
    CancelFileClipboard {
        session_epoch: u64,
        transfer_id: String,
    },
    CommitFileClipboard {
        session_epoch: u64,
        transfer_id: String,
        applied: bool,
    },
    CommitClipboard {
        session_epoch: u64,
        sequence: u64,
        applied: bool,
    },
    RegisterInput {
        topology: deskfuse_core::DisplayTopology,
    },
    PollInput {
        session_epoch: u64,
        after_sequence: u64,
        desktop: InputDesktop,
    },
    InputFault {
        message: String,
    },
    EnterInputControl {
        session_id: String,
        target_display: String,
        x: f64,
        y: f64,
    },
    SendInputEvents {
        session_id: String,
        target_display: String,
        batch: u64,
        events: Vec<deskfuse_core::SharedInputEvent>,
    },
    LeaveInputControl {
        session_id: String,
    },
    ReleaseInputControl {
        session_id: String,
    },
    EnableInputControl {
        peer_id: String,
    },
    DisableInputControl,
    RestartInputControl {
        peer_id: String,
    },
    MoveControlDevice {
        device_id: String,
        display_id: String,
        x: f64,
        y: f64,
    },
    SetInputSharingSettings {
        settings: deskfuse_core::InputSharingSettings,
    },
    GetWorkflows,
    SaveWorkflow {
        workflow: deskfuse_core::workflow::Workflow,
    },
    DeleteWorkflow {
        id: String,
    },
    SaveWorkflowRule {
        rule: deskfuse_core::workflow::WorkflowRule,
    },
    DeleteWorkflowRule {
        id: String,
    },
    RunWorkflow {
        workflow_id: String,
        input: Vec<WorkflowInput>,
    },
    CancelWorkflow {
        run_id: String,
    },
    GetWorkflowCatalog,
    RestoreWorkflowConflict {
        conflict_id: String,
    },
    ReclaimInput,
    ListRemoteApps,
    ListInstalledApps,
    RegisterRemoteApp {
        name: String,
        path: String,
        display_name: String,
    },
    UnregisterRemoteApp {
        name: String,
    },
    RemoteAppConnection {
        name: String,
    },
}
// Intentionally no Debug: user clipboard body is not diagnostic data.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Format {
    pub mime: String,
    pub data_base64: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowInput {
    pub mime: String,
    pub data_base64: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Clipboard {
    pub sequence: u64,
    pub peer_id: String,
    pub formats: Vec<Format>,
    /// Ordered with text/image copies; absent for ordinary content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_transfer_id: Option<String>,
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
    InputRegistered {
        session_epoch: u64,
    },
    InputPoll {
        state: InputState,
        deliveries: Vec<InputDelivery>,
        sequence: u64,
    },
    Workflows {
        workflows: Vec<deskfuse_core::workflow::Workflow>,
    },
    WorkflowResult {
        run_id: String,
        workflow_id: String,
        formats: Vec<Format>,
        message: String,
    },
    WorkflowCatalog {
        json: String,
    },
    RemoteApps {
        available: bool,
        reason: Option<String>,
        apps: Vec<RemoteAppInfo>,
    },
    InstalledApps {
        apps: Vec<InstalledAppInfo>,
    },
    RemoteAppConnection {
        rdp: String,
    },
    Queued {
        message: String,
    },
    Platform {
        event: PlatformEvent,
    },
    Rejected {
        /// Stable legacy string retained for v2 desktop clients.
        code: String,
        message: String,
        /// New typed mapping; old clients ignore this additive field.
        #[serde(default)]
        boundary_error: BoundaryErrorCode,
    },
    UnsupportedVersion,
}
impl Reply {
    pub fn queued() -> Self {
        Self::Queued {
            message: "queued; observe snapshot for completion".into(),
        }
    }

    /// Construct new rejection responses from the typed boundary code. The
    /// legacy `code` field is only a canonical compatibility projection; new
    /// command paths must not add string-to-error mappings at the call site.
    pub fn rejected_typed(code: BoundaryErrorCode, message: impl Into<String>) -> Self {
        Self::Rejected {
            code: code.as_str().into(),
            message: message.into(),
            boundary_error: code,
        }
    }

    /// Compatibility constructor for pre-typed v2 commands. Keep all legacy
    /// string interpretation here so new boundary code cannot copy the map.
    pub fn rejected(code: &str, message: &str) -> Self {
        Self::Rejected {
            code: code.into(),
            message: message.into(),
            boundary_error: BoundaryErrorCode::from_legacy_code(code),
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
    pub file_transfers: Vec<FileTransferView>,
    pub file_cache: FileCache,
    pub pairing_status: Option<String>,
    pub last_error: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct FileCache {
    pub max_bytes: u64,
    pub used_bytes: u64,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Capabilities {
    pub clipboard: bool,
    pub files: bool,
    pub input: bool,
    #[serde(default)]
    pub elevated_input: bool,
    #[serde(default)]
    pub secure_desktop_input: bool,
    #[serde(default)]
    pub login_input: bool,
    #[serde(default)]
    pub secure_attention_sequence: bool,
    pub updater: bool,
    pub remote_app: bool,
    /// Runtime observations, not user preferences.
    #[serde(default)]
    pub workflow_execution: bool,
    #[serde(default)]
    pub workflow_sync: bool,
}
/// The fixed helper probes its own session/desktop; the Session-0 service must not
/// mistake its own desktop for the user's input desktop. Unknown values fail closed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputDesktop {
    Default,
    Protected,
    Locked,
    #[default]
    Unavailable,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct InputSecurityState {
    pub session_bound: bool,
    pub ui_access: bool,
    pub elevated: bool,
    pub desktop: InputDesktop,
    pub reason: Option<String>,
    /// Secure-desktop and login capabilities require a separately authenticated
    /// session agent. Ordinary UIAccess helper state must never populate this.
    #[serde(default)]
    pub session_agent: SessionAgentState,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct SessionAgentState {
    /// True only while the service has a live, kernel-authenticated agent
    /// channel for the current interactive session.
    pub connected: bool,
    /// True only after service authorization, explicit user authorization and
    /// session/epoch binding have all been validated.
    pub authorized: bool,
    /// Lock screen or UAC secure-desktop input; login is reported separately.
    pub secure_desktop_input: bool,
    pub locked_desktop_input: bool,
    pub login_input: bool,
    /// Deliberately always false: SAS is owned by Windows and is never injected
    /// or intercepted by DeskFuse.
    pub secure_attention_sequence: bool,
    pub reason: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct InputState {
    #[serde(default)]
    pub security: InputSecurityState,
    pub helper_connected: bool,
    pub managed: bool,
    pub status: String,
    pub reason: Option<String>,
    pub controller_id: Option<String>,
    pub display_count: usize,
    /// The local device identity, needed to anchor the input layout.
    pub local_device_id: String,
    /// Effective sharing settings; `None` before the input runtime reports.
    pub settings: Option<deskfuse_core::InputSharingSettings>,
    /// The control session this machine currently holds, if any.
    pub controlling: Option<ControlView>,
    /// Outgoing controller activations, one per participant peer.
    pub activations: Vec<ActivationView>,
    /// Remote input topologies of participant peers.
    pub remote_topologies: Vec<RemoteTopologyView>,
    /// Input session links with granted control sessions.
    pub links: Vec<LinkView>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct ControlView {
    pub session_id: String,
    pub peer_id: String,
    pub target_display: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct ActivationView {
    pub peer_id: String,
    pub status: String,
    pub reason: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct RemoteTopologyView {
    pub peer_id: String,
    pub topology: deskfuse_core::DisplayTopology,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct LinkView {
    pub peer_id: String,
    pub ready: bool,
    pub control_session: Option<String>,
    pub lease_revision: u64,
    pub motion_mode: Option<String>,
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
        events: Vec<deskfuse_core::SharedInputEvent>,
    },
    Release {
        sequence: u64,
        reason: String,
    },
    ControlGranted {
        sequence: u64,
        session_id: String,
        lease_ms: u64,
    },
    ControlDenied {
        sequence: u64,
        reason: String,
    },
    ControlReleased {
        sequence: u64,
        session_id: String,
        reason: String,
    },
}
impl InputDelivery {
    #[must_use]
    pub fn sequence(&self) -> u64 {
        match self {
            Self::Lease { sequence, .. }
            | Self::Batch { sequence, .. }
            | Self::Release { sequence, .. }
            | Self::ControlGranted { sequence, .. }
            | Self::ControlDenied { sequence, .. }
            | Self::ControlReleased { sequence, .. } => *sequence,
        }
    }
    #[must_use]
    pub fn is_controller_event(&self) -> bool {
        matches!(
            self,
            Self::ControlGranted { .. } | Self::ControlDenied { .. } | Self::ControlReleased { .. }
        )
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

/// A registered RemoteApp surfaced to the controller for connection.
#[derive(Clone, Serialize, Deserialize)]
pub struct RemoteAppInfo {
    pub name: String,
    pub display_name: String,
    pub path: String,
    pub command_line: String,
    pub icon_path: String,
    pub icon_index: u32,
}

/// An installed executable surfaced to the local picker for registration.
#[derive(Clone, Serialize, Deserialize)]
pub struct InstalledAppInfo {
    pub name: String,
    pub path: String,
}
