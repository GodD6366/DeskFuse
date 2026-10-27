//! Authenticated desktop coordinator; shared state never contains private keys.
use super::{
    Control, Network, clipboard_sync, input_security, lan_web, remote_app, security, session_agent,
    workflow,
};
use crate::{
    desktop_protocol as ipc,
    policy::{ClientSession, SessionPolicy, same_session},
    protocol,
};
use base64::Engine;
use deskfuse_core::{ConnectionState, DeviceId, Endpoint, FileTransferSettings, TrustState};
use deskfuse_platform::{
    BoundaryErrorCode, NativeRevision, PlatformCommand, PlatformEvent,
    ports::{ApplyResult, CapturedContent, ContentData},
};
use deskfuse_runtime::{RuntimeCommand, RuntimeEvent};
use deskfuse_transfer::FileBridge;
use std::{
    collections::VecDeque,
    io,
    net::Ipv4Addr,
    os::windows::io::AsRawHandle,
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    net::windows::named_pipe::NamedPipeServer,
    sync::mpsc,
    time::timeout,
};
use windows_sys::Win32::Foundation::HANDLE;
#[derive(Default)]
pub(super) struct State {
    owner: Option<ClientSession>,
    epoch: u64,
    heartbeat: Option<Instant>,
    local_sequence: u64,
    published_sequence: u64,
    sync: clipboard_sync::Sync,
    pairing_status: Option<String>,
    last_error: Option<String>,
    input_heartbeat: Option<Instant>,
    input_helper: Option<input_security::HelperIdentity>,
    input_desktop: ipc::InputDesktop,
    input_topology: Option<deskfuse_core::DisplayTopology>,
    input_deliveries: VecDeque<ipc::InputDelivery>,
    input_sequence: u64,
    /// 已交给 helper 的最高投递序号。推送和轮询共用它，避免同一批输入被执行两次。
    input_delivered: u64,
    /// 持有当前 helper 长连接的连接编号；0 表示没有可推送的连接。
    input_link: u64,
    /// 当前这一代 helper 声明过能收推送帧。重连后同进程的 helper 不必重新声明，
    /// 推送不会因为一次传输抖动永久降级。
    input_push_capable: bool,
    input_reported_activation: Option<String>,
    input_last_lease: Option<(String, u64)>,
    controlling: Option<ipc::ControlView>,
    session_agent: session_agent::SessionAgent,
    file_bridge: Option<FileBridge>,
    file_settings: FileTransferSettings,
    lan_web: Option<lan_web::LanWebService>,
    library: Option<std::sync::Arc<deskfuse_engine::clipboard_library::Library>>,
    library_sync: Option<deskfuse_engine::library_sync::Sync>,
    workflow_sync_at: Option<Instant>,
    /// Last host-reported capture. This is metadata for typed receipts only;
    /// native clipboard access remains in the interactive WPF process.
    platform_capture: Option<CapturedContent>,
}
impl State {
    pub(super) fn clear(&mut self) {
        crate::native::file_transfer::set_delivery_enabled(false);
        if let Some(files) = self.file_bridge.take() {
            files.reset(true);
        }
        if let Some(lan_web) = self.lan_web.take() {
            lan_web.pause_session();
        }
        *self = Self::default();
    }
    fn apply_file_settings(&mut self, settings: FileTransferSettings) {
        crate::native::file_transfer::set_delivery_enabled(settings.enabled);
        let disabling = self.file_settings.enabled && !settings.enabled;
        self.file_settings = settings;
        if disabling && let Some(files) = self.file_bridge.as_ref() {
            files.reset(true);
        }
    }
    fn active(&self, control: &Control) -> bool {
        !control.stopping.load(Ordering::Acquire)
            && self.epoch == control.epoch.load(Ordering::Acquire)
            && self
                .heartbeat
                .is_some_and(|instant| instant.elapsed() < Duration::from_secs(5))
            && self.owner.is_some_and(|owner| {
                security::session_still_active(owner, control.session_policy())
            })
    }
    fn helper_available(&self) -> bool {
        self.input_helper
            .as_ref()
            .is_some_and(|helper| helper.alive())
            && self.input_desktop == ipc::InputDesktop::Default
            && self
                .input_heartbeat
                .is_some_and(|instant| instant.elapsed() < Duration::from_secs(1))
            // Input injection never follows the relaxed policy: the unlocked
            // physical console remains mandatory regardless of user switches.
            && self
                .owner
                .is_some_and(|owner| security::session_still_active(owner, SessionPolicy::STRICT))
    }
    fn input_active(&self, control: &Control) -> bool {
        self.active(control) && self.helper_available()
    }
    fn push_input(&mut self, delivery: impl FnOnce(u64) -> ipc::InputDelivery) {
        self.input_sequence = self.input_sequence.saturating_add(1).max(1);
        if self.input_deliveries.len() >= 512 {
            self.input_deliveries.clear();
            self.input_sequence = self.input_sequence.saturating_add(1);
            self.input_deliveries
                .push_back(ipc::InputDelivery::Release {
                    sequence: self.input_sequence,
                    reason: "input helper queue overflow; all held input released".into(),
                });
            return;
        }
        self.input_deliveries
            .push_back(delivery(self.input_sequence));
    }
    fn release_input(&mut self, reason: impl Into<String>) {
        let reason = reason.into();
        self.input_deliveries.clear();
        self.push_input(|sequence| ipc::InputDelivery::Release { sequence, reason });
        self.input_last_lease = None;
        self.controlling = None;
    }
    /// helper 还没收到的投递。推送只是快路径：helper 的轮询仍然按
    /// `after_sequence` 拿同一批数据，所以丢掉一帧推送不会丢输入。
    fn input_push_batch(&self) -> Option<(Vec<ipc::InputDelivery>, u64)> {
        let deliveries = self
            .input_deliveries
            .iter()
            .filter(|delivery| delivery.sequence() > self.input_delivered)
            .take(64)
            .cloned()
            .collect::<Vec<_>>();
        let sequence = deliveries.last().map(ipc::InputDelivery::sequence)?;
        Some((deliveries, sequence))
    }
    fn commit_input_push(&mut self, sequence: u64) {
        self.input_delivered = self.input_delivered.max(sequence);
    }
    fn input_state(&self, network: &Network) -> ipc::InputState {
        let input = network.input_runtime.handle.input_snapshot();
        let managed = input.managed_control.as_ref();
        let runtime_snapshot = network.runtime.snapshot_view().snapshot;
        let activations = input
            .activations
            .iter()
            .map(|(peer_id, activation)| ipc::ActivationView {
                peer_id: peer_id.to_string(),
                status: enum_string(&activation.status),
                reason: activation.reason.clone(),
            })
            .collect();
        let remote_topologies = input
            .remote_topologies
            .iter()
            .map(|(peer_id, topology)| ipc::RemoteTopologyView {
                peer_id: peer_id.to_string(),
                topology: topology.clone(),
            })
            .collect();
        let links = input
            .links
            .iter()
            .map(|link| ipc::LinkView {
                peer_id: link.peer_id.to_string(),
                ready: link.ready,
                control_session: link.control_session.map(|id| id.to_string()),
                lease_revision: link.lease_revision,
                motion_mode: link.motion_mode.clone(),
                motion_fallback_reason: link.motion_fallback_reason.clone(),
            })
            .collect();
        let active = input
            .links
            .iter()
            .find(|link| link.control_session.is_some());
        let mode = active
            .and_then(|link| link.motion_mode.clone())
            .unwrap_or_default();
        let fallback = active.and_then(|link| link.motion_fallback_reason.clone());
        deskfuse_core::fold_pending_stages();
        let quality = deskfuse_input::quality::current_report(&mode, fallback.as_deref());
        let session_agent = self.session_agent.snapshot();
        ipc::InputState {
            helper_connected: self.helper_available(),
            security: ipc::InputSecurityState {
                session_bound: self.helper_available(),
                ui_access: self
                    .helper_available()
                    && self.input_helper.as_ref().is_some_and(|h| h.ui_access),
                elevated: self
                    .helper_available()
                    && self.input_helper.as_ref().is_some_and(|h| h.elevated),
                desktop: if self.helper_available() {
                    self.input_desktop
                } else {
                    ipc::InputDesktop::Unavailable
                },
                reason: Some(if self.helper_available() {
                    "UIAccess verified; Default desktop only; secure/login desktops and SAS unsupported"
                } else {
                    "No live authenticated UIAccess helper on the unlocked Default desktop"
                }.into()),
                session_agent,
            },
            managed: managed.is_some(),
            status: managed.map_or_else(|| "standby".into(), |value| enum_string(&value.status)),
            reason: managed.and_then(|value| value.reason.clone()),
            controller_id: managed.map(|value| value.controller.to_string()),
            display_count: input
                .local_topology
                .as_ref()
                .map_or(0, |topology| topology.displays.len()),
            local_device_id: runtime_snapshot.local_device_id.to_string(),
            settings: Some(input.settings.clone()),
            controlling: self.controlling.clone(),
            activations,
            remote_topologies,
            remote_wallpapers: input.remote_wallpapers.iter().map(|(peer, displays)| {
                (peer.to_string(), displays.iter().map(|(display, jpeg)| {
                    (display.0.clone(), base64::engine::general_purpose::STANDARD.encode(jpeg))
                }).collect())
            }).collect(),
            links,
            environment: ipc::EnvironmentView {
                ssid: input.environment.ssid.clone(),
                message: input.environment.message.clone(),
                conflict: input.environment.conflict,
                observed_ssid: input.environment.observed_ssid.clone(),
                peer_ssids: input
                    .environment
                    .peer_ssids
                    .iter()
                    .map(|(device_id, ssid)| ipc::PeerSsidView {
                        device_id: device_id.to_string(),
                        ssid: ssid.clone(),
                    })
                    .collect(),
                rules: input
                    .environment
                    .rules
                    .iter()
                    .map(|(ssid, controller)| ipc::EnvironmentRuleView {
                        ssid: ssid.clone(),
                        controller: controller.to_string(),
                    })
                    .collect(),
            },
            scenes: input
                .scenes
                .iter()
                .map(|scene| ipc::SceneView {
                    id: scene.id.clone(),
                    name: scene.name.clone(),
                    ssid: scene.ssid.clone(),
                })
                .collect(),
            quality,
        }
    }
    pub(super) fn input_event(
        &mut self,
        event: RuntimeEvent,
        network: &Network,
        control: &Control,
    ) {
        if !self.input_active(control) {
            if let RuntimeEvent::SharedInputReceived { session_id, .. } = event {
                let _ = network
                    .input_runtime
                    .handle
                    .try_send(RuntimeCommand::ReleaseInputControl { session_id });
            }
            return;
        }
        match event {
            RuntimeEvent::SharedInputReceived {
                session_id,
                target_display,
                state_epoch,
                events,
                ..
            } => self.push_input(|sequence| ipc::InputDelivery::Batch {
                sequence,
                session_id: session_id.to_string(),
                target_display: target_display.to_string(),
                state_epoch,
                events,
            }),
            RuntimeEvent::InputControlGranted {
                session_id,
                lease_expires_at: _,
                ..
            } => self.push_input(|sequence| ipc::InputDelivery::ControlGranted {
                sequence,
                session_id: session_id.to_string(),
                lease_ms: 1_500,
            }),
            RuntimeEvent::InputControlDenied {
                session_id, reason, ..
            } => {
                if self
                    .controlling
                    .as_ref()
                    .is_some_and(|control| control.session_id == session_id.to_string())
                {
                    self.controlling = None;
                }
                self.push_input(|sequence| ipc::InputDelivery::ControlDenied { sequence, reason });
            }
            RuntimeEvent::InputControlReleased {
                session_id, reason, ..
            } => {
                if self
                    .controlling
                    .as_ref()
                    .is_some_and(|control| control.session_id == session_id.to_string())
                {
                    self.controlling = None;
                }
                self.push_input(|sequence| ipc::InputDelivery::ControlReleased {
                    sequence,
                    session_id: session_id.to_string(),
                    reason,
                });
            }
            RuntimeEvent::InputDeliveryInvalidated { reason }
            | RuntimeEvent::InputLinkUnavailable { reason, .. } => self.release_input(reason),
            _ => {}
        }
    }
    pub(super) fn tick(&mut self, network: &Network, control: &Control) {
        if !self.active(control) {
            if let Some(managed) = network
                .input_runtime
                .handle
                .input_snapshot()
                .managed_control
                && managed.status != deskfuse_core::InputActivationStatus::Unavailable
            {
                let _ = network.input_runtime.handle.try_send(
                    RuntimeCommand::SetManagedInputAvailability {
                        activation_id: managed.activation_id,
                        status: deskfuse_core::InputActivationStatus::Unavailable,
                        reason: Some("Windows interactive desktop session unavailable".into()),
                    },
                );
            }
            self.clear();
            return;
        }
        self.sync.tick(&network.runtime);
        if let Some(library_sync) = &mut self.library_sync {
            library_sync.refresh_silent(&network.runtime);
        }
        if self
            .workflow_sync_at
            .is_none_or(|at| at.elapsed() >= Duration::from_secs(5))
        {
            if let Ok(frames) = network.workflow_catalog.frames() {
                let peers = network.runtime.snapshot_view().snapshot.peers;
                for peer in peers.into_iter().filter(|peer| {
                    peer.connection == deskfuse_core::ConnectionState::Connected
                        && peer.routes.iter().any(|route| {
                            route
                                .capabilities
                                .contains(deskfuse_core::Capabilities::WORKFLOW_SYNC)
                        })
                }) {
                    for frame in &frames {
                        let _ = network.runtime.try_send(RuntimeCommand::SendWorkflowSync {
                            peer_id: peer.id,
                            payload: frame.clone(),
                        });
                    }
                }
            }
            self.workflow_sync_at = Some(Instant::now());
        }
        if self.file_settings.enabled
            && let Some(files) = self.file_bridge.as_ref()
        {
            let connected = self.sync.file_peers(&network.runtime);
            files.tick(&connected);
        }
        let input_active = self.input_active(control);
        let snapshot = network.input_runtime.handle.input_snapshot();
        if input_active {
            if let Some(topology) = self.input_topology.clone()
                && snapshot.local_topology.as_ref() != Some(&topology)
            {
                let _ = network
                    .input_runtime
                    .handle
                    .try_send(RuntimeCommand::PublishDisplayTopology(topology));
            }
            if let Some(managed) = snapshot.managed_control.as_ref()
                && managed.status != deskfuse_core::InputActivationStatus::PausedByUser
                && snapshot.local_topology.is_some()
            {
                let activation = managed.activation_id.to_string();
                if self.input_reported_activation.as_deref() != Some(&activation) {
                    let _ = network.input_runtime.handle.try_send(
                        RuntimeCommand::SetManagedInputAvailability {
                            activation_id: managed.activation_id,
                            status: deskfuse_core::InputActivationStatus::Ready,
                            reason: None,
                        },
                    );
                    self.input_reported_activation = Some(activation);
                }
                if let Some(link) = snapshot.links.iter().find(|link| {
                    link.peer_id == managed.controller
                        && link.ready
                        && link.control_session.is_some()
                }) {
                    let session = link.control_session.expect("filtered control session");
                    let identity = (session.to_string(), link.lease_revision);
                    if self.input_last_lease.as_ref() != Some(&identity) {
                        self.push_input(|sequence| ipc::InputDelivery::Lease {
                            sequence,
                            session_id: session.to_string(),
                            lease_revision: link.lease_revision,
                        });
                        self.input_last_lease = Some(identity);
                    }
                }
            } else {
                self.input_reported_activation = None;
                self.input_last_lease = None;
            }
        } else if let Some(reported) = self.input_reported_activation.take() {
            if let Some(managed) = snapshot.managed_control.as_ref()
                && managed.activation_id.to_string() == reported
            {
                let _ = network.input_runtime.handle.try_send(
                    RuntimeCommand::SetManagedInputAvailability {
                        activation_id: managed.activation_id,
                        status: deskfuse_core::InputActivationStatus::Unavailable,
                        reason: Some("Windows input helper disconnected".into()),
                    },
                );
            }
            self.release_input("Windows input helper disconnected");
        }
    }
    pub(super) fn event(&mut self, event: RuntimeEvent, network: &Network, control: &Control) {
        if !self.active(control) {
            self.clear();
            return;
        }
        match event {
            RuntimeEvent::ClipboardReceived { peer_id, payload } => {
                if payload.mime == deskfuse_engine::library_sync::MIME {
                    if let Some(library_sync) = &mut self.library_sync {
                        library_sync.refresh_silent(&network.runtime);
                        library_sync.receive(peer_id, &payload.data);
                    }
                } else if let Some(files) = self.file_bridge.as_ref() {
                    self.sync.receive(
                        &network.runtime,
                        peer_id,
                        payload,
                        files,
                        self.file_settings.enabled,
                    )
                }
            }
            RuntimeEvent::WorkflowSyncReceived { peer_id, payload } => {
                match network.workflow_catalog.receive(&payload) {
                    Ok(frames) => {
                        for frame in frames {
                            let _ = network.runtime.try_send(RuntimeCommand::SendWorkflowSync {
                                peer_id,
                                payload: frame,
                            });
                        }
                    }
                    Err(error) => self.last_error = Some(error.chars().take(256).collect()),
                }
            }
            RuntimeEvent::PairingStatus { .. } => {
                self.pairing_status =
                    Some("pairing state changed; inspect pending invitations".into())
            }
            RuntimeEvent::PairingRequestSent { .. } => {
                self.pairing_status = Some("invitation sent; awaiting recipient decision".into())
            }
            RuntimeEvent::PairingFinished { outcome, .. } => {
                self.pairing_status = Some(outcome.chars().take(256).collect())
            }
            RuntimeEvent::PairingDecisionFailed { .. } => {
                self.last_error = Some("pairing decision failed; refresh and retry".into())
            }
            RuntimeEvent::Error(error) => {
                // 运行时给出的拒绝原因通常是可执行的（例如“只能在主控电脑上调整
                // 屏幕布局”）。统一替换成一句通用文案，界面就只会显示“网络失败”，
                // 用户既不知道发生了什么，也没法修。
                self.last_error = Some(error.to_string());
            }
            RuntimeEvent::ClipboardSent {
                result: Err(error),
                sync_id: Some(_),
                ..
            } => {
                // Handshake/ack frames (sync_id None) retry in the background;
                // only a real content-frame failure is actionable for the user.
                self.last_error = Some(
                    format!("clipboard transport failed ({error}); copy again after reconnection")
                        .chars()
                        .take(256)
                        .collect(),
                )
            }
            _ => {}
        }
    }
    pub(super) fn lagged(&mut self) {
        // A lost event may invalidate ordering, so retire all content and handshakes.
        self.sync = clipboard_sync::Sync::default();
        if let Some(files) = self.file_bridge.take() {
            files.reset(true);
        }
        self.last_error = Some("event stream overflow; clipboard session reset; copy again".into());
    }
    fn snapshot(
        &mut self,
        network: &Network,
        auth: ClientSession,
        epoch: u64,
        policy: SessionPolicy,
    ) -> ipc::Snapshot {
        let view = network.runtime.snapshot_view();
        let mut peers = view
            .snapshot
            .peers
            .iter()
            .map(|peer| ipc::Peer {
                peer_id: peer.id.to_string(),
                display_name: peer.display_name.clone(),
                trust: enum_string(&peer.trust),
                connection: enum_string(&peer.connection),
                discovered_only: false,
                selected_route: peer.selected_route.map(|id| id.to_string()),
                routes: peer
                    .routes
                    .iter()
                    .filter_map(|r| serde_json::to_value(r).ok())
                    .collect(),
            })
            .collect::<Vec<_>>();
        for hint in view.discovered_peers.iter() {
            if peers.iter().any(|p| p.peer_id == hint.peer_id.to_string()) {
                continue;
            }
            peers.push(ipc::Peer {
                peer_id: hint.peer_id.to_string(),
                display_name: hint.display_name.clone(),
                trust: "unknown".into(),
                connection: "disconnected".into(),
                discovered_only: true,
                selected_route: None,
                routes: hint
                    .routes
                    .iter()
                    .filter_map(|r| serde_json::to_value(r).ok())
                    .collect(),
            });
        }
        let pending = view
            .device_network
            .pending
            .iter()
            .map(|p| ipc::Pending {
                request_id: p.request_id.to_string(),
                peer_id: p.peer_id.to_string(),
                display_name: p.display_name.clone(),
                incoming: p.incoming,
                expires_at_unix_ms: p.expires_at_unix_ms,
                owner_name: p.owner_name.clone(),
                member_names: p.member_names.clone(),
            })
            .collect();
        ipc::Snapshot {
            package_version: env!("CARGO_PKG_VERSION").into(),
            session_epoch: epoch,
            client_session_id: auth.pipe_session,
            sequence: view.snapshot.sequence,
            library_revision: self.library.as_ref().map(|library| library.generation()),
            local_device_id: view.snapshot.local_device_id.to_string(),
            local_name: "DeskFuse Windows".into(),
            network_running: view.snapshot.sequence > 0
                && network.runtime.subscribe().has_changed().is_ok(),
            control_port: network.control_port,
            capabilities: ipc::Capabilities {
                clipboard: self.owner == Some(auth) && self.epoch == epoch,
                files: self.owner == Some(auth)
                    && self.epoch == epoch
                    && self.file_settings.enabled
                    && self.file_bridge.is_some()
                    && !self.sync.file_peers(&network.runtime).is_empty(),
                input: self.helper_available()
                    && self
                        .input_heartbeat
                        .is_some_and(|instant| instant.elapsed() < Duration::from_secs(1)),
                elevated_input: self.helper_available()
                    && self.input_helper.as_ref().is_some_and(|h| h.ui_access),
                secure_desktop_input: self.session_agent.secure_desktop_input_available(),
                login_input: self.session_agent.login_input_available(),
                secure_attention_sequence: self.session_agent.secure_attention_sequence_available(),
                updater: false,
                remote_app: matches!(
                    remote_app::availability(),
                    remote_app::Availability::Available
                ),
                workflow_execution: self.owner == Some(auth) && self.epoch == epoch,
                workflow_sync: self.owner == Some(auth) && self.epoch == epoch,
            },
            input: self.input_state(network),
            peers,
            pending,
            file_settings: self.file_settings.clone(),
            lan_web: self.lan_web.as_ref().map_or_else(
                || ipc::LanWebState::default(),
                lan_web::LanWebService::state,
            ),
            file_transfers: self
                .file_bridge
                .as_ref()
                .map_or_else(Vec::new, FileBridge::views),
            file_cache: self.file_bridge.as_ref().map_or(
                ipc::FileCache {
                    max_bytes: 0,
                    used_bytes: 0,
                },
                |bridge| {
                    let usage = bridge.cache_usage();
                    ipc::FileCache {
                        max_bytes: usage.max_bytes,
                        used_bytes: usage.used_bytes,
                    }
                },
            ),
            pairing_status: self
                .sync
                .status
                .clone()
                .or_else(|| self.pairing_status.clone()),
            // Deliver each error once; the client notifies on non-empty values.
            last_error: self.last_error.take().or_else(|| {
                view.device_network
                    .integrity_error
                    .as_ref()
                    .map(|_| "device network integrity check failed".into())
            }),
            session_policy: ipc::SessionPolicyView {
                allow_locked: policy.allow_locked,
                allow_remote: policy.allow_remote,
            },
            session_full_presence: auth.fully_present(),
        }
    }
    fn publish_file_data_object(
        &mut self,
        network: &Network,
        paths: Vec<String>,
        target_peer_id: Option<String>,
    ) -> Result<String, String> {
        if !self.file_settings.enabled {
            return Err("文件传输已关闭".into());
        }
        let Some(files) = self.file_bridge.as_ref() else {
            return Err("文件通道尚未就绪".into());
        };
        let mut peers = self.sync.file_peers(&network.runtime);
        if let Some(target) = target_peer_id.as_deref() {
            peers.retain(|peer| peer.to_string() == target);
            if peers.is_empty() {
                return Err("目标设备不在线或不支持文件通道".into());
            }
        }
        let frames = files.publish_local(
            &peers,
            paths.into_iter().map(std::path::PathBuf::from).collect(),
        )?;
        let mut first_id = None;
        for (peer, id, frame) in frames {
            first_id.get_or_insert_with(|| id.to_hex());
            let data = frame.encode().map_err(|error| error.to_string())?;
            self.sync.announce_file_offer(&network.runtime, peer, data);
        }
        first_id.ok_or_else(|| "没有已连接的受信任文件设备".into())
    }

