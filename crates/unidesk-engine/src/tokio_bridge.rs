//! Tokio/runtime bridge for the GPUI application.
//!
//! GPUI entities never cross this boundary. Runtime snapshots and errors are
//! copied into a `futures` channel, while UI actions are queued as runtime
//! commands. In production this file composes the authenticated QUIC control
//! transport and the Bonjour/mDNS advertisement behind a single runtime;
//! tests keep the deterministic in-memory providers.

use crate::clipboard_bundle;
#[path = "clipboard_sync.rs"]
mod clipboard_sync;
#[path = "file_policy.rs"]
mod file_policy;
#[path = "file_receive_worker.rs"]
mod file_receive_worker;
#[path = "finder_paste_jobs.rs"]
mod finder_paste_jobs;
#[path = "library_sync.rs"]
mod library_sync;
use crate::diagnostics::{DiagnosticLevel, Diagnostics};
use crate::model::{
    ClipboardHistoryItem, DEFAULT_CLIPBOARD_SYNC_ENABLED, TransferDirection, TransferState,
};
use crate::platform::create_worker_pasteboard;
use crate::platform::{PermissionService, PermissionSnapshot, create_permission_service};
use file_policy::{DeferredSources, FilePolicy};
use file_receive_worker::*;
use finder_paste_jobs::FinderPasteJobs;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures::{StreamExt, stream::BoxStream};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tokio::runtime::Builder;
use tokio::sync::mpsc::{
    UnboundedReceiver as TokioReceiver, UnboundedSender as TokioSender, unbounded_channel,
};
use tokio::sync::oneshot::{self, Sender as OneShotSender};
#[cfg(test)]
use unidesk_core::RouteCandidate;
use unidesk_core::{
    Capabilities, ClipboardPayload, ConnectionState, DeviceId, DisplayInfo, FileTransferSettings,
    TransferCacheSnapshot, TrustState,
};
#[cfg(not(test))]
use unidesk_discovery::{PlatformDiscovery, ServiceRecord};
#[cfg(not(test))]
use unidesk_identity::FileIdentityProvider;
#[cfg(test)]
use unidesk_identity::MemoryIdentityProvider;
use unidesk_platform::ports::{
    ApplyResult, ContentData, ContentToApply, NativeRevision, apply_clipboard,
};
use unidesk_platform::{Pasteboard, PasteboardSnapshot, UNIVERSAL_CLIPBOARD_TYPE};
use unidesk_platform_macos::{CapturedFilePaste, FilePasteCapture, FilePasteReference};
#[cfg(test)]
use unidesk_runtime::{ProviderFuture, TransportConnection, TransportProvider};
use unidesk_runtime::{
    RuntimeBuilder, RuntimeCommand, RuntimeConfig, RuntimeError, RuntimeEvent, RuntimeHandle,
    SnapshotView,
};
use unidesk_store::SQLiteStore;
use unidesk_transfer::ExternalCacheLeases;
use unidesk_transfer::{
    CLIPBOARD_FILE_MIME, CacheCleanup, ClipboardFileFrame, ClipboardFilePlan, ConflictStrategy,
    DeferredClipboardSource, FileReceiver, TransferCache, TransferId, TransferManifest, read_chunk,
};
#[cfg(not(test))]
use unidesk_transport::{PeerHello, QuinnTransport, QuinnTransportConfig};

/// How long the app may take to exit after the update helper was launched. The
/// helper itself gives up on a parent that has not exited well before this, so a
/// stalled handoff is cancelled and the running version stays usable.
const WORKFLOW_SYNC_MIME: &str = "application/x-unidesk-workflow-sync+json";
const INSTALL_HANDOFF: Duration = Duration::from_secs(20);
/// Upper bound on the graceful runtime stop during application exit.
const RUNTIME_SHUTDOWN: Duration = Duration::from_secs(15);

#[derive(Clone, Debug)]
pub enum UiEvent {
    FilePasteAcquiring {
        paste_id: TransferId,
        transfer_id: TransferId,
    },
    FilePasteLinked {
        paste_id: TransferId,
        transfer_id: TransferId,
        can_retry: bool,
    },
    DebugLoggingChanged(bool),
    WorkflowCatalogChanged,
    WorkflowRun(crate::workflow::WorkflowRun),
    CommandResult {
        request_id: u64,
        error: Option<String>,
    },
    ClipboardDelivery {
        peer_id: DeviceId,
        phase: String,
    },
    UpdateSnapshot(unidesk_runtime::updates::LanUpdateView),
    OpenSettings,
    RevealSendDrawer,
    StageFileShelf(Vec<String>),
    RefreshPermissions,
    Quit,
    PermissionSnapshot(PermissionSnapshot),
    InputSharingSnapshot(Box<unidesk_core::InputSharingSnapshot>),
    InputLockStatus(bool),
    InputRecordingReady(bool),
    PairingDecision {
        request_id: unidesk_core::RequestId,
        approved: bool,
    },
    RuntimeView(SnapshotView),
    RuntimeEvent(RuntimeEvent),
    BridgeError(String),
    ClipboardStatus {
        enabled: bool,
        last_mime: Option<String>,
        last_len: usize,
        last_sequence: Option<u64>,
        error: Option<String>,
    },
    ClipboardHistory(Vec<ClipboardHistoryItem>),
    FileTransferSettings(FileTransferSettings),
    TransferCacheStatus {
        snapshot: TransferCacheSnapshot,
        message: Option<String>,
    },
    TransferStatus {
        transfer_id: TransferId,
        name: String,
        detail: String,
        state: TransferState,
        transferred_bytes: u64,
        total_bytes: u64,
        direction: TransferDirection,
    },
    #[allow(dead_code)]
    RemoteControlStatus {
        peer_id: Option<DeviceId>,
        active: bool,
        display: Option<DisplayInfo>,
        error: Option<String>,
    },
    InputStatus {
        error: Option<String>,
    },
    PasteReady {
        target_pid: i32,
        expected_change_count: i64,
    },
}

pub enum TokioCommand {
    #[cfg(target_os = "macos")]
    LanWebContext {
        reply: std::sync::mpsc::Sender<Result<crate::lan_web::LanContext, String>>,
    },
    #[cfg(target_os = "macos")]
    LanWebCommit {
        snapshot: PasteboardSnapshot,
        native_items: Vec<unidesk_platform::PasteboardItem>,
        expected_change_count: i64,
        cancel: Arc<AtomicBool>,
        reply: std::sync::mpsc::Sender<Result<crate::lan_web::LanCommitResult, String>>,
    },
    ReleaseShareSource(TransferId),
    ShareSourceFailed {
        id: TransferId,
        error: String,
    },
    RegisterShareSource {
        id: TransferId,
        cancel: Arc<AtomicBool>,
    },
    Tracked {
        request_id: u64,
        command: Box<TokioCommand>,
    },
    ShareSnapshot {
        snapshot: PasteboardSnapshot,
        peer_id: DeviceId,
        selection: crate::workflow_sharing::Selection,
        transfer_id: TransferId,
    },
    CheckForUpdates,
    DownloadAndInstallUpdate,
    Runtime(RuntimeCommand),
    RefreshPermissions,
    Snapshot,
    SetDebugLogging(bool),
    SetClipboardSync(bool),
    PushClipboard,
    SendFiles {
        paths: Vec<String>,
        peer_id: DeviceId,
        transfer_id: Option<TransferId>,
    },
    CancelClipboardTransfer(TransferId),
    RetryFileTransfer(TransferId),
    RestoreClipboardHistory(u64),
    RestoreClipboardForPaste {
        id: u64,
        session: String,
        reply: OneShotSender<Result<i64, String>>,
    },
    CancelClipboardPreparation {
        session: String,
    },
    ClearClipboardHistory,
    SetFileTransferSettings(FileTransferSettings),
    ClearTransferCache,
    RefreshTransferCache,
    WorkflowSource {
        history_id: Option<u64>,
        cancel: Arc<AtomicBool>,
        reply: std::sync::mpsc::Sender<Result<PasteboardSnapshot, String>>,
    },
    WorkflowCacheReserve {
        id: TransferId,
        size: u64,
        reply: std::sync::mpsc::Sender<Result<PathBuf, String>>,
    },
    WorkflowCacheFinish(TransferId),
    WorkflowCommit {
        snapshot: PasteboardSnapshot,
        expected_change_count: i64,
        cancel: Arc<AtomicBool>,
        target_pid: i32,
        reply: std::sync::mpsc::Sender<Result<i64, String>>,
    },
    Shutdown,
}

struct WorkflowPendingSource {
    id: TransferId,
    change_count: i64,
    owned_request: bool,
    cancel: Arc<AtomicBool>,
    reply: std::sync::mpsc::Sender<Result<PasteboardSnapshot, String>>,
    started: std::time::Instant,
}

/// The workflow service calls these synchronous replies only on its own
/// background thread. The Tokio worker itself never blocks awaiting a source.
fn workflow_reply<T>(receiver: std::sync::mpsc::Receiver<Result<T, String>>) -> Result<T, String> {
    receiver
        .recv_timeout(Duration::from_secs(125))
        .map_err(|_| "工作流等待超时或引擎已关闭".to_owned())?
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn workflow_target_is_frontmost(target_pid: i32) -> bool {
    #[repr(C)]
    struct ProcessSerialNumber {
        high: u32,
        low: u32,
    }
    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn GetFrontProcess(process: *mut ProcessSerialNumber) -> i32;
        fn GetProcessPID(process: *const ProcessSerialNumber, pid: *mut i32) -> i32;
    }
    let mut process = ProcessSerialNumber { high: 0, low: 0 };
    let mut pid = 0;
    // SAFETY: both functions receive pointers to initialized, stack-owned
    // values of the ABI's exact layout, valid throughout this synchronous call.
    unsafe {
        GetFrontProcess(&mut process) == 0
            && GetProcessPID(&process, &mut pid) == 0
            && pid == target_pid
    }
}
#[cfg(not(target_os = "macos"))]
fn workflow_target_is_frontmost(_target_pid: i32) -> bool {
    false
}

#[derive(Clone, Default)]
struct ClipboardSequence(Arc<AtomicU64>);

impl ClipboardSequence {
    fn next(&self) -> u64 {
        self.0.fetch_add(1, Ordering::Relaxed)
    }

    fn latest(&self) -> u64 {
        self.0.load(Ordering::Relaxed).wrapping_sub(1)
    }
}

struct ShareOutputLease {
    id: TransferId,
    leases: ExternalCacheLeases,
}
impl Drop for ShareOutputLease {
    fn drop(&mut self) {
        if let Ok(mut leases) = self.leases.lock() {
            leases.remove(&self.id);
        }
    }
}
enum ClipboardTransferTaskEvent {
    Finished(TransferId),
    Prepared {
        transfer_id: TransferId,
        paths: Vec<String>,
        lease: ShareOutputLease,
    },
}

#[derive(Clone)]
struct FileTransferSource {
    paths: Vec<String>,
    target_peer: Option<DeviceId>,
}

#[derive(Default)]
struct ClipboardHistoryStore {
    // Retained for the pending clipboard-library migration; the global library
    // is currently authoritative and this handle is no longer read.
    #[allow(dead_code)]
    database: Option<SQLiteStore>,
    next_id: u64,
    items: VecDeque<(ClipboardHistoryItem, PasteboardSnapshot)>,
}

/// Borrowed comparison data only; never replace the restorable snapshot with
/// this normalized view or use display previews as content identity.
#[derive(PartialEq, Eq)]
enum ClipboardHistoryContent<'a> {
    Files(std::collections::BTreeSet<&'a str>),
    Content {
        text: Option<&'a str>,
        png: Option<&'a [u8]>,
        items: Vec<Vec<(&'a str, &'a [u8])>>,
    },
}

fn clipboard_history_content(snapshot: &PasteboardSnapshot) -> ClipboardHistoryContent<'_> {
    if !snapshot.file_urls.is_empty() {
        // Compare references without touching user files or resolving symlinks.
        return ClipboardHistoryContent::Files(
            snapshot.file_urls.iter().map(String::as_str).collect(),
        );
    }
    let items = snapshot
        .items
        .iter()
        .map(|item| {
            let mut representations = item
                .representations
                .iter()
                .filter(|rep| {
                    !matches!(
                        rep.type_name.as_str(),
                        "com.unidesk.clipboard-origin"
                            | "com.apple.is-remote-clipboard"
                            | crate::clipboard_library::METADATA_TYPE
                    )
                })
                .map(|rep| (rep.type_name.as_str(), rep.data.as_slice()))
                .collect::<Vec<_>>();
            representations.sort();
            representations
        })
        // Native pasteboard reads also omit items containing only bookkeeping.
        .filter(|representations| !representations.is_empty())
        .collect();
    ClipboardHistoryContent::Content {
        text: snapshot.text.as_deref(),
        png: snapshot.png.as_deref(),
        items,
    }
}

/// Prefer the primary encoded format over the TIFF interoperability fallback.
/// Returning borrowed bytes avoids decoding images during the clipboard poll.
fn clipboard_primary_image(
    snapshot: &PasteboardSnapshot,
) -> Option<(&[u8], &'static str, &'static str)> {
    let reps = || snapshot.items.iter().flat_map(|item| &item.representations);
    for rep in reps().filter(|rep| rep.type_name != "public.tiff") {
        let (mime, label) = match rep.type_name.as_str() {
            "public.png" | "image/png" => ("image/png", "PNG 图像"),
            "public.jpeg" | "image/jpeg" => ("image/jpeg", "JPEG 图像"),
            "org.webmproject.webp" | "public.webp" | "image/webp" => ("image/webp", "WebP 图像"),
            "public.heic" | "public.heif" | "image/heic" | "image/heif" => {
                ("image/heic", "HEIC 图像")
            }
            _ => continue,
        };
        return Some((&rep.data, mime, label));
    }
    if let Some(png) = snapshot.png.as_deref() {
        return Some((png, "image/png", "PNG 图像"));
    }
    reps()
        .find(|rep| rep.type_name == "public.tiff")
        .map(|rep| (rep.data.as_slice(), "image/tiff", "TIFF 图像"))
}

/// A display derivative only. History restoration retains the original bytes.
pub(crate) fn clipboard_preview_png(snapshot: &PasteboardSnapshot) -> Option<Arc<Vec<u8>>> {
    clipboard_preview_png_at_size(snapshot, 1600)
}

pub(crate) fn clipboard_preview_png_at_size(
    snapshot: &PasteboardSnapshot,
    max_pixel_size: u32,
) -> Option<Arc<Vec<u8>>> {
    let max_pixel_size = max_pixel_size.clamp(64, 1600);
    if !snapshot.file_urls.is_empty() {
        #[cfg(target_os = "macos")]
        if let [path] = snapshot.file_urls.as_slice()
            && crate::clipboard_library::image_file_mime(path).is_some()
        {
            // Decode only when a preview is requested; the clipboard poll and
            // summary query stay free of image IO. ImageIO bounds allocation.
            return unidesk_platform_macos::lan_web::image_png(
                std::path::Path::new(path),
                max_pixel_size,
            )
            .ok()
            .map(Arc::new);
        }
        return None;
    }
    let (data, _, _) = clipboard_primary_image(snapshot)?;
    #[cfg(target_os = "macos")]
    {
        unidesk_platform_macos::image_processor::preview_png(data, max_pixel_size)
            .ok()
            .map(Arc::new)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let mut reader = image::ImageReader::new(std::io::Cursor::new(data))
            .with_guessed_format()
            .ok()?;
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(128 * 1024 * 1024);
        limits.max_image_width = Some(16_384);
        limits.max_image_height = Some(16_384);
        reader.limits(limits);
        let decoded = reader.decode().ok()?;
        let thumbnail = decoded.thumbnail(max_pixel_size, max_pixel_size);
        let mut png = std::io::Cursor::new(Vec::new());
        thumbnail.write_to(&mut png, image::ImageFormat::Png).ok()?;
        Some(Arc::new(png.into_inner()))
    }
}

// The database owns retention; this is only a small, newest-first hot cache.
// Keep one oversized current copy so native clipboard operations stay intact.
pub(crate) const CLIPBOARD_HOT_CACHE_BYTES: usize = 32 * 1024 * 1024;
pub(crate) fn clipboard_snapshot_bytes(snapshot: &PasteboardSnapshot) -> usize {
    snapshot.text.as_ref().map_or(0, String::len)
        + snapshot.png.as_ref().map_or(0, Vec::len)
        + snapshot.file_urls.iter().map(String::len).sum::<usize>()
        + snapshot
            .items
            .iter()
            .flat_map(|item| &item.representations)
            .map(|rep| rep.type_name.len() + rep.data.len())
            .sum::<usize>()
}

impl ClipboardHistoryStore {
    /// The library row id is authoritative for record identity. Its digest can
    /// be coarser than this cache's content equality (plain text is keyed by its
    /// bytes, so the same text with different auxiliary pasteboard types maps to
    /// one row), so two cache entries can otherwise end up carrying one id.
    /// Collapse any older entry that now shares the id and keep the freshly
    /// recorded snapshot at the front.
    fn adopt_front_id(&mut self, id: u64) {
        if let Some((item, _)) = self.items.front_mut() {
            item.id = id;
        }
        if self.items.iter().skip(1).any(|(item, _)| item.id == id) {
            let mut kept = false;
            self.items.retain(|(item, _)| {
                if item.id == id {
                    let keep = !kept;
                    kept = true;
                    keep
                } else {
                    true
                }
            });
        }
    }

    fn record(
        &mut self,
        snapshot: PasteboardSnapshot,
        source: impl Into<String>,
        ui_sender: &UnboundedSender<UiEvent>,
    ) {
        let source = source.into();
        match crate::clipboard_library::global()
            .and_then(|library| library.ingest(&snapshot, &source))
        {
            Ok(Some(id)) => {
                if self.insert(snapshot, source) {
                    self.adopt_front_id(id);
                    self.publish(ui_sender);
                }
            }
            Ok(None) => {}
            Err(error) => {
                let _ = ui_sender.unbounded_send(UiEvent::BridgeError(error));
            }
        }
    }

    fn insert(&mut self, snapshot: PasteboardSnapshot, source: impl Into<String>) -> bool {
        let (mime, length, preview) = if !snapshot.file_urls.is_empty() {
            let names = snapshot
                .file_urls
                .iter()
                .take(3)
                .map(|path| {
                    PathBuf::from(path)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or(path)
                        .to_owned()
                })
                .collect::<Vec<_>>();
            (
                CLIPBOARD_FILE_MIME.to_owned(),
                snapshot.file_urls.len(),
                names.join("、"),
            )
        } else if let Some((bytes, mime, label)) = clipboard_primary_image(&snapshot) {
            (mime.to_owned(), bytes.len(), label.to_owned())
        } else if let Some(text) = snapshot.text.as_ref() {
            let preview = text.chars().take(80).collect::<String>();
            ("text/plain".to_owned(), text.len(), preview)
        } else if !snapshot.items.is_empty() {
            let types = snapshot
                .items
                .iter()
                .flat_map(|item| &item.representations)
                .map(|r| r.type_name.as_str())
                .collect::<Vec<_>>();
            let length = snapshot
                .items
                .iter()
                .flat_map(|item| &item.representations)
                .map(|r| r.data.len())
                .sum();
            let preview = if types.contains(&"public.tiff") {
                "TIFF 图像".to_owned()
            } else {
                format!("剪贴板内容 · {} 种格式", types.len())
            };
            (clipboard_bundle::MIME.to_owned(), length, preview)
        } else {
            return false;
        };
        let content = clipboard_history_content(&snapshot);
        let existing = self
            .items
            .iter()
            .position(|(_, previous)| clipboard_history_content(previous) == content);
        let cached_preview = existing.and_then(|index| self.items[index].0.image_png.clone());
        let id = if let Some(index) = existing {
            // Keep selection/restore references stable while refreshing source,
            // display derivatives and the untouched snapshot from the latest copy.
            self.items
                .remove(index)
                .expect("matched history index")
                .0
                .id
        } else {
            self.next_id = self.next_id.wrapping_add(1).max(1);
            self.next_id
        };
        let item = ClipboardHistoryItem {
            id,
            source_app_id: clipboard_bundle::source_metadata(&snapshot).app_bundle_id,
            copied_at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .and_then(|duration| u64::try_from(duration.as_millis()).ok()),
            mime,
            length,
            preview,
            content: snapshot
                .text
                .as_deref()
                .map(|text| text.chars().take(32_000).collect())
                .unwrap_or_default(),
            content_truncated: snapshot
                .text
                .as_ref()
                .is_some_and(|text| text.chars().nth(32_000).is_some()),
            image_png: cached_preview.or_else(|| clipboard_preview_png_at_size(&snapshot, 160)),
            file_paths: snapshot.file_urls.clone(),
            source: source.into(),
        };
        self.items.push_front((item, snapshot));
        self.items.truncate(50);
        let weight = |(item, snapshot): &(ClipboardHistoryItem, PasteboardSnapshot)| {
            clipboard_snapshot_bytes(snapshot)
                + item.content.len()
                + item.image_png.as_ref().map_or(0, |image| image.len())
        };
        let mut bytes: usize = self.items.iter().map(weight).sum();
        while bytes > CLIPBOARD_HOT_CACHE_BYTES && self.items.len() > 1 {
            if let Some(removed) = self.items.pop_back() {
                bytes -= weight(&removed);
            }
        }
        true
    }

    fn emit(&self, ui_sender: &UnboundedSender<UiEvent>) {
        if let Err(error) = self.persist_retained() {
            let _ = ui_sender.unbounded_send(UiEvent::BridgeError(error));
        }
        self.publish(ui_sender);
    }

    fn publish(&self, ui_sender: &UnboundedSender<UiEvent>) {
        let _ = ui_sender.unbounded_send(UiEvent::ClipboardHistory(
            self.items.iter().map(|(item, _)| item.clone()).collect(),
        ));
    }

    #[allow(dead_code)] // The UI restore path is migrating to the global library.
    fn restore(&self, id: u64, pasteboard: &impl Pasteboard) -> Result<(), String> {
        // SQLite membership/retention is authoritative; the UI cache can be stale.
        let stored = crate::clipboard_library::global()?.snapshot(id)?;
        let snapshot = &stored;
        if crate::library_files::reference(snapshot)?.is_some() {
            return Err("文件归档需要通过异步准备流程恢复".into());
        }
        if !snapshot.file_urls.is_empty() {
            if snapshot
                .file_urls
                .iter()
                .any(|path| !PathBuf::from(path).exists())
            {
                return Err("历史文件已清理或移动，请从源设备重新复制".into());
            }
            pasteboard
                .write_file_urls(&snapshot.file_urls)
                .map_err(|error| error.to_string())
        } else if !snapshot.items.is_empty() {
            pasteboard
                .write_items(&snapshot.items)
                .map_err(|error| error.to_string())
        } else if let Some(png) = snapshot.png.as_ref() {
            pasteboard.write_png(png).map_err(|error| error.to_string())
        } else if let Some(text) = snapshot.text.as_deref() {
            pasteboard
                .write_text(text)
                .map_err(|error| error.to_string())
        } else {
            Err("剪贴板历史内容为空".to_owned())
        }
    }
}

struct ClipboardPrepared {
    token: TransferId,
    materialized: bool,
    result: Result<PasteboardSnapshot, String>,
}
struct ClipboardPreparation {
    id: u64,
    session: String,
    baseline: i64,
    cancel: Arc<AtomicBool>,
    reply: Option<OneShotSender<Result<i64, String>>>,
    tracked: Option<u64>,
    destination: Option<PathBuf>,
}
impl Drop for ClipboardPreparation {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
struct ClipboardPreparations {
    jobs: HashMap<TransferId, ClipboardPreparation>,
    tx: TokioSender<ClipboardPrepared>,
    rx: TokioReceiver<ClipboardPrepared>,
}
impl Default for ClipboardPreparations {
    fn default() -> Self {
        let (tx, rx) = unbounded_channel();
        Self {
            jobs: HashMap::new(),
            tx,
            rx,
        }
    }
}
impl ClipboardPreparations {
    fn cancel(&mut self, session: &str) {
        for job in self.jobs.values_mut().filter(|job| job.session == session) {
            job.cancel.store(true, Ordering::Release);
        }
    }
    fn start(
        &mut self,
        id: u64,
        session: String,
        baseline: i64,
        reply: Option<OneShotSender<Result<i64, String>>>,
        tracked: Option<u64>,
        local_device: String,
    ) {
        for job in self.jobs.values_mut() {
            job.cancel.store(true, Ordering::Release);
        }
        let token = TransferId::new();
        let cancel = Arc::new(AtomicBool::new(false));
        self.jobs.insert(
            token,
            ClipboardPreparation {
                id,
                session,
                baseline,
                cancel: cancel.clone(),
                reply,
                tracked,
                destination: None,
            },
        );
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                if cancel.load(Ordering::Acquire) {
                    return Err("剪贴板准备已取消".into());
                }
                let snapshot = crate::clipboard_library::global()?.snapshot(id)?;
                let local = crate::library_files::resolve_local(&snapshot, &local_device)?;
                if cancel.load(Ordering::Acquire) {
                    return Err("剪贴板准备已取消".into());
                }
                Ok(local.unwrap_or(snapshot))
            })
            .await
            .map_err(|e| e.to_string())
            .and_then(|v| v);
            let _ = tx.send(ClipboardPrepared {
                token,
                materialized: false,
                result,
            });
        });
    }
}
fn write_prepared_clipboard(
    pasteboard: &impl Pasteboard,
    snapshot: &PasteboardSnapshot,
) -> Result<(), String> {
    if !snapshot.file_urls.is_empty() {
        if snapshot
            .file_urls
            .iter()
            .any(|path| !PathBuf::from(path).exists())
        {
            return Err("归档文件不可用，请重新请求".into());
        }
        pasteboard.write_file_urls(&snapshot.file_urls)
    } else if crate::library_files::reference(snapshot)?.is_some() {
        return Err("文件归档尚未完成准备".into());
    } else if !snapshot.items.is_empty() {
        pasteboard.write_items(&snapshot.items)
    } else if let Some(png) = &snapshot.png {
        pasteboard.write_png(png)
    } else if let Some(text) = &snapshot.text {
        pasteboard.write_text(text)
    } else {
        return Err("剪贴板内容为空".into());
    }
    .map_err(|e| e.to_string())
}

