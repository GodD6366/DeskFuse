// UniFFI exports require owned arguments across the native language boundary.
#![allow(clippy::needless_pass_by_value)]
use deskfuse_core as core;
use deskfuse_engine::{model, state::EngineState};

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
    /// 未在线时的简短原因（已手动断开 / 对方已断开 / 等待对方确认），否则为空。
    pub link_status: String,
    pub input_reason: String,
    /// The far side reclaimed input locally; a new enable cycle is required.
    pub input_restart_required: bool,
    pub remote_app_rdp: bool,
    /// The selected live route negotiated support for remote update triggers.
    pub can_trigger_update: bool,
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

/// 两端连接状态的说明：(列表里的短状态, 详情行的完整说明)。
fn link_hold_text(
    network: &deskfuse_runtime::DeviceNetworkSnapshot,
    device_id: &str,
) -> Option<(&'static str, &'static str)> {
    let id = device_id.parse::<core::DeviceId>().ok()?;
    let hold = network.link_holds.iter().find(|hold| hold.peer_id == id)?;
    Some(match hold.kind {
        deskfuse_runtime::PeerLinkHoldKind::LocalSuspended => (
            "已手动断开",
            "已在本机手动断开，不会自动重连；点击“连接”恢复",
        ),
        deskfuse_runtime::PeerLinkHoldKind::RemoteSuspended => (
            "对方已断开",
            "对方设备已手动断开与本机的连接，需在对方设备上点击“连接”",
        ),
        deskfuse_runtime::PeerLinkHoldKind::AwaitingPeer => {
            ("等待对方确认", "本机已连上对方，正在等待对方回连确认")
        }
    })
}