    fn platform_command(&mut self, command: PlatformCommand, network: &Network) -> ipc::Reply {
        let failed = |code: BoundaryErrorCode, message: &str| ipc::Reply::Platform {
            event: PlatformEvent::Failed {
                code,
                message: message.into(),
            },
        };
        match command {
            // These variants predate the host payload protocol. Do not let a
            // Session-0 process infer or access interactive clipboard state.
            PlatformCommand::ReadClipboard
            | PlatformCommand::WriteClipboard { .. }
            | PlatformCommand::ExecuteWorkflow { .. } => failed(
                BoundaryErrorCode::InvalidRequest,
                "typed Windows host payload required",
            ),
            PlatformCommand::CreateFileDataObject {
                paths,
                target_peer_id,
            } => {
                if paths.is_empty() || paths.len() > 512 {
                    return failed(BoundaryErrorCode::InvalidRequest, "invalid file list");
                }
                match self.publish_file_data_object(network, paths, target_peer_id) {
                    Ok(id) => ipc::Reply::Platform {
                        event: PlatformEvent::FileDataObjectCreated { id },
                    },
                    Err(message) => failed(BoundaryErrorCode::Native, &message),
                }
            }
            PlatformCommand::ReadClipboardHost { content } => {
                if content.revision.value() < 0
                    || content.data.bytes.len() > ipc::MAX_CONTENT
                    || content.data.file_urls.len() > 512
                {
                    return failed(
                        BoundaryErrorCode::InvalidRequest,
                        "invalid clipboard capture",
                    );
                }
                self.platform_capture = Some(content.clone());
                ipc::Reply::Platform {
                    event: PlatformEvent::ClipboardCaptured(content),
                }
            }
            PlatformCommand::WriteClipboardHost {
                content,
                expected_revision,
            } => {
                if expected_revision.value() < 0
                    || content.data.bytes.len() > ipc::MAX_CONTENT
                    || content.data.file_urls.len() > 512
                {
                    return failed(BoundaryErrorCode::InvalidRequest, "invalid clipboard write");
                }
                let Some(next) = expected_revision.value().checked_add(1) else {
                    return failed(BoundaryErrorCode::Conflict, "clipboard revision exhausted");
                };
                ipc::Reply::Platform {
                    event: PlatformEvent::ClipboardApplied(ApplyResult::Applied {
                        new_revision: NativeRevision::new(next),
                    }),
                }
            }
            PlatformCommand::ExecuteWorkflowHost { workflow_id, input } => {
                let input_bytes = input
                    .iter()
                    .try_fold(0usize, |total, item| total.checked_add(item.bytes.len()));
                if input.is_empty() || input_bytes.is_none_or(|total| total > ipc::MAX_CONTENT) {
                    return failed(
                        BoundaryErrorCode::InvalidRequest,
                        "workflow input is empty or oversized",
                    );
                }
                let workflow = match network.workflow_catalog.list() {
                    Ok(workflows) => workflows.into_iter().find(|item| item.id == workflow_id),
                    Err(_) => {
                        return failed(BoundaryErrorCode::Ipc, "cannot read workflow catalog");
                    }
                };
                let Some(workflow) = workflow else {
                    return failed(BoundaryErrorCode::InvalidRequest, "workflow does not exist");
                };
                let wire_input = input
                    .into_iter()
                    .map(|item| ipc::WorkflowInput {
                        mime: item.mime,
                        data_base64: base64::engine::general_purpose::STANDARD.encode(item.bytes),
                    })
                    .collect::<Vec<_>>();
                match workflow::execute(&workflow, &wire_input) {
                    Ok(result) => {
                        let output = result
                            .formats
                            .iter()
                            .find(|format| format.mime == "text/plain")
                            .or_else(|| result.formats.first())
                            .and_then(|format| {
                                base64::engine::general_purpose::STANDARD
                                    .decode(&format.data_base64)
                                    .ok()
                                    .map(|bytes| ContentData {
                                        mime: format.mime.clone(),
                                        bytes,
                                        file_urls: Vec::new(),
                                        items: Vec::new(),
                                    })
                            });
                        ipc::Reply::Platform {
                            event: PlatformEvent::WorkflowCompleted {
                                workflow_id,
                                message: result.message,
                                content: output,
                            },
                        }
                    }
                    Err(message) => failed(BoundaryErrorCode::CapabilityDenied, &message),
                }
            }
        }
    }

