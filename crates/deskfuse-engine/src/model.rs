//! UI-facing model and runtime-to-presentation mapping.
//!
//! This module owns no GPUI state and performs no I/O.  Discovery hints remain
//! separate from authenticated peers so that an mDNS result can never appear
//! as a trusted or connected device.

use std::collections::BTreeSet;
use std::str::FromStr;

use deskfuse_core::{
    ConnectionState, DeviceId, DisplayInfo, LivenessState, NetworkSnapshot, PairingRequest,
    RequestId, RouteId, RouteSource as CoreRouteSource, TrustState as CoreTrustState,
};
use deskfuse_platform::PermissionSnapshot;
use deskfuse_runtime::{DiscoveredPeerHint, RuntimeCommand, RuntimeEvent, SnapshotView};

/// Clipboard synchronization is active on first launch. The worker still
/// snapshots the current pasteboard change count before polling, so enabling
/// this default never republishes content copied before DeskFuse started.
pub const DEFAULT_CLIPBOARD_SYNC_ENABLED: bool = true;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteSource {
    Direct,
    Manual,
    /// A direct route established through the public-network provider.
    PublicDirect,
    /// A relayed route established through the public-network provider.
    PublicRelay,
}

impl RouteSource {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Direct => "直接链路",
            Self::Manual => "设备网络",
            Self::PublicDirect => "公网直连",
            Self::PublicRelay => "公网中继",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Ready,
    Checking,
    Offline,
    NotEnabled,
}

