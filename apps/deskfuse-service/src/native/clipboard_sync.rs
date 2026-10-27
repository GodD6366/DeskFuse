//! Session-bound mac-compatible clipboard epochs and actual desktop-apply receipts.
use super::{clipboard_codec, file_transfer};
use crate::desktop_protocol::{Clipboard, Format, MAX_CONTENT};
use deskfuse_core::{
    Capabilities, ClipboardPayload, ConnectionState, DeviceId, RequestId, TrustState,
};
use deskfuse_protocol::clipboard_sync as wire;
use deskfuse_runtime::RuntimeHandle;
use deskfuse_transfer::{ClipboardFileFrame, FileBridge};
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
    delivery_ticket: Option<u64>,
}
struct Peer {
    generation: u64,
    local: Vec<u8>,
    remote: Option<Vec<u8>>,
    ready: bool,
    max_content_bytes: u64,
    hello_due: Instant,
    incoming: Option<Incoming>,
    outgoing: Option<Outgoing>,
    receipt: Option<(u64, Vec<u8>, u32)>,
}
fn data_frames(
    peer: &Peer,
    revision: u64,
    mime: &str,
    body: &[u8],
    metadata: &wire::SourceMetadata,
) -> Vec<SyncMessage> {
    let sync_id = epoch();
    let digest = Sha256::digest(body).to_vec();
    body.chunks(wire::CHUNK_BYTES)
        .enumerate()
        .map(|(index, data)| {
            SyncMessage::data(
                peer.local.clone(),
                peer.remote.clone().unwrap_or_default(),
                sync_id.clone(),
                revision,
                mime.to_owned(),
                digest.clone(),
                body.len() as u64,
                (index * wire::CHUNK_BYTES) as u64,
                data.to_vec(),
                metadata,
            )
        })
        .collect()
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
        self.send_many_gated(runtime, peer, frames, None);
    }

    fn send_many_gated(
        &mut self,
        runtime: &RuntimeHandle,
        peer: DeviceId,
        frames: Vec<SyncMessage>,
        delivery_ticket: Option<u64>,
    ) {
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
                if delivery_ticket.is_some_and(|ticket| !file_transfer::delivery_current(ticket)) {
                    break;
                }
                // One 1 MiB chunk legitimately needs several seconds on Wi-Fi;
                // a shorter budget silently cancels in-flight writes.
                match tokio::time::timeout(
                    Duration::from_secs(10),
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
                max_content_bytes: wire::MAX_BYTES as u64,
                hello_due: now,
                incoming: None,
                outgoing: None,
                receipt: None,
            });
            if !peer.ready && now >= peer.hello_due {
                peer.hello_due = now + Duration::from_secs(1);
                sends.push((
                    id,
                    SyncMessage::hello(
                        peer.local.clone(),
                        peer.remote.clone().unwrap_or_default(),
                        (MAX_CONTENT + 65536) as u64,
                    ),
                ));
            }
            if peer.incoming.as_ref().is_some_and(|incoming| {
                now.duration_since(incoming.started) > Duration::from_secs(10)
            }) {
                peer.incoming = None;
            }
            if let Some(job) = &mut peer.outgoing {
                if job
                    .delivery_ticket
                    .is_some_and(|ticket| !file_transfer::delivery_current(ticket))
                {
                    peer.outgoing = None;
                    continue;
                }
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
    pub fn receive(
        &mut self,
        runtime: &RuntimeHandle,
        id: DeviceId,
        payload: ClipboardPayload,
        files: &FileBridge,
        files_enabled: bool,
    ) {
        // Content frames use the authenticated direct file MIME channel and
        // must not enter the acknowledged text/image reassembly state machine.
        if payload.mime == deskfuse_transfer::CLIPBOARD_FILE_MIME {
            if files_enabled && let Ok(frame) = ClipboardFileFrame::decode(&payload.data) {
                files.on_frame(id, frame);
            }
            return;
        }
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
            if !frame.hello_policy_ok() {
                peer.ready = false;
                peer.incoming = None;
                peer.outgoing = None;
                self.status = Some(wire::POLICY_REJECTED_HELLO.into());
                return;
            }
            peer.max_content_bytes = frame.advertised_max_content();
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
                    let reply = SyncMessage::hello(
                        peer.local.clone(),
                        frame.source_epoch,
                        (MAX_CONTENT + 65536) as u64,
                    );
                    self.send(runtime, id, reply);
                }
            } else if frame.target_epoch.is_empty() {
                let reply = SyncMessage::hello(
                    peer.local.clone(),
                    frame.source_epoch,
                    (MAX_CONTENT + 65536) as u64,
                );
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
        if !frame.data_policy_ok() {
            peer.incoming = None;
            self.ack(runtime, id, &frame, wire::REJECTED);
            self.status = Some(wire::POLICY_REJECTED_CONTENT.into());
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
            || incoming.frame.source_metadata != frame.source_metadata
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
        // File references share the same ordered native-application cursor as
        // text/image copies. Recording an offer is not native OLE publication.
        let (formats, file_transfer_id) = if frame.mime == deskfuse_transfer::CLIPBOARD_FILE_MIME {
            if !files_enabled {
                self.ack(runtime, id, &frame, wire::REJECTED);
                return;
            }
            match ClipboardFileFrame::decode(&incoming.body) {
                Ok(file_frame @ ClipboardFileFrame::Deferred { .. }) => {
                    let ClipboardFileFrame::Deferred { transfer_id, .. } = &file_frame else {
                        unreachable!()
                    };
                    let transfer_id = transfer_id.to_hex();
                    files.accept_wire_offer(id, file_frame);
                    (Vec::new(), Some(transfer_id))
                }
                _ => {
                    self.ack(runtime, id, &frame, wire::REJECTED);
                    return;
                }
            }
        } else {
            let Ok(formats) = clipboard_codec::decode(&frame.mime, &incoming.body) else {
                self.ack(runtime, id, &frame, wire::REJECTED);
                return;
            };
            (formats, None)
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
                source_metadata: serde_json::from_slice(&frame.source_metadata)
                    .expect("validated source metadata"),
                file_transfer_id,
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
        metadata: &wire::SourceMetadata,
    ) -> Result<(), &'static str> {
        self.invalidate_copy(runtime);
        let (mime, body) = clipboard_codec::encode(formats)
            .map_err(|()| "invalid or oversized clipboard formats")?;
        if !self.peers.values().any(|p| p.ready) {
            return Err("no clipboard-ready trusted device; wait for handshake and copy again");
        }
        if !self
            .peers
            .values()
            .any(|p| p.ready && body.len() as u64 <= p.max_content_bytes)
        {
            return Err("no clipboard-ready peer accepts this content size");
        }
        self.revision = self.revision.saturating_add(1);
        let now = Instant::now();
        let mut sends = Vec::new();
        for (id, peer) in &mut self.peers {
            if !peer.ready || body.len() as u64 > peer.max_content_bytes {
                continue;
            }
            let chunks = data_frames(peer, self.revision, &mime, &body, metadata);
            for frame in &chunks {
                sends.push((*id, frame.clone()));
            }
            peer.outgoing = Some(Outgoing {
                frames: chunks,
                started: now,
                retry: now + Duration::from_secs(2),
                delivery_ticket: None,
            });
        }
        self.send_grouped(runtime, sends);
        self.status = Some("clipboard queued; waiting for remote application receipt".into());
        Ok(())
    }

    /// Publish explicit content to a single ready peer. Ordinary local copies
    /// still broadcast; this path backs the "send to device" action.
    pub fn publish_to(
        &mut self,
        runtime: &RuntimeHandle,
        peer: DeviceId,
        formats: &[Format],
    ) -> Result<(), String> {
        let (mime, body) = clipboard_codec::encode(formats)
            .map_err(|()| "剪贴板内容格式无效或超过 16 MiB 上限".to_owned())?;
        let Some(state) = self.peers.get_mut(&peer) else {
            return Err("目标设备尚未完成剪贴板握手，请稍后重试".into());
        };
        if !state.ready {
            return Err("目标设备尚未完成剪贴板握手，请稍后重试".into());
        }
        self.revision = self.revision.saturating_add(1);
        let revision = self.revision;
        if body.len() as u64 > state.max_content_bytes {
            return Err("内容超过对端接收上限".into());
        }
        let chunks = data_frames(
            state,
            revision,
            &mime,
            &body,
            &wire::SourceMetadata {
                no_history: true,
                ..Default::default()
            },
        );
        let now = Instant::now();
        state.outgoing = Some(Outgoing {
            frames: chunks.clone(),
            started: now,
            retry: now + Duration::from_secs(2),
            delivery_ticket: None,
        });
        self.send_many(runtime, peer, chunks);
        self.status = Some("shared clipboard entry; waiting for remote application".into());
        Ok(())
    }

    /// Only authenticated current-generation peers can use the file channel.
    pub fn file_peers(&self, runtime: &RuntimeHandle) -> Vec<DeviceId> {
        let view = runtime.snapshot_view();
        self.peers
            .iter()
            .filter_map(|(id, state)| {
                (state.ready
                    && runtime.control_generation(*id) == Some(state.generation)
                    && view.snapshot.peers.iter().any(|peer| {
                        peer.id == *id
                            && peer.selected_route.is_some_and(|route| {
                                peer.routes.iter().any(|r| {
                                    r.id == route
                                        && r.capabilities
                                            .contains(Capabilities::DEFERRED_FILE_CLIPBOARD)
                                })
                            })
                    }))
                .then_some(*id)
            })
            .collect()
    }

    /// Announces one deferred file offer to a single ready peer over the
    /// acknowledged sync channel. The announcement is chunked, retried and
    /// acknowledged like every other clipboard payload; the wire ACK only
    /// confirms that the peer recorded the offer, never that files moved.
    pub fn announce_file_offer(
        &mut self,
        runtime: &RuntimeHandle,
        peer: DeviceId,
        frame_data: Vec<u8>,
    ) {
        let Some(delivery_ticket) = file_transfer::delivery_ticket() else {
            self.status = Some("文件传输已关闭".into());
            return;
        };
        let Some(state) = self.peers.get_mut(&peer) else {
            return;
        };
        if !state.ready {
            return;
        }
        self.revision = self.revision.saturating_add(1);
        let revision = self.revision;
        if frame_data.len() as u64 > state.max_content_bytes {
            self.status = Some("文件引用超过对端接收上限".into());
            return;
        }
        let chunks = data_frames(
            state,
            revision,
            deskfuse_transfer::CLIPBOARD_FILE_MIME,
            &frame_data,
            &wire::SourceMetadata {
                no_history: true,
                ..Default::default()
            },
        );
        let now = Instant::now();
        state.outgoing = Some(Outgoing {
            frames: chunks.clone(),
            started: now,
            retry: now + Duration::from_secs(2),
            delivery_ticket: Some(delivery_ticket),
        });
        self.send_many_gated(runtime, peer, chunks, Some(delivery_ticket));
        self.status = Some("file offer announced; waiting for the remote request".into());
    }
}

