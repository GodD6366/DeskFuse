//! Authenticated desktop coordinator; shared state never contains private keys.
use super::{Control, Network, clipboard_sync, remote_app, security};
use crate::{
    desktop_protocol as ipc,
    policy::{ClientSession, same_session},
    protocol,
};
use std::{
    collections::VecDeque,
    io,
    net::Ipv4Addr,
    os::windows::io::AsRawHandle,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
use tokio::{io::AsyncReadExt, net::windows::named_pipe::NamedPipeServer};
use unidesk_core::Endpoint;
use unidesk_runtime::{RuntimeCommand, RuntimeEvent};
use unidesk_transfer::FileBridge;
#[derive(Default)]
pub(super) struct State {
    owner: Option<ClientSession>,
    epoch: u64,
    heartbeat: Option<Instant>,
    local_sequence: u64,
    published_sequence: u64,
    last_request_id: u64,
    sync: clipboard_sync::Sync,
    pairing_status: Option<String>,
    last_error: Option<String>,
    input_heartbeat: Option<Instant>,
    input_topology: Option<unidesk_core::DisplayTopology>,
    input_deliveries: VecDeque<ipc::InputDelivery>,
    input_sequence: u64,
    input_reported_activation: Option<String>,
    input_last_lease: Option<(String, u64)>,
    file_bridge: Option<FileBridge>,
}
impl State {
    pub(super) fn clear(&mut self) {
        if let Some(files) = self.file_bridge.take() {
            files.reset(true);
        }
        *self = Self::default();
    }
    fn active(&self, control: &Control) -> bool {
        !control.stopping.load(Ordering::Acquire)
            && self.epoch == control.epoch.load(Ordering::Acquire)
            && self
                .heartbeat
                .is_some_and(|instant| instant.elapsed() < Duration::from_secs(5))
            && self
                .owner
                .is_some_and(|owner| security::session_still_active(owner))
    }
    fn input_active(&self, control: &Control) -> bool {
        self.active(control)
            && self
                .input_heartbeat
                .is_some_and(|instant| instant.elapsed() < Duration::from_secs(1))
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
    }
    fn input_state(&self, network: &Network) -> ipc::InputState {
        let snapshot = network.input_runtime.handle.input_snapshot();
        let managed = snapshot.managed_control.as_ref();
        ipc::InputState {
            helper_connected: self
                .input_heartbeat
                .is_some_and(|instant| instant.elapsed() < Duration::from_secs(1)),
            managed: managed.is_some(),
            status: managed.map_or_else(|| "standby".into(), |value| enum_string(&value.status)),
            reason: managed.and_then(|value| value.reason.clone()),
            controller_id: managed.map(|value| value.controller.to_string()),
            display_count: snapshot
                .local_topology
                .as_ref()
                .map_or(0, |topology| topology.displays.len()),
            local_device_id: network.runtime.snapshot_view().snapshot.local_device_id.to_string(),
            settings: Some(snapshot.effective_settings()),
            controlling: None,
            activations: snapshot
                .activations
                .iter()
                .map(|(peer, state)| ipc::ActivationView {
                    peer_id: peer.to_string(),
                    status: enum_string(&state.status),
                    reason: state.reason.clone(),
                })
                .collect(),
            remote_topologies: snapshot
                .remote_topologies
                .iter()
                .map(|(peer, topology)| ipc::RemoteTopologyView {
                    peer_id: peer.to_string(),
                    topology: topology.clone(),
                })
                .collect(),
            links: snapshot
                .links
                .iter()
                .map(|link| ipc::LinkView {
                    peer_id: link.peer_id.to_string(),
                    ready: link.ready,
                    control_session: link.control_session.map(|id| id.to_string()),
                    lease_revision: link.lease_revision,
                    motion_mode: link.motion_mode.clone(),
                })
                .collect(),
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
            RuntimeEvent::InputControlReleased { reason, .. }
            | RuntimeEvent::InputDeliveryInvalidated { reason }
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
                && managed.status != unidesk_core::InputActivationStatus::Unavailable
            {
                let _ = network.input_runtime.handle.try_send(
                    RuntimeCommand::SetManagedInputAvailability {
                        activation_id: managed.activation_id,
                        status: unidesk_core::InputActivationStatus::Unavailable,
                        reason: Some("Windows interactive desktop session unavailable".into()),
                    },
                );
            }
            self.clear();
            return;
        }
        self.sync.tick(&network.runtime);
        if let Some(files) = self.file_bridge.as_ref() {
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
                && managed.status != unidesk_core::InputActivationStatus::PausedByUser
                && snapshot.local_topology.is_some()
            {
                let activation = managed.activation_id.to_string();
                if self.input_reported_activation.as_deref() != Some(&activation) {
                    let _ = network.input_runtime.handle.try_send(
                        RuntimeCommand::SetManagedInputAvailability {
                            activation_id: managed.activation_id,
                            status: unidesk_core::InputActivationStatus::Ready,
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
                        status: unidesk_core::InputActivationStatus::Unavailable,
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
                if let Some(files) = self.file_bridge.as_ref() {
                    self.sync.receive(&network.runtime, peer_id, payload, files)
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
            RuntimeEvent::Error(_) => {
                self.last_error =
                    Some("network command failed; inspect device connection and retry".into())
            }
            RuntimeEvent::ClipboardSent { result: Err(_), .. } => {
                self.last_error =
                    Some("clipboard transport failed; copy again after reconnection".into())
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
    fn snapshot(&self, network: &Network, auth: ClientSession, epoch: u64) -> ipc::Snapshot {
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
            local_device_id: view.snapshot.local_device_id.to_string(),
            local_name: "uniDesk Windows".into(),
            network_running: view.snapshot.sequence > 0
                && network.runtime.subscribe().has_changed().is_ok(),
            control_port: network.control_port,
            capabilities: ipc::Capabilities {
                clipboard: self.owner == Some(auth) && self.epoch == epoch,
                files: self.owner == Some(auth)
                    && self.epoch == epoch
                    && self.file_bridge.is_some()
                    && !self.sync.file_peers(&network.runtime).is_empty(),
                input: self
                    .input_heartbeat
                    .is_some_and(|instant| instant.elapsed() < Duration::from_secs(1)),
                updater: false,
                remote_app: matches!(
                    remote_app::availability(),
                    remote_app::Availability::Available
                ),
            },
            input: self.input_state(network),
            peers,
            pending,
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
            last_error: self.last_error.clone().or_else(|| {
                view.device_network
                    .integrity_error
                    .as_ref()
                    .map(|_| "device network integrity check failed".into())
            }),
        }
    }
    fn command(
        &mut self,
        command: ipc::Command,
        network: &Network,
        auth: ClientSession,
        epoch: u64,
    ) -> ipc::Reply {
        if matches!(&command, ipc::Command::Snapshot) {
            self.clear();
            self.owner = Some(auth);
            self.epoch = epoch;
            self.heartbeat = Some(Instant::now());
            self.file_bridge = Some(crate::native::file_transfer::session_bridge());
            self.sync.tick(&network.runtime);
            return ipc::Reply::Snapshot(self.snapshot(network, auth, epoch));
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
        match command {
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
                let (clipboard, clipboard_sequence) = self.sync.poll(after_sequence);
                ipc::Reply::Poll {
                    snapshot: self.snapshot(network, auth, epoch),
                    clipboard,
                    clipboard_sequence,
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
                match self.sync.publish(&network.runtime, &formats) {
                    Ok(()) => ipc::Reply::queued(),
                    Err(message) => ipc::Reply::rejected("clipboard_unavailable", message),
                }
            }
            ipc::Command::PublishFileClipboard {
                session_epoch,
                sequence,
                paths,
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
                self.local_sequence = sequence;
                self.published_sequence = sequence;
                self.sync.invalidate_copy(&network.runtime);
                let Some(files) = self.file_bridge.as_ref() else {
                    return ipc::Reply::rejected("files_unavailable", "文件通道尚未就绪");
                };
                let peers = self.sync.file_peers(&network.runtime);
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
                    files.cancel_incoming(&transfer_id, "Windows desktop cancelled file transfer");
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
                self.input_heartbeat = Some(Instant::now());
                self.input_topology = Some(topology.clone());
                self.input_reported_activation = None;
                self.input_last_lease = None;
                self.release_input("Windows input helper generation opened");
                match network
                    .input_runtime
                    .handle
                    .try_send(RuntimeCommand::PublishDisplayTopology(topology))
                {
                    Ok(()) => ipc::Reply::queued(),
                    Err(_) => ipc::Reply::rejected("busy", "input runtime queue unavailable"),
                }
            }
            ipc::Command::PollInput { after_sequence } => {
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
                            status: unidesk_core::InputActivationStatus::Unavailable,
                            reason: Some(message.chars().take(256).collect()),
                        },
                    );
                }
                self.input_reported_activation = None;
                ipc::Reply::queued()
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
            ipc::Command::Rename { .. } => ipc::Reply::rejected(
                "unsupported",
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
                let apps = remote_app::list_remote_apps()
                    .into_iter()
                    .map(remote_app_info)
                    .collect();
                ipc::Reply::RemoteApps {
                    available,
                    reason,
                    apps,
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
                if name.trim().is_empty() || path.trim().is_empty() {
                    return ipc::Reply::rejected(
                        "invalid_request",
                        "RemoteApp 名称与可执行文件路径不能为空",
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
                    Err(_) => {
                        ipc::Reply::rejected("remote_app_unavailable", "无法写入 RemoteApp 注册表")
                    }
                }
            }
            ipc::Command::UnregisterRemoteApp { name } => match remote_app::unregister_app(&name) {
                Ok(()) => ipc::Reply::queued(),
                Err(_) => {
                    ipc::Reply::rejected("remote_app_unavailable", "无法移除 RemoteApp 注册项")
                }
            },
            ipc::Command::RemoteAppConnection { name } => {
                let Some(entry) = remote_app::list_remote_apps()
                    .into_iter()
                    .find(|entry| entry.name == name)
                else {
                    return ipc::Reply::rejected("not_found", "该应用尚未注册为 RemoteApp");
                };
                ipc::Reply::RemoteAppConnection {
                    rdp: remote_app::rdp_file(&entry),
                }
            }
            other => match runtime_command(other) {
                Some(command) => {
                    self.last_error = None;
                    match network.runtime.try_send(command) {
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

fn enum_string(value: &impl serde::Serialize) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".into())
}
fn runtime_command(command: ipc::Command) -> Option<RuntimeCommand> {
    Some(match command {
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
        ipc::Command::Revoke { peer_id } => RuntimeCommand::RevokePeer {
            peer_id: peer_id.parse().ok()?,
        },
        _ => return None,
    })
}
pub(super) async fn handle(
    mut pipe: NamedPipeServer,
    network: &Network,
    control: &Control,
) -> io::Result<()> {
    let request: ipc::Request = protocol::read_frame_limited(&mut pipe, ipc::MAX_FRAME).await?;
    let mut epoch = control.epoch.load(Ordering::Acquire);
    let before = match security::authenticate(pipe.as_raw_handle()) {
        Ok(auth) => auth,
        Err(_) => {
            // No business data is disclosed to an unauthenticated session.
            protocol::write_frame_limited(
                &mut pipe,
                &ipc::Response {
                    version: ipc::VERSION,
                    request_id: request.request_id,
                    result: ipc::Reply::rejected(
                        "session_unavailable",
                        "当前会话未通过本机解锁桌面校验；请在本机已解锁桌面打开 uniDesk。",
                    ),
                },
                ipc::MAX_FRAME,
            )
            .await?;
            let _ = pipe.read_u8().await;
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
            let helper_request = matches!(
                &request.command,
                ipc::Command::RegisterInput { .. }
                    | ipc::Command::PollInput { .. }
                    | ipc::Command::InputFault { .. }
            );
            // Helper polling has an independent process-local request sequence.
            // Its commands are idempotent and remain bound to the authenticated
            // desktop generation, so they do not share the GUI monotonic cursor.
            if !helper_request
                && !matches!(&request.command, ipc::Command::Snapshot)
                && request.request_id <= state.last_request_id
            {
                ipc::Reply::rejected(
                    "stale_request",
                    "request identifier is not newer than the previous command",
                )
            } else {
                let result = state.command(request.command, network, before, epoch);
                if !helper_request {
                    state.last_request_id = request.request_id;
                }
                result
            }
        }
    };
    let after = security::authenticate(pipe.as_raw_handle())?;
    if control.stopping.load(Ordering::Acquire)
        || !same_session(before, after, epoch, control.epoch.load(Ordering::Acquire))
    {
        return Err(io::Error::other("session changed"));
    }
    protocol::write_frame_limited(
        &mut pipe,
        &ipc::Response {
            version: ipc::VERSION,
            request_id: request.request_id,
            result,
        },
        ipc::MAX_FRAME,
    )
    .await?;
    // Tokio's flush does not wait for the Windows peer to consume pipe bytes.
    // Keep this instance alive until the desktop acknowledges reading the frame
    // (or an old client closes). The enclosing request timeout bounds this wait.
    let _ = pipe.read_u8().await;
    Ok(())
}