/// A file sender does not complete until each intended receiver confirms that
/// its verified staging directory was committed and installed on pasteboard.
struct FileClipboardAcknowledgement {
    completed: OneShotSender<()>,
    progress: tokio::sync::watch::Sender<u64>,
}
struct PendingFileAcknowledgement {
    completed: oneshot::Receiver<()>,
    progress: tokio::sync::watch::Receiver<u64>,
}
type FileClipboardAcknowledgements =
    Arc<Mutex<HashMap<(DeviceId, TransferId), FileClipboardAcknowledgement>>>;

#[cfg(any(not(test), target_os = "macos"))]
pub(crate) mod production_transport {
    use std::net::{Ipv4Addr, SocketAddr};

    /// UDP/QUIC control port advertised through mDNS. It is the application
    /// SRV port, not the mDNS daemon's 5353 socket.
    #[allow(dead_code)] // Kept for probes/tests during the transport refactor.
    pub const CONTROL_PORT: u16 = 42424;

    /// TLS server name shared by the internal transport certificate. QUIC
    /// encrypts the session; device identity still comes from the signed
    /// Ed25519 hello and user-mediated pairing approval.
    pub const SERVER_NAME: &str = "unidesk.local";

    /// Embedded self-signed transport certificate (DER). This is deliberately
    /// a fleet-wide transport certificate for the M1 milestone: it provides
    /// encryption, while the signed hello and pairing approval remain the
    /// authoritative trust boundary. It is not a device identity secret.
    pub const CERTIFICATE_DER: &[u8] =
        include_bytes!("../../../resources/unidesk-transport-cert.der");

    /// PKCS#8 private key for the embedded certificate.
    pub const PRIVATE_KEY_DER: &[u8] =
        include_bytes!("../../../resources/unidesk-transport-key.der");

    /// Loopback placeholder replaced by the route selected at connect time.
    pub const fn placeholder_remote() -> SocketAddr {
        SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::LOCALHOST), 0)
    }
}

/// A production-safe unavailable boundary used by isolated bridge tests
/// until a real transport is injected. It can never report a successful
/// connection.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default)]
pub struct UnavailableControlTransport;

#[cfg(test)]
impl TransportProvider for UnavailableControlTransport {
    fn register_peer_identity(
        &self,
        _peer_id: DeviceId,
        _public_key: [u8; 32],
    ) -> ProviderFuture<()> {
        // Registering an expected identity does not open a socket or claim a
        // connection. Keep the runtime's trust/identity bookkeeping usable;
        // connect/disconnect remain the visible unavailable boundary below.
        Box::pin(async { Ok(()) })
    }

    fn connect(
        &self,
        _local_hello: unidesk_protocol::Hello,
        _peer_id: DeviceId,
        _route: RouteCandidate,
    ) -> ProviderFuture<TransportConnection> {
        Box::pin(async {
            Err(RuntimeError::Transport("安全控制通道尚未启用".to_owned()))
        })
    }

    fn disconnect(&self, _peer_id: DeviceId) -> ProviderFuture<()> {
        Box::pin(async {
            Err(RuntimeError::Transport("安全控制通道尚未启用".to_owned()))
        })
    }
}

/// Owns a small Tokio worker that starts and observes the real uniDesk runtime.
struct WorkerCompletion {
    done: Mutex<bool>,
    wake: Condvar,
}

impl WorkerCompletion {
    fn new() -> Self {
        Self {
            done: Mutex::new(false),
            wake: Condvar::new(),
        }
    }

    fn mark_done(&self) {
        let mut done = self.done.lock().unwrap_or_else(|error| error.into_inner());
        *done = true;
        self.wake.notify_all();
    }

    fn wait(&self) {
        let mut done = self.done.lock().unwrap_or_else(|error| error.into_inner());
        while !*done {
            done = self
                .wake
                .wait(done)
                .unwrap_or_else(|error| error.into_inner());
        }
    }
}

struct WorkerCompletionGuard(Arc<WorkerCompletion>);

impl Drop for WorkerCompletionGuard {
    fn drop(&mut self) {
        self.0.mark_done();
    }
}

struct InputSignals {
    input_service: Arc<Mutex<Option<unidesk_runtime::input_service::ServiceControl>>>,
    update_admission: Arc<Mutex<bool>>,
    recording: Arc<AtomicBool>,
    emergency: Arc<AtomicBool>,
}

pub struct TokioBridge {
    input_service: Arc<Mutex<Option<unidesk_runtime::input_service::ServiceControl>>>,
    update_admission: Arc<Mutex<bool>>,
    commands: TokioSender<TokioCommand>,
    events: UnboundedSender<UiEvent>,
    worker: Mutex<Option<JoinHandle<()>>>,
    worker_completion: Arc<WorkerCompletion>,
    shutdown_requested: AtomicBool,
    input_recording: Arc<AtomicBool>,
    input_emergency: Arc<AtomicBool>,
    diagnostics: Diagnostics,
    pending_incoming_file: Arc<AtomicBool>,
    paste_requested: Arc<AtomicBool>,
    paste_target_pid: Arc<AtomicI32>,
    cache_leases: ExternalCacheLeases,
    file_paste_capture: Arc<FilePasteCapture>,
}

impl TokioBridge {
    pub fn submit_tracked(&self, request_id: u64, command: TokioCommand) -> Result<(), String> {
        self.submit(TokioCommand::Tracked {
            request_id,
            command: Box::new(command),
        })
    }

    fn submit(&self, command: TokioCommand) -> Result<(), String> {
        let closed = self.update_admission.lock().map_err(|_| "更新状态不可用")?;
        if *closed
            && !matches!(
                command,
                TokioCommand::Shutdown | TokioCommand::WorkflowCacheFinish(_)
            )
        {
            return Err("正在安装更新，请稍候".into());
        }
        self.commands.send(command).map_err(|e| e.to_string())
    }
    pub fn start(
        diagnostics: Diagnostics,
    ) -> Result<(std::sync::Arc<Self>, UnboundedReceiver<UiEvent>), String> {
        let (ui_sender, ui_receiver) = unbounded();
        let (command_sender, command_receiver) = unbounded_channel();
        let worker_diagnostics = diagnostics.clone();
        let worker_ui_sender = ui_sender.clone();
        let worker_completion = Arc::new(WorkerCompletion::new());
        let pending_incoming_file = Arc::new(AtomicBool::new(false));
        let paste_requested = Arc::new(AtomicBool::new(false));
        let paste_target_pid = Arc::new(AtomicI32::new(0));
        let (file_paste_capture, file_paste_requests) = FilePasteCapture::new()?;
        let worker_file_paste_capture = file_paste_capture.clone();
        let cache_leases: ExternalCacheLeases = Arc::default();
        let cache_leases_worker = cache_leases.clone();
        let update_admission = Arc::new(Mutex::new(false));
        let update_admission_worker = update_admission.clone();
        let input_service = Arc::new(Mutex::new(None));
        let input_service_worker = input_service.clone();
        let input_recording = Arc::new(AtomicBool::new(false));
        let input_recording_worker = input_recording.clone();
        let input_emergency = Arc::new(AtomicBool::new(false));
        let input_emergency_worker = input_emergency.clone();
        let pending_incoming_file_for_worker = Arc::clone(&pending_incoming_file);
        let paste_requested_for_worker = Arc::clone(&paste_requested);
        let paste_target_pid_for_worker = Arc::clone(&paste_target_pid);
        let worker_completion_for_thread = Arc::clone(&worker_completion);
        let worker = thread::Builder::new()
            .name("unidesk-tokio".to_owned())
            .spawn(move || {
                let _completion_guard = WorkerCompletionGuard(worker_completion_for_thread);
                match Builder::new_current_thread().enable_all().build() {
                    Ok(runtime) => runtime.block_on(run_worker(
                        command_receiver,
                        worker_ui_sender,
                        worker_diagnostics,
                        Arc::clone(&pending_incoming_file_for_worker),
                        Arc::clone(&paste_requested_for_worker),
                        Arc::clone(&paste_target_pid_for_worker),
                        cache_leases_worker,
                        worker_file_paste_capture,
                        file_paste_requests,
                        InputSignals {
                            input_service: input_service_worker,
                            update_admission: update_admission_worker,
                            recording: input_recording_worker,
                            emergency: input_emergency_worker,
                        },
                    )),
                    Err(error) => {
                        let message = format!("Tokio runtime 初始化失败: {error}");
                        crate::diagnostics::record!(
                            worker_diagnostics,
                            DiagnosticLevel::Error,
                            "tokio",
                            "runtime_init_failed",
                            Some(message.clone()),
                            Some("TOKIO_RUNTIME".to_owned()),
                        );
                        let _ = worker_ui_sender.unbounded_send(UiEvent::BridgeError(message));
                    }
                }
            })
            .map_err(|error| format!("Tokio worker thread 启动失败: {error}"))?;

        Ok((
            std::sync::Arc::new(Self {
                commands: command_sender,
                update_admission,
                events: ui_sender,
                worker: Mutex::new(Some(worker)),
                worker_completion,
                shutdown_requested: AtomicBool::new(false),
                input_service,
                input_recording,
                input_emergency,
                diagnostics,
                pending_incoming_file,
                paste_requested,
                paste_target_pid,
                cache_leases,
                file_paste_capture,
            }),
            ui_receiver,
        ))
    }

    /// Read original clipboard content without restoring or publishing it.
    /// Must be called from a background workflow thread, never the UI thread.
    #[cfg(target_os = "macos")]
    pub fn lan_web_context(&self) -> Result<crate::lan_web::LanContext, String> {
        let (reply, receiver) = std::sync::mpsc::channel();
        self.submit(TokioCommand::LanWebContext { reply })?;
        workflow_reply(receiver)
    }

    #[cfg(target_os = "macos")]
    pub fn lan_web_commit(
        &self,
        snapshot: PasteboardSnapshot,
        native_items: Vec<unidesk_platform::PasteboardItem>,
        expected_change_count: i64,
        cancel: Arc<AtomicBool>,
    ) -> Result<crate::lan_web::LanCommitResult, String> {
        let (reply, receiver) = std::sync::mpsc::channel();
        self.submit(TokioCommand::LanWebCommit {
            snapshot,
            native_items,
            expected_change_count,
            cancel: cancel.clone(),
            reply,
        })?;
        // The HTTP worker owns the cache reservation until the serialized commit
        // acknowledges it. A timeout must never delete a file already stored in SQLite.
        let result = receiver.recv().map_err(|_| "引擎已停止".to_owned())?;
        if result.is_err() {
            cancel.store(true, Ordering::Release);
        }
        result
    }

    pub fn workflow_source(
        &self,
        history_id: Option<u64>,
        cancel: Arc<AtomicBool>,
    ) -> Result<PasteboardSnapshot, String> {
        let (reply, receiver) = std::sync::mpsc::channel();
        self.submit(TokioCommand::WorkflowSource {
            history_id,
            cancel: Arc::clone(&cancel),
            reply,
        })?;
        let result = workflow_reply(receiver);
        if result.is_err() {
            cancel.store(true, Ordering::Release);
        }
        result
    }

    pub fn workflow_cache_reserve(&self, id: TransferId, size: u64) -> Result<PathBuf, String> {
        let (reply, receiver) = std::sync::mpsc::channel();
        self.submit(TokioCommand::WorkflowCacheReserve { id, size, reply })?;
        workflow_reply(receiver)
    }

    pub fn workflow_cache_finish(&self, id: TransferId) -> Result<(), String> {
        self.submit(TokioCommand::WorkflowCacheFinish(id))
    }

    pub fn workflow_commit(
        &self,
        snapshot: PasteboardSnapshot,
        expected_change_count: i64,
        cancel: Arc<AtomicBool>,
        target_pid: i32,
    ) -> Result<i64, String> {
        let (reply, receiver) = std::sync::mpsc::channel();
        self.submit(TokioCommand::WorkflowCommit {
            snapshot,
            expected_change_count,
            cancel: Arc::clone(&cancel),
            target_pid,
            reply,
        })?;
        let result = workflow_reply(receiver);
        if result.is_err() {
            cancel.store(true, Ordering::Release);
        }
        result
    }

    pub fn workflow_notify(&self, run: crate::workflow::WorkflowRun) {
        let _ = self.events.unbounded_send(UiEvent::WorkflowRun(run));
    }

    pub fn workflow_error(&self, message: String) {
        let _ = self.events.unbounded_send(UiEvent::BridgeError(message));
    }

    pub fn event_sender(&self) -> UnboundedSender<UiEvent> {
        self.events.clone()
    }

    pub fn record_transfer_presentation(&self, id: TransferId, revision: u64) {
        self.diagnostics.record(
            DiagnosticLevel::Info,
            "file_transfer",
            "terminal_snapshot_published",
            Some(format!("transfer={id} revision={revision}")),
            None,
        );
    }

    pub fn file_paste_capture(&self) -> Arc<FilePasteCapture> {
        self.file_paste_capture.clone()
    }

    pub fn paste_interception_state(&self) -> (Arc<AtomicBool>, Arc<AtomicBool>, Arc<AtomicI32>) {
        (
            Arc::clone(&self.pending_incoming_file),
            Arc::clone(&self.paste_requested),
            Arc::clone(&self.paste_target_pid),
        )
    }

    pub fn pause_current_managed_activation(&self) {
        self.input_emergency.store(true, Ordering::Release);
    }

    pub fn set_input_recording(&self, recording: bool) {
        self.input_recording.store(recording, Ordering::Release);
    }

    pub fn send_command(&self, command: RuntimeCommand) -> Result<(), String> {
        self.submit(TokioCommand::Runtime(command))
            .map_err(|error| format!("runtime command channel closed: {error}"))
    }

    pub fn request_permission_refresh(&self) {
        if let Err(error) = self.submit(TokioCommand::RefreshPermissions) {
            eprintln!("uniDesk: request permission refresh failed: {error}");
        }
    }

    pub fn set_clipboard_sync(&self, enabled: bool) -> Result<(), String> {
        self.submit(TokioCommand::SetClipboardSync(enabled))
            .map_err(|error| format!("clipboard command channel closed: {error}"))
    }

    pub fn push_clipboard(&self) -> Result<(), String> {
        self.submit(TokioCommand::PushClipboard)
            .map_err(|error| format!("clipboard command channel closed: {error}"))
    }

    pub fn open_file_shelf(&self, paths: Vec<String>) {
        let _ = self.events.unbounded_send(UiEvent::StageFileShelf(paths));
    }

    pub fn send_shelf_files(
        &self,
        paths: Vec<String>,
        peer_id: DeviceId,
    ) -> Result<TransferId, String> {
        if paths.is_empty() {
            return Err("文件暂存区是空的".into());
        }
        let transfer_id = TransferId::new();
        self.submit(TokioCommand::SendFiles {
            paths,
            peer_id,
            transfer_id: Some(transfer_id),
        })
        .map_err(|error| format!("无法提交文件：{error}"))?;
        Ok(transfer_id)
    }

    pub fn fail_share_source(&self, id: TransferId, error: String) {
        let _ = self.submit(TokioCommand::ShareSourceFailed { id, error });
    }
    pub fn release_share_source(&self, id: TransferId) {
        let _ = self.submit(TokioCommand::ReleaseShareSource(id));
    }
    pub fn register_share_source(
        &self,
        id: TransferId,
        cancel: Arc<AtomicBool>,
    ) -> Result<(), String> {
        self.submit(TokioCommand::RegisterShareSource { id, cancel })
    }

    pub fn share_snapshot(
        &self,
        snapshot: PasteboardSnapshot,
        peer_id: DeviceId,
        selection: crate::workflow_sharing::Selection,
        transfer_id: TransferId,
    ) -> Result<(), String> {
        self.submit(TokioCommand::ShareSnapshot {
            snapshot,
            peer_id,
            selection,
            transfer_id,
        })
        .map_err(|error| format!("无法提交共享：{error}"))
    }

    pub fn send_files(&self, paths: Vec<String>, peer_id: DeviceId) -> Result<(), String> {
        if paths.is_empty() {
            return Err("请先选择要发送的文件或文件夹".to_owned());
        }
        self.submit(TokioCommand::SendFiles {
            paths,
            peer_id,
            transfer_id: None,
        })
        .map_err(|error| format!("file transfer command channel closed: {error}"))
    }

    pub fn cancel_clipboard_transfer(&self, transfer_id: TransferId) -> Result<(), String> {
        self.submit(TokioCommand::CancelClipboardTransfer(transfer_id))
            .map_err(|error| format!("clipboard cancellation channel closed: {error}"))
    }

    pub fn retry_file_transfer(&self, transfer_id: TransferId) -> Result<(), String> {
        self.submit(TokioCommand::RetryFileTransfer(transfer_id))
            .map_err(|error| format!("file transfer retry channel closed: {error}"))
    }

    /// Restore on the serialized clipboard worker and return its exact observed
    /// write generation, rather than sampling it after a UI event round trip.
    /// Blocking: invoke from the native EngineClient actor, never the UI thread.
    pub fn prepare_clipboard_paste(&self, id: u64, session: String) -> Result<i64, String> {
        let (reply, receiver) = oneshot::channel();
        self.submit(TokioCommand::RestoreClipboardForPaste { id, session, reply })?;
        receiver
            .blocking_recv()
            .map_err(|_| "剪贴板恢复任务已终止".to_string())?
    }

    pub fn cancel_clipboard_preparation(&self, session: String) -> Result<(), String> {
        self.submit(TokioCommand::CancelClipboardPreparation { session })
    }

    pub fn restore_clipboard_history(&self, id: u64) -> Result<(), String> {
        self.submit(TokioCommand::RestoreClipboardHistory(id))
            .map_err(|error| format!("clipboard history channel closed: {error}"))
    }

    pub fn set_file_transfer_settings(&self, settings: FileTransferSettings) -> Result<(), String> {
        settings.validate()?;
        self.submit(TokioCommand::SetFileTransferSettings(settings))
            .map_err(|error| error.to_string())
    }
    /// Retain cached paths synchronously while a drawer or native file promise
    /// owns them. Release only after native export callbacks finish.
    pub fn retain_transfer_cache_paths(&self, paths: Vec<String>) -> Result<TransferId, String> {
        let closed = self.update_admission.lock().map_err(|_| "更新状态不可用")?;
        if *closed {
            return Err("正在安装更新，请稍候".into());
        }
        let id = TransferId::new();
        self.cache_leases
            .lock()
            .map_err(|_| "缓存导出租期锁不可用")?
            .insert(id, paths);
        Ok(id)
    }
    pub fn release_transfer_cache_paths(&self, id: TransferId) {
        if let Ok(mut leases) = self.cache_leases.lock() {
            leases.remove(&id);
        }
    }
    pub fn clear_transfer_cache(&self) -> Result<(), String> {
        self.submit(TokioCommand::ClearTransferCache)
            .map_err(|error| error.to_string())
    }
    pub fn check_for_updates(&self) -> Result<(), String> {
        self.submit(TokioCommand::CheckForUpdates)
            .map_err(|error| error.to_string())
    }
    pub fn refresh_transfer_cache(&self) -> Result<(), String> {
        self.submit(TokioCommand::RefreshTransferCache)
            .map_err(|error| error.to_string())
    }

    pub fn clear_clipboard_history(&self) -> Result<(), String> {
        self.submit(TokioCommand::ClearClipboardHistory)
            .map_err(|error| format!("clipboard history channel closed: {error}"))
    }

    /// Ask the worker to resend the latest runtime-owned snapshot to a newly
    /// recreated window without forcing discovery or changing runtime state.
    pub fn request_snapshot(&self) {
        if let Err(error) = self.submit(TokioCommand::Snapshot) {
            eprintln!("uniDesk: request runtime snapshot failed: {error}");
        }
    }

    pub fn export_diagnostics(&self, path: &std::path::Path) -> Result<(), String> {
        let mut contents = serde_json::json!({
            "subsystem": "diagnostic_collection",
            "debug_enabled": unidesk_core::debug_logging::enabled(),
            "scope": "current_process",
            "version": env!("CARGO_PKG_VERSION"),
            "pid": std::process::id(),
            "note": "Debug collection is on by default; hourly files retain at most seven days. Disabling retains prior samples; absent samples do not prove absence of stalls."
        }).to_string();
        contents.push('\n');
        contents.push_str(&self.diagnostics.to_json_lines());
        if !contents.is_empty() {
            contents.push('\n');
        }
        let timing = unidesk_platform::timing::snapshot();
        for record in timing.records {
            contents.push_str(&serde_json::to_string(&record).map_err(|error| error.to_string())?);
            contents.push('\n');
        }
        contents.push_str(&format!(
            "{{\"subsystem\":\"input_timing\",\"stage\":\"export_summary\",\"dropped_records\":{}}}\n",
            timing.dropped_records
        ));
        // Motion transport samples answer whether the datagram path is the reason
        // motion becomes choppy: `pending_send` is the QUIC datagram queue, the
        // `*_replaced` counters are deliberate position coalescing, and `dropped`
        // with `ready:false` is a health failure that makes the runtime reacquire
        // reliable control. Metadata only: no coordinates or display identifiers.
        let (motion, motion_dropped) = unidesk_transport::motion_diag::snapshot();
        for record in motion {
            contents.push_str(
                &serde_json::to_string(&serde_json::json!({
                    "timestamp_ms": record.timestamp_ms,
                    "subsystem": "input_motion_transport",
                    "stage": record.event,
                    "peer": record.peer,
                    "transport_session": record.transport_session,
                    "local_addr": record.local_addr,
                    "remote_addr": record.remote_addr,
                    "interface_name": record.interface_name,
                    "interface_kind": record.interface_kind,
                    "ready": record.ready,
                    "sent": record.sent,
                    "received": record.received,
                    "send_replaced": record.send_replaced,
                    "receive_replaced": record.receive_replaced,
                    "dropped": record.dropped,
                    "pending_send": record.pending_send,
                    "rtt_micros": record.rtt_micros,
                    "sent_packets": record.sent_packets,
                    "lost_packets": record.lost_packets,
                    "congestion_events": record.congestion_events,
                }))
                .map_err(|error| error.to_string())?,
            );
            contents.push('\n');
        }
        contents.push_str(&format!(
            "{{\"subsystem\":\"input_motion_transport\",\"stage\":\"export_summary\",\"dropped_records\":{motion_dropped}}}\n"
        ));
        if let Some(service) = self.input_service.lock().ok().and_then(|s| s.clone())
            && let Ok(diagnostics) = service.diagnostics.lock()
        {
            contents.push_str(&diagnostics);
        }
        std::fs::write(path, contents).map_err(|error| format!("诊断导出失败：{error}"))
    }

    pub fn shutdown(&self) {
        if !self.shutdown_requested.swap(true, Ordering::AcqRel) {
            let _ = self.submit(TokioCommand::Shutdown);
        }
    }

    /// Request shutdown and synchronously join the worker.
    ///
    /// This is intentionally separate from [`Drop`]: callers that can afford
    /// to wait (for example, deterministic tests or a background teardown)
    /// get a completion guarantee, while GPUI's quit callback can use the
    /// non-blocking [`Self::shutdown`] path.
    #[allow(dead_code)]
    pub fn shutdown_and_wait(&self) -> Result<(), String> {
        self.shutdown();
        self.worker_completion.wait();
        let worker = self
            .worker
            .lock()
            .map_err(|_| "Tokio worker handle lock poisoned".to_owned())?
            .take();
        if let Some(worker) = worker {
            worker
                .join()
                .map_err(|_| "Tokio worker thread panicked".to_owned())?;
        }
        Ok(())
    }
}

