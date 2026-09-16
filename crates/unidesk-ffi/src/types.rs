// UniFFI exports require owned arguments across the native language boundary.
#![allow(clippy::needless_pass_by_value)]
use unidesk_core as core;
use unidesk_engine::{model, state::EngineState};

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum EngineError {
    #[error("{message}")]
    Failed { message: String },
}
impl From<String> for EngineError {
    fn from(message: String) -> Self {
        Self::Failed { message }
    }
}

#[derive(Clone, Debug, uniffi::Record)]
#[allow(clippy::struct_excessive_bools)] // Independent device capability/status flags.
pub struct Device {
    pub id: String,
    pub name: String,
    pub detail: String,
    pub endpoint: String,
    pub routes: Vec<DeviceRoute>,
    pub trusted: bool,
    pub connected: bool,
    pub member: bool,
    pub blocked: bool,
    pub route_preference: RoutePreference,
    pub input_reason: String,
    /// The far side reclaimed input locally; a new enable cycle is required.
    pub input_restart_required: bool,
}
/// A discovered/cached route is a candidate, not proof of a live session.
#[derive(Clone, Debug, uniffi::Record)]
pub struct DeviceRoute {
    pub id: String,
    pub endpoint: String,
    pub source: String,
    pub status: String,
    pub active: bool,
    pub preferred: bool,
    pub failures: u32,
    pub path: String,
    pub rtt_ms: Option<u32>,
    pub last_probe_unix_ms: Option<u64>,
    pub probe_error: Option<String>,
}