    fn canonical_library(
        &mut self,
        network: &Network,
    ) -> Result<std::sync::Arc<deskfuse_engine::clipboard_library::Library>, String> {
        if let Some(library) = &self.library {
            return Ok(library.clone());
        }
        let owner = self.owner.ok_or("desktop session unavailable")?;
        let directory = security::user_library_dir(&owner).map_err(|error| error.to_string())?;
        std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        let local_id = network
            .runtime
            .snapshot_view()
            .snapshot
            .local_device_id
            .to_hex();
        let library = deskfuse_engine::clipboard_library::Library::open_at(
            directory.join("state.sqlite3"),
            local_id,
        )?;
        self.library_sync = Some(deskfuse_engine::library_sync::Sync::with_library(
            library.clone(),
        ));
        self.library = Some(library.clone());
        Ok(library)
    }

    fn command(
        &mut self,
        command: ipc::Command,
        network: &Network,
        control: &Control,
        auth: ClientSession,
        epoch: u64,
        policy: SessionPolicy,
    ) -> ipc::Reply {
        if matches!(&command, ipc::Command::Snapshot) {
            self.clear();
            self.owner = Some(auth);
            self.epoch = epoch;
            self.heartbeat = Some(Instant::now());
            self.file_bridge = Some(crate::native::file_transfer::session_bridge());
            let settings = match network.store.file_transfer_settings() {
                Ok(settings) => settings,
                Err(_) => {
                    self.last_error = Some("无法读取文件传输设置，已暂时关闭文件共享".into());
                    FileTransferSettings {
                        enabled: false,
                        ..FileTransferSettings::default()
                    }
                }
            };
            let files_enabled = settings.enabled;
            self.apply_file_settings(settings);
            self.lan_web = Some(network.lan_web.clone());
            if !files_enabled && let Err(error) = network.lan_web.set_files_enabled(false) {
                self.last_error = Some(error);
            }
            network.lan_web.resume_session();
            if let Err(error) = self.canonical_library(network) {
                self.last_error = Some(format!("内容库初始化失败：{error}"));
            }
            self.sync.tick(&network.runtime);
            return ipc::Reply::Snapshot(self.snapshot(network, auth, epoch, policy));
        }
        if !auth.fully_present() && requires_full_presence(&command) {
            return ipc::Reply::rejected(
                "session_restricted",
                "该操作需要在本机已解锁桌面执行；锁屏或远程桌面会话期间不可用。",
            );
        }
        let owner_matches = self.owner == Some(auth) && self.epoch == epoch;
        if !owner_matches {
            return ipc::Reply::rejected("session_expired", "open a fresh desktop snapshot");
        }
        let input_command = matches!(
            &command,
            ipc::Command::RegisterInput { .. }
                | ipc::Command::PollInput { .. }
                | ipc::Command::InputFault { .. }
        );
        if !input_command {
            self.heartbeat = Some(Instant::now());
        }
        let command_is_leave = matches!(&command, ipc::Command::LeaveInputControl { .. });
        match command {
            ipc::Command::Platform {
                session_epoch,
                sequence,
                command,
            } => {
                if session_epoch != epoch || sequence == 0 {
                    return ipc::Reply::Platform {
                        event: PlatformEvent::Failed {
                            code: BoundaryErrorCode::Conflict,
                            message: "desktop session or request sequence is stale".into(),
                        },
                    };
                }
                self.platform_command(command, network)
            }
            ipc::Command::Poll {
                session_epoch,
                after_sequence,
            } => {
                if session_epoch != epoch {
                    return ipc::Reply::rejected(
                        "session_expired",
                        "open a fresh desktop snapshot",
                    );
                }
                self.sync.tick(&network.runtime);
                let (mut clipboard, clipboard_sequence) = self.sync.poll(after_sequence);
                if let Some(item) = clipboard.as_mut() {
                    item.source_metadata.no_history |=
                        self.library.as_ref().is_none_or(|library| {
                            library.excluded(item.source_metadata.app_bundle_id.as_deref())
                        });
                }
                let lan_web_uploads = self
                    .lan_web
                    .as_ref()
                    .map_or_else(Vec::new, lan_web::LanWebService::deliveries);
                ipc::Reply::Poll {
                    snapshot: self.snapshot(network, auth, epoch, policy),
                    clipboard,
                    clipboard_sequence,
                    lan_web_uploads,
                }
            }
            ipc::Command::InvalidateClipboard {
                session_epoch,
                sequence,
            } => {
                if session_epoch != epoch || sequence == 0 || sequence <= self.local_sequence {
                    return ipc::Reply::rejected(
                        "stale_clipboard",
                        "clipboard generation is no longer current",
                    );
                }
                self.local_sequence = sequence;
                self.sync.invalidate_copy(&network.runtime);
                ipc::Reply::queued()
            }
            ipc::Command::PublishClipboard {
                session_epoch,
                sequence,
                formats,
                source_app,
            } => {
                if session_epoch != epoch
                    || sequence == 0
                    || sequence < self.local_sequence
                    || sequence <= self.published_sequence
                {
                    return ipc::Reply::rejected(
                        "stale_clipboard",
                        "clipboard generation is no longer current",
                    );
                }
                self.local_sequence = sequence;
                self.published_sequence = sequence;
                self.sync.tick(&network.runtime);
                if source_app.as_ref().is_some_and(|app| app.len() > 512) {
                    return ipc::Reply::rejected(
                        "invalid_request",
                        "clipboard source app is too long",
                    );
                }
                let metadata = deskfuse_protocol::clipboard_sync::SourceMetadata {
                    no_history: source_app.is_none()
                        || self
                            .library
                            .as_ref()
                            .is_none_or(|library| library.excluded(source_app.as_deref())),
                    app_bundle_id: source_app,
                    device_id: Some(
                        network
                            .runtime
                            .snapshot_view()
                            .snapshot
                            .local_device_id
                            .to_string(),
                    ),
                    created_at_ms: Some(
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64,
                    ),
                    image_digest: None,
                };
                match self.sync.publish(&network.runtime, &formats, &metadata) {
                    Ok(()) => ipc::Reply::queued(),
                    Err(message) => ipc::Reply::rejected("clipboard_unavailable", message),
                }
            }
            ipc::Command::SetFileTransferEnabled { enabled } => {
                let mut settings = match network.store.file_transfer_settings() {
                    Ok(settings) => settings,
                    Err(error) => {
                        return ipc::Reply::rejected("store_unavailable", &error.to_string());
                    }
                };
                settings.enabled = enabled;
                if let Err(message) = settings.validate() {
                    return ipc::Reply::rejected("invalid_settings", &message);
                }
                if let Err(error) = network.store.set_file_transfer_settings(&settings) {
                    return ipc::Reply::rejected("store_unavailable", &error.to_string());
                }
                self.apply_file_settings(settings);
                if !enabled && let Err(error) = network.lan_web.set_files_enabled(false) {
                    self.last_error = Some(error);
                }
                ipc::Reply::queued()
            }
            ipc::Command::ConfigureLanWeb { enabled } => {
                if enabled && !self.file_settings.enabled {
                    return ipc::Reply::rejected("files_disabled", "文件传输已关闭");
                }
                match network.lan_web.configure(enabled) {
                    Ok(_) => ipc::Reply::queued(),
                    Err(message) => ipc::Reply::rejected("lan_web_unavailable", &message),
                }
            }
            ipc::Command::CommitLanWebUpload {
                session_epoch,
                upload_id,
                recorded,
                clipboard_written,
            } => {
                if session_epoch != epoch {
                    return ipc::Reply::rejected(
                        "session_expired",
                        "open a fresh desktop snapshot",
                    );
                }
                if network
                    .lan_web
                    .commit(&upload_id, recorded, clipboard_written)
                {
                    ipc::Reply::queued()
                } else {
                    ipc::Reply::rejected("stale_upload", "网页上传已不再等待桌面确认")
                }
            }
            ipc::Command::PublishFileClipboard {
                session_epoch,
                sequence,
                paths,
                target_peer_id,
            } => {
                if session_epoch != epoch
                    || sequence == 0
                    || sequence <= self.local_sequence
                    || sequence <= self.published_sequence
                {
                    return ipc::Reply::rejected(
                        "stale_clipboard",
                        "clipboard generation is no longer current",
                    );
                }
                if !self.file_settings.enabled {
                    return ipc::Reply::rejected("files_disabled", "文件传输已关闭");
                }
                self.local_sequence = sequence;
                self.published_sequence = sequence;
                self.sync.invalidate_copy(&network.runtime);
                let Some(files) = self.file_bridge.as_ref() else {
                    return ipc::Reply::rejected("files_unavailable", "文件通道尚未就绪");
                };
                let mut peers = self.sync.file_peers(&network.runtime);
                if let Some(target) = target_peer_id.as_deref() {
                    peers.retain(|peer| peer.to_string() == target);
                    if peers.is_empty() {
                        return ipc::Reply::rejected(
                            "peer_unavailable",
                            "目标设备不在线或不支持文件通道",
                        );
                    }
                }
                let frames = match files.publish_local(
                    &peers,
                    paths.into_iter().map(std::path::PathBuf::from).collect(),
                ) {
                    Ok(frames) => frames,
                    Err(message) => return ipc::Reply::rejected("files_unavailable", &message),
                };
                for (peer, _id, frame) in frames {
                    if let Ok(data) = frame.encode() {
                        self.sync.announce_file_offer(&network.runtime, peer, data);
                    }
                }
                ipc::Reply::queued()
            }
            ipc::Command::RequestFileClipboard {
                session_epoch,
                transfer_id,
            } => {
                if session_epoch != epoch {
                    return ipc::Reply::rejected(
                        "session_expired",
                        "open a fresh desktop snapshot",
                    );
                }
                if !self.file_settings.enabled {
                    return ipc::Reply::rejected("files_disabled", "文件传输已关闭");
                }
                match self.file_bridge.as_ref().map_or_else(
                    || Err("文件通道尚未就绪".to_owned()),
                    |files| files.request_incoming(&transfer_id),
                ) {
                    Ok(()) => ipc::Reply::queued(),
                    Err(message) => ipc::Reply::rejected("files_unavailable", &message),
                }
            }
            ipc::Command::CancelFileClipboard {
                session_epoch,
                transfer_id,
            } => {
                if session_epoch != epoch {
                    return ipc::Reply::rejected(
                        "session_expired",
                        "open a fresh desktop snapshot",
                    );
                }
                if let Some(files) = self.file_bridge.as_ref() {
                    files.cancel_transfer(&transfer_id, "Windows desktop cancelled file transfer");
                }
                ipc::Reply::queued()
            }
            ipc::Command::CommitFileClipboard {
                session_epoch,
                transfer_id,
                applied,
            } => {
                if session_epoch != epoch {
                    return ipc::Reply::rejected(
                        "session_expired",
                        "open a fresh desktop snapshot",
                    );
                }
                match self.file_bridge.as_ref().map_or_else(
                    || Err("文件通道尚未就绪".to_owned()),
                    |files| files.commit_incoming(&transfer_id, applied),
                ) {
                    Ok(true) => ipc::Reply::queued(),
                    Ok(false) => ipc::Reply::rejected("stale_transfer", "文件传输不再等待桌面确认"),
                    Err(message) => ipc::Reply::rejected("files_unavailable", &message),
                }
            }
            ipc::Command::CommitClipboard {
                session_epoch,
                sequence,
                applied,
            } => {
                if session_epoch != epoch || !self.sync.commit(&network.runtime, sequence, applied)
                {
                    return ipc::Reply::rejected(
                        "stale_clipboard",
                        "clipboard application belongs to an expired generation",
                    );
                }
                ipc::Reply::queued()
            }
            ipc::Command::RegisterInput { topology } => {
                if topology.validate().is_err() {
                    return ipc::Reply::rejected("invalid_input", "invalid display topology");
                }
                self.input_heartbeat = None;
                self.input_desktop = ipc::InputDesktop::Unavailable;
                self.input_topology = Some(topology.clone());
                self.input_reported_activation = None;
                self.input_last_lease = None;
                self.release_input("Windows input helper generation opened");
                match network
                    .input_runtime
                    .handle
                    .try_send(RuntimeCommand::PublishDisplayTopology(topology))
                {
                    Ok(()) => ipc::Reply::InputRegistered {
                        session_epoch: epoch,
                    },
                    Err(_) => ipc::Reply::rejected("busy", "input runtime queue unavailable"),
                }
            }
            // 连接本身在 `handle` 里登记；这里只确认 helper 能收推送帧。
            ipc::Command::EnableInputPush => ipc::Reply::queued(),
            ipc::Command::PollInput {
                session_epoch,
                after_sequence,
                desktop,
                stages,
            } => {
                if session_epoch != epoch {
                    return ipc::Reply::rejected("session_expired", "input helper epoch expired");
                }
                self.input_desktop = desktop;
                if desktop != ipc::InputDesktop::Default {
                    self.input_heartbeat = None;
                    self.session_agent
                        .clear("secure session agent is not available through the ordinary helper");
                    self.release_input("Windows input desktop is protected or unavailable");
                    return ipc::Reply::rejected(
                        "desktop_unavailable",
                        "Default input desktop required",
                    );
                }
                if self.input_topology.is_none() {
                    return ipc::Reply::rejected(
                        "input_unregistered",
                        "register the Windows input helper first",
                    );
                }
                self.input_heartbeat = Some(Instant::now());
                while self
                    .input_deliveries
                    .front()
                    .is_some_and(|delivery| delivery.sequence() <= after_sequence)
                {
                    self.input_deliveries.pop_front();
                }
                let deliveries = self
                    .input_deliveries
                    .iter()
                    .take(64)
                    .cloned()
                    .collect::<Vec<_>>();
                let sequence = deliveries
                    .last()
                    .map_or(after_sequence, ipc::InputDelivery::sequence);
                // 轮询回复同样算已交付，推送不会把它再发一遍。
                self.input_delivered = self.input_delivered.max(sequence);
                // 只有这份轮询会被 helper 当成已接收。拒绝路径由 helper 把计数加回去。
                deskfuse_core::add_pending_stages(stages);
                ipc::Reply::InputPoll {
                    state: self.input_state(network),
                    deliveries,
                    sequence,
                }
            }
            ipc::Command::InputFault { message } => {
                self.input_heartbeat = None;
                self.input_last_lease = None;
                self.release_input("Windows input helper reported a native failure");
                let snapshot = network.input_runtime.handle.input_snapshot();
                if let Some(managed) = snapshot.managed_control {
                    let _ = network.input_runtime.handle.try_send(
                        RuntimeCommand::SetManagedInputAvailability {
                            activation_id: managed.activation_id,
                            status: deskfuse_core::InputActivationStatus::Unavailable,
                            reason: Some(message.chars().take(256).collect()),
                        },
                    );
                }
                self.input_reported_activation = None;
                ipc::Reply::queued()
            }
            ipc::Command::SetSessionPolicy {
                allow_locked,
                allow_remote,
            } => {
                // Reachable only for fully present sessions; the deny-list above
                // rejects this command for relaxed sessions before dispatch.
                let next = SessionPolicy {
                    allow_locked,
                    allow_remote,
                };
                let Ok(json) = serde_json::to_string(&next) else {
                    return ipc::Reply::rejected("invalid_input", "invalid session policy");
                };
                match network.store.set_session_policy_json(&json) {
                    Ok(()) => {
                        control.set_session_policy(next);
                        ipc::Reply::queued()
                    }
                    Err(error) => ipc::Reply::rejected("storage_unavailable", &error.to_string()),
                }
            }
            ipc::Command::SetInputSharingSettings { settings } => {
                if let Err(error) = settings.validate() {
                    return ipc::Reply::rejected("invalid_input", &error.to_string());
                }
                // 输入设置驱动的是 42425 输入运行时；控制 runtime 内部的 input
                // actor 只写共享 store，不会让已运行的输入会话感知变更。
                match network
                    .input_runtime
                    .handle
                    .try_send(RuntimeCommand::SetInputSharingSettings(settings))
                {
                    Ok(()) => ipc::Reply::queued(),
                    Err(_) => ipc::Reply::rejected("busy", "input runtime queue unavailable"),
                }
            }
            ipc::Command::EnableInputControl { peer_id } => {
                let Ok(peer_id) = peer_id.parse::<deskfuse_core::DeviceId>() else {
                    return ipc::Reply::rejected("invalid_input", "invalid input peer id");
                };
                let settings = controller_settings(
                    network.input_runtime.handle.input_snapshot().settings,
                    Some(peer_id),
                );
                match settings.validate() {
                    Ok(()) => match network
                        .input_runtime
                        .handle
                        .try_send(RuntimeCommand::SetInputSharingSettings(settings))
                    {
                        Ok(()) => ipc::Reply::queued(),
                        Err(_) => ipc::Reply::rejected("busy", "input runtime queue unavailable"),
                    },
                    Err(error) => ipc::Reply::rejected("invalid_input", &error.to_string()),
                }
            }
            ipc::Command::DisableInputControl => {
                let settings = controller_settings(
                    network.input_runtime.handle.input_snapshot().settings,
                    None,
                );
                match network
                    .input_runtime
                    .handle
                    .try_send(RuntimeCommand::SetInputSharingSettings(settings))
                {
                    Ok(()) => {
                        self.controlling = None;
                        ipc::Reply::queued()
                    }
                    Err(_) => ipc::Reply::rejected("busy", "input runtime queue unavailable"),
                }
            }
            ipc::Command::RestartInputControl { peer_id } => {
                let Ok(peer_id) = peer_id.parse::<deskfuse_core::DeviceId>() else {
                    return ipc::Reply::rejected("invalid_input", "invalid input peer id");
                };
                match network
                    .input_runtime
                    .handle
                    .try_send(RuntimeCommand::RestartInputActivation { peer_id })
                {
                    Ok(()) => ipc::Reply::queued(),
                    Err(_) => ipc::Reply::rejected("busy", "input runtime queue unavailable"),
                }
            }
            ipc::Command::EnterInputControl {
                session_id,
                target_display,
                x,
                y,
            } => {
                let Ok(session_id) = parse_input_session(&session_id) else {
                    return ipc::Reply::rejected("invalid_input", "invalid input session id");
                };
                if target_display.trim().is_empty() || !x.is_finite() || !y.is_finite() {
                    return ipc::Reply::rejected("invalid_input", "invalid input target");
                }
                match network
                    .input_runtime
                    .handle
                    .try_send(RuntimeCommand::EnterInputControl {
                        session_id,
                        target_display: target_display.clone().into(),
                        x,
                        y,
                    }) {
                    Ok(()) => {
                        let peer_id = network
                            .input_runtime
                            .handle
                            .input_snapshot()
                            .links
                            .iter()
                            .find(|link| link.control_session == Some(session_id))
                            .map(|link| link.peer_id.to_string())
                            .unwrap_or_default();
                        self.controlling = Some(ipc::ControlView {
                            session_id: session_id.to_string(),
                            peer_id,
                            target_display,
                        });
                        ipc::Reply::queued()
                    }
                    Err(_) => ipc::Reply::rejected("busy", "input runtime queue unavailable"),
                }
            }
            ipc::Command::SendInputEvents {
                session_id,
                target_display,
                batch: _,
                events,
                relative,
            } => {
                let Ok(session_id) = parse_input_session(&session_id) else {
                    return ipc::Reply::rejected("invalid_input", "invalid input session id");
                };
                if target_display.trim().is_empty()
                    || events.is_empty()
                    || events.len() > 256
                    || events.iter().any(|event| event.validate().is_err())
                {
                    return ipc::Reply::rejected("invalid_input", "invalid input event batch");
                }
                match network
                    .input_runtime
                    .handle
                    .try_send(RuntimeCommand::SendSharedInput {
                        session_id,
                        target_display: target_display.into(),
                        events,
                        relative,
                    }) {
                    Ok(()) => ipc::Reply::queued(),
                    Err(_) => ipc::Reply::rejected("busy", "input runtime queue unavailable"),
                }
            }
            ipc::Command::LeaveInputControl { session_id }
            | ipc::Command::ReleaseInputControl { session_id } => {
                let Ok(session_id) = parse_input_session(&session_id) else {
                    return ipc::Reply::rejected("invalid_input", "invalid input session id");
                };
                let command = if command_is_leave {
                    RuntimeCommand::LeaveInputControl { session_id }
                } else {
                    RuntimeCommand::ReleaseInputControl { session_id }
                };
                match network.input_runtime.handle.try_send(command) {
                    Ok(()) => {
                        if self
                            .controlling
                            .as_ref()
                            .is_some_and(|control| control.session_id == session_id.to_string())
                        {
                            self.controlling = None;
                        }
                        ipc::Reply::queued()
                    }
                    Err(_) => ipc::Reply::rejected("busy", "input runtime queue unavailable"),
                }
            }
            ipc::Command::LibraryQuery { request } => {
                match self
                    .canonical_library(network)
                    .and_then(|library| library.query(&request))
                {
                    Ok(json) => ipc::Reply::LibraryResult { json },
                    Err(message) => ipc::Reply::rejected("library_unavailable", &message),
                }
            }
            ipc::Command::LibraryMutate { request } => {
                match self
                    .canonical_library(network)
                    .and_then(|library| library.mutate(&request))
                {
                    Ok(json) => ipc::Reply::LibraryResult { json },
                    Err(message) => ipc::Reply::rejected("library_unavailable", &message),
                }
            }
            ipc::Command::LibraryContent { id } => {
                match self
                    .canonical_library(network)
                    .and_then(|library| library_content_reply(&library, id))
                {
                    Ok(reply) => reply,
                    Err(message) => ipc::Reply::rejected("library_unavailable", &message),
                }
            }
            ipc::Command::LibraryThumbnail { id, max_pixel_size } => {
                match self
                    .canonical_library(network)
                    .and_then(|library| library.thumbnail(id, max_pixel_size.unwrap_or(160)))
                {
                    Ok(bytes) if bytes.is_empty() => ipc::Reply::LibraryThumbnail {
                        mime: "image/png".into(),
                        data_base64: String::new(),
                    },
                    Ok(bytes) => ipc::Reply::LibraryThumbnail {
                        mime: "image/png".into(),
                        data_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
                    },
                    Err(message) => ipc::Reply::rejected("library_unavailable", &message),
                }
            }
            ipc::Command::LibraryIngest {
                formats,
                files,
                source,
                device_id,
                app_id,
                app_name,
                copied_at_ms,
            } => {
                let decoded = formats
                    .into_iter()
                    .map(|format| {
                        let data = base64::engine::general_purpose::STANDARD
                            .decode(format.data_base64)
                            .map_err(|_| "剪贴板内容不是有效的 base64".to_owned())?;
                        Ok((format.mime, data))
                    })
                    .collect::<Result<Vec<_>, String>>();
                match decoded.and_then(|formats| {
                    let total = formats.iter().map(|(_, data)| data.len()).sum::<usize>();
                    if total > ipc::MAX_CONTENT {
                        return Err("剪贴板内容超过 16 MiB 上限".into());
                    }
                    let text = formats
                        .iter()
                        .find(|(mime, _)| {
                            mime == "text/plain" || mime == "text/plain;charset=utf-8"
                        })
                        .and_then(|(_, data)| String::from_utf8(data.clone()).ok());
                    let png = formats
                        .iter()
                        .find(|(mime, _)| mime == "image/png")
                        .map(|(_, data)| data.clone());
                    let snapshot = deskfuse_platform::PasteboardSnapshot {
                        text,
                        png,
                        file_urls: files,
                        items: vec![deskfuse_platform::PasteboardItem {
                            representations: formats
                                .into_iter()
                                .map(|(type_name, data)| {
                                    deskfuse_platform::PasteboardRepresentation { type_name, data }
                                })
                                .collect(),
                        }],
                        ..Default::default()
                    };
                    let local = source == "本机" || source == "工作流";
                    let origin = deskfuse_engine::clipboard_library::Origin {
                        device_id: if local {
                            network
                                .runtime
                                .snapshot_view()
                                .snapshot
                                .local_device_id
                                .to_hex()
                        } else {
                            device_id
                        },
                        device_name: source.clone(),
                        app_id,
                        app_name,
                        copied_at_ms,
                        no_history: false,
                    };
                    self.canonical_library(network)
                        .and_then(|library| library.ingest_with_origin(&snapshot, &source, origin))
                }) {
                    Ok(id) => ipc::Reply::LibraryResult {
                        json: serde_json::to_string(&id).unwrap_or_else(|_| "null".into()),
                    },
                    Err(message) => ipc::Reply::rejected("library_unavailable", &message),
                }
            }
            ipc::Command::ShareLibraryEntry { id, peer_id } => {
                let Ok(peer) = DeviceId::from_hex(&peer_id) else {
                    return ipc::Reply::rejected_typed(
                        BoundaryErrorCode::InvalidRequest,
                        "目标设备标识无效",
                    );
                };
                let connected =
                    network
                        .runtime
                        .snapshot_view()
                        .snapshot
                        .peers
                        .iter()
                        .any(|candidate| {
                            candidate.id == peer
                                && candidate.trust == TrustState::Trusted
                                && candidate.connection == ConnectionState::Connected
                        });
                if !connected {
                    return ipc::Reply::rejected_typed(
                        BoundaryErrorCode::InvalidRequest,
                        "目标设备未连接或未受信任",
                    );
                }
                let content = match self
                    .canonical_library(network)
                    .and_then(|library| library_content_reply(&library, id))
                {
                    Ok(ipc::Reply::LibraryContent { formats, files }) => (formats, files),
                    Ok(_) => {
                        return ipc::Reply::rejected(
                            "library_unavailable",
                            "内容库条目没有可共享的内容",
                        );
                    }
                    Err(message) => return ipc::Reply::rejected("library_unavailable", &message),
                };
                let (formats, files) = content;
                if !files.is_empty() {
                    return match self.publish_file_data_object(network, files, Some(peer_id)) {
                        Ok(_) => ipc::Reply::Queued {
                            message: "已向目标设备提供文件；对方接受后开始传输。".into(),
                        },
                        Err(message) => {
                            ipc::Reply::rejected_typed(BoundaryErrorCode::Native, message)
                        }
                    };
                }
                match self.sync.publish_to(&network.runtime, peer, &formats) {
                    Ok(()) => ipc::Reply::Queued {
                        message: "已发送内容；等待目标设备应用到剪贴板。".into(),
                    },
                    Err(message) => {
                        ipc::Reply::rejected_typed(BoundaryErrorCode::Conflict, message)
                    }
                }
            }
            ipc::Command::ShareContent { peer_id, formats } => {
                if formats.is_empty() {
                    return ipc::Reply::rejected_typed(
                        BoundaryErrorCode::InvalidRequest,
                        "没有可共享的内容",
                    );
                }
                let Ok(peer) = DeviceId::from_hex(&peer_id) else {
                    return ipc::Reply::rejected_typed(
                        BoundaryErrorCode::InvalidRequest,
                        "目标设备标识无效",
                    );
                };
                let connected =
                    network
                        .runtime
                        .snapshot_view()
                        .snapshot
                        .peers
                        .iter()
                        .any(|candidate| {
                            candidate.id == peer
                                && candidate.trust == TrustState::Trusted
                                && candidate.connection == ConnectionState::Connected
                        });
                if !connected {
                    return ipc::Reply::rejected_typed(
                        BoundaryErrorCode::InvalidRequest,
                        "目标设备未连接或未受信任",
                    );
                }
                match self.sync.publish_to(&network.runtime, peer, &formats) {
                    Ok(()) => ipc::Reply::Queued {
                        message: "已发送处理结果；等待目标设备应用到剪贴板。".into(),
                    },
                    Err(message) => {
                        ipc::Reply::rejected_typed(BoundaryErrorCode::Conflict, message)
                    }
                }
            }
            ipc::Command::GetWorkflows => match network.workflow_catalog.list() {
                Ok(workflows) => ipc::Reply::Workflows { workflows },
                Err(_) => ipc::Reply::rejected("storage_unavailable", "无法读取工作流目录"),
            },
            ipc::Command::SaveWorkflow { workflow } => {
                match network.workflow_catalog.save(workflow) {
                    Ok(()) => ipc::Reply::queued(),
                    Err(error) => ipc::Reply::rejected("invalid_workflow", &error),
                }
            }
            ipc::Command::DeleteWorkflow { id } => match network.workflow_catalog.delete(&id) {
                Ok(()) => ipc::Reply::queued(),
                Err(error) => ipc::Reply::rejected("storage_unavailable", &error),
            },
            ipc::Command::RunWorkflow { workflow_id, input } => {
                let workflow = match network.workflow_catalog.list() {
                    Ok(workflows) => workflows.into_iter().find(|item| item.id == workflow_id),
                    Err(_) => {
                        return ipc::Reply::rejected("storage_unavailable", "无法读取工作流目录");
                    }
                };
                let Some(workflow) = workflow else {
                    return ipc::Reply::rejected("not_found", "工作流不存在");
                };
                match workflow::execute(&workflow, &input) {
                    Ok(result) => ipc::Reply::WorkflowResult {
                        run_id: result.run_id,
                        workflow_id,
                        formats: result.formats,
                        message: result.message,
                    },
                    Err(message) => ipc::Reply::rejected("workflow_failed", &message),
                }
            }
            ipc::Command::CancelWorkflow { .. } => ipc::Reply::rejected(
                "not_supported",
                "当前 Windows 工作流步骤为受限同步执行，无需取消",
            ),
            ipc::Command::GetWorkflowCatalog => match network.workflow_catalog.view_json() {
                Ok(json) => ipc::Reply::WorkflowCatalog { json },
                Err(error) => ipc::Reply::rejected("storage_unavailable", &error),
            },
            ipc::Command::RestoreWorkflowConflict { conflict_id } => {
                match network.workflow_catalog.restore(&conflict_id) {
                    Ok(()) => ipc::Reply::queued(),
                    Err(error) => ipc::Reply::rejected("not_found", &error),
                }
            }
            ipc::Command::ReclaimInput => {
                self.release_input("Windows user reclaimed local input");
                self.input_reported_activation = None;
                match network
                    .input_runtime
                    .handle
                    .try_send(RuntimeCommand::PauseCurrentManagedActivation)
                {
                    Ok(()) => ipc::Reply::queued(),
                    Err(_) => ipc::Reply::rejected("busy", "input runtime queue unavailable"),
                }
            }
            ipc::Command::Rename { .. } => ipc::Reply::rejected_typed(
                BoundaryErrorCode::Unsupported,
                "authoritative runtime rename is not yet available",
            ),
            ipc::Command::ListRemoteApps => {
                let (available, reason) = match remote_app::availability() {
                    remote_app::Availability::Available => (true, None),
                    remote_app::Availability::UnsupportedEdition { edition } => (
                        false,
                        Some(format!(
                            "当前 Windows 版本（{edition}）不支持 RemoteApp；需要专业版或更高版本"
                        )),
                    ),
                };
                let apps = if available {
                    remote_app::list_remote_apps()
                        .into_iter()
                        .filter(|entry| remote_app::validate_entry(entry))
                        .filter(|entry| {
                            std::fs::metadata(&entry.path)
                                .map(|metadata| metadata.is_file())
                                .unwrap_or(false)
                        })
                        .map(remote_app_info)
                        .collect()
                } else {
                    Vec::new()
                };
                ipc::Reply::RemoteApps {
                    available,
                    reason,
                    apps,
                    host: remote_app::host_status_cached().map(|host| (*host).clone()),
                }
            }
            ipc::Command::ListInstalledApps => {
                let apps = remote_app::list_installed_apps()
                    .into_iter()
                    .map(installed_app_info)
                    .collect();
                ipc::Reply::InstalledApps { apps }
            }
            ipc::Command::RegisterRemoteApp {
                name,
                path,
                display_name,
            } => {
                if !remote_app::valid_name(name.trim()) || path.trim().is_empty() {
                    return ipc::Reply::rejected_typed(
                        BoundaryErrorCode::InvalidRequest,
                        "RemoteApp 名称与可执行文件路径不能为空",
                    );
                }
                if let remote_app::Availability::UnsupportedEdition { edition } =
                    remote_app::availability()
                {
                    return ipc::Reply::rejected_typed(
                        BoundaryErrorCode::Unsupported,
                        format!(
                            "当前 Windows 版本（{edition}）不支持 RemoteApp；需要专业版或更高版本"
                        ),
                    );
                }
                if !std::fs::metadata(&path)
                    .map(|metadata| metadata.is_file())
                    .unwrap_or(false)
                {
                    return ipc::Reply::rejected_typed(
                        BoundaryErrorCode::InvalidRequest,
                        "RemoteApp 可执行文件路径必须指向实际文件",
                    );
                }
                let full_name = if display_name.trim().is_empty() {
                    path.rsplit('\\')
                        .next()
                        .filter(|stem| !stem.is_empty())
                        .map(|stem| stem.strip_suffix(".exe").unwrap_or(stem).to_owned())
                        .unwrap_or_else(|| path.clone())
                } else {
                    display_name
                };
                let entry = remote_app::RemoteAppEntry {
                    name,
                    full_name,
                    path,
                    command_line: String::new(),
                    command_line_setting: 1,
                    icon_path: String::new(),
                    icon_index: 0,
                    show_in_tswa: 0,
                };
                match remote_app::register_app(&entry) {
                    Ok(()) => ipc::Reply::queued(),
                    Err(_) => ipc::Reply::rejected_typed(
                        BoundaryErrorCode::Native,
                        "无法写入 RemoteApp 注册表",
                    ),
                }
            }
            ipc::Command::UnregisterRemoteApp { name } => {
                if let remote_app::Availability::UnsupportedEdition { edition } =
                    remote_app::availability()
                {
                    return ipc::Reply::rejected_typed(
                        BoundaryErrorCode::Unsupported,
                        format!("当前 Windows 版本（{edition}）不支持 RemoteApp"),
                    );
                }
                match remote_app::unregister_app(&name) {
                    Ok(()) => ipc::Reply::queued(),
                    Err(_) => ipc::Reply::rejected_typed(
                        BoundaryErrorCode::Native,
                        "无法移除 RemoteApp 注册项",
                    ),
                }
            }
            ipc::Command::RemoteAppConnection { name } => {
                if let remote_app::Availability::UnsupportedEdition { edition } =
                    remote_app::availability()
                {
                    return ipc::Reply::rejected_typed(
                        BoundaryErrorCode::Unsupported,
                        format!("当前 Windows 版本（{edition}）不支持 RemoteApp"),
                    );
                }
                let Some(entry) = remote_app::list_remote_apps().into_iter().find(|entry| {
                    remote_app::validate_entry(entry)
                        && entry.name == name
                        && std::fs::metadata(&entry.path)
                            .map(|metadata| metadata.is_file())
                            .unwrap_or(false)
                }) else {
                    return ipc::Reply::rejected_typed(
                        BoundaryErrorCode::InvalidRequest,
                        "该应用尚未注册为 RemoteApp",
                    );
                };
                match remote_app::rdp_file(&entry) {
                    Ok(rdp) => ipc::Reply::RemoteAppConnection { rdp },
                    Err(_) => ipc::Reply::rejected_typed(
                        BoundaryErrorCode::InvalidRequest,
                        "该 RemoteApp 注册信息无效，请重新注册",
                    ),
                }
            }
            other => match runtime_command(other) {
                Some(command) => {
                    self.last_error = None;
                    // runtime_command 产出的都是键鼠共享命令（场景、布局、拖动），
                    // 与上面的 SetInputSharingSettings 一样属于 42425 输入运行时。
                    let sent = if command.is_input() {
                        network.input_runtime.handle.try_send(command)
                    } else {
                        network.runtime.try_send(command)
                    };
                    match sent {
                        Ok(()) => ipc::Reply::queued(),
                        Err(_) => ipc::Reply::rejected("busy", "runtime command queue unavailable"),
                    }
                }
                None => ipc::Reply::rejected(
                    "invalid_request",
                    "invalid command identifier or IPv4 address",
                ),
            },
        }
    }
}
fn remote_app_info(entry: remote_app::RemoteAppEntry) -> ipc::RemoteAppInfo {
    ipc::RemoteAppInfo {
        name: entry.name,
        display_name: entry.full_name,
        path: entry.path,
        command_line: entry.command_line,
        icon_path: entry.icon_path,
        icon_index: entry.icon_index,
    }
}