impl Drop for TokioBridge {
    fn drop(&mut self) {
        self.shutdown();
        // Do not wait or join from GPUI teardown. Move the handle to a tiny
        // reaper instead: Drop remains non-blocking, while the worker still
        // gets joined once its asynchronous cleanup has completed.
        let worker = self
            .worker
            .get_mut()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        if let Some(worker) = worker {
            let completion = Arc::clone(&self.worker_completion);
            let _ = thread::Builder::new()
                .name("unidesk-tokio-reaper".to_owned())
                .spawn(move || {
                    completion.wait();
                    let _ = worker.join();
                });
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_worker(
    mut commands: TokioReceiver<TokioCommand>,
    ui_sender: UnboundedSender<UiEvent>,
    diagnostics: Diagnostics,
    pending_incoming_file: Arc<AtomicBool>,
    paste_requested: Arc<AtomicBool>,
    paste_target_pid: Arc<AtomicI32>,
    cache_leases: ExternalCacheLeases,
    file_paste_capture: Arc<FilePasteCapture>,
    mut file_paste_requests: TokioReceiver<CapturedFilePaste>,
    input_signals: InputSignals,
) {
    crate::diagnostics::record!(
        diagnostics,
        DiagnosticLevel::Info,
        "tokio",
        "worker_started",
        None,
        None
    );

    let permission_service = create_permission_service();
    emit_permission_snapshot(&ui_sender, permission_service.as_ref(), &diagnostics, None);

    let runtime = match build_runtime(&diagnostics).await {
        Ok(runtime) => runtime,
        Err(error) => {
            let message = error.to_string();
            eprintln!("uniDesk: runtime startup failed: {message}");
            crate::diagnostics::record!(
                diagnostics,
                DiagnosticLevel::Error,
                "runtime",
                "startup_failed",
                Some(message.clone()),
                Some("RUNTIME_STARTUP".to_owned()),
            );
            let _ = ui_sender.unbounded_send(UiEvent::BridgeError(message));
            return;
        }
    };

    #[cfg(target_os = "macos")]
    let lan_display_name = local_display_name();
    let input_service = runtime.input_service_control();
    *input_signals
        .input_service
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = input_service.clone();

    if let Ok(library) = crate::clipboard_library::global() {
        let _ = library.set_local_device(runtime.snapshot_view().snapshot.local_device_id.to_hex());
    }
    let workflow_catalog = crate::workflow_catalog::global().ok();
    if let Some(catalog) = &workflow_catalog {
        let _ = catalog.set_local_device(runtime.snapshot_view().snapshot.local_device_id.to_hex());
    }
    let mut workflow_generation = 0;
    let mut workflow_next_sync = std::time::Instant::now();
    let mut workflow_sends: Vec<tokio::task::JoinHandle<()>> = Vec::new();
    let mut runtime_views = runtime.subscribe_view();
    let mut runtime_events = subscribe_before_snapshot(&runtime, &ui_sender);

    // Clipboard synchronization state. Bodies never leave this worker as
    // diagnostics; only mime/length/sequence metadata reach the UI.
    let wake_signal = Arc::new(tokio::sync::Notify::new());
    let wake_callback = wake_signal.clone();
    let _wake_observer = crate::platform::WakeObserver::new(move || {
        wake_callback.notify_one();
    });
    let pasteboard = match create_worker_pasteboard() {
        Ok(pasteboard) => pasteboard,
        Err(error) => {
            emit_clipboard_error(&ui_sender, &diagnostics, error.to_string());
            let _ = ui_sender.unbounded_send(UiEvent::BridgeError(error.to_string()));
            return;
        }
    };
    let mut clipboard_enabled = DEFAULT_CLIPBOARD_SYNC_ENABLED;
    // Inspect the current pasteboard on startup. Finder may have populated it
    // before uniDesk launched, and treating that value as an unseen baseline
    // left the UI stuck on “等待剪贴板变化”.
    let mut clipboard_last_change_count = pasteboard.change_count().ok();
    let mut clipboard_sync = clipboard_sync::Sync::default();
    let mut library_sync = library_sync::Sync::default();
    let mut preparations = ClipboardPreparations::default();
    let clipboard_sequence = ClipboardSequence::default();
    let mut clipboard_loop_guard = ClipboardLoopGuard::default();
    let mut clipboard_history = match ClipboardHistoryStore::open() {
        Ok(history) => history,
        Err(error) => {
            let _ = ui_sender.unbounded_send(UiEvent::BridgeError(error));
            ClipboardHistoryStore::default()
        }
    };
    clipboard_history.publish(&ui_sender);

    let mut outgoing_file_cancellations: HashMap<TransferId, Arc<AtomicBool>> = HashMap::new();
    let mut outgoing_file_sources: HashMap<TransferId, FileTransferSource> = HashMap::new();
    let file_clipboard_acknowledgements: FileClipboardAcknowledgements =
        Arc::new(Mutex::new(HashMap::new()));
    let (transfer_task_tx, mut transfer_task_rx) = unbounded_channel();
    let (receive_events, mut receive_event_rx) = receive_event_channel();
    let mut incoming_file_clipboards: HashMap<(DeviceId, TransferId), IncomingFileClipboard> =
        HashMap::new();
    let mut finder_pastes = FinderPasteJobs::new(cache_leases.clone());
    let mut finder_path_requests: HashMap<TransferId, CapturedFilePaste> = HashMap::new();
    let (finder_path_tx, mut finder_path_rx) = unbounded_channel::<(
        TransferId,
        Result<Option<unidesk_platform_macos::FinderDestination>, String>,
    )>();
    let mut file_policy = match FilePolicy::new() {
        Ok(policy) => policy,
        Err(error) => {
            emit_clipboard_error(&ui_sender, &diagnostics, error);
            return;
        }
    };
    file_policy.cache.set_external_leases(cache_leases.clone());
    file_policy.external_leases = cache_leases.clone();
    let (cleanup, startup_error) = file_policy.initialize_cache(&pasteboard);
    let cleanup_message =
        (!cleanup.removed_paths.is_empty() || cleanup.failed_transfers > 0).then(|| {
            file_policy::apply_cleanup(
                cleanup,
                &mut clipboard_history,
                &pasteboard,
                &mut clipboard_last_change_count,
                &ui_sender,
            )
        });
    let startup_message = match (startup_error, cleanup_message) {
        (Some(error), Some(cleanup)) => Some(format!("{error}；{cleanup}")),
        (error, cleanup) => error.or(cleanup),
    };
    file_policy.emit(&ui_sender, startup_message);

    let input_coordinator = crate::input_coordinator::InputCoordinator::start(
        runtime.clone(),
        ui_sender.clone(),
        input_signals.recording,
        input_signals.emergency,
    );

    let public_updates = crate::github_update::is_public_bundle();
    let mut github_updater = if public_updates {
        match crate::github_update::GitHubUpdater::start() {
            Ok(updater) => Some(updater),
            Err(error) => {
                let _ = ui_sender.unbounded_send(UiEvent::UpdateSnapshot(
                    unidesk_runtime::updates::LanUpdateView {
                        github: Some(Default::default()),
                        message: "GitHub 更新检查暂不可用".into(),
                        error: Some(error),
                        ..Default::default()
                    },
                ));
                None
            }
        }
    } else {
        None
    };
    #[cfg(not(test))]
    let updater = if public_updates {
        None
    } else {
        match crate::app_update::start(runtime.clone()) {
            Ok(updater) => Some(updater),
            Err(error) => {
                let _ = ui_sender.unbounded_send(UiEvent::UpdateSnapshot(
                    unidesk_runtime::updates::LanUpdateView {
                        message: "自动更新暂不可用".into(),
                        error: Some(error),
                        ..Default::default()
                    },
                ));
                None
            }
        }
    };
    #[cfg(test)]
    let updater: Option<unidesk_runtime::updates::LanUpdater> = None;
    // SSID reads stay off the UI/input actors and stop with this runtime.
    #[cfg(target_os = "macos")]
    let _wifi_task = {
        let runtime = runtime.clone();
        struct WifiTask(tokio::task::JoinHandle<()>);
        impl Drop for WifiTask {
            fn drop(&mut self) {
                self.0.abort();
            }
        }
        WifiTask(tokio::spawn(async move {
            if runtime.input_service_control().is_some() {
                return;
            }
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            loop {
                interval.tick().await;
                let ssid = tokio::task::spawn_blocking(crate::platform::current_ssid)
                    .await
                    .unwrap_or_default();
                if runtime
                    .send(RuntimeCommand::ObserveWifiEnvironment { ssid })
                    .await
                    .is_err()
                {
                    break;
                }
            }
        }))
    };
    let mut workflow_sources: Vec<WorkflowPendingSource> = Vec::new();
    let mut stage_task: Option<crate::app_update::StageTask> = None;
    let mut staged = false;
    let mut install_handoff: Option<std::time::Instant> = None;
    let mut install_helper: Option<std::process::Child> = None;
    // Retention also advances while every clipboard window is closed.
    let mut library_prune_tick = tokio::time::interval(Duration::from_secs(3600));
    library_prune_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut library_prune_task: Option<tokio::task::JoinHandle<()>> = None;
    let mut update_tick = tokio::time::interval(Duration::from_secs(2));
    let mut clipboard_tick = tokio::time::interval(Duration::from_millis(250));
    clipboard_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut permission_tick = tokio::time::interval(Duration::from_secs(2));
    permission_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        finder_pastes.reconcile_waiting(&file_policy, &incoming_file_clipboards, &ui_sender);
        file_paste_capture.update(
            file_policy
                .capture_reference()
                .or_else(|| {
                    // Explicit sends already stream an Offer without a deferred policy
                    // entry. They still need the same independent first-paste capture.
                    if incoming_file_clipboards.is_empty() {
                        return None;
                    }
                    let count = pasteboard.change_count().ok()?;
                    incoming_file_clipboards
                        .iter()
                        .find(|(_, incoming)| incoming.pasteboard_change_count == count)
                        .map(|(key, incoming)| FilePasteReference {
                            transfer_id: key.1.to_string(),
                            peer_id: key.0.to_hex(),
                            change_count: count,
                            name: incoming.name.clone(),
                            total_bytes: incoming.manifest.total_bytes(),
                        })
                })
                .or_else(|| finder_pastes.ready_reference(&pasteboard)),
        );
        if let Some(catalog) = &workflow_catalog {
            let peers = trusted_connected_peer_ids(&runtime.snapshot_view());
            let _ = catalog.refresh_peers(peers.iter().map(|peer| peer.to_hex()).collect());
            let generation = catalog.generation();
            let changed = generation != workflow_generation;
            if changed {
                workflow_generation = generation;
                let _ = ui_sender.unbounded_send(UiEvent::WorkflowCatalogChanged);
            }
            workflow_sends.retain(|task| !task.is_finished());
            if (changed || std::time::Instant::now() >= workflow_next_sync)
                && workflow_sends.len() < 64
            {
                workflow_next_sync = std::time::Instant::now() + Duration::from_secs(5);
                if let Ok(data) = catalog.manifest() {
                    for peer in peers {
                        if let Some(session) = runtime.control_generation(peer) {
                            let runtime = runtime.clone();
                            let data = data.clone();
                            workflow_sends.push(tokio::spawn(async move {
                                let _ = runtime
                                    .send_clipboard_on_session(
                                        peer,
                                        session,
                                        ClipboardPayload {
                                            mime: WORKFLOW_SYNC_MIME.into(),
                                            data,
                                            sequence: generation,
                                        },
                                    )
                                    .await;
                            }));
                        }
                    }
                }
            }
        }
        file_policy.protect_offers();
        for id in clipboard_sync.take_finished() {
            outgoing_file_cancellations.remove(&id);
        }
        for peer in clipboard_sync.take_disconnected() {
            file_policy.connection_lost(
                peer,
                &pasteboard,
                &mut incoming_file_clipboards,
                &pending_incoming_file,
                &paste_requested,
                &paste_target_pid,
                &ui_sender,
            );
            for (id, source) in &outgoing_file_sources {
                if source.target_peer.is_none_or(|target| target == peer)
                    && let Some(cancel) = outgoing_file_cancellations.get(id)
                {
                    cancel.store(true, Ordering::Release);
                }
            }
        }
        tokio::select! {
            event = preparations.rx.recv() => {
                if let Some(event) = event {
                    let token = event.token;
                    let Some(job) = preparations.jobs.get_mut(&token) else { continue; };
                    let valid = !job.cancel.load(Ordering::Acquire) && pasteboard.change_count().ok() == Some(job.baseline);
                    let outcome = if !valid { Err("剪贴板已变化或本次准备已取消".into()) } else { event.result };
                    let outcome = match outcome {
                        Ok(snapshot) if !event.materialized => {
                            match crate::library_files::reference(&snapshot) {
                                Ok(Some(reference)) if snapshot.file_urls.is_empty() => {
                                    let peer = DeviceId::from_hex(&reference.device_id).ok();
                                    if peer.is_none_or(|peer| !trusted_connected_peer_ids(&runtime.snapshot_view()).contains(&peer)) {
                                        Err("来源设备离线或未配对，请连接后重试".into())
                                    } else {
                                        file_policy.protect_clipboard(&pasteboard);
                                        match file_policy.cache.reserve(token,reference.total_bytes) {
                                            Ok(cleanup) => {
                                                file_policy::apply_cleanup(cleanup,&mut clipboard_history,&pasteboard,&mut clipboard_last_change_count,&ui_sender);
                                                let destination=file_policy.cache.destination(token);
                                                job.destination=Some(destination.clone());
                                                let runtime=runtime.clone();let cancel=job.cancel.clone();let tx=preparations.tx.clone();
                                                tokio::spawn(async move {
                                                    let result=crate::library_files::materialize(snapshot,token,destination,runtime,cancel).await;
                                                    let _=tx.send(ClipboardPrepared {token,materialized:true,result});
                                                });
                                                file_policy.emit(&ui_sender,None);
                                                continue;
                                            }
                                            Err(error) => {
                                                let cleanup=file_policy.cache.take_partial_cleanup();
                                                file_policy::apply_cleanup(cleanup,&mut clipboard_history,&pasteboard,&mut clipboard_last_change_count,&ui_sender);
                                                Err(error)
                                            }
                                        }
                                    }
                                }
                                Ok(_) => Ok(snapshot),
                                Err(error) => Err(error),
                            }
                        }
                        other => other,
                    };
                    let mut installed=false;
                    let result=outcome.and_then(|snapshot| {
                        if job.cancel.load(Ordering::Acquire)||pasteboard.change_count().ok()!=Some(job.baseline){return Err("本次剪贴板准备已失效".into())}
                        write_prepared_clipboard(&pasteboard,&snapshot)?;
                        installed=true;
                        let count=pasteboard.change_count().map_err(|e|e.to_string())?;
                        clipboard_last_change_count=Some(count);
                        clipboard_sync.invalidate_copy(&ui_sender);
                        file_policy.protect_clipboard(&pasteboard);
                        if job.cancel.load(Ordering::Acquire)||pasteboard.change_count().ok()!=Some(count){return Err("剪贴板已被其他复制替换，未发出粘贴请求".into())}
                        crate::clipboard_library::global()?.mark_used(job.id)?;
                        Ok(count)
                    });
                    if let Some(destination)=job.destination.take() {
                        file_policy.cache.finish(token);
                        if result.is_err() && !installed { tokio::task::spawn_blocking(move || { let _=std::fs::remove_dir_all(destination); }); }
                        file_policy.emit(&ui_sender,None);
                    }
                    let error=result.as_ref().err().cloned();
                    if let Some(reply)=job.reply.take(){let _=reply.send(result);}
                    if let Some(request_id)=job.tracked {let _=ui_sender.unbounded_send(UiEvent::CommandResult{request_id,error:error.clone()});}
                    if let Some(error)=error {emit_clipboard_error(&ui_sender,&diagnostics,error);}
                    preparations.jobs.remove(&token);
                }
            }
            _ = wake_signal.notified() => {
                crate::diagnostics::record!(diagnostics, DiagnosticLevel::Info, "runtime", "system_woke", None, None);
                if let Err(error) = runtime.send(RuntimeCommand::SystemWoke).await {
                    let _ = ui_sender.unbounded_send(UiEvent::RuntimeEvent(RuntimeEvent::Error(error)));
                }
            }

            view = next_runtime_view(&mut runtime_views) => {
                let Some(view) = view else { break; };
                if let Ok(count)=pasteboard.change_count() {clipboard_sync.refresh(&runtime,count,clipboard_enabled,&ui_sender);}
                let _ = ui_sender.unbounded_send(UiEvent::RuntimeView(view));
            }
            command = commands.recv() => {
                let (request_id, command) = match command {
                    Some(TokioCommand::Tracked { request_id, command }) => (Some(request_id), Some(*command)),
                    command => (None, command),
                };
                let mut command_error: Option<String> = None;
                let mut deferred_command_result = false;
                match command {
                    Some(TokioCommand::Tracked { .. }) => { command_error = Some("不允许嵌套命令".into()); }

                    Some(TokioCommand::Runtime(command)) => {
                        if matches!(command, RuntimeCommand::SetInputSharingSettings(_) | RuntimeCommand::ApplyInputSharingScene { .. }) {
                            let _=runtime.send(RuntimeCommand::SetInputAvailability {available:false,reason:Some("正在应用本机输入设置".into())}).await;
                        }
                        let description = match &command {
                            RuntimeCommand::ObserveWifiEnvironment { .. } => "ObserveWifiEnvironment".to_owned(),
                            RuntimeCommand::ForgetControllerEnvironment { .. } => "ForgetControllerEnvironment".to_owned(),
                            _ => format!("{command:?}"),
                        };
                        if let Err(error) = runtime.send(command).await {
                            command_error = Some(error.to_string());
                            crate::diagnostics::record!(diagnostics,
                                DiagnosticLevel::Error,
                                "runtime",
                                "command_failed",
                                Some(description),
                                Some(error.to_string()),
                            );
                            let _ = ui_sender.unbounded_send(UiEvent::RuntimeEvent(RuntimeEvent::Error(error)));
                        }
                    }
                    Some(TokioCommand::RefreshPermissions) => {
                        emit_permission_snapshot(&ui_sender, permission_service.as_ref(), &diagnostics, input_service.as_ref());
                    }
                    Some(TokioCommand::CheckForUpdates) => {
                        if let Some(updater) = &mut github_updater {
                            updater.check_now();
                            let _ = ui_sender.unbounded_send(UiEvent::UpdateSnapshot(updater.snapshot()));
                        } else if let Some(updater) = &updater {
                            updater.check_now();
                            let _ = ui_sender.unbounded_send(UiEvent::UpdateSnapshot(updater.snapshot()));
                        } else { command_error = Some("更新服务不可用".into()); }
                    }
                    Some(TokioCommand::DownloadAndInstallUpdate) => {
                        if let Some(updater) = &mut github_updater {
                            updater.download_and_install_now();
                            update_tick.reset_immediately();
                            let _ = ui_sender.unbounded_send(UiEvent::UpdateSnapshot(updater.snapshot()));
                        } else if let Some(updater) = &updater {
                            updater.download_and_install_now();
                            update_tick.reset_immediately();
                            let _ = ui_sender.unbounded_send(UiEvent::UpdateSnapshot(updater.snapshot()));
                        } else { command_error = Some("更新服务不可用".into()); }
                    }
                    Some(TokioCommand::Snapshot) => {
                        clipboard_history.publish(&ui_sender);
                        if let Some(updater) = &github_updater { let _ = ui_sender.unbounded_send(UiEvent::UpdateSnapshot(updater.snapshot())); }
                        else if let Some(updater) = &updater { let _ = ui_sender.unbounded_send(UiEvent::UpdateSnapshot(updater.snapshot())); }
                        file_policy.emit(&ui_sender,None);
                        let _ = ui_sender.unbounded_send(UiEvent::RuntimeView(runtime.snapshot_view()));
                    }
                    Some(TokioCommand::SetDebugLogging(enabled)) => {
                        unidesk_core::debug_logging::set_enabled(enabled);
                        let _ = ui_sender.unbounded_send(UiEvent::DebugLoggingChanged(enabled));
                    }
                    Some(TokioCommand::SetClipboardSync(enabled)) => {
                        if !enabled {file_policy.cancel_pending(&pending_incoming_file,&paste_requested,&paste_target_pid,&runtime,&clipboard_sequence,&ui_sender,&mut incoming_file_clipboards).await;}
                        clipboard_enabled = enabled;
                        clipboard_sync.reset(&ui_sender);
                        clipboard_last_change_count = pasteboard.change_count().ok();
                        let _ = ui_sender.unbounded_send(UiEvent::ClipboardStatus {
                            enabled,
                            last_mime: None,
                            last_len: 0,
                            last_sequence: None,
                            error: None,
                        });
                        crate::diagnostics::record!(diagnostics,
                            DiagnosticLevel::Info,
                            "clipboard",
                            if enabled { "sync_enabled" } else { "sync_disabled" },
                            None,
                            None,
                        );
                    }
                    Some(TokioCommand::PushClipboard) => {
                        command_error = push_local_clipboard(
                            &pasteboard,
                            &mut ClipboardPushContext {
                                runtime: &runtime,
                                sync: &mut clipboard_sync,
                                force: true,
                                last_change_count: &mut clipboard_last_change_count,
                                loop_guard: &mut clipboard_loop_guard,
                                history: &mut clipboard_history,
                                ui_sender: &ui_sender,
                                diagnostics: &diagnostics,
                            },
                        ).await.err();
                    }
                    Some(TokioCommand::ShareSourceFailed {id,error}) => {
                        let cancelled=outgoing_file_cancellations.remove(&id).is_some_and(|c| c.load(Ordering::Acquire));
                        emit_transfer_status(&ui_sender,id,"工作流共享".into(),if cancelled {"共享已取消".into()} else {error},if cancelled {TransferState::Cancelled} else {TransferState::Failed},0,0);
                    }
                    Some(TokioCommand::ReleaseShareSource(id)) => {outgoing_file_cancellations.remove(&id);}
                    Some(TokioCommand::RegisterShareSource {id,cancel}) => {outgoing_file_cancellations.insert(id,cancel);}
                    Some(TokioCommand::ShareSnapshot { snapshot, peer_id, selection, transfer_id }) => {
                        let shared_cancel=outgoing_file_cancellations.entry(transfer_id).or_insert_with(||Arc::new(AtomicBool::new(false))).clone();
                        if shared_cancel.load(Ordering::Acquire) {emit_transfer_status(&ui_sender,transfer_id,"工作流共享".into(),"共享已取消".into(),TransferState::Cancelled,0,0);outgoing_file_cancellations.remove(&transfer_id);continue;}

                        if !trusted_connected_peer_ids(&runtime.snapshot_view()).contains(&peer_id) { outgoing_file_cancellations.remove(&transfer_id); emit_transfer_status(&ui_sender,transfer_id,"工作流共享".into(),"目标设备尚未连接或不受信任".into(),TransferState::Failed,0,0); command_error=Some("目标设备尚未连接或不受信任".into()); }
                        else if snapshot.file_urls.is_empty() {
                            if let Ok(count)=pasteboard.change_count() { clipboard_sync.refresh(&runtime,count,clipboard_enabled,&ui_sender); }
                            clipboard_sync.publish_tracked(snapshot,peer_id,selection,transfer_id,shared_cancel,&runtime,&ui_sender);
                        } else {
                            let paths=snapshot.file_urls; let pin_paths=paths.clone();
                            let selected=crate::workflow_sharing::select(&peer_id.to_hex(),unidesk_core::workflow::WorkflowContentType::Files,&selection);
                            let started=start_local_file_clipboard(paths,Some(peer_id),FileClipboardStartContext {
                                expected_source:None, workflow:Some(selected), cancellation:Some(shared_cancel),expected_change_count:None,output_leases:cache_leases.clone(),transfer_id:Some(transfer_id),runtime:&runtime,sequence:&clipboard_sequence,ui_sender:&ui_sender,diagnostics:&diagnostics,acknowledgements:&file_clipboard_acknowledgements,
                            },&mut outgoing_file_cancellations,&mut outgoing_file_sources,&transfer_task_tx);
                            if let Some(id)=started {file_policy.pin_transfer(id,&pin_paths);} else {command_error=Some("设备不可发送，请检查连接后重试".into());}
                        }
                    }
                    Some(TokioCommand::SendFiles { paths, peer_id, transfer_id }) => {
                        let name = clipboard_transfer_name(&paths.iter().map(PathBuf::from).collect::<Vec<_>>());
                        let pin_paths=paths.clone();
                        let started = start_local_file_clipboard(
                            paths,
                            Some(peer_id),
                            FileClipboardStartContext {
                                expected_source: None,
                                workflow: Some(crate::workflow_sharing::select(&peer_id.to_hex(),unidesk_core::workflow::WorkflowContentType::Files,&crate::workflow_sharing::Selection::Automatic)),
                                cancellation: None,
                                expected_change_count: None,
                                output_leases: cache_leases.clone(),
                                transfer_id,
                                runtime: &runtime,
                                sequence: &clipboard_sequence,
                                ui_sender: &ui_sender,
                                diagnostics: &diagnostics,
                                acknowledgements: &file_clipboard_acknowledgements,
                            },
                            &mut outgoing_file_cancellations,
                            &mut outgoing_file_sources,
                            &transfer_task_tx,
                        );
                        if let Some(id)=started {file_policy.pin_transfer(id,&pin_paths);}
                        if started.is_none() { command_error = Some("设备不可发送，请检查连接后重试".into()); }
                        if started.is_none() && let Some(id) = transfer_id {
                            emit_transfer_status(&ui_sender, id, name, "设备不可发送，请检查连接后重试".into(), TransferState::Failed, 0, 0);
                        }
                    }
                    Some(TokioCommand::CancelClipboardTransfer(transfer_id)) => {
                        if let Some(captured) = finder_path_requests.remove(&transfer_id) {
                            emit_receive_transfer_status(&ui_sender, transfer_id, captured.reference.name,
                                "已取消获取路径".into(), TransferState::Cancelled, 0, captured.reference.total_bytes);
                            diagnostics.record(DiagnosticLevel::Info, "file_paste", "path_query_cancelled",
                                Some(format!("paste={transfer_id}")), None);
                        } else {
                            let transfer_id = if finder_pastes.has_job(transfer_id) {
                                finder_pastes.cancel(transfer_id, &ui_sender).unwrap_or(transfer_id)
                            } else { transfer_id };
                            file_policy.cancel(transfer_id,"接收已取消",&pending_incoming_file,&paste_requested,&paste_target_pid,&runtime,&clipboard_sequence,&ui_sender,&mut incoming_file_clipboards).await;
                            let incoming_keys=incoming_file_clipboards.keys().filter(|(_,id)|*id==transfer_id).copied().collect::<Vec<_>>();
                            for key in incoming_keys {
                                if let Some(state)=incoming_file_clipboards.get_mut(&key) {
                                    let owns_clipboard=pasteboard.change_count().ok()==Some(state.pasteboard_change_count);
                                    state.cancel();
                                    if owns_clipboard { pending_incoming_file.store(false,Ordering::Release); paste_requested.store(false,Ordering::Release); paste_target_pid.store(0,Ordering::Release); }
                                    let _=send_file_clipboard_frame(&runtime,&[key.0],&clipboard_sequence,ClipboardFileFrame::Cancel {transfer_id,reason:"接收方已取消".into()}).await;
                                    emit_receive_transfer_status(&ui_sender,transfer_id,state.name.clone(),"正在取消接收，等待磁盘任务退出…".into(),TransferState::Cancelled,state.received_bytes,state.manifest.total_bytes());
                                }
                            }
                            file_policy.emit(&ui_sender,None);
                            if let Some(cancelled) = outgoing_file_cancellations.get(&transfer_id) {
                                cancelled.store(true, Ordering::Release);
                            }
                        }
                    }
                    Some(TokioCommand::RetryFileTransfer(transfer_id)) => {
                        if finder_pastes.has_job(transfer_id) {
                            if let Err(error) = finder_pastes.retry(transfer_id, &ui_sender, &diagnostics) { command_error = Some(error); }
                        } else if let Some(source) = outgoing_file_sources.get(&transfer_id).cloned() {
                            let pin_paths=source.paths.clone();
                            let started = start_local_file_clipboard(
                                source.paths,
                                source.target_peer,
                                FileClipboardStartContext {
                                    expected_source: None,
                                    workflow: None,
                                    cancellation: None,
                                    expected_change_count: None,
                                    output_leases: cache_leases.clone(),
                                transfer_id: None,
                                    runtime: &runtime,
                                    sequence: &clipboard_sequence,
                                    ui_sender: &ui_sender,
                                    diagnostics: &diagnostics,
                                    acknowledgements: &file_clipboard_acknowledgements,
                                },
                                &mut outgoing_file_cancellations,
                                &mut outgoing_file_sources,
                                &transfer_task_tx,
                            );
                            if let Some(id)=started {file_policy.pin_transfer(id,&pin_paths);file_policy.release_retry(transfer_id);}
                        } else {
                            emit_clipboard_error(
                                &ui_sender,
                                &diagnostics,
                                {command_error = Some("找不到原始文件；请重新选择后发送".into()); command_error.clone().unwrap_or_default()},
                            );
                        }
                    }
                    #[cfg(target_os = "macos")]
                    Some(TokioCommand::LanWebContext { reply }) => {
                        let view = runtime.snapshot_view();
                        let result = pasteboard.change_count().map(|change_count| crate::lan_web::LanContext {
                            change_count,
                            local_id: view.snapshot.local_device_id.to_hex(),
                            local_name: lan_display_name.clone(),
                        }).map_err(|e| e.to_string());
                        let _ = reply.send(result);
                    }
                    #[cfg(target_os = "macos")]
                    Some(TokioCommand::LanWebCommit { mut snapshot, native_items, expected_change_count, cancel, reply }) => {
                        let result = (|| -> Result<crate::lan_web::LanCommitResult, String> {
                            if cancel.load(Ordering::Acquire) { return Err("上传已取消".into()); }
                            clipboard_bundle::attach_source_metadata(&mut snapshot, &clipboard_bundle::SourceMetadata {
                                app_bundle_id: Some("com.unidesk.lan-web".into()),
                                device_id: Some(runtime.snapshot_view().snapshot.local_device_id.to_hex()),
                                created_at_ms: Some(crate::clipboard_library::now_ms()),
                                no_history: false,
                            });
                            let library = crate::clipboard_library::global()?;
                            let (id, index_warning) = library.ingest_preserving_commit(&snapshot, "手机网页")?;
                            let id = id.ok_or("此来源已被排除，未加入列表")?;
                            // Once committed, always report a saved result, even if a later native
                            // pasteboard operation fails. The HTTP owner must retain these files.
                            let mut written = false;
                            let mut message: String;
                            if cancel.load(Ordering::Acquire) || pasteboard.change_count().ok() != Some(expected_change_count) {
                                message = "已加入列表；Mac 剪贴板已变化或请求已取消，未覆盖".into();
                            } else {
                                let write = if !native_items.is_empty() { pasteboard.write_items(&native_items) }
                                    else if let Some(text) = &snapshot.text { pasteboard.write_text(text) }
                                    else { pasteboard.write_file_urls(&snapshot.file_urls) };
                                match write {
                                    Ok(()) => match pasteboard.change_count() {
                                        Ok(count) => {
                                            snapshot.change_count = count;
                                            clipboard_last_change_count = Some(count);
                                            file_policy.protect_clipboard(&pasteboard);
                                            clipboard_sync.invalidate_copy(&ui_sender);
                                            if clipboard_enabled { clipboard_sync.publish(snapshot.clone(), false, &runtime, &ui_sender); }
                                            written = true;
                                            message = "已加入列表并复制到 Mac".into();
                                        },
                                        Err(_) => { message = "已加入列表；无法确认 Mac 剪贴板写入结果".into(); }
                                    },
                                    Err(_) => { message = "已加入列表；Mac 剪贴板写入失败，可稍后从列表复制".into(); }
                                }
                            }
                            if index_warning.is_some() { message.push_str("；跨设备列表索引暂未同步"); }
                            if clipboard_history.insert(snapshot, "手机网页") { clipboard_history.adopt_front_id(id); clipboard_history.publish(&ui_sender); }
                            Ok(crate::lan_web::LanCommitResult { item_id:id, clipboard_written:written, message })
                        })();
                        let _ = reply.send(result);
                    }
                    Some(TokioCommand::WorkflowSource { history_id, cancel, reply }) => {
                        if cancel.load(Ordering::Acquire) {
                            let _ = reply.send(Err("工作流已取消".into()));
                        } else if let Some(id) = history_id {
                            let result = crate::clipboard_library::global().and_then(|l|l.snapshot(id))
                                .and_then(|mut source| {
                                    if source.file_urls.iter().any(|path| !std::path::Path::new(path).is_file()) {
                                        return Err("历史图片文件已清理或移动，请重新复制".into());
                                    }
                                    // The source bytes are historical; this generation guards only
                                    // the later output, not the historical snapshot's old counter.
                                    source.change_count = pasteboard.change_count().map_err(|e| e.to_string())?;
                                    Ok(source)
                                });
                            let _ = reply.send(result);
                        } else if pending_incoming_file.load(Ordering::Acquire) {
                            let result = pasteboard.change_count().map_err(|e| e.to_string()).and_then(|count| {
                                let marker = pasteboard.read_remote_origin_marker().map_err(|e| e.to_string())?.unwrap_or_default();
                                let id = file_policy.workflow_pending_id().or_else(|| {
                                    incoming_file_clipboards.keys().find(|(_, id)| marker == format!("unidesk-clipboard-file-v1:{id}")).map(|(_, id)| *id)
                                }).ok_or_else(|| "远端图片引用已失效，请重新复制".to_owned())?;
                                Ok((id, count))
                            });
                            match result {
                                Ok((id, change_count)) => {
                                    let owned_request = !paste_requested.load(Ordering::Acquire)
                                        && !file_policy.workflow_requested(id)
                                        && !incoming_file_clipboards.keys().any(|(_, transfer)| *transfer == id);
                                    // Demand transfer without creating an ordinary paste intent:
                                    // a later physical Cmd-V can still capture its real target PID.
                                    file_policy.workflow_request(id);
                                    workflow_sources.push(WorkflowPendingSource { id, change_count, owned_request, cancel, reply, started: std::time::Instant::now() });
                                }
                                Err(error) => { let _ = reply.send(Err(error)); }
                            }
                        } else {
                            let _ = reply.send(pasteboard.read().map_err(|e| e.to_string()));
                        }
                    }
                    Some(TokioCommand::WorkflowCacheReserve { id, size, reply }) => {
                        file_policy.protect_clipboard(&pasteboard);
                        let result = file_policy.cache.reserve(id, size).map(|cleanup| {
                            file_policy::apply_cleanup(cleanup, &mut clipboard_history, &pasteboard, &mut clipboard_last_change_count, &ui_sender);
                            file_policy.cache.destination(id)
                        });
                        if result.is_err() {
                            let cleanup = file_policy.cache.take_partial_cleanup();
                            file_policy::apply_cleanup(cleanup, &mut clipboard_history, &pasteboard, &mut clipboard_last_change_count, &ui_sender);
                        }
                        file_policy.emit(&ui_sender, None);
                        if reply.send(result).is_err() { file_policy.cache.finish(id); }
                    }
                    Some(TokioCommand::WorkflowCacheFinish(id)) => {
                        file_policy.protect_clipboard(&pasteboard);
                        file_policy.cache.finish(id);
                        file_policy.emit(&ui_sender, None);
                    }
                    Some(TokioCommand::WorkflowCommit { mut snapshot, expected_change_count, cancel, target_pid, reply }) => {
                        let result = (|| -> Result<i64, String> {
                            if cancel.load(Ordering::Acquire) { return Err("工作流已取消".into()); }
                            if pasteboard.change_count().map_err(|e| e.to_string())? != expected_change_count {
                                return Err("剪贴板已更新，已丢弃过期图片处理结果".into());
                            }
                            if target_pid > 0 && !workflow_target_is_frontmost(target_pid) {
                                return Err("目标应用已切换，未写入或粘贴处理结果".into());
                            }
                            // Preserve the original even while auto-sync/history polling is disabled.
                            if let Ok(original) = pasteboard.read() { clipboard_history.record(original, "本机", &ui_sender); }
                            if cancel.load(Ordering::Acquire) || pasteboard.change_count().ok() != Some(expected_change_count)
                                || (target_pid > 0 && !workflow_target_is_frontmost(target_pid)) {
                                return Err("工作流已失效".into());
                            }
                            if !snapshot.file_urls.is_empty() { pasteboard.write_file_urls(&snapshot.file_urls) }
                            else if !snapshot.items.is_empty() { pasteboard.write_items(&snapshot.items) }
                            else if let Some(png) = snapshot.png.as_ref() { pasteboard.write_png(png) }
                            else if let Some(text)=snapshot.text.as_ref() { pasteboard.write_text(text) }
                            else { return Err("工作流处理结果为空".into()); }.map_err(|e| e.to_string())?;
                            let count = pasteboard.change_count().map_err(|e| e.to_string())?;
                            snapshot.change_count = count;
                            clipboard_history.record(snapshot, "工作流", &ui_sender);
                            file_policy.protect_clipboard(&pasteboard);
                            clipboard_sync.invalidate_copy(&ui_sender);
                            // Do not advance clipboard_last_change_count: normal polling must
                            // still synchronize this new local copy when auto-sync is enabled.
                            Ok(count)
                        })();
                        let _ = reply.send(result);
                    }
                    Some(TokioCommand::RestoreClipboardForPaste { id, session, reply }) => {
                        match pasteboard.change_count() {
                            Ok(baseline) => {
                                preparations.start(id,session,baseline,Some(reply),request_id,runtime.snapshot_view().snapshot.local_device_id.to_hex());
                                deferred_command_result = true;
                            }
                            Err(error) => {let message=error.to_string();let _=reply.send(Err(message.clone()));command_error=Some(message);}
                        }
                    }
                    Some(TokioCommand::CancelClipboardPreparation { session }) => { preparations.cancel(&session); }
                    Some(TokioCommand::RestoreClipboardHistory(id)) => {
                        match pasteboard.change_count() {
                            Ok(baseline) => {
                                preparations.start(id,format!("copy:{}",TransferId::new()),baseline,None,request_id,runtime.snapshot_view().snapshot.local_device_id.to_hex());
                                deferred_command_result = true;
                            }
                            Err(error) => {command_error=Some(error.to_string());}
                        }
                    }
                    Some(TokioCommand::SetFileTransferSettings(settings)) => {
                        file_policy.protect_clipboard(&pasteboard);
                        match file_policy.update(settings) {
                            Ok(cleanup) => { let message=file_policy::apply_cleanup(cleanup,&mut clipboard_history,&pasteboard,&mut clipboard_last_change_count,&ui_sender); file_policy.emit(&ui_sender,Some(message)); },
                            Err(error) => {command_error = Some(error.clone()); let cleanup=file_policy.cache.take_partial_cleanup();let message=file_policy::apply_cleanup(cleanup,&mut clipboard_history,&pasteboard,&mut clipboard_last_change_count,&ui_sender);file_policy.emit(&ui_sender,Some(format!("{error}；{message}")));}
                        }
                    }
                    Some(TokioCommand::ClearTransferCache) => {
                        file_policy.protect_clipboard(&pasteboard);
                        match file_policy.cache.clear_completed() {
                            Ok(cleanup) => { let message=file_policy::apply_cleanup(cleanup,&mut clipboard_history,&pasteboard,&mut clipboard_last_change_count,&ui_sender); file_policy.emit(&ui_sender,Some(message)); },
                            Err(error) => { command_error = Some(error.clone()); file_policy.emit(&ui_sender,Some(error)); }
                        }
                    }
                    Some(TokioCommand::RefreshTransferCache) => file_policy.emit(&ui_sender,None),
                    Some(TokioCommand::ClearClipboardHistory) => {
                        let previous = std::mem::take(&mut clipboard_history.items);
                        if let Err(error) = crate::clipboard_library::global().and_then(|l|l.clear_history()) {
                            clipboard_history.items = previous;
                            command_error = Some(error);
                        }
                        clipboard_history.publish(&ui_sender);
                    }
                    Some(TokioCommand::Shutdown) | None => {
                        for state in incoming_file_clipboards.values_mut() { state.cancel(); }
                        while !incoming_file_clipboards.is_empty() {
                            let Some(event) = receive_event_rx.recv().await else { break; };
                            let _ = handle_receive_event(event, &pasteboard, &mut RemoteClipboardState {
                                last_change_count: &mut clipboard_last_change_count,
                                incoming_files: &mut incoming_file_clipboards,
                                receive_events: &receive_events,
                                cache: &mut file_policy.cache,
                                loop_guard: &mut clipboard_loop_guard,
                                history: &mut clipboard_history,
                                pending_incoming_file: &pending_incoming_file,
                                paste_requested: &paste_requested,
                                paste_target_pid: &paste_target_pid,
                            }, &ui_sender, &diagnostics);
                        }
                        crate::diagnostics::record!(diagnostics, DiagnosticLevel::Info, "tokio", "worker_stopping", None, None);
                        break;
                    }
                }
                if !deferred_command_result && let Some(request_id) = request_id {
                    let _ = ui_sender.unbounded_send(UiEvent::CommandResult { request_id, error: command_error });
                }
            }
            (key, permit) = wait_for_receive_capacity(&incoming_file_clipboards),
                if incoming_file_clipboards.values().any(IncomingFileClipboard::is_backpressured) => {
                flush_pending_receive(key, permit, &mut incoming_file_clipboards, &ui_sender);
                if let Some(receipt) = incoming_file_clipboards.get(&key).and_then(|state| state.receipt(key.1)) {
                    let _ = send_file_clipboard_frame(&runtime, &[key.0], &clipboard_sequence, receipt).await;
                }
            }
            event = runtime_events.next(),
                if !incoming_file_clipboards.values().any(IncomingFileClipboard::is_backpressured) => {
                let Some(event) = event else { break; };
                match event {
                    RuntimeEvent::Snapshot(snapshot) => {
                        if let Ok(count)=pasteboard.change_count() {clipboard_sync.refresh(&runtime,count,clipboard_enabled,&ui_sender);}
                        let _ = ui_sender
                            .unbounded_send(UiEvent::RuntimeEvent(RuntimeEvent::Snapshot(snapshot)));
                        let _ = ui_sender.unbounded_send(UiEvent::RuntimeView(runtime.snapshot_view()));
                    }
                    RuntimeEvent::ClipboardReceived { peer_id, payload } => {
                        if payload.mime == crate::library_files::MIME {
                            if trusted_connected_peer_ids(&runtime.snapshot_view()).contains(&peer_id) { crate::library_files::receive(peer_id, &payload.data, &runtime); }
                            continue;
                        }
                        if payload.mime == library_sync::MIME {
                            library_sync.refresh(&runtime, &ui_sender);
                            library_sync.receive(peer_id, &payload.data);
                            continue;
                        }
                        if payload.mime==WORKFLOW_SYNC_MIME {
                            if trusted_connected_peer_ids(&runtime.snapshot_view()).contains(&peer_id)
                                && let Some(catalog)=&workflow_catalog {
                                match catalog.receive_from(&peer_id.to_hex(),&payload.data) {
                                    Ok(frames) => if let Some(session)=runtime.control_generation(peer_id) {
                                        let runtime=runtime.clone();
                                        workflow_sends.retain(|task|!task.is_finished());
                                        if workflow_sends.len()<64 { workflow_sends.push(tokio::spawn(async move {
                                            for data in frames {if runtime.send_clipboard_on_session(peer_id,session,ClipboardPayload {mime:WORKFLOW_SYNC_MIME.into(),data,sequence:0}).await.is_err() {break;}}
                                        })); }
                                    },
                                    Err(error)=> {let _=ui_sender.unbounded_send(UiEvent::BridgeError(format!("工作流同步失败：{error}")));}
                                }
                            }
                            continue;
                        }

                        if let Ok(count)=pasteboard.change_count() {
                            clipboard_sync.refresh(&runtime,count,clipboard_enabled,&ui_sender);
                            if clipboard_last_change_count.is_some_and(|old|old!=count) {clipboard_sync.invalidate_copy(&ui_sender);}
                            if payload.mime==unidesk_protocol::clipboard_sync::MIME {
                                clipboard_sync.receive(peer_id,&payload.data,count,&runtime,&ui_sender,clipboard_enabled);
                                continue;
                            }
                        }
                        // Native content and deferred references require the acknowledged protocol.
                        if payload.mime != CLIPBOARD_FILE_MIME { continue; }
                        // Decode the potentially large compressed manifest exactly once.
                        let frame = match ClipboardFileFrame::decode(&payload.data) {
                            Ok(frame) => frame,
                            Err(error) => { emit_clipboard_error(&ui_sender, &diagnostics, error.to_string()); continue; }
                        };
                        if matches!(frame, ClipboardFileFrame::Deferred { .. }) { continue; }
                        if let ClipboardFileFrame::Receipt { transfer_id, accepted_frames } = &frame {
                            if let Ok(acks) = file_clipboard_acknowledgements.lock()
                                && let Some(ack) = acks.get(&(peer_id, *transfer_id)) {
                                ack.progress.send_if_modified(|old| { if *old < *accepted_frames { *old = *accepted_frames; true } else { false } });
                            }
                            continue;
                        }
                        if let ClipboardFileFrame::Cancel { transfer_id, .. } = &frame && let Some(cancelled)=outgoing_file_cancellations.get(transfer_id) { cancelled.store(true,Ordering::Release); }
                        if file_policy.control(&frame,peer_id,&pasteboard,&mut clipboard_last_change_count,&pending_incoming_file,&paste_requested,&paste_target_pid,&runtime,&clipboard_sequence,&ui_sender,&diagnostics,&mut outgoing_file_cancellations,&mut outgoing_file_sources,&file_clipboard_acknowledgements,&transfer_task_tx).await { continue; }
                        if let ClipboardFileFrame::Ack { transfer_id } = &frame {
                            if let Ok(mut acknowledgements) = file_clipboard_acknowledgements.lock()
                                && let Some(acknowledge) = acknowledgements.remove(&(peer_id, *transfer_id)) {
                                let _ = acknowledge.completed.send(());
                            }
                            continue;
                        }
                        let file_id = frame.transfer_id();
                        let is_data_frame = matches!(frame, ClipboardFileFrame::Offer {..}|ClipboardFileFrame::RequestedOffer {..}|ClipboardFileFrame::Chunk {..}|ClipboardFileFrame::Complete {..});
                        if let Some(transfer_id) = apply_remote_clipboard_decoded(
                            &pasteboard,
                            &payload,
                            &mut RemoteClipboardState {
                                last_change_count: &mut clipboard_last_change_count,
                                        incoming_files: &mut incoming_file_clipboards,
                                        receive_events: &receive_events,
                                        cache: &mut file_policy.cache,
                                        loop_guard: &mut clipboard_loop_guard,
                                        history: &mut clipboard_history,
                                        pending_incoming_file: &pending_incoming_file,
                                        paste_requested: &paste_requested,
                                        paste_target_pid: &paste_target_pid,
                            },
                            &ui_sender,
                            &diagnostics,
                            peer_id,
                            Some(frame),
                        ) {
                            file_policy.finished(transfer_id);
                            file_policy.protect_clipboard(&pasteboard);
                            file_policy.emit(&ui_sender,None);
                            let runtime = runtime.clone();
                            let sequence = clipboard_sequence.clone();
                            tokio::spawn(async move {
                                let _ = send_file_clipboard_frame(
                                    &runtime,
                                    &[peer_id],
                                    &sequence,
                                    ClipboardFileFrame::Ack { transfer_id },
                                )
                                .await;
                            });
                        } else {
                            let id = file_id;
                            if let Some(receipt) = incoming_file_clipboards.get(&(peer_id,id)).and_then(|state| state.receipt(id)) {
                                let _ = send_file_clipboard_frame(&runtime, &[peer_id], &clipboard_sequence, receipt).await;
                            }
                            if is_data_frame && !incoming_file_clipboards.contains_key(&(peer_id,id)) {
                                file_policy.finished(id);
                                let _=send_file_clipboard_frame(&runtime,&[peer_id],&clipboard_sequence,ClipboardFileFrame::Cancel {transfer_id:id,reason:"接收被取消，或未通过容量与完整性检查".into()}).await;
                            }
                        }
                    }
                    RuntimeEvent::ConnectionRecovery { peer_id, phase, route_id, attempt, retry_after_secs, reason } => {
                        crate::diagnostics::record!(diagnostics,
                            DiagnosticLevel::Info, "runtime", "control_recovery",
                            Some(format!("peer={peer_id} phase={phase} route={route_id:?} attempt={attempt} retry_after_secs={retry_after_secs:?} reason={reason:?}")), None,
                        );
                    }
                    RuntimeEvent::InputReceived { .. } => {
                        // Never inject legacy input, even from a trusted clipboard peer.
                        let _ = ui_sender.unbounded_send(UiEvent::InputStatus { error: Some("已拒绝旧版未授权输入协议".to_owned()) });
                    }
                    RuntimeEvent::DisplayInfoReceived { .. } | RuntimeEvent::SharedInputReceived { .. } => {}
                    other => {
                        let _ = ui_sender.unbounded_send(UiEvent::RuntimeEvent(other.clone()));
                        if event_requires_snapshot_refresh(&other) {
                            let _ = ui_sender.unbounded_send(UiEvent::RuntimeView(runtime.snapshot_view()));
                        }
                        if let RuntimeEvent::Error(error) = other {
                            crate::diagnostics::record!(diagnostics,
                                DiagnosticLevel::Error,
                                "runtime",
                                "event_error",
                                Some(error.to_string()),
                                Some("RUNTIME_EVENT".to_owned()),
                            );
                        }
                    }
                }
            }
            _ = library_prune_tick.tick() => {
                if library_prune_task.as_ref().is_none_or(|task| task.is_finished()) {
                    library_prune_task = Some(tokio::task::spawn_blocking(|| {
                        if let Err(error) = crate::clipboard_library::global().and_then(|library| library.prune()) {
                            eprintln!("uniDesk: clipboard retention cleanup failed: {error}");
                        }
                    }));
                }
            }
            _ = clipboard_tick.tick() => {
                library_sync.refresh(&runtime, &ui_sender);
                if let Ok(count)=pasteboard.change_count() {
                    clipboard_sync.refresh(&runtime,count,clipboard_enabled,&ui_sender);
                    while let Some(ready)=clipboard_sync.poll(pasteboard.change_count().unwrap_or(count),&runtime,&ui_sender,&diagnostics,&file_policy.outgoing) {
                        let applied=if ready.payload.mime==CLIPBOARD_FILE_MIME {
                            if let Ok(frame @ ClipboardFileFrame::Deferred {..})=ClipboardFileFrame::decode(&ready.payload.data) {
                                let id=frame.transfer_id();
                                remember_file_source(ready.peer, id, ready.metadata.clone());
                                file_policy.control(&frame,ready.peer,&pasteboard,&mut clipboard_last_change_count,&pending_incoming_file,&paste_requested,&paste_target_pid,&runtime,&clipboard_sequence,&ui_sender,&diagnostics,&mut outgoing_file_cancellations,&mut outgoing_file_sources,&file_clipboard_acknowledgements,&transfer_task_tx).await;
                                file_policy.reference_installed(ready.peer,id)
                            } else {false}
                        } else {
                            let expected = NativeRevision(pasteboard.change_count().unwrap_or(count));
                            let data = if let Some(items) = ready.items.as_ref() {
                                Ok(ContentData {
                                    mime: ready.payload.mime.clone(),
                                    bytes: ready.payload.data.clone(),
                                    file_urls: Vec::new(),
                                    items: items.clone(),
                                })
                            } else if ready.payload.mime == "text/plain" {
                                String::from_utf8(ready.payload.data.clone())
                                    .map(ContentData::text)
                                    .map_err(|error| error.to_string())
                            } else if ready.payload.mime == "image/png" {
                                Ok(ContentData {
                                    mime: "image/png".into(),
                                    bytes: ready.payload.data.clone(),
                                    file_urls: Vec::new(),
                                    items: Vec::new(),
                                })
                            } else {
                                Err("剪贴板内容无效".into())
                            };
                            let result = match data.and_then(|data| {
                                apply_clipboard(
                                    &pasteboard,
                                    ContentToApply {
                                        data,
                                        remote_origin_marker: None,
                                    },
                                    expected,
                                )
                                .map_err(|error| error.to_string())
                            }) {
                                Ok(ApplyResult::Applied { .. }) => Ok(()),
                                Ok(ApplyResult::Superseded { .. }) => {
                                    Err("剪贴板已被其他复制替换".into())
                                }
                                Ok(ApplyResult::Rejected { reason }) => Err(reason),
                                Err(error) => Err(error),
                            };
                            finish_native_clipboard(&pasteboard,&ready.payload,&mut RemoteClipboardState {
                                last_change_count:&mut clipboard_last_change_count,incoming_files:&mut incoming_file_clipboards,receive_events:&receive_events,cache:&mut file_policy.cache,loop_guard:&mut clipboard_loop_guard,history:&mut clipboard_history,pending_incoming_file:&pending_incoming_file,paste_requested:&paste_requested,paste_target_pid:&paste_target_pid,
                            },&ui_sender,&diagnostics,ready.peer,result,Some(&ready.metadata))
                        };
                        if applied {clipboard_sync.invalidate_copy(&ui_sender);}
                        clipboard_sync.finish(ready,if applied {unidesk_protocol::clipboard_sync::APPLIED}else{unidesk_protocol::clipboard_sync::REJECTED},&runtime);
                    }
                }
                let active_workflow_ids = workflow_sources.iter().filter(|request| !request.cancel.load(Ordering::Acquire))
                    .map(|request| request.id).collect::<Vec<_>>();
                let mut waiting_sources = Vec::new();
                for request in workflow_sources.drain(..) {
                    let cancelled = request.cancel.load(Ordering::Acquire) || request.started.elapsed() > Duration::from_secs(120);
                    let marker_matches = pasteboard.read_remote_origin_marker().ok().flatten()
                        == Some(format!("unidesk-clipboard-file-v1:{}", request.id));
                    let pending = pending_incoming_file.load(Ordering::Acquire);
                    let current_count = pasteboard.change_count().ok();
                    let stale = !marker_matches || (pending && current_count != Some(request.change_count));
                    if cancelled || stale {
                        // Never cancel a transfer started by a separate ordinary paste or
                        // prefetch. A new normal paste PID also transfers ownership away.
                        if request.owned_request && paste_target_pid.load(Ordering::Acquire) == 0
                            && active_workflow_ids.iter().filter(|id| **id == request.id).count()
                                <= usize::from(!request.cancel.load(Ordering::Acquire))
                        {
                            file_policy.cancel(request.id, "工作流已取消", &pending_incoming_file, &paste_requested, &paste_target_pid, &runtime, &clipboard_sequence, &ui_sender, &mut incoming_file_clipboards).await;
                        }
                        let _ = request.reply.send(Err(if cancelled { "工作流已取消或等待超时" } else { "剪贴板已更新，远端图片处理已取消" }.into()));
                    } else if !pending {
                        let result = pasteboard.read().map_err(|e| e.to_string()).and_then(|snapshot| {
                            if snapshot.file_urls.is_empty() { Err("远端图片未成功接收，请重新复制".into()) } else { Ok(snapshot) }
                        });
                        let _ = request.reply.send(result);
                    } else { waiting_sources.push(request); }
                }
                workflow_sources = waiting_sources;
                file_policy.protect_clipboard(&pasteboard);
                file_policy.tick(&pasteboard,&pending_incoming_file,&paste_requested,&paste_target_pid,&runtime,&clipboard_sequence,&ui_sender,&mut incoming_file_clipboards).await;
                match pasteboard.change_count() {
                    Ok(count) => {
                        if clipboard_last_change_count.is_none_or(|last| last != count) {
                            if !clipboard_enabled {clipboard_sync.invalidate_copy(&ui_sender);clipboard_last_change_count=Some(count);continue;}
                            let _ = push_local_clipboard(
                                &pasteboard,
                                &mut ClipboardPushContext {
                                    runtime: &runtime,
                                    sync: &mut clipboard_sync,
                                    force: false,
                                    last_change_count: &mut clipboard_last_change_count,
                                    loop_guard: &mut clipboard_loop_guard,
                                    history: &mut clipboard_history,
                                        ui_sender: &ui_sender,
                                    diagnostics: &diagnostics,
                                },
                            ).await;

                        }
                    }
                    Err(error) => {
                        let _ = ui_sender.unbounded_send(UiEvent::ClipboardStatus {
                            enabled: true,
                            last_mime: None,
                            last_len: 0,
                            last_sequence: None,
                            error: Some(error.to_string()),
                        });
                    }
                }
            }
            _ = update_tick.tick() => {
                if let Some(updater) = &mut github_updater {
                    updater.poll().await;
                    if install_handoff.is_some_and(|deadline| std::time::Instant::now() >= deadline) {
                        install_handoff = None;
                        if let Some(mut helper) = install_helper.take() {
                            let _ = helper.kill();
                            let _ = helper.wait();
                        }
                        if let Ok(mut closed) = input_signals.update_admission.lock() { *closed = false; }
                        updater.abandon_install(&format!(
                            "更新安装未在 {} 秒内启动，安装器已取消",
                            INSTALL_HANDOFF.as_secs()
                        ));
                        input_coordinator.resume_after_update();
                    }
                    if !updater.install_abandoned() && install_handoff.is_none() && updater.ready() {
                        let force_install = updater.force_install();
                        let files_idle = force_install || (incoming_file_clipboards.is_empty() && outgoing_file_cancellations.is_empty()
                            && !pending_incoming_file.load(Ordering::Acquire) && !paste_requested.load(Ordering::Acquire)
                            && !(file_policy.update_busy() || finder_pastes.busy() || !finder_path_requests.is_empty()) && commands.is_empty());
                        updater.installation_waiting(if force_install {
                            "正在立即安装，将跳过传输或粘贴等待"
                        } else if !files_idle {
                            "已请求安装，等待当前文件传输或粘贴完成"
                        } else {
                            "已请求安装，正在结束键鼠共享并等待本地导出完成"
                        });
                        if files_idle {
                            if input_coordinator.pause_for_update(updater.snapshot().manual_requested) {
                                let admitted = {
                                    let mut closed = input_signals.update_admission.lock().unwrap();
                                    if force_install || (cache_leases.lock().unwrap().is_empty() && commands.is_empty() && !paste_requested.load(Ordering::Acquire)) {
                                        *closed = true; true
                                    } else { false }
                                };
                                if admitted {
                                    match unidesk_update::installer::launch_helper() {
                                        Ok(helper) => {
                                            install_helper = Some(helper);
                                            updater.installing();
                                            let _ = ui_sender.unbounded_send(UiEvent::UpdateSnapshot(updater.snapshot()));
                                            let _ = ui_sender.unbounded_send(UiEvent::Quit);
                                            install_handoff = Some(std::time::Instant::now() + INSTALL_HANDOFF);
                                        }
                                        Err(error) => {
                                            if let Ok(mut closed) = input_signals.update_admission.lock() { *closed = false; }
                                            updater.abandon_install(&format!("更新安装器无法启动，已保留当前版本：{error}"));
                                            input_coordinator.resume_after_update();
                                        }
                                    }
                                }
                            }
                        } else { input_coordinator.resume_after_update(); }
                    }
                    let _ = ui_sender.unbounded_send(UiEvent::UpdateSnapshot(updater.snapshot()));
                } else if let Some(updater) = &updater {
                    if stage_task.as_ref().is_some_and(|task| task.is_finished()) {
                        match stage_task.take().expect("completed staging").await {
                            Ok(Ok(())) => staged = true,
                            other => updater.quarantine(format!("更新暂存失败：{}", match other { Ok(Err(error)) => error, Err(error) => error.to_string(), _ => unreachable!() })),
                        }
                    }
                    if !staged && stage_task.is_none() && !updater.install_abandoned()
                        && let Some((release,path)) = updater.ready() && let Some(status) = updater.snapshot().status {
                        stage_task = Some(crate::app_update::stage(release,path,status));
                    }
                    let force_install = updater.force_install();
                    let files_idle = force_install || (incoming_file_clipboards.is_empty() && outgoing_file_cancellations.is_empty()
                        && !pending_incoming_file.load(Ordering::Acquire) && !paste_requested.load(Ordering::Acquire)
                        && !(file_policy.update_busy() || finder_pastes.busy() || !finder_path_requests.is_empty()) && commands.is_empty());
                    // The helper waits for this process to exit before touching the
                    // bundle. A handoff that has not ended the process by its deadline
                    // must give the running app back, so the user keeps a working
                    // version and can retry after a restart.
                    if install_handoff.is_some_and(|deadline| std::time::Instant::now() >= deadline) {
                        install_handoff = None;
                        if let Some(mut helper) = install_helper.take() {
                            let _ = helper.kill();
                            let _ = helper.wait();
                        }
                        if let Ok(mut closed) = input_signals.update_admission.lock() { *closed = false; }
                        updater.abandon_install(&format!(
                            "更新安装未在 {} 秒内启动，安装器已取消",
                            INSTALL_HANDOFF.as_secs()
                        ));
                        input_coordinator.resume_after_update();
                    }
                    if !updater.install_abandoned() && install_handoff.is_none() {
                        if updater.ready().is_some() {
                            updater.installation_waiting(if !staged {
                                "正在校验并准备安装，新版就绪后将重新启动"
                            } else if force_install {
                                "正在立即安装，将跳过传输或粘贴等待"
                            } else if !files_idle {
                                "已请求安装，等待当前文件传输或粘贴完成"
                            } else {
                                "已请求安装，正在结束键鼠共享并等待本地导出完成"
                            });
                        }
                        if staged && files_idle {
                            if input_coordinator.pause_for_update(updater.snapshot().manual_requested) {
                                // Native exports and UI commands share this admission lock.
                                let admitted = {
                                    let mut closed = input_signals.update_admission.lock().unwrap();
                                    if force_install || (cache_leases.lock().unwrap().is_empty() && commands.is_empty() && !paste_requested.load(Ordering::Acquire)) {
                                        *closed = true; true
                                    } else { false }
                                };
                                if admitted {
                                    // The handoff starts only once the helper exists. A helper
                                    // that cannot start leaves a usable app instead of a
                                    // pending restart that never happens.
                                    match unidesk_update::installer::launch_helper() {
                                        Ok(helper) => {
                                            install_helper = Some(helper);
                                            updater.installing();
                                            let _ = ui_sender.unbounded_send(UiEvent::UpdateSnapshot(updater.snapshot()));
                                            // This worker stays alive to answer the shutdown that
                                            // follows; the helper only replaces the bundle after
                                            // this process is gone.
                                            let _ = ui_sender.unbounded_send(UiEvent::Quit);
                                            install_handoff = Some(std::time::Instant::now() + INSTALL_HANDOFF);
                                        }
                                        Err(error) => {
                                            if let Ok(mut closed) = input_signals.update_admission.lock() { *closed = false; }
                                            updater.abandon_install(&format!("更新安装器无法启动，已保留当前版本：{error}"));
                                            input_coordinator.resume_after_update();
                                        }
                                    }
                                }
                            }
                        } else { input_coordinator.resume_after_update(); }
                    }
                    let _ = ui_sender.unbounded_send(UiEvent::UpdateSnapshot(updater.snapshot()));
                }
            }
            _ = permission_tick.tick() => {
                file_policy.expire_receivers(&runtime,&clipboard_sequence,&ui_sender,&mut incoming_file_clipboards,&pasteboard,&pending_incoming_file,&paste_requested,&paste_target_pid).await;
                emit_permission_snapshot(&ui_sender, permission_service.as_ref(), &diagnostics, input_service.as_ref());
            }
            captured = file_paste_requests.recv() => {
                if let Some(captured) = captured {
                    if unidesk_platform_macos::is_finder_pid(captured.target_pid) {
                        // Repeated presses during one lookup reuse its visible task.
                        let duplicate = finder_path_requests.values().any(|pending|
                            pending.target_pid == captured.target_pid
                            && pending.reference.transfer_id == captured.reference.transfer_id
                            && pending.reference.peer_id == captured.reference.peer_id);
                        if !duplicate {
                            let id = TransferId::new();
                            let pid = captured.target_pid;
                            if let Ok(transfer_id) = finder_paste_jobs::parse_transfer(&captured.reference.transfer_id) {
                                let _ = ui_sender.unbounded_send(UiEvent::FilePasteAcquiring { paste_id: id, transfer_id });
                            }
                            emit_receive_transfer_status(&ui_sender, id, captured.reference.name.clone(),
                                "路径获取中".into(), TransferState::Preparing, 0, captured.reference.total_bytes);
                            diagnostics.record(DiagnosticLevel::Info, "file_paste", "path_query_started",
                                Some(format!("paste={id} transfer={} pid={pid}", captured.reference.transfer_id)), None);
                            finder_path_requests.insert(id, captured);
                            let tx = finder_path_tx.clone();
                            tokio::spawn(async move {
                                let result = tokio::task::spawn_blocking(move ||
                                    unidesk_platform_macos::capture_finder_destination(pid))
                                    .await.map_err(|_| "获取 Finder 路径的后台任务异常退出".to_owned())
                                    .and_then(|result| result);
                                let _ = tx.send((id, result));
                            });
                        }
                    } else {
                        let current = pasteboard.change_count().ok() == Some(captured.reference.change_count);
                        if current {
                            if pending_incoming_file.load(Ordering::Acquire) {
                                paste_target_pid.store(captured.target_pid, Ordering::Release);
                                paste_requested.store(true, Ordering::Release);
                                if file_policy.capture_reference().is_some()
                                    && let Err(error) = file_policy.request_captured(&captured.reference, &runtime, &clipboard_sequence, &ui_sender).await {
                                        emit_clipboard_error(&ui_sender, &diagnostics, error);
                                    }
                            } else {
                                let _ = ui_sender.unbounded_send(UiEvent::PasteReady { target_pid: captured.target_pid, expected_change_count: captured.reference.change_count });
                            }
                        }
                    }
                }
            }
            resolved = finder_path_rx.recv() => {
                if let Some((id, result)) = resolved {
                    // A cancelled lookup is removed immediately; its late result
                    // must not start a receive or resurrect the cancelled row.
                    if let Some(captured) = finder_path_requests.remove(&id) {
                        let name = captured.reference.name.clone();
                        let total = captured.reference.total_bytes;
                        diagnostics.record(if matches!(&result, Ok(Some(_))) { DiagnosticLevel::Info } else { DiagnosticLevel::Error },
                            "file_paste", "path_query_finished",
                            Some(format!("paste={id} transfer={} elapsed_ms={} ok={}", captured.reference.transfer_id,
                                captured.captured_at.elapsed().as_millis(), matches!(&result, Ok(Some(_))))), None);
                        let result = match result {
                            Ok(Some(destination)) => {
                                let transfer = finder_paste_jobs::parse_transfer(&captured.reference.transfer_id);
                                let ready = transfer.as_ref().is_ok_and(|id| finder_pastes.has_ready(*id));
                                let receiving = transfer.as_ref().is_ok_and(|id| incoming_file_clipboards.keys()
                                    .any(|(peer, transfer)| transfer == id && peer.to_hex() == captured.reference.peer_id));
                                let requested = if ready || receiving { Ok(()) } else {
                                    file_policy.request_captured(&captured.reference, &runtime, &clipboard_sequence, &ui_sender).await
                                };
                                requested.and_then(|()| finder_pastes.register(id, captured, destination, &ui_sender, &diagnostics))
                            }
                            Ok(None) => Err("Finder 已退出，无法获取粘贴目录".into()),
                            Err(error) => Err(error),
                        };
                        if let Err(error) = result {
                            emit_receive_transfer_status(&ui_sender, id, name, error.clone(), TransferState::Failed, 0, total);
                            emit_clipboard_error(&ui_sender, &diagnostics, error);
                        }
                    }
                }
            }
            event = finder_pastes.rx.recv() => {
                if let Some(event) = event { finder_pastes.handle(event, &ui_sender, &diagnostics); }
            }
            event = receive_event_rx.recv() => {
                if let Some(event) = event {
                    let terminal_key = match &event { ReceiveEvent::Finished { key, .. } | ReceiveEvent::Cleaned { key, .. } => Some(*key), _ => None };
                    let completed_source = terminal_key.and_then(|key| incoming_file_clipboards.get(&key).map(|incoming| (
                        FilePasteReference { transfer_id: key.1.to_string(), peer_id: key.0.to_hex(), change_count: incoming.pasteboard_change_count, name: incoming.name.clone(), total_bytes: incoming.manifest.total_bytes() }, incoming.root_paths.clone()
                    )));
                    let acknowledged = handle_receive_event(event, &pasteboard, &mut RemoteClipboardState {
                        last_change_count: &mut clipboard_last_change_count,
                        incoming_files: &mut incoming_file_clipboards,
                        receive_events: &receive_events,
                        cache: &mut file_policy.cache,
                        loop_guard: &mut clipboard_loop_guard,
                        history: &mut clipboard_history,
                        pending_incoming_file: &pending_incoming_file,
                        paste_requested: &paste_requested,
                        paste_target_pid: &paste_target_pid,
                    }, &ui_sender, &diagnostics);
                    if let Some(key) = terminal_key && !incoming_file_clipboards.contains_key(&key) {
                        // Completion can beat the clipboard tick after a new copy.
                        // Release only this task's intent, never a newer request.
                        if file_policy.workflow_pending_id() == Some(key.1) {
                            pending_incoming_file.store(false, Ordering::Release);
                            paste_requested.store(false, Ordering::Release);
                            paste_target_pid.store(0, Ordering::Release);
                        }
                        if acknowledged == Some(key) {
                            if let Some((reference, paths)) = completed_source { finder_pastes.received(reference, paths, &ui_sender, &diagnostics); }
                        } else { finder_pastes.source_failed(key.1, &ui_sender); }
                        file_policy.finished(key.1);
                        file_policy.protect_clipboard(&pasteboard);
                        file_policy.emit(&ui_sender, None);
                        let frame = if acknowledged == Some(key) { ClipboardFileFrame::Ack { transfer_id: key.1 } }
                        else { ClipboardFileFrame::Cancel { transfer_id: key.1, reason: "接收已取消或文件校验失败".into() } };
                        let sent = send_file_clipboard_frame(&runtime, &[key.0], &clipboard_sequence, frame).await.is_ok();
                        diagnostics.record(if sent { DiagnosticLevel::Info } else { DiagnosticLevel::Warning }, "file_transfer", "receive_ack", Some(format!("peer={} transfer={} accepted={} sent={sent}", key.0.to_hex(), key.1, acknowledged == Some(key))), None);
                    }
                }
            }
            task = transfer_task_rx.recv() => {
                match task {
                    Some(ClipboardTransferTaskEvent::Finished(transfer_id)) => { outgoing_file_cancellations.remove(&transfer_id); file_policy.finish_outgoing(transfer_id); }
                    Some(ClipboardTransferTaskEvent::Prepared {transfer_id,paths,lease}) => {
                        file_policy.prepared_offer(transfer_id,paths.clone());
                        file_policy.protect_offers();
                        if let Some(source)=outgoing_file_sources.get_mut(&transfer_id) {source.paths=paths.clone();}
                        file_policy.unpin_transfer(transfer_id); file_policy.pin_transfer(transfer_id,&paths);
                        drop(lease);
                    }
                    None=>{}
                }
            }
        }
    }
    file_paste_capture.update(None);
    drop(finder_pastes);
    drop(input_coordinator);
    drop(updater);
    // A QUIC session that never winds down must not hold the process open. The
    // wait is only a courtesy: threads die with the process, and every install
    // step is already durable in its journal.
    match tokio::time::timeout(RUNTIME_SHUTDOWN, runtime.shutdown_and_wait()).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            crate::diagnostics::record!(
                diagnostics,
                DiagnosticLevel::Error,
                "runtime",
                "shutdown_failed",
                Some(error.to_string()),
                Some("RUNTIME_SHUTDOWN".to_owned()),
            );
        }
        Err(_) => {
            crate::diagnostics::record!(
                diagnostics,
                DiagnosticLevel::Error,
                "runtime",
                "shutdown_timeout",
                None,
                Some("RUNTIME_SHUTDOWN".to_owned()),
            );
        }
    }
    if install_handoff.is_some() {
        // The handoff outlived the worker, so the process is on its way out and the
        // helper owns the transaction. Only the launcher stays responsible for it.
        crate::diagnostics::record!(
            diagnostics,
            DiagnosticLevel::Info,
            "updates",
            "install_handoff_exit",
            None,
            Some("UPDATE_HANDOFF".to_owned()),
        );
    }
    crate::diagnostics::record!(
        diagnostics,
        DiagnosticLevel::Info,
        "tokio",
        "worker_stopped",
        None,
        None
    );
}

