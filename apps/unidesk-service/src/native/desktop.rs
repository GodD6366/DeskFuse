//! Authenticated desktop coordinator; shared state never contains private keys.
use super::{Control, Network, clipboard_sync, file_transfer, security};
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
use unidesk_core::{ConnectionState, DeviceId, Endpoint, TrustState};
use unidesk_runtime::{RuntimeCommand, RuntimeEvent};
use unidesk_transfer::{ClipboardFileFrame, FileBridge};
pub(super) struct State {
    owner: Option<ClientSession>,
    epoch: u64,
    heartbeat: Option<Instant>,
    local_sequence: u64,
    published_sequence: u64,
    last_request_id: u64,
    sync: clipboard_sync::Sync,
    files: FileBridge,
    generation: u64,
    pairing_status: Option<String>,
    last_error: Option<String>,
    input_heartbeat: Option<Instant>,
    input_topology: Option<unidesk_core::DisplayTopology>,
    input_deliveries: VecDeque<ipc::InputDelivery>,
    input_sequence: u64,
    input_reported_activation: Option<String>,
    input_last_lease: Option<(String, u64)>,
    input_available: bool,
    /// Mirror of the control session this desktop holds as an input controller.
    /// The runtime remains authoritative; this only deduplicates helper IPC.
    control: Option<ControlMirror>,
}

/// Controller-side session mirror for helper command validation.
struct ControlMirror {
    session_id: String,
    peer_id: DeviceId,
    target_display: String,
    entered: bool,
    last_batch: u64,
}

