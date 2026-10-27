//! Independent, resumable library replication. This transport never touches NSPasteboard.
//! Mount beside clipboard_sync in tokio_bridge; call refresh on its periodic tick and
//! receive before the ordinary clipboard decoder when MIME matches.
use crate::tokio_bridge::{UiEvent, trusted_connected_peer_ids};
use deskfuse_core::{ClipboardPayload, DeviceId};
use deskfuse_runtime::RuntimeHandle;
use futures::channel::mpsc::UnboundedSender;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::HashMap, sync::Arc, time::Duration};

use crate::clipboard_library::{Library, LogPosition, PullCursor};

pub const MIME: &str = "application/x-deskfuse-library-sync-v1";
const CHUNK: usize = 128 * 1024;
const MAX_BLOB: usize = 257 * 1024 * 1024;
// Maximum frame size for Operations messages. Must match receive limit.
const MAX_OPERATIONS_FRAME: usize = CHUNK * 2 + 64 * 1024; // 327,680 bytes
const MAX_OPERATIONS_BYTES: usize = 16 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Frame {
    Hello {
        version: u32,
        // Old v1 peers ignore this field and keep receiving ordinary frames.
        #[serde(default)]
        operation_chunks: bool,
        /// 发送端日志纪元与高水位；旧版本不发送，接收端退回每次从头拉取。
        #[serde(default)]
        log: Option<LogPosition>,
    },
    Pull {
        after: u64,
    },
    Operations {
        after: u64,
        next: u64,
        operations: Vec<Value>,
    },
    OperationsChunk {
        after: u64,
        next: u64,
        hash: String,
        total: usize,
        offset: usize,
        data: String,
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
pub struct Sync {
    peers: HashMap<DeviceId, PeerWorker>,
    library: Option<Arc<Library>>,
}
impl Sync {
    #[must_use]
    pub fn with_library(library: Arc<Library>) -> Self {
        Self {
            peers: HashMap::new(),
            library: Some(library),
        }
    }
    pub fn stop(&mut self) {
        self.peers.clear();
    }
    pub fn refresh(&mut self, runtime: &RuntimeHandle, ui: &UnboundedSender<UiEvent>) {
        self.refresh_inner(runtime, Some(ui));
    }
    pub fn refresh_silent(&mut self, runtime: &RuntimeHandle) {
        self.refresh_inner(runtime, None);
    }
    fn refresh_inner(&mut self, runtime: &RuntimeHandle, ui: Option<&UnboundedSender<UiEvent>>) {
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
            let ui = ui.cloned();
            let library = self.library.clone();
            let task = tokio::spawn(async move {
                if let Err(error) = worker(peer, generation, runtime, library, rx).await
                    && let Some(ui) = ui
                {
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
        if bytes.len() > MAX_OPERATIONS_FRAME {
            if deskfuse_core::debug_logging::enabled() {
                eprintln!(
                    "[library_sync] peer={:?} dropping oversized frame: {} bytes (limit: {})",
                    peer,
                    bytes.len(),
                    MAX_OPERATIONS_FRAME
                );
            }
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

    // Check frame size before sending to prevent infinite retransmission
    if bytes.len() > MAX_OPERATIONS_FRAME {
        if deskfuse_core::debug_logging::enabled() {
            eprintln!(
                "[library_sync] peer={:?} BLOCKED oversized Operations frame: {} bytes (limit: {})",
                peer,
                bytes.len(),
                MAX_OPERATIONS_FRAME
            );
        }
        return Err(format!(
            "Operations 帧大小 {} 字节超过限制 {} 字节，无法发送",
            bytes.len(),
            MAX_OPERATIONS_FRAME
        ));
    }

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
/// Keep large operations intact, including their UUID, clock and search text.
/// Sending one chunk per worker turn lets both peers drain their receive queues.
struct OperationsTransfer {
    after: u64,
    next: u64,
    hash: String,
    bytes: Vec<u8>,
    offset: usize,
}
impl OperationsTransfer {
    fn new(after: u64, next: u64, bytes: Vec<u8>) -> Result<Self, String> {
        if bytes.len() > MAX_OPERATIONS_BYTES {
            return Err("列表操作超过分块传输上限".into());
        }
        Ok(Self {
            after,
            next,
            hash: hex::encode(Sha256::digest(&bytes)),
            bytes,
            offset: 0,
        })
    }
    fn chunk(&self) -> Frame {
        let end = (self.offset + CHUNK).min(self.bytes.len());
        Frame::OperationsChunk {
            after: self.after,
            next: self.next,
            hash: self.hash.clone(),
            total: self.bytes.len(),
            offset: self.offset,
            data: hex::encode(&self.bytes[self.offset..end]),
        }
    }
    fn advance(&mut self) -> bool {
        self.offset = (self.offset + CHUNK).min(self.bytes.len());
        self.offset == self.bytes.len()
    }
}
fn prepare_operations(
    after: u64,
    next: u64,
    operations: Vec<Value>,
    supports_chunks: bool,
) -> Result<(Option<Frame>, Option<OperationsTransfer>), String> {
    let frame = Frame::Operations {
        after,
        next,
        operations,
    };
    let bytes = serde_json::to_vec(&frame).map_err(|e| e.to_string())?;
    if bytes.len() <= MAX_OPERATIONS_FRAME {
        return Ok((Some(frame), None));
    }
    if !supports_chunks {
        return Err("对端需升级以支持大列表操作分块同步".into());
    }
    Ok((None, Some(OperationsTransfer::new(after, next, bytes)?)))
}

#[derive(Default)]
struct OperationsReceiver(Option<OperationsAssembly>);
struct OperationsAssembly {
    after: u64,
    next: u64,
    hash: String,
    total: usize,
    bytes: Vec<u8>,
}
impl OperationsReceiver {
    fn receive(&mut self, cursor: u64, frame: Frame) -> Result<Option<Frame>, String> {
        let Frame::OperationsChunk {
            after,
            next,
            hash,
            total,
            offset,
            data,
        } = frame
        else {
            return Ok(Some(frame));
        };
        if after != cursor || next <= after {
            return Ok(None);
        }
        if !valid_hash(&hash)
            || total == 0
            || total > MAX_OPERATIONS_BYTES
            || data.len() > CHUNK * 2
        {
            return Err("列表操作分块无效".into());
        }
        let data = hex::decode(data).map_err(|_| "列表操作分块编码无效")?;
        if data.is_empty() || offset.checked_add(data.len()).is_none_or(|end| end > total) {
            return Err("列表操作分块范围无效".into());
        }
        if offset == 0 {
            self.0 = Some(OperationsAssembly {
                after,
                next,
                hash: hash.clone(),
                bytes: Vec::new(),
                total,
            });
        }
        let Some(current) = self.0.as_mut() else {
            return Ok(None);
        };
        if current.after != after
            || current.next != next
            || current.hash != hash
            || current.total != total
            || current.bytes.len() != offset
        {
            self.0 = None;
            return Ok(None);
        }
        current.bytes.extend(data);
        if current.bytes.len() != total {
            return Ok(None);
        }
        let current = self.0.take().unwrap();
        if hex::encode(Sha256::digest(&current.bytes)) != hash {
            return Err("列表操作 SHA-256 校验失败".into());
        }
        let frame: Frame =
            serde_json::from_slice(&current.bytes).map_err(|_| "列表操作编码无效")?;
        match frame {
            Frame::Operations {
                after: a,
                next: n,
                ref operations,
            } if a == after && n == next && operations.len() == 1 => Ok(Some(frame)),
            _ => Err("列表操作分块游标无效".into()),
        }
    }
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
fn hello(library: &crate::clipboard_library::Library) -> Result<Frame, String> {
    Ok(Frame::Hello {
        version: 1,
        operation_chunks: true,
        log: Some(library.operation_log_position()?),
    })
}
async fn worker(
    peer: DeviceId,
    generation: u64,
    runtime: RuntimeHandle,
    library: Option<Arc<Library>>,
    mut rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
) -> Result<(), String> {
    let library = match library {
        Some(library) => library,
        None => crate::clipboard_library::global()?,
    };
    let mut timer = tokio::time::interval(Duration::from_secs(3));
    let mut blob_task = BlobTask(None);
    let mut receive_updated = std::time::Instant::now();
    let mut ready = false;
    let mut supports_chunks = false;
    let mut outgoing_operations: Option<OperationsTransfer> = None;
    let mut incoming_operations = OperationsReceiver::default();
    let mut operations_updated = std::time::Instant::now();
    let mut hello_attempts = 0;
    let mut cursor = 0;
    // 对端日志纪元；存在时游标随合并事务持久化，重连后从断点继续。
    let mut remote_epoch: Option<String> = None;
    let peer_key = peer.to_string();
    let mut pending = std::collections::BTreeSet::<String>::new();
    let mut receiving: Option<(String, usize, Vec<u8>)> = None;

    loop {
        if runtime.control_generation(peer) != Some(generation) {
            return Ok(());
        }
        tokio::select! {
            biased; // Drain received chunks before sending more, keeping the bounded queue clear.
            _ = timer.tick() => {
                if !ready {
                    hello_attempts += 1;
                    if hello_attempts > 10 { return Err("对端尚未响应列表同步协议，请更新两端应用".into()); }
                }
                if operations_updated.elapsed() > Duration::from_secs(30) { incoming_operations.0 = None; }
                if !ready || incoming_operations.0.is_none() {
                    let frame = if ready { Frame::Pull { after: cursor } } else { hello(&library)? };
                    send(&runtime, peer, generation, frame).await?;
                }
                if receive_updated.elapsed() > Duration::from_secs(30) { receiving = None; }
                if ready && receiving.is_none() {
                    // Payloads requested on demand from the UI are queued in the
                    // store and picked up here for the next peer request.
                    if let Ok(wanted) = library.take_wanted_blobs() {
                        for hash in wanted {
                            if deskfuse_core::debug_logging::enabled() {
                                eprintln!("[library_sync] peer={:?} requesting blob hash={}", peer, hash);
                            }
                            pending.insert(hash);
                        }
                    }
                    if let Some(hash) = pending.first() {
                        if deskfuse_core::debug_logging::enabled() {
                            eprintln!("[library_sync] peer={:?} sending BlobRequest hash={}", peer, hash);
                        }
                        send(&runtime, peer, generation, Frame::BlobRequest { hash: hash.clone() }).await?;
                    }
                }
            }
            bytes = rx.recv() => {
                let Some(bytes) = bytes else { return Ok(()); };
                let frame: Frame = serde_json::from_slice(&bytes).map_err(|_| "无效列表协议帧")?;
                if matches!(frame, Frame::OperationsChunk { .. }) {
                    if !ready || !supports_chunks { return Err("未协商列表分块能力".into()); }
                    operations_updated = std::time::Instant::now();
                }
                let Some(frame) = incoming_operations.receive(cursor, frame)? else { continue; };
                match frame {
                    Frame::Hello { version, operation_chunks, log } => {
                        if version != 1 { return Err("对端需升级列表同步协议".into()); }
                        if !ready { send(&runtime, peer, generation, hello(&library)?).await?; }
                        supports_chunks = operation_chunks;
                        if let Some(log) = log {
                            cursor = library.pull_cursor(&peer_key, &log)?;
                            remote_epoch = Some(log.epoch);
                        }
                        if !ready {
                            // 游标不再回到 0，已同步归档缺失的正文要从库里重新排队。
                            pending.extend(library.missing_archive_blobs()?);
                        }
                        ready = true;
                        send(&runtime, peer, generation, Frame::Pull { after: cursor }).await?;
                    }
                    _ if !ready => continue,
                    Frame::Pull { after } => {
                        if outgoing_operations.is_some() { continue; }
                        // One operation per frame bounds the wire. seq is the sender's
                        // local append-log cursor, not a Lamport version or remote cursor.
                        let operations = library.export_operations(after, 1)?;
                        let next = operations.last().and_then(|o| o.get("seq")).and_then(Value::as_u64).unwrap_or(after);
                        if !operations.is_empty() && next <= after { return Err("列表操作缺少递增 seq".into()); }

                        let (frame, transfer) = prepare_operations(after, next, operations, supports_chunks)?;
                        outgoing_operations = transfer;
                        if let Some(frame) = frame { send(&runtime, peer, generation, frame).await?; }
                    }
                    Frame::Operations { after, next, operations } => {
                        if after != cursor || next < after || operations.len() > 1 { continue; }
                        if !operations.is_empty() && next == after { return Err("无效列表游标".into()); }

                        if deskfuse_core::debug_logging::enabled() && !operations.is_empty() {
                            eprintln!(
                                "[library_sync] peer={:?} received Operations: after={}, next={}, count={}",
                                peer, after, next, operations.len()
                            );
                        }

                        for operation in &operations {
                            // Index records carry a blob_hash too, but ordinary
                            // history payloads are fetched on demand, not eagerly.
                            if operation["key"].as_str().is_some_and(|key| key.starts_with("index:")) { continue; }
                            blob_hashes(operation, &mut pending);
                        }
                        // Storage validates, deduplicates by persistent operation UUID,
                        // resolves versions/tombstones, and commits atomically before ACK.
                        match &remote_epoch {
                            Some(epoch) if next > cursor => library.merge_pulled_operations(
                                operations,
                                &PullCursor { peer: peer_key.clone(), epoch: epoch.clone(), next },
                            )?,
                            _ => library.merge_operations(operations)?,
                        }
                        incoming_operations.0 = None;
                        let mut completed = Vec::new();
                        for hash in &pending { if library.read_blob(hash)?.is_some() { completed.push(hash.clone()); } }
                        for hash in completed { pending.remove(&hash); }
                        let advanced = next > cursor;
                        cursor = next;

                        if deskfuse_core::debug_logging::enabled() && advanced {
                            eprintln!(
                                "[library_sync] peer={:?} cursor advanced to {}",
                                peer, cursor
                            );
                        }

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
                            if deskfuse_core::debug_logging::enabled() {
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
                            if deskfuse_core::debug_logging::enabled() {
                                eprintln!("[library_sync] peer={:?} blob complete hash={} size={}", peer, hash, bytes.len());
                            }
                            library.write_blob(&hash, bytes)?;
                            pending.remove(&hash);
                            receiving = None;
                        }
                    }
                    Frame::OperationsChunk { .. } => unreachable!("chunks are assembled before dispatch"),
                    Frame::Missing { hash } => {
                        if deskfuse_core::debug_logging::enabled() {
                            eprintln!("[library_sync] peer={:?} blob missing hash={}", peer, hash);
                        }
                        if receiving.as_ref().is_some_and(|(id, _, _)| *id == hash) { receiving = None; }
                        // 对端没有这份正文；留在队首会每个周期重复请求并挡住后续正文。
                        pending.remove(&hash);
                    }
                }
            }
            _ = async {}, if outgoing_operations.is_some() => {
                let transfer = outgoing_operations.as_mut().unwrap();
                send(&runtime, peer, generation, transfer.chunk()).await?;
                if transfer.advance() { outgoing_operations = None; }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard_library::Library;
    use deskfuse_platform::PasteboardSnapshot;
    use serde_json::json;

    fn deliver(cursor: u64, mut transfer: OperationsTransfer) -> Frame {
        let mut receiver = OperationsReceiver::default();
        loop {
            let bytes = serde_json::to_vec(&transfer.chunk()).unwrap();
            assert!(bytes.len() <= MAX_OPERATIONS_FRAME);
            let result = receiver
                .receive(cursor, serde_json::from_slice(&bytes).unwrap())
                .unwrap();
            if transfer.advance() {
                return result.unwrap();
            }
            assert!(result.is_none()); // No operation/cursor ACK before the last chunk.
        }
    }

    #[tokio::test]
    async fn stop_aborts_replication_workers_and_closes_inbox() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let task = tokio::spawn(async move { while rx.recv().await.is_some() {} });
        let abort = task.abort_handle();
        let sender = tx.clone();
        let mut sync = Sync::default();
        sync.peers.insert(
            DeviceId::from_bytes([7; 32]),
            PeerWorker {
                generation: 1,
                tx,
                task,
            },
        );
        sync.stop();
        tokio::task::yield_now().await;
        assert!(abort.is_finished());
        assert!(sender.send(vec![1]).await.is_err());
        assert!(sync.peers.is_empty());
    }

    #[test]
    fn large_index_does_not_block_later_manual_membership() {
        let a = Library::open().unwrap();
        let b = Library::open().unwrap();
        a.set_local_device("device-a".into()).unwrap();
        b.set_local_device("device-b".into()).unwrap();
        a.mutate(r#"{"op":"save_list","list":{"id":"manual","name":"manual","kind":"manual"},"compact":true}"#).unwrap();
        let text = "中文\"\\\n".repeat(60_000) + "search-tail-marker";
        let id = a
            .ingest(
                &PasteboardSnapshot {
                    text: Some(text.clone()),
                    ..Default::default()
                },
                "local",
            )
            .unwrap()
            .unwrap();
        a.mutate(&json!({"op":"membership","list_id":"manual","history_id":id,"present":true,"compact":true}).to_string()).unwrap();

        let mut cursor = 0;
        let mut chunked = 0;
        let mut pending = std::collections::BTreeSet::new();
        // Repeat from cursor zero to cover reconnect/replay deduplication.
        for _ in 0..2 {
            loop {
                let ops = a.export_operations(cursor, 1).unwrap();
                if ops.is_empty() {
                    break;
                }
                let next = ops[0]["seq"].as_u64().unwrap();
                let expected = ops.clone();
                let (frame, transfer) = prepare_operations(cursor, next, ops, true).unwrap();
                let frame = if let Some(transfer) = transfer {
                    chunked += 1;
                    deliver(cursor, transfer)
                } else {
                    frame.unwrap()
                };
                let Frame::Operations {
                    after,
                    next,
                    operations,
                } = frame
                else {
                    panic!("expected operations");
                };
                assert_eq!(after, cursor);
                assert_eq!(operations, expected);
                for op in &operations {
                    if !op["key"].as_str().unwrap().starts_with("index:") {
                        blob_hashes(op, &mut pending);
                    }
                }
                b.merge_operations(operations).unwrap();
                cursor = next;
            }
            assert!(chunked > 0);
            let page: Value = serde_json::from_str(
                &b.query(r#"{"op":"items","list_id":"manual","include_total":true}"#)
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(page["total"], 1);
            let archived = page["items"][0]["id"].as_u64().unwrap();
            for hash in &pending {
                b.write_blob(hash, &a.read_blob(hash).unwrap().unwrap())
                    .unwrap();
            }
            assert_eq!(
                b.snapshot(archived).unwrap().text.as_deref(),
                Some(text.as_str())
            );
            let search: Value = serde_json::from_str(
                &b.query(r#"{"op":"items","query":"search-tail-marker","include_total":true}"#)
                    .unwrap(),
            )
            .unwrap();
            assert!(!search["items"].as_array().unwrap().is_empty());
            cursor = 0;
        }
    }

    #[test]
    fn old_peers_keep_small_frames_and_require_upgrade_for_large_operations() {
        let frame: Frame = serde_json::from_str(r#"{"kind":"hello","version":1}"#).unwrap();
        assert!(matches!(
            frame,
            Frame::Hello {
                version: 1,
                operation_chunks: false,
                log: None
            }
        ));
        let (small, transfer) =
            prepare_operations(0, 1, vec![json!({"key":"list:test"})], false).unwrap();
        assert!(matches!(small, Some(Frame::Operations { .. })));
        assert!(transfer.is_none());
        assert!(
            prepare_operations(
                0,
                1,
                vec![json!({"text":"x".repeat(MAX_OPERATIONS_FRAME)})],
                false
            )
            .err()
            .unwrap()
            .contains("升级")
        );
    }

    #[test]
    fn chunks_can_restart_but_cannot_commit_gaps_corruption_or_unbounded_data() {
        let (_, transfer) = prepare_operations(
            4,
            9,
            vec![json!({"text":"x".repeat(MAX_OPERATIONS_FRAME)})],
            true,
        )
        .unwrap();
        let mut transfer = transfer.unwrap();
        let mut receiver = OperationsReceiver::default();
        assert!(receiver.receive(4, transfer.chunk()).unwrap().is_none());
        transfer.advance();
        let second = transfer.chunk();
        transfer.advance();
        assert!(receiver.receive(4, transfer.chunk()).unwrap().is_none()); // Gap clears partial state.
        assert!(receiver.receive(4, second).unwrap().is_none());
        transfer.offset = 0;
        assert!(receiver.receive(5, transfer.chunk()).unwrap().is_none()); // Stale cursor.
        assert!(receiver.receive(4, transfer.chunk()).unwrap().is_none());
        assert!(receiver.receive(4, transfer.chunk()).unwrap().is_none()); // Restart first chunk.
        loop {
            transfer.advance();
            if let Some(Frame::Operations { next: 9, .. }) =
                receiver.receive(4, transfer.chunk()).unwrap()
            {
                break;
            }
        }
        let bytes = serde_json::to_vec(&Frame::Operations {
            after: 4,
            next: 9,
            operations: vec![json!({})],
        })
        .unwrap();
        let mut bad = OperationsTransfer::new(4, 9, bytes).unwrap().chunk();
        if let Frame::OperationsChunk { hash, .. } = &mut bad {
            *hash = "0".repeat(64);
        }
        assert!(receiver.receive(4, bad).err().unwrap().contains("SHA-256"));
        let mut bad = transfer.chunk();
        if let Frame::OperationsChunk { total, offset, .. } = &mut bad {
            *total = MAX_OPERATIONS_BYTES + 1;
            *offset = 0;
        }
        assert!(receiver.receive(4, bad).is_err());
    }

    fn archive_one(library: &Library) -> String {
        library.mutate(r#"{"op":"save_list","list":{"id":"manual","name":"manual","kind":"manual"},"compact":true}"#).unwrap();
        let id = library
            .ingest(
                &PasteboardSnapshot {
                    text: Some("断点续传".into()),
                    ..Default::default()
                },
                "local",
            )
            .unwrap()
            .unwrap();
        library
            .mutate(&json!({"op":"membership","list_id":"manual","history_id":id,"present":true,"compact":true}).to_string())
            .unwrap();
        let mut hashes = std::collections::BTreeSet::new();
        for op in library.export_operations(0, 100).unwrap() {
            if op["key"].as_str().unwrap().starts_with("item:") {
                blob_hashes(&op, &mut hashes);
            }
        }
        hashes.into_iter().next().unwrap()
    }

    #[test]
    fn pull_cursor_resumes_per_peer_epoch_and_requeues_missing_archives() {
        let a = Library::open().unwrap();
        let b = Library::open().unwrap();
        a.set_local_device("device-a".into()).unwrap();
        b.set_local_device("device-b".into()).unwrap();
        let hash = archive_one(&a);
        let position = a.operation_log_position().unwrap();
        assert_eq!(a.operation_log_position().unwrap().epoch, position.epoch);
        assert!(position.last_seq > 0);

        // 第一次连接只拉一条就断开：游标停在已提交的位置。
        assert_eq!(b.pull_cursor("peer-a", &position).unwrap(), 0);
        let first = a.export_operations(0, 1).unwrap();
        let next = first[0]["seq"].as_u64().unwrap();
        let cursor = PullCursor {
            peer: "peer-a".into(),
            epoch: position.epoch.clone(),
            next,
        };
        b.merge_pulled_operations(first, &cursor).unwrap();
        assert_eq!(b.pull_cursor("peer-a", &position).unwrap(), next);

        // 合并失败不推进游标。
        let broken = PullCursor {
            next: next + 100,
            ..cursor
        };
        assert!(
            b.merge_pulled_operations(vec![json!({"op_id":"bad"})], &broken)
                .is_err()
        );
        assert_eq!(b.pull_cursor("peer-a", &position).unwrap(), next);

        // 重连后从断点继续，拉完全部剩余操作。
        let mut after = b.pull_cursor("peer-a", &position).unwrap();
        loop {
            let ops = a.export_operations(after, 1).unwrap();
            let Some(next) = ops.last().and_then(|op| op["seq"].as_u64()) else {
                break;
            };
            b.merge_pulled_operations(
                ops,
                &PullCursor {
                    peer: "peer-a".into(),
                    epoch: position.epoch.clone(),
                    next,
                },
            )
            .unwrap();
            after = next;
        }
        let page: Value = serde_json::from_str(
            &b.query(r#"{"op":"items","list_id":"manual","include_total":true}"#)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(page["total"], 1);

        // 不再重放日志后，缺失的归档正文由库重新排队；写入后出队。
        assert_eq!(b.missing_archive_blobs().unwrap(), vec![hash.clone()]);
        b.write_blob(&hash, &a.read_blob(&hash).unwrap().unwrap())
            .unwrap();
        assert!(b.missing_archive_blobs().unwrap().is_empty());

        // 其他对端、新纪元或对端从备份回退高水位，都从头拉取。
        assert_eq!(b.pull_cursor("peer-c", &position).unwrap(), 0);
        let rebuilt = LogPosition {
            epoch: "rebuilt".into(),
            ..position.clone()
        };
        assert_eq!(b.pull_cursor("peer-a", &rebuilt).unwrap(), 0);
        let restored = LogPosition {
            last_seq: after - 1,
            ..position
        };
        assert_eq!(b.pull_cursor("peer-a", &restored).unwrap(), 0);
    }

    #[test]
    fn hello_carries_log_position_for_new_peers() {
        let library = Library::open().unwrap();
        let bytes = serde_json::to_vec(&hello(&library).unwrap()).unwrap();
        let Frame::Hello {
            log: Some(log),
            operation_chunks: true,
            ..
        } = serde_json::from_slice(&bytes).unwrap()
        else {
            panic!("expected hello with log position");
        };
        assert_eq!(log, library.operation_log_position().unwrap());
    }
}
