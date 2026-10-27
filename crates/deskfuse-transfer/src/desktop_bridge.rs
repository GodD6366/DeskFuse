//! Desktop-service file clipboard bridge.
//!
//! The state machine behind a desktop service (the Windows SCM service today)
//! that speaks the same [`ClipboardFileFrame`] protocol as the macOS engine
//! over `CLIPBOARD_FILE_MIME` payloads:
//!
//! - Sender side: one local copy registers a metadata-only
//!   [`DeferredClipboardSource`] offer per peer; the caller announces the
//!   returned [`ClipboardFileFrame::Deferred`] frame per peer over the
//!   acknowledged clipboard-sync channel. A [`ClipboardFileFrame::Request`]
//!   starts a bounded background job that revalidates the source, materialises
//!   the manifest, streams [`ClipboardFileFrame::RequestedOffer`] + chunks +
//!   [`ClipboardFileFrame::Complete`], honours receiver
//!   [`ClipboardFileFrame::Receipt`] flow-control credit, and waits for the
//!   receiver's [`ClipboardFileFrame::Ack`].
//! - Receiver side: a [`ClipboardFileFrame::Deferred`] records a pending
//!   offer. The desktop user explicitly requests the content; chunks are
//!   written by a bounded disk worker behind [`IncomingStore`]; after hash
//!   verification and commit the desktop copies the staged files out and
//!   confirms with [`FileBridge::commit_incoming`], which sends the protocol
//!   acknowledgement.
//!
//! The bridge is transport-independent: every frame leaves through
//! [`FileFrameSink`], blocking filesystem work happens in the caller (metadata
//! scans) or on dedicated worker threads it spawns, and no thread blocks while
//! holding the state lock for an unbounded time.

use crate::{
    ChunkDescriptor, ClipboardFileFrame, ClipboardFilePlan, DeferredClipboardSource,
    DeferredEntryKind, DeferredFileEntry, RelativePath, TransferId, TransferManifest,
};
use deskfuse_core::DeviceId;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[cfg(windows)]
use crate::Sha256Digest;
#[cfg(unix)]
use crate::{ConflictStrategy, FileReceiver, FileStaging};
#[cfg(windows)]
use sha2::Digest as _;
#[cfg(windows)]
use std::io::{Read, Seek, SeekFrom, Write};

/// Ten minutes, mirroring the macOS engine's deferred-offer lifetime.
pub const OFFER_LIFETIME: Duration = Duration::from_secs(10 * 60);
/// The desktop must copy staged files out and commit within this window.
pub const APPLY_DEADLINE: Duration = Duration::from_secs(60);
/// Bounded queue of chunks handed to one receiving disk worker.
pub const RECEIVE_QUEUE_CHUNKS: usize = 32;
/// A sending job aborts when the receiver stops crediting receipts.
pub const SEND_STALL_DEADLINE: Duration = Duration::from_secs(30);
/// Waiting for the receiver's final acknowledgement (large trees hash slowly).
pub const ACK_DEADLINE: Duration = Duration::from_secs(600);
/// Terminal rows stay visible to the desktop UI for this long.
pub const FINISHED_RETENTION: Duration = Duration::from_secs(10 * 60);
/// Concurrent receiving disk workers.
pub const MAX_RECEIVING: usize = 3;
/// Concurrent sending jobs.
pub const MAX_SENDING: usize = 4;
/// Pending incoming offers retained before the oldest is evicted.
pub const MAX_INCOMING_OFFERS: usize = 8;
/// Maximum outgoing offers retained (bounded metadata snapshots).
pub const MAX_OUTGOING_OFFERS: usize = 16;
/// Default per-transfer cache budget, matching the macOS default settings.
pub const DEFAULT_CACHE_MAX_BYTES: u64 = 1024 * 1024 * 1024;

/// Delivery boundary for every frame the bridge emits.
pub trait FileFrameSink: Send + Sync {
    /// Queue one frame. `Err` means the sink is unavailable (the owning
    /// service is stopping); callers treat that as a transfer failure.
    fn send(&self, peer: DeviceId, frame: ClipboardFileFrame) -> Result<(), String>;
}

/// Blocking staging store for one incoming transfer.
///
/// Implementations write chunks into a private staging area, verify every
/// complete-file hash on `complete`, and expose final absolute paths only
/// after the commit succeeded.
pub trait IncomingStore: Send {
    fn write_chunk(
        &mut self,
        path: &RelativePath,
        descriptor: ChunkDescriptor,
        bytes: &[u8],
    ) -> Result<(), String>;
    /// Verify every manifest file hash, commit the staging area into the
    /// transfer destination, and return the final absolute file paths.
    fn complete(&mut self) -> Result<Vec<String>, String>;
    /// Remove all staged bytes. Called on cancellation or failure.
    fn abort(&mut self);
}

/// Creates the platform receiving store for one transfer under `root`.
pub type StoreFactory = Arc<
    dyn Fn(&TransferManifest, TransferId) -> Result<Box<dyn IncomingStore>, String> + Send + Sync,
>;

/// Builds the default per-transfer store for this platform under `root`.
pub fn default_store(
    manifest: &TransferManifest,
    id: TransferId,
    root: &Path,
) -> Result<Box<dyn IncomingStore>, String> {
    #[cfg(unix)]
    {
        UnixStore::new(manifest.clone(), id, root)
            .map(|store| Box::new(store) as Box<dyn IncomingStore>)
    }
    #[cfg(windows)]
    {
        WindowsStore::new(manifest.clone(), id, root)
            .map(|store| Box::new(store) as Box<dyn IncomingStore>)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (manifest, id, root);
        Err("此平台没有文件接收实现".into())
    }
}

/// Factory bound to one cache root; per-transfer destinations live beneath it.
#[must_use]
pub fn default_factory(root: PathBuf) -> StoreFactory {
    Arc::new(move |manifest, id| {
        let destination = root.join(id.to_hex());
        fs::create_dir_all(&destination)
            .map_err(|error| format!("无法创建接收缓存目录：{error}"))?;
        default_store(manifest, id, &destination)
    })
}

// ---------------------------------------------------------------------------
// Views
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FileTransferView {
    pub transfer_id: String,
    pub peer_id: String,
    /// `outgoing` or `incoming`.
    pub direction: String,
    pub names: Vec<String>,
    /// Complete relative tree metadata. Stable `lindex` values are used by OLE.
    #[serde(default)]
    pub entries: Vec<DeferredFileEntry>,
    pub total_bytes: u64,
    pub transferred_bytes: u64,
    /// `offered` | `requested` | `preparing` | `receiving` | `verifying` |
    /// `staged` | `sending` | `completed` | `failed` | `cancelled`.
    pub state: String,
    pub detail: Option<String>,
    /// Absolute staged file paths; only present once the receiver committed.
    pub staged_paths: Vec<String>,
    pub created_unix_ms: u64,
    pub updated_unix_ms: u64,
}

/// Cache accounting surfaced beside the transfer queue.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq, Default)]
pub struct FileCacheUsage {
    pub max_bytes: u64,
    pub used_bytes: u64,
}

// ---------------------------------------------------------------------------
// Internal state
// ---------------------------------------------------------------------------

struct OutgoingOffer {
    peer: DeviceId,
    roots: Vec<PathBuf>,
    source: DeferredClipboardSource,
    cancel: Arc<AtomicBool>,
    created: Instant,
}

/// Receiver flow-control credit shared between the sending job and the bridge.
struct ReceiptWatch {
    accepted: Mutex<u64>,
    changed: Mutex<Instant>,
}

impl ReceiptWatch {
    fn new() -> Self {
        Self {
            accepted: Mutex::new(0),
            changed: Mutex::new(Instant::now()),
        }
    }
    fn credit(&self, accepted: u64) {
        let mut accepted_guard = self.accepted.lock().expect("receipt lock");
        if accepted > *accepted_guard {
            *accepted_guard = accepted;
            *self.changed.lock().expect("receipt time lock") = Instant::now();
        }
    }
    fn accepted(&self) -> u64 {
        *self.accepted.lock().expect("receipt lock")
    }
    fn idle_for(&self) -> Duration {
        Instant::now().duration_since(*self.changed.lock().expect("receipt time lock"))
    }
}

/// 0 = pending, 1 = applied, 2 = refused.
type AckFlag = AtomicU8;

/// Sending-phase labels shared between the job thread and the state lock.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SendPhase {
    Preparing,
    Sending,
    Verifying,
}

impl SendPhase {
    const fn label(self) -> &'static str {
        match self {
            Self::Preparing => "preparing",
            Self::Sending => "sending",
            Self::Verifying => "verifying",
        }
    }
}

struct SendingJob {
    peer: DeviceId,
    names: Vec<String>,
    entries: Vec<DeferredFileEntry>,
    total: u64,
    sent_bytes: u64,
    phase: SendPhase,
    cancel: Arc<AtomicBool>,
    ack: Arc<AckFlag>,
    receipts: Arc<ReceiptWatch>,
    created: Instant,
}

struct IncomingOffer {
    peer: DeviceId,
    names: Vec<String>,
    entries: Vec<DeferredFileEntry>,
    total_bytes: u64,
    created: Instant,
    requested: bool,
}

struct ReceivingTransfer {
    peer: DeviceId,
    names: Vec<String>,
    entries: Vec<DeferredFileEntry>,
    total_bytes: u64,
    accepted_frames: u64,
    received_bytes: u64,
    commands: Option<std::sync::mpsc::SyncSender<WorkerCommand>>,
    events: std::sync::mpsc::Receiver<WorkerEvent>,
    destination: PathBuf,
    finishing: bool,
    created: Instant,
}
struct PendingApply {
    transfer_id: TransferId,
    peer: DeviceId,
    names: Vec<String>,
    entries: Vec<DeferredFileEntry>,
    total_bytes: u64,
    staged_paths: Vec<String>,
    destination: PathBuf,
    created: Instant,
}

struct FinishedRow {
    view: FileTransferView,
    finished_at: Instant,
}

enum WorkerCommand {
    Chunk {
        path: RelativePath,
        descriptor: ChunkDescriptor,
        bytes: Vec<u8>,
    },
    Finish,
}

enum WorkerEvent {
    Progress(u64),
    Finished(Result<Vec<String>, String>),
}

#[derive(Default)]
struct State {
    outgoing: HashMap<TransferId, OutgoingOffer>,
    sending: HashMap<TransferId, SendingJob>,
    incoming: HashMap<TransferId, IncomingOffer>,
    receiving: HashMap<TransferId, ReceivingTransfer>,
    pending_apply: Option<PendingApply>,
    finished: Vec<FinishedRow>,
    used_bytes: u64,
    last_cache_scan: Option<Instant>,
}

