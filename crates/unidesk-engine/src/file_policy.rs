//! Deferred clipboard offers and the application-owned receive cache policy.
use super::*;
use std::time::Instant;

const OFFER_LIFETIME: Duration = Duration::from_secs(10 * 60);
#[derive(Default)]
pub(super) struct DeferredSourceStore {
    generation: u64,
    entries: HashMap<TransferId, DeferredOffer>,
}
impl DeferredSourceStore {
    pub fn clear(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.entries.clear();
    }
}
#[derive(Clone)]
struct DeferredOffer {
    source: DeferredClipboardSource,
    peer: DeviceId,
    created: Instant,
}
pub(super) type DeferredSources = Arc<Mutex<DeferredSourceStore>>;
struct PendingOffer {
    id: TransferId,
    peer: DeviceId,
    name: String,
    total_bytes: u64,
    change_count: i64,
    requested: bool,
    created: Instant,
}
pub(super) struct FilePolicy {
    pub settings: FileTransferSettings,
    pub cache: TransferCache,
    pub outgoing: DeferredSources,
    store: SQLiteStore,
    pending: Option<PendingOffer>,
    workflow_demand: Option<TransferId>,
    outgoing_pins: HashMap<TransferId, Vec<TransferId>>,
    clipboard_paths: Vec<String>,
    clipboard_change_count: Option<i64>,
    clipboard_pins: Vec<TransferId>,
    paste_leases: Vec<(Instant, Vec<TransferId>)>,
}
impl FilePolicy {
    pub(super) fn workflow_request(&mut self, id: TransferId) {
        if self.pending.as_ref().is_some_and(|offer| offer.id == id) {
            self.workflow_demand = Some(id);
        }
    }
    pub(super) fn workflow_pending_id(&self) -> Option<TransferId> {
        self.pending.as_ref().map(|offer| offer.id)
    }
    pub(super) fn workflow_requested(&self, id: TransferId) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|offer| offer.id == id && offer.requested)
    }

    pub(super) fn connection_lost(
        &mut self,
        peer: DeviceId,
        pasteboard: &impl Pasteboard,
        incoming: &mut HashMap<(DeviceId, TransferId), IncomingFileClipboard>,
        flag: &AtomicBool,
        requested: &AtomicBool,
        target: &AtomicI32,
        ui: &UnboundedSender<UiEvent>,
    ) {
        if self.pending.as_ref().is_some_and(|p| p.peer == peer) {
            if let Some(offer) = self.pending.take() {
                if pasteboard.change_count().ok() == Some(offer.change_count) {
                    flag.store(false, Ordering::Release);
                    requested.store(false, Ordering::Release);
                    target.store(0, Ordering::Release);
                }
            }
        }
        for ((id, transfer), state) in incoming.iter_mut().filter(|((id, _), _)| *id == peer) {
            if pasteboard.change_count().ok() == Some(state.pasteboard_change_count) {
                flag.store(false, Ordering::Release);
                requested.store(false, Ordering::Release);
                target.store(0, Ordering::Release);
            }
            state.cancel();
            emit_receive_transfer_status(
                ui,
                *transfer,
                state.name.clone(),
                "连接已中断，请重新复制".into(),
                TransferState::Cancelled,
                state.received_bytes,
                state.manifest.total_bytes(),
            );
            let _ = id;
        }
        if let Ok(mut store) = self.outgoing.lock() {
            store.entries.retain(|_, offer| offer.peer != peer);
        }
    }

    pub(super) fn reference_installed(&self, peer: DeviceId, id: TransferId) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|p| p.peer == peer && p.id == id)
    }

    pub fn update_busy(&self) -> bool {
        self.paste_leases
            .iter()
            .any(|(created, _)| created.elapsed() < Duration::from_secs(120))
            || self.pending.as_ref().is_some_and(|p| p.requested)
            || !self.outgoing_pins.is_empty()
    }
    pub fn new() -> Result<Self, String> {
        #[cfg(test)]
        let store = SQLiteStore::open_in_memory();
        #[cfg(not(test))]
        let store = SQLiteStore::new();
        let store = store.map_err(|e| e.to_string())?;
        let settings = store.file_transfer_settings().map_err(|e| e.to_string())?;
        #[cfg(test)]
        let root = std::env::temp_dir()
            .canonicalize()
            .map_err(|e| e.to_string())?
            .join(format!("unidesk-test-cache-{}", TransferId::new()));
        #[cfg(not(test))]
        let root = dirs::cache_dir()
            .ok_or("无法确定系统缓存目录")?
            .join("com.unidesk.app/clipboard");
        // Establish pasteboard/native export leases before the first eviction.
        // Restarting the app must not delete files still offered by the OS.
        let cache = TransferCache::new(root, settings.cache_max_bytes)?;
        Ok(Self {
            settings,
            cache,
            outgoing: Arc::default(),
            store,
            pending: None,
            workflow_demand: None,
            outgoing_pins: HashMap::new(),
            clipboard_paths: Vec::new(),
            clipboard_change_count: None,
            clipboard_pins: Vec::new(),
            paste_leases: Vec::new(),
        })
    }
    pub fn initialize_cache(
        &mut self,
        pasteboard: &impl Pasteboard,
    ) -> (CacheCleanup, Option<String>) {
        self.protect_clipboard(pasteboard);
        match self.cache.set_limit(self.settings.cache_max_bytes) {
            Ok(cleanup) => (cleanup, None),
            Err(error) => (self.cache.take_partial_cleanup(), Some(error)),
        }
    }
    pub fn emit(&self, ui: &UnboundedSender<UiEvent>, message: Option<String>) {
        let _ = ui.unbounded_send(UiEvent::FileTransferSettings(self.settings.clone()));
        match self.cache.snapshot() {
            Ok(snapshot) => {
                let _ = ui.unbounded_send(UiEvent::TransferCacheStatus { snapshot, message });
            }
            Err(error) => {
                let _ = ui.unbounded_send(UiEvent::BridgeError(error));
            }
        }
    }
    pub fn update(&mut self, settings: FileTransferSettings) -> Result<CacheCleanup, String> {
        settings.validate()?;
        // Check active reservations before persisting; historical eviction uses
        // only the new accepted budget and never interrupts an active transfer.
        if self.cache.snapshot()?.active_reserved_bytes > settings.cache_max_bytes {
            return Err("新上限小于活动任务预留空间，请等待任务完成或先取消".into());
        }
        self.store
            .set_file_transfer_settings(&settings)
            .map_err(|e| e.to_string())?;
        self.settings = settings;
        let cleanup = self.cache.set_limit(self.settings.cache_max_bytes)?;
        Ok(cleanup)
    }
    #[allow(clippy::too_many_arguments)]
    pub async fn control(
        &mut self,
        frame: &ClipboardFileFrame,
        peer: DeviceId,
        pasteboard: &impl Pasteboard,
        last_change: &mut Option<i64>,
        pending_flag: &AtomicBool,
        requested_flag: &AtomicBool,
        target_pid: &AtomicI32,
        runtime: &RuntimeHandle,
        sequence: &ClipboardSequence,
        ui: &UnboundedSender<UiEvent>,
        diagnostics: &Diagnostics,
        cancellations: &mut HashMap<TransferId, Arc<AtomicBool>>,
        sources: &mut HashMap<TransferId, FileTransferSource>,
        acknowledgements: &FileClipboardAcknowledgements,
        task_events: &TokioSender<ClipboardTransferTaskEvent>,
    ) -> bool {
        match frame {
            ClipboardFileFrame::Deferred {
                transfer_id,
                names,
                total_bytes,
            } => {
                if self
                    .pending
                    .as_ref()
                    .is_some_and(|offer| offer.id == *transfer_id && offer.peer == peer)
                {
                    return true;
                }
                if let Some(old) = self.pending.take() {
                    let runtime = runtime.clone();
                    let sequence = sequence.clone();
                    tokio::spawn(async move {
                        let _ = send_file_clipboard_frame(
                            &runtime,
                            &[old.peer],
                            &sequence,
                            ClipboardFileFrame::Cancel {
                                transfer_id: old.id,
                                reason: "剪贴板已被新内容替换".into(),
                            },
                        )
                        .await;
                    });
                }
                let marker = format!("unidesk-clipboard-file-v1:{transfer_id}");
                if let Err(error) = pasteboard.clear_for_remote_file(&marker) {
                    emit_clipboard_error(ui, diagnostics, error.to_string());
                    return true;
                }
                *last_change = pasteboard.change_count().ok();
                let Some(change_count) = *last_change else {
                    return true;
                };
                pending_flag.store(true, Ordering::Release);
                requested_flag.store(false, Ordering::Release);
                target_pid.store(0, Ordering::Release);
                let name = names.join("、");
                self.pending = Some(PendingOffer {
                    id: *transfer_id,
                    peer,
                    name,
                    total_bytes: *total_bytes,
                    change_count,
                    requested: false,
                    created: Instant::now(),
                });
                diagnostics.record(
                    DiagnosticLevel::Info,
                    "clipboard",
                    "deferred_received",
                    Some(format!(
                        "peer={} transfer={} items={} bytes={} change_count={change_count}",
                        peer.to_hex(),
                        transfer_id,
                        names.len(),
                        total_bytes,
                    )),
                    None,
                );
                let _ = ui.unbounded_send(UiEvent::ClipboardStatus {
                    enabled: true,
                    last_mime: Some(CLIPBOARD_FILE_MIME.into()),
                    last_len: usize::try_from(*total_bytes).unwrap_or(usize::MAX),
                    last_sequence: Some(sequence.latest()),
                    error: None,
                });
                // No transfer row/progress until bytes have actually been requested.
                true
            }
            ClipboardFileFrame::RequestedOffer { transfer_id, .. } => {
                let valid = self.pending.as_ref().is_some_and(|offer| {
                    offer.id == *transfer_id
                        && offer.peer == peer
                        && offer.requested
                        && pasteboard.change_count().ok() == Some(offer.change_count)
                });
                if !valid {
                    let _ = send_file_clipboard_frame(
                        runtime,
                        &[peer],
                        sequence,
                        ClipboardFileFrame::Cancel {
                            transfer_id: *transfer_id,
                            reason: "粘贴请求已失效".into(),
                        },
                    )
                    .await;
                }
                !valid
            }
            ClipboardFileFrame::Request { transfer_id } => {
                if cancellations.contains_key(transfer_id) {
                    return true;
                }
                diagnostics.record(
                    DiagnosticLevel::Info,
                    "clipboard",
                    "file_request_received",
                    Some(format!("peer={} transfer={transfer_id}", peer.to_hex())),
                    None,
                );
                let offer = self
                    .outgoing
                    .lock()
                    .ok()
                    .and_then(|store| store.entries.get(transfer_id).cloned());
                if let Some(offer) = offer
                    .filter(|offer| offer.peer == peer && offer.created.elapsed() < OFFER_LIFETIME)
                {
                    self.pin_transfer(
                        *transfer_id,
                        &offer
                            .source
                            .roots
                            .iter()
                            .map(|path| path.to_string_lossy().into_owned())
                            .collect::<Vec<_>>(),
                    );
                    let started = start_local_file_clipboard(
                        offer
                            .source
                            .roots
                            .iter()
                            .map(|p| p.to_string_lossy().into_owned())
                            .collect(),
                        Some(peer),
                        FileClipboardStartContext {
                            runtime,
                            sequence,
                            ui_sender: ui,
                            diagnostics,
                            acknowledgements,
                            transfer_id: Some(*transfer_id),
                            expected_source: Some(offer.source),
                        },
                        cancellations,
                        sources,
                        task_events,
                    );
                    if started.is_none() {
                        self.unpin_transfer(*transfer_id);
                    }
                } else {
                    let _ = send_file_clipboard_frame(
                        runtime,
                        &[peer],
                        sequence,
                        ClipboardFileFrame::Cancel {
                            transfer_id: *transfer_id,
                            reason: "源剪贴板已失效，请重新复制文件".into(),
                        },
                    )
                    .await;
                }
                true
            }
            _ => false,
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub async fn tick(
        &mut self,
        pasteboard: &impl Pasteboard,
        flag: &AtomicBool,
        requested: &AtomicBool,
        target: &AtomicI32,
        runtime: &RuntimeHandle,
        sequence: &ClipboardSequence,
        ui: &UnboundedSender<UiEvent>,
        incoming: &mut HashMap<(DeviceId, TransferId), IncomingFileClipboard>,
    ) {
        let Some(offer) = self.pending.as_ref() else {
            return;
        };
        if !flag.load(Ordering::Acquire) {
            self.pending = None;
            return;
        }
        // Receiving chunks do not change the pasteboard; a changed generation
        // always means that this paste intent has been superseded.
        let invalid = pasteboard.change_count().ok() != Some(offer.change_count);
        let disconnected =
            !trusted_connected_peer_ids(&runtime.snapshot_view()).contains(&offer.peer);
        let expired = offer.created.elapsed() >= OFFER_LIFETIME;
        if invalid || disconnected || expired {
            let reason = if invalid {
                "剪贴板已被新内容替换"
            } else if disconnected {
                "源设备已断开，请连接后重新复制"
            } else {
                "文件剪贴板已过期，请重新复制"
            };
            self.cancel(
                offer.id, reason, flag, requested, target, runtime, sequence, ui, incoming,
            )
            .await;
            return;
        }
        if !offer.requested
            && (requested.load(Ordering::Acquire)
                || self.workflow_demand == Some(offer.id)
                || self.settings.should_prefetch(offer.total_bytes))
        {
            let offer = self.pending.as_mut().expect("checked pending");
            if offer.total_bytes > self.settings.cache_max_bytes {
                let id = offer.id;
                self.cancel(
                    id,
                    "文件超过缓存上限，请在设置中调整后重新复制",
                    flag,
                    requested,
                    target,
                    runtime,
                    sequence,
                    ui,
                    incoming,
                )
                .await;
                return;
            }
            if !requested.load(Ordering::Acquire)
                && self.workflow_demand != Some(offer.id)
                && self.cache.snapshot().is_ok_and(|snapshot| {
                    offer
                        .total_bytes
                        .saturating_add(snapshot.active_reserved_bytes)
                        > snapshot.max_bytes
                })
            {
                return;
            }
            offer.requested = true;
            emit_receive_transfer_status(
                ui,
                offer.id,
                offer.name.clone(),
                "已请求源设备，正在准备文件…".into(),
                TransferState::Preparing,
                0,
                offer.total_bytes,
            );
            if let Err(error) = send_file_clipboard_frame(
                runtime,
                &[offer.peer],
                sequence,
                ClipboardFileFrame::Request {
                    transfer_id: offer.id,
                },
            )
            .await
            {
                let id = offer.id;
                self.cancel(
                    id, &error, flag, requested, target, runtime, sequence, ui, incoming,
                )
                .await;
            }
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub async fn cancel(
        &mut self,
        id: TransferId,
        reason: &str,
        flag: &AtomicBool,
        requested: &AtomicBool,
        target: &AtomicI32,
        runtime: &RuntimeHandle,
        sequence: &ClipboardSequence,
        ui: &UnboundedSender<UiEvent>,
        incoming: &mut HashMap<(DeviceId, TransferId), IncomingFileClipboard>,
    ) {
        if !self.pending.as_ref().is_some_and(|offer| offer.id == id) {
            return;
        }
        let offer = self.pending.take().expect("matching pending");
        if let Some(state) = incoming.get_mut(&(offer.peer, id)) {
            state.cancel();
        } else {
            self.cache.finish(id);
        }
        flag.store(false, Ordering::Release);
        requested.store(false, Ordering::Release);
        target.store(0, Ordering::Release);
        let _ = send_file_clipboard_frame(
            runtime,
            &[offer.peer],
            sequence,
            ClipboardFileFrame::Cancel {
                transfer_id: id,
                reason: reason.chars().take(200).collect(),
            },
        )
        .await;
        emit_receive_transfer_status(
            ui,
            id,
            offer.name,
            reason.into(),
            TransferState::Cancelled,
            0,
            offer.total_bytes,
        );
        self.emit(ui, Some(reason.into()));
    }
    #[allow(clippy::too_many_arguments)]
    pub async fn expire_receivers(
        &mut self,
        runtime: &RuntimeHandle,
        sequence: &ClipboardSequence,
        ui: &UnboundedSender<UiEvent>,
        incoming: &mut HashMap<(DeviceId, TransferId), IncomingFileClipboard>,
        pasteboard: &impl Pasteboard,
        flag: &AtomicBool,
        requested: &AtomicBool,
        target: &AtomicI32,
    ) {
        let peers = trusted_connected_peer_ids(&runtime.snapshot_view());
        let expired = incoming
            .iter()
            .filter(|((peer, _), state)| {
                state.last_activity.elapsed() > Duration::from_secs(120) || !peers.contains(peer)
            })
            .map(|(key, _)| *key)
            .collect::<Vec<_>>();
        for key in expired {
            if self.pending.as_ref().is_some_and(|offer| offer.id == key.1) {
                self.cancel(
                    key.1,
                    "接收连接中断或超时，请重新复制",
                    flag,
                    requested,
                    target,
                    runtime,
                    sequence,
                    ui,
                    incoming,
                )
                .await;
            } else if let Some(state) = incoming.get_mut(&key) {
                let name = state.name.clone();
                let total = state.manifest.total_bytes();
                let received = state.received_bytes;
                if pasteboard.change_count().ok() == Some(state.pasteboard_change_count) {
                    flag.store(false, Ordering::Release);
                    requested.store(false, Ordering::Release);
                    target.store(0, Ordering::Release);
                }
                state.cancel();
                let _ = send_file_clipboard_frame(
                    runtime,
                    &[key.0],
                    sequence,
                    ClipboardFileFrame::Cancel {
                        transfer_id: key.1,
                        reason: "接收连接中断或超时".into(),
                    },
                )
                .await;
                emit_receive_transfer_status(
                    ui,
                    key.1,
                    name,
                    "接收连接中断或超时，等待磁盘任务退出".into(),
                    TransferState::Failed,
                    received,
                    total,
                );
                self.emit(ui, None);
            }
        }
    }
    /// Keep current clipboard files and a bounded grace period for Finder's
    /// asynchronous copy. This is conservative retention, not a Finder-complete
    /// signal; native paste completion still requires physical acceptance.
    pub fn protect_clipboard(&mut self, pasteboard: &impl Pasteboard) {
        let count = pasteboard.change_count().ok();
        if count != self.clipboard_change_count {
            let Ok(snapshot) = pasteboard.read() else {
                return;
            };
            self.clipboard_change_count = count;
            let paths = snapshot.file_urls;
            if self.clipboard_paths != paths {
                if !self.clipboard_pins.is_empty() {
                    self.paste_leases
                        .push((Instant::now(), std::mem::take(&mut self.clipboard_pins)));
                }
                self.clipboard_pins = self.cache.pin_sources(&paths);
                self.clipboard_paths = paths;
            }
        }
        let mut retained = Vec::new();
        for (created, pins) in self.paste_leases.drain(..) {
            if created.elapsed() >= Duration::from_secs(120) {
                self.cache.unpin_sources(&pins);
            } else {
                retained.push((created, pins));
            }
        }
        self.paste_leases = retained;
    }
    pub fn pin_transfer(&mut self, id: TransferId, paths: &[String]) {
        let pins = self.cache.pin_sources(paths);
        self.outgoing_pins.insert(id, pins);
    }
    pub fn unpin_transfer(&mut self, id: TransferId) {
        if let Some(pins) = self.outgoing_pins.remove(&id) {
            self.cache.unpin_sources(&pins);
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub async fn cancel_pending(
        &mut self,
        flag: &AtomicBool,
        requested: &AtomicBool,
        target: &AtomicI32,
        runtime: &RuntimeHandle,
        sequence: &ClipboardSequence,
        ui: &UnboundedSender<UiEvent>,
        incoming: &mut HashMap<(DeviceId, TransferId), IncomingFileClipboard>,
    ) {
        if let Some(id) = self.pending.as_ref().map(|offer| offer.id) {
            self.cancel(
                id,
                "剪贴板同步已关闭",
                flag,
                requested,
                target,
                runtime,
                sequence,
                ui,
                incoming,
            )
            .await;
        }
        if let Ok(mut sources) = self.outgoing.lock() {
            sources.clear();
        }
    }
    pub fn finished(&mut self, id: TransferId) {
        if self.pending.as_ref().is_some_and(|offer| offer.id == id) {
            self.pending = None;
        }
    }
}

pub(super) fn register_offers(
    store: &mut DeferredSourceStore,
    source: &DeferredClipboardSource,
    peers: impl IntoIterator<Item = DeviceId>,
) -> Vec<(DeviceId, TransferId)> {
    peers
        .into_iter()
        .map(|peer| {
            let id = TransferId::new();
            store.entries.insert(
                id,
                DeferredOffer {
                    source: source.clone(),
                    peer,
                    created: Instant::now(),
                },
            );
            (peer, id)
        })
        .collect()
}

pub(super) fn apply_cleanup(
    cleanup: CacheCleanup,
    history: &mut ClipboardHistoryStore,
    pasteboard: &impl Pasteboard,
    last_change: &mut Option<i64>,
    ui: &UnboundedSender<UiEvent>,
) -> String {
    let old_len = history.items.len();
    history.items.retain(|(_, snapshot)| {
        !snapshot.file_urls.iter().any(|path| {
            cleanup
                .removed_paths
                .iter()
                .any(|root| PathBuf::from(path).starts_with(root))
        })
    });
    if old_len != history.items.len() {
        history.emit(ui);
    }
    if pasteboard.read().ok().is_some_and(|snapshot| {
        snapshot.file_urls.iter().any(|path| {
            cleanup
                .removed_paths
                .iter()
                .any(|root| PathBuf::from(path).starts_with(root))
        })
    }) {
        let _ = pasteboard.clear_for_remote_file("unidesk-transfer-cache-cleared");
        *last_change = pasteboard.change_count().ok();
    }
    format!(
        "已清除 {} 组历史文件，释放 {} 字节；正在使用的文件已保留，{} 组清理失败",
        cleanup.removed_paths.len(),
        cleanup.removed_bytes,
        cleanup.failed_transfers
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use unidesk_platform::MemoryPasteboard;
    use unidesk_runtime::{DiscoveryEvent, MemoryDiscovery, MemoryTransport, PeerMessage};

    async fn connected_runtime() -> (RuntimeHandle, MemoryTransport, DeviceId) {
        let (runtime, transport, peers) = connected_peers(&[Capabilities::NETWORK
            | Capabilities::CLIPBOARD
            | Capabilities::TRANSFER
            | Capabilities::DEFERRED_FILE_CLIPBOARD])
        .await;
        (runtime, transport, peers[0])
    }
    async fn connected_peers(
        capabilities: &[Capabilities],
    ) -> (RuntimeHandle, MemoryTransport, Vec<DeviceId>) {
        let discovery = MemoryDiscovery::new();
        let transport = MemoryTransport::new();
        let runtime = RuntimeBuilder::new()
            .with_discovery(discovery.clone())
            .with_transport(transport.clone())
            .build()
            .unwrap();
        let mut events = runtime.events();
        let mut peers = Vec::new();
        for (index, capabilities) in capabilities.iter().copied().enumerate() {
            let identity = MemoryIdentityProvider::new().load_or_create().unwrap();
            let mut hello = unidesk_protocol::Hello {
                device_id: identity.device_id().as_bytes().to_vec(),
                display_name: format!("file-test-peer-{index}"),
                public_key: identity.verifying_key_bytes().to_vec(),
                nonce: vec![3; 32],
                signature: Vec::new(),
                capabilities: capabilities.bits(),
            };
            hello.signature = identity
                .sign(&unidesk_protocol::canonical_hello_payload(&hello))
                .as_bytes()
                .to_vec();
            let peer = identity.device_id();
            peers.push(peer);
            transport.set_remote_hello(peer, hello.clone());
            discovery.push(DiscoveryEvent::PeerDiscovered {
                hello,
                routes: vec![unidesk_core::RouteCandidate::new(
                    unidesk_core::Endpoint::new("127.0.0.1", 4321 + index as u16).unwrap(),
                    unidesk_core::RouteSource::Mdns,
                    unidesk_core::RouteHealth::Healthy,
                    capabilities,
                )],
            });
            tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    if let Ok(RuntimeEvent::PairingRequest(request)) = events.recv().await {
                        runtime
                            .send(RuntimeCommand::ApprovePairing {
                                request_id: request.request_id(),
                            })
                            .await
                            .unwrap();
                        break;
                    }
                }
                while !trusted_connected_peer_ids(&runtime.snapshot_view()).contains(&peer) {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
        }
        transport.clear_sent();
        (runtime, transport, peers)
    }
    #[test]
    fn startup_eviction_protects_files_already_on_the_system_clipboard() {
        let mut policy = FilePolicy::new().unwrap();
        policy.settings.cache_max_bytes = 8;
        let current = policy.cache.destination(TransferId::new());
        let old = policy.cache.destination(TransferId::new());
        for path in [&current, &old] {
            std::fs::create_dir(path).unwrap();
            std::fs::write(path.join("file"), [0; 8]).unwrap();
        }
        let pasteboard = MemoryPasteboard::new();
        pasteboard
            .write_file_urls(&[current.join("file").to_string_lossy().into_owned()])
            .unwrap();
        let (cleanup, error) = policy.initialize_cache(&pasteboard);
        assert_eq!(error, None);
        assert_eq!(cleanup.removed_paths, vec![old]);
        assert!(current.join("file").exists());
        assert_eq!(policy.cache.snapshot().unwrap().used_bytes, 8);
        let root = policy.cache.root().to_path_buf();
        drop(policy);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn independent_peer_offers_can_start_together_and_cancel_separately() {
        let caps = Capabilities::NETWORK
            | Capabilities::CLIPBOARD
            | Capabilities::TRANSFER
            | Capabilities::DEFERRED_FILE_CLIPBOARD;
        let (runtime, _, peers) = connected_peers(&[caps, caps]).await;
        let mut policy = FilePolicy::new().unwrap();
        let source_path = policy.cache.root().join("external-source.txt");
        std::fs::write(&source_path, b"clipboard source").unwrap();
        let source = DeferredClipboardSource::inspect(vec![source_path]).unwrap();
        let offers = register_offers(&mut policy.outgoing.lock().unwrap(), &source, peers.clone());
        assert_ne!(offers[0].1, offers[1].1);
        let pasteboard = MemoryPasteboard::new();
        let mut last = None;
        let pending = AtomicBool::new(false);
        let requested = AtomicBool::new(false);
        let target = AtomicI32::new(0);
        let sequence = ClipboardSequence::default();
        let (ui, _) = unbounded();
        let diagnostics = Diagnostics::new(8);
        let mut cancellations = HashMap::new();
        let mut sources = HashMap::new();
        let acknowledgements = Arc::default();
        let (tasks, mut finished) = unbounded_channel();
        for (peer, id) in &offers {
            policy
                .control(
                    &ClipboardFileFrame::Request { transfer_id: *id },
                    *peer,
                    &pasteboard,
                    &mut last,
                    &pending,
                    &requested,
                    &target,
                    &runtime,
                    &sequence,
                    &ui,
                    &diagnostics,
                    &mut cancellations,
                    &mut sources,
                    &acknowledgements,
                    &tasks,
                )
                .await;
        }
        assert_eq!(
            cancellations.len(),
            2,
            "second peer must not be mistaken for a duplicate paste"
        );
        assert_eq!(sources[&offers[0].1].target_peer, Some(peers[0]));
        assert_eq!(sources[&offers[1].1].target_peer, Some(peers[1]));
        cancellations[&offers[0].1].store(true, Ordering::Release);
        assert!(!cancellations[&offers[1].1].load(Ordering::Acquire));
        cancellations[&offers[1].1].store(true, Ordering::Release);
        tokio::time::timeout(Duration::from_secs(2), async {
            for _ in 0..2 {
                finished.recv().await.unwrap();
            }
        })
        .await
        .unwrap();
        runtime.shutdown_and_wait().await.unwrap();
        let root = policy.cache.root().to_path_buf();
        drop(policy);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn deferred_offer_allocates_nothing_and_repeated_paste_sends_one_request() {
        let (runtime, transport, peer) = connected_runtime().await;
        let mut policy = FilePolicy::new().unwrap();
        let pasteboard = MemoryPasteboard::new();
        let pending = AtomicBool::new(false);
        let requested = AtomicBool::new(false);
        let target = AtomicI32::new(0);
        let mut last = None;
        let sequence = ClipboardSequence::default();
        let (ui, mut events) = unbounded();
        let diagnostics = Diagnostics::new(8);
        let mut cancellations = HashMap::new();
        let mut sources = HashMap::new();
        let ack = Arc::default();
        let (tasks, _) = unbounded_channel();
        let mut incoming = HashMap::new();
        let id = TransferId::new();
        let frame = ClipboardFileFrame::Deferred {
            transfer_id: id,
            names: vec!["large-folder".into()],
            total_bytes: 400 * 1024 * 1024,
        };
        assert!(
            policy
                .control(
                    &frame,
                    peer,
                    &pasteboard,
                    &mut last,
                    &pending,
                    &requested,
                    &target,
                    &runtime,
                    &sequence,
                    &ui,
                    &diagnostics,
                    &mut cancellations,
                    &mut sources,
                    &ack,
                    &tasks
                )
                .await
        );
        for _ in 0..3 {
            policy
                .tick(
                    &pasteboard,
                    &pending,
                    &requested,
                    &target,
                    &runtime,
                    &sequence,
                    &ui,
                    &mut incoming,
                )
                .await;
        }
        assert!(transport.sent_messages().is_empty());
        assert_eq!(policy.cache.snapshot().unwrap().used_bytes, 0);
        assert_eq!(fs_entries(policy.cache.root()), 0);
        assert!(
            !std::iter::from_fn(|| events.try_recv().ok())
                .any(|event| matches!(event, UiEvent::TransferStatus { .. }))
        );
        requested.store(true, Ordering::Release);
        target.store(42, Ordering::Release);
        for _ in 0..10 {
            policy
                .tick(
                    &pasteboard,
                    &pending,
                    &requested,
                    &target,
                    &runtime,
                    &sequence,
                    &ui,
                    &mut incoming,
                )
                .await;
        }
        tokio::time::timeout(Duration::from_secs(1), async {
            while transport.sent_messages().is_empty() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let requests=transport.sent_messages().into_iter().filter(|(_,message)|matches!(message,PeerMessage::Clipboard(payload) if matches!(ClipboardFileFrame::decode(&payload.data),Ok(ClipboardFileFrame::Request {transfer_id}) if transfer_id==id))).count();
        assert_eq!(requests, 1);
        assert_eq!(fs_entries(policy.cache.root()), 0);
        let request_generation = policy.pending.as_ref().unwrap().change_count;
        pasteboard.write_text("new copied text").unwrap();
        assert_ne!(pasteboard.change_count().unwrap(), request_generation);
        policy
            .tick(
                &pasteboard,
                &pending,
                &requested,
                &target,
                &runtime,
                &sequence,
                &ui,
                &mut incoming,
            )
            .await;
        assert!(!pending.load(Ordering::Acquire));
        assert!(!requested.load(Ordering::Acquire));
        assert_eq!(target.load(Ordering::Acquire), 0);
        assert_eq!(
            pasteboard.read().unwrap().text.as_deref(),
            Some("new copied text")
        );
        runtime.shutdown_and_wait().await.unwrap();
        let root = policy.cache.root().to_path_buf();
        drop(policy);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn current_clipboard_pin_and_recent_paste_lease_protect_cache() {
        let mut policy = FilePolicy::new().unwrap();
        let pasteboard = MemoryPasteboard::new();
        let id = TransferId::new();
        std::fs::create_dir(policy.cache.destination(id)).unwrap();
        let file = policy.cache.destination(id).join("file.txt");
        std::fs::write(&file, b"keep for paste").unwrap();
        pasteboard
            .write_file_urls(&[file.to_string_lossy().into_owned()])
            .unwrap();
        policy.protect_clipboard(&pasteboard);
        assert_eq!(policy.cache.clear_completed().unwrap().removed_bytes, 0);
        pasteboard.write_text("next clipboard").unwrap();
        policy.protect_clipboard(&pasteboard);
        assert_eq!(policy.cache.clear_completed().unwrap().removed_bytes, 0);
        policy.paste_leases[0].0 = Instant::now() - Duration::from_secs(121);
        policy.protect_clipboard(&pasteboard);
        assert_eq!(policy.cache.clear_completed().unwrap().removed_bytes, 14);
        let root = policy.cache.root().to_path_buf();
        drop(policy);
        std::fs::remove_dir_all(root).unwrap();
    }
    fn fs_entries(path: &std::path::Path) -> usize {
        std::fs::read_dir(path).unwrap().count()
    }
}
