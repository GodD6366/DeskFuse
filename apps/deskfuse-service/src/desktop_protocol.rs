//! Desktop session-bound IPC. See `docs/WINDOWS_DESKTOP_IPC.md`.
use deskfuse_input::quality::InputQualityReport;
use deskfuse_platform::{BoundaryErrorCode, PlatformCommand, PlatformEvent};
use serde::{Deserialize, Serialize};

/// 缺省质量报告。`empty` 必须为真，否则跳过序列化后再读回来会变成非空闲。
pub(crate) fn idle_quality() -> InputQualityReport {
    InputQualityReport {
        empty: true,
        grade: "idle".into(),
        present_supported: false,
        ..InputQualityReport::default()
    }
}

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
/// Request identifier reserved for frames the service sends without a request.
/// The helper treats `request_id == 0` as an unsolicited push and never
/// acknowledges it, so the request/response ack stream stays balanced.
pub const PUSH_REQUEST_ID: u64 = 0;
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
    RefreshPeers,
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
    Connect {
        peer_id: String,
    },
    Revoke {
        peer_id: String,
    },
    /// Remove the peer from every list, as opposed to `Revoke`, which keeps
    /// it visible as blocked. Mirrors the macOS Forget/Block split.
    Forget {
        peer_id: String,
    },
    /// Unblock a blocked peer so it can request pairing again.
    AllowPairing {
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
        #[serde(default)]
        source_app: Option<String>,
    },
    SetFileTransferEnabled {
        enabled: bool,
    },
    ConfigureLanWeb {
        enabled: bool,
    },
    CommitLanWebUpload {
        session_epoch: u64,
        upload_id: String,
        recorded: bool,
        clipboard_written: bool,
    },
    PublishFileClipboard {
        session_epoch: u64,
        sequence: u64,
        paths: Vec<String>,
        /// 只向这台设备报价；缺省向所有文件通道就绪的设备广播。
        #[serde(default)]
        target_peer_id: Option<String>,
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
    /// Declare that this helper applies `Reply::InputPush` frames. Sent once
    /// after a successful registration; a service without push support rejects
    /// the command and the helper keeps its ordinary poll cadence.
    EnableInputPush,
    PollInput {
        session_epoch: u64,
        after_sequence: u64,
        desktop: InputDesktop,
        /// helper 进程交来的采集和注入耗时。旧 helper 省略时按零处理。
        #[serde(
            default,
            skip_serializing_if = "deskfuse_core::PendingStages::is_empty"
        )]
        stages: deskfuse_core::PendingStages,
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
        /// 这一批里累加的设备计数。缺省表示没有相对移动。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        relative: Option<(f64, f64)>,
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
    /// Persist the session-gate relaxation switches. Requires a fully present
    /// (unlocked physical console) session; relaxed sessions cannot widen access.
    SetSessionPolicy {
        allow_locked: bool,
        allow_remote: bool,
    },
    /// Persist the current layout as a named scene.
    SaveScene {
        name: String,
    },
    ApplyScene {
        scene_id: String,
    },
    RenameScene {
        scene_id: String,
        name: String,
    },
    DeleteScene {
        scene_id: String,
    },
    /// Create or refresh the layout scene bound to the current Wi-Fi network.
    SaveEnvironmentLayout {
        name: Option<String>,
    },
    LibraryQuery {
        request: String,
    },
    LibraryMutate {
        request: String,
    },
    /// Materialize one library row. Payload transfer may be queued for a
    /// synced row; the caller must treat a rejection as "not available yet".
    LibraryContent {
        id: u64,
    },
    /// Display-sized PNG for the library grid; never the restorable payload.
    LibraryThumbnail {
        id: u64,
        #[serde(default)]
        max_pixel_size: Option<u32>,
    },
    LibraryIngest {
        formats: Vec<Format>,
        files: Vec<String>,
        source: String,
        #[serde(default)]
        device_id: String,
        #[serde(default)]
        app_id: Option<String>,
        #[serde(default)]
        app_name: Option<String>,
        #[serde(default)]
        copied_at_ms: Option<u64>,
    },
    /// Send one canonical library entry to a single paired peer. Text and
    /// image entries use the acknowledged clipboard channel; entries with
    /// files use the existing deferred file offer path.
    ShareLibraryEntry {
        id: u64,
        peer_id: String,
    },
    /// Send workflow output that is not in the library yet. Mirrors the
    /// macOS share menu's "使用工作流发送"; nothing is written locally.
    ShareContent {
        peer_id: String,
        formats: Vec<Format>,
    },
    GetWorkflows,
    SaveWorkflow {
        workflow: deskfuse_core::workflow::Workflow,
    },
    DeleteWorkflow {
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
    #[serde(default)]
    pub source_metadata: deskfuse_protocol::clipboard_sync::SourceMetadata,
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
// 每次 IPC 请求只构造一次并立即序列化，枚举大小不影响性能。
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Serialize, Deserialize)]
pub enum Reply {
    Snapshot(Snapshot),
    Poll {
        snapshot: Snapshot,
        clipboard: Option<Clipboard>,
        clipboard_sequence: u64,
        #[serde(default)]
        lan_web_uploads: Vec<LanWebUpload>,
    },
    InputRegistered {
        session_epoch: u64,
    },
    InputPoll {
        state: InputState,
        deliveries: Vec<InputDelivery>,
        sequence: u64,
    },
    /// Unsolicited delivery on the helper's persistent connection. Carries only
    /// what the poll reply carries for deliveries; the helper still polls for
    /// the state view, so push latency never depends on the poll cadence.
    InputPush {
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
    LibraryResult {
        json: String,
    },
    LibraryContent {
        formats: Vec<Format>,
        files: Vec<String>,
    },
    LibraryThumbnail {
        mime: String,
        data_base64: String,
    },
    RemoteApps {
        available: bool,
        reason: Option<String>,
        apps: Vec<RemoteAppInfo>,
        /// 服务端预检：RDP 监听端口、监听证书指纹与逐项检查。
        /// 来自后台缓存；服务刚启动尚未完成首次预检时为 `None`。
        #[serde(default)]
        host: Option<crate::remote_rdp_protocol::RdpHost>,
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
    #[serde(default)]
    pub library_revision: Option<u64>,
    pub local_device_id: String,
    pub local_name: String,
    pub network_running: bool,
    pub control_port: u16,
    pub capabilities: Capabilities,
    pub input: InputState,
    pub peers: Vec<Peer>,
    pub pending: Vec<Pending>,
    #[serde(default)]
    pub file_settings: deskfuse_core::FileTransferSettings,
    #[serde(default)]
    pub lan_web: LanWebState,
    pub file_transfers: Vec<FileTransferView>,
    pub file_cache: FileCache,
    pub pairing_status: Option<String>,
    pub last_error: Option<String>,
    /// Effective session-gate policy switches persisted by the service.
    #[serde(default)]
    pub session_policy: SessionPolicyView,
    /// False while this client session itself runs relaxed (locked console or
    /// remote protocol); such sessions cannot change policy, pairing or input.
    #[serde(default)]
    pub session_full_presence: bool,
}
/// User-facing copy of the service session gate; see `policy::SessionPolicy`.
#[derive(Clone, Copy, Default, Serialize, Deserialize)]
pub struct SessionPolicyView {
    pub allow_locked: bool,
    pub allow_remote: bool,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct FileCache {
    pub max_bytes: u64,
    pub used_bytes: u64,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct LanWebState {
    pub enabled: bool,
    pub running: bool,
    pub session_available: bool,
    pub address: Option<String>,
    pub addresses: Vec<String>,
    pub port: u16,
    pub error: Option<String>,
    pub tasks: Vec<LanWebTask>,
    pub qr_size: usize,
    pub qr_modules: Vec<bool>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct LanWebTask {
    pub id: String,
    pub name: String,
    pub state: String,
    pub received_bytes: u64,
    pub total_bytes: Option<u64>,
    pub recorded: Option<bool>,
    pub clipboard_written: Option<bool>,
    pub message: String,
    #[serde(skip)]
    pub result: Option<serde_json::Value>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct LanWebUpload {
    pub id: String,
    pub name: String,
    pub formats: Vec<Format>,
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
    /// Authenticated, bounded JPEG previews, keyed by peer and display id.
    #[serde(default)]
    pub remote_wallpapers:
        std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>>,
    /// Input session links with granted control sessions.
    pub links: Vec<LinkView>,
    /// Wi-Fi environment observed by the service (SSID is a switch key for
    /// layout restore, never an authorization).
    #[serde(default)]
    pub environment: EnvironmentView,
    /// Saved input layout scenes, including Wi-Fi-bound ones.
    #[serde(default)]
    pub scenes: Vec<SceneView>,
    /// 服务进程里的质量报告。helper 进程的采集和注入不在这里。
    #[serde(
        default = "idle_quality",
        skip_serializing_if = "InputQualityReport::is_idle"
    )]
    pub quality: InputQualityReport,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct EnvironmentView {
    pub ssid: Option<String>,
    pub message: String,
    pub conflict: bool,
    /// 本机实际读到的 SSID；`ssid` 是场景判定采用的值（可能来自对端上报）。
    #[serde(default)]
    pub observed_ssid: Option<String>,
    /// 各在线对端经输入链路上报的 SSID，用于界面核对两端是否同一网络。
    #[serde(default)]
    pub peer_ssids: Vec<PeerSsidView>,
    /// 已保存的 Wi-Fi → 首选主控规则。
    #[serde(default)]
    pub rules: Vec<EnvironmentRuleView>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct PeerSsidView {
    pub device_id: String,
    pub ssid: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct EnvironmentRuleView {
    pub ssid: String,
    pub controller: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct SceneView {
    pub id: String,
    pub name: String,
    pub ssid: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion_fallback_reason: Option<String>,
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