impl Health {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Ready => "健康",
            Self::Checking => "检查中",
            Self::Offline => "离线",
            Self::NotEnabled => "尚未启用",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustState {
    Trusted,
    Pending,
    Unknown,
    Blocked,
}

impl TrustState {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Trusted => "已信任",
            Self::Pending => "待确认",
            Self::Unknown => "未验证",
            Self::Blocked => "已阻止",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferState {
    Preparing,
    Transferring,
    Verifying,
    Delivering,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferDirection {
    Send,
    Receive,
}

impl TransferDirection {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Send => "发送",
            Self::Receive => "接收",
        }
    }
}

impl TransferState {
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
    pub const fn label(self) -> &'static str {
        match self {
            Self::Preparing => "准备中",
            Self::Transferring => "传输中",
            Self::Verifying => "校验中",
            Self::Delivering => "保存中",
            Self::Completed => "已完成",
            Self::Failed => "失败",
            Self::Cancelled => "已取消",
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeStatus {
    Ready,
    Discovering,
    Offline,
    Disabled,
    Error,
}

impl RuntimeStatus {
    pub const fn to_health(self) -> Health {
        match self {
            Self::Ready => Health::Ready,
            Self::Discovering => Health::Checking,
            Self::Offline | Self::Error => Health::Offline,
            Self::Disabled => Health::NotEnabled,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Ready => "运行中",
            Self::Discovering => "发现中",
            Self::Offline => "离线",
            Self::Disabled => "已停用",
            Self::Error => "错误",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub subtitle: String,
    pub route_source: RouteSource,
    pub health: Health,
    pub trust: TrustState,
    pub endpoint: String,
    pub signal: String,
    pub selected_route_id: Option<RouteId>,
    /// Preserve every runtime candidate; one display endpoint is not the route set.
    pub routes: Vec<deskfuse_core::RouteCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferItem {
    pub id: String,
    pub name: String,
    pub detail: String,
    pub state: TransferState,
    pub transferred_bytes: u64,
    pub total_bytes: u64,
    pub direction: TransferDirection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardHistoryItem {
    pub id: u64,
    /// Source application metadata travels with the first UI snapshot.
    pub source_app_id: Option<String>,
    /// Last observed copy time on this Mac; absent for untimed retained entries.
    pub copied_at_ms: Option<u64>,
    pub mime: String,
    pub length: usize,
    pub preview: String,
    pub content: String,
    pub content_truncated: bool,
    pub image_png: Option<std::sync::Arc<Vec<u8>>>,
    pub file_paths: Vec<String>,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingRequestView {
    pub request_id: RequestId,
    pub peer: Device,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSnapshot {
    pub snapshot: NetworkSnapshot,
    pub discovered_peers: Vec<DiscoveredPeerHint>,
}

impl From<SnapshotView> for RuntimeSnapshot {
    fn from(view: SnapshotView) -> Self {
        Self {
            snapshot: view.snapshot,
            discovered_peers: view.discovered_peers,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppModel {
    pub local: Device,
    /// Authenticated peers only. Discovery hints are intentionally separate.
    pub peers: Vec<Device>,
    pub discovered_peers: Vec<Device>,
    pub pairing_requests: Vec<PairingRequestView>,
    pub device_network: deskfuse_runtime::DeviceNetworkSnapshot,
    pub pairing_status: Option<String>,
    pub runtime_status: RuntimeStatus,
    pub runtime_error: Option<String>,
    pub updates: deskfuse_runtime::updates::LanUpdateView,
    runtime_error_sticky: bool,
    pub permissions: PermissionSnapshot,
    pub transfer_queue: Vec<TransferItem>,
    pub clipboard_sync_enabled: bool,
    pub clipboard_last_mime: Option<String>,
    pub clipboard_last_len: usize,
    pub clipboard_last_sequence: Option<u64>,
    pub clipboard_error: Option<String>,
    pub clipboard_delivery: std::collections::BTreeMap<deskfuse_core::DeviceId, String>,
    pub clipboard_history: Vec<ClipboardHistoryItem>,
    pub remote_control_peer: Option<String>,
    pub remote_control_active: bool,
    pub remote_display: Option<DisplayInfo>,
    pub remote_control_error: Option<String>,
    pub input_error: Option<String>,
    pub input_sharing: deskfuse_core::InputSharingSnapshot,
    pub input_locked: bool,
}

impl AppModel {
    pub fn offline(error: impl Into<String>) -> Self {
        let mut model = Self {
            local: Device {
                id: "unknown".to_owned(),
                name: "这台 Mac".to_owned(),
                subtitle: "网络运行时不可用".to_owned(),
                route_source: RouteSource::Direct,
                health: Health::Offline,
                trust: TrustState::Trusted,
                endpoint: "—".to_owned(),
                signal: "—".to_owned(),
                selected_route_id: None,
                routes: Vec::new(),
            },
            peers: Vec::new(),
            discovered_peers: Vec::new(),
            pairing_requests: Vec::new(),
            device_network: Default::default(),
            pairing_status: None,
            runtime_status: RuntimeStatus::Error,
            runtime_error: Some(error.into()),
            updates: Default::default(),
            runtime_error_sticky: true,
            permissions: PermissionSnapshot::all_unknown(),
            transfer_queue: Vec::new(),
            clipboard_sync_enabled: DEFAULT_CLIPBOARD_SYNC_ENABLED,
            clipboard_last_mime: None,
            clipboard_last_len: 0,
            clipboard_last_sequence: None,
            clipboard_error: None,
            clipboard_delivery: Default::default(),
            clipboard_history: Vec::new(),
            remote_control_peer: None,
            remote_control_active: false,
            remote_display: None,
            remote_control_error: None,
            input_error: None,
            input_sharing: Default::default(),
            input_locked: false,
        };
        model.reconcile_device_presentations();
        model
    }

    pub fn starting() -> Self {
        let mut model = Self::offline("网络运行时正在启动；等待真实设备状态");
        model.local.subtitle = "网络运行时正在启动".to_owned();
        model.local.health = Health::Checking;
        model.runtime_status = RuntimeStatus::Discovering;
        model.runtime_error = None;
        model.runtime_error_sticky = false;
        model
    }

    pub fn from_runtime_view(view: SnapshotView) -> Self {
        let device_network = view.device_network.clone();
        let runtime = RuntimeSnapshot::from(view);
        let local = local_device(runtime.snapshot.local_device_id, RuntimeStatus::Ready);
        let peers = runtime
            .snapshot
            .peers
            .iter()
            .map(device_from_peer)
            .collect();
        let discovered_peers = runtime
            .discovered_peers
            .iter()
            .map(device_from_hint)
            .collect();
        let mut model = Self {
            local,
            peers,
            discovered_peers,
            pairing_requests: Vec::new(),
            device_network: Default::default(),
            pairing_status: None,
            runtime_status: RuntimeStatus::Ready,
            runtime_error: None,
            updates: Default::default(),
            runtime_error_sticky: false,
            permissions: PermissionSnapshot::all_unknown(),
            transfer_queue: Vec::new(),
            clipboard_sync_enabled: DEFAULT_CLIPBOARD_SYNC_ENABLED,
            clipboard_last_mime: None,
            clipboard_last_len: 0,
            clipboard_last_sequence: None,
            clipboard_error: None,
            clipboard_delivery: Default::default(),
            clipboard_history: Vec::new(),
            remote_control_peer: None,
            remote_control_active: false,
            remote_display: None,
            remote_control_error: None,
            input_error: None,
            input_sharing: Default::default(),
            input_locked: false,
        };
        model.device_network = device_network;
        for request in model
            .device_network
            .pending
            .iter()
            .filter(|p| p.incoming)
            .filter_map(|p| p.request.clone())
            .collect::<Vec<_>>()
        {
            model.add_pairing_request(request);
        }
        model.reconcile_device_presentations();
        model
    }

    pub fn apply_runtime_view(&mut self, view: SnapshotView) {
        let updates = std::mem::take(&mut self.updates);
        let pairing_status = self.pairing_status.clone();
        let transfer_queue = std::mem::take(&mut self.transfer_queue);
        let clipboard_sync_enabled = self.clipboard_sync_enabled;
        let clipboard_last_mime = self.clipboard_last_mime.clone();
        let clipboard_last_len = self.clipboard_last_len;
        let clipboard_last_sequence = self.clipboard_last_sequence;
        let clipboard_error = self.clipboard_error.clone();
        let clipboard_delivery = self.clipboard_delivery.clone();
        let clipboard_history = std::mem::take(&mut self.clipboard_history);
        let remote_control_peer = self.remote_control_peer.clone();
        let remote_control_active = self.remote_control_active;
        let remote_display = self.remote_display;
        let remote_control_error = self.remote_control_error.clone();
        let input_error = self.input_error.clone();
        let input_sharing = self.input_sharing.clone();
        let input_locked = self.input_locked;
        let permissions = self.permissions;
        let sticky_error = self.runtime_error_sticky;
        let previous_local = self.local.clone();
        let previous_status = self.runtime_status;
        let previous_error = self.runtime_error.clone();
        *self = Self::from_runtime_view(view);
        self.updates = updates;
        self.pairing_status = pairing_status;
        self.transfer_queue = transfer_queue;
        self.clipboard_sync_enabled = clipboard_sync_enabled;
        self.clipboard_last_mime = clipboard_last_mime;
        self.clipboard_last_len = clipboard_last_len;
        self.clipboard_last_sequence = clipboard_last_sequence;
        self.clipboard_error = clipboard_error;
        self.clipboard_delivery = clipboard_delivery;
        self.clipboard_history = clipboard_history;
        self.remote_control_peer = remote_control_peer;
        self.remote_control_active = remote_control_active;
        self.remote_display = remote_display;
        self.remote_control_error = remote_control_error;
        self.input_error = input_error;
        self.input_sharing = input_sharing;
        self.input_locked = input_locked;
        self.reconcile_device_presentations();
        self.permissions = permissions;
        if sticky_error {
            // A view is only a state observation. It is not proof that the
            // failed startup/command recovered, so keep the error sticky until
            // an explicit recovery action clears it.
            self.local = previous_local;
            self.runtime_status = previous_status;
            self.runtime_error = previous_error;
            self.runtime_error_sticky = true;
        }
    }

    pub fn record_runtime_error(&mut self, error: impl Into<String>) {
        self.runtime_status = RuntimeStatus::Error;
        self.runtime_error = Some(runtime_error_label(&error.into()));
        self.runtime_error_sticky = true;
    }

    pub fn clear_pairing_block_error(&mut self) {
        if self.runtime_error.as_ref().is_some_and(|e| {
            e.contains("已在本机被阻止") || e.contains("device network sender is blocked")
        }) {
            self.runtime_error = None;
            self.runtime_error_sticky = false;
        }
    }

    pub fn recover_runtime(&mut self) {
        // Clearing a sticky error is an explicit recovery transition, not a
        // claim that the next snapshot is already healthy.
        self.runtime_status = RuntimeStatus::Discovering;
        self.runtime_error = None;
        self.runtime_error_sticky = false;
        self.local.health = Health::Checking;
        self.local.signal = "—".to_owned();
    }

    pub fn apply_permission_snapshot(&mut self, snapshot: PermissionSnapshot) {
        self.permissions = snapshot;
    }

    pub fn apply_clipboard_status(
        &mut self,
        enabled: bool,
        last_mime: Option<String>,
        last_len: usize,
        last_sequence: Option<u64>,
        error: Option<String>,
    ) {
        self.clipboard_sync_enabled = enabled;
        self.clipboard_last_mime = last_mime;
        self.clipboard_last_len = last_len;
        self.clipboard_last_sequence = last_sequence;
        self.clipboard_error = error;
    }

    pub fn apply_transfer_status(&mut self, item: TransferItem) {
        if let Some(existing) = self
            .transfer_queue
            .iter_mut()
            .find(|existing| existing.id == item.id)
        {
            // Transfer IDs identify attempts. A final state is immutable even
            // when a slower producer still has queued progress for this ID.
            if existing.state.is_terminal() {
                return;
            }
            let rank = |state| match state {
                TransferState::Preparing => 0,
                TransferState::Transferring => 1,
                TransferState::Verifying => 2,
                TransferState::Delivering => 3,
                _ => 4,
            };
            if rank(item.state) < rank(existing.state) {
                return;
            }
            let mut item = item;
            if item.state == existing.state {
                item.transferred_bytes = item.transferred_bytes.max(existing.transferred_bytes);
            }
            *existing = item;
        } else {
            self.transfer_queue.push(item);
        }
    }

    pub fn apply_remote_control_status(
        &mut self,
        peer_id: Option<String>,
        active: bool,
        display: Option<DisplayInfo>,
        error: Option<String>,
    ) {
        self.remote_control_peer = peer_id;
        self.remote_control_active = active;
        self.remote_display = display;
        self.remote_control_error = error;
    }

    pub fn apply_input_status(&mut self, error: Option<String>) {
        self.input_error = error;
    }

    /// Trusted peers that can act as a remote-control target. Discovery hints
    /// and pending pairing requests are intentionally excluded.
    pub fn input_sharing_summary(&self) -> String {
        if let Some(managed) = &self.input_sharing.managed_control {
            if managed.status == deskfuse_core::InputActivationStatus::PausedByUser {
                return "已收回控制 · 等待主控重新启用".into();
            }
            let name = self
                .peers
                .iter()
                .find(|peer| peer.id == managed.controller.to_hex())
                .map(|peer| peer.name.as_str())
                .unwrap_or("设备网络主控");
            return format!(
                "由 {name} 主控启用 · {}",
                input_activation_status_label(managed.status)
            );
        }
        if self.input_sharing.settings.role == deskfuse_core::InputShareRole::Controller {
            if self.input_sharing.settings.enabled {
                let mut text = String::from("本机主控已开启");
                if let Some(link) = self
                    .input_sharing
                    .links
                    .iter()
                    .find(|link| link.control_session.is_some())
                {
                    let mode = match link.motion_mode.as_deref() {
                        Some("relative") => "相对移动",
                        Some("hybrid") => "绝对坐标",
                        _ => "可靠路径",
                    };
                    text.push_str(" · ");
                    text.push_str(mode);
                    if let Some(reason) = link
                        .motion_fallback_reason
                        .as_deref()
                        .filter(|reason| !reason.is_empty())
                    {
                        text.push_str(" · ");
                        text.push_str(reason);
                    }
                }
                text
            } else {
                "本机主控已关闭".into()
            }
        } else {
            "等待设备网络主控启用".into()
        }
    }

    pub fn input_availability_reason(&self, peer: &str) -> String {
        if self.input_sharing.managed_control.is_some() {
            return self.input_sharing_summary();
        }
        if self.input_restart_required(peer) {
            // The far side reclaimed input locally, so this device stops
            // answering the automatic path until a new enable cycle starts.
            return "对端已收回输入 · 可重新启用共享".into();
        }
        if let Some(activation) =
            parse_device_id(peer).and_then(|peer| self.input_sharing.activations.get(&peer))
        {
            let status = input_activation_status_label(activation.status);
            return match activation.reason.as_deref() {
                Some(reason)
                    if activation.status == deskfuse_core::InputActivationStatus::Unavailable =>
                {
                    format!("{status} · {}", input_reason_label(Some(reason)))
                }
                _ => status.into(),
            };
        }
        if !self.input_sharing.settings.enabled {
            return "鼠标共享已关闭".into();
        }
        if self.device_network.integrity_error.is_some() {
            return "设备网络暂不可用，请退出后重新配对".into();
        }
        self.input_sharing
            .links
            .iter()
            .find(|link| link.peer_id.to_hex() == peer)
            .map(|link| {
                if link.ready && link.control_session.is_some() {
                    "已授权的局域网链路".into()
                } else {
                    input_reason_label(link.reason.as_deref()).to_owned()
                }
            })
            .unwrap_or_else(|| "等待输入链路状态".into())
    }

    /// True when this device reclaimed input locally and the controller's
    /// current enable cycle is retired. Only an explicit restart resumes it.
    pub fn input_restart_required(&self, peer: &str) -> bool {
        if self.input_sharing.managed_control.is_some()
            || !self.input_sharing.settings.enabled
            || self.input_sharing.settings.role != deskfuse_core::InputShareRole::Controller
        {
            return false;
        }
        parse_device_id(peer)
            .and_then(|peer| self.input_sharing.activations.get(&peer))
            .is_some_and(|activation| {
                activation.status == deskfuse_core::InputActivationStatus::PausedByUser
            })
    }

    pub fn is_network_member(&self, peer: &str) -> bool {
        self.device_network.integrity_error.is_none()
            && (self.device_network.network_id.is_none()
                || parse_device_id(peer)
                    .is_some_and(|id| self.device_network.member_ids.contains(&id)))
    }

    pub fn network_graph_peers(&self) -> Vec<&Device> {
        self.peers
            .iter()
            .filter(|peer| peer.trust == TrustState::Trusted && self.is_network_member(&peer.id))
            .collect()
    }

    pub fn control_targets(&self) -> Vec<&Device> {
        let local = parse_device_id(&self.local.id);
        self.peers
            .iter()
            .filter(|peer| {
                peer.trust == TrustState::Trusted
                    && local.is_some_and(|local| {
                        parse_device_id(&peer.id)
                            .is_some_and(|remote| self.device_network.allows(local, remote))
                    })
            })
            .collect()
    }

    pub fn apply_runtime_event(&mut self, event: RuntimeEvent) {
        match event {
            RuntimeEvent::Snapshot(snapshot) => {
                let sticky_error = self.runtime_error_sticky;
                if !sticky_error {
                    self.local = local_device(snapshot.local_device_id, RuntimeStatus::Ready);
                    self.runtime_status = RuntimeStatus::Ready;
                    self.runtime_error = None;
                }
                self.peers = snapshot.peers.iter().map(device_from_peer).collect();
                self.reconcile_device_presentations();
            }
            RuntimeEvent::DiscoveredPeerHint(hint) => {
                let device = device_from_hint(&hint);
                if self.peers.iter().any(|peer| peer.id == device.id) {
                    self.discovered_peers.retain(|peer| peer.id != device.id);
                    return;
                }
                if let Some(existing) = self
                    .discovered_peers
                    .iter_mut()
                    .find(|existing| existing.id == device.id)
                {
                    *existing = device;
                } else {
                    self.discovered_peers.push(device);
                }
                if self.runtime_status != RuntimeStatus::Error {
                    self.runtime_status = RuntimeStatus::Discovering;
                }
            }
            RuntimeEvent::PeerHintRemoved { peer_id } => {
                // Discovery hints are UI-only and must disappear as soon as
                // the runtime says that discovery no longer observes them.
                // Authenticated peers live in `self.peers`, so a same-ID
                // trusted device is intentionally left untouched here.
                let peer_id = peer_id.to_hex();
                self.discovered_peers.retain(|peer| peer.id != peer_id);
            }
            RuntimeEvent::PairingStatus { message } => {
                self.pairing_status = Some(message);
            }
            RuntimeEvent::PairingRequest(request) => self.add_pairing_request(request),
            RuntimeEvent::PairingRequestSent { .. } => {
                self.pairing_status = None;
            }
            RuntimeEvent::PairingFinished {
                request_id,
                outcome,
                ..
            } => {
                self.pairing_status = Some(
                    match outcome.as_str() {
                        "accepted" => "配对已完成，设备网络成员正在自动连接",
                        "rejected" => "本次配对邀请已被拒绝",
                        "expired" => "配对邀请已过期，可以重新发起",
                        _ => "配对未完成，请检查设备连接后重试",
                    }
                    .into(),
                );
                self.pairing_requests.retain(|r| r.request_id != request_id);
                self.device_network
                    .pending
                    .retain(|r| r.request_id != request_id);
            }
            RuntimeEvent::PairingRejected {
                request_id, reason, ..
            } => {
                self.pairing_requests
                    .retain(|request| request.request_id != request_id);
                self.runtime_status = RuntimeStatus::Error;
                self.runtime_error = Some(reason.unwrap_or_else(|| "配对请求被拒绝".to_owned()));
                self.runtime_error_sticky = true;
            }
            RuntimeEvent::ClipboardReceived { payload, .. } => {
                // Metadata only. Clipboard bodies must never be retained by
                // the UI model or its diagnostics.
                self.clipboard_last_mime = Some(payload.mime);
                self.clipboard_last_len = payload.data.len();
                self.clipboard_last_sequence = Some(payload.sequence);
                self.clipboard_error = None;
            }
            RuntimeEvent::InputReceived { .. } => {
                self.input_error = Some("已拒绝旧版未授权输入协议".to_owned());
            }
            RuntimeEvent::DisplayInfoReceived { display, .. } => {
                self.remote_display = Some(display);
            }
            RuntimeEvent::Error(error) => {
                // A peer handshake or command failure does not stop the local
                // listener. Keep its last observed health while showing the error.
                if !matches!(
                    error,
                    deskfuse_runtime::RuntimeError::Transport(_)
                        | deskfuse_runtime::RuntimeError::Protocol(_)
                        | deskfuse_runtime::RuntimeError::UnknownPeer(_)
                        | deskfuse_runtime::RuntimeError::UnknownPairingRequest(_)
                        | deskfuse_runtime::RuntimeError::PeerBlocked(_)
                        | deskfuse_runtime::RuntimeError::InvalidCommand(_)
                ) {
                    self.runtime_status = RuntimeStatus::Error;
                }
                self.runtime_error = Some(runtime_error_label(&error.to_string()));
                self.runtime_error_sticky = true;
            }
            _ => {}
        }
    }

    fn add_pairing_request(&mut self, request: PairingRequest) {
        let request_id = request.request_id();
        let peer_id = request.device_id().to_hex();
        if self
            .peers
            .iter()
            .any(|peer| peer.id == peer_id && peer.trust == TrustState::Trusted)
        {
            self.pairing_requests
                .retain(|existing| existing.peer.id != peer_id);
            return;
        }
        if self
            .pairing_requests
            .iter()
            .any(|existing| existing.request_id == request_id)
        {
            return;
        }
        let peer = Device {
            id: request.device_id().to_hex(),
            name: request.display_name().to_owned(),
            subtitle: "收到配对请求 · 等待你的确认".to_owned(),
            route_source: RouteSource::Direct,
            health: Health::Checking,
            trust: TrustState::Pending,
            endpoint: "待安全通道".to_owned(),
            signal: "—".to_owned(),
            selected_route_id: None,
            routes: Vec::new(),
        };
        self.pairing_requests
            .push(PairingRequestView { request_id, peer });
    }

    fn reconcile_device_presentations(&mut self) {
        let authenticated_ids = self
            .peers
            .iter()
            .map(|peer| peer.id.clone())
            .collect::<BTreeSet<_>>();
        let trusted_ids = self
            .peers
            .iter()
            .filter(|peer| peer.trust == TrustState::Trusted)
            .map(|peer| peer.id.clone())
            .collect::<BTreeSet<_>>();
        self.discovered_peers
            .retain(|peer| !authenticated_ids.contains(&peer.id));
        self.pairing_requests
            .retain(|request| !trusted_ids.contains(&request.peer.id));
    }

    pub fn command_refresh(&self) -> RuntimeCommand {
        RuntimeCommand::RefreshPeers
    }

    pub fn command_connect(&self, peer_id: &str) -> Option<RuntimeCommand> {
        parse_device_id(peer_id).map(|id| {
            if self
                .peers
                .iter()
                .any(|p| p.id == peer_id && p.trust == TrustState::Trusted)
            {
                RuntimeCommand::connect(id)
            } else {
                RuntimeCommand::RequestPairing { peer_id: id }
            }
        })
    }

    pub fn command_disconnect(&self, peer_id: &str) -> Option<RuntimeCommand> {
        parse_device_id(peer_id).map(RuntimeCommand::disconnect)
    }

    pub fn command_approve(&self, request_id: &str) -> Option<RuntimeCommand> {
        request_id
            .parse::<RequestId>()
            .ok()
            .map(RuntimeCommand::approve_pairing)
    }

    pub fn command_reject(&self, request_id: &str) -> Option<RuntimeCommand> {
        request_id
            .parse::<RequestId>()
            .ok()
            .map(|id| RuntimeCommand::RejectPairing {
                request_id: id,
                reason: Some("用户在 DeskFuse 中拒绝".to_owned()),
            })
    }

    pub fn command_forget(&self, peer_id: &str) -> Option<RuntimeCommand> {
        parse_device_id(peer_id).map(RuntimeCommand::forget_peer)
    }
}

fn parse_device_id(value: &str) -> Option<DeviceId> {
    DeviceId::from_str(value).ok()
}

pub fn input_activation_status_label(status: deskfuse_core::InputActivationStatus) -> &'static str {
    use deskfuse_core::InputActivationStatus::*;
    match status {
        Preparing => "正在准备输入服务",
        Ready => "输入服务已就绪",
        WaitingAccessibility => "等待辅助功能权限",
        WaitingInputMonitoring => "等待输入监控权限",
        LockedSession => "等待解锁桌面",
        BusyLocalController => "该设备正在作为主控使用",
        BusyOtherController => "该设备正由另一台主控管理",
        PausedByUser => "已紧急收回，等待主控重新启用",
        Unavailable => "暂不可用",
    }
}

fn input_reason_label(reason: Option<&str>) -> &str {
    if reason.is_some_and(|reason| {
        let lower = reason.to_ascii_lowercase();
        lower.contains("upgrade")
            || lower.contains("activation capability")
            || lower.contains("input_activation")
    }) {
        return "请将两端升级到支持主控启用的最新版 DeskFuse";
    }
    let Some(reason) = reason else {
        return "等待对端授权与租约";
    };
    if !reason.is_ascii() {
        return reason;
    }
    if reason.contains("disabled") {
        "鼠标共享已关闭"
    } else if reason.contains("input stream") || reason.contains("reconnecting") {
        "输入连接中断，正在重新连接"
    } else if reason.contains("permission") || reason.contains("unavailable") {
        "需要系统权限与已解锁的桌面会话"
    } else if reason.contains("trusted connected") {
        "需要已配对并在线、支持鼠标共享的设备"
    } else if reason.contains("physical") || reason.contains("LAN") || reason.contains("interface")
    {
        "等待真实局域网链路（Wi-Fi、以太网或雷雳）"
    } else if reason.contains("authorized") || reason.contains("lease") {
        "等待对端明确授权与控制租约"
    } else if reason.contains("controller") {
        "受控端已由另一台主控占用"
    } else {
        "共享暂不可用，请检查设备连接与授权"
    }
}

fn local_device(id: DeviceId, status: RuntimeStatus) -> Device {
    Device {
        id: id.to_hex(),
        name: "这台 Mac".to_owned(),
        subtitle: format!("设备 ID · {}", short_id(id)),
        route_source: RouteSource::Direct,
        health: status.to_health(),
        trust: TrustState::Trusted,
        endpoint: "本机".to_owned(),
        signal: if status == RuntimeStatus::Ready {
            "运行中".to_owned()
        } else {
            "—".to_owned()
        },
        selected_route_id: None,
        routes: Vec::new(),
    }
}

fn device_from_peer(peer: &deskfuse_core::PeerSnapshot) -> Device {
    // Display the actually selected route, not a fallback candidate.
    // When connected, selected_route must exist and be valid per core validation.
    let route = peer
        .selected_route
        .and_then(|id| peer.routes.iter().find(|route| route.id == id));
    let route_source = route.map_or(RouteSource::Direct, |route| map_route_source(route.source));
    let endpoint = route.map_or_else(
        || "无可用端点".to_owned(),
        |route| route.endpoint.to_string(),
    );
    Device {
        id: peer.id.to_hex(),
        name: peer.display_name.clone(),
        subtitle: peer_subtitle(peer.liveness, peer.connection),
        route_source,
        health: map_health(peer.connection, peer.liveness),
        trust: map_trust(peer.trust),
        endpoint,
        signal: map_signal(peer.liveness),
        selected_route_id: peer.selected_route,
        routes: peer.routes.clone(),
    }
}

fn device_from_hint(hint: &DiscoveredPeerHint) -> Device {
    let route = hint.routes.first();
    Device {
        id: hint.peer_id.to_hex(),
        name: hint.display_name.clone(),
        subtitle: "已发现 · 待安全通道".to_owned(),
        route_source: route.map_or(RouteSource::Direct, |route| map_route_source(route.source)),
        health: Health::Checking,
        trust: TrustState::Unknown,
        endpoint: route.map_or_else(
            || "待安全通道".to_owned(),
            |route| route.endpoint.to_string(),
        ),
        signal: "—".to_owned(),
        selected_route_id: None,
        routes: hint.routes.clone(),
    }
}

fn map_route_source(source: CoreRouteSource) -> RouteSource {
    match source {
        CoreRouteSource::Manual => RouteSource::Manual,
        CoreRouteSource::PublicRelay => RouteSource::PublicRelay,
        CoreRouteSource::PublicDirect => RouteSource::PublicDirect,
        CoreRouteSource::Mdns | CoreRouteSource::CachedDirect => RouteSource::Direct,
    }
}

fn map_trust(trust: CoreTrustState) -> TrustState {
    match trust {
        CoreTrustState::Trusted => TrustState::Trusted,
        CoreTrustState::Pending | CoreTrustState::Pairing => TrustState::Pending,
        CoreTrustState::Blocked => TrustState::Blocked,
        CoreTrustState::Unknown => TrustState::Unknown,
    }
}

fn map_health(connection: ConnectionState, liveness: LivenessState) -> Health {
    match connection {
        ConnectionState::Connected => Health::Ready,
        ConnectionState::Connecting => Health::Checking,
        ConnectionState::Backoff | ConnectionState::Failed => Health::Offline,
        ConnectionState::Disconnected => match liveness {
            LivenessState::Online => Health::Checking,
            LivenessState::Unknown | LivenessState::Offline | LivenessState::Stale => {
                Health::Offline
            }
        },
    }
}

fn map_signal(liveness: LivenessState) -> String {
    match liveness {
        LivenessState::Online => "在线".to_owned(),
        LivenessState::Stale => "陈旧".to_owned(),
        LivenessState::Offline => "离线".to_owned(),
        LivenessState::Unknown => "—".to_owned(),
    }
}

fn peer_subtitle(liveness: LivenessState, connection: ConnectionState) -> String {
    format!(
        "{} · {}",
        liveness_label(liveness),
        connection_label(connection)
    )
}

fn liveness_label(liveness: LivenessState) -> &'static str {
    match liveness {
        LivenessState::Online => "在线",
        LivenessState::Offline => "离线",
        LivenessState::Stale => "状态陈旧",
        LivenessState::Unknown => "等待状态",
    }
}

fn connection_label(connection: ConnectionState) -> &'static str {
    match connection {
        ConnectionState::Connected => "控制通道已连接",
        ConnectionState::Connecting => "正在连接",
        ConnectionState::Backoff => "连接退避",
        ConnectionState::Failed => "控制通道失败",
        ConnectionState::Disconnected => "控制通道未连接",
    }
}

fn runtime_error_label(error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    if lower.contains("fresh session proof")
        || lower.contains("fresh identity signer")
        || lower.contains("device-network upgrade required")
        || lower.contains("protocol version")
    {
        "设备身份握手不兼容，请将两端都升级到最新版 DeskFuse 后重新连接".into()
    } else if lower.contains("timed out") || lower.contains("timeout") {
        "设备连接超时，请确认对方在线且网络可达；若对方运行旧版，请将两端都升级到最新版 DeskFuse"
            .into()
    } else if lower.contains("connection is closed") {
        "设备连接已关闭，请确认对方在线后重新连接；旧版设备需与本机一起升级".into()
    } else {
        error.to_owned()
    }
}

fn short_id(id: DeviceId) -> String {
    id.to_hex().chars().take(12).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repair_action_clears_only_pairing_block_errors() {
        let mut model = AppModel::starting();
        model.record_runtime_error("device network sender is blocked or requires upgrade");
        model.clear_pairing_block_error();
        assert!(model.runtime_error.is_none());
        assert!(!model.runtime_error_sticky);
        model.record_runtime_error("disk persistence failed");
        let error = model.runtime_error.clone();
        model.clear_pairing_block_error();
        assert_eq!(model.runtime_error, error);
        assert!(model.runtime_error_sticky);
    }

    use deskfuse_core::{Capabilities, NetworkSnapshot, PeerSnapshot, RouteCandidate, RouteHealth};
    use deskfuse_identity::MemoryIdentityProvider;

    #[test]
    fn activation_statuses_are_readable_and_override_the_saved_off_switch() {
        use deskfuse_core::{
            ConnectionContext, InputActivationId, InputActivationStatus as Status, InterfaceKind,
            LinkScope, ManagedInputControl, TransportSessionId,
        };
        for status in [
            Status::Preparing,
            Status::Ready,
            Status::WaitingAccessibility,
            Status::WaitingInputMonitoring,
            Status::LockedSession,
            Status::BusyLocalController,
            Status::BusyOtherController,
            Status::PausedByUser,
            Status::Unavailable,
        ] {
            assert!(!input_activation_status_label(status).is_ascii());
        }
        let mut model = AppModel::starting();
        let persisted = model.input_sharing.settings.clone();
        let controller = DeviceId::from_bytes([9; 32]);
        model.input_sharing.managed_control = Some(ManagedInputControl {
            activation_id: InputActivationId::new(),
            controller,
            context: ConnectionContext {
                session_id: TransportSessionId::new(),
                local_addr: "192.0.2.1:42424".parse().unwrap(),
                remote_addr: "192.0.2.2:42424".parse().unwrap(),
                interface_name: Some("en0".into()),
                interface_kind: InterfaceKind::WiFi,
                scope: LinkScope::Lan,
            },
            status: Status::WaitingAccessibility,
            reason: None,
        });
        assert!(
            model
                .input_availability_reason(&controller.to_hex())
                .contains("等待辅助功能权限")
        );
        assert!(
            !model
                .input_availability_reason(&controller.to_hex())
                .contains("已关闭")
        );
        model.input_sharing.managed_control.as_mut().unwrap().status = Status::PausedByUser;
        assert!(
            model
                .input_availability_reason(&controller.to_hex())
                .contains("已收回控制")
        );
        assert_eq!(
            model.input_sharing_summary(),
            "已收回控制 · 等待主控重新启用"
        );
        model.input_sharing.managed_control.as_mut().unwrap().status = Status::Ready;
        assert!(model.input_sharing_summary().contains("主控启用"));
        assert!(!model.input_sharing_summary().contains("已关闭"));
        model.input_sharing.managed_control = None;
        assert_eq!(model.input_sharing_summary(), "等待设备网络主控启用");
        assert_eq!(model.input_sharing.settings, persisted);
    }

    #[test]
    fn graph_contains_only_verified_network_members_but_retains_legacy_management() {
        let mut model = AppModel::starting();
        let local = DeviceId::from_bytes([1; 32]);
        let member = DeviceId::from_bytes([2; 32]);
        let legacy = DeviceId::from_bytes([3; 32]);
        model.local.id = local.to_hex();
        for id in [member, legacy] {
            let mut peer = model.local.clone();
            peer.id = id.to_hex();
            peer.trust = TrustState::Trusted;
            model.peers.push(peer);
        }
        assert_eq!(model.network_graph_peers().len(), 2);
        model.device_network.network_id = Some("verified".into());
        model.device_network.member_ids = vec![local, member];
        assert!(model.is_network_member(&model.local.id));
        assert_eq!(
            model
                .network_graph_peers()
                .iter()
                .map(|p| &p.id)
                .collect::<Vec<_>>(),
            vec![&member.to_hex()]
        );
        assert!(!model.is_network_member(&legacy.to_hex()));
        assert_eq!(model.peers.len(), 2, "legacy peer must remain manageable");
        model.device_network.integrity_error = Some("invalid roster".into());
        assert!(model.network_graph_peers().is_empty());
        assert!(!model.is_network_member(&model.local.id));
        model.device_network = Default::default();
        assert_eq!(model.network_graph_peers().len(), 2);
    }

    #[test]
    fn peer_handshake_failure_keeps_local_listener_ready_and_explains_upgrade() {
        let mut model = AppModel::from_runtime_view(SnapshotView {
            device_network: Default::default(),
            snapshot: snapshot_with_no_peers(),
            discovered_peers: vec![],
        });
        model.apply_runtime_event(RuntimeEvent::Error(
            deskfuse_runtime::RuntimeError::Transport("fresh session proof required".into()),
        ));
        assert_eq!(model.runtime_status, RuntimeStatus::Ready);
        assert_eq!(model.local.health, Health::Ready);
        assert!(model.runtime_error.as_ref().unwrap().contains("两端都升级"));
        model.apply_runtime_view(SnapshotView {
            device_network: Default::default(),
            snapshot: snapshot_with_no_peers(),
            discovered_peers: vec![],
        });
        assert_eq!(model.runtime_status, RuntimeStatus::Ready);
        assert!(model.runtime_error.is_some());
        model.apply_runtime_event(RuntimeEvent::Error(deskfuse_runtime::RuntimeError::Store(
            "disk unavailable".into(),
        )));
        assert_eq!(model.runtime_status, RuntimeStatus::Error);
    }

    #[test]
    fn network_membership_limits_mouse_candidates_without_changing_clipboard_preference() {
        let mut model = AppModel::starting();
        let local = DeviceId::from_bytes([1; 32]);
        model.local.id = local.to_hex();
        let member = DeviceId::from_bytes([2; 32]);
        let outsider = DeviceId::from_bytes([3; 32]);
        for id in [member, outsider] {
            let mut peer = model.local.clone();
            peer.id = id.to_hex();
            peer.trust = TrustState::Trusted;
            model.peers.push(peer);
        }
        assert_eq!(model.control_targets().len(), 2);
        model.device_network.network_id = Some("signed-network".into());
        model.device_network.member_ids = vec![local, member];
        assert_eq!(
            model
                .control_targets()
                .iter()
                .map(|p| p.id.clone())
                .collect::<Vec<_>>(),
            vec![member.to_hex()]
        );
        assert!(model.clipboard_sync_enabled);
        model.device_network.member_ids = vec![member];
        assert!(
            model.control_targets().is_empty(),
            "a removed local member cannot reuse saved scene targets"
        );
        model.device_network.integrity_error = Some("invalid stored roster".into());
        model.device_network.network_id = None;
        assert!(model.control_targets().is_empty());
        model.input_sharing.settings.role = deskfuse_core::InputShareRole::Controlled;
        model.input_sharing.settings.authorized_controller = Some(member);
        assert!(!model.input_sharing.settings.enabled);
        assert!(model.clipboard_sync_enabled);
    }

    #[test]
    fn disabled_mouse_sharing_takes_priority_over_transitional_or_permission_reasons() {
        let mut model = AppModel::starting();
        let peer = DeviceId::from_public_key([2; 32]).unwrap();
        model
            .input_sharing
            .links
            .push(deskfuse_core::InputPeerStatus {
                motion_mode: None,
                motion_fallback_reason: None,
                peer_id: peer,
                context: None,
                ready: false,
                reason: Some("正在应用本机输入设置".into()),
                control_session: None,
                lease_revision: 0,
            });
        assert_eq!(
            model.input_availability_reason(&peer.to_hex()),
            "鼠标共享已关闭"
        );
        model.input_sharing.settings.enabled = true;
        model.input_sharing.links[0].reason = Some("requires a physical LAN interface".into());
        assert!(
            model
                .input_availability_reason(&peer.to_hex())
                .contains("真实局域网")
        );
        model.input_sharing.links[0].reason = Some("input permission unavailable".into());
        assert!(
            model
                .input_availability_reason(&peer.to_hex())
                .contains("系统权限")
        );
    }

    #[test]
    fn input_sharing_is_independent_and_defaults_off() {
        let model = AppModel::starting();
        assert!(model.clipboard_sync_enabled);
        assert!(!model.input_sharing.settings.enabled);
        assert_eq!(
            model.input_sharing.settings.switch_mode,
            deskfuse_core::EdgeSwitchMode::Instant
        );
        assert!(!model.input_locked);
    }

    #[test]
    fn clipboard_peer_trust_does_not_fabricate_input_readiness() {
        let mut model = AppModel::starting();
        model.input_sharing.settings.enabled = true;
        assert_eq!(
            model.input_availability_reason("missing"),
            "等待输入链路状态"
        );
    }

    fn snapshot_with_no_peers() -> NetworkSnapshot {
        let identity = MemoryIdentityProvider::new()
            .load_or_create()
            .expect("identity");
        NetworkSnapshot::new(0, identity.device_id(), Vec::new()).expect("snapshot")
    }

    #[test]
    fn empty_initial_real_snapshot_produces_zero_peers() {
        let snapshot = snapshot_with_no_peers();
        let model = AppModel::from_runtime_view(SnapshotView {
            device_network: Default::default(),
            snapshot,
            discovered_peers: Vec::new(),
        });
        assert!(model.peers.is_empty());
        assert!(model.discovered_peers.is_empty());
        assert_eq!(model.runtime_status, RuntimeStatus::Ready);
    }

    #[test]
    fn discovery_hints_never_become_authenticated_peers() {
        let identity = MemoryIdentityProvider::new()
            .load_or_create()
            .expect("identity");
        let hint = DiscoveredPeerHint {
            peer_id: DeviceId::from_bytes([7; 32]),
            display_name: "Nearby".to_owned(),
            capabilities: Capabilities::NETWORK,
            routes: vec![RouteCandidate::new(
                "192.0.2.1:42424".parse().expect("endpoint"),
                CoreRouteSource::Mdns,
                RouteHealth::Unknown,
                Capabilities::NETWORK,
            )],
        };
        let mut model = AppModel::from_runtime_view(SnapshotView {
            device_network: Default::default(),
            snapshot: NetworkSnapshot::new(0, identity.device_id(), Vec::new()).expect("snapshot"),
            discovered_peers: vec![hint],
        });
        assert!(model.peers.is_empty());
        assert_eq!(model.discovered_peers.len(), 1);
        assert_eq!(model.discovered_peers[0].trust, TrustState::Unknown);
        model.apply_runtime_event(RuntimeEvent::Error(
            deskfuse_runtime::RuntimeError::Transport("test".to_owned()),
        ));
        assert!(model.peers.is_empty());
    }

    #[test]
    fn authenticated_peer_suppresses_same_id_hint_and_removal_preserves_peer() {
        let local = MemoryIdentityProvider::new()
            .load_or_create()
            .expect("local identity");
        let remote = MemoryIdentityProvider::new()
            .load_or_create()
            .expect("remote identity");
        let route = RouteCandidate::new(
            "192.0.2.9:42424".parse().expect("endpoint"),
            CoreRouteSource::Mdns,
            RouteHealth::Healthy,
            Capabilities::NETWORK,
        );
        let authenticated_peer = PeerSnapshot::new(
            remote.device_id(),
            "Remote",
            remote.public_key_bytes(),
            LivenessState::Online,
            CoreTrustState::Trusted,
            ConnectionState::Connected,
            vec![route.clone()],
            Some(route.id),
        )
        .expect("peer");
        let hint = DiscoveredPeerHint {
            peer_id: remote.device_id(),
            display_name: "Remote nearby".to_owned(),
            capabilities: Capabilities::NETWORK,
            routes: vec![route],
        };
        let mut model = AppModel::from_runtime_view(SnapshotView {
            device_network: Default::default(),
            snapshot: NetworkSnapshot::new(1, local.device_id(), vec![authenticated_peer])
                .expect("snapshot"),
            discovered_peers: vec![hint.clone()],
        });
        assert_eq!(model.peers.len(), 1);
        assert!(model.discovered_peers.is_empty());

        model.apply_runtime_event(RuntimeEvent::DiscoveredPeerHint(hint));
        assert!(model.discovered_peers.is_empty());

        model.apply_runtime_event(RuntimeEvent::PeerHintRemoved {
            peer_id: remote.device_id(),
        });

        assert!(model.discovered_peers.is_empty());
        assert_eq!(model.peers.len(), 1);
        assert_eq!(model.peers[0].id, remote.device_id().to_hex());
        assert_eq!(model.peers[0].trust, TrustState::Trusted);
    }

    #[test]
    fn permission_snapshot_is_stored_without_inference() {
        let mut model = AppModel::starting();
        let snapshot = PermissionSnapshot::new(
            deskfuse_platform::PermissionStatus::Granted,
            deskfuse_platform::PermissionStatus::Denied,
            deskfuse_platform::PermissionStatus::Unknown,
        );
        model.apply_permission_snapshot(snapshot);
        assert_eq!(model.permissions, snapshot);
        let identity = MemoryIdentityProvider::new()
            .load_or_create()
            .expect("identity");
        model.apply_runtime_view(SnapshotView {
            device_network: Default::default(),
            snapshot: NetworkSnapshot::new(1, identity.device_id(), Vec::new()).expect("snapshot"),
            discovered_peers: Vec::new(),
        });
        assert_eq!(model.permissions, snapshot);
        assert_eq!(model.runtime_status, RuntimeStatus::Ready);
        assert_eq!(model.runtime_error, None);
    }

    #[test]
    fn runtime_error_preserves_last_snapshot_health() {
        let local = MemoryIdentityProvider::new()
            .load_or_create()
            .expect("local identity");
        let remote = MemoryIdentityProvider::new()
            .load_or_create()
            .expect("remote identity");
        let route = RouteCandidate::new(
            "192.0.2.8:42424".parse().expect("endpoint"),
            CoreRouteSource::Manual,
            RouteHealth::Healthy,
            Capabilities::NETWORK,
        );
        let peer = PeerSnapshot::new(
            remote.device_id(),
            "Remote",
            remote.public_key_bytes(),
            LivenessState::Online,
            CoreTrustState::Trusted,
            ConnectionState::Connected,
            vec![route.clone()],
            Some(route.id),
        )
        .expect("peer");
        let mut model = AppModel::from_runtime_view(SnapshotView {
            device_network: Default::default(),
            snapshot: NetworkSnapshot::new(1, local.device_id(), vec![peer]).expect("snapshot"),
            discovered_peers: Vec::new(),
        });
        assert_eq!(model.local.health, Health::Ready);
        assert_eq!(model.peers[0].health, Health::Ready);

        model.apply_runtime_event(RuntimeEvent::Error(
            deskfuse_runtime::RuntimeError::Discovery("mDNS unavailable".to_owned()),
        ));
        assert_eq!(model.runtime_status, RuntimeStatus::Error);
        assert_eq!(
            model.runtime_error.as_deref(),
            Some("discovery operation failed: mDNS unavailable")
        );
        assert_eq!(model.local.health, Health::Ready);
        assert_eq!(model.peers[0].health, Health::Ready);
    }

    #[test]
    fn offline_startup_error_is_sticky_until_explicit_recovery() {
        let mut model = AppModel::offline("runtime failed to start");
        model.apply_runtime_view(SnapshotView {
            device_network: Default::default(),
            snapshot: snapshot_with_no_peers(),
            discovered_peers: Vec::new(),
        });
        assert_eq!(model.runtime_status, RuntimeStatus::Error);
        assert_eq!(
            model.runtime_error.as_deref(),
            Some("runtime failed to start")
        );
        model.recover_runtime();
        assert_eq!(model.runtime_status, RuntimeStatus::Discovering);
        assert_eq!(model.runtime_error, None);
    }

    #[test]
    fn runtime_error_stays_sticky_across_snapshot_and_view_until_recovery() {
        let local = MemoryIdentityProvider::new()
            .load_or_create()
            .expect("local identity");
        let remote = MemoryIdentityProvider::new()
            .load_or_create()
            .expect("remote identity");
        let route = RouteCandidate::new(
            "192.0.2.22:42424".parse().expect("endpoint"),
            CoreRouteSource::Manual,
            RouteHealth::Healthy,
            Capabilities::NETWORK,
        );
        let peer = PeerSnapshot::new(
            remote.device_id(),
            "Remote",
            remote.public_key_bytes(),
            LivenessState::Online,
            CoreTrustState::Trusted,
            ConnectionState::Connected,
            vec![route.clone()],
            Some(route.id),
        )
        .expect("peer");
        let mut model = AppModel::from_runtime_view(SnapshotView {
            device_network: Default::default(),
            snapshot: NetworkSnapshot::new(1, local.device_id(), vec![peer.clone()])
                .expect("snapshot"),
            discovered_peers: Vec::new(),
        });
        model.apply_runtime_event(RuntimeEvent::Error(deskfuse_runtime::RuntimeError::Store(
            "trust store unavailable".to_owned(),
        )));
        let error = model.runtime_error.clone();

        model.apply_runtime_event(RuntimeEvent::Snapshot(
            NetworkSnapshot::new(2, local.device_id(), vec![peer]).expect("snapshot"),
        ));
        assert_eq!(model.runtime_status, RuntimeStatus::Error);
        assert_eq!(model.runtime_error, error);
        assert_eq!(model.local.health, Health::Ready);

        model.apply_runtime_view(SnapshotView {
            device_network: Default::default(),
            snapshot: NetworkSnapshot::new(3, local.device_id(), Vec::new()).expect("snapshot"),
            discovered_peers: Vec::new(),
        });
        assert_eq!(model.runtime_status, RuntimeStatus::Error);
        assert_eq!(model.runtime_error, error);
        assert_eq!(model.local.health, Health::Ready);

        model.recover_runtime();
        assert_eq!(model.runtime_status, RuntimeStatus::Discovering);
        assert_eq!(model.runtime_error, None);
    }

    #[test]
    fn discovery_hint_action_sends_invitation_without_local_approval() {
        let hint_id = DeviceId::from_bytes([8; 32]);
        let model = AppModel::from_runtime_view(SnapshotView {
            device_network: Default::default(),
            snapshot: snapshot_with_no_peers(),
            discovered_peers: vec![DiscoveredPeerHint {
                peer_id: hint_id,
                display_name: "Nearby".to_owned(),
                capabilities: Capabilities::NETWORK,
                routes: Vec::new(),
            }],
        });
        assert_eq!(
            model.command_connect(&hint_id.to_hex()),
            Some(RuntimeCommand::RequestPairing { peer_id: hint_id })
        );
        assert!(model.peers.is_empty());
        assert_eq!(model.discovered_peers[0].trust, TrustState::Unknown);
    }

    #[test]
    fn public_route_source_mapping_preserves_direct_and_relay() {
        assert_eq!(
            map_route_source(CoreRouteSource::PublicDirect),
            RouteSource::PublicDirect
        );
        assert_eq!(
            map_route_source(CoreRouteSource::PublicRelay),
            RouteSource::PublicRelay
        );
        assert_ne!(RouteSource::PublicDirect, RouteSource::PublicRelay);
        assert_ne!(
            RouteSource::PublicDirect.label(),
            RouteSource::PublicRelay.label()
        );
    }

    #[test]
    fn clipboard_sync_defaults_on_and_survives_runtime_refresh() {
        let mut model = AppModel::starting();
        assert!(model.clipboard_sync_enabled);

        let identity = MemoryIdentityProvider::new()
            .load_or_create()
            .expect("identity");
        model.apply_runtime_view(SnapshotView {
            device_network: Default::default(),
            snapshot: NetworkSnapshot::new(0, identity.device_id(), Vec::new()).expect("snapshot"),
            discovered_peers: Vec::new(),
        });
        assert!(model.clipboard_sync_enabled);
    }

    #[test]
    fn transfer_status_updates_one_queue_item_in_place() {
        let mut model = AppModel::starting();
        model.apply_transfer_status(TransferItem {
            id: "transfer-1".to_owned(),
            name: "Folder".to_owned(),
            detail: "正在发送".to_owned(),
            state: TransferState::Transferring,
            transferred_bytes: 1,
            total_bytes: 4,
            direction: TransferDirection::Send,
        });
        model.apply_transfer_status(TransferItem {
            id: "transfer-1".to_owned(),
            name: "Folder".to_owned(),
            detail: "已完成".to_owned(),
            state: TransferState::Completed,
            transferred_bytes: 4,
            total_bytes: 4,
            direction: TransferDirection::Send,
        });
        assert_eq!(model.transfer_queue.len(), 1);
        assert_eq!(model.transfer_queue[0].state, TransferState::Completed);
        assert_eq!(model.transfer_queue[0].transferred_bytes, 4);
    }

    #[test]
    fn command_mapping_is_explicit_and_runtime_owned() {
        let identity = MemoryIdentityProvider::new()
            .load_or_create()
            .expect("identity");
        let model = AppModel::from_runtime_view(SnapshotView {
            device_network: Default::default(),
            snapshot: NetworkSnapshot::new(0, identity.device_id(), Vec::new()).expect("snapshot"),
            discovered_peers: Vec::new(),
        });
        assert_eq!(model.command_refresh(), RuntimeCommand::RefreshPeers);
        assert_eq!(
            model.command_connect(&identity.device_id().to_hex()),
            Some(RuntimeCommand::RequestPairing {
                peer_id: identity.device_id()
            })
        );
        let request_id = RequestId::new();
        assert_eq!(
            model.command_approve(&request_id.to_string()),
            Some(RuntimeCommand::ApprovePairing { request_id })
        );
        assert_eq!(
            model.command_reject(&request_id.to_string()),
            Some(RuntimeCommand::RejectPairing {
                request_id,
                reason: Some("用户在 DeskFuse 中拒绝".to_owned())
            })
        );
        assert_eq!(
            model.command_forget(&identity.device_id().to_hex()),
            Some(RuntimeCommand::ForgetPeer {
                peer_id: identity.device_id()
            })
        );
    }

    #[test]
    fn peer_mapping_preserves_real_connection_and_trust() {
        let local = MemoryIdentityProvider::new()
            .load_or_create()
            .expect("identity");
        let remote = MemoryIdentityProvider::new()
            .load_or_create()
            .expect("remote");
        let route = RouteCandidate::new(
            "192.0.2.2:42424".parse().expect("endpoint"),
            CoreRouteSource::Manual,
            RouteHealth::Healthy,
            Capabilities::NETWORK,
        );
        let peer = PeerSnapshot::new(
            remote.device_id(),
            "Remote",
            remote.public_key_bytes(),
            LivenessState::Online,
            CoreTrustState::Trusted,
            ConnectionState::Connected,
            vec![route.clone()],
            Some(route.id),
        )
        .expect("peer");
        let model = AppModel::from_runtime_view(SnapshotView {
            device_network: Default::default(),
            snapshot: NetworkSnapshot::new(1, local.device_id(), vec![peer]).expect("snapshot"),
            discovered_peers: Vec::new(),
        });
        assert_eq!(model.peers[0].health, Health::Ready);
        assert_eq!(model.peers[0].trust, TrustState::Trusted);
        assert_eq!(model.peers[0].route_source, RouteSource::Manual);
    }
}