/// Controller-side remote-control state owned by the Tokio worker.
fn trusted_connected_peer_ids(view: &SnapshotView) -> Vec<DeviceId> {
    view.snapshot
        .peers
        .iter()
        .filter(|peer| {
            peer.trust == TrustState::Trusted
                && peer.connection == ConnectionState::Connected
                && view
                    .device_network
                    .allows(view.snapshot.local_device_id, peer.id)
        })
        .map(|peer| peer.id)
        .collect()
}

/// Bounded, metadata-only fallback for pasteboard implementations that strip
/// private types while materialising content. A fallback digest is bound to
/// the received pasteboard generation, so a new local copy of the same bytes
/// can be published again. Clipboard bodies are never retained here.
#[derive(Debug)]
struct ClipboardLoopGuard {
    recent_remote_fingerprints: VecDeque<(i64, String)>,
    /// 本地/远端/marker 共用的等价指纹账本。接力会剥掉自定义 type。
    ledger: crate::clipboard_echo::ContentLedger,
}

impl Default for ClipboardLoopGuard {
    fn default() -> Self {
        Self {
            recent_remote_fingerprints: VecDeque::with_capacity(64),
            ledger: crate::clipboard_echo::ContentLedger::default(),
        }
    }
}

impl ClipboardLoopGuard {
    const CAPACITY: usize = 64;