struct Inner {
    cache_root: PathBuf,
    factory: StoreFactory,
    sink: Arc<dyn FileFrameSink>,
    limits: Limits,
    state: Mutex<State>,
}

/// Tunable bounds; `Default` mirrors the macOS engine's constants.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub offer_lifetime: Duration,
    pub apply_deadline: Duration,
    pub send_stall_deadline: Duration,
    pub ack_deadline: Duration,
    pub finished_retention: Duration,
    pub receive_queue_chunks: usize,
    pub max_receiving: usize,
    pub max_sending: usize,
    pub max_incoming_offers: usize,
    pub max_outgoing_offers: usize,
    pub cache_max_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            offer_lifetime: OFFER_LIFETIME,
            apply_deadline: APPLY_DEADLINE,
            send_stall_deadline: SEND_STALL_DEADLINE,
            ack_deadline: ACK_DEADLINE,
            finished_retention: FINISHED_RETENTION,
            receive_queue_chunks: RECEIVE_QUEUE_CHUNKS,
            max_receiving: MAX_RECEIVING,
            max_sending: MAX_SENDING,
            max_incoming_offers: MAX_INCOMING_OFFERS,
            max_outgoing_offers: MAX_OUTGOING_OFFERS,
            cache_max_bytes: DEFAULT_CACHE_MAX_BYTES,
        }
    }
}

/// Session-bound file clipboard state machine. Cheap to clone; every method is
/// bounded for the caller (blocking scans are explicit, streaming runs on the
/// bridge's own worker threads).
#[derive(Clone)]
pub struct FileBridge {
    inner: Arc<Inner>,
}

