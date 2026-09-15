//! Authenticated desktop coordinator; shared state never contains private keys.
use super::{Control, Network, clipboard_sync, security};
use crate::{
    desktop_protocol as ipc,
    policy::{ClientSession, same_session},
    protocol,
};
use std::{
    io,
    net::Ipv4Addr,
    os::windows::io::AsRawHandle,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
use tokio::{io::AsyncReadExt, net::windows::named_pipe::NamedPipeServer};
use unidesk_core::Endpoint;
use unidesk_runtime::{RuntimeCommand, RuntimeEvent};
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
}
impl State {
    pub(super) fn clear(&mut self) {
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
    pub(super) fn tick(&mut self, network: &Network, control: &Control) {
        if !self.active(control) {
            self.clear();
            return;
        }
        self.sync.tick(&network.runtime);
    }
    pub(super) fn event(&mut self, event: RuntimeEvent, network: &Network, control: &Control) {
        if !self.active(control) {
            self.clear();
            return;
        }
        match event {
            RuntimeEvent::ClipboardReceived { peer_id, payload } => {
                self.sync.receive(&network.runtime, peer_id, payload)
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
                files: false,
                input: false,
                updater: false,
            },
            peers,
            pending,
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
        if matches!(command, ipc::Command::Snapshot) {
            self.clear();
            self.owner = Some(auth);
            self.epoch = epoch;
            self.heartbeat = Some(Instant::now());
            self.sync.tick(&network.runtime);
            return ipc::Reply::Snapshot(self.snapshot(network, auth, epoch));
        }
        let owner_matches = self.owner == Some(auth) && self.epoch == epoch;
        if !owner_matches {
            return ipc::Reply::rejected("session_expired", "open a fresh desktop snapshot");
        }
        self.heartbeat = Some(Instant::now());
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
            if matches!(request.command, ipc::Command::Snapshot) {
                // Fresh application generation; stale requests cannot cross a reconnect.
                epoch = control.epoch.fetch_add(1, Ordering::AcqRel) + 1;
            } else if !state.active(control) {
                state.clear();
            }
            // No await in command admission: session epoch checked with lock held.
            if !matches!(request.command, ipc::Command::Snapshot)
                && request.request_id <= state.last_request_id
            {
                ipc::Reply::rejected(
                    "stale_request",
                    "request identifier is not newer than the previous command",
                )
            } else {
                let result = state.command(request.command, network, before, epoch);
                state.last_request_id = request.request_id;
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