fn installed_app_info(app: remote_app::InstalledApp) -> ipc::InstalledAppInfo {
    ipc::InstalledAppInfo {
        name: app.name,
        path: app.path,
    }
}

fn parse_input_session(value: &str) -> Result<deskfuse_core::InputSessionId, ()> {
    value
        .parse::<uuid::Uuid>()
        .map(deskfuse_core::InputSessionId)
        .map_err(|_| ())
}

pub(crate) fn controller_settings(
    mut settings: deskfuse_core::InputSharingSettings,
    peer: Option<deskfuse_core::DeviceId>,
) -> deskfuse_core::InputSharingSettings {
    match peer {
        Some(peer) => {
            settings.enabled = true;
            settings.role = deskfuse_core::InputShareRole::Controller;
            settings.authorized_controller = None;
            if !settings.participants.contains(&peer) {
                settings.participants.push(peer);
            }
        }
        None => {
            settings.enabled = false;
            settings.role = deskfuse_core::InputShareRole::Disabled;
            settings.authorized_controller = None;
        }
    }
    settings
}

/// Commands that create trust, move input or widen the session gate itself.
/// Relaxed (locked-console or remote-protocol) sessions keep clipboard, file
/// and status features only; everything here still requires the unlocked
/// physical console.
fn library_content_reply(
    library: &deskfuse_engine::clipboard_library::Library,
    id: u64,
) -> Result<ipc::Reply, String> {
    let snapshot = library.snapshot(id)?;
    // `content` resolves file references to paths the interactive user can open;
    // the native snapshot keeps the original representations.
    let resolved = library.content(id).ok();
    let files = resolved
        .as_ref()
        .map(|item| item.file_paths.clone())
        .filter(|paths| !paths.is_empty())
        .unwrap_or_else(|| snapshot.file_urls.clone());
    let mut formats: Vec<ipc::Format> = Vec::new();
    let mut total = 0usize;
    let mut push = |mime: &str, data: &[u8]| -> Result<(), String> {
        if formats.iter().any(|format| format.mime == mime) || data.is_empty() {
            return Ok(());
        }
        total = total.saturating_add(data.len());
        if total > ipc::MAX_CONTENT {
            return Err("内容库条目超过 16 MiB 上限".into());
        }
        formats.push(ipc::Format {
            mime: mime.into(),
            data_base64: base64::engine::general_purpose::STANDARD.encode(data),
        });
        Ok(())
    };
    for item in &snapshot.items {
        for representation in &item.representations {
            let mime = match representation.type_name.as_str() {
                "public.utf8-plain-text" | "public.plain-text" => "text/plain",
                "public.png" => "image/png",
                "public.html" => "text/html",
                "public.rtf" => "text/rtf",
                _ => continue,
            };
            push(mime, &representation.data)?;
        }
    }
    if let Some(text) = &snapshot.text {
        push("text/plain", text.as_bytes())?;
    }
    if let Some(png) = &snapshot.png {
        push("image/png", png)?;
    }
    if formats.is_empty() && files.is_empty() {
        return Err("内容库条目没有可用于剪贴板的内容".into());
    }
    Ok(ipc::Reply::LibraryContent { formats, files })
}