fn device_routes(device: &model::Device) -> Vec<DeviceRoute> {
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
    pub enabled: bool,
    pub prefetch: bool,
    pub threshold: u64,
    pub maximum: u64,
    pub download_directory: String,
}
impl From<FileSettings> for core::FileTransferSettings {
    fn from(s: FileSettings) -> Self {
        Self {
            enabled: s.enabled,
            small_file_prefetch_enabled: s.prefetch,
            small_file_threshold_bytes: s.threshold,
            cache_max_bytes: s.maximum,
            download_directory: s.download_directory,
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
impl From<PermissionKind> for deskfuse_platform::PermissionKind {
    fn from(k: PermissionKind) -> Self {
        match k {
            PermissionKind::Accessibility => Self::Accessibility,
            PermissionKind::FullDiskAccess => Self::FullDiskAccess,
            PermissionKind::InputMonitoring => Self::InputMonitoring,
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
    pub feature_enabled: bool,
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
        s.feature_enabled = self.feature_enabled;
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
        let [left, right, up, down, lock, emergency]: [Shortcut; 6] =
            self.shortcuts.try_into().map_err(|_| {
                EngineError::from("快捷键配置需要方向、锁定和紧急收回共六组按键".to_string())
            })?;
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
            emergency: key(emergency),
        };
        s.validate().map_err(|e| EngineError::from(e.to_string()))?;
        Ok(s)
    }
}
#[cfg(test)]
mod input_shortcut_tests {
    use super::*;

    #[test]
    fn six_shortcut_dto_round_trips_custom_emergency() {
        let original = core::InputSharingSettings::default();
        let mut dto = InputSettings::from(&original);
        assert_eq!(dto.shortcuts.len(), 6);
        assert_eq!(dto.shortcuts[5].key_code, 53);
        dto.shortcuts[5] = Shortcut {
            key_code: 14,
            modifiers: 0x0c0000,
        };
        let updated = dto.clone().into_core(&original).unwrap();
        assert_eq!(updated.shortcuts.emergency.key_code, 14);
        assert_eq!(updated.shortcuts.emergency.modifiers, 0x0c0000);
        assert_eq!(updated.shortcuts.left, original.shortcuts.left);
        dto.shortcuts.pop();
        assert!(dto.into_core(&original).is_err());
    }
}

impl From<&core::InputSharingSettings> for InputSettings {
    fn from(s: &core::InputSharingSettings) -> Self {
        Self {
            feature_enabled: s.feature_enabled,
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
                &s.shortcuts.emergency,
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
    pub wallpaper_jpeg: Option<Vec<u8>>,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct Scene {
    pub id: String,
    pub name: String,
    /// When set, this scene is the remembered layout of that Wi-Fi network and
    /// is applied automatically when the stable environment returns to it.
    pub ssid: Option<String>,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct EnvironmentRule {
    pub ssid: String,
    pub controller: String,
}
/// 对端经输入链路上报的 Wi-Fi 标识。
#[derive(Clone, Debug, uniffi::Record)]
pub struct PeerSsid {
    pub device_id: String,
    pub ssid: String,
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
    pub can_operate_remote_applications: bool,
    pub locked: bool,
    pub recording: bool,
    pub displays: Vec<Display>,
    pub scenes: Vec<Scene>,
    pub environment_message: String,
    pub ssid: Option<String>,
    /// 本机实际读到的 SSID，`ssid` 则是场景判定采用的值（可能来自对端）。
    pub observed_ssid: Option<String>,
    pub peer_ssids: Vec<PeerSsid>,
    pub environment_conflict: bool,
    pub candidates: Vec<String>,
    pub rules: Vec<EnvironmentRule>,
    pub quality: InputQuality,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct InputQuality {
    pub grade: String,
    pub rtt_p50_ms: Option<f64>,
    pub rtt_p95_ms: Option<f64>,
    pub rtt_max_ms: Option<f64>,
    pub hitches_per_minute: f64,
    pub slow_injections: u32,
    pub samples: Vec<InputQualitySample>,
    pub suggestions: Vec<InputQualitySuggestion>,
    /// 最近一次采样时数据报运动通道是否就绪。为假表示运动已降级到可靠通道；
    /// 为空表示窗口里还没有采样。它是「为什么曲线不动」的唯一线索，因为降级
    /// 后鼠标照常可用，只有这里能看出通道换过。
    pub datagram_ready: Option<bool>,
}

/// 一条可执行、可复测的优化建议。
#[derive(Clone, Debug, uniffi::Record)]
pub struct InputQualitySuggestion {
    /// 稳定标识，复测时用它对照。
    pub id: String,
    /// `act` 需要处理、`info` 只是说明、`ok` 无需动作。
    pub priority: String,
    /// 触发这条建议的指标。
    pub trigger: String,
    /// 用户可以做的动作。
    pub action: String,
    /// 期望改善的指标。
    pub expected: String,
    /// 操作后如何再测。
    pub retest: String,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct InputQualitySample {
    /// Seconds from the beginning of the rolling 60-second window.
    pub second: i64,
    pub rtt_ms: Option<f64>,
    /// 这一秒的卡顿总数，等于网络与本机采集两类之和。
    pub hitches: u32,
    /// 这一秒网络路径造成的卡顿。
    pub hitch_network: u32,
    /// 这一秒本机采集排队造成的卡顿。
    pub hitch_capture: u32,
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
    Block {
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
    PublishDisplayWallpaper {
        display_id: String,
        jpeg: Vec<u8>,
    },
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
    /// Create or refresh the layout bound to the current Wi-Fi network.
    SaveEnvironmentLayout {
        name: Option<String>,
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
    TriggerPeerUpdate {
        peer_id: String,
    },
    TriggerPeerUpdates {
        peer_ids: Vec<String>,
    },
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
    ClipboardLibraryChanged {
        revision: u64,
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

fn input_quality_cutoff(now_ms: u128) -> u128 {
    (now_ms / 1_000 * 1_000).saturating_sub(60_000)
}

impl From<&EngineState> for EngineSnapshot {
    #[allow(clippy::too_many_lines)] // Single field-by-field native snapshot projection.
    fn from(s: &EngineState) -> Self {
        let m = &s.model;
        let input = &m.input_sharing;
        let quality_records = s.input_quality.as_ref().map_or_else(
            deskfuse_transport::motion_diag::quality_snapshot,
            |quality| quality.records.clone(),
        );
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        // Align buckets to wall-clock seconds so refreshing the snapshot cannot
        // regroup historical samples and change their median.
        let cutoff = input_quality_cutoff(now_ms);
        let mut quality = deskfuse_platform::timing::InputQualityWindow::default();
        let mut observed_seconds = std::collections::BTreeSet::new();
        let mut samples = std::collections::BTreeMap::<i64, (Vec<f64>, u32, u32)>::new();
        let mut max_rtt_ms = None;
        // 记录按时间追加，循环走到最后就是最新一次采样的通道状态。
        let mut datagram_ready = None;
        for record in quality_records
            .iter()
            .filter(|record| (cutoff..=now_ms).contains(&record.timestamp_ms))
        {
            datagram_ready = Some(record.ready);
            let second = (record.timestamp_ms.saturating_sub(cutoff) / 1_000) as i64;
            let sample = samples.entry(second).or_default();
            if record.rtt_micros > 0 {
                let rtt_ms = record.rtt_micros as f64 / 1_000.0;
                sample.0.push(rtt_ms);
                max_rtt_ms = Some(max_rtt_ms.map_or(rtt_ms, |max: f64| max.max(rtt_ms)));
            } else {
                sample.0.push(0.0);
            }
            if observed_seconds.insert(record.timestamp_ms / 1_000) {
                quality.observe_motion_second();
            }
            if record.rtt_micros > 0 {
                quality.observe_rtt(record.rtt_micros as f64 / 1_000.0);
            }
            quality.observe_slow_injections(record.slow_injections);
            let hitch_count = u64::from(record.hitches);
            sample.1 = sample.1.saturating_add(hitch_count.min(32) as u32);
            for _ in 0..hitch_count.min(32) {
                quality.observe_hitch(
                    deskfuse_platform::timing::HitchSource::Network,
                    (record.rtt_micros as f64 / 1_000.0).max(16.0),
                );
            }
            // 采集排队平均达到 8 ms 才算一次本机采集卡顿，与慢注入同阈值。
            let average_capture_micros = record
                .capture_queue_micros
                .checked_div(record.capture_count)
                .unwrap_or(0);
            if average_capture_micros >= 8_000 {
                sample.2 = sample.2.saturating_add(1);
                quality.observe_hitch(
                    deskfuse_platform::timing::HitchSource::Capture,
                    average_capture_micros as f64 / 1_000.0,
                );
            }
        }
        // Idle observations fill only seconds without input-link samples. They
        // are graph continuity, not packet timings or hitch statistics.
        for &(timestamp_ms, rtt_ms) in &s.idle_quality_samples {
            if (cutoff..=now_ms).contains(&timestamp_ms) {
                let second = (timestamp_ms.saturating_sub(cutoff) / 1_000) as i64;
                samples
                    .entry(second)
                    .or_insert_with(|| (vec![rtt_ms], 0, 0));
            }
        }
        let quality = quality.snapshot();
        // 建议直接来自诊断报告，避免 Swift 再复制一份阈值规则。
        let lease_link = input
            .links
            .iter()
            .find(|link| link.control_session.is_some());
        let quality_report = s.input_quality.as_ref().map_or_else(
            || {
                deskfuse_input::quality::build_report(
                    &deskfuse_core::quality_ticks(),
                    lease_link
                        .and_then(|link| link.motion_mode.as_deref())
                        .unwrap_or_default(),
                    lease_link.and_then(|link| link.motion_fallback_reason.as_deref()),
                )
            },
            |quality| quality.report.clone(),
        );
        let suggestions = quality_report
            .suggestions
            .iter()
            .map(|item| InputQualitySuggestion {
                id: item.id.clone(),
                priority: item.priority.clone(),
                trigger: item.trigger.clone(),
                action: item.action.clone(),
                expected: item.expected.clone(),
                retest: item.retest.clone(),
            })
            .collect();
        let samples = samples
            .into_iter()
            .map(|(second, (mut rtts, hitch_network, hitch_capture))| {
                rtts.sort_by(f64::total_cmp);
                let rtt_ms = (!rtts.is_empty()).then(|| rtts[rtts.len() / 2]);
                InputQualitySample {
                    second,
                    rtt_ms,
                    hitches: hitch_network.saturating_add(hitch_capture),
                    hitch_network,
                    hitch_capture,
                }
            })
            .collect();
        let local = m.local.id.clone();
        let mut devices = m
            .network_graph_peers()
            .into_iter()
            .chain(m.discovered_peers.iter())
            .fold(Vec::<Device>::new(), |mut devices, d| {
                if d.id != local && !devices.iter().any(|x| x.id == d.id) {
                    let hold = link_hold_text(&m.device_network, &d.id);
                    devices.push(Device {
                        id: d.id.clone(),
                        name: d.name.clone(),
                        detail: hold
                            .map_or_else(|| d.subtitle.clone(), |(_, note)| note.to_owned()),
                        endpoint: d.endpoint.clone(),
                        routes: device_routes(d),
                        trusted: matches!(d.trust, model::TrustState::Trusted),
                        connected: matches!(d.health, model::Health::Ready),
                        member: m.is_network_member(&d.id),
                        blocked: d
                            .id
                            .parse::<core::DeviceId>()
                            .is_ok_and(|id| m.device_network.blocked_peer_ids.contains(&id)),
                        link_status: hold
                            .map(|(status, _)| status.to_owned())
                            .unwrap_or_default(),
                        input_reason: m.input_availability_reason(&d.id),
                        input_restart_required: m.input_restart_required(&d.id),
                        remote_app_rdp: d.routes.iter().any(|route| {
                            Some(route.id) == d.selected_route_id
                                && route
                                    .capabilities
                                    .contains(core::Capabilities::REMOTE_APPLICATION_RDP)
                        }),
                        can_trigger_update: d.routes.iter().any(|route| {
                            Some(route.id) == d.selected_route_id
                                && route
                                    .capabilities
                                    .contains(core::Capabilities::LAN_UPDATE_TRIGGER)
                        }),
                    });
                }
                devices
            });
        // 已连接排在前面；同组再按名称，避免刷新时跳动。
        devices.sort_by(|a, b| {
            b.connected
                .cmp(&a.connected)
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| a.id.cmp(&b.id))
        });
        let displays = local
            .parse::<core::DeviceId>()
            .ok()
            .filter(|_| input.can_configure_layout())
            .map(|local_id| {
                deskfuse_input::layout::layout_displays(input, local_id, false)
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
                            wallpaper_jpeg: input
                                .remote_wallpapers
                                .get(&screen.key.device_id)
                                .and_then(|images| images.get(&screen.key.display_id))
                                .cloned(),
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
                enabled: s.file_settings.enabled,
                prefetch: s.file_settings.small_file_prefetch_enabled,
                threshold: s.file_settings.small_file_threshold_bytes,
                maximum: s.file_settings.cache_max_bytes,
                download_directory: s.file_settings.download_directory.clone(),
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
                let k: deskfuse_platform::PermissionKind = kind.into();
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
                can_operate_remote_applications: input.can_operate_remote_applications(),
                locked: m.input_locked,
                recording: s.recording,
                displays,
                scenes: input
                    .scenes
                    .iter()
                    .map(|s| Scene {
                        id: s.id.clone(),
                        name: s.name.clone(),
                        ssid: s.ssid.clone(),
                    })
                    .collect(),
                environment_message: input.environment.message.clone(),
                ssid: input.environment.ssid.clone(),
                observed_ssid: input.environment.observed_ssid.clone(),
                peer_ssids: input
                    .environment
                    .peer_ssids
                    .iter()
                    .map(|(id, ssid)| PeerSsid {
                        device_id: id.to_string(),
                        ssid: ssid.clone(),
                    })
                    .collect(),
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
                quality: InputQuality {
                    grade: format!("{:?}", quality.grade).to_lowercase(),
                    rtt_p50_ms: quality.rtt_p50_ms,
                    rtt_p95_ms: quality.rtt_p95_ms,
                    rtt_max_ms: max_rtt_ms,
                    hitches_per_minute: quality.hitches_per_minute,
                    slow_injections: quality.slow_injections,
                    samples,
                    suggestions,
                    datagram_ready,
                },
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
                        deskfuse_update::UpdatePhase::WaitingForIdle
                            | deskfuse_update::UpdatePhase::Staged
                    )
                }),
                downloading: m.updates.checking
                    && m.updates.status.as_ref().is_some_and(|s| {
                        matches!(
                            s.phase,
                            deskfuse_update::UpdatePhase::Downloading
                                | deskfuse_update::UpdatePhase::Verifying
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
            Ok(deskfuse_input::layout::LayoutRect {
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
    let (x, y) = deskfuse_input::layout::snap_device_move(
        &rects,
        device,
        delta_x,
        delta_y,
        if attach { None } else { Some(snap_distance) },
    )?;
    Ok(LayoutOffset { x, y })
}

#[cfg(test)]
mod input_quality_tests {
    use super::*;

    #[test]
    fn curve_buckets_only_move_horizontally_at_second_boundaries() {
        let records = [65_100u128, 65_900, 66_100];
        let buckets = |now| records.map(|at| (at - input_quality_cutoff(now)) / 1_000);
        assert_eq!(buckets(70_100), [55, 55, 56]);
        assert_eq!(buckets(70_999), [55, 55, 56]);
        assert_eq!(buckets(71_000), [54, 54, 55]);
    }

    fn now() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    }

    #[test]
    fn idle_curve_keeps_observed_seconds_without_faking_packet_statistics() {
        let mut state = EngineState {
            input_quality: Some(Default::default()),
            ..Default::default()
        };
        let start = now().saturating_sub(3_000);
        for second in 0..4 {
            state.sample_idle_input_quality(start + second * 1_000);
        }
        let quality = EngineSnapshot::from(&state).input.quality;
        assert_eq!(quality.samples.len(), 4);
        assert!(
            quality
                .samples
                .iter()
                .all(|sample| sample.rtt_ms == Some(0.0))
        );
        assert_eq!(quality.rtt_p50_ms, None);
        assert_eq!(quality.hitches_per_minute, 0.0);
        assert_eq!(quality.grade, "idle");
        assert_eq!(state.revision, 4);
    }

    #[test]
    fn service_rtt_and_injection_counts_reach_ui_even_on_reliable_only_link() {
        let mut state = EngineState::default();
        let at = now().saturating_sub(100);
        state.sample_idle_input_quality(at);
        assert!(state.apply(
            deskfuse_engine::tokio_bridge::UiEvent::InputQualitySnapshot(
                deskfuse_runtime::input_service_types::InputServiceQuality {
                    records: vec![deskfuse_transport::motion_diag::QualityRecord {
                        timestamp_ms: at,
                        rtt_micros: 7_500,
                        hitches: 1,
                        slow_injections: 2,
                        capture_queue_micros: 9_000,
                        capture_count: 1,
                        ready: false,
                    }],
                    report: deskfuse_input::quality::build_report(
                        &[],
                        "reliable-only",
                        Some("test fallback")
                    ),
                },
            )
        ));
        let quality = EngineSnapshot::from(&state).input.quality;
        assert_eq!(quality.samples.len(), 1);
        assert_eq!(quality.samples[0].rtt_ms, Some(7.5));
        assert_eq!(quality.samples[0].hitch_network, 1);
        assert_eq!(quality.samples[0].hitch_capture, 1);
        assert_eq!(quality.slow_injections, 2);
        assert_eq!(quality.rtt_p50_ms, Some(7.5));
        assert_eq!(quality.datagram_ready, Some(false));
        assert!(!quality.suggestions.is_empty());
    }

    #[test]
    fn idle_history_is_bounded_and_uses_only_the_connected_selected_route() {
        let mut state = EngineState {
            input_quality: Some(Default::default()),
            ..Default::default()
        };
        let mut peer = state.model.local.clone();
        peer.id = deskfuse_core::DeviceId::from_bytes([77; 32]).to_hex();
        peer.trust = model::TrustState::Trusted;
        peer.health = model::Health::Ready;
        let mut active = deskfuse_core::RouteCandidate::new(
            deskfuse_core::Endpoint::new("127.0.0.1", 42425).unwrap(),
            deskfuse_core::RouteSource::Mdns,
            deskfuse_core::RouteHealth::Healthy,
            deskfuse_core::Capabilities::INPUT_SHARING,
        );
        active.rtt_ms = Some(12);
        peer.selected_route_id = Some(active.id);
        let mut unused = active.clone();
        unused.id = deskfuse_core::RouteId::new();
        unused.rtt_ms = Some(999);
        peer.routes = vec![active, unused];
        state.model.peers.push(peer);
        let start = now().saturating_sub(65_000);
        for second in 0..66 {
            state.sample_idle_input_quality(start + second * 1_000);
        }
        assert_eq!(state.idle_quality_samples.len(), 60);
        let quality = EngineSnapshot::from(&state).input.quality;
        assert!(!quality.samples.is_empty());
        assert!(
            quality
                .samples
                .iter()
                .all(|sample| sample.rtt_ms == Some(12.0))
        );
        state.model.peers[0].health = model::Health::Offline;
        state.sample_idle_input_quality(now());
        assert_eq!(state.idle_quality_samples.back().unwrap().1, 0.0);
    }
}