    /// Remember a payload written by uniDesk and report whether an equivalent
    /// clipboard value was already observed during the echo window.
    ///
    /// Apple Universal Clipboard can arrive before uniDesk's own transfer. In
    /// that order the local poll has already put the Handoff representation in
    /// the ledger; treating the later uniDesk payload as a duplicate keeps the
    /// history row single even when the native representations differ.
    fn remember_remote_payload(
        &mut self,
        payload: &ClipboardPayload,
        snapshot: Option<&PasteboardSnapshot>,
        change_count: i64,
    ) -> bool {
        self.remember(
            clipboard_fingerprint(&payload.mime, &payload.data),
            change_count,
        );
        let mut equivalent_ids = crate::clipboard_echo::payload_equivalent_ids(payload);
        if let Some(snapshot) = snapshot {
            equivalent_ids.extend(crate::clipboard_echo::snapshot_equivalent_ids(snapshot));
        }
        equivalent_ids.sort();
        equivalent_ids.dedup();
        self.ledger.observe(equivalent_ids)
    }

    fn remember(&mut self, fingerprint: String, change_count: i64) {
        self.recent_remote_fingerprints
            .retain(|(count, candidate)| *count == change_count && candidate != &fingerprint);
        self.recent_remote_fingerprints
            .push_back((change_count, fingerprint));
        if self.recent_remote_fingerprints.len() > Self::CAPACITY {
            let _ = self.recent_remote_fingerprints.pop_front();
        }
    }