fn requires_full_presence(command: &ipc::Command) -> bool {
    matches!(
        command,
        ipc::Command::SetSessionPolicy { .. }
            | ipc::Command::RequestPairing { .. }
            | ipc::Command::RequestPairingAt { .. }
            | ipc::Command::Connect { .. }
            | ipc::Command::AcceptPairing { .. }
            | ipc::Command::AllowPairing { .. }
            | ipc::Command::EnterInputControl { .. }
            | ipc::Command::SendInputEvents { .. }
            | ipc::Command::LeaveInputControl { .. }
            | ipc::Command::ReleaseInputControl { .. }
            | ipc::Command::EnableInputControl { .. }
            | ipc::Command::DisableInputControl
            | ipc::Command::RestartInputControl { .. }
            | ipc::Command::MoveControlDevice { .. }
            | ipc::Command::SetInputSharingSettings { .. }
            | ipc::Command::SetFileTransferEnabled { .. }
            | ipc::Command::ConfigureLanWeb { .. }
            | ipc::Command::ReclaimInput
            | ipc::Command::RegisterRemoteApp { .. }
            | ipc::Command::UnregisterRemoteApp { .. }
    )
}
fn enum_string(value: &impl serde::Serialize) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".into())
}
fn runtime_command(command: ipc::Command) -> Option<RuntimeCommand> {
    Some(match command {
        ipc::Command::RefreshPeers => RuntimeCommand::RefreshPeers,
        ipc::Command::RequestPairing { peer_id } => RuntimeCommand::RequestPairing {
            peer_id: peer_id.parse().ok()?,
        },
        ipc::Command::RequestPairingAt { ipv4 } => {
            let ip: Ipv4Addr = ipv4.parse().ok()?;
            if ip.is_unspecified() || ip.is_loopback() || ip.is_multicast() || ip.is_broadcast() {
                return None;
            }
            RuntimeCommand::RequestPairingAt {
                endpoint: Endpoint::new(ip.to_string(), 42424).ok()?,
            }
        }
        ipc::Command::AcceptPairing { request_id } => RuntimeCommand::ApprovePairing {
            request_id: request_id.parse().ok()?,
        },
        ipc::Command::RejectPairing { request_id } => RuntimeCommand::RejectPairing {
            request_id: request_id.parse().ok()?,
            reason: None,
        },
        ipc::Command::Disconnect { peer_id } => RuntimeCommand::Disconnect {
            peer_id: peer_id.parse().ok()?,
        },
        ipc::Command::Connect { peer_id } => RuntimeCommand::Connect {
            peer_id: peer_id.parse().ok()?,
        },
        ipc::Command::Revoke { peer_id } => RuntimeCommand::RevokePeer {
            peer_id: peer_id.parse().ok()?,
        },
        ipc::Command::Forget { peer_id } => RuntimeCommand::ForgetPeer {
            peer_id: peer_id.parse().ok()?,
        },
        ipc::Command::AllowPairing { peer_id } => RuntimeCommand::AllowPairing {
            peer_id: peer_id.parse().ok()?,
        },
        ipc::Command::SaveScene { name } => RuntimeCommand::SaveInputSharingScene { name },
        ipc::Command::ApplyScene { scene_id } => {
            RuntimeCommand::ApplyInputSharingScene { scene_id }
        }
        ipc::Command::RenameScene { scene_id, name } => {
            RuntimeCommand::RenameInputSharingScene { scene_id, name }
        }
        ipc::Command::DeleteScene { scene_id } => {
            RuntimeCommand::DeleteInputSharingScene { scene_id }
        }
        ipc::Command::SaveEnvironmentLayout { name } => {
            RuntimeCommand::SaveEnvironmentLayout { name }
        }
        ipc::Command::MoveControlDevice {
            device_id,
            display_id,
            x,
            y,
        } => RuntimeCommand::MoveInputDevice {
            device_id: device_id.parse().ok()?,
            display_id: display_id.into(),
            x,
            y,
        },
        _ => return None,
    })
}
/// A silent client is gone. The fixed helper reports every few milliseconds,
/// so only an abandoned connection reaches this.
const IDLE_CONNECTION: Duration = Duration::from_secs(30);
/// The first frame is bounded tightly so a squatter cannot hold a client slot
/// for the whole idle window.
const FIRST_FRAME: Duration = Duration::from_secs(3);
/// Fallback push tick. Push is driven by a change signal; the tick only bounds
/// the damage of a wake-up lost to the window between two `select` rounds.
const PUSH_TICK: Duration = Duration::from_millis(50);