impl FileBridge {
    #[must_use]
    pub fn new(
        cache_root: PathBuf,
        factory: StoreFactory,
        sink: Arc<dyn FileFrameSink>,
        limits: Limits,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                cache_root,
                factory,
                sink,
                limits,
                state: Mutex::new(State::default()),
            }),
        }
    }

    /// Bridge with the default per-transfer store implementation under `root`.
    #[must_use]
    pub fn with_default_store(cache_root: PathBuf, sink: Arc<dyn FileFrameSink>) -> Self {
        Self::new(
            cache_root.clone(),
            default_factory(cache_root),
            sink,
            Limits::default(),
        )
    }

    #[must_use]
    pub fn cache_root(&self) -> &Path {
        &self.inner.cache_root
    }

    #[must_use]
    pub fn limits(&self) -> Limits {
        self.inner.limits
    }

    /// Registers metadata-only outgoing offers for one local copy and returns
    /// one [`ClipboardFileFrame::Deferred`] frame per peer. Callers send these
    /// over the acknowledged clipboard-sync channel, which acknowledges each
    /// announcement like any other clipboard payload.
    ///
    /// Inspecting the source tree blocks; callers offload this call. Source
    /// identity is revalidated before any content byte is read.
    pub fn publish_local(
        &self,
        peers: &[DeviceId],
        roots: Vec<PathBuf>,
    ) -> Result<Vec<(DeviceId, TransferId, ClipboardFileFrame)>, String> {
        if peers.is_empty() {
            return Err("没有已连接的受信任文件设备".into());
        }
        if roots.is_empty() {
            return Err("没有可发送的文件或文件夹".into());
        }
        let source =
            DeferredClipboardSource::inspect(roots.clone()).map_err(|error| error.to_string())?;
        let mut state = self.inner.state.lock().expect("file bridge state");
        let mut frames = Vec::new();
        for peer in peers {
            let id = TransferId::new();
            state.outgoing.insert(
                id,
                OutgoingOffer {
                    peer: *peer,
                    roots: roots.clone(),
                    source: source.clone(),
                    cancel: Arc::new(AtomicBool::new(false)),
                    created: Instant::now(),
                },
            );
            frames.push((
                *peer,
                id,
                ClipboardFileFrame::Deferred {
                    transfer_id: id,
                    names: source.names.clone(),
                    total_bytes: source.total_bytes,
                    entries: source.deferred_entries().to_vec(),
                },
            ));
        }
        while state.outgoing.len() > self.inner.limits.max_outgoing_offers {
            let Some(oldest) = state
                .outgoing
                .iter()
                .min_by_key(|(_, offer)| offer.created)
                .map(|(id, _)| *id)
            else {
                break;
            };
            if let Some(offer) = state.outgoing.remove(&oldest) {
                offer.cancel.store(true, Ordering::Release);
            }
        }
        Ok(frames)
    }

    /// Handles one authenticated file clipboard frame from a peer.
    pub fn on_frame(&self, peer: DeviceId, frame: ClipboardFileFrame) {
        match frame {
            ClipboardFileFrame::Deferred {
                transfer_id,
                names,
                total_bytes,
                entries,
            } => {
                self.record_incoming_offer(peer, transfer_id, names, total_bytes, entries);
            }
            ClipboardFileFrame::Request { transfer_id } => self.start_sending(peer, transfer_id),
            ClipboardFileFrame::RequestedOffer {
                transfer_id,
                manifest,
            } => {
                self.start_receiving(peer, transfer_id, manifest);
            }
            ClipboardFileFrame::Chunk {
                transfer_id,
                path,
                descriptor,
                bytes,
            } => {
                self.accept_chunk(peer, transfer_id, path, descriptor, bytes);
            }
            ClipboardFileFrame::Complete { transfer_id } => {
                self.finish_receiving(peer, transfer_id)
            }
            ClipboardFileFrame::Receipt {
                transfer_id,
                accepted_frames,
            } => {
                let state = self.inner.state.lock().expect("file bridge state");
                if let Some(job) = state.sending.get(&transfer_id)
                    && job.peer == peer
                {
                    job.receipts.credit(accepted_frames);
                }
            }
            ClipboardFileFrame::Ack { transfer_id } => {
                let state = self.inner.state.lock().expect("file bridge state");
                if let Some(job) = state.sending.get(&transfer_id)
                    && job.peer == peer
                {
                    job.ack.store(1, Ordering::Release);
                }
            }
            ClipboardFileFrame::Cancel {
                transfer_id,
                reason,
            } => {
                self.cancel_from_peer(peer, transfer_id, &reason);
            }
            ClipboardFileFrame::Offer { transfer_id, .. } => {
                // Unsolicited full offers are not part of this direction's
                // contract; refuse so the sender stops instead of stalling.
                let _ = self.inner.sink.send(
                    peer,
                    ClipboardFileFrame::Cancel {
                        transfer_id,
                        reason: "接收端未请求此文件内容".into(),
                    },
                );
            }
        }
    }

    /// Wire-level acceptance decision for an announcement assembled by the
    /// acknowledged clipboard-sync channel. Well-formed deferred offers are
    /// recorded and acknowledged (APPLIED); malformed payloads are rejected by
    /// the caller before reaching this method.
    pub fn accept_wire_offer(&self, peer: DeviceId, frame: ClipboardFileFrame) -> bool {
        self.on_frame(peer, frame);
        true
    }

    /// Desktop accept: request the file content for one pending offer.
    pub fn request_incoming(&self, transfer_id: &str) -> Result<(), String> {
        let id = parse_transfer(transfer_id)?;
        let mut state = self.inner.state.lock().expect("file bridge state");
        let Some(offer) = state.incoming.get(&id) else {
            return Err("文件引用已失效，请重新复制".into());
        };
        if offer.requested {
            return Ok(());
        }
        if offer.total_bytes > self.inner.limits.cache_max_bytes {
            return Err("文件超过缓存上限，请调整设置后重试".into());
        }
        if state.receiving.len() >= self.inner.limits.max_receiving {
            return Err("已有文件正在接收，请等待完成后再试".into());
        }
        let peer = offer.peer;
        state
            .incoming
            .get_mut(&id)
            .expect("checked offer")
            .requested = true;
        self.inner
            .sink
            .send(peer, ClipboardFileFrame::Request { transfer_id: id })?;
        Ok(())
    }

    /// User cancellation applies to both shelf sends and incoming transfers.
    pub fn cancel_transfer(&self, transfer_id: &str, reason: &str) {
        let Ok(id) = parse_transfer(transfer_id) else {
            return;
        };
        let outgoing_peer = {
            let state = self.inner.state.lock().expect("file bridge state");
            state
                .sending
                .get(&id)
                .map(|job| job.peer)
                .or_else(|| state.outgoing.get(&id).map(|offer| offer.peer))
        };
        if let Some(peer) = outgoing_peer {
            self.cancel_from_peer(peer, id, reason);
            let _ = self.inner.sink.send(
                peer,
                ClipboardFileFrame::Cancel {
                    transfer_id: id,
                    reason: bounded_reason(reason),
                },
            );
        } else {
            self.cancel_incoming(transfer_id, reason);
        }
    }

    /// Desktop cancel of one pending or active incoming transfer.
    pub fn cancel_incoming(&self, transfer_id: &str, reason: &str) {
        let Ok(id) = parse_transfer(transfer_id) else {
            return;
        };
        let mut state = self.inner.state.lock().expect("file bridge state");
        if state.receiving.contains_key(&id) {
            let peer = state.receiving.get(&id).expect("checked").peer;
            self.cancel_receiving(&mut state, id, peer, reason.to_owned());
            return;
        }
        if state
            .pending_apply
            .as_ref()
            .is_some_and(|pending| pending.transfer_id == id)
        {
            let pending = state.pending_apply.take().expect("checked pending");
            let _ = self.inner.sink.send(
                pending.peer,
                ClipboardFileFrame::Cancel {
                    transfer_id: pending.transfer_id,
                    reason: bounded_reason(reason),
                },
            );
            self.push_finished(
                &mut state,
                FileTransferView {
                    transfer_id: pending.transfer_id.to_hex(),
                    peer_id: pending.peer.to_hex(),
                    direction: "incoming".into(),
                    names: pending.names,
                    entries: pending.entries,
                    total_bytes: pending.total_bytes,
                    transferred_bytes: pending.total_bytes,
                    state: "cancelled".into(),
                    detail: Some(reason.to_owned()),
                    staged_paths: Vec::new(),
                    created_unix_ms: unix_ms(pending.created),
                    updated_unix_ms: unix_ms_now(),
                },
            );
            remove_dir_best_effort(pending.destination);
            return;
        }
        if let Some(offer) = state.incoming.remove(&id) {
            let _ = self.inner.sink.send(
                offer.peer,
                ClipboardFileFrame::Cancel {
                    transfer_id: id,
                    reason: bounded_reason(reason),
                },
            );
            self.push_finished(
                &mut state,
                FileTransferView {
                    transfer_id: id.to_hex(),
                    peer_id: offer.peer.to_hex(),
                    direction: "incoming".into(),
                    names: offer.names,
                    entries: offer.entries,
                    total_bytes: offer.total_bytes,
                    transferred_bytes: 0,
                    state: "cancelled".into(),
                    detail: Some(reason.to_owned()),
                    staged_paths: Vec::new(),
                    created_unix_ms: unix_ms(offer.created),
                    updated_unix_ms: unix_ms_now(),
                },
            );
        }
    }

    /// Desktop commit after copying staged files out. `applied == true` sends
    /// the protocol acknowledgement; otherwise the sender is cancelled and
    /// staged bytes are removed. Returns whether the commit matched a staged
    /// transfer.
    pub fn commit_incoming(&self, transfer_id: &str, applied: bool) -> Result<bool, String> {
        let id = parse_transfer(transfer_id)?;
        let mut state = self.inner.state.lock().expect("file bridge state");
        let Some(pending) = state.pending_apply.take() else {
            return Ok(false);
        };
        if pending.transfer_id != id {
            state.pending_apply = Some(pending);
            return Ok(false);
        }
        if applied {
            self.inner.sink.send(
                pending.peer,
                ClipboardFileFrame::Ack {
                    transfer_id: pending.transfer_id,
                },
            )?;
            self.push_finished(
                &mut state,
                FileTransferView {
                    transfer_id: pending.transfer_id.to_hex(),
                    peer_id: pending.peer.to_hex(),
                    direction: "incoming".into(),
                    names: pending.names.clone(),
                    entries: pending.entries.clone(),
                    total_bytes: pending.total_bytes,
                    transferred_bytes: pending.total_bytes,
                    state: "completed".into(),
                    detail: None,
                    staged_paths: pending.staged_paths.clone(),
                    created_unix_ms: unix_ms(pending.created),
                    updated_unix_ms: unix_ms_now(),
                },
            );
        } else {
            self.inner.sink.send(
                pending.peer,
                ClipboardFileFrame::Cancel {
                    transfer_id: pending.transfer_id,
                    reason: "接收端未能完成文件落地".into(),
                },
            )?;
            self.push_finished(
                &mut state,
                FileTransferView {
                    transfer_id: pending.transfer_id.to_hex(),
                    peer_id: pending.peer.to_hex(),
                    direction: "incoming".into(),
                    names: pending.names,
                    entries: pending.entries,
                    total_bytes: pending.total_bytes,
                    transferred_bytes: pending.total_bytes,
                    state: "failed".into(),
                    detail: Some("文件落地被本机取消".into()),
                    staged_paths: Vec::new(),
                    created_unix_ms: unix_ms(pending.created),
                    updated_unix_ms: unix_ms_now(),
                },
            );
        }
        remove_dir_best_effort(pending.destination);
        Ok(true)
    }

    /// Applies timeouts, drains worker events and handles disconnects. Called
    /// on the service tick with the currently connected peer list.
    pub fn tick(&self, connected: &[DeviceId]) {
        let mut state = self.inner.state.lock().expect("file bridge state");
        let now = Instant::now();
        self.drain_worker_events(&mut state);
        // Expire outgoing offers; late requests are refused with a Cancel.
        let expired: Vec<TransferId> = state
            .outgoing
            .iter()
            .filter(|(_, offer)| {
                now.duration_since(offer.created) >= self.inner.limits.offer_lifetime
            })
            .map(|(id, _)| *id)
            .collect();
        for id in expired {
            if let Some(offer) = state.outgoing.remove(&id) {
                offer.cancel.store(true, Ordering::Release);
            }
        }
        // Unrequested pending offers expire by lifetime or peer disconnect.
        let expired: Vec<TransferId> = state
            .incoming
            .iter()
            .filter(|(_, offer)| {
                !offer.requested
                    && (now.duration_since(offer.created) >= self.inner.limits.offer_lifetime
                        || !connected.contains(&offer.peer))
            })
            .map(|(id, _)| *id)
            .collect();
        for id in expired {
            state.incoming.remove(&id);
        }
        // Receiving transfers die with their peer.
        let dead: Vec<TransferId> = state
            .receiving
            .iter()
            .filter(|(_, active)| !connected.contains(&active.peer))
            .map(|(id, _)| *id)
            .collect();
        for id in dead {
            let peer = state.receiving.get(&id).expect("checked").peer;
            self.cancel_receiving(&mut state, id, peer, "源设备已断开，请重新复制".into());
        }
        // Sending jobs abort on cancel flags, peer disconnects or receipt stalls.
        let stalled: Vec<TransferId> = state
            .sending
            .iter()
            .filter(|(_, job)| {
                job.cancel.load(Ordering::Acquire)
                    || !connected.contains(&job.peer)
                    || (job.phase == SendPhase::Sending
                        && job.receipts.idle_for() >= self.inner.limits.send_stall_deadline)
            })
            .map(|(id, _)| *id)
            .collect();
        for id in stalled {
            if let Some(job) = state.sending.get(&id) {
                // The job thread observes the flag, replies with a Cancel frame
                // (unless the receiver already cancelled) and settles its row.
                job.cancel.store(true, Ordering::Release);
            }
        }
        // Pending apply deadline: staged bytes are removed, sender cancelled.
        if let Some(pending) = state.pending_apply.as_ref()
            && now.duration_since(pending.created) >= self.inner.limits.apply_deadline
        {
            let pending = state.pending_apply.take().expect("checked pending");
            let _ = self.inner.sink.send(
                pending.peer,
                ClipboardFileFrame::Cancel {
                    transfer_id: pending.transfer_id,
                    reason: "接收端文件落地超时".into(),
                },
            );
            self.push_finished(
                &mut state,
                FileTransferView {
                    transfer_id: pending.transfer_id.to_hex(),
                    peer_id: pending.peer.to_hex(),
                    direction: "incoming".into(),
                    names: pending.names,
                    entries: pending.entries,
                    total_bytes: pending.total_bytes,
                    transferred_bytes: pending.total_bytes,
                    state: "failed".into(),
                    detail: Some("文件落地确认超时".into()),
                    staged_paths: Vec::new(),
                    created_unix_ms: unix_ms(pending.created),
                    updated_unix_ms: unix_ms_now(),
                },
            );
            remove_dir_best_effort(pending.destination);
        }
        state.finished.retain(|row| {
            now.duration_since(row.finished_at) < self.inner.limits.finished_retention
        });
        // Cache usage, refreshed at most every ten seconds.
        if state
            .last_cache_scan
            .is_none_or(|last| now.duration_since(last) >= Duration::from_secs(10))
        {
            state.used_bytes = directory_bytes(&self.inner.cache_root, 4096);
            state.last_cache_scan = Some(now);
        }
    }

    /// Cancels and clears everything. With `notify`, peers receive explicit
    /// Cancel frames (the owning desktop session is still able to send);
    /// otherwise state is dropped silently (service stopping).
    pub fn reset(&self, notify: bool) {
        let mut state = self.inner.state.lock().expect("file bridge state");
        let receiving: Vec<(TransferId, DeviceId)> = state
            .receiving
            .iter()
            .map(|(id, active)| (*id, active.peer))
            .collect();
        // Receiving workers abort their staging when the command channel
        // closes; the destination removal is theirs, not ours.
        for active in state.receiving.values_mut() {
            active.commands = None;
        }
        if let Some(pending) = state.pending_apply.take() {
            remove_dir_best_effort(pending.destination);
        }
        state.receiving.clear();
        state.sending.clear();
        state.incoming.clear();
        // Sending job threads share these cancel flags and reply with their
        // own Cancel frames when they observe them.
        let cancelled = std::mem::take(&mut state.outgoing);
        for offer in cancelled.into_values() {
            offer.cancel.store(true, Ordering::Release);
        }
        drop(state);
        if notify {
            for (id, peer) in receiving {
                let _ = self.inner.sink.send(
                    peer,
                    ClipboardFileFrame::Cancel {
                        transfer_id: id,
                        reason: "本机会话已结束".into(),
                    },
                );
            }
        }
    }

    #[must_use]
    pub fn views(&self) -> Vec<FileTransferView> {
        let state = self.inner.state.lock().expect("file bridge state");
        let mut views = Vec::new();
        for (id, offer) in &state.incoming {
            views.push(FileTransferView {
                transfer_id: id.to_hex(),
                peer_id: offer.peer.to_hex(),
                direction: "incoming".into(),
                names: offer.names.clone(),
                entries: offer.entries.clone(),
                total_bytes: offer.total_bytes,
                transferred_bytes: 0,
                state: if offer.requested {
                    "requested"
                } else {
                    "offered"
                }
                .into(),
                detail: None,
                staged_paths: Vec::new(),
                created_unix_ms: unix_ms(offer.created),
                updated_unix_ms: unix_ms(offer.created),
            });
        }
        for (id, active) in &state.receiving {
            views.push(FileTransferView {
                transfer_id: id.to_hex(),
                peer_id: active.peer.to_hex(),
                direction: "incoming".into(),
                names: active.names.clone(),
                entries: active.entries.clone(),
                total_bytes: active.total_bytes,
                transferred_bytes: active.received_bytes,
                state: if active.finishing {
                    "verifying"
                } else {
                    "receiving"
                }
                .into(),
                detail: None,
                staged_paths: Vec::new(),
                created_unix_ms: unix_ms(active.created),
                updated_unix_ms: unix_ms(active.created),
            });
        }
        if let Some(pending) = state.pending_apply.as_ref() {
            views.push(FileTransferView {
                transfer_id: pending.transfer_id.to_hex(),
                peer_id: pending.peer.to_hex(),
                direction: "incoming".into(),
                names: pending.names.clone(),
                entries: pending.entries.clone(),
                total_bytes: pending.total_bytes,
                transferred_bytes: pending.total_bytes,
                state: "staged".into(),
                detail: None,
                staged_paths: pending.staged_paths.clone(),
                created_unix_ms: unix_ms(pending.created),
                updated_unix_ms: unix_ms(pending.created),
            });
        }
        for (id, job) in &state.sending {
            views.push(FileTransferView {
                transfer_id: id.to_hex(),
                peer_id: job.peer.to_hex(),
                direction: "outgoing".into(),
                names: job.names.clone(),
                entries: job.entries.clone(),
                total_bytes: job.total,
                transferred_bytes: if job.phase == SendPhase::Preparing {
                    0
                } else {
                    job.sent_bytes
                },
                state: job.phase.label().into(),
                detail: None,
                staged_paths: Vec::new(),
                created_unix_ms: unix_ms(job.created),
                updated_unix_ms: unix_ms(job.created),
            });
        }
        for (id, offer) in &state.outgoing {
            if state.sending.contains_key(id) {
                continue;
            }
            views.push(FileTransferView {
                transfer_id: id.to_hex(),
                peer_id: offer.peer.to_hex(),
                direction: "outgoing".into(),
                names: offer.source.names.clone(),
                entries: offer.source.deferred_entries().to_vec(),
                total_bytes: offer.source.total_bytes,
                transferred_bytes: 0,
                state: "offered".into(),
                detail: None,
                staged_paths: Vec::new(),
                created_unix_ms: unix_ms(offer.created),
                updated_unix_ms: unix_ms(offer.created),
            });
        }
        for row in &state.finished {
            views.push(row.view.clone());
        }
        views
    }

    #[must_use]
    pub fn cache_usage(&self) -> FileCacheUsage {
        let state = self.inner.state.lock().expect("file bridge state");
        FileCacheUsage {
            max_bytes: self.inner.limits.cache_max_bytes,
            used_bytes: state.used_bytes,
        }
    }
}

