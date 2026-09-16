//! Independent, resumable library replication. This transport never touches NSPasteboard.
//! Mount beside clipboard_sync in tokio_bridge; call refresh on its periodic tick and
//! receive before the ordinary clipboard decoder when MIME matches.
use super::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub(super) const MIME: &str = "application/x-unidesk-library-sync-v1";
const CHUNK: usize = 128 * 1024;
const MAX_BLOB: usize = 257 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Frame {
    Hello {
        version: u32,
    },
    Pull {
        after: u64,
    },
    Operations {
        after: u64,
        next: u64,
        operations: Vec<Value>,
    },
    BlobRequest {
        hash: String,
    },
    Blob {
        hash: String,
        total: usize,
        offset: usize,
        data: String,
    },
    Missing {
        hash: String,
    },
}
struct PeerWorker {
    generation: u64,
    tx: tokio::sync::mpsc::Sender<Vec<u8>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for PeerWorker {
    fn drop(&mut self) {
        self.task.abort();
    }
}
#[derive(Default)]
pub(super) struct Sync {
    peers: HashMap<DeviceId, PeerWorker>,
}
impl Sync {
    pub fn refresh(&mut self, runtime: &RuntimeHandle, ui: &UnboundedSender<UiEvent>) {
        let trusted = trusted_connected_peer_ids(&runtime.snapshot_view());
        self.peers.retain(|id, p| {
            trusted.contains(id)
                && runtime.control_generation(*id) == Some(p.generation)
                && !p.task.is_finished()
        });
        for peer in trusted {
            if self.peers.contains_key(&peer) {
                continue;
            }
            let Some(generation) = runtime.control_generation(peer) else {
                continue;
            };
            let (tx, rx) = tokio::sync::mpsc::channel(16);
            let runtime = runtime.clone();
            let ui = ui.clone();
            let task = tokio::spawn(async move {
                if let Err(error) = worker(peer, generation, runtime, rx).await {
                    let _ = ui.unbounded_send(UiEvent::ClipboardDelivery {
                        peer_id: peer,
                        phase: format!("列表同步失败：{error}"),
                    });
                }
            });
            self.peers.insert(
                peer,
                PeerWorker {
                    generation,
                    tx,
                    task,
                },
            );
        }
    }
    pub fn receive(&self, peer: DeviceId, bytes: &[u8]) {
        // Keep untrusted JSON bounded before allocation. Queue has one trusted sender
        // per session; worker validation applies a separate decoded-blob bound.
        if bytes.len() > CHUNK * 2 + 64 * 1024 {
            return;
        }
        if let Some(p) = self.peers.get(&peer) {
            let _ = p.tx.try_send(bytes.to_vec());
        }
    }
}
async fn send(
    runtime: &RuntimeHandle,
    peer: DeviceId,
    generation: u64,
    frame: Frame,
) -> Result<(), String> {
    let bytes = serde_json::to_vec(&frame).map_err(|e| e.to_string())?;
    runtime
        .send_clipboard_on_session(
            peer,
            generation,
            ClipboardPayload::new(MIME, bytes, 1).map_err(|e| e.to_string())?,
        )
        .await
        .map_err(|e| e.to_string())
}
fn valid_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())
}
fn blob_hashes(value: &Value, found: &mut std::collections::BTreeSet<String>) {
    match value {
        Value::Object(values) => {
            for (key, v) in values {
                if key == "blob_hash" {
                    if let Some(hash) = v.as_str().filter(|hash| valid_hash(hash)) {
                        found.insert(hash.to_owned());
                    }
                } else {
                    blob_hashes(v, found);
                }
            }
        }
        Value::Array(values) => {
            for v in values {
                blob_hashes(v, found);
            }
        }
        _ => {}
    }
}
struct BlobTask(Option<tokio::task::JoinHandle<()>>);
impl Drop for BlobTask {
    fn drop(&mut self) {
        if let Some(task) = &self.0 {
            task.abort();
        }
    }
}
async fn worker(
    peer: DeviceId,
    generation: u64,
    runtime: RuntimeHandle,
    mut rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
) -> Result<(), String> {
    let library = crate::clipboard_library::global()?;
    let mut timer = tokio::time::interval(Duration::from_secs(3));
    let mut blob_task = BlobTask(None);
    let mut receive_updated = std::time::Instant::now();
    let mut ready = false;
    let mut hello_attempts = 0;
    let mut cursor = 0;
    let mut pending = std::collections::BTreeSet::<String>::new();
    let mut receiving: Option<(String, usize, Vec<u8>)> = None;
    loop {
        if runtime.control_generation(peer) != Some(generation) {
            return Ok(());
        }
        tokio::select! {
            _ = timer.tick() => {
                if !ready {
                    hello_attempts += 1;
                    if hello_attempts > 10 { return Err("对端尚未响应列表同步协议，请更新两端应用".into()); }
                }
                send(&runtime, peer, generation, if ready { Frame::Pull { after: cursor } } else { Frame::Hello { version: 1 } }).await?;
                if receive_updated.elapsed() > Duration::from_secs(30) { receiving = None; }
                if ready && receiving.is_none() {
                    // Payloads requested on demand from the UI are queued in the
                    // store and picked up here for the next peer request.
                    if let Ok(wanted) = library.take_wanted_blobs() {
                        for hash in wanted {
                            if unidesk_core::debug_logging::enabled() {
                                eprintln!("[library_sync] peer={:?} requesting blob hash={}", peer, hash);
                            }
                            pending.insert(hash);
                        }
                    }
                    if let Some(hash) = pending.first() {
                        if unidesk_core::debug_logging::enabled() {
                            eprintln!("[library_sync] peer={:?} sending BlobRequest hash={}", peer, hash);
                        }
                        send(&runtime, peer, generation, Frame::BlobRequest { hash: hash.clone() }).await?;
                    }
                }
            }
            bytes = rx.recv() => {
                let Some(bytes) = bytes else { return Ok(()); };
                let frame: Frame = serde_json::from_slice(&bytes).map_err(|_| "无效列表协议帧")?;
                match frame {
                    Frame::Hello { version } => {
                        if version != 1 { return Err("对端需升级列表同步协议".into()); }
                        if !ready { send(&runtime, peer, generation, Frame::Hello { version: 1 }).await?; }
                        ready = true;
                        send(&runtime, peer, generation, Frame::Pull { after: cursor }).await?;
                    }
                    _ if !ready => continue,
                    Frame::Pull { after } => {
                        // One operation per frame bounds the wire. seq is the sender's
                        // local append-log cursor, not a Lamport version or remote cursor.
                        let operations = library.export_operations(after, 1)?;
                        let next = operations.last().and_then(|o| o.get("seq")).and_then(Value::as_u64).unwrap_or(after);
                        if !operations.is_empty() && next <= after { return Err("列表操作缺少递增 seq".into()); }
                        send(&runtime, peer, generation, Frame::Operations { after, next, operations }).await?;
                    }
                    Frame::Operations { after, next, operations } => {
                        if after != cursor || next < after || operations.len() > 1 { continue; }
                        if !operations.is_empty() && next == after { return Err("无效列表游标".into()); }
                        for operation in &operations {
                            // Index records carry a blob_hash too, but ordinary
                            // history payloads are fetched on demand, not eagerly.
                            if operation["key"].as_str().is_some_and(|key| key.starts_with("index:")) { continue; }
                            blob_hashes(operation, &mut pending);
                        }
                        // Storage validates, deduplicates by persistent operation UUID,
                        // resolves versions/tombstones, and commits atomically before ACK.
                        library.merge_operations(operations)?;
                        let mut completed = Vec::new();
                        for hash in &pending { if library.read_blob(hash)?.is_some() { completed.push(hash.clone()); } }
                        for hash in completed { pending.remove(&hash); }
                        let advanced = next > cursor;
                        cursor = next;
                        if advanced { send(&runtime, peer, generation, Frame::Pull { after: cursor }).await?; }
                    }
                    Frame::BlobRequest { hash } => {
                        if !valid_hash(&hash) { return Err("无效归档校验码".into()); }
                        if blob_task.0.as_ref().is_some_and(|task| !task.is_finished()) { continue; }
                        let library = library.clone();
                        let runtime = runtime.clone();
                        blob_task.0 = Some(tokio::spawn(async move {
                            if let Ok(Some(bytes)) = library.read_blob(&hash) {
                                if bytes.len() > MAX_BLOB || hex::encode(Sha256::digest(&bytes)) != hash { return; }
                                for offset in (0..bytes.len().max(1)).step_by(CHUNK) {
                                    let end = (offset + CHUNK).min(bytes.len());
                                    if send(&runtime, peer, generation, Frame::Blob { hash: hash.clone(), total: bytes.len(), offset, data: hex::encode(&bytes[offset..end]) }).await.is_err() { return; }
                                }
                            } else { let _ = send(&runtime, peer, generation, Frame::Missing { hash }).await; }
                        }));
                    }
                    Frame::Blob { hash, total, offset, data } => {
                        if !valid_hash(&hash) || !pending.contains(&hash) || total > MAX_BLOB || data.len() > CHUNK * 2 { continue; }
                        let data = hex::decode(data).map_err(|_| "无效归档分块")?;
                        if offset == 0 {
                            if unidesk_core::debug_logging::enabled() {
                                eprintln!("[library_sync] peer={:?} receiving blob hash={} total={} bytes", peer, hash, total);
                            }
                            receiving = Some((hash.clone(), total, Vec::new()));
                        }
                        let Some((id, expected, bytes)) = receiving.as_mut() else { continue; };
                        if *id != hash || *expected != total || bytes.len() != offset || offset.checked_add(data.len()).is_none_or(|end| end > total) { receiving = None; continue; }
                        bytes.extend(data);
                        receive_updated = std::time::Instant::now();
                        if bytes.len() == total {
                            if hex::encode(Sha256::digest(&*bytes)) != hash { return Err("归档 SHA-256 校验失败".into()); }
                            if unidesk_core::debug_logging::enabled() {
                                eprintln!("[library_sync] peer={:?} blob complete hash={} size={}", peer, hash, bytes.len());
                            }
                            library.write_blob(&hash, bytes)?;
                            pending.remove(&hash);
                            receiving = None;
                        }
                    }
                    Frame::Missing { hash } => {
                        if unidesk_core::debug_logging::enabled() {
                            eprintln!("[library_sync] peer={:?} blob missing hash={}", peer, hash);
                        }
                        if receiving.as_ref().is_some_and(|(id, _, _)| *id == hash) { receiving = None; }
                    }
                }
            }
        }
    }
}