/// Serves one desktop client connection for as long as the client uses it.
///
/// The connection is the unit of work, not the request. The fixed input helper
/// keeps a single authenticated pipe for its whole process: reconnecting per
/// poll made the listening instance the scarce resource, so the helper lost the
/// race with ordinary clients and spent its life on `ERROR_PIPE_BUSY`.
///
/// One writer task owns the write side, which lets the service push input
/// deliveries while the reader waits for the next request without interleaving
/// two frames on the same byte stream.
pub(super) async fn handle(
    pipe: NamedPipeServer,
    network: &Network,
    control: Arc<Control>,
    link: u64,
) -> io::Result<()> {
    // Handle values are opaque, not addresses: carrying one as `usize` keeps the
    // reader loop `Send` while the halves own the pipe for its lifetime.
    let raw = pipe.as_raw_handle() as usize;
    let (mut reader, writer) = tokio::io::split(pipe);
    let (outgoing, mut frames) = mpsc::channel::<(ipc::Response, bool)>(4);
    let (written, mut written_rx) = mpsc::channel::<()>(4);
    let writer_task = tokio::spawn(async move {
        let mut writer = writer;
        while let Some((frame, answers_request)) = frames.recv().await {
            if protocol::write_frame_limited(&mut writer, &frame, ipc::MAX_FRAME)
                .await
                .is_err()
            {
                break;
            }
            // Only a reply carries an acknowledgement back to the reader. A push
            // has no ack, and signalling for one would let unread signals pile up
            // until the writer blocks on this channel and stops draining frames.
            if answers_request && written.send(()).await.is_err() {
                break;
            }
        }
    });
    let pusher = tokio::spawn(push_loop(Arc::clone(&control), outgoing.clone(), link));
    let result = serve(
        &mut reader,
        raw,
        network,
        &control,
        link,
        &outgoing,
        &mut written_rx,
    )
    .await;
    pusher.abort();
    writer_task.abort();
    result
}