#[cfg(test)]
mod policy_tests {
    use super::*;

    #[test]
    fn windows_chunks_carry_the_mac_source_policy_without_losing_origin() {
        let peer = Peer {
            generation: 1,
            local: vec![1; 16],
            remote: Some(vec![2; 16]),
            ready: true,
            max_content_bytes: wire::MAX_BYTES as u64,
            hello_due: Instant::now(),
            incoming: None,
            outgoing: None,
            receipt: None,
        };
        let metadata = wire::SourceMetadata {
            app_bundle_id: Some("excluded-app.exe".into()),
            device_id: Some("origin".into()),
            created_at_ms: Some(123),
            no_history: true,
            image_digest: None,
        };
        let body = vec![42; wire::CHUNK_BYTES + 17];
        let frames = data_frames(&peer, 4, clipboard_codec::BUNDLE_MIME, &body, &metadata);
        assert_eq!(frames.len(), 2);
        for frame in &frames {
            let decoded = SyncMessage::decode_checked(&frame.encode_checked().unwrap()).unwrap();
            assert!(decoded.data_policy_ok());
            assert_eq!(
                serde_json::from_slice::<wire::SourceMetadata>(&decoded.source_metadata).unwrap(),
                metadata
            );
            assert_eq!(decoded.sync_id, frames[0].sync_id);
        }
        assert_eq!(frames[1].offset, wire::CHUNK_BYTES as u64);
        let hello = SyncMessage::hello(
            peer.local,
            peer.remote.unwrap(),
            (MAX_CONTENT + 65536) as u64,
        );
        assert!(hello.hello_policy_ok());
        assert_eq!(hello.advertised_max_content(), (MAX_CONTENT + 65536) as u64);
    }
}
