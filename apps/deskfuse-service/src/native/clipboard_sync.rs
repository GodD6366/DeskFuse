//! Session-bound mac-compatible clipboard epochs and actual desktop-apply receipts.
use super::clipboard_codec;
use crate::desktop_protocol::{Clipboard, Format, MAX_CONTENT};
use deskfuse_core::{
    Capabilities, ClipboardPayload, ConnectionState, DeviceId, RequestId, TrustState,
};
use deskfuse_protocol::clipboard_sync as wire;
use deskfuse_runtime::RuntimeHandle;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use tokio::task::JoinHandle;
use wire::SyncMessage;
fn epoch() -> Vec<u8> {
    RequestId::new().as_uuid().as_bytes().to_vec()
}
struct Incoming {
    frame: SyncMessage,
    body: Vec<u8>,
    started: Instant,
}
struct Outgoing {
    frames: Vec<SyncMessage>,
    started: Instant,
    retry: Instant,
}
struct Peer {
    generation: u64,
    local: Vec<u8>,
    remote: Option<Vec<u8>>,
    ready: bool,
    hello_due: Instant,
    incoming: Option<Incoming>,
    outgoing: Option<Outgoing>,
    receipt: Option<(u64, Vec<u8>, u32)>,
}
struct Apply {
    clipboard: Clipboard,
    peer: DeviceId,
    frame: SyncMessage,
    started: Instant,
}
#[derive(Default)]
pub struct Sync {
    peers: HashMap<DeviceId, Peer>,
    apply: Option<Apply>,
    cursor: u64,
    revision: u64,
    helpers: Vec<JoinHandle<()>>,
    pub status: Option<String>,
}
impl Drop for Sync {
    fn drop(&mut self) {
        for task in &self.helpers {
            task.abort();
        }
    }
}
impl Sync {
    fn send(&mut self, runtime: &RuntimeHandle, peer: DeviceId, frame: SyncMessage) {
        self.send_many(runtime, peer, vec![frame]);
    }
    fn send_many(&mut self, runtime: &RuntimeHandle, peer: DeviceId, frames: Vec<SyncMessage>) {
        self.helpers.retain(|task| !task.is_finished());
        if self.helpers.len() >= 64 {
            self.status = Some("clipboard transport busy".into());
            return;
        }
        let Some(generation) = runtime.control_generation(peer) else {
            return;
        };
        let payloads = frames
            .into_iter()
            .filter_map(|frame| {
                ClipboardPayload::new(wire::MIME, frame.encode_checked().ok()?, frame.revision).ok()
            })
            .collect::<Vec<_>>();
        let runtime = runtime.clone();
        // Every chunk in one job is written in order; aborting the desktop state
        // drops the tracked writer receipt and cancels remaining queued chunks.
        self.helpers.push(tokio::spawn(async move {
            for payload in payloads {
                match tokio::time::timeout(
                    Duration::from_secs(3),
                    runtime.send_clipboard_on_session(peer, generation, payload),
                )
                .await
                {
                    Ok(Ok(())) => {}
                    _ => break,
                }
            }
        }));
    }
    fn send_grouped(&mut self, runtime: &RuntimeHandle, sends: Vec<(DeviceId, SyncMessage)>) {
        let mut groups: HashMap<DeviceId, Vec<SyncMessage>> = HashMap::new();
        for (id, frame) in sends {
            groups.entry(id).or_default().push(frame);
        }
        for (id, frames) in groups {
            self.send_many(runtime, id, frames);
        }
    }
    pub fn tick(&mut self, runtime: &RuntimeHandle) {
        let view = runtime.snapshot_view();
        let mut eligible = HashMap::new();
        for peer in &view.snapshot.peers {
            if peer.trust == TrustState::Trusted
                && peer.connection == ConnectionState::Connected
                && view
                    .device_network
                    .allows(view.snapshot.local_device_id, peer.id)
                && peer.selected_route.is_some_and(|id| {
                    peer.routes.iter().any(|r| {
                        r.id == id
                            && !r.source.is_public()
                            && r.capabilities.contains(Capabilities::CLIPBOARD_SYNC_V2)
                    })
                })
                && let Some(generation) = runtime.control_generation(peer.id)
            {
                eligible.insert(peer.id, generation);
            }
        }
        self.peers
            .retain(|id, peer| eligible.get(id) == Some(&peer.generation));
        if self
            .apply
            .as_ref()
            .is_some_and(|apply| !self.peers.contains_key(&apply.peer))
        {
            self.apply = None;
        }
        let now = Instant::now();
        let mut sends = Vec::new();
        for (id, generation) in eligible.into_iter().take(16) {
            let peer = self.peers.entry(id).or_insert_with(|| Peer {
                generation,
                local: epoch(),
                remote: None,
                ready: false,
                hello_due: now,
                incoming: None,
                outgoing: None,
                receipt: None,
            });
            if !peer.ready && now >= peer.hello_due {
                peer.hello_due = now + Duration::from_secs(1);
                sends.push((
                    id,
                    SyncMessage {
                        kind: wire::HELLO,
                        source_epoch: peer.local.clone(),
                        target_epoch: peer.remote.clone().unwrap_or_default(),
                        ..Default::default()
                    },
                ));
            }
            if peer.incoming.as_ref().is_some_and(|incoming| {
                now.duration_since(incoming.started) > Duration::from_secs(10)
            }) {
                peer.incoming = None;
            }
            if let Some(job) = &mut peer.outgoing {
                if now.duration_since(job.started) > Duration::from_secs(12) {
                    peer.outgoing = None;
                    self.status = Some("clipboard acknowledgement timed out; copy again".into());
                } else if now >= job.retry {
                    job.retry = now + Duration::from_secs(2);
                    for frame in &job.frames {
                        sends.push((id, frame.clone()));
                    }
                }
            }
        }
        if self
            .apply
            .as_ref()
            .is_some_and(|a| now.duration_since(a.started) > Duration::from_secs(10))
        {
            if let Some(apply) = self.apply.take() {
                self.ack(runtime, apply.peer, &apply.frame, wire::REJECTED);
            }
            self.status = Some("desktop clipboard application timed out".into());
        }
        self.send_grouped(runtime, sends);
    }
    fn ack(&mut self, runtime: &RuntimeHandle, id: DeviceId, frame: &SyncMessage, outcome: u32) {
        let Some(peer) = self.peers.get_mut(&id) else {
            return;
        };
        if peer
            .receipt
            .as_ref()
            .is_none_or(|(revision, _, _)| frame.revision >= *revision)
        {
            peer.receipt = Some((frame.revision, frame.sync_id.clone(), outcome));
        }
        self.send(
            runtime,
            id,
            SyncMessage {
                kind: wire::ACK,
                source_epoch: frame.target_epoch.clone(),
                target_epoch: frame.source_epoch.clone(),
                sync_id: frame.sync_id.clone(),
                revision: frame.revision,
                outcome,
                ..Default::default()
            },
        );
    }
    pub fn receive(&mut self, runtime: &RuntimeHandle, id: DeviceId, payload: ClipboardPayload) {
        if payload.mime != wire::MIME {
            return;
        }
        let Ok(frame) = SyncMessage::decode_checked(&payload.data) else {
            return;
        };
        let Some(peer) = self.peers.get_mut(&id) else {
            return;
        };
        if runtime.control_generation(id) != Some(peer.generation) {
            return;
        }
        if frame.kind == wire::HELLO {
            let matched = frame.target_epoch == peer.local;
            let different = peer.remote.as_ref() != Some(&frame.source_epoch);
            if matched || peer.remote.is_none() {
                if different {
                    peer.remote = Some(frame.source_epoch.clone());
                    peer.ready = false;
                    peer.incoming = None;
                    peer.outgoing = None;
                    peer.receipt = None;
                    if self.apply.as_ref().is_some_and(|a| a.peer == id) {
                        self.apply = None;
                    }
                }
                let was_ready = peer.ready;
                peer.ready |= matched;
                if different || !was_ready || !matched {
                    let reply = SyncMessage {
                        kind: wire::HELLO,
                        source_epoch: peer.local.clone(),
                        target_epoch: frame.source_epoch,
                        ..Default::default()
                    };
                    self.send(runtime, id, reply);
                }
            } else if frame.target_epoch.is_empty() {
                let reply = SyncMessage {
                    kind: wire::HELLO,
                    source_epoch: peer.local.clone(),
                    target_epoch: frame.source_epoch,
                    ..Default::default()
                };
                self.send(runtime, id, reply);
            }
            return;
        }
        if !peer.ready
            || frame.target_epoch != peer.local
            || peer.remote.as_ref() != Some(&frame.source_epoch)
        {
            return;
        }
        if frame.kind == wire::ACK {
            if peer.outgoing.as_ref().is_some_and(|job| {
                job.frames[0].sync_id == frame.sync_id && job.frames[0].revision == frame.revision
            }) {
                peer.outgoing = None;
                self.status = Some(
                    match frame.outcome {
                        wire::APPLIED => "clipboard applied by remote desktop",
                        wire::SUPERSEDED => "clipboard superseded by newer copy",
                        _ => "clipboard rejected by remote desktop",
                    }
                    .into(),
                );
            }
            return;
        }
        if let Some((revision, sync_id, outcome)) = &peer.receipt {
            if frame.revision <= *revision {
                let outcome = if frame.revision == *revision && frame.sync_id == *sync_id {
                    *outcome
                } else {
                    wire::SUPERSEDED
                };
                self.ack(runtime, id, &frame, outcome);
                return;
            }
        }
        if self
            .apply
            .as_ref()
            .is_some_and(|a| a.peer == id && a.frame.sync_id == frame.sync_id)
        {
            return;
        }
        if self
            .apply
            .as_ref()
            .is_some_and(|a| a.peer == id && frame.revision < a.frame.revision)
        {
            self.ack(runtime, id, &frame, wire::SUPERSEDED);
            return;
        }
        if frame.total > (MAX_CONTENT + 65536) as u64 {
            self.ack(runtime, id, &frame, wire::REJECTED);
            return;
        }
        if peer
            .incoming
            .as_ref()
            .is_some_and(|i| frame.revision < i.frame.revision)
        {
            self.ack(runtime, id, &frame, wire::SUPERSEDED);
            return;
        }
        if frame.offset == 0 {
            peer.incoming = Some(Incoming {
                frame: frame.clone(),
                body: Vec::new(),
                started: Instant::now(),
            });
        }
        let Some(incoming) = &mut peer.incoming else {
            return;
        };
        if incoming.frame.sync_id != frame.sync_id
            || incoming.frame.revision != frame.revision
            || incoming.frame.digest != frame.digest
            || incoming.frame.mime != frame.mime
            || incoming.frame.total != frame.total
            || incoming.body.len() as u64 != frame.offset
        {
            return;
        }
        incoming.body.extend_from_slice(&frame.data);
        if incoming.body.len() as u64 != frame.total {
            return;
        }
        let incoming = peer.incoming.take().expect("incoming was checked");
        if Sha256::digest(&incoming.body).as_slice() != frame.digest {
            self.ack(runtime, id, &frame, wire::REJECTED);
            return;
        }
        let Ok(formats) = clipboard_codec::decode(&frame.mime, &incoming.body) else {
            self.ack(runtime, id, &frame, wire::REJECTED);
            return;
        };
        if let Some(previous) = self.apply.take() {
            self.ack(runtime, previous.peer, &previous.frame, wire::SUPERSEDED);
        }
        self.cursor = self.cursor.saturating_add(1);
        self.apply = Some(Apply {
            clipboard: Clipboard {
                sequence: self.cursor,
                peer_id: id.to_string(),
                formats,
            },
            peer: id,
            frame,
            started: Instant::now(),
        });
        self.status = Some("clipboard awaiting desktop application".into());
    }
    pub fn poll(&self, after: u64) -> (Option<Clipboard>, u64) {
        (
            self.apply
                .as_ref()
                .filter(|a| a.clipboard.sequence > after)
                .map(|a| a.clipboard.clone()),
            self.cursor,
        )
    }
    pub fn commit(&mut self, runtime: &RuntimeHandle, sequence: u64, applied: bool) -> bool {
        if !self
            .apply
            .as_ref()
            .is_some_and(|a| a.clipboard.sequence == sequence)
        {
            return false;
        }
        let apply = self.apply.take().expect("apply was checked");
        self.ack(
            runtime,
            apply.peer,
            &apply.frame,
            if applied {
                wire::APPLIED
            } else {
                wire::REJECTED
            },
        );
        self.status = Some(
            if applied {
                "clipboard applied by Windows desktop"
            } else {
                "Windows clipboard application rejected"
            }
            .into(),
        );
        true
    }
    /// Retire every remote copy observed before a native local change. Persist
    /// SUPERSEDED receipts so retransmission of offset zero cannot revive it.
    pub fn invalidate_copy(&mut self, runtime: &RuntimeHandle) {
        // Retire older local sends as well. Drop cancels tracked writer jobs;
        // acknowledgements collected below are sent as fresh helpers.
        for task in self.helpers.drain(..) {
            task.abort();
        }
        let mut retired = Vec::new();
        for (id, peer) in &mut self.peers {
            peer.outgoing = None;
            if let Some(incoming) = peer.incoming.take() {
                retired.push((*id, incoming.frame));
            }
        }
        if let Some(apply) = self.apply.take() {
            retired.push((apply.peer, apply.frame));
        }
        for (id, frame) in retired {
            self.ack(runtime, id, &frame, wire::SUPERSEDED);
        }
        self.status = Some("local clipboard changed; older remote copies superseded".into());
    }
    pub fn publish(
        &mut self,
        runtime: &RuntimeHandle,
        formats: &[Format],
    ) -> Result<(), &'static str> {
        self.invalidate_copy(runtime);
        let (mime, body) = clipboard_codec::encode(formats)
            .map_err(|()| "invalid or oversized clipboard formats")?;
        if !self.peers.values().any(|p| p.ready) {
            return Err("no clipboard-ready trusted device; wait for handshake and copy again");
        }
        self.revision = self.revision.saturating_add(1);
        let digest = Sha256::digest(&body).to_vec();
        let now = Instant::now();
        let mut sends = Vec::new();
        for (id, peer) in &mut self.peers {
            if !peer.ready {
                continue;
            }
            let sync_id = epoch();
            let chunks = body
                .chunks(wire::CHUNK_BYTES)
                .enumerate()
                .map(|(index, data)| SyncMessage {
                    kind: wire::DATA,
                    source_epoch: peer.local.clone(),
                    target_epoch: peer.remote.clone().unwrap_or_default(),
                    sync_id: sync_id.clone(),
                    revision: self.revision,
                    mime: mime.clone(),
                    digest: digest.clone(),
                    total: body.len() as u64,
                    offset: (index * wire::CHUNK_BYTES) as u64,
                    data: data.to_vec(),
                    ..Default::default()
                })
                .collect::<Vec<_>>();
            for frame in &chunks {
                sends.push((*id, frame.clone()));
            }
            peer.outgoing = Some(Outgoing {
                frames: chunks,
                started: now,
                retry: now + Duration::from_secs(2),
            });
        }
        self.send_grouped(runtime, sends);
        self.status = Some("clipboard queued; waiting for remote application receipt".into());
        Ok(())
    }
}