impl FileBridge {
    fn record_incoming_offer(
        &self,
        peer: DeviceId,
        id: TransferId,
        names: Vec<String>,
        total_bytes: u64,
        entries: Vec<DeferredFileEntry>,
    ) {
        let entries = if entries.is_empty() {
            names
                .iter()
                .enumerate()
                .map(|(lindex, name)| DeferredFileEntry {
                    path: RelativePath::new(name).expect("validated deferred name"),
                    kind: DeferredEntryKind::File,
                    size: if names.len() == 1 { total_bytes } else { 0 },
                    lindex: lindex as u32,
                })
                .collect()
        } else {
            entries
        };
        let mut state = self.inner.state.lock().expect("file bridge state");
        // A retransmitted announcement for a known offer is idempotent.
        if state
            .incoming
            .get(&id)
            .is_some_and(|offer| offer.peer == peer)
        {
            return;
        }
        if state.receiving.contains_key(&id)
            || state
                .pending_apply
                .as_ref()
                .is_some_and(|pending| pending.transfer_id == id)
        {
            return;
        }
        while state.incoming.len() >= self.inner.limits.max_incoming_offers {
            let Some(oldest) = state
                .incoming
                .iter()
                .filter(|(_, offer)| !offer.requested)
                .min_by_key(|(_, offer)| offer.created)
                .map(|(id, _)| *id)
                .or_else(|| {
                    state
                        .incoming
                        .iter()
                        .min_by_key(|(_, offer)| offer.created)
                        .map(|(id, _)| *id)
                })
            else {
                break;
            };
            state.incoming.remove(&oldest);
        }
        state.incoming.insert(
            id,
            IncomingOffer {
                peer,
                names,
                entries,
                total_bytes,
                created: Instant::now(),
                requested: false,
            },
        );
    }

    fn start_sending(&self, peer: DeviceId, transfer_id: TransferId) {
        let mut state = self.inner.state.lock().expect("file bridge state");
        if state.sending.contains_key(&transfer_id) {
            return;
        }
        let Some((offer_peer, roots, source, cancel, created)) =
            state.outgoing.get(&transfer_id).map(|offer| {
                (
                    offer.peer,
                    offer.roots.clone(),
                    offer.source.clone(),
                    Arc::clone(&offer.cancel),
                    offer.created,
                )
            })
        else {
            let _ = self.inner.sink.send(
                peer,
                ClipboardFileFrame::Cancel {
                    transfer_id,
                    reason: "源剪贴板已失效，请重新复制文件".into(),
                },
            );
            return;
        };
        if offer_peer != peer
            || Instant::now().duration_since(created) >= self.inner.limits.offer_lifetime
            || cancel.load(Ordering::Acquire)
        {
            let _ = self.inner.sink.send(
                peer,
                ClipboardFileFrame::Cancel {
                    transfer_id,
                    reason: "源剪贴板已失效，请重新复制文件".into(),
                },
            );
            return;
        }
        if state.sending.len() >= self.inner.limits.max_sending {
            let _ = self.inner.sink.send(
                peer,
                ClipboardFileFrame::Cancel {
                    transfer_id,
                    reason: "发送任务已达上限，请稍后重试".into(),
                },
            );
            return;
        }
        let ack = Arc::new(AtomicU8::new(0));
        state.sending.insert(
            transfer_id,
            SendingJob {
                peer,
                names: source.names.clone(),
                entries: source.deferred_entries().to_vec(),
                total: source.total_bytes,
                sent_bytes: 0,
                phase: SendPhase::Preparing,
                cancel: Arc::clone(&cancel),
                ack: Arc::clone(&ack),
                receipts: Arc::new(ReceiptWatch::new()),
                created,
            },
        );
        let bridge = self.clone();
        let spawned = std::thread::Builder::new()
            .name("deskfuse-file-send".into())
            .spawn(move || send_job(bridge, peer, transfer_id, roots, source, cancel, ack))
            .is_ok();
        if !spawned {
            state.sending.remove(&transfer_id);
            let _ = self.inner.sink.send(
                peer,
                ClipboardFileFrame::Cancel {
                    transfer_id,
                    reason: "无法启动发送任务".into(),
                },
            );
        }
    }

    fn start_receiving(&self, peer: DeviceId, transfer_id: TransferId, manifest: TransferManifest) {
        let mut state = self.inner.state.lock().expect("file bridge state");
        let Some(offer) = state.incoming.get(&transfer_id) else {
            let _ = self.inner.sink.send(
                peer,
                ClipboardFileFrame::Cancel {
                    transfer_id,
                    reason: "粘贴请求已失效".into(),
                },
            );
            return;
        };
        if offer.peer != peer {
            return;
        }
        if !offer.requested {
            // Only an explicit desktop request may pull content.
            let _ = self.inner.sink.send(
                peer,
                ClipboardFileFrame::Cancel {
                    transfer_id,
                    reason: "粘贴请求已失效".into(),
                },
            );
            return;
        }
        if state.receiving.contains_key(&transfer_id) {
            return;
        }
        if state.receiving.len() >= self.inner.limits.max_receiving {
            state.incoming.remove(&transfer_id);
            let _ = self.inner.sink.send(
                peer,
                ClipboardFileFrame::Cancel {
                    transfer_id,
                    reason: "接收任务已达上限，请稍后重试".into(),
                },
            );
            return;
        }
        if manifest.total_bytes() > self.inner.limits.cache_max_bytes {
            state.incoming.remove(&transfer_id);
            let _ = self.inner.sink.send(
                peer,
                ClipboardFileFrame::Cancel {
                    transfer_id,
                    reason: "文件超过缓存上限".into(),
                },
            );
            return;
        }
        let factory = self.inner.factory.clone();
        let destination = self.inner.cache_root.join(transfer_id.to_hex());
        let (commands, events) = spawn_receive_worker(manifest, transfer_id, factory);
        let IncomingOffer {
            names,
            entries,
            total_bytes,
            created,
            ..
        } = state.incoming.remove(&transfer_id).expect("checked offer");
        state.receiving.insert(
            transfer_id,
            ReceivingTransfer {
                peer,
                names,
                entries,
                total_bytes,
                accepted_frames: 1,
                received_bytes: 0,
                commands: Some(commands),
                events,
                destination,
                finishing: false,
                created,
            },
        );
        // The offer counts as accepted frame 1 for the sender's flow control.
        let _ = self.inner.sink.send(
            peer,
            ClipboardFileFrame::Receipt {
                transfer_id,
                accepted_frames: 1,
            },
        );
    }

    fn accept_chunk(
        &self,
        peer: DeviceId,
        transfer_id: TransferId,
        path: RelativePath,
        descriptor: ChunkDescriptor,
        bytes: Vec<u8>,
    ) {
        let mut state = self.inner.state.lock().expect("file bridge state");
        let Some(active) = state.receiving.get_mut(&transfer_id) else {
            if state
                .pending_apply
                .as_ref()
                .is_none_or(|pending| pending.transfer_id != transfer_id)
            {
                let _ = self.inner.sink.send(
                    peer,
                    ClipboardFileFrame::Cancel {
                        transfer_id,
                        reason: "接收已取消，或未通过容量与完整性检查".into(),
                    },
                );
            }
            return;
        };
        if active.peer != peer || active.finishing {
            return;
        }
        let Some(commands) = active.commands.as_ref() else {
            return;
        };
        // Bounded blocking send: the disk worker drains continuously, so this
        // stalls at most about one chunk write when the queue is full.
        if commands
            .send(WorkerCommand::Chunk {
                path,
                descriptor,
                bytes,
            })
            .is_err()
        {
            // The worker's terminal event reports the failure; the settle pass
            // owns failure reporting, so do not also cancel here.
            active.commands = None;
            return;
        }
        let active = state.receiving.get_mut(&transfer_id).expect("checked");
        active.accepted_frames = active.accepted_frames.saturating_add(1);
        let accepted = active.accepted_frames;
        let _ = self.inner.sink.send(
            peer,
            ClipboardFileFrame::Receipt {
                transfer_id,
                accepted_frames: accepted,
            },
        );
    }

    fn finish_receiving(&self, peer: DeviceId, transfer_id: TransferId) {
        let mut state = self.inner.state.lock().expect("file bridge state");
        let Some(active) = state.receiving.get_mut(&transfer_id) else {
            if state
                .pending_apply
                .as_ref()
                .is_none_or(|pending| pending.transfer_id != transfer_id)
            {
                let _ = self.inner.sink.send(
                    peer,
                    ClipboardFileFrame::Cancel {
                        transfer_id,
                        reason: "接收已取消，或未通过容量与完整性检查".into(),
                    },
                );
            }
            return;
        };
        if active.peer != peer || active.finishing {
            return;
        }
        active.finishing = true;
        if let Some(commands) = active.commands.take() {
            // The worker drains queued chunks first, then verifies and commits.
            let _ = commands.send(WorkerCommand::Finish);
        }
    }

    fn cancel_receiving(
        &self,
        state: &mut State,
        transfer_id: TransferId,
        peer: DeviceId,
        reason: String,
    ) {
        if let Some(mut active) = state.receiving.remove(&transfer_id) {
            active.commands.take(); // closes the channel; the worker aborts staging
            self.push_finished(
                state,
                FileTransferView {
                    transfer_id: transfer_id.to_hex(),
                    peer_id: peer.to_hex(),
                    direction: "incoming".into(),
                    names: active.names,
                    entries: active.entries,
                    total_bytes: active.total_bytes,
                    transferred_bytes: active.received_bytes,
                    state: "cancelled".into(),
                    detail: Some(reason.clone()),
                    staged_paths: Vec::new(),
                    created_unix_ms: unix_ms(active.created),
                    updated_unix_ms: unix_ms_now(),
                },
            );
            remove_dir_best_effort(active.destination);
        }
        let _ = self.inner.sink.send(
            peer,
            ClipboardFileFrame::Cancel {
                transfer_id,
                reason: bounded_reason(&reason),
            },
        );
    }