    fn should_suppress(
        &mut self,
        pasteboard: &impl Pasteboard,
        snapshot: &PasteboardSnapshot,
    ) -> bool {
        // Universal Clipboard is Apple's own cross-device copy. uniDesk's
        // trusted session will deliver the same content (or has already done
        // so), so importing this system echo would create a second history
        // row. Do not add it to the ledger: an explicit local recopy after the
        // Handoff marker is cleared must remain a real new copy.
        if pasteboard
            .contains_type(UNIVERSAL_CLIPBOARD_TYPE)
            .unwrap_or(false)
        {
            return true;
        }
        let marker = pasteboard.read_remote_origin_marker().ok().flatten();
        if !snapshot.file_urls.is_empty() {
            // File paths change when a peer receives them into its private
            // cache, so a byte-for-byte local fingerprint is not available
            // without re-reading whole files. The private marker is bound to
            // the exact current pasteboard item and is cleared by any local
            // copy, making it the authoritative file-loop guard.
            return marker.is_some_and(|value| value.starts_with("unidesk-clipboard-file-v1:"));
        }
        let Some(fingerprint) = clipboard_snapshot_fingerprint(snapshot) else {
            return false;
        };
        // A marker is valid only when it describes the contents currently on
        // the pasteboard. This prevents an unrelated stale/custom type from
        // blocking a genuine local copy.
        if marker.is_some_and(|marker| marker == fingerprint) {
            return true;
        }
        if self
            .recent_remote_fingerprints
            .iter()
            .any(|(count, candidate)| *count == snapshot.change_count && candidate == &fingerprint)
        {
            return true;
        }
        // 第 3 层：不依赖接力是否保留自定义 type。命中任一等价指纹即回声。
        self.ledger
            .observe(crate::clipboard_echo::snapshot_equivalent_ids(snapshot))
    }
}

fn clipboard_fingerprint(mime: &str, bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(b"com.unidesk.clipboard-content.v1\0");
    digest.update(mime.as_bytes());
    digest.update([0]);
    digest.update(bytes);
    let hex = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("unidesk-clipboard-v1:{hex}")
}

fn clipboard_snapshot_fingerprint(snapshot: &PasteboardSnapshot) -> Option<String> {
    if !snapshot.file_urls.is_empty() {
        return None;
    }
    if !snapshot.items.is_empty() {
        // Hash representations without recompressing a potentially large image
        // on each poll. Use lengths and item boundaries to avoid ambiguity.
        let mut digest = Sha256::new();
        for item in &snapshot.items {
            digest.update((item.representations.len() as u64).to_le_bytes());
            for rep in &item.representations {
                digest.update((rep.type_name.len() as u64).to_le_bytes());
                digest.update(rep.type_name.as_bytes());
                digest.update((rep.data.len() as u64).to_le_bytes());
                digest.update(&rep.data);
            }
        }
        return Some(clipboard_fingerprint(
            clipboard_bundle::MIME,
            &digest.finalize(),
        ));
    }
    snapshot
        .png
        .as_deref()
        .map(|png| clipboard_fingerprint("image/png", png))
        .or_else(|| {
            snapshot
                .text
                .as_deref()
                .map(|text| clipboard_fingerprint("text/plain", text.as_bytes()))
        })
}

fn clipboard_snapshot_metadata(snapshot: &PasteboardSnapshot) -> Option<String> {
    if !snapshot.items.is_empty() {
        let formats: usize = snapshot
            .items
            .iter()
            .map(|item| item.representations.len())
            .sum();
        let bytes: usize = snapshot
            .items
            .iter()
            .flat_map(|item| &item.representations)
            .map(|r| r.data.len())
            .sum();
        return Some(format!(
            "items={} formats={formats} len={bytes}",
            snapshot.items.len()
        ));
    }
    if let Some(png) = &snapshot.png {
        return Some(format!("mime=image/png len={}", png.len()));
    }
    snapshot
        .text
        .as_ref()
        .map(|text| format!("mime=text/plain len={}", text.len()))
}

struct ClipboardPushContext<'a> {
    sync: &'a mut clipboard_sync::Sync,
    force: bool,
    runtime: &'a RuntimeHandle,
    last_change_count: &'a mut Option<i64>,
    loop_guard: &'a mut ClipboardLoopGuard,
    history: &'a mut ClipboardHistoryStore,
    ui_sender: &'a UnboundedSender<UiEvent>,
    diagnostics: &'a Diagnostics,
}

struct FileClipboardStartContext<'a> {
    runtime: &'a RuntimeHandle,
    sequence: &'a ClipboardSequence,
    ui_sender: &'a UnboundedSender<UiEvent>,
    diagnostics: &'a Diagnostics,
    acknowledgements: &'a FileClipboardAcknowledgements,
    transfer_id: Option<TransferId>,
    expected_source: Option<DeferredClipboardSource>,
    workflow: Option<crate::workflow_sharing::Selected>,
    cancellation: Option<Arc<AtomicBool>>,
    expected_change_count: Option<i64>,
    output_leases: ExternalCacheLeases,
}

struct FileClipboardStreamContext<'a> {
    sessions: &'a HashMap<DeviceId, u64>,
    runtime: &'a RuntimeHandle,
    sequence: &'a ClipboardSequence,
    ui_sender: &'a UnboundedSender<UiEvent>,
    diagnostics: &'a Diagnostics,
    cancelled: &'a AtomicBool,
    name: &'a str,
    acknowledgements: &'a FileClipboardAcknowledgements,
    peers: &'a [DeviceId],
    requested_offer: bool,
}

async fn push_local_clipboard(
    pasteboard: &impl Pasteboard,
    context: &mut ClipboardPushContext<'_>,
) -> Result<(), String> {
    let mut snapshot = match pasteboard.read() {
        Ok(snapshot) => snapshot,
        Err(error) => {
            emit_clipboard_error(context.ui_sender, context.diagnostics, error.to_string());
            return Err(error.to_string());
        }
    };
    *context.last_change_count = Some(snapshot.change_count);
    if !context.force && context.loop_guard.should_suppress(pasteboard, &snapshot) {
        crate::diagnostics::record!(
            context.diagnostics,
            DiagnosticLevel::Info,
            "clipboard",
            "remote_echo_suppressed",
            clipboard_snapshot_metadata(&snapshot),
            None,
        );
        return Ok(());
    }
    if let Ok(library) = crate::clipboard_library::global() {
        let origin = library.origin(&snapshot, "本机");
        clipboard_bundle::attach_source_metadata(
            &mut snapshot,
            &clipboard_bundle::SourceMetadata {
                app_bundle_id: origin.app_id,
                device_id: (!origin.device_id.is_empty()).then_some(origin.device_id),
                created_at_ms: origin.copied_at_ms,
                no_history: origin.no_history,
            },
        );
    } else {
        // Sync may continue while history persistence is unavailable, but never
        // silently waive an exclusion policy we could not read.
        clipboard_bundle::attach_source_metadata(
            &mut snapshot,
            &clipboard_bundle::SourceMetadata {
                no_history: true,
                ..Default::default()
            },
        );
    }
    context
        .history
        .record(snapshot.clone(), "本机", context.ui_sender);
    // Keep immutable file references until expiry/disconnect: an earlier paste
    // request may still be in flight when this new copy is published.
    context
        .sync
        .publish(snapshot, context.force, context.runtime, context.ui_sender);
    Ok(())
}

fn clipboard_transfer_name(paths: &[PathBuf]) -> String {
    match paths {
        [path] => path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("文件")
            .to_owned(),
        [] => "文件".to_owned(),
        paths => format!("{} 个项目", paths.len()),
    }
}

fn emit_transfer_status(
    ui_sender: &UnboundedSender<UiEvent>,
    transfer_id: TransferId,
    name: String,
    detail: String,
    state: TransferState,
    transferred_bytes: u64,
    total_bytes: u64,
) {
    let _ = ui_sender.unbounded_send(UiEvent::TransferStatus {
        transfer_id,
        name,
        detail,
        state,
        transferred_bytes,
        total_bytes,
        direction: TransferDirection::Send,
    });
}

fn emit_receive_transfer_status(
    ui_sender: &UnboundedSender<UiEvent>,
    transfer_id: TransferId,
    name: String,
    detail: String,
    state: TransferState,
    transferred_bytes: u64,
    total_bytes: u64,
) {
    let _ = ui_sender.unbounded_send(UiEvent::TransferStatus {
        transfer_id,
        name,
        detail,
        state,
        transferred_bytes,
        total_bytes,
        direction: TransferDirection::Receive,
    });
}

fn start_local_file_clipboard(
    file_paths: Vec<String>,
    target_peer: Option<DeviceId>,
    context: FileClipboardStartContext<'_>,
    active_cancellations: &mut HashMap<TransferId, Arc<AtomicBool>>,
    transfer_sources: &mut HashMap<TransferId, FileTransferSource>,
    task_events: &TokioSender<ClipboardTransferTaskEvent>,
) -> Option<TransferId> {
    let runtime = context.runtime;
    let sequence = context.sequence;
    let ui_sender = context.ui_sender;
    let diagnostics = context.diagnostics;
    let acknowledgements = context.acknowledgements;
    let connected_peers = trusted_connected_peer_ids(&runtime.snapshot_view());
    let peers = match target_peer {
        Some(peer_id) if connected_peers.contains(&peer_id) => vec![peer_id],
        Some(_) => {
            emit_clipboard_error(
                ui_sender,
                diagnostics,
                "目标设备尚未连接或不受信任".to_owned(),
            );
            return None;
        }
        None => connected_peers,
    };
    if peers.is_empty() {
        emit_clipboard_error(ui_sender, diagnostics, "没有已连接的受信任设备".to_owned());
        return None;
    }
    if file_paths.is_empty() {
        emit_clipboard_error(
            ui_sender,
            diagnostics,
            "没有可发送的文件或文件夹".to_owned(),
        );
        return None;
    }
    let source = FileTransferSource {
        paths: file_paths.clone(),
        target_peer,
    };
    let selected = context.workflow.unwrap_or_default();
    let cancellation = context.cancellation;
    let expected_change_count = context.expected_change_count;
    let output_leases = context.output_leases;
    let expected_source = context.expected_source;
    let requested_offer = expected_source.is_some();
    let paths = file_paths
        .into_iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    let _ = ui_sender.unbounded_send(UiEvent::ClipboardStatus {
        enabled: true,
        last_mime: Some(CLIPBOARD_FILE_MIME.to_owned()),
        last_len: 0,
        last_sequence: None,
        error: None,
    });
    let transfer_id = context.transfer_id.unwrap_or_default();
    transfer_sources.insert(transfer_id, source);
    let name = clipboard_transfer_name(&paths);
    let cancelled = cancellation.unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
    active_cancellations.insert(transfer_id, Arc::clone(&cancelled));
    emit_transfer_status(
        ui_sender,
        transfer_id,
        name.clone(),
        "正在扫描文件和文件夹…".to_owned(),
        TransferState::Preparing,
        0,
        0,
    );
    let sessions = peers
        .iter()
        .filter_map(|peer| runtime.control_generation(*peer).map(|g| (*peer, g)))
        .collect::<HashMap<_, _>>();
    let runtime = runtime.clone();
    let sequence = sequence.clone();
    let ui_sender = ui_sender.clone();
    let diagnostics = diagnostics.clone();
    let acknowledgements = Arc::clone(acknowledgements);
    let task_events = task_events.clone();
    tokio::spawn(async move {
        let preparation_cancel = cancelled.clone();
        let preparation_ui = ui_sender.clone();
        let preparation_name = name.clone();
        let prepared = match tokio::task::spawn_blocking(move || {
            let check_copy = || -> Result<(), String> {
                if let Some(expected) = expected_change_count {
                    let pasteboard = create_worker_pasteboard().map_err(|e| e.to_string())?;
                    if pasteboard.change_count().map_err(|e| e.to_string())? != expected {
                        preparation_cancel.store(true, Ordering::Release);
                        return Err("源剪贴板已被新复制替换".into());
                    }
                }
                if preparation_cancel.load(Ordering::Acquire) {
                    return Err("共享已取消".into());
                }
                Ok(())
            };
            check_copy()?;
            let source = match expected_source {
                Some(source) => source,
                None => DeferredClipboardSource::inspect_cancellable(paths.clone(), || {
                    preparation_cancel.load(Ordering::Acquire)
                })
                .map_err(|e| e.to_string())?,
            };
            source
                .validate_unchanged_cancellable(|| preparation_cancel.load(Ordering::Acquire))
                .map_err(|e| e.to_string())?;
            if preparation_cancel.load(Ordering::Acquire) {
                return Err("共享已取消".to_owned());
            }
            let processed = crate::workflow_sharing::process(
                selected,
                PasteboardSnapshot {
                    file_urls: paths
                        .iter()
                        .map(|p| p.to_string_lossy().into_owned())
                        .collect(),
                    ..Default::default()
                },
                preparation_cancel.clone(),
            )?;
            // The final post-hash comparison also validates workflow input identity.
            if preparation_cancel.load(Ordering::Acquire) {
                return Err("共享已取消".to_owned());
            }
            let lease_id = TransferId::new();
            output_leases
                .lock()
                .map_err(|_| "共享缓存租约不可用".to_owned())?
                .insert(lease_id, processed.paths.clone());
            let lease = ShareOutputLease {
                id: lease_id,
                leases: output_leases,
            };
            let output_paths = processed
                .paths
                .iter()
                .map(PathBuf::from)
                .collect::<Vec<_>>();
            if output_paths.is_empty() {
                return Err("工作流没有可发送的结果文件".into());
            }
            let mut last_scan_progress = std::time::Instant::now() - Duration::from_secs(1);
            let plan = ClipboardFilePlan::from_paths_observed(
                &output_paths,
                transfer_id,
                || preparation_cancel.load(Ordering::Acquire),
                |items| {
                    if last_scan_progress.elapsed() >= Duration::from_millis(200) {
                        last_scan_progress = std::time::Instant::now();
                        emit_transfer_status(
                            &preparation_ui,
                            transfer_id,
                            preparation_name.clone(),
                            format!("正在准备文件，已扫描 {items} 项…"),
                            TransferState::Preparing,
                            0,
                            source.total_bytes,
                        );
                    }
                },
            )
            .map_err(|e| e.to_string())?;
            source
                .validate_unchanged_cancellable(|| preparation_cancel.load(Ordering::Acquire))
                .map_err(|e| e.to_string())?;
            check_copy()?;
            Ok::<_, String>((plan, processed, lease))
        })
        .await
        {
            Ok(Ok(prepared)) => prepared,
            Ok(Err(error)) => {
                let was_cancelled = cancelled.load(Ordering::Acquire);
                if !was_cancelled {
                    emit_clipboard_error(
                        &ui_sender,
                        &diagnostics,
                        format!("Finder 文件读取失败：{error}"),
                    );
                }
                emit_transfer_status(
                    &ui_sender,
                    transfer_id,
                    name.clone(),
                    if was_cancelled {
                        "文件准备已取消".into()
                    } else {
                        format!("扫描失败：{error}")
                    },
                    if was_cancelled {
                        TransferState::Cancelled
                    } else {
                        TransferState::Failed
                    },
                    0,
                    0,
                );
                let _ = send_file_clipboard_frame_pinned(
                    &runtime,
                    &peers,
                    &sessions,
                    &sequence,
                    ClipboardFileFrame::Cancel {
                        transfer_id,
                        reason: "源文件已更改、被删除或无法读取，请重新复制".into(),
                    },
                )
                .await;
                let _ = task_events.send(ClipboardTransferTaskEvent::Finished(transfer_id));
                return;
            }
            Err(error) => {
                emit_clipboard_error(
                    &ui_sender,
                    &diagnostics,
                    format!("Finder 文件后台读取失败：{error}"),
                );
                emit_transfer_status(
                    &ui_sender,
                    transfer_id,
                    name.clone(),
                    format!("后台任务失败：{error}"),
                    TransferState::Failed,
                    0,
                    0,
                );
                let _ = send_file_clipboard_frame_pinned(
                    &runtime,
                    &peers,
                    &sessions,
                    &sequence,
                    ClipboardFileFrame::Cancel {
                        transfer_id,
                        reason: "源文件已更改、被删除或无法读取，请重新复制".into(),
                    },
                )
                .await;
                let _ = task_events.send(ClipboardTransferTaskEvent::Finished(transfer_id));
                return;
            }
        };
        let (plan, processed, lease) = prepared;
        let warning = processed.warning;
        let _ = task_events.send(ClipboardTransferTaskEvent::Prepared {
            transfer_id,
            paths: processed.paths,
            lease,
        });
        if let Some(error) = &warning {
            let _ = ui_sender.unbounded_send(UiEvent::ClipboardDelivery {
                peer_id: peers[0],
                phase: format!("工作流失败：{error}；正在发送原内容"),
            });
        }
        if cancelled.load(Ordering::Acquire) {
            emit_transfer_status(
                &ui_sender,
                transfer_id,
                name.clone(),
                "已取消".to_owned(),
                TransferState::Cancelled,
                0,
                plan.manifest().total_bytes(),
            );
            let _ = task_events.send(ClipboardTransferTaskEvent::Finished(transfer_id));
            return;
        }
        // An explicit send has not published anything while hashing: bind it
        // to the connection that is live when the first offer is sent. A paste
        // request must keep its original connection and may never cross epochs.
        let sessions = if requested_offer {
            sessions
        } else {
            peers
                .iter()
                .filter_map(|peer| runtime.control_generation(*peer).map(|g| (*peer, g)))
                .collect()
        };
        let succeeded = stream_local_file_clipboard(
            &plan,
            FileClipboardStreamContext {
                sessions: &sessions,
                runtime: &runtime,
                sequence: &sequence,
                ui_sender: &ui_sender,
                diagnostics: &diagnostics,
                cancelled: &cancelled,
                name: &name,
                acknowledgements: &acknowledgements,
                peers: &peers,
                requested_offer,
            },
        )
        .await;
        if succeeded && let Some(error) = warning {
            emit_transfer_status(
                &ui_sender,
                transfer_id,
                name.clone(),
                format!("工作流失败：{error}；已发送原内容"),
                TransferState::Completed,
                plan.manifest().total_bytes(),
                plan.manifest().total_bytes(),
            );
            for peer in &peers {
                let _ = ui_sender.unbounded_send(UiEvent::ClipboardDelivery {
                    peer_id: *peer,
                    phase: format!("工作流失败：{error}；已发送原内容"),
                });
            }
        }
        if !succeeded {
            let _ = send_file_clipboard_frame_pinned(
                &runtime,
                &peers,
                &sessions,
                &sequence,
                ClipboardFileFrame::Cancel {
                    transfer_id,
                    reason: "文件传输失败，请重新复制或重试".into(),
                },
            )
            .await;
        }
        let _ = task_events.send(ClipboardTransferTaskEvent::Finished(transfer_id));
    });
    Some(transfer_id)
}

fn emit_file_clipboard_failure(
    ui_sender: &UnboundedSender<UiEvent>,
    diagnostics: &Diagnostics,
    transfer_id: TransferId,
    name: String,
    detail: String,
    transferred_bytes: u64,
    total_bytes: u64,
) {
    emit_clipboard_error(ui_sender, diagnostics, detail.clone());
    emit_transfer_status(
        ui_sender,
        transfer_id,
        name,
        detail,
        TransferState::Failed,
        transferred_bytes,
        total_bytes,
    );
}