async fn serve(
    reader: &mut (impl AsyncRead + Unpin),
    raw: usize,
    network: &Network,
    control: &Control,
    link: u64,
    outgoing: &mpsc::Sender<(ipc::Response, bool)>,
    written: &mut mpsc::Receiver<()>,
) -> io::Result<()> {
    let mut first = true;
    loop {
        let window = if first { FIRST_FRAME } else { IDLE_CONNECTION };
        first = false;
        let request: ipc::Request =
            match timeout(window, protocol::read_frame_limited(reader, ipc::MAX_FRAME)).await {
                Ok(Ok(request)) => request,
                Ok(Err(error)) => return Err(error),
                Err(_) => return Ok(()),
            };
        let mut epoch = control.epoch.load(Ordering::Acquire);
        let helper_request = matches!(
            &request.command,
            ipc::Command::RegisterInput { .. }
                | ipc::Command::PollInput { .. }
                | ipc::Command::EnableInputPush
                | ipc::Command::InputFault { .. }
        );
        // Helper polls are high-frequency and policy-independent; every other
        // request picks up persisted policy changes before authentication.
        if !helper_request {
            control.refresh_session_policy(network);
        }
        let policy = control.session_policy();
        let before = match security::authenticate(raw as HANDLE, policy) {
            Ok(auth) => auth,
            Err(_) => {
                // No business data is disclosed to an unauthenticated session.
                let _ = outgoing
                    .send((
                        ipc::Response {
                            version: ipc::VERSION,
                            request_id: request.request_id,
                            result: ipc::Reply::rejected(
                                "session_unavailable",
                                "当前会话未通过本机桌面校验；请在本机已解锁桌面打开 DeskFuse（锁屏或远程桌面使用需先在本机设置中开启）。",
                            ),
                        },
                        true,
                    ))
                    .await;
                let _ = written.recv().await;
                let _ = reader.read_u8().await;
                return Ok(());
            }
        };
        let result = {
            let mut state = control
                .desktop
                .lock()
                .map_err(|_| io::Error::other("desktop state unavailable"))?;
            if control.stopping.load(Ordering::Acquire)
                || epoch != control.epoch.load(Ordering::Acquire)
            {
                return Err(io::Error::other("session changed"));
            }
            if request.version != ipc::VERSION {
                ipc::Reply::UnsupportedVersion
            } else {
                if matches!(&request.command, ipc::Command::Snapshot) {
                    // Fresh application generation; stale requests cannot cross a reconnect.
                    epoch = control.epoch.fetch_add(1, Ordering::AcqRel) + 1;
                } else if !state.active(control) {
                    state.clear();
                }
                let mut helper_reject: Option<ipc::Reply> = None;
                let helper_identity = if helper_request {
                    match input_security::authenticate(raw as HANDLE, before, policy) {
                        Ok(identity) => Some(identity),
                        // Reply instead of silently dropping the connection: the
                        // helper otherwise only sees an EOF and cannot tell an
                        // expired helper identity from a service-side refusal.
                        Err(_) => {
                            let message = if state.input_helper.is_some() {
                                "input helper identity rejected; restart the desktop session"
                            } else {
                                "input helper authentication failed"
                            };
                            helper_reject =
                                Some(ipc::Reply::rejected("helper_unauthenticated", message));
                            None
                        }
                    }
                } else {
                    None
                };
                if helper_reject.is_none() {
                    if let Some(identity) = helper_identity {
                        if let Some(registered) = state.input_helper.as_ref() {
                            if !registered.same_process(&identity) && registered.alive() {
                                // Replacing a dead registration is normal supervision;
                                // only a live concurrent helper is a conflict.
                                helper_reject = Some(ipc::Reply::rejected(
                                    "helper_conflict",
                                    "another live input helper is already registered",
                                ));
                            }
                        } else if !matches!(&request.command, ipc::Command::RegisterInput { .. }) {
                            helper_reject = Some(ipc::Reply::rejected(
                                "helper_unregistered",
                                "input helper must register before polling",
                            ));
                        }
                        if helper_reject.is_none() {
                            state.input_helper = Some(identity);
                            // 只有声明过推送的 helper 才拿到推送连接。旧 helper 不
                            // 发这条命令，于是继续按自己的节奏轮询。
                            if matches!(&request.command, ipc::Command::EnableInputPush) {
                                state.input_push_capable = true;
                            }
                            // 重新注册或重连的 helper 接管推送，旧连接立刻失去它。
                            state.input_link = if state.input_push_capable { link } else { 0 };
                        }
                    }
                }
                if let Some(reject) = helper_reject {
                    reject
                } else {
                    // `request_id` only correlates this reply; it is not an ordering
                    // authority. Every request arrives on its own authenticated pipe,
                    // and the commands that need ordering (clipboard and platform
                    // writes) already validate their own `sequence`/`session_epoch`.
                    // A process-wide "must be newer" cursor rejected legitimate
                    // commands after client restarts and reconnects.
                    state.command(request.command, network, control, before, epoch, policy)
                }
            }
        };
        let after = security::authenticate(raw as HANDLE, policy)?;
        if control.stopping.load(Ordering::Acquire)
            || !same_session(
                before,
                after,
                epoch,
                control.epoch.load(Ordering::Acquire),
                policy,
            )
        {
            return Err(io::Error::other("session changed"));
        }
        if outgoing
            .send((
                ipc::Response {
                    version: ipc::VERSION,
                    request_id: request.request_id,
                    result,
                },
                true,
            ))
            .await
            .is_err()
            || written.recv().await.is_none()
        {
            return Ok(());
        }
        // Tokio's flush does not wait for the Windows peer to consume pipe bytes.
        // Read the acknowledgement before the next request so a peer that stopped
        // reading cannot desynchronise the stream; a closed peer ends the loop.
        if reader.read_u8().await.is_err() {
            return Ok(());
        }
        // 这次请求可能产生了投递；唤醒这条连接上的推送。
        control.signal_input_push();
    }
}