    fn cancel_from_peer(&self, peer: DeviceId, transfer_id: TransferId, reason: &str) {
        let mut state = self.inner.state.lock().expect("file bridge state");
        if let Some(job) = state.sending.get(&transfer_id)
            && job.peer == peer
        {
            // The sender is already telling us to stop; the job exits quietly.
            job.cancel.store(true, Ordering::Release);
            job.ack.store(2, Ordering::Release);
            return;
        }
        if state
            .receiving
            .get(&transfer_id)
            .is_some_and(|active| active.peer == peer)
        {
            self.cancel_receiving(&mut state, transfer_id, peer, reason.to_owned());
            return;
        }
        if state
            .pending_apply
            .as_ref()
            .is_some_and(|pending| pending.transfer_id == transfer_id && pending.peer == peer)
        {
            let pending = state.pending_apply.take().expect("checked pending");
            self.push_finished(
                &mut state,
                FileTransferView {
                    transfer_id: pending.transfer_id.to_hex(),
                    peer_id: pending.peer.to_hex(),
                    direction: "incoming".into(),
                    names: pending.names,
                    entries: pending.entries,
                    total_bytes: pending.total_bytes,
                    transferred_bytes: pending.total_bytes,
                    state: "cancelled".into(),
                    detail: Some(reason.to_owned()),
                    staged_paths: Vec::new(),
                    created_unix_ms: unix_ms(pending.created),
                    updated_unix_ms: unix_ms_now(),
                },
            );
            remove_dir_best_effort(pending.destination);
            return;
        }
        if state
            .incoming
            .get(&transfer_id)
            .is_some_and(|offer| offer.peer == peer)
        {
            state.incoming.remove(&transfer_id);
        }
        if let Some(cancel) = state
            .outgoing
            .get(&transfer_id)
            .filter(|offer| offer.peer == peer)
            .map(|offer| Arc::clone(&offer.cancel))
        {
            state.outgoing.remove(&transfer_id);
            cancel.store(true, Ordering::Release);
        }
    }

    /// Progress updates from the sending job thread; ignored after a reset.
    fn note_send_phase(&self, transfer_id: TransferId, phase: SendPhase) {
        let mut state = self.inner.state.lock().expect("file bridge state");
        if let Some(job) = state.sending.get_mut(&transfer_id) {
            job.phase = phase;
        }
    }

    fn note_send_bytes(&self, transfer_id: TransferId, added: u64) {
        let mut state = self.inner.state.lock().expect("file bridge state");
        if let Some(job) = state.sending.get_mut(&transfer_id) {
            job.sent_bytes = job.sent_bytes.saturating_add(added);
        }
    }

    fn drain_worker_events(&self, state: &mut State) {
        let mut completions: Vec<(TransferId, Vec<String>)> = Vec::new();
        let mut failures: Vec<(TransferId, String)> = Vec::new();
        for (id, active) in state.receiving.iter_mut() {
            while let Ok(event) = active.events.try_recv() {
                match event {
                    WorkerEvent::Progress(bytes) => {
                        active.received_bytes = active.received_bytes.saturating_add(bytes);
                    }
                    WorkerEvent::Finished(Ok(paths)) => completions.push((*id, paths)),
                    WorkerEvent::Finished(Err(error)) => failures.push((*id, error)),
                }
            }
        }
        for (id, paths) in completions {
            if let Some(active) = state.receiving.remove(&id) {
                // The apply window counts from the moment the bytes are staged
                // and verified, not from the original offer.
                state.pending_apply = Some(PendingApply {
                    transfer_id: id,
                    peer: active.peer,
                    names: active.names,
                    entries: active.entries,
                    total_bytes: active.total_bytes,
                    staged_paths: paths,
                    destination: active.destination,
                    created: Instant::now(),
                });
            }
        }
        for (id, error) in failures {
            if let Some(active) = state.receiving.remove(&id) {
                let detail = format!("文件校验或写入失败：{error}");
                self.push_finished(
                    state,
                    FileTransferView {
                        transfer_id: id.to_hex(),
                        peer_id: active.peer.to_hex(),
                        direction: "incoming".into(),
                        names: active.names,
                        entries: active.entries,
                        total_bytes: active.total_bytes,
                        transferred_bytes: active.received_bytes,
                        state: "failed".into(),
                        detail: Some(detail),
                        staged_paths: Vec::new(),
                        created_unix_ms: unix_ms(active.created),
                        updated_unix_ms: unix_ms_now(),
                    },
                );
                let _ = self.inner.sink.send(
                    active.peer,
                    ClipboardFileFrame::Cancel {
                        transfer_id: id,
                        reason: "文件校验或写入失败".into(),
                    },
                );
                remove_dir_best_effort(active.destination);
            }
        }
        // Terminal sending rows: the ack flag settles the job outcome.
        let settled: Vec<TransferId> = state
            .sending
            .iter()
            .filter(|(_, job)| job.ack.load(Ordering::Acquire) != 0)
            .map(|(id, _)| *id)
            .collect();
        for id in settled {
            let Some(job) = state.sending.remove(&id) else {
                continue;
            };
            let applied = job.ack.load(Ordering::Acquire) == 1;
            let cancelled = job.cancel.load(Ordering::Acquire);
            let (state_label, detail) = if applied {
                ("completed".to_owned(), None)
            } else if cancelled {
                ("cancelled".to_owned(), Some("发送已取消".to_owned()))
            } else {
                ("failed".to_owned(), Some("接收端未能完成校验".to_owned()))
            };
            if applied {
                state.outgoing.remove(&id);
            }
            self.push_finished(
                state,
                FileTransferView {
                    transfer_id: id.to_hex(),
                    peer_id: job.peer.to_hex(),
                    direction: "outgoing".into(),
                    names: job.names,
                    entries: job.entries,
                    total_bytes: job.total,
                    transferred_bytes: job.sent_bytes,
                    state: state_label,
                    detail,
                    staged_paths: Vec::new(),
                    created_unix_ms: unix_ms(job.created),
                    updated_unix_ms: unix_ms_now(),
                },
            );
        }
    }

    fn push_finished(&self, state: &mut State, view: FileTransferView) {
        state.finished.push(FinishedRow {
            view,
            finished_at: Instant::now(),
        });
        while state.finished.len() > 32 {
            state.finished.remove(0);
        }
    }
}

// ---------------------------------------------------------------------------
// Sending job (dedicated thread)
// ---------------------------------------------------------------------------