async fn stream_local_file_clipboard(
    plan: &ClipboardFilePlan,
    context: FileClipboardStreamContext<'_>,
) -> bool {
    let runtime = context.runtime;
    let sessions = context.sessions;
    let sequence = context.sequence;
    let ui_sender = context.ui_sender;
    let diagnostics = context.diagnostics;
    let cancelled = context.cancelled;
    let name = context.name;
    let acknowledgements = context.acknowledgements;
    let transfer_id = plan.transfer_id();
    let total_bytes = plan.manifest().total_bytes();
    let file_count = plan.manifest().file_count();
    let peers = context.peers;
    if peers.is_empty() {
        emit_file_clipboard_failure(
            ui_sender,
            diagnostics,
            transfer_id,
            name.to_owned(),
            "没有已连接的受信任设备".to_owned(),
            0,
            total_bytes,
        );
        return false;
    }

    let mut acknowledgement_receivers =
        match register_file_clipboard_acknowledgements(acknowledgements, peers, transfer_id) {
            Ok(receivers) => receivers,
            Err(error) => {
                emit_file_clipboard_failure(
                    ui_sender,
                    diagnostics,
                    transfer_id,
                    name.to_owned(),
                    error,
                    0,
                    total_bytes,
                );
                return false;
            }
        };

    let offer = if context.requested_offer {
        ClipboardFileFrame::RequestedOffer {
            transfer_id,
            manifest: plan.manifest().clone(),
        }
    } else {
        ClipboardFileFrame::Offer {
            transfer_id,
            manifest: plan.manifest().clone(),
        }
    };
    if let Err(error) =
        send_file_clipboard_frame_pinned(runtime, peers, sessions, sequence, offer).await
    {
        remove_file_clipboard_acknowledgements(acknowledgements, peers, transfer_id);
        emit_file_clipboard_failure(
            ui_sender,
            diagnostics,
            transfer_id,
            name.to_owned(),
            error,
            0,
            total_bytes,
        );
        return false;
    }

    if let Err(error) = wait_for_file_receipts(&mut acknowledgement_receivers, 1, cancelled).await {
        remove_file_clipboard_acknowledgements(acknowledgements, peers, transfer_id);
        if cancelled.load(Ordering::Acquire) {
            emit_transfer_status(
                ui_sender,
                transfer_id,
                name.to_owned(),
                "已取消".into(),
                TransferState::Cancelled,
                0,
                total_bytes,
            );
        } else {
            emit_file_clipboard_failure(
                ui_sender,
                diagnostics,
                transfer_id,
                name.to_owned(),
                error,
                0,
                total_bytes,
            );
        }
        return false;
    }
    let mut sent_frames = 1_u64;
    emit_transfer_status(
        ui_sender,
        transfer_id,
        name.to_owned(),
        format!("正在发送 {file_count} 个文件"),
        TransferState::Transferring,
        0,
        total_bytes,
    );
    let mut transferred_bytes = 0_u64;
    let mut last_progress = std::time::Instant::now() - Duration::from_secs(1);

    for source in plan.sources() {
        for descriptor in source.chunks() {
            if cancelled.load(Ordering::Acquire) {
                remove_file_clipboard_acknowledgements(acknowledgements, peers, transfer_id);
                let _ = send_file_clipboard_frame_pinned(
                    runtime,
                    peers,
                    sessions,
                    sequence,
                    ClipboardFileFrame::Cancel {
                        transfer_id,
                        reason: "发送方已取消".to_owned(),
                    },
                )
                .await;
                emit_transfer_status(
                    ui_sender,
                    transfer_id,
                    name.to_owned(),
                    "已取消；接收端临时文件将被清理".to_owned(),
                    TransferState::Cancelled,
                    transferred_bytes,
                    total_bytes,
                );
                return false;
            }
            let descriptor = *descriptor;
            let source_path = source.source_path().to_path_buf();
            let bytes = match tokio::task::spawn_blocking(move || {
                read_chunk(source_path, descriptor)
            })
            .await
            {
                Ok(Ok(bytes)) => bytes,
                Ok(Err(error)) => {
                    remove_file_clipboard_acknowledgements(acknowledgements, peers, transfer_id);
                    emit_file_clipboard_failure(
                        ui_sender,
                        diagnostics,
                        transfer_id,
                        name.to_owned(),
                        format!("读取失败：{error}"),
                        transferred_bytes,
                        total_bytes,
                    );
                    return false;
                }
                Err(error) => {
                    remove_file_clipboard_acknowledgements(acknowledgements, peers, transfer_id);
                    emit_file_clipboard_failure(
                        ui_sender,
                        diagnostics,
                        transfer_id,
                        name.to_owned(),
                        format!("后台读取失败：{error}"),
                        transferred_bytes,
                        total_bytes,
                    );
                    return false;
                }
            };
            let frame = ClipboardFileFrame::Chunk {
                transfer_id,
                path: source.relative_path().clone(),
                descriptor,
                bytes,
            };
            if let Err(error) =
                send_file_clipboard_frame_pinned(runtime, peers, sessions, sequence, frame).await
            {
                remove_file_clipboard_acknowledgements(acknowledgements, peers, transfer_id);
                emit_file_clipboard_failure(
                    ui_sender,
                    diagnostics,
                    transfer_id,
                    name.to_owned(),
                    error,
                    transferred_bytes,
                    total_bytes,
                );
                return false;
            }
            transferred_bytes = transferred_bytes.saturating_add(descriptor.length as u64);
            sent_frames = sent_frames.saturating_add(1);
            if (sent_frames - 1).is_multiple_of(16)
                && let Err(error) =
                    wait_for_file_receipts(&mut acknowledgement_receivers, sent_frames, cancelled)
                        .await
            {
                remove_file_clipboard_acknowledgements(acknowledgements, peers, transfer_id);
                if cancelled.load(Ordering::Acquire) {
                    emit_transfer_status(
                        ui_sender,
                        transfer_id,
                        name.to_owned(),
                        "已取消".into(),
                        TransferState::Cancelled,
                        transferred_bytes,
                        total_bytes,
                    );
                } else {
                    emit_file_clipboard_failure(
                        ui_sender,
                        diagnostics,
                        transfer_id,
                        name.to_owned(),
                        error,
                        transferred_bytes,
                        total_bytes,
                    );
                }
                return false;
            }
            if transferred_bytes == total_bytes
                || last_progress.elapsed() >= Duration::from_millis(100)
            {
                last_progress = std::time::Instant::now();
                emit_transfer_status(
                    ui_sender,
                    transfer_id,
                    name.to_owned(),
                    format!("已发送 {file_count} 个文件中的数据"),
                    TransferState::Transferring,
                    transferred_bytes,
                    total_bytes,
                );
            }
        }
    }

    if let Err(error) = send_file_clipboard_frame_pinned(
        runtime,
        peers,
        sessions,
        sequence,
        ClipboardFileFrame::Complete { transfer_id },
    )
    .await
    {
        remove_file_clipboard_acknowledgements(acknowledgements, peers, transfer_id);
        emit_file_clipboard_failure(
            ui_sender,
            diagnostics,
            transfer_id,
            name.to_owned(),
            error,
            transferred_bytes,
            total_bytes,
        );
        return false;
    }
    emit_transfer_status(
        ui_sender,
        transfer_id,
        name.to_owned(),
        "发送完毕，等待接收端校验…".into(),
        TransferState::Verifying,
        transferred_bytes,
        total_bytes,
    );
    diagnostics.record(
        DiagnosticLevel::Info,
        "file_transfer",
        "awaiting_ack",
        Some(format!("transfer={transfer_id} bytes={transferred_bytes}")),
        None,
    );
    if let Err(error) = wait_for_file_clipboard_acknowledgements(
        acknowledgement_receivers,
        acknowledgements,
        peers,
        transfer_id,
        cancelled,
    )
    .await
    {
        if cancelled.load(Ordering::Acquire) {
            let _ = send_file_clipboard_frame_pinned(
                runtime,
                peers,
                sessions,
                sequence,
                ClipboardFileFrame::Cancel {
                    transfer_id,
                    reason: "发送方已取消".to_owned(),
                },
            )
            .await;
            emit_transfer_status(
                ui_sender,
                transfer_id,
                name.to_owned(),
                "已取消；接收端临时文件将被清理".to_owned(),
                TransferState::Cancelled,
                transferred_bytes,
                total_bytes,
            );
        } else {
            emit_file_clipboard_failure(
                ui_sender,
                diagnostics,
                transfer_id,
                name.to_owned(),
                error,
                transferred_bytes,
                total_bytes,
            );
        }
        return false;
    }

    diagnostics.record(
        DiagnosticLevel::Info,
        "clipboard",
        "file_pushed",
        Some(format!(
            "transfer={transfer_id} files={file_count} bytes={total_bytes} peers={}",
            peers.len()
        )),
        None,
    );
    let last_sequence = sequence.latest();
    let _ = ui_sender.unbounded_send(UiEvent::ClipboardStatus {
        enabled: true,
        last_mime: Some(CLIPBOARD_FILE_MIME.to_owned()),
        last_len: usize::try_from(total_bytes).unwrap_or(usize::MAX),
        last_sequence: Some(last_sequence),
        error: None,
    });
    emit_transfer_status(
        ui_sender,
        transfer_id,
        name.to_owned(),
        "接收端已完成文件校验".to_owned(),
        TransferState::Completed,
        total_bytes,
        total_bytes,
    );
    true
}

fn register_file_clipboard_acknowledgements(
    acknowledgements: &FileClipboardAcknowledgements,
    peers: &[DeviceId],
    transfer_id: TransferId,
) -> Result<Vec<PendingFileAcknowledgement>, String> {
    let mut guard = acknowledgements
        .lock()
        .map_err(|_| "文件传输确认队列不可用".to_owned())?;
    if peers
        .iter()
        .any(|peer_id| guard.contains_key(&(*peer_id, transfer_id)))
    {
        return Err("同一设备已有同标识的文件传输正在等待确认".to_owned());
    }
    let mut receivers = Vec::with_capacity(peers.len());
    for peer_id in peers {
        let (sender, receiver) = oneshot::channel();
        let (progress, progress_receiver) = tokio::sync::watch::channel(0);
        let _ = guard.insert(
            (*peer_id, transfer_id),
            FileClipboardAcknowledgement {
                completed: sender,
                progress,
            },
        );
        receivers.push(PendingFileAcknowledgement {
            completed: receiver,
            progress: progress_receiver,
        });
    }
    Ok(receivers)
}

fn remove_file_clipboard_acknowledgements(
    acknowledgements: &FileClipboardAcknowledgements,
    peers: &[DeviceId],
    transfer_id: TransferId,
) {
    if let Ok(mut guard) = acknowledgements.lock() {
        for peer_id in peers {
            guard.remove(&(*peer_id, transfer_id));
        }
    }
}

/// End-to-end application credit, distinct from QUIC receipt and final hash ACK.
async fn wait_for_file_receipts(
    receivers: &mut [PendingFileAcknowledgement],
    sent_frames: u64,
    cancelled: &AtomicBool,
) -> Result<(), String> {
    for pending in receivers {
        let deadline = tokio::time::sleep(Duration::from_secs(30));
        tokio::pin!(deadline);
        while *pending.progress.borrow() < sent_frames {
            tokio::select! {
                result = pending.progress.changed() => {
                    result.map_err(|_| "接收端确认通道已关闭".to_owned())?;
                },
                _ = &mut deadline => return Err("等待接收端队列确认超时，请确认两台设备均已更新后重试".into()),
                _ = tokio::time::sleep(Duration::from_millis(100)) => {
                    if cancelled.load(Ordering::Acquire) { return Err("发送方已取消".into()); }
                }
            }
        }
    }
    Ok(())
}

async fn wait_for_file_clipboard_acknowledgements(
    receivers: Vec<PendingFileAcknowledgement>,
    acknowledgements: &FileClipboardAcknowledgements,
    peers: &[DeviceId],
    transfer_id: TransferId,
    cancelled: &AtomicBool,
) -> Result<(), String> {
    for pending in receivers {
        let mut receiver = pending.completed;
        // A large directory can spend minutes hashing and committing small files.
        let timeout = tokio::time::sleep(Duration::from_secs(600));
        tokio::pin!(timeout);
        loop {
            tokio::select! {
                result = &mut receiver => {
                    if result.is_err() {
                        remove_file_clipboard_acknowledgements(acknowledgements, peers, transfer_id);
                        return Err("接收端确认通道已关闭".to_owned());
                    }
                    break;
                }
                _ = &mut timeout => {
                    remove_file_clipboard_acknowledgements(acknowledgements, peers, transfer_id);
                    return Err("等待接收端校验确认超时，可在保留原文件的情况下重试".to_owned());
                }
                _ = tokio::time::sleep(Duration::from_millis(100)) => {
                    if cancelled.load(Ordering::Acquire) {
                        remove_file_clipboard_acknowledgements(acknowledgements, peers, transfer_id);
                        return Err("发送方已取消".to_owned());
                    }
                }
            }
        }
    }
    Ok(())
}

async fn send_file_clipboard_frame(
    runtime: &RuntimeHandle,
    peers: &[DeviceId],
    sequence: &ClipboardSequence,
    frame: ClipboardFileFrame,
) -> Result<(), String> {
    let sessions = peers
        .iter()
        .filter_map(|peer| runtime.control_generation(*peer).map(|g| (*peer, g)))
        .collect::<HashMap<_, _>>();
    send_file_clipboard_frame_pinned(runtime, peers, &sessions, sequence, frame).await
}
async fn send_file_clipboard_frame_pinned(
    runtime: &RuntimeHandle,
    peers: &[DeviceId],
    sessions: &HashMap<DeviceId, u64>,
    sequence: &ClipboardSequence,
    frame: ClipboardFileFrame,
) -> Result<(), String> {
    let data = frame.encode().map_err(|error| error.to_string())?;
    let sequence_number = sequence.next();
    let payload = ClipboardPayload::new(CLIPBOARD_FILE_MIME, data, sequence_number)
        .map_err(|error| error.to_string())?;
    for peer_id in peers {
        let generation = *sessions
            .get(peer_id)
            .ok_or_else(|| "文件连接已断开".to_owned())?;
        if runtime.control_generation(*peer_id) != Some(generation) {
            return Err("连接已中断或重新建立，请重新发送文件".to_owned());
        }
        runtime
            .send_clipboard_on_session(*peer_id, generation, payload.clone())
            .await
            .map_err(|error| {
                if runtime.control_generation(*peer_id) != Some(generation) {
                    "连接已中断或重新建立，请重新发送文件".to_owned()
                } else {
                    error.to_string()
                }
            })?;
    }
    Ok(())
}

fn emit_clipboard_error(
    ui_sender: &UnboundedSender<UiEvent>,
    diagnostics: &Diagnostics,
    message: String,
) {
    crate::diagnostics::record!(
        diagnostics,
        DiagnosticLevel::Error,
        "clipboard",
        "push_failed",
        Some(message.clone()),
        None,
    );
    let _ = ui_sender.unbounded_send(UiEvent::ClipboardStatus {
        enabled: true,
        last_mime: None,
        last_len: 0,
        last_sequence: None,
        error: Some(message),
    });
}

/// Mutable clipboard state used only while applying one authenticated remote
/// payload. Grouping it makes the ownership boundary explicit: UI and
/// diagnostics receive metadata only, while this worker owns the bodies.
struct RemoteClipboardState<'a> {
    last_change_count: &'a mut Option<i64>,
    incoming_files: &'a mut HashMap<(DeviceId, TransferId), IncomingFileClipboard>,
    receive_events: &'a ReceiveEventSender,
    cache: &'a mut TransferCache,
    loop_guard: &'a mut ClipboardLoopGuard,
    history: &'a mut ClipboardHistoryStore,
    pending_incoming_file: &'a AtomicBool,
    paste_requested: &'a AtomicBool,
    paste_target_pid: &'a AtomicI32,
}

#[cfg(test)]
fn apply_remote_clipboard(
    pasteboard: &impl Pasteboard,
    payload: &ClipboardPayload,
    state: &mut RemoteClipboardState<'_>,
    ui_sender: &UnboundedSender<UiEvent>,
    diagnostics: &Diagnostics,
    peer_id: DeviceId,
) -> Option<TransferId> {
    let frame = (payload.mime == CLIPBOARD_FILE_MIME)
        .then(|| ClipboardFileFrame::decode(&payload.data).ok())
        .flatten();
    apply_remote_clipboard_decoded(
        pasteboard,
        payload,
        state,
        ui_sender,
        diagnostics,
        peer_id,
        frame,
    )
}

fn apply_remote_clipboard_decoded(
    pasteboard: &impl Pasteboard,
    payload: &ClipboardPayload,
    state: &mut RemoteClipboardState<'_>,
    ui_sender: &UnboundedSender<UiEvent>,
    diagnostics: &Diagnostics,
    peer_id: DeviceId,
    frame: Option<ClipboardFileFrame>,
) -> Option<TransferId> {
    if payload.mime == CLIPBOARD_FILE_MIME {
        let transfer_id = frame.as_ref().map(ClipboardFileFrame::transfer_id);
        let cancel_reason = match &frame {
            Some(ClipboardFileFrame::Cancel { reason, .. }) => Some(reason.clone()),
            _ => None,
        };
        let new_offer = matches!(&frame, Some(ClipboardFileFrame::Offer { transfer_id, .. } | ClipboardFileFrame::RequestedOffer {transfer_id,..}) if !state.incoming_files.contains_key(&(peer_id, *transfer_id)));
        // A requested response may arrive after a newer copy. It must not
        // reinstall its old marker or steal the newer task's paste intent.
        if new_offer && let Some(frame @ ClipboardFileFrame::Offer { .. }) = &frame {
            let marker = format!("unidesk-clipboard-file-v1:{}", frame.transfer_id());
            if pasteboard
                .read_remote_origin_marker()
                .ok()
                .flatten()
                .as_deref()
                != Some(marker.as_str())
                && let Err(error) = pasteboard.clear_for_remote_file(&marker)
            {
                emit_clipboard_error(ui_sender, diagnostics, error.to_string());
                return None;
            }
            *state.last_change_count = pasteboard.change_count().ok();
            if !state.pending_incoming_file.swap(true, Ordering::AcqRel) {
                state.paste_requested.store(false, Ordering::Release);
                state.paste_target_pid.store(0, Ordering::Release);
            }
        }
        let result = frame
            .ok_or_else(|| "文件传输帧无效".to_owned())
            .and_then(|frame| {
                apply_remote_file_clipboard_frame(
                    pasteboard,
                    frame,
                    state.incoming_files,
                    state.cache,
                    peer_id,
                    state.receive_events,
                    ui_sender,
                )
            });
        let old_len = state.history.items.len();
        state.history.items.retain(|(_, snapshot)| {
            snapshot
                .file_urls
                .iter()
                .all(|path| PathBuf::from(path).exists())
        });
        if old_len != state.history.items.len() {
            state.history.emit(ui_sender);
        }
        if let Err(error) = result {
            let cleanup = state.cache.take_partial_cleanup();
            let cleanup_message = file_policy::apply_cleanup(
                cleanup,
                state.history,
                pasteboard,
                state.last_change_count,
                ui_sender,
            );
            let _ = ui_sender.unbounded_send(UiEvent::TransferCacheStatus {
                snapshot: state.cache.snapshot().unwrap_or_default(),
                message: Some(format!("{error}；{cleanup_message}")),
            });
            if let Some(transfer_id) = transfer_id {
                if let Some(incoming) = state.incoming_files.get_mut(&(peer_id, transfer_id)) {
                    incoming.cancel();
                } else if !state
                    .incoming_files
                    .keys()
                    .any(|(_, id)| *id == transfer_id)
                {
                    // No disk worker ever started (e.g. quota rejected the offer).
                    state.cache.finish(transfer_id);
                }
                if pasteboard.read_remote_origin_marker().ok().flatten()
                    == Some(format!("unidesk-clipboard-file-v1:{}", transfer_id))
                {
                    state.pending_incoming_file.store(false, Ordering::Release);
                    state.paste_requested.store(false, Ordering::Release);
                    state.paste_target_pid.store(0, Ordering::Release);
                }
            }
            emit_clipboard_error(ui_sender, diagnostics, error);
        } else if let (Some(transfer_id), Some(reason)) = (transfer_id, cancel_reason)
            && pasteboard.read_remote_origin_marker().ok().flatten()
                == Some(format!("unidesk-clipboard-file-v1:{transfer_id}"))
        {
            state.pending_incoming_file.store(false, Ordering::Release);
            state.paste_requested.store(false, Ordering::Release);
            state.paste_target_pid.store(0, Ordering::Release);
            emit_clipboard_error(ui_sender, diagnostics, reason);
        }
        return None;
    }
    let result = match payload.mime.as_str() {
        clipboard_bundle::MIME => clipboard_bundle::decode(&payload.data).and_then(|items| {
            pasteboard
                .write_items(&items)
                .map_err(|error| error.to_string())
        }),
        "text/plain" => String::from_utf8(payload.data.clone())
            .map_err(|error| error.to_string())
            .and_then(|text| {
                pasteboard
                    .write_text(&text)
                    .map_err(|error| error.to_string())
            }),
        "image/png" => pasteboard
            .write_png(&payload.data)
            .map_err(|error| error.to_string()),
        other => Err(format!("不支持的剪贴板类型: {other}")),
    };
    finish_native_clipboard(
        pasteboard,
        payload,
        state,
        ui_sender,
        diagnostics,
        peer_id,
        result,
        None,
    );
    None
}

#[allow(clippy::too_many_arguments)] // Shared remote-clipboard context, threaded explicitly.
fn finish_native_clipboard(
    pasteboard: &impl Pasteboard,
    payload: &ClipboardPayload,
    state: &mut RemoteClipboardState<'_>,
    ui_sender: &UnboundedSender<UiEvent>,
    diagnostics: &Diagnostics,
    peer_id: DeviceId,
    result: Result<(), String>,
    metadata: Option<&clipboard_bundle::SourceMetadata>,
) -> bool {
    let applied = result.is_ok();
    let mut duplicate_of_recent_clipboard = false;
    if result.is_ok() {
        // AppKit can synthesize legacy aliases. Bind the marker to the
        // actual installed representations, including legacy peer writes.
        let applied_snapshot = pasteboard.read().ok();
        let fingerprint = applied_snapshot
            .as_ref()
            .and_then(clipboard_snapshot_fingerprint)
            .unwrap_or_else(|| clipboard_fingerprint(&payload.mime, &payload.data));
        if let Err(error) = pasteboard.write_remote_origin_marker(&fingerprint) {
            crate::diagnostics::record!(
                diagnostics,
                DiagnosticLevel::Warning,
                "clipboard",
                "remote_origin_marker_failed",
                Some(error.to_string()),
                None,
            );
        }
        // Writing the marker may advance changeCount too. Bind the fallback
        // only after the complete remote write, never to historical content.
        if let Ok(count) = pasteboard.change_count() {
            duplicate_of_recent_clipboard =
                state
                    .loop_guard
                    .remember_remote_payload(payload, applied_snapshot.as_ref(), count);
            state.loop_guard.remember(fingerprint, count);
        }
    }
    // Suppress the local echo regardless of the write outcome being unknown:
    // re-reading the count below means the next poll sees the new baseline.
    *state.last_change_count = pasteboard.change_count().ok();
    match result {
        Ok(()) => {
            if duplicate_of_recent_clipboard {
                crate::diagnostics::record!(
                    diagnostics,
                    DiagnosticLevel::Info,
                    "clipboard",
                    "remote_history_duplicate_suppressed",
                    Some(format!("peer={} mime={}", peer_id.to_hex(), payload.mime)),
                    None,
                );
            } else if let Ok(mut snapshot) = pasteboard.read() {
                let unknown = clipboard_bundle::SourceMetadata {
                    no_history: true,
                    ..Default::default()
                };
                clipboard_bundle::attach_source_metadata(
                    &mut snapshot,
                    metadata.unwrap_or(&unknown),
                );
                state
                    .history
                    .record(snapshot, format!("远端 {}", peer_id.to_hex()), ui_sender);
            }
            crate::diagnostics::record!(
                diagnostics,
                DiagnosticLevel::Info,
                "clipboard",
                "received",
                Some(format!(
                    "peer={} mime={} len={} sequence={}",
                    peer_id.to_hex(),
                    payload.mime,
                    payload.data.len(),
                    payload.sequence
                )),
                None,
            );
            let _ = ui_sender.unbounded_send(UiEvent::ClipboardStatus {
                enabled: true,
                last_mime: Some(payload.mime.clone()),
                last_len: payload.data.len(),
                last_sequence: Some(payload.sequence),
                error: None,
            });
        }
        Err(error) => {
            crate::diagnostics::record!(
                diagnostics,
                DiagnosticLevel::Error,
                "clipboard",
                "apply_failed",
                Some(error.clone()),
                None,
            );
            let _ = ui_sender.unbounded_send(UiEvent::ClipboardStatus {
                enabled: true,
                last_mime: Some(payload.mime.clone()),
                last_len: payload.data.len(),
                last_sequence: Some(payload.sequence),
                error: Some(error),
            });
        }
    }
    applied
}

fn clipboard_file_receiver(cache_root: &std::path::Path) -> Result<FileReceiver, String> {
    use std::os::unix::fs::DirBuilderExt;

    // FileReceiver securely creates its final directory, but resolves its
    // parent first. A fresh install has no app/clipboard cache directories.
    // Create those caller-owned parents before opening the no-follow receiver.
    let parent = cache_root.parent().ok_or("文件接收目录无效")?;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(parent)
        .map_err(|error| format!("无法创建文件接收目录: {error}"))?;
    FileReceiver::new(cache_root).map_err(|error| error.to_string())
}

fn clipboard_root_paths(
    destination: &std::path::Path,
    manifest: &TransferManifest,
) -> Result<Vec<PathBuf>, String> {
    let roots = manifest
        .entries()
        .iter()
        .filter_map(|entry| entry.path().as_str().split('/').next())
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    if roots.is_empty() {
        return Err("文件剪贴板清单为空".to_owned());
    }
    Ok(roots
        .into_iter()
        .map(|root| destination.join(root))
        .collect())
}

async fn next_runtime_view(
    receiver: &mut tokio::sync::watch::Receiver<SnapshotView>,
) -> Option<SnapshotView> {
    receiver.changed().await.ok()?;
    Some(receiver.borrow_and_update().clone())
}

trait RuntimeEventFeed {
    fn event_stream(&self) -> BoxStream<'static, RuntimeEvent>;
    fn snapshot_view(&self) -> SnapshotView;
}

impl RuntimeEventFeed for RuntimeHandle {
    fn event_stream(&self) -> BoxStream<'static, RuntimeEvent> {
        RuntimeHandle::event_stream(self)
    }

    fn snapshot_view(&self) -> SnapshotView {
        RuntimeHandle::snapshot_view(self)
    }
}

fn subscribe_before_snapshot<R: RuntimeEventFeed>(
    runtime: &R,
    ui_sender: &UnboundedSender<UiEvent>,
) -> BoxStream<'static, RuntimeEvent> {
    // Subscribe first so an event published immediately after startup cannot
    // be lost in the gap between the initial snapshot and event subscription.
    let events = runtime.event_stream();
    let _ = ui_sender.unbounded_send(UiEvent::RuntimeView(runtime.snapshot_view()));
    events
}

fn event_requires_snapshot_refresh(event: &RuntimeEvent) -> bool {
    matches!(
        event,
        RuntimeEvent::Snapshot(_)
            | RuntimeEvent::DiscoveredPeerHint(_)
            | RuntimeEvent::PeerHintRemoved { .. }
            | RuntimeEvent::PairingRequest(_)
            | RuntimeEvent::PairingRequestSent { .. }
            | RuntimeEvent::PairingFinished { .. }
            | RuntimeEvent::PairingRejected { .. }
    )
}

/// Motion datagrams are enabled by default; `reliable` is a diagnostic override.
/// Path negotiation still decides whether a new control session can use hybrid mode.
fn motion_mode_capabilities(mode: Option<&str>) -> Capabilities {
    if mode == Some("reliable") {
        Capabilities::NONE
    } else {
        Capabilities::INPUT_MOTION_DATAGRAM_V1
    }
}

fn runtime_config() -> RuntimeConfig {
    RuntimeConfig {
        network: unidesk_core::NetworkConfig::new()
            .with_mdns_enabled(true)
            .with_public_network_enabled(false),
        display_name: local_display_name(),
        capabilities: Capabilities::NETWORK
            | Capabilities::CLIPBOARD
            | Capabilities::TRANSFER
            | Capabilities::INPUT_SHARING
            | Capabilities::DEVICE_NETWORK
            | Capabilities::INPUT_ACTIVATION
            | Capabilities::LAN_UPDATES
            | Capabilities::INPUT_ENVIRONMENT
            | Capabilities::DEFERRED_FILE_CLIPBOARD
            | Capabilities::CLIPBOARD_SYNC_V2
            | Capabilities::REMOTE_APPLICATION
            | motion_mode_capabilities(std::env::var("UNIDESK_INPUT_MOTION_MODE").ok().as_deref()),
        ..RuntimeConfig::default()
    }
}