fn device_routes(device: &model::Device, public_network_enabled: bool) -> Vec<DeviceRoute> {
    let connected = device.health == model::Health::Ready;
    let mut routes = device
        .routes
        .iter()
        .map(|route| {
            let selected = device.selected_route_id == Some(route.id);
            let active = connected && selected;
            DeviceRoute {
                id: route.id.to_string(),
                endpoint: route.endpoint.to_string(),
                source: match route.source {
                    core::RouteSource::Mdns => "Bonjour 发现",
                    core::RouteSource::Manual => "手动添加",
                    core::RouteSource::CachedDirect => "缓存地址",
                    core::RouteSource::PublicDirect => "公网直连",
                    core::RouteSource::PublicRelay => "公网中继",
                }
                .into(),
                status: if active {
                    "当前连接"
                } else if route.source.is_public() && !public_network_enabled {
                    "公网未启用"
                } else if route.probe_error.is_some() {
                    "探测失败"
                } else {
                    match route.health {
                        core::RouteHealth::Healthy => "最近可达",
                        core::RouteHealth::Unknown => "尚未探测",
                        core::RouteHealth::Backoff => "等待重试",
                        core::RouteHealth::Unavailable => "暂不可用",
                    }
                }
                .into(),
                active,
                // Runtime policy's next candidate is not necessarily the final
                // interface-ranked dial target, so the UI calls this a candidate.
                preferred: !connected && selected,
                failures: route.failure_count,
                path: route
                    .path_kind
                    .map_or("", core::InterfaceKind::label)
                    .into(),
                rtt_ms: route.rtt_ms,
                last_probe_unix_ms: route.last_probe_unix_ms,
                probe_error: route.probe_error.clone(),
            }
        })
        .collect::<Vec<_>>();
    routes.sort_by(|a, b| {
        b.active
            .cmp(&a.active)
            .then(b.preferred.cmp(&a.preferred))
            .then(a.endpoint.cmp(&b.endpoint))
            .then(a.id.cmp(&b.id))
    });
    let mut deduped = Vec::<DeviceRoute>::new();
    for route in routes {
        if let Some(existing) = deduped
            .iter_mut()
            .find(|item| item.endpoint == route.endpoint)
        {
            existing.failures = existing.failures.max(route.failures);
            if existing.path.is_empty() {
                existing.path = route.path;
            }
            if route.last_probe_unix_ms > existing.last_probe_unix_ms {
                existing.rtt_ms = route.rtt_ms;
                existing.last_probe_unix_ms = route.last_probe_unix_ms;
                existing.probe_error = route.probe_error;
                if !existing.active {
                    existing.status = route.status;
                }
            } else if existing.last_probe_unix_ms.is_none() && route.last_probe_unix_ms.is_none() {
                existing.rtt_ms = match (existing.rtt_ms, route.rtt_ms) {
                    (Some(left), Some(right)) => Some(left.min(right)),
                    (None, value) | (value, None) => value,
                };
            }
            continue;
        }
        deduped.push(route);
    }
    deduped
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct Pairing {
    pub id: String,
    pub peer_id: String,
    pub name: String,
    pub incoming: bool,
    pub expires_at_ms: u64,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct HistoryItem {
    pub id: u64,
    pub source_app_id: Option<String>,
    pub source_device_id: Option<String>,
    pub copied_at_ms: Option<u64>,
    pub mime: String,
    pub length: u64,
    pub preview: String,
    pub source: String,
    pub has_image: bool,
    pub file_count: u32,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ClipboardContent {
    pub can_request_files: bool,
    pub id: u64,
    pub text: String,
    pub truncated: bool,
    pub image_png: Vec<u8>,
    pub file_paths: Vec<String>,
}
#[derive(Clone, Debug, uniffi::Enum)]
pub enum TransferPhase {
    Queued,
    Sending,
    Receiving,
    Verifying,
    Delivering,
    Completed,
    Failed,
    Cancelled,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct Transfer {
    pub id: String,
    pub name: String,
    pub detail: String,
    pub phase: TransferPhase,
    pub transferred: u64,
    pub total: u64,
    pub outgoing: bool,
    pub can_retry: bool,
    pub saved: u64,
    pub save_total: u64,
}
#[derive(Clone, Debug, uniffi::Record)]
#[allow(clippy::struct_excessive_bools)] // Independent file policy switches.
pub struct FileSettings {
    pub prefetch: bool,
    pub threshold: u64,
    pub maximum: u64,
}
impl From<FileSettings> for core::FileTransferSettings {
    fn from(s: FileSettings) -> Self {
        Self {
            small_file_prefetch_enabled: s.prefetch,
            small_file_threshold_bytes: s.threshold,
            cache_max_bytes: s.maximum,
        }
    }
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct CacheStatus {
    pub used: u64,
    pub reserved: u64,
    pub maximum: u64,
    pub active: u64,
    pub completed: u64,
    pub message: String,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct Permission {
    pub kind: PermissionKind,
    pub name: String,
    pub granted: bool,
    pub status: String,
    pub settings_url: String,
}
#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum PermissionKind {
    Accessibility,
    FullDiskAccess,
    InputMonitoring,
}
impl From<PermissionKind> for unidesk_platform::PermissionKind {
    fn from(k: PermissionKind) -> Self {
        match k {
            PermissionKind::Accessibility => Self::Accessibility,
            PermissionKind::FullDiskAccess => Self::FullDiskAccess,
            PermissionKind::InputMonitoring => Self::InputMonitoring,
        }
    }
}
#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum RoutePreference {
    Automatic,
    Thunderbolt,
    Ethernet,
    Wifi,
}
impl From<RoutePreference> for core::RoutePreference {
    fn from(p: RoutePreference) -> Self {
        match p {
            RoutePreference::Automatic => Self::Automatic,
            RoutePreference::Thunderbolt => Self::Thunderbolt,
            RoutePreference::Ethernet => Self::Ethernet,
            RoutePreference::Wifi => Self::WiFi,
        }
    }
}
impl From<core::RoutePreference> for RoutePreference {
    fn from(p: core::RoutePreference) -> Self {
        match p {
            core::RoutePreference::Automatic => Self::Automatic,
            core::RoutePreference::Thunderbolt => Self::Thunderbolt,
            core::RoutePreference::Ethernet => Self::Ethernet,
            core::RoutePreference::WiFi => Self::Wifi,
        }
    }
}
#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum SwitchMode {
    Instant,
    Dwell,
    DoubleTap,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct Shortcut {
    pub key_code: u16,
    pub modifiers: u64,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct Placement {
    pub device_id: String,
    pub display_id: String,
    pub x: f64,
    pub y: f64,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct InputSettings {
    pub enabled: bool,
    pub controller: bool,
    pub participants: Vec<String>,
    pub switch_mode: SwitchMode,
    pub disabled_corners: u8,
    pub corner_size: u16,
    pub placements: Vec<Placement>,
    pub shortcuts: Vec<Shortcut>,
}
impl InputSettings {
    /// # Errors
    /// Returns an error for invalid device IDs, shortcut count, display placements or input settings.
    pub fn into_core(
        self,
        previous: &core::InputSharingSettings,
    ) -> Result<core::InputSharingSettings, EngineError> {
        let mut s = previous.clone();
        s.enabled = self.enabled;
        s.role = if self.controller {
            core::InputShareRole::Controller
        } else {
            core::InputShareRole::Disabled
        };
        s.participants = self
            .participants
            .into_iter()
            .map(|s| s.parse().map_err(|_| "设备 ID 无效".to_string().into()))
            .collect::<Result<_, EngineError>>()?;
        s.switch_mode = match self.switch_mode {
            SwitchMode::Instant => core::EdgeSwitchMode::Instant,
            SwitchMode::Dwell => core::EdgeSwitchMode::Dwell250Ms,
            SwitchMode::DoubleTap => core::EdgeSwitchMode::DoubleTap500Ms,
        };
        s.disabled_corners = self.disabled_corners;
        s.corner_size = self.corner_size;
        s.placements = self
            .placements
            .into_iter()
            .map(|p| {
                Ok(core::DisplayPlacementSetting {
                    display: core::DisplayKey {
                        device_id: p
                            .device_id
                            .parse()
                            .map_err(|_| EngineError::from("设备 ID 无效".to_string()))?,
                        display_id: p.display_id.into(),
                    },
                    x: p.x,
                    y: p.y,
                })
            })
            .collect::<Result<_, EngineError>>()?;
        let [left, right, up, down, lock]: [Shortcut; 5] = self
            .shortcuts
            .try_into()
            .map_err(|_| EngineError::from("快捷键配置需要五个方向/锁定按键".to_string()))?;
        let key = |s: Shortcut| core::InputShortcut {
            key_code: s.key_code,
            modifiers: s.modifiers,
        };
        s.shortcuts = core::InputShortcutBindings {
            left: key(left),
            right: key(right),
            up: key(up),
            down: key(down),
            lock: key(lock),
        };
        s.validate().map_err(|e| EngineError::from(e.to_string()))?;
        Ok(s)
    }
}
impl From<&core::InputSharingSettings> for InputSettings {
    fn from(s: &core::InputSharingSettings) -> Self {
        Self {
            enabled: s.enabled,
            controller: s.role == core::InputShareRole::Controller,
            participants: s.participants.iter().map(ToString::to_string).collect(),
            switch_mode: match s.switch_mode {
                core::EdgeSwitchMode::Instant => SwitchMode::Instant,
                core::EdgeSwitchMode::Dwell250Ms => SwitchMode::Dwell,
                core::EdgeSwitchMode::DoubleTap500Ms => SwitchMode::DoubleTap,
            },
            disabled_corners: s.disabled_corners,
            corner_size: s.corner_size,
            placements: s
                .placements
                .iter()
                .map(|p| Placement {
                    device_id: p.display.device_id.to_string(),
                    display_id: p.display.display_id.to_string(),
                    x: p.x,
                    y: p.y,
                })
                .collect(),
            shortcuts: [
                &s.shortcuts.left,
                &s.shortcuts.right,
                &s.shortcuts.up,
                &s.shortcuts.down,
                &s.shortcuts.lock,
            ]
            .into_iter()
            .map(|k| Shortcut {
                key_code: k.key_code,
                modifiers: k.modifiers,
            })
            .collect(),
        }
    }
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct Display {
    pub id: String,
    pub device_id: String,
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub scale: f64,
    pub rotation: u16,
    pub local: bool,
    pub ready: bool,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct Scene {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct EnvironmentRule {
    pub ssid: String,
    pub controller: String,
}
#[derive(Clone, Debug, uniffi::Record)]
#[allow(clippy::struct_excessive_bools)] // Independent native status flags.
pub struct InputState {
    pub settings: InputSettings,
    pub summary: String,
    pub error: Option<String>,
    pub managed: bool,
    pub managed_paused: bool,
    pub can_configure: bool,
    pub locked: bool,
    pub recording: bool,
    pub displays: Vec<Display>,
    pub scenes: Vec<Scene>,
    pub environment_message: String,
    pub ssid: Option<String>,
    pub environment_conflict: bool,
    pub candidates: Vec<String>,
    pub rules: Vec<EnvironmentRule>,
}
#[derive(Clone, Debug, uniffi::Record)]
#[allow(clippy::struct_excessive_bools)] // Independent native status flags.
pub struct UpdateState {
    pub github: bool,
    pub release_url: Option<String>,
    pub version: String,
    pub build: u64,
    pub checking: bool,
    pub available: bool,
    pub manual_requested: bool,
    pub ready: bool,
    pub downloading: bool,
    pub received_bytes: u64,
    pub total_bytes: u64,
    pub target_version: Option<String>,
    pub message: String,
    pub error: Option<String>,
}
#[derive(Clone, Debug, uniffi::Record)]
#[allow(clippy::struct_excessive_bools)] // Independent UI state flags in an FFI record.
pub struct EngineSnapshot {
    pub debug_logging_enabled: bool,
    pub revision: u64,
    pub local_id: String,
    pub local_name: String,
    pub runtime_status: String,
    pub error: Option<String>,
    pub network_id: Option<String>,
    pub devices: Vec<Device>,
    pub pairing: Vec<Pairing>,
    pub pairing_message: Option<String>,
    pub clipboard_enabled: bool,
    pub clipboard_error: Option<String>,
    pub history: Vec<HistoryItem>,
    pub transfers: Vec<Transfer>,
    pub files: FileSettings,
    pub cache: CacheStatus,
    pub permissions: Vec<Permission>,
    pub input: InputState,
    pub update: UpdateState,
    pub public_network_enabled: bool,
}
#[derive(Clone, Debug, uniffi::Enum)]
pub enum EngineCommand {
    SetDebugLogging {
        enabled: bool,
    },
    RefreshDevices,
    Pair {
        peer_id: String,
    },
    PairAt {
        ip: String,
    },
    Approve {
        request_id: String,
    },
    Reject {
        request_id: String,
    },
    Forget {
        peer_id: String,
    },
    AllowPairing {
        peer_id: String,
    },
    Disconnect {
        peer_id: String,
    },
    Connect {
        peer_id: String,
    },
    LeaveNetwork,
    SetRoute {
        peer_id: String,
        preference: RoutePreference,
    },
    SetPublicNetwork {
        enabled: bool,
    },
    SetClipboard {
        enabled: bool,
    },
    PushClipboard,
    RestoreHistory {
        id: u64,
    },
    ClearHistory,
    SendFiles {
        peer_id: String,
        paths: Vec<String>,
    },
    CancelTransfer {
        id: String,
    },
    RetryTransfer {
        id: String,
    },
    SetFiles {
        settings: FileSettings,
    },
    ClearCache,
    RefreshCache,
    MoveInputDevice {
        device_id: String,
        display_id: String,
        x: f64,
        y: f64,
    },
    SetInput {
        settings: InputSettings,
    },
    /// Resume one device after a local reclaim without toggling the switch.
    RestartInputActivation {
        peer_id: String,
    },
    SaveScene {
        name: String,
    },
    ApplyScene {
        id: String,
    },
    RenameScene {
        id: String,
        name: String,
    },
    DeleteScene {
        id: String,
    },
    ChooseController {
        id: String,
        remember: bool,
    },
    ForgetEnvironment {
        ssid: String,
    },
    CheckUpdates,
    DownloadAndInstallUpdate,
    RefreshPermissions,
}
#[derive(Clone, Debug, uniffi::Enum)]
#[allow(clippy::large_enum_variant)] // UniFFI exports value records, not Box<record>.
pub enum EngineEvent {
    WorkflowCatalogChanged,
    WorkflowRun {
        run: crate::WorkflowRun,
    },
    Snapshot {
        snapshot: EngineSnapshot,
    },
    /// The worker applied the command; remote operations still have their own task/pairing state.
    CommandApplied {
        request_id: u64,
    },
    CommandFailed {
        request_id: u64,
        message: String,
    },
    PairingFailed {
        request_id: String,
        message: String,
    },
    PasteReady {
        target_pid: i32,
        change_count: i64,
    },
    StageFiles {
        paths: Vec<String>,
    },
    Quit,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct CommandReceipt {
    pub request_id: u64,
    pub transfer_id: Option<String>,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ExportLease {
    pub id: String,
    pub paths: Vec<String>,
}
#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum ExportOutcome {
    Copied,
    Moved,
    Cancelled,
    Failed,
}

impl From<&EngineState> for EngineSnapshot {
    #[allow(clippy::too_many_lines)] // Single field-by-field native snapshot projection.
    fn from(s: &EngineState) -> Self {
        let m = &s.model;
        let input = &m.input_sharing;
        let local = m.local.id.clone();
        let devices = m
            .network_graph_peers()
            .into_iter()
            .chain(m.discovered_peers.iter())
            .fold(Vec::<Device>::new(), |mut devices, d| {
                if d.id != local && !devices.iter().any(|x| x.id == d.id) {
                    devices.push(Device {
                        id: d.id.clone(),
                        name: d.name.clone(),
                        detail: d.subtitle.clone(),
                        endpoint: d.endpoint.clone(),
                        routes: device_routes(d, m.public_network_enabled),
                        trusted: matches!(d.trust, model::TrustState::Trusted),
                        connected: matches!(d.health, model::Health::Ready),
                        member: m.is_network_member(&d.id),
                        blocked: d
                            .id
                            .parse::<core::DeviceId>()
                            .is_ok_and(|id| m.device_network.blocked_peer_ids.contains(&id)),
                        route_preference: d
                            .id
                            .parse::<core::DeviceId>()
                            .ok()
                            .and_then(|id| m.device_network.route_preferences.get(&id))
                            .copied()
                            .unwrap_or_default()
                            .into(),
                        input_reason: m.input_availability_reason(&d.id),
                        input_restart_required: m.input_restart_required(&d.id),
                    });
                }
                devices
            });
        let displays = local
            .parse::<core::DeviceId>()
            .ok()
            .filter(|_| input.can_configure_layout())
            .map(|local_id| {
                unidesk_input::layout::layout_displays(input, local_id, false)
                    .into_iter()
                    .map(|screen| {
                        let id = screen.key.device_id.to_string();
                        Display {
                            id: screen.key.display_id.to_string(),
                            device_id: id.clone(),
                            name: if id == local {
                                m.local.name.clone()
                            } else {
                                devices
                                    .iter()
                                    .find(|p| p.id == id)
                                    .map_or_else(|| id.clone(), |p| p.name.clone())
                            },
                            x: screen.x,
                            y: screen.y,
                            width: f64::from(screen.display.width),
                            height: f64::from(screen.display.height),
                            scale: screen.display.scale_factor,
                            rotation: screen.display.rotation_degrees,
                            local: id == local,
                            ready: screen.ready,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self {
            revision: s.revision,
            debug_logging_enabled: s.debug_logging_enabled,
            local_id: local,
            local_name: m.local.name.clone(),
            runtime_status: m.runtime_status.label().into(),
            error: m.runtime_error.clone(),
            network_id: m.device_network.network_id.clone(),
            devices,
            pairing: m
                .device_network
                .pending
                .iter()
                .map(|p| Pairing {
                    id: p.request_id.to_string(),
                    peer_id: p.peer_id.to_string(),
                    name: p.display_name.clone(),
                    incoming: p.incoming,
                    expires_at_ms: p.expires_at_unix_ms,
                })
                .collect(),
            pairing_message: m.pairing_status.clone(),
            clipboard_enabled: m.clipboard_sync_enabled,
            clipboard_error: m.clipboard_error.clone(),
            history: m
                .clipboard_history
                .iter()
                .map(|h| HistoryItem {
                    id: h.id,
                    source_app_id: h.source_app_id.clone(),
                    source_device_id: None, // Legacy clipboard history doesn't track origin device
                    copied_at_ms: h.copied_at_ms,
                    mime: h.mime.clone(),
                    length: h.length as u64,
                    preview: h.preview.clone(),
                    source: h.source.clone(),
                    has_image: h.image_png.is_some(),
                    file_count: u32::try_from(h.file_paths.len()).unwrap_or(u32::MAX),
                })
                .collect(),
            transfers: m
                .transfer_queue
                .iter()
                .filter(|t| {
                    !matches!(t.direction, model::TransferDirection::Receive)
                        || !s.file_paste_sources.values().any(|source| source == &t.id)
                })
                .map(|t| Transfer {
                    id: t.id.clone(),
                    name: t.name.clone(),
                    detail: t.detail.clone(),
                    phase: match t.state {
                        model::TransferState::Preparing => TransferPhase::Queued,
                        model::TransferState::Transferring => {
                            if matches!(t.direction, model::TransferDirection::Send) {
                                TransferPhase::Sending
                            } else {
                                TransferPhase::Receiving
                            }
                        }
                        model::TransferState::Verifying => TransferPhase::Verifying,
                        model::TransferState::Delivering => TransferPhase::Delivering,
                        model::TransferState::Completed => TransferPhase::Completed,
                        model::TransferState::Failed => TransferPhase::Failed,
                        model::TransferState::Cancelled => TransferPhase::Cancelled,
                    },
                    transferred: s
                        .file_paste_sources
                        .get(&t.id)
                        .and_then(|id| m.transfer_queue.iter().find(|source| &source.id == id))
                        .map_or(t.transferred_bytes, |source| source.transferred_bytes),
                    total: s
                        .file_paste_sources
                        .get(&t.id)
                        .and_then(|id| m.transfer_queue.iter().find(|source| &source.id == id))
                        .map_or(t.total_bytes, |source| source.total_bytes),
                    outgoing: matches!(t.direction, model::TransferDirection::Send),
                    can_retry: matches!(
                        t.state,
                        model::TransferState::Failed | model::TransferState::Cancelled
                    ) && (matches!(t.direction, model::TransferDirection::Send)
                        || s.file_paste_retryable.contains(&t.id)),
                    saved: if s.file_paste_delivering.contains(&t.id) {
                        t.transferred_bytes
                    } else {
                        0
                    },
                    save_total: if s.file_paste_delivering.contains(&t.id) {
                        t.total_bytes
                    } else {
                        0
                    },
                })
                .collect(),
            files: FileSettings {
                prefetch: s.file_settings.small_file_prefetch_enabled,
                threshold: s.file_settings.small_file_threshold_bytes,
                maximum: s.file_settings.cache_max_bytes,
            },
            cache: CacheStatus {
                used: s.cache.used_bytes,
                reserved: s.cache.active_reserved_bytes,
                maximum: s.cache.max_bytes,
                active: s.cache.active_transfers as u64,
                completed: s.cache.completed_transfers as u64,
                message: s.cache_message.clone().unwrap_or_default(),
            },
            permissions: [
                (PermissionKind::Accessibility, "辅助功能"),
                (PermissionKind::FullDiskAccess, "完全磁盘访问"),
            ]
            .into_iter()
            .map(|(kind, name)| {
                let k: unidesk_platform::PermissionKind = kind.into();
                let status = m.permissions.status(k);
                Permission {
                    kind,
                    name: name.into(),
                    granted: status.is_granted(),
                    status: if status.is_granted() {
                        "已授权"
                    } else {
                        "未授权或需要操作"
                    }
                    .into(),
                    settings_url: k.system_settings_url().into(),
                }
            })
            .collect(),
            input: InputState {
                settings: (&input.settings).into(),
                summary: m.input_sharing_summary(),
                error: m.input_error.clone(),
                managed: input.managed_control.is_some(),
                managed_paused: input
                    .managed_control
                    .as_ref()
                    .is_some_and(|a| a.status == core::InputActivationStatus::PausedByUser),
                can_configure: input.can_configure_layout(),
                locked: m.input_locked,
                recording: s.recording,
                displays,
                scenes: input
                    .scenes
                    .iter()
                    .map(|s| Scene {
                        id: s.id.clone(),
                        name: s.name.clone(),
                    })
                    .collect(),
                environment_message: input.environment.message.clone(),
                ssid: input.environment.ssid.clone(),
                environment_conflict: input.environment.conflict,
                candidates: input
                    .environment
                    .candidates
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
                rules: input
                    .environment
                    .rules
                    .iter()
                    .map(|(ssid, id)| EnvironmentRule {
                        ssid: ssid.clone(),
                        controller: id.to_string(),
                    })
                    .collect(),
            },
            update: UpdateState {
                github: m.updates.github.is_some(),
                release_url: m
                    .updates
                    .github
                    .as_ref()
                    .and_then(|github| github.release_url.clone()),
                version: m.updates.installed.as_ref().map_or_else(
                    || env!("CARGO_PKG_VERSION").into(),
                    |i| i.version.to_string(),
                ),
                build: m.updates.installed.as_ref().map_or(0, |i| i.build_number),
                checking: m.updates.checking,
                available: m.updates.installed.is_some(),
                manual_requested: m.updates.manual_requested,
                ready: m.updates.status.as_ref().is_some_and(|s| {
                    matches!(
                        s.phase,
                        unidesk_update::UpdatePhase::WaitingForIdle
                            | unidesk_update::UpdatePhase::Staged
                    )
                }),
                downloading: m.updates.checking
                    && m.updates.status.as_ref().is_some_and(|s| {
                        matches!(
                            s.phase,
                            unidesk_update::UpdatePhase::Downloading
                                | unidesk_update::UpdatePhase::Verifying
                        )
                    }),
                received_bytes: m.updates.status.as_ref().map_or(0, |s| s.received_bytes),
                total_bytes: m.updates.status.as_ref().map_or(0, |s| s.artifact.size),
                target_version: m
                    .updates
                    .github
                    .as_ref()
                    .and_then(|github| github.target_version.clone())
                    .or_else(|| {
                        m.updates.status.as_ref().map(|s| {
                            format!("{} · build {}", s.release.version, s.release.build_number)
                        })
                    }),
                message: m.updates.message.clone(),
                error: m.updates.error.clone(),
            },
            public_network_enabled: m.public_network_enabled,
        }
    }
}

/// Canvas-only transient translation. Committing a move is a separate engine command.
#[derive(Clone, Debug, uniffi::Record)]
pub struct LayoutOffset {
    pub x: f64,
    pub y: f64,
}

#[uniffi::export]
/// # Errors
/// Returns an error for invalid device/display identifiers or invalid layout geometry.
pub fn preview_display_move(
    displays: Vec<Display>,
    device_id: String,
    delta_x: f64,
    delta_y: f64,
    snap_distance: f64,
    attach: bool,
) -> Result<LayoutOffset, EngineError> {
    let device = device_id
        .parse::<core::DeviceId>()
        .map_err(|_| "无效的设备 ID".to_string())?;
    let rects = displays
        .into_iter()
        .map(|d| {
            Ok(unidesk_input::layout::LayoutRect {
                key: core::DisplayKey {
                    device_id: d
                        .device_id
                        .parse()
                        .map_err(|_| "无效的设备 ID".to_string())?,
                    display_id: core::DisplayId::new(d.id),
                },
                x: d.x,
                y: d.y,
                width: d.width,
                height: d.height,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let (x, y) = unidesk_input::layout::snap_device_move(
        &rects,
        device,
        delta_x,
        delta_y,
        if attach { None } else { Some(snap_distance) },
    )?;
    Ok(LayoutOffset { x, y })
}