/// Hands input deliveries to the helper that owns this connection.
///
/// Push is only the fast path. The helper's poll returns the same deliveries by
/// `after_sequence`, so a dropped or late push delays input by one poll instead
/// of losing it.
async fn push_loop(
    control: Arc<Control>,
    outgoing: mpsc::Sender<(ipc::Response, bool)>,
    link: u64,
) {
    let mut changes = control.input_push.subscribe();
    // 还没有 helper 的连接不要每秒抢 20 次桌面锁：池满时那正是压垮 accept 的部分。
    let mut wait = Duration::from_millis(250);
    loop {
        let _ = timeout(wait, changes.changed()).await;
        if outgoing.is_closed() {
            return;
        }
        let batch = {
            let Ok(state) = control.desktop.lock() else {
                return;
            };
            if state.input_link != link {
                // 另一个连接接管了 helper，或者还没有 helper：这个推送者没有收件人。
                if state.input_link != 0 {
                    return;
                }
                wait = Duration::from_millis(250);
                continue;
            }
            wait = PUSH_TICK;
            state.input_push_batch()
        };
        let Some((deliveries, sequence)) = batch else {
            continue;
        };
        let frame = ipc::Response {
            version: ipc::VERSION,
            request_id: ipc::PUSH_REQUEST_ID,
            result: ipc::Reply::InputPush {
                deliveries,
                sequence,
            },
        };
        if outgoing.try_send((frame, false)).is_ok()
            && let Ok(mut state) = control.desktop.lock()
        {
            // 送出去了才算已交付。没送出去的一批由下一次轮询补上。
            state.commit_input_push(sequence);
        }
    }
}