impl Default for State {
    fn default() -> Self {
        Self {
            owner: None,
            epoch: 0,
            heartbeat: None,
            local_sequence: 0,
            published_sequence: 0,
            last_request_id: 0,
            sync: clipboard_sync::Sync::default(),
            files: file_transfer::session_bridge(),
            generation: 0,
            pairing_status: None,
            last_error: None,
            input_heartbeat: None,
            input_topology: None,
            input_deliveries: VecDeque::new(),
            input_sequence: 0,
            input_reported_activation: None,
            input_last_lease: None,
            input_available: false,
            control: None,
        }
    }
}
impl State {
    pub(super) fn clear(&mut self) {
        // Cancel running file jobs first so their worker threads and peer
        // notifications stop with the session; the replacement bridge is fresh.
        self.files.reset(false);
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
        let effective = snapshot.effective_settings();
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
            local_device_id: network
                .runtime
                .snapshot_view()
                .snapshot
                .local_device_id
                .to_hex(),
            settings: Some(effective),
            controlling: self.control.as_ref().map(|control| ipc::ControlView {
                session_id: control.session_id.clone(),
                peer_id: control.peer_id.to_hex(),
                target_display: control.target_display.clone(),
            }),
            activations: snapshot
                .activations
                .iter()
                .map(|(peer, activation)| ipc::ActivationView {
                    peer_id: peer.to_hex(),
                    status: enum_string(&activation.status),
                    reason: activation.reason.clone(),
                })
                .collect(),
            remote_topologies: snapshot
                .remote_topologies
                .iter()
                .map(|(peer, topology)| ipc::RemoteTopologyView {
                    peer_id: peer.to_hex(),
                    topology: topology.clone(),
                })
                .collect(),
            links: snapshot
                .links
                .iter()
                .map(|link| ipc::LinkView {
                    peer_id: link.peer_id.to_hex(),
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
            // Without a live helper no controller session can be serviced.
            if let RuntimeEvent::InputControlGranted { session_id, .. } = &event {
                let _ =
                    network
                        .input_runtime
                        .handle
                        .try_send(RuntimeCommand::ReleaseInputControl {
                            session_id: *session_id,
                        });
            }
            return;
        }
        match event {
            RuntimeEvent::SharedInputReceived {
                session_id,
                target_display,
                state_epoch,
                events,
                peer_id,
                ..
            } => {
                // A receiver never captures locally, but a remote controller
                // entering our desktop invalidates our own control session.
                if self
                    .control
                    .as_ref()
                    .is_some_and(|mirror| mirror.peer_id == peer_id)
                {
                    self.release_control(network, "远端已接管输入控制");
                }
                self.push_input(|sequence| ipc::InputDelivery::Batch {
                    sequence,
                    session_id: session_id.to_string(),
                    target_display: target_display.to_string(),
                    state_epoch,
                    events,
                })
            }
            RuntimeEvent::InputControlGranted {
                peer_id,
                session_id,
                ..
            } => {
                // One control session at a time; entering a new one leaves the old.
                if self
                    .control
                    .as_ref()
                    .is_some_and(|mirror| mirror.session_id != session_id.to_string())
                {
                    self.release_control(network, "新的控制会话已开始");
                }
                self.control = Some(ControlMirror {
                    session_id: session_id.to_string(),
                    peer_id,
                    target_display: String::new(),
                    entered: false,
                    last_batch: 0,
                });
                self.push_input(|sequence| ipc::InputDelivery::ControlGranted {
                    sequence,
                    session_id: session_id.to_string(),
                    // The runtime grants a fixed 1500 ms initial lease; renewals
                    // ride the per-second link revision, not this value.
                    lease_ms: 1_500,
                });
            }
            RuntimeEvent::InputControlDenied { reason, .. } => {
                self.control = None;
                self.push_input(|sequence| ipc::InputDelivery::ControlDenied {
                    sequence,
                    reason: reason.chars().take(256).collect(),
                });
            }
            RuntimeEvent::InputControlReleased {
                session_id, reason, ..
            } => {
                let released = self
                    .control
                    .as_ref()
                    .is_some_and(|mirror| mirror.session_id == session_id.to_string());
                if released {
                    self.control = None;
                }
                self.push_input(|sequence| ipc::InputDelivery::ControlReleased {
                    sequence,
                    session_id: session_id.to_string(),
                    reason: reason.chars().take(256).collect(),
                });
            }
            RuntimeEvent::InputLinkUnavailable {
                reason, peer_id, ..
            } if self
                .control
                .as_ref()
                .is_some_and(|mirror| mirror.peer_id == peer_id) =>
            {
                self.release_control(network, &reason);
                self.push_input(|sequence| ipc::InputDelivery::ControlReleased {
                    sequence,
                    session_id: String::new(),
                    reason: reason.chars().take(256).collect(),
                });
            }
            RuntimeEvent::InputDeliveryInvalidated { reason }
            | RuntimeEvent::InputLinkUnavailable { reason, .. } => self.release_input(reason),
            _ => {}
        }
    }

    /// Best-effort release of the mirrored control session. The runtime stays
    /// authoritative: releasing a session that no longer exists is a no-op there.
    fn release_control(&mut self, network: &Network, reason: &str) {
        if let Some(mirror) = self.control.take() {
            if let Ok(session) = mirror.session_id.parse::<uuid::Uuid>() {
                let _ =
                    network
                        .input_runtime
                        .handle
                        .try_send(RuntimeCommand::ReleaseInputControl {
                            session_id: unidesk_core::InputSessionId(session),
                        });
            }
            self.push_input(|sequence| ipc::InputDelivery::ControlReleased {
                sequence,
                session_id: mirror.session_id,
                reason: reason.chars().take(256).collect(),
            });
        }
    }

    /// Reports native input availability to the input runtime. The helper's
    /// heartbeat is the availability signal for the ordinary desktop: without
    /// it neither controller capture nor receiver injection can run.
    fn set_input_available(&mut self, network: &Network, available: bool, reason: Option<&str>) {
        if self.input_available == available && available {
            return;
        }
        self.input_available = available;
        let _ = network
            .input_runtime
            .handle
            .try_send(RuntimeCommand::SetInputAvailability {
                available,
                reason: reason.map(str::to_owned),
            });
    }
    pub(super) fn tick(&mut self, network: &Network, control: &Control) {
        if !self.active(control) {
            // Explicitly notify peers before the state replacement drops the
            // in-flight file transfer tracking. The bridge's sink silently
            // refuses sends before the runtime attaches (service stopping).
            self.files.reset(true);
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
        let connected = connected_peer_ids(network);
        self.files.tick(&connected);
        self.sync.tick(&network.runtime);
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
            // The helper is gone: stop any controller session and stop
            // reporting this desktop as input-capable.
            self.release_control(network, "Windows input helper disconnected");
            self.set_input_available(network, false, Some("Windows input helper disconnected"));
        }
    }
    pub(super) fn event(&mut self, event: RuntimeEvent, network: &Network, control: &Control) {
        if !self.active(control) {
            self.clear();
            return;
        }
        match event {
            RuntimeEvent::ClipboardReceived { peer_id, payload } => {
                // Direct CLIPBOARD_FILE_MIME payloads carry the per-frame file
                // protocol (Request/Receipt/Ack/Cancel/stream frames); only the
                // Deferred announcement rides the acknowledged sync channel.
                if payload.mime == unidesk_transfer::CLIPBOARD_FILE_MIME {
                    if let Ok(frame) = ClipboardFileFrame::decode(&payload.data) {
                        self.files.on_frame(peer_id, frame);
                    }
                } else {
                    self.sync
                        .receive(&network.runtime, peer_id, payload, &mut self.files);
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
        self.last_error = Some("event stream overflow; clipboard session reset; copy again".into());
    }
    fn snapshot(&self, network: &Network, auth: ClientSession, epoch: u64) -> ipc::Snapshot {
        let view = network.runtime.snapshot_view();
        let usage = self.files.cache_usage();
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
            local_name: file_transfer::computer_display_name(),
            network_running: view.snapshot.sequence > 0
                && network.runtime.subscribe().has_changed().is_ok(),
            control_port: network.control_port,
            capabilities: ipc::Capabilities {
                clipboard: self.owner == Some(auth) && self.epoch == epoch,
                files: self.owner == Some(auth) && self.epoch == epoch,
                input: self
                    .input_heartbeat
                    .is_some_and(|instant| instant.elapsed() < Duration::from_secs(1)),
                updater: false,
            },
            input: self.input_state(network),
            peers,
            pending,
            file_transfers: self.files.views(),
            file_cache: ipc::FileCache {
                max_bytes: usage.max_bytes,
                used_bytes: usage.used_bytes,
            },
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
            // A fresh generation invalidates in-flight async file publication.
            self.generation = self.generation.wrapping_add(1);
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
                | ipc::Command::EnterInputControl { .. }
                | ipc::Command::SendInputEvents { .. }
                | ipc::Command::LeaveInputControl { .. }
                | ipc::Command::ReleaseInputControl { .. }
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
                if paths.is_empty() || paths.len() > 512 {
                    return ipc::Reply::rejected(
                        "invalid_request",
                        "本机文件引用数量必须在 1 到 512 之间",
                    );
                }
                let peers = file_eligible_peers(network);
                if peers.is_empty() {
                    return ipc::Reply::rejected(
                        "no_file_peer",
                        "当前没有已连接且支持跨设备文件传输的受信任设备",
                    );
                }
                // Metadata inspection can take seconds on large trees and must
                // never hold the desktop state lock; the offer registration and
                // announcement happen when the scan finishes.
                let bridge = self.files.clone();
                let generation = self.generation;
                let runtime = network.runtime.clone();
                let roots = paths
                    .into_iter()
                    .map(std::path::PathBuf::from)
                    .collect::<Vec<_>>();
                tokio::spawn(async move {
                    let published =
                        tokio::task::spawn_blocking(move || bridge.publish_local(&peers, roots))
                            .await;
                    let frames = match published {
                        Ok(Ok(frames)) => frames,
                        Ok(Err(message)) => {
                            record_bridge_error(generation, message);
                            return;
                        }
                        Err(_) => return,
                    };
                    if let Some(control) = crate::native::desktop_control()
                        && let Ok(mut state) = control.desktop.lock()
                        && state.generation == generation
                    {
                        for (peer, _transfer_id, frame) in frames {
                            if let Ok(data) = frame.encode() {
                                state.sync.announce_file_offer(&runtime, peer, data);
                            }
                        }
                    }
                });
                ipc::Reply::queued()
            }
            ipc::Command::RequestFileClipboard { transfer_id } => {
                match self.files.request_incoming(&transfer_id) {
                    Ok(()) => ipc::Reply::queued(),
                    Err(message) => ipc::Reply::rejected("file_unavailable", &message),
                }
            }
            ipc::Command::CancelFileClipboard { transfer_id } => {
                self.files.cancel_incoming(&transfer_id, "用户取消接收");
                ipc::Reply::queued()
            }
            ipc::Command::CommitFileClipboard {
                transfer_id,
                applied,
            } => match self.files.commit_incoming(&transfer_id, applied) {
                Ok(true) => ipc::Reply::queued(),
                Ok(false) => ipc::Reply::rejected("stale_transfer", "文件任务不存在或已经结束"),
                Err(message) => ipc::Reply::rejected("invalid_request", &message),
            },
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
                self.set_input_available(network, true, None);
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
                self.release_control(network, "输入代理已停止");
                self.set_input_available(network, false, Some("Windows input helper unavailable"));
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
                self.release_control(network, "Windows user reclaimed local input");
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
            ipc::Command::EnterInputControl {
                session_id,
                target_display,
                x,
                y,
            } => {
                let Some(mirror) = self.control.as_mut() else {
                    return ipc::Reply::rejected("stale_control", "no granted control session");
                };
                if mirror.session_id != session_id {
                    return ipc::Reply::rejected("stale_control", "session is no longer current");
                }
                if mirror.entered {
                    // Idempotent replay of the same entry; the runtime already holds it.
                    return ipc::Reply::queued();
                }
                mirror.entered = true;
                mirror.target_display = target_display.clone();
                match parse_input_session(&session_id) {
                    Ok(session) => {
                        match network.input_runtime.handle.try_send(
                            RuntimeCommand::EnterInputControl {
                                session_id: session,
                                target_display: unidesk_core::DisplayId::from(target_display),
                                x,
                                y,
                            },
                        ) {
                            Ok(()) => ipc::Reply::queued(),
                            Err(_) => {
                                self.release_control(network, "input runtime queue unavailable");
                                ipc::Reply::rejected("busy", "input runtime queue unavailable")
                            }
                        }
                    }
                    Err(_) => {
                        self.release_control(network, "invalid control session identifier");
                        ipc::Reply::rejected(
                            "invalid_request",
                            "invalid control session identifier",
                        )
                    }
                }
            }
            ipc::Command::SendInputEvents {
                session_id,
                target_display,
                batch,
                events,
            } => {
                let Some(mirror) = self.control.as_mut() else {
                    return ipc::Reply::rejected("stale_control", "no granted control session");
                };
                if mirror.session_id != session_id
                    || !mirror.entered
                    || mirror.target_display != target_display
                {
                    return ipc::Reply::rejected("stale_control", "session is no longer current");
                }
                if batch <= mirror.last_batch {
                    // A retransmitted helper request must never inject twice.
                    return ipc::Reply::queued();
                }
                if events.len() > 256 {
                    return ipc::Reply::rejected("invalid_request", "input batch is too large");
                }
                mirror.last_batch = batch;
                let Ok(session) = parse_input_session(&session_id) else {
                    self.release_control(network, "invalid control session identifier");
                    return ipc::Reply::rejected(
                        "invalid_request",
                        "invalid control session identifier",
                    );
                };
                match network
                    .input_runtime
                    .handle
                    .try_send(RuntimeCommand::SendSharedInput {
                        session_id: session,
                        target_display: unidesk_core::DisplayId::from(target_display),
                        events,
                    }) {
                    Ok(()) => ipc::Reply::queued(),
                    Err(_) => {
                        self.release_control(network, "input runtime queue unavailable");
                        ipc::Reply::rejected("busy", "input runtime queue unavailable")
                    }
                }
            }
            ipc::Command::LeaveInputControl { session_id } => {
                let Some(mirror) = self.control.as_mut() else {
                    return ipc::Reply::queued();
                };
                if mirror.session_id != session_id {
                    return ipc::Reply::queued();
                }
                mirror.entered = false;
                match parse_input_session(&session_id) {
                    Ok(session) => {
                        let _ = network.input_runtime.handle.try_send(
                            RuntimeCommand::LeaveInputControl {
                                session_id: session,
                            },
                        );
                    }
                    Err(_) => self.release_control(network, "invalid control session identifier"),
                }
                ipc::Reply::queued()
            }
            ipc::Command::ReleaseInputControl { session_id } => {
                if self
                    .control
                    .as_ref()
                    .is_some_and(|mirror| mirror.session_id == session_id)
                {
                    self.release_control(network, "控制已释放");
                }
                if let Ok(session) = parse_input_session(&session_id) {
                    let _ = network.input_runtime.handle.try_send(
                        RuntimeCommand::ReleaseInputControl {
                            session_id: session,
                        },
                    );
                }
                ipc::Reply::queued()
            }
            ipc::Command::EnableInputControl { peer_id } => {
                let Ok(peer) = peer_id.parse::<DeviceId>() else {
                    return ipc::Reply::rejected("invalid_request", "invalid peer identifier");
                };
                let settings = network.input_runtime.handle.input_snapshot().settings;
                let settings = controller_settings(settings, Some(peer));
                if let Err(error) = settings.validate() {
                    return ipc::Reply::rejected("invalid_request", &error.to_string());
                }
                match network
                    .input_runtime
                    .handle
                    .try_send(RuntimeCommand::SetInputSharingSettings(settings))
                {
                    Ok(()) => ipc::Reply::queued(),
                    Err(_) => ipc::Reply::rejected("busy", "input runtime queue unavailable"),
                }
            }
            ipc::Command::DisableInputControl => {
                let settings = network.input_runtime.handle.input_snapshot().settings;
                let settings = controller_settings(settings, None);
                if let Err(error) = settings.validate() {
                    return ipc::Reply::rejected("invalid_request", &error.to_string());
                }
                match network
                    .input_runtime
                    .handle
                    .try_send(RuntimeCommand::SetInputSharingSettings(settings))
                {
                    Ok(()) => ipc::Reply::queued(),
                    Err(_) => ipc::Reply::rejected("busy", "input runtime queue unavailable"),
                }
            }
            ipc::Command::RestartInputControl { peer_id } => {
                let Ok(peer) = peer_id.parse::<DeviceId>() else {
                    return ipc::Reply::rejected("invalid_request", "invalid peer identifier");
                };
                match network
                    .input_runtime
                    .handle
                    .try_send(RuntimeCommand::RestartInputActivation { peer_id: peer })
                {
                    Ok(()) => ipc::Reply::queued(),
                    Err(_) => ipc::Reply::rejected("busy", "input runtime queue unavailable"),
                }
            }
            ipc::Command::MoveControlDevice {
                device_id,
                display_id,
                x,
                y,
            } => {
                let Ok(device) = device_id.parse::<DeviceId>() else {
                    return ipc::Reply::rejected("invalid_request", "invalid device identifier");
                };
                match network
                    .input_runtime
                    .handle
                    .try_send(RuntimeCommand::MoveInputDevice {
                        device_id: device,
                        display_id: unidesk_core::DisplayId::from(display_id),
                        x,
                        y,
                    }) {
                    Ok(()) => ipc::Reply::queued(),
                    Err(_) => ipc::Reply::rejected("busy", "input runtime queue unavailable"),
                }
            }
            ipc::Command::Rename { .. } => ipc::Reply::rejected(
                "unsupported",
                "authoritative runtime rename is not yet available",
            ),
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
/// Applies a controller toggle to the effective sharing settings. Enabling
/// keeps every saved shortcut/corner/placement and only changes role and
/// participants; disabling returns to the disabled role but retains the saved
/// participant list so the next enable is a one-click action.
pub(crate) fn controller_settings(
    mut settings: unidesk_core::InputSharingSettings,
    peer: Option<DeviceId>,
) -> unidesk_core::InputSharingSettings {
    match peer {
        Some(peer) => {
            settings.enabled = true;
            settings.role = unidesk_core::InputShareRole::Controller;
            if !settings.participants.contains(&peer) {
                settings.participants.push(peer);
            }
        }
        None => {
            settings.enabled = false;
            settings.role = unidesk_core::InputShareRole::Disabled;
        }
    }
    settings
}

/// Parses a control session identifier from the helper without panicking.
fn parse_input_session(value: &str) -> Result<unidesk_core::InputSessionId, ()> {
    value
        .parse::<uuid::Uuid>()
        .map(unidesk_core::InputSessionId)
        .map_err(|_| ())
}

/// Trusted, connected peers whose selected route advertises the deferred file
/// clipboard capability — the same eligibility shape as clipboard sync, with
/// the file capability instead of the sync-version bit.
fn file_eligible_peers(network: &Network) -> Vec<DeviceId> {
    let view = network.runtime.snapshot_view();
    view.snapshot
        .peers
        .iter()
        .filter(|peer| {
            peer.trust == TrustState::Trusted
                && peer.connection == ConnectionState::Connected
                && view
                    .device_network
                    .allows(view.snapshot.local_device_id, peer.id)
                && peer.selected_route.is_some_and(|id| {
                    peer.routes.iter().any(|route| {
                        route.id == id
                            && !route.source.is_public()
                            && route
                                .capabilities
                                .contains(unidesk_core::Capabilities::DEFERRED_FILE_CLIPBOARD)
                    })
                })
        })
        .map(|peer| peer.id)
        .collect()
}

/// Trusted, connected peer identities for file-transfer lifetime tracking.
fn connected_peer_ids(network: &Network) -> Vec<DeviceId> {
    let view = network.runtime.snapshot_view();
    view.snapshot
        .peers
        .iter()
        .filter(|peer| {
            peer.trust == TrustState::Trusted && peer.connection == ConnectionState::Connected
        })
        .map(|peer| peer.id)
        .collect()
}

/// Records a bridge publication failure for the desktop only when the owning
/// session is still current; stale scans are dropped silently.
fn record_bridge_error(generation: u64, message: String) {
    if let Some(control) = crate::native::desktop_control()
        && let Ok(mut state) = control.desktop.lock()
        && state.generation == generation
    {
        state.last_error = Some(message);
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
                    | ipc::Command::EnterInputControl { .. }
                    | ipc::Command::SendInputEvents { .. }
                    | ipc::Command::LeaveInputControl { .. }
                    | ipc::Command::ReleaseInputControl { .. }
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