fn send_job(
    bridge: FileBridge,
    peer: DeviceId,
    transfer_id: TransferId,
    roots: Vec<PathBuf>,
    source: DeferredClipboardSource,
    cancel: Arc<AtomicBool>,
    ack: Arc<AtomicU8>,
) {
    let sink = bridge.inner.sink.clone();
    let cancelled = || cancel.load(Ordering::Acquire);
    // Reply with a Cancel frame only when nobody else has already settled the
    // transfer (a receiver cancel or a session reset pre-set the flag).
    let fail = |detail: &str| {
        if ack.swap(2, Ordering::AcqRel) == 0 {
            let _ = sink.send(
                peer,
                ClipboardFileFrame::Cancel {
                    transfer_id,
                    reason: bounded_reason(detail),
                },
            );
        }
    };
    if source.validate_unchanged_cancellable(cancelled).is_err() {
        fail("源文件已更改、被删除或无法读取，请重新复制");
        return;
    }
    if cancelled() {
        ack.store(2, Ordering::Release);
        return;
    }
    let plan = match ClipboardFilePlan::from_paths_observed(&roots, transfer_id, cancelled, |_| {})
    {
        Ok(plan) => plan,
        Err(error) => {
            fail(&format!("文件读取失败：{error}"));
            return;
        }
    };
    if source.validate_unchanged_cancellable(cancelled).is_err() {
        fail("源文件已更改，请重新复制");
        return;
    }
    bridge.note_send_phase(transfer_id, SendPhase::Sending);
    if sink
        .send(
            peer,
            ClipboardFileFrame::RequestedOffer {
                transfer_id,
                manifest: plan.manifest().clone(),
            },
        )
        .is_err()
    {
        fail("文件通道不可用");
        return;
    }
    let mut sent_frames = 1_u64;
    for source_file in plan.sources() {
        for descriptor in source_file.chunks() {
            if cancelled() {
                fail("发送方已取消");
                return;
            }
            let bytes = match crate::read_chunk(source_file.source_path(), *descriptor) {
                Ok(bytes) => bytes,
                Err(error) => {
                    fail(&format!("读取失败：{error}"));
                    return;
                }
            };
            let frame = ClipboardFileFrame::Chunk {
                transfer_id,
                path: source_file.relative_path().clone(),
                descriptor: *descriptor,
                bytes,
            };
            if sink.send(peer, frame).is_err() {
                fail("文件通道不可用");
                return;
            }
            sent_frames += 1;
            bridge.note_send_bytes(transfer_id, u64::from(descriptor.length));
            // Flow-control credit: the receiver must keep acknowledging frames.
            if sent_frames.is_multiple_of(16) {
                let receipts = {
                    let state = bridge.inner.state.lock().expect("file bridge state");
                    state
                        .sending
                        .get(&transfer_id)
                        .map(|job| Arc::clone(&job.receipts))
                };
                if let Err(error) = wait_for_receipts(receipts, sent_frames, &cancel) {
                    fail(&error);
                    return;
                }
            }
        }
    }
    bridge.note_send_phase(transfer_id, SendPhase::Verifying);
    if sink
        .send(peer, ClipboardFileFrame::Complete { transfer_id })
        .is_err()
    {
        fail("文件通道不可用");
        return;
    }
    let deadline = Instant::now() + bridge.inner.limits.ack_deadline;
    while ack.load(Ordering::Acquire) == 0 {
        if cancelled() {
            fail("发送方已取消");
            return;
        }
        if Instant::now() >= deadline {
            fail("等待接收端校验确认超时，可在保留原文件的情况下重试");
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn wait_for_receipts(
    receipts: Option<Arc<ReceiptWatch>>,
    sent_frames: u64,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let Some(receipts) = receipts else {
        return Err("发送任务已结束".into());
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    while receipts.accepted() < sent_frames {
        if cancel.load(Ordering::Acquire) {
            return Err("发送方已取消".into());
        }
        if Instant::now() >= deadline {
            return Err("等待接收端队列确认超时，请确认两台设备均已更新后重试".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Receiving disk worker (dedicated thread)
// ---------------------------------------------------------------------------

fn spawn_receive_worker(
    manifest: TransferManifest,
    transfer_id: TransferId,
    factory: StoreFactory,
) -> (
    std::sync::mpsc::SyncSender<WorkerCommand>,
    std::sync::mpsc::Receiver<WorkerEvent>,
) {
    let (commands, worker_rx) =
        std::sync::mpsc::sync_channel::<WorkerCommand>(RECEIVE_QUEUE_CHUNKS);
    let (event_tx, event_rx) = std::sync::mpsc::channel::<WorkerEvent>();
    let spawned = std::thread::Builder::new()
        .name("deskfuse-file-receive".into())
        .spawn({
            let event_tx = event_tx.clone();
            move || {
                let mut store = match factory(&manifest, transfer_id) {
                    Ok(store) => store,
                    Err(error) => {
                        let _ = event_tx.send(WorkerEvent::Finished(Err(error)));
                        return;
                    }
                };
                while let Ok(command) = worker_rx.recv() {
                    let finish = matches!(command, WorkerCommand::Finish);
                    match command {
                        WorkerCommand::Chunk {
                            path,
                            descriptor,
                            bytes,
                        } => {
                            if let Err(error) = store.write_chunk(&path, descriptor, &bytes) {
                                let _ = event_tx.send(WorkerEvent::Finished(Err(error)));
                                return;
                            }
                            let _ =
                                event_tx.send(WorkerEvent::Progress(u64::from(descriptor.length)));
                        }
                        WorkerCommand::Finish => match store.complete() {
                            Ok(paths) => {
                                let _ = event_tx.send(WorkerEvent::Finished(Ok(paths)));
                            }
                            Err(error) => {
                                let _ = event_tx.send(WorkerEvent::Finished(Err(error)));
                            }
                        },
                    }
                    if finish {
                        return;
                    }
                }
                // Channel closed without Finish: the transfer was cancelled.
                store.abort();
            }
        })
        .is_ok();
    if !spawned {
        // No worker is available; report the failure immediately.
        let _ = event_tx.send(WorkerEvent::Finished(Err("无法启动接收磁盘任务".into())));
    }
    (commands, event_rx)
}

// ---------------------------------------------------------------------------
// Platform receiving stores
// ---------------------------------------------------------------------------

#[cfg(unix)]
struct UnixStore {
    receiver: FileReceiver,
    staging: Option<FileStaging>,
}

#[cfg(unix)]
impl UnixStore {
    fn new(manifest: TransferManifest, id: TransferId, destination: &Path) -> Result<Self, String> {
        let receiver = FileReceiver::new(destination).map_err(|error| error.to_string())?;
        let staging = receiver
            .begin_transfer(id, &manifest)
            .map_err(|error| error.to_string())?;
        Ok(Self {
            receiver,
            staging: Some(staging),
        })
    }
}

#[cfg(unix)]
impl IncomingStore for UnixStore {
    fn write_chunk(
        &mut self,
        path: &RelativePath,
        descriptor: ChunkDescriptor,
        bytes: &[u8],
    ) -> Result<(), String> {
        let staging = self
            .staging
            .as_mut()
            .ok_or_else(|| "接收任务已结束".to_owned())?;
        staging
            .write_chunk(path, descriptor, bytes)
            .map_err(|error| error.to_string())
    }

    fn complete(&mut self) -> Result<Vec<String>, String> {
        let staging = self
            .staging
            .take()
            .ok_or_else(|| "接收任务已结束".to_owned())?;
        // commit_staging performs the complete-file hash verification exactly
        // once; do not hash every byte twice here.
        let report = self
            .receiver
            .clone()
            .commit_staging(staging, ConflictStrategy::Overwrite)
            .map_err(|error| error.to_string())?;
        Ok(report
            .written
            .into_iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect())
    }

    fn abort(&mut self) {
        if let Some(staging) = self.staging.take() {
            let _ = self.receiver.abort_staging(staging);
        }
    }
}

#[cfg(windows)]
struct WindowsStore {
    destination: PathBuf,
    staging: PathBuf,
    manifest: TransferManifest,
    finished: bool,
}

#[cfg(windows)]
impl WindowsStore {
    fn new(manifest: TransferManifest, id: TransferId, destination: &Path) -> Result<Self, String> {
        // Manifest paths were validated by TransferManifest::new on Windows
        // (reserved names, device aliases, case collisions). The staging root
        // is created fresh by the service; its ancestors are SYSTEM-only.
        let staging = destination.join(format!(".staging-{id}"));
        fs::create_dir_all(&staging).map_err(|error| format!("无法创建暂存目录：{error}"))?;
        Ok(Self {
            destination: destination.to_path_buf(),
            staging,
            manifest,
            finished: false,
        })
    }

    fn entry(&self, path: &RelativePath) -> Result<&crate::TransferEntry, String> {
        self.manifest
            .entries()
            .binary_search_by(|entry| entry.path().cmp(path))
            .ok()
            .map(|index| &self.manifest.entries()[index])
            .ok_or_else(|| format!("路径 {path} 不在文件清单中"))
    }

    fn staged_path(&self, path: &RelativePath) -> Result<PathBuf, String> {
        let target = self.staging.join(path.to_path_buf());
        if target.as_os_str().len() > 900 {
            return Err(format!("暂存路径过长：{path}"));
        }
        Ok(target)
    }
}

#[cfg(windows)]
impl IncomingStore for WindowsStore {
    fn write_chunk(
        &mut self,
        path: &RelativePath,
        descriptor: ChunkDescriptor,
        bytes: &[u8],
    ) -> Result<(), String> {
        if self.finished {
            return Err("接收任务已结束".into());
        }
        descriptor.validate().map_err(|error| error.to_string())?;
        let entry = self.entry(path)?;
        if !entry.is_file() {
            return Err(format!("路径 {path} 不是文件"));
        }
        let end = descriptor
            .offset
            .checked_add(u64::from(descriptor.length))
            .ok_or_else(|| "写入范围溢出".to_owned())?;
        if end > entry.size() {
            return Err(format!("写入块超出文件 {path} 的声明大小"));
        }
        if let Some(expected) = descriptor.hash
            && crate::verify_sha256(bytes, expected).is_err()
        {
            return Err(format!("块校验失败：{path}"));
        }
        let target = self.staged_path(path)?;
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| format!("无法创建暂存子目录：{error}"))?;
            if fs::symlink_metadata(parent)
                .map_err(|error| error.to_string())?
                .file_type()
                .is_symlink()
            {
                return Err("暂存目录中出现了符号链接，已终止接收".into());
            }
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&target)
            .map_err(|error| format!("无法写入暂存文件 {path}：{error}"))?;
        file.seek(SeekFrom::Start(descriptor.offset))
            .and_then(|_| file.write_all(bytes))
            .map_err(|error| format!("暂存写入失败 {path}：{error}"))?;
        Ok(())
    }

    fn complete(&mut self) -> Result<Vec<String>, String> {
        if self.finished {
            return Err("接收任务已结束".into());
        }
        // Verify every complete-file hash before moving anything.
        const BUFFER: usize = 1024 * 1024;
        let mut buffer = vec![0_u8; BUFFER];
        for entry in self.manifest.files() {
            let staged = self.staged_path(entry.path())?;
            let mut file = fs::File::open(&staged)
                .map_err(|error| format!("无法读取暂存文件 {}：{error}", entry.path()))?;
            let metadata = file.metadata().map_err(|error| error.to_string())?;
            if metadata.len() != entry.size() {
                return Err(format!(
                    "暂存文件 {} 大小不符：{} != {}",
                    entry.path(),
                    metadata.len(),
                    entry.size()
                ));
            }
            let mut hasher = sha2::Sha256::new();
            loop {
                let read = file.read(&mut buffer).map_err(|error| error.to_string())?;
                if read == 0 {
                    break;
                }
                hasher.update(&buffer[..read]);
            }
            let digest = sha2::Digest::finalize(hasher);
            let mut actual = [0_u8; 32];
            actual.copy_from_slice(&digest);
            let expected = entry
                .hash()
                .ok_or_else(|| "清单缺少文件校验值".to_owned())?;
            if Sha256Digest::from_bytes(actual) != expected {
                return Err(format!("暂存文件 {} 校验失败", entry.path()));
            }
        }
        // Commit directories first so empty folders and complete subtrees are
        // preserved even when no file supplied a parent directory.
        for entry in self.manifest.directories() {
            let staged = self.staged_path(entry.path())?;
            fs::create_dir_all(&staged)
                .map_err(|error| format!("无法创建暂存目录 {}：{error}", entry.path()))?;
        }
        // Commit: move staged files into the transfer destination.
        let mut written = Vec::new();
        for entry in self.manifest.files() {
            let staged = self.staged_path(entry.path())?;
            let final_path = self.destination.join(entry.path().to_path_buf());
            if let Some(parent) = final_path.parent() {
                fs::create_dir_all(parent).map_err(|error| format!("无法创建接收目录：{error}"))?;
            }
            fs::rename(&staged, &final_path)
                .map_err(|error| format!("无法落盘 {}：{error}", entry.path()))?;
            written.push(final_path.to_string_lossy().into_owned());
        }
        for entry in self.manifest.directories() {
            let staged = self.staged_path(entry.path())?;
            let final_path = self.destination.join(entry.path().to_path_buf());
            fs::create_dir_all(&final_path)
                .map_err(|error| format!("无法落盘目录 {}：{error}", entry.path()))?;
            let _ = fs::remove_dir_all(&staged);
            written.push(final_path.to_string_lossy().into_owned());
        }
        let _ = fs::remove_dir_all(&self.staging);
        self.finished = true;
        Ok(written)
    }

    fn abort(&mut self) {
        self.finished = true;
        let _ = fs::remove_dir_all(&self.destination);
    }
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

fn parse_transfer(value: &str) -> Result<TransferId, String> {
    value
        .parse::<TransferId>()
        .map_err(|_| format!("无效的传输标识：{value}"))
}

fn bounded_reason(reason: &str) -> String {
    reason.chars().take(200).collect()
}

fn unix_ms(created: Instant) -> u64 {
    let elapsed = created.elapsed();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    now.saturating_sub(elapsed).as_millis() as u64
}

fn unix_ms_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Removes a bridge-owned per-transfer directory on a background thread. Only
/// per-transfer directories beneath the cache root are removable; the root
/// itself is never deleted.
fn remove_dir_best_effort(path: PathBuf) {
    std::thread::Builder::new()
        .name("deskfuse-file-cleanup".into())
        .spawn(move || {
            let _ = fs::remove_dir_all(path);
        })
        .ok();
}

fn directory_bytes(root: &Path, max_files: usize) -> u64 {
    let mut total = 0_u64;
    let mut stack = vec![root.to_path_buf()];
    let mut visited = 0_usize;
    while let Some(directory) = stack.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            visited += 1;
            if visited > max_files {
                return total;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.is_dir() {
                stack.push(entry.path());
            } else {
                total = total.saturating_add(metadata.len());
            }
        }
    }
    total
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    const PEER_A: DeviceId = DeviceId::from_bytes([1; 32]);
    const PEER_B: DeviceId = DeviceId::from_bytes([2; 32]);

    #[derive(Default)]
    struct Recorder {
        frames: StdMutex<Vec<ClipboardFileFrame>>,
    }
    impl FileFrameSink for Recorder {
        fn send(&self, _peer: DeviceId, frame: ClipboardFileFrame) -> Result<(), String> {
            self.frames.lock().expect("recorder").push(frame);
            Ok(())
        }
    }
    impl Recorder {
        fn drain_matching(
            &self,
            predicate: impl Fn(&ClipboardFileFrame) -> bool,
        ) -> Vec<ClipboardFileFrame> {
            let mut guard = self.frames.lock().expect("recorder");
            let mut kept = Vec::new();
            let mut matched = Vec::new();
            for frame in guard.drain(..) {
                if predicate(&frame) {
                    matched.push(frame);
                } else {
                    kept.push(frame);
                }
            }
            *guard = kept;
            matched
        }
    }

    struct Harness {
        root: PathBuf,
        bridge: FileBridge,
        recorder: Arc<Recorder>,
    }

    fn harness(name: &str) -> Harness {
        let root =
            std::env::temp_dir().join(format!("deskfuse-bridge-{name}-{}", TransferId::new()));
        fs::create_dir_all(&root).expect("root");
        let recorder = Arc::new(Recorder::default());
        let bridge = FileBridge::with_default_store(root.clone(), recorder.clone());
        Harness {
            root,
            bridge,
            recorder,
        }
    }

    fn write_source(root: &Path, relative: &str, bytes: &[u8]) -> PathBuf {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("parent");
        }
        fs::write(&path, bytes).expect("write");
        path
    }

    fn wait_until(deadline: Duration, mut predicate: impl FnMut() -> bool) -> bool {
        let start = Instant::now();
        while Instant::now().duration_since(start) < deadline {
            if predicate() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        predicate()
    }

    fn offer_view(
        bridge: &FileBridge,
        direction: &'static str,
        state: &'static str,
    ) -> Option<FileTransferView> {
        bridge
            .views()
            .into_iter()
            .find(|view| view.direction == direction && view.state == state)
    }

    fn cleanup(harnesses: &[Harness]) {
        for harness in harnesses {
            harness.bridge.reset(false);
        }
        std::thread::sleep(Duration::from_millis(120));
        for harness in harnesses {
            let _ = fs::remove_dir_all(&harness.root);
        }
    }

    #[test]
    fn local_publish_registers_per_peer_offers_and_deferred_frames() {
        let sender = harness("publish");
        let source = write_source(&sender.root, "report.txt", b"hello");
        let frames = sender
            .bridge
            .publish_local(&[PEER_A, PEER_B], vec![source])
            .expect("publish");
        assert_eq!(frames.len(), 2);
        assert_ne!(frames[0].1, frames[1].1);
        let views = sender.bridge.views();
        assert_eq!(
            views.iter().filter(|v| v.direction == "outgoing").count(),
            2
        );
        for (_, id, frame) in frames {
            let ClipboardFileFrame::Deferred {
                transfer_id,
                names,
                total_bytes,
                ..
            } = frame
            else {
                panic!("expected deferred frame");
            };
            assert_eq!(transfer_id, id);
            assert_eq!(names, vec!["report.txt".to_owned()]);
            assert_eq!(total_bytes, 5);
        }
        sender.bridge.reset(false);
        let _ = fs::remove_dir_all(&sender.root);
    }

    #[test]
    fn shelf_cancel_retires_outgoing_offer_without_removing_source() {
        let sender = harness("shelf-cancel");
        let source = write_source(&sender.root, "keep.txt", b"keep source");
        let frames = sender
            .bridge
            .publish_local(&[PEER_A], vec![source.clone()])
            .unwrap();
        let id = frames[0].1;
        sender.bridge.cancel_transfer(&id.to_hex(), "shelf closed");
        assert!(
            !sender
                .bridge
                .views()
                .iter()
                .any(|row| row.transfer_id == id.to_hex() && row.state == "offered")
        );
        assert_eq!(fs::read(&source).unwrap(), b"keep source");
        sender.bridge.reset(false);
        let _ = fs::remove_dir_all(&sender.root);
    }

    #[test]
    fn publish_requires_peers_and_roots() {
        let sender = harness("publish-guard");
        let source = write_source(&sender.root, "a.txt", b"x");
        assert!(
            sender
                .bridge
                .publish_local(&[], vec![source.clone()])
                .is_err()
        );
        assert!(sender.bridge.publish_local(&[PEER_A], Vec::new()).is_err());
        sender.bridge.reset(false);
        let _ = fs::remove_dir_all(&sender.root);
    }

    #[test]
    fn loopback_transfer_delivers_real_bytes_and_completes() {
        let sender = harness("loop-send");
        let receiver = harness("loop-recv");
        let payload: Vec<u8> = (0..(3 * 1024 * 1024 + 17))
            .map(|i| (i % 251) as u8)
            .collect();
        let source = write_source(&sender.root, "media/blob.bin", &payload);
        let frames = sender
            .bridge
            .publish_local(&[PEER_B], vec![source])
            .expect("publish");
        let (_, transfer_id, deferred) = frames.into_iter().next().expect("one frame");
        receiver.bridge.on_frame(PEER_A, deferred);
        assert!(offer_view(&receiver.bridge, "incoming", "offered").is_some());

        // Desktop accepts the offer; the Request leaves through the sink.
        receiver
            .bridge
            .request_incoming(&transfer_id.to_hex())
            .expect("request");
        let requests = receiver
            .recorder
            .drain_matching(|frame| matches!(frame, ClipboardFileFrame::Request { .. }));
        assert_eq!(requests.len(), 1);

        // The sender answers with a materialised offer and chunk stream.
        sender
            .bridge
            .on_frame(PEER_B, requests.into_iter().next().expect("request"));
        let mut deliverable = Vec::new();
        assert!(
            wait_until(Duration::from_secs(15), || {
                deliverable.extend(
                    sender.recorder.drain_matching(|frame| {
                        !matches!(frame, ClipboardFileFrame::Receipt { .. })
                    }),
                );
                matches!(
                    deliverable.last(),
                    Some(ClipboardFileFrame::Complete { .. })
                )
            }),
            "sender never completed the stream"
        );

        // Feed the whole stream to the receiver, then settle its worker.
        for frame in &deliverable {
            receiver.bridge.on_frame(PEER_A, frame.clone());
        }
        assert!(
            wait_until(Duration::from_secs(15), || {
                receiver.bridge.tick(&[PEER_A]);
                offer_view(&receiver.bridge, "incoming", "staged").is_some()
            }),
            "receiver never staged the transfer"
        );
        let staged = offer_view(&receiver.bridge, "incoming", "staged").expect("staged");
        assert_eq!(staged.total_bytes as usize, payload.len());
        assert!(!staged.staged_paths.is_empty());
        let staged_bytes = fs::read(&staged.staged_paths[0]).expect("staged file");
        assert_eq!(staged_bytes, payload);

        // The desktop copies out and commits; the sender receives the ack.
        receiver
            .bridge
            .commit_incoming(&transfer_id.to_hex(), true)
            .expect("commit");
        let acks = receiver
            .recorder
            .drain_matching(|frame| matches!(frame, ClipboardFileFrame::Ack { .. }));
        assert_eq!(acks.len(), 1);
        sender
            .bridge
            .on_frame(PEER_B, acks.into_iter().next().expect("ack"));
        assert!(
            wait_until(Duration::from_secs(10), || {
                sender.bridge.tick(&[PEER_B]);
                offer_view(&sender.bridge, "outgoing", "completed").is_some()
            }),
            "sender never recorded completion"
        );
        assert!(offer_view(&sender.bridge, "outgoing", "offered").is_none());
        cleanup(&[sender, receiver]);
    }

    #[test]
    fn receiver_can_cancel_before_and_after_request() {
        let sender = harness("cancel-send");
        let receiver = harness("cancel-recv");
        let source = write_source(&sender.root, "big.bin", b"payload");
        let frames = sender
            .bridge
            .publish_local(&[PEER_B], vec![source])
            .expect("publish");
        let (_, transfer_id, deferred) = frames.into_iter().next().expect("frame");
        receiver.bridge.on_frame(PEER_A, deferred);

        // Cancel while only offered: the sender is told directly.
        receiver
            .bridge
            .cancel_incoming(&transfer_id.to_hex(), "用户取消接收");
        let cancels = receiver
            .recorder
            .drain_matching(|frame| matches!(frame, ClipboardFileFrame::Cancel { .. }));
        assert_eq!(cancels.len(), 1);
        assert!(offer_view(&receiver.bridge, "incoming", "offered").is_none());

        // Deliver the cancel to the sender: the outgoing offer is removed.
        sender
            .bridge
            .on_frame(PEER_B, cancels.into_iter().next().expect("cancel"));
        assert!(offer_view(&sender.bridge, "outgoing", "offered").is_none());
        cleanup(&[sender, receiver]);
    }

    #[test]
    fn sender_refuses_requests_for_unknown_or_expired_offers() {
        let sender = harness("refuse");
        sender.bridge.on_frame(
            PEER_B,
            ClipboardFileFrame::Request {
                transfer_id: TransferId::new(),
            },
        );
        let cancels = sender
            .recorder
            .drain_matching(|frame| matches!(frame, ClipboardFileFrame::Cancel { .. }));
        assert_eq!(cancels.len(), 1);

        // An expired offer is refused the same way.
        let limits = Limits {
            offer_lifetime: Duration::from_millis(50),
            ..Limits::default()
        };
        let recorder = Arc::new(Recorder::default());
        let root =
            std::env::temp_dir().join(format!("deskfuse-bridge-expired-{}", TransferId::new()));
        fs::create_dir_all(&root).expect("root");
        let bridge = FileBridge::new(
            root.clone(),
            default_factory(root.clone()),
            recorder.clone(),
            limits,
        );
        let source = write_source(&root, "old.txt", b"x");
        let frames = bridge
            .publish_local(&[PEER_B], vec![source])
            .expect("publish");
        let (_, id, _) = frames.into_iter().next().expect("frame");
        std::thread::sleep(Duration::from_millis(120));
        bridge.tick(&[PEER_B]);
        assert!(offer_view(&bridge, "outgoing", "offered").is_none());
        bridge.on_frame(PEER_B, ClipboardFileFrame::Request { transfer_id: id });
        let cancels =
            recorder.drain_matching(|frame| matches!(frame, ClipboardFileFrame::Cancel { .. }));
        assert_eq!(cancels.len(), 1);
        bridge.reset(false);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn oversize_offers_cannot_be_requested() {
        let receiver = harness("quota");
        let id = TransferId::new();
        receiver.bridge.on_frame(
            PEER_A,
            ClipboardFileFrame::Deferred {
                transfer_id: id,
                names: vec!["huge.bin".into()],
                total_bytes: receiver.bridge.limits().cache_max_bytes + 1,
                entries: vec![DeferredFileEntry {
                    path: RelativePath::new("huge.bin").expect("path"),
                    kind: DeferredEntryKind::File,
                    size: receiver.bridge.limits().cache_max_bytes + 1,
                    lindex: 0,
                }],
            },
        );
        let error = receiver
            .bridge
            .request_incoming(&id.to_hex())
            .expect_err("quota rejection");
        assert!(error.contains("缓存上限"));
        receiver.bridge.reset(false);
        let _ = fs::remove_dir_all(&receiver.root);
    }

    #[test]
    fn duplicate_deferred_announcements_record_one_offer() {
        let receiver = harness("duplicate");
        let id = TransferId::new();
        for _ in 0..3 {
            receiver.bridge.on_frame(
                PEER_A,
                ClipboardFileFrame::Deferred {
                    transfer_id: id,
                    names: vec!["same.txt".into()],
                    total_bytes: 4,
                    entries: vec![DeferredFileEntry {
                        path: RelativePath::new("same.txt").expect("path"),
                        kind: DeferredEntryKind::File,
                        size: 4,
                        lindex: 0,
                    }],
                },
            );
        }
        assert_eq!(receiver.bridge.views().len(), 1);
        receiver.bridge.reset(false);
        let _ = fs::remove_dir_all(&receiver.root);
    }

    #[test]
    fn peer_disconnect_expires_offers_and_receiving_transfers() {
        let receiver = harness("disconnect");
        let sender = harness("disconnect-send");
        let payload = vec![7_u8; 2 * 1024 * 1024];
        let source = write_source(&sender.root, "doc.bin", &payload);
        let frames = sender
            .bridge
            .publish_local(&[PEER_B], vec![source])
            .expect("publish");
        let (_, transfer_id, deferred) = frames.into_iter().next().expect("frame");
        receiver.bridge.on_frame(PEER_A, deferred);
        receiver
            .bridge
            .request_incoming(&transfer_id.to_hex())
            .expect("request");
        let request = receiver
            .recorder
            .drain_matching(|frame| matches!(frame, ClipboardFileFrame::Request { .. }))
            .into_iter()
            .next()
            .expect("request");
        sender.bridge.on_frame(PEER_B, request);
        let mut stream = Vec::new();
        assert!(wait_until(Duration::from_secs(15), || {
            stream.extend(
                sender
                    .recorder
                    .drain_matching(|frame| !matches!(frame, ClipboardFileFrame::Receipt { .. })),
            );
            matches!(stream.last(), Some(ClipboardFileFrame::Complete { .. }))
        }));
        for frame in &stream {
            receiver.bridge.on_frame(PEER_A, frame.clone());
        }
        // The peer disappears: the receiving transfer is cancelled and the
        // sender's job observes the disconnect and replies with a Cancel.
        receiver.bridge.tick(&[]);
        assert!(offer_view(&receiver.bridge, "incoming", "receiving").is_none());
        assert!(wait_until(Duration::from_secs(15), || {
            sender.bridge.tick(&[]);
            !sender
                .recorder
                .drain_matching(|frame| matches!(frame, ClipboardFileFrame::Cancel { .. }))
                .is_empty()
        }));
        cleanup(&[sender, receiver]);
    }

    #[test]
    fn unrequested_offer_rejects_materialised_stream_frames() {
        let receiver = harness("unrequested");
        let sender = harness("unrequested-send");
        let source = write_source(&sender.root, "secret.txt", b"top secret");
        let frames = sender
            .bridge
            .publish_local(&[PEER_B], vec![source])
            .expect("publish");
        let (_, transfer_id, deferred) = frames.into_iter().next().expect("frame");
        receiver.bridge.on_frame(PEER_A, deferred);
        // The sender streams a requested offer that this receiver never asked
        // for (another desktop requested it). The receiver refuses it.
        let manifest = crate::TransferManifest::from_path(write_source(
            &sender.root,
            "secret.txt",
            b"top secret",
        ))
        .expect("manifest");
        receiver.bridge.on_frame(
            PEER_A,
            ClipboardFileFrame::RequestedOffer {
                transfer_id,
                manifest,
            },
        );
        let cancels = receiver
            .recorder
            .drain_matching(|frame| matches!(frame, ClipboardFileFrame::Cancel { .. }));
        assert_eq!(cancels.len(), 1);
        cleanup(&[sender, receiver]);
    }

    #[test]
    fn apply_deadline_cancels_the_sender_and_cleans_staging() {
        let receiver_root = std::env::temp_dir().join(format!(
            "deskfuse-bridge-deadline-recv-{}",
            TransferId::new()
        ));
        fs::create_dir_all(&receiver_root).expect("root");
        let receiver_recorder = Arc::new(Recorder::default());
        let receiver = Harness {
            root: receiver_root.clone(),
            bridge: FileBridge::new(
                receiver_root.clone(),
                default_factory(receiver_root.clone()),
                receiver_recorder.clone(),
                Limits {
                    apply_deadline: Duration::from_millis(300),
                    ..Limits::default()
                },
            ),
            recorder: receiver_recorder,
        };
        let sender = harness("deadline-send");
        let payload = vec![9_u8; 256 * 1024];
        let source = write_source(&sender.root, "slow.bin", &payload);
        let frames = sender
            .bridge
            .publish_local(&[PEER_B], vec![source])
            .expect("publish");
        let (_, transfer_id, deferred) = frames.into_iter().next().expect("frame");
        receiver.bridge.on_frame(PEER_A, deferred);
        receiver
            .bridge
            .request_incoming(&transfer_id.to_hex())
            .expect("request");
        let request = receiver
            .recorder
            .drain_matching(|frame| matches!(frame, ClipboardFileFrame::Request { .. }))
            .into_iter()
            .next()
            .expect("request");
        sender.bridge.on_frame(PEER_B, request);
        let mut stream = Vec::new();
        assert!(wait_until(Duration::from_secs(15), || {
            stream.extend(
                sender
                    .recorder
                    .drain_matching(|frame| !matches!(frame, ClipboardFileFrame::Receipt { .. })),
            );
            matches!(stream.last(), Some(ClipboardFileFrame::Complete { .. }))
        }));
        for frame in &stream {
            receiver.bridge.on_frame(PEER_A, frame.clone());
        }
        assert!(wait_until(Duration::from_secs(15), || {
            receiver.bridge.tick(&[PEER_A]);
            offer_view(&receiver.bridge, "incoming", "staged").is_some()
        }));
        // Never commit; the deadline must cancel the sender and remove bytes.
        let staged_root = receiver.bridge.cache_root().join(transfer_id.to_hex());
        assert!(wait_until(Duration::from_secs(15), || {
            receiver.bridge.tick(&[PEER_A]);
            for frame in receiver
                .recorder
                .drain_matching(|frame| matches!(frame, ClipboardFileFrame::Cancel { .. }))
            {
                sender.bridge.on_frame(PEER_B, frame);
            }
            sender.bridge.tick(&[PEER_B]);
            offer_view(&sender.bridge, "outgoing", "cancelled").is_some() && !staged_root.exists()
        }));
        cleanup(&[sender, receiver]);
    }

    #[test]
    fn failed_commit_reports_failure_to_the_sender() {
        let receiver = harness("commit-fail");
        let sender = harness("commit-fail-send");
        let payload = vec![3_u8; 128 * 1024];
        let source = write_source(&sender.root, "f.txt", &payload);
        let frames = sender
            .bridge
            .publish_local(&[PEER_B], vec![source])
            .expect("publish");
        let (_, transfer_id, deferred) = frames.into_iter().next().expect("frame");
        receiver.bridge.on_frame(PEER_A, deferred);
        receiver
            .bridge
            .request_incoming(&transfer_id.to_hex())
            .expect("request");
        let request = receiver
            .recorder
            .drain_matching(|frame| matches!(frame, ClipboardFileFrame::Request { .. }))
            .into_iter()
            .next()
            .expect("request");
        sender.bridge.on_frame(PEER_B, request);
        let mut stream = Vec::new();
        assert!(wait_until(Duration::from_secs(15), || {
            stream.extend(
                sender
                    .recorder
                    .drain_matching(|frame| !matches!(frame, ClipboardFileFrame::Receipt { .. })),
            );
            matches!(stream.last(), Some(ClipboardFileFrame::Complete { .. }))
        }));
        for frame in &stream {
            receiver.bridge.on_frame(PEER_A, frame.clone());
        }
        assert!(wait_until(Duration::from_secs(15), || {
            receiver.bridge.tick(&[PEER_A]);
            offer_view(&receiver.bridge, "incoming", "staged").is_some()
        }));
        receiver
            .bridge
            .commit_incoming(&transfer_id.to_hex(), false)
            .expect("commit");
        let cancels = receiver
            .recorder
            .drain_matching(|frame| matches!(frame, ClipboardFileFrame::Cancel { .. }));
        assert_eq!(cancels.len(), 1);
        sender
            .bridge
            .on_frame(PEER_B, cancels.into_iter().next().expect("cancel"));
        assert!(wait_until(Duration::from_secs(10), || {
            sender.bridge.tick(&[PEER_B]);
            offer_view(&sender.bridge, "outgoing", "cancelled").is_some()
        }));
        cleanup(&[sender, receiver]);
    }

    #[test]
    fn cache_usage_reports_per_transfer_directories() {
        let receiver = harness("usage");
        assert_eq!(receiver.bridge.cache_usage().used_bytes, 0);
        let id = TransferId::new();
        let dir = receiver.bridge.cache_root().join(id.to_hex());
        fs::create_dir_all(&dir).expect("dir");
        fs::write(dir.join("blob"), vec![0_u8; 2048]).expect("blob");
        receiver.bridge.tick(&[PEER_A]);
        assert_eq!(receiver.bridge.cache_usage().used_bytes, 2048);
        receiver.bridge.reset(false);
        let _ = fs::remove_dir_all(&receiver.root);
    }
}