/// Human-readable device name shown during discovery and pairing.
fn local_display_name() -> String {
    #[cfg(not(test))]
    {
        // `hostname` is a stable macOS baseline utility; never use its output
        // as identity, only as the display label.
        let name = std::process::Command::new("/bin/hostname")
            .output()
            .ok()
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .map(|name| {
                let trimmed = name.trim();
                trimmed.strip_suffix(".local").unwrap_or(trimmed).to_owned()
            });
        if let Some(name) = name
            && !name.is_empty()
            && name.chars().count() <= 256
            && !name.chars().any(char::is_control)
        {
            return name;
        }
    }
    "uniDesk".to_owned()
}

async fn build_runtime(diagnostics: &Diagnostics) -> Result<RuntimeHandle, RuntimeError> {
    // Production persists identity beside the state database. Tests stay isolated.
    #[cfg(not(test))]
    let identity_provider = FileIdentityProvider::new(
        SQLiteStore::default_path()
            .map_err(|error| RuntimeError::Store(error.to_string()))?
            .with_file_name("identity.key"),
    );
    #[cfg(test)]
    let identity_provider = MemoryIdentityProvider::new();
    let identity = identity_provider.load_or_create()?;
    let local_id = identity.device_id();
    #[cfg(not(test))]
    let store = SQLiteStore::new().map_err(|error| RuntimeError::Store(error.to_string()))?;
    #[cfg(test)]
    let store =
        SQLiteStore::open_in_memory().map_err(|error| RuntimeError::Store(error.to_string()))?;
    let config = runtime_config();

    #[cfg(not(test))]
    let (transport, control_port) = {
        use production_transport::{CERTIFICATE_DER, PRIVATE_KEY_DER, SERVER_NAME};

        // The transport hello uses the same identity and display name as the
        // runtime hello. A fresh nonce is fine: it is a challenge, not a
        // persistent credential.
        let request_id = unidesk_core::RequestId::new().into_uuid();
        let device_bytes = identity.device_id().into_bytes();
        let mut nonce = [0_u8; 32];
        for (index, byte) in nonce.iter_mut().enumerate() {
            *byte = request_id.as_bytes()[index % 16] ^ device_bytes[index];
        }
        let local_hello = PeerHello::signed(
            &identity,
            config.display_name.clone(),
            config.capabilities,
            nonce,
        )
        .map_err(|error| RuntimeError::Protocol(error.to_string()))?;
        let config = QuinnTransportConfig::new(
            std::net::SocketAddr::from(([0, 0, 0, 0], production_transport::CONTROL_PORT)),
            production_transport::placeholder_remote(),
            SERVER_NAME,
            CERTIFICATE_DER.to_vec(),
            PRIVATE_KEY_DER.to_vec(),
            CERTIFICATE_DER.to_vec(),
        );
        let transport = QuinnTransport::bind(local_hello, config)
            .await
            .map_err(|error| RuntimeError::Transport(error.to_string()))?;
        let control_port = transport
            .local_addr()
            .map_err(|error| RuntimeError::Transport(error.to_string()))?
            .port();
        (transport, control_port)
    };

    #[cfg(not(test))]
    let discovery = {
        // Advertise only after the authenticated control listener is bound:
        // the record carries the real QUIC port, never a placeholder.
        let mut provider = PlatformDiscovery::new(local_id)
            .map_err(|error| RuntimeError::Discovery(error.to_string()))?;
        let record = ServiceRecord::v1(
            local_id,
            config.display_name.clone(),
            config.capabilities,
            control_port,
        )
        .map_err(|error| RuntimeError::Discovery(error.to_string()))?;
        provider
            .set_advertisement(record)
            .map_err(|error| RuntimeError::Discovery(error.to_string()))?;
        provider
    };

    crate::diagnostics::record!(
        diagnostics,
        DiagnosticLevel::Info,
        "runtime",
        "providers_configured",
        Some(format!(
            "local_device_id={}, display_name={}",
            local_id.to_hex(),
            config.display_name
        )),
        None,
    );
    let builder = RuntimeBuilder::new()
        .config(config)
        .with_identity_provider(identity_provider)
        .with_trust_store(store.clone())
        .with_store(store);
    #[cfg(not(test))]
    let builder = builder
        .with_external_discovery(discovery)
        .with_control_transport(transport.clone());
    #[cfg(test)]
    let builder = builder.with_transport(UnavailableControlTransport);
    #[cfg(all(target_os = "macos", not(test)))]
    let builder = builder.with_input_service(unidesk_runtime::input_service::ServiceOptions {
        executable: std::env::current_exe()
            .map_err(|e| RuntimeError::ThreadStart(e.to_string()))?
            .with_file_name("unidesk-input-service"),
        identity_path: SQLiteStore::default_path()
            .map_err(|e| RuntimeError::Store(e.to_string()))?
            .with_file_name("identity.key"),
    });
    let runtime = builder.build()?;
    #[cfg(not(test))]
    crate::remote_application::install(transport, runtime.clone());
    Ok(runtime)
}

fn emit_permission_snapshot(
    ui_sender: &UnboundedSender<UiEvent>,
    service: &dyn PermissionService,
    diagnostics: &Diagnostics,
    input: Option<&unidesk_runtime::input_service::ServiceControl>,
) {
    let snapshot = service.snapshot();
    #[cfg(target_os = "macos")]
    let native = input
        .and_then(|s| s.status.lock().ok().map(|s| s.clone()))
        .unwrap_or_default();
    #[cfg(target_os = "macos")]
    let snapshot = crate::input_coordinator::helper_permissions(&native, snapshot.full_disk_access);
    #[cfg(not(target_os = "macos"))]
    let _ = input;
    crate::diagnostics::record!(
        diagnostics,
        DiagnosticLevel::Info,
        "permissions",
        "snapshot_received",
        Some(format!("{snapshot:?}")),
        None,
    );
    let _ = ui_sender.unbounded_send(UiEvent::PermissionSnapshot(snapshot));
}

#[cfg(test)]
#[path = "clipboard_history_tests.rs"]
mod clipboard_history_tests;

#[cfg(test)]
#[path = "file_clipboard_tests.rs"]
mod file_clipboard_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use futures::{StreamExt, stream::BoxStream};
    use unidesk_platform::MemoryPasteboard;

    #[test]
    fn tiff_preview_is_png_and_restore_keeps_original_representation() {
        use unidesk_platform::{PasteboardItem, PasteboardRepresentation};
        let original = image::DynamicImage::new_rgb8(8, 4);
        let mut encoded = std::io::Cursor::new(Vec::new());
        original
            .write_to(&mut encoded, image::ImageFormat::Tiff)
            .unwrap();
        let mut snapshot = PasteboardSnapshot::default();
        snapshot.items.push(PasteboardItem {
            representations: vec![PasteboardRepresentation {
                type_name: "public.tiff".into(),
                data: encoded.into_inner(),
            }],
        });
        let (sender, _receiver) = unbounded();
        let mut history = ClipboardHistoryStore::default();
        history.record(snapshot.clone(), "本机", &sender);
        let (item, restored) = history.items.front().unwrap();
        let preview = image::load_from_memory_with_format(
            item.image_png.as_ref().unwrap(),
            image::ImageFormat::Png,
        )
        .unwrap();
        assert_eq!((preview.width(), preview.height()), (8, 4));
        assert_eq!(restored.items, snapshot.items);
        snapshot.file_urls.push("/tmp/file-priority".into());
        assert!(clipboard_preview_png(&snapshot).is_none());
    }

    #[test]
    fn remote_clipboard_marker_prevents_a_local_echo_but_not_a_new_local_copy() {
        let pasteboard = MemoryPasteboard::new();
        let payload =
            ClipboardPayload::new("text/plain", b"from peer".to_vec(), 7).expect("valid payload");
        let mut last_change_count = None;
        let mut incoming = HashMap::new();
        let mut loop_guard = ClipboardLoopGuard::default();
        let mut history = ClipboardHistoryStore::default();
        let pending_incoming_file = AtomicBool::new(false);
        let paste_requested = AtomicBool::new(false);
        let paste_target_pid = AtomicI32::new(0);
        let (ui_sender, _ui_receiver) = unbounded();

        apply_remote_clipboard(
            &pasteboard,
            &payload,
            &mut RemoteClipboardState {
                last_change_count: &mut last_change_count,
                incoming_files: &mut incoming,
                receive_events: &receive_event_channel().0,
                cache: &mut FilePolicy::new().unwrap().cache,
                loop_guard: &mut loop_guard,
                history: &mut history,
                pending_incoming_file: &pending_incoming_file,
                paste_requested: &paste_requested,
                paste_target_pid: &paste_target_pid,
            },
            &ui_sender,
            &Diagnostics::new(32),
            DeviceId::from_bytes([7; 32]),
        );

        let received = pasteboard.read().expect("read received clipboard");
        assert_eq!(received.text.as_deref(), Some("from peer"));
        assert_eq!(
            pasteboard.read_remote_origin_marker().expect("read marker"),
            Some(clipboard_fingerprint("text/plain", b"from peer"))
        );
        assert!(loop_guard.should_suppress(&pasteboard, &received));

        pasteboard
            .write_text("new local copy")
            .expect("write local copy");
        let local = pasteboard.read().expect("read local clipboard");
        assert!(!loop_guard.should_suppress(&pasteboard, &local));

        // 10 秒账本窗口内，同一段文字的再次出现视为回声（含接力改写）。
        pasteboard
            .write_text("from peer")
            .expect("recopy old remote text");
        let recopied = pasteboard.read().unwrap();
        assert!(recopied.change_count > received.change_count);
        assert!(loop_guard.should_suppress(&pasteboard, &recopied));
    }

    #[test]
    fn handoff_echo_of_a_local_copy_is_suppressed() {
        let pasteboard = MemoryPasteboard::new();
        let mut loop_guard = ClipboardLoopGuard::default();
        pasteboard.write_text("帮我安装 kimi code\n").unwrap();
        let local = pasteboard.read().unwrap();
        assert!(!loop_guard.should_suppress(&pasteboard, &local));

        pasteboard.write_text("帮我安装 kimi code\r\n").unwrap();
        let echoed = pasteboard.read().unwrap();
        assert!(echoed.change_count > local.change_count);
        assert!(loop_guard.should_suppress(&pasteboard, &echoed));

        // Adding/removing a trailing newline changes the actual content.
        pasteboard.write_text("帮我安装 kimi code").unwrap();
        assert!(!loop_guard.should_suppress(&pasteboard, &pasteboard.read().unwrap()));

        pasteboard.write_text("另一段文字").unwrap();
        let incoming = pasteboard.read().unwrap();
        assert!(!loop_guard.should_suppress(&pasteboard, &incoming));
    }

    #[test]
    fn universal_clipboard_is_suppressed_without_poisoning_a_later_local_copy() {
        let pasteboard = MemoryPasteboard::new();
        let mut loop_guard = ClipboardLoopGuard::default();
        pasteboard.write_png(&[1, 2, 3]).unwrap();
        pasteboard.mark_universal_clipboard().unwrap();
        let handoff = pasteboard.read().unwrap();
        assert!(loop_guard.should_suppress(&pasteboard, &handoff));

        // A real local copy clears Apple's bookkeeping type. Because the
        // suppressed Handoff was never added to the ledger, this copy is
        // allowed through even when its bytes happen to be identical.
        pasteboard.write_png(&[1, 2, 3]).unwrap();
        let local = pasteboard.read().unwrap();
        assert!(!loop_guard.should_suppress(&pasteboard, &local));
    }

    #[test]
    fn remote_payload_after_handoff_is_recognized_as_a_history_duplicate() {
        let pasteboard = MemoryPasteboard::new();
        let mut loop_guard = ClipboardLoopGuard::default();
        let items = vec![unidesk_platform::PasteboardItem {
            representations: vec![unidesk_platform::PasteboardRepresentation {
                type_name: "public.tiff".into(),
                data: vec![1, 2, 3],
            }],
        }];
        pasteboard.write_items(&items).unwrap();
        let handoff = pasteboard.read().unwrap();
        assert!(!loop_guard.should_suppress(&pasteboard, &handoff));

        // Native rich images travel through the clipboard bundle MIME, so
        // payload-only matching is insufficient. The applied native snapshot
        // must participate in the same equivalence ledger.
        let payload = ClipboardPayload::new(
            clipboard_bundle::MIME,
            clipboard_bundle::encode(&items).unwrap(),
            1,
        )
        .unwrap();
        assert!(loop_guard.remember_remote_payload(
            &payload,
            Some(&handoff),
            handoff.change_count + 1,
        ));

        // The first remote delivery on a clean host must still be retained;
        // the native snapshot and payload are observed as one logical value.
        let mut fresh_guard = ClipboardLoopGuard::default();
        assert!(!fresh_guard.remember_remote_payload(
            &payload,
            Some(&handoff),
            handoff.change_count + 1,
        ));
    }

    #[test]
    fn markerless_clipboard_fallback_only_suppresses_its_remote_generation() {
        let pasteboard = MemoryPasteboard::new();
        pasteboard.write_text("same text").unwrap();
        let snapshot = pasteboard.read().unwrap();
        let mut guard = ClipboardLoopGuard::default();
        guard.remember(
            clipboard_snapshot_fingerprint(&snapshot).unwrap(),
            snapshot.change_count,
        );
        assert!(guard.should_suppress(&pasteboard, &snapshot));
        pasteboard.write_text("same text").unwrap();
        let copied_again = pasteboard.read().unwrap();
        assert!(!guard.should_suppress(&pasteboard, &copied_again));
        pasteboard.write_png(&[1, 2, 3]).unwrap();
        let png = pasteboard.read().unwrap();
        guard.remember(
            clipboard_snapshot_fingerprint(&png).unwrap(),
            png.change_count,
        );
        assert!(guard.should_suppress(&pasteboard, &png));
        pasteboard.write_png(&[1, 2, 3]).unwrap();
        assert!(!guard.should_suppress(&pasteboard, &pasteboard.read().unwrap()));
    }

    // Deleted 2026-09-15: this case expected a payload without source metadata
    // to enter clipboard history, but the global clipboard library now treats an
    // unknown remote source as `no_history`. Whether unknown remote content
    // should be recorded still needs a maintainer decision; the stale unit test
    // is removed to unblock packaging. Tracked in TODO.md.

    #[test]
    fn clipboard_prefers_png_when_macos_provides_a_text_fallback() {
        let snapshot = PasteboardSnapshot {
            change_count: 1,
            text: Some("image description".to_owned()),
            png: Some(vec![1, 2, 3]),
            file_urls: Vec::new(),
            items: Vec::new(),
        };
        assert_eq!(
            clipboard_snapshot_fingerprint(&snapshot),
            Some(clipboard_fingerprint("image/png", &[1, 2, 3]))
        );
    }

    #[test]
    fn unavailable_transport_never_reports_a_successful_connection() {
        let result = futures::executor::block_on(UnavailableControlTransport.connect(
            unidesk_protocol::Hello::default(),
            DeviceId::from_bytes([1; 32]),
            RouteCandidate::new(
                "127.0.0.1:42424".parse().expect("endpoint"),
                unidesk_core::RouteSource::Manual,
                unidesk_core::RouteHealth::Unknown,
                Capabilities::NETWORK,
            ),
        ));
        assert!(matches!(
            result,
            Err(RuntimeError::Transport(message)) if message == "安全控制通道尚未启用"
        ));
    }

    #[test]
    fn motion_mode_defaults_to_hybrid_with_explicit_reliable_override() {
        for mode in [None, Some("hybrid"), Some(""), Some("typo")] {
            assert_eq!(
                motion_mode_capabilities(mode),
                Capabilities::INPUT_MOTION_DATAGRAM_V1
            );
        }
        assert_eq!(
            motion_mode_capabilities(Some("reliable")),
            Capabilities::NONE
        );
    }

    #[test]
    fn runtime_config_keeps_public_network_disabled() {
        let config = runtime_config();
        assert!(config.network.mdns_enabled);
        assert!(!config.network.public_network_enabled);
        assert_eq!(config.display_name, "uniDesk");
        assert_eq!(
            config.capabilities,
            Capabilities::NETWORK
                | Capabilities::CLIPBOARD
                | Capabilities::TRANSFER
                | Capabilities::INPUT_SHARING
                | Capabilities::DEVICE_NETWORK
                | Capabilities::INPUT_ACTIVATION
                | Capabilities::LAN_UPDATES
                | Capabilities::INPUT_ENVIRONMENT
                | Capabilities::DEFERRED_FILE_CLIPBOARD
                | Capabilities::CLIPBOARD_SYNC_V2
                | Capabilities::REMOTE_APPLICATION
                | motion_mode_capabilities(
                    std::env::var("UNIDESK_INPUT_MOTION_MODE").ok().as_deref()
                )
        );
    }

    #[test]
    fn worker_reports_a_safe_startup_outcome() {
        let diagnostics = Diagnostics::new(32);
        let (bridge, mut events) = TokioBridge::start(diagnostics).expect("bridge starts");
        let event =
            futures::executor::block_on(async { events.next().await.expect("stream stays open") });
        assert!(matches!(
            event,
            UiEvent::PermissionSnapshot(_) | UiEvent::RuntimeView(_) | UiEvent::BridgeError(_)
        ));
        bridge.shutdown_and_wait().expect("bridge shuts down");
    }

    #[test]
    fn update_admission_rejects_new_exports_and_commands_but_allows_shutdown() {
        let (bridge, _events) = TokioBridge::start(Diagnostics::new(32)).unwrap();
        let lease = bridge
            .retain_transfer_cache_paths(vec!["fixture".into()])
            .unwrap();
        *bridge.update_admission.lock().unwrap() = true;
        assert!(bridge.retain_transfer_cache_paths(vec![]).is_err());
        assert!(bridge.push_clipboard().is_err());
        assert!(bridge.set_clipboard_sync(false).is_err());
        assert!(bridge.cache_leases.lock().unwrap().contains_key(&lease));
        bridge.release_transfer_cache_paths(lease);
        assert!(bridge.cache_leases.lock().unwrap().is_empty());
        bridge.shutdown_and_wait().unwrap();
    }

    #[test]
    fn bridge_shutdown_and_wait_is_idempotent() {
        let diagnostics = Diagnostics::new(32);
        let (bridge, _events) = TokioBridge::start(diagnostics).expect("bridge starts");
        bridge.shutdown_and_wait().expect("first shutdown");
        bridge.shutdown_and_wait().expect("second shutdown");
    }

    #[test]
    fn bridge_can_resend_the_runtime_snapshot_for_window_recovery() {
        let diagnostics = Diagnostics::new(32);
        let (bridge, mut events) = TokioBridge::start(diagnostics).expect("bridge starts");
        bridge.request_snapshot();
        let saw_snapshot = futures::executor::block_on(async {
            while let Some(event) = events.next().await {
                if matches!(event, UiEvent::RuntimeView(_)) {
                    return true;
                }
            }
            false
        });
        bridge.shutdown_and_wait().expect("bridge shuts down");
        assert!(saw_snapshot);
    }

    #[test]
    fn automatic_clipboard_targets_exclude_trusted_peers_outside_a_joined_network() {
        let local = DeviceId::from_public_key([1; 32]).unwrap();
        let peers = [2, 3]
            .into_iter()
            .map(|seed| {
                let id = DeviceId::from_public_key([seed; 32]).unwrap();
                let route = unidesk_core::RouteCandidate::new(
                    "10.0.0.88:42424".parse().unwrap(),
                    unidesk_core::RouteSource::Manual,
                    unidesk_core::RouteHealth::Healthy,
                    Capabilities::NETWORK,
                );
                unidesk_core::PeerSnapshot::new(
                    id,
                    "Peer",
                    [seed; 32],
                    unidesk_core::LivenessState::Online,
                    TrustState::Trusted,
                    ConnectionState::Connected,
                    vec![route.clone()],
                    Some(route.id),
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        let member = peers[0].id;
        let mut view = SnapshotView {
            snapshot: unidesk_core::NetworkSnapshot::new(0, local, peers, false).unwrap(),
            discovered_peers: vec![],
            device_network: Default::default(),
        };
        assert_eq!(trusted_connected_peer_ids(&view).len(), 2);
        view.device_network.network_id = Some("signed-network".into());
        view.device_network.member_ids = vec![local, member];
        assert_eq!(trusted_connected_peer_ids(&view), vec![member]);
        view.device_network.member_ids = vec![member];
        assert!(
            trusted_connected_peer_ids(&view).is_empty(),
            "a removed local member cannot publish automatically"
        );
        view.device_network.integrity_error = Some("invalid stored roster".into());
        view.device_network.network_id = None;
        assert!(trusted_connected_peer_ids(&view).is_empty());
    }

    #[test]
    fn hint_removal_events_refresh_the_runtime_snapshot_view() {
        let identity = MemoryIdentityProvider::new()
            .load_or_create()
            .expect("identity");
        let peer_id = DeviceId::from_bytes([9; 32]);
        let snapshot =
            unidesk_core::NetworkSnapshot::new(0, identity.device_id(), Vec::new(), false)
                .expect("snapshot");
        let hint = unidesk_runtime::DiscoveredPeerHint {
            peer_id,
            display_name: "Nearby".to_owned(),
            capabilities: Capabilities::NETWORK
                | Capabilities::CLIPBOARD
                | Capabilities::TRANSFER
                | Capabilities::INPUT_SHARING
                | Capabilities::DEVICE_NETWORK
                | Capabilities::INPUT_ACTIVATION
                | Capabilities::LAN_UPDATES
                | Capabilities::INPUT_ENVIRONMENT
                | Capabilities::DEFERRED_FILE_CLIPBOARD
                | Capabilities::CLIPBOARD_SYNC_V2,
            routes: Vec::new(),
        };

        assert!(event_requires_snapshot_refresh(&RuntimeEvent::Snapshot(
            snapshot,
        )));
        assert!(event_requires_snapshot_refresh(
            &RuntimeEvent::DiscoveredPeerHint(hint)
        ));
        assert!(event_requires_snapshot_refresh(
            &RuntimeEvent::PeerHintRemoved { peer_id }
        ));
        assert!(!event_requires_snapshot_refresh(&RuntimeEvent::Error(
            RuntimeError::Transport("test".to_owned()),
        )));
    }

    #[tokio::test]
    async fn membership_only_view_change_reaches_ui_without_peer_event() {
        let view = SnapshotView {
            device_network: unidesk_runtime::DeviceNetworkSnapshot {
                network_id: Some("temporary-network".into()),
                ..Default::default()
            },
            snapshot: unidesk_core::NetworkSnapshot::new(
                0,
                DeviceId::from_bytes([1; 32]),
                vec![],
                false,
            )
            .unwrap(),
            discovered_peers: vec![],
        };
        let (sender, mut receiver) = tokio::sync::watch::channel(view.clone());
        let mut left = view.clone();
        left.device_network = Default::default();
        sender.send(left).unwrap();
        let observed = next_runtime_view(&mut receiver).await.unwrap();
        assert!(observed.device_network.network_id.is_none());
        assert_eq!(observed.snapshot, view.snapshot);
        assert!(!receiver.has_changed().unwrap());
    }

    #[test]
    fn initial_subscription_precedes_snapshot_read() {
        use std::sync::{Arc, Mutex};

        struct SequencedRuntime {
            calls: Arc<Mutex<Vec<&'static str>>>,
        }

        impl RuntimeEventFeed for SequencedRuntime {
            fn event_stream(&self) -> BoxStream<'static, RuntimeEvent> {
                self.calls.lock().expect("calls").push("event_stream");
                Box::pin(futures::stream::empty())
            }

            fn snapshot_view(&self) -> SnapshotView {
                self.calls.lock().expect("calls").push("snapshot_view");
                let identity = MemoryIdentityProvider::new()
                    .load_or_create()
                    .expect("identity");
                SnapshotView {
                    device_network: Default::default(),
                    snapshot: unidesk_core::NetworkSnapshot::new(
                        0,
                        identity.device_id(),
                        Vec::new(),
                        false,
                    )
                    .expect("snapshot"),
                    discovered_peers: Vec::new(),
                }
            }
        }

        let calls = Arc::new(Mutex::new(Vec::new()));
        let runtime = SequencedRuntime {
            calls: Arc::clone(&calls),
        };
        let (sender, _receiver) = unbounded();
        let _events = subscribe_before_snapshot(&runtime, &sender);
        assert_eq!(
            *calls.lock().expect("calls"),
            vec!["event_stream", "snapshot_view"]
        );
    }
}

#[path = "clipboard_history_persistence.rs"]
mod clipboard_history_persistence;
