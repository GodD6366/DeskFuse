//! Per-connection clipboard epochs, latest-only sends, and idempotent receipts.
use super::*;
use deskfuse_platform::PasteboardItem;
use deskfuse_protocol::clipboard_sync as wire;
use std::time::Instant;
use wire::SyncMessage;

type Epoch = Vec<u8>;
fn epoch() -> Epoch {
    deskfuse_core::RequestId::new()
        .as_uuid()
        .as_bytes()
        .to_vec()
}
struct Job {
    id: Vec<u8>,
    revision: u64,
    ack: tokio::sync::mpsc::Sender<u32>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.task.abort();
    }
}
struct Incoming {
    frame: SyncMessage,
    baseline: i64,
    data: Vec<u8>,
    processing: bool,
    started: Instant,
}
struct Peer {
    decode: Option<tokio::task::JoinHandle<()>>,
    round: u64,
    pending: Option<Arc<ClipboardPayload>>,
    generation: u64,
    epoch: Epoch,
    remote: Option<Epoch>,
    ready: bool,
    baseline: i64,
    hello_due: Instant,
    job: Option<Job>,
    incoming: Option<Incoming>,
    receipt: Option<(u64, Vec<u8>, u32)>,
    /// Receive budget the remote advertised on its HELLO; older peers get
    /// the full wire limit until they prove otherwise.
    max_content: u64,
}
impl Drop for Peer {
    fn drop(&mut self) {
        if let Some(task) = &self.decode {
            task.abort();
        }
    }
}
impl Peer {
    fn should_send(&self, count: i64, force: bool) -> bool {
        force || self.baseline != count
    }
    fn accepts(&self, frame: &SyncMessage) -> bool {
        self.ready
            && frame.target_epoch == self.epoch
            && self.remote.as_ref() == Some(&frame.source_epoch)
    }
    fn duplicate(&self, frame: &SyncMessage) -> Option<u32> {
        self.receipt
            .as_ref()
            .filter(|(revision, _, _)| frame.revision <= *revision)
            .map(|(revision, id, outcome)| {
                if frame.revision == *revision && frame.sync_id == *id {
                    *outcome
                } else {
                    wire::SUPERSEDED
                }
            })
    }
}
pub(super) struct Ready {
    pub peer: DeviceId,
    pub payload: ClipboardPayload,
    pub items: Option<Vec<PasteboardItem>>,
    pub metadata: clipboard_bundle::SourceMetadata,
    frame: SyncMessage,
}
enum Prepared {
    Native(ClipboardPayload, Option<String>),
    Files(DeferredClipboardSource, crate::workflow_sharing::Selected),
}
enum Work {
    Prepared {
        revision: u64,
        targets: Vec<(DeviceId, u64, Epoch, u64)>,
        change_count: i64,
        explicit: bool,
        copy_generation: u64,
        result: Result<Prepared, String>,
    },
    Decoded {
        peer: DeviceId,
        frame: SyncMessage,
        baseline: i64,
        result: Result<(ClipboardPayload, Option<Vec<PasteboardItem>>), String>,
    },
    Sent {
        peer: DeviceId,
        epoch: Epoch,
        id: Vec<u8>,
        revision: u64,
        result: Result<(), String>,
    },
}
pub(super) struct Sync {
    cpu: Arc<tokio::sync::Semaphore>,
    peers: HashMap<DeviceId, Peer>,
    revision: u64,
    metadata: HashMap<u64, clipboard_bundle::SourceMetadata>,
    copy_generation: u64,
    disconnected: Vec<DeviceId>,
    preparations: Vec<tokio::task::JoinHandle<()>>,
    prepare_cancels: Vec<Arc<AtomicBool>>,
    warnings: HashMap<DeviceId, String>,
    manual: HashMap<DeviceId, (TransferId, Arc<AtomicBool>)>,
    finished: Vec<TransferId>,
    tx: TokioSender<Work>,
    rx: TokioReceiver<Work>,
    helpers: Vec<tokio::task::JoinHandle<()>>,
}
impl Default for Sync {
    fn default() -> Self {
        let (tx, rx) = unbounded_channel();
        Self {
            cpu: Arc::new(tokio::sync::Semaphore::new(4)),
            peers: HashMap::new(),
            revision: 0,
            metadata: HashMap::new(),
            copy_generation: 0,
            disconnected: Vec::new(),
            preparations: Vec::new(),
            prepare_cancels: Vec::new(),
            warnings: HashMap::new(),
            manual: HashMap::new(),
            finished: Vec::new(),
            tx,
            rx,
            helpers: Vec::new(),
        }
    }
}
impl Drop for Sync {
    fn drop(&mut self) {
        for cancel in &self.prepare_cancels {
            cancel.store(true, Ordering::Release);
        }
        for task in &self.preparations {
            task.abort();
        }
        for task in &self.helpers {
            task.abort();
        }
    }
}
impl Sync {
    pub fn take_finished(&mut self) -> Vec<TransferId> {
        std::mem::take(&mut self.finished)
    }
    fn finish_manual(
        &mut self,
        peer: DeviceId,
        result: Result<(), String>,
        ui: &UnboundedSender<UiEvent>,
    ) {
        if let Some((id, cancel)) = self.manual.remove(&peer) {
            let state = if cancel.load(Ordering::Acquire) {
                TransferState::Cancelled
            } else if result.is_ok() {
                TransferState::Completed
            } else {
                TransferState::Failed
            };
            let detail = if cancel.load(Ordering::Acquire) {
                "共享已取消".into()
            } else {
                result.err().unwrap_or_else(|| "已共享到目标设备".into())
            };
            emit_transfer_status(ui, id, "工作流共享".into(), detail, state, 0, 0);
            self.finished.push(id);
        }
    }
    #[allow(clippy::too_many_arguments)] // Tracking context is explicit at each call site.
    pub fn publish_tracked(
        &mut self,
        snapshot: PasteboardSnapshot,
        peer: DeviceId,
        selection: crate::workflow_sharing::Selection,
        id: TransferId,
        cancel: Arc<AtomicBool>,
        runtime: &RuntimeHandle,
        ui: &UnboundedSender<UiEvent>,
    ) {
        if let Some((_, previous_cancel)) = self.manual.get(&peer) {
            previous_cancel.store(true, Ordering::Release);
            self.finish_manual(peer, Err("已被该设备的新共享替换".into()), ui);
        }
        self.manual.insert(peer, (id, cancel));
        if !self.peers.contains_key(&peer) {
            self.finish_manual(peer, Err("设备尚未建立可用的共享会话".into()), ui);
            return;
        }
        self.publish_to(snapshot, peer, selection, runtime, ui);
    }
    pub fn take_disconnected(&mut self) -> Vec<DeviceId> {
        std::mem::take(&mut self.disconnected)
    }
    pub fn reset(&mut self, ui: &UnboundedSender<UiEvent>) {
        self.disconnected.extend(self.peers.keys().copied());
        self.peers.clear();
        self.invalidate_copy(ui);
    }
    pub fn invalidate_copy(&mut self, ui: &UnboundedSender<UiEvent>) {
        let manual_peers = self.manual.keys().copied().collect::<Vec<_>>();
        for peer in manual_peers {
            if let Some((_, cancel)) = self.manual.get(&peer) {
                cancel.store(true, Ordering::Release);
            }
            self.finish_manual(peer, Err("已被新复制替换".into()), ui);
        }
        self.revision = self.revision.saturating_add(1);
        self.copy_generation = self.copy_generation.saturating_add(1);
        for cancel in self.prepare_cancels.drain(..) {
            cancel.store(true, Ordering::Release);
        }
        for task in self.preparations.drain(..) {
            task.abort();
        }
        self.warnings.clear();
        for peer in self.peers.values_mut() {
            if let Some(task) = peer.decode.take() {
                task.abort();
            }
            peer.job = None;
            peer.pending = None;
            if let Some(incoming) = peer.incoming.take() {
                peer.receipt = Some((
                    incoming.frame.revision,
                    incoming.frame.sync_id,
                    wire::SUPERSEDED,
                ));
            }
        }
    }
    fn status(ui: &UnboundedSender<UiEvent>, peer: DeviceId, phase: &str) {
        let _ = ui.unbounded_send(UiEvent::ClipboardDelivery {
            peer_id: peer,
            phase: phase.to_owned(),
        });
    }
    pub fn refresh(
        &mut self,
        runtime: &RuntimeHandle,
        count: i64,
        _enabled: bool,
        ui: &UnboundedSender<UiEvent>,
    ) {
        let view = runtime.snapshot_view();
        let connected = trusted_connected_peer_ids(&view);
        let removed = self
            .peers
            .keys()
            .filter(|peer| {
                !connected.contains(peer)
                    || runtime.control_generation(**peer) != Some(self.peers[peer].generation)
            })
            .copied()
            .collect::<Vec<_>>();
        for peer in removed {
            self.disconnected.push(peer);
            self.peers.remove(&peer);
            if let Some((_, cancel)) = self.manual.get(&peer) {
                cancel.store(true, Ordering::Release);
            }
            self.finish_manual(peer, Err("设备已断开".into()), ui);
            Self::status(ui, peer, "已断开，等待新复制");
        }
        for peer in connected {
            let capable = view
                .snapshot
                .peers
                .iter()
                .find(|p| p.id == peer)
                .is_some_and(|p| {
                    p.selected_route.is_some_and(|id| {
                        p.routes.iter().any(|r| {
                            r.id == id && r.capabilities.contains(Capabilities::CLIPBOARD_SYNC_V2)
                        })
                    })
                });
            if !capable {
                Self::status(ui, peer, "需要两端更新后同步");
                continue;
            }
            let Some(generation) = runtime.control_generation(peer) else {
                continue;
            };
            let state = self.peers.entry(peer).or_insert_with(|| Peer {
                decode: None,
                round: 0,
                pending: None,
                generation,
                epoch: epoch(),
                remote: None,
                ready: false,
                baseline: count,
                hello_due: Instant::now(),
                job: None,
                incoming: None,
                receipt: None,
                max_content: wire::MAX_BYTES as u64,
            });
            if !state.ready && Instant::now() >= state.hello_due {
                state.hello_due = Instant::now() + Duration::from_secs(1);
                let hello = SyncMessage::hello(
                    state.epoch.clone(),
                    state.remote.clone().unwrap_or_default(),
                    wire::MAX_BYTES as u64,
                );
                self.control(runtime, peer, hello);
            }
        }
        let retired = self
            .manual
            .iter()
            .filter(|(peer, (_, cancel))| {
                cancel.load(Ordering::Acquire) || !self.peers.contains_key(peer)
            })
            .map(|(peer, _)| *peer)
            .collect::<Vec<_>>();
        for peer in retired {
            if let Some(p) = self.peers.get_mut(&peer) {
                p.round = p.round.saturating_add(1);
                p.job = None;
                p.pending = None;
            }
            self.finish_manual(peer, Err("共享已取消或设备已断开".into()), ui);
        }
        self.helpers.retain(|t| !t.is_finished());
    }
    fn control(&mut self, runtime: &RuntimeHandle, peer: DeviceId, message: SyncMessage) {
        let Some(generation) = runtime.control_generation(peer) else {
            return;
        };
        let Ok(data) = message.encode_checked() else {
            return;
        };
        let Ok(payload) = ClipboardPayload::new(wire::MIME, data, message.revision) else {
            return;
        };
        let runtime = runtime.clone();
        // Limit helpers even if a peer floods control frames. A lost ACK is retried.
        self.helpers.retain(|t| !t.is_finished());
        if self.helpers.len() >= 64 {
            return;
        }
        self.helpers.push(tokio::spawn(async move {
            let _ = runtime
                .send_clipboard_on_session(peer, generation, payload)
                .await;
        }));
    }
    pub fn publish(
        &mut self,
        snapshot: PasteboardSnapshot,
        force: bool,
        runtime: &RuntimeHandle,
        ui: &UnboundedSender<UiEvent>,
    ) {
        self.invalidate_copy(ui);
        self.prepare(
            snapshot,
            None,
            crate::workflow_sharing::Selection::Automatic,
            force,
            runtime,
            ui,
        );
    }
    /// Explicit sharing neither writes the source pasteboard nor cancels another target.
    pub fn publish_to(
        &mut self,
        snapshot: PasteboardSnapshot,
        peer: DeviceId,
        selection: crate::workflow_sharing::Selection,
        runtime: &RuntimeHandle,
        ui: &UnboundedSender<UiEvent>,
    ) {
        self.revision = self.revision.saturating_add(1);
        if let Some(state) = self.peers.get_mut(&peer) {
            state.round = state.round.saturating_add(1);
            state.job = None;
            state.pending = None;
        }
        self.prepare(snapshot, Some(peer), selection, true, runtime, ui);
    }
    fn prepare(
        &mut self,
        snapshot: PasteboardSnapshot,
        target: Option<DeviceId>,
        selection: crate::workflow_sharing::Selection,
        force: bool,
        _runtime: &RuntimeHandle,
        ui: &UnboundedSender<UiEvent>,
    ) {
        let revision = self.revision;
        self.metadata
            .insert(revision, clipboard_bundle::source_metadata(&snapshot));
        self.metadata
            .retain(|key, _| key.saturating_add(256) >= revision);
        let copy_generation = self.copy_generation;
        let explicit = target.is_some();
        let targets = self
            .peers
            .iter()
            .filter(|(id, p)| {
                target.is_none_or(|target| target == **id)
                    && p.should_send(snapshot.change_count, force)
            })
            .map(|(id, p)| (*id, p.generation, p.epoch.clone(), p.round))
            .collect::<Vec<_>>();
        if targets.is_empty() {
            if let Some(peer) = target {
                Self::status(ui, peer, "设备尚未建立可用的共享会话");
            }
            return;
        }
        self.preparations.retain(|task| !task.is_finished());
        for target in targets {
            let selected = crate::workflow_sharing::select(
                &target.0.to_hex(),
                crate::workflow_sharing::content_type(&snapshot),
                &selection,
            );
            // A source image hash cannot describe a workflow's output, and an
            // explicit file send must retain its requested file semantics.
            if (explicit || selected.workflow.is_some())
                && let Some(metadata) = self.metadata.get_mut(&revision)
            {
                metadata.image_digest = None;
            }
            Self::status(
                ui,
                target.0,
                if selected.workflow.is_some() {
                    "正在执行共享工作流…"
                } else if !snapshot.file_urls.is_empty() {
                    "正在读取文件目录…"
                } else {
                    "排队中"
                },
            );
            let cpu = self.cpu.clone();
            let tx = self.tx.clone();
            let snapshot = snapshot.clone();
            let cancel = if explicit {
                self.manual
                    .get(&target.0)
                    .map(|(_, cancel)| cancel.clone())
                    .unwrap_or_else(|| Arc::new(AtomicBool::new(false)))
            } else {
                Arc::new(AtomicBool::new(false))
            };
            self.prepare_cancels.push(cancel.clone());
            let change_count = snapshot.change_count;
            self.preparations.push(tokio::spawn(async move {
                let Ok(permit) = cpu.acquire_owned().await else {
                    return;
                };
                let result = tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    if cancel.load(Ordering::Acquire) {
                        return Err("共享已取消".into());
                    }
                    if !snapshot.file_urls.is_empty() {
                        return DeferredClipboardSource::inspect_cancellable(
                            snapshot.file_urls.iter().map(PathBuf::from).collect(),
                            || cancel.load(Ordering::Acquire),
                        )
                        .map(|source| Prepared::Files(source, selected))
                        .map_err(|e| e.to_string());
                    }
                    let processed =
                        crate::workflow_sharing::process(selected, snapshot, cancel.clone())?;
                    if cancel.load(Ordering::Acquire) {
                        return Err("共享已取消".into());
                    }
                    let snapshot = processed.snapshot;
                    if !snapshot.file_urls.is_empty() {
                        // Native-to-file output is already prepared; never run it again.
                        return DeferredClipboardSource::inspect_cancellable(
                            snapshot.file_urls.iter().map(PathBuf::from).collect(),
                            || cancel.load(Ordering::Acquire),
                        )
                        .map(|source| {
                            Prepared::Files(source, crate::workflow_sharing::Selected::default())
                        })
                        .map_err(|e| e.to_string());
                    }
                    let payload = encode_snapshot(snapshot, revision)?;
                    Ok(Prepared::Native(payload, processed.warning))
                })
                .await
                .map_err(|_| "剪贴板准备任务失败".to_owned())
                .and_then(|r| r);
                let _ = tx.send(Work::Prepared {
                    revision,
                    targets: vec![target],
                    change_count,
                    explicit,
                    copy_generation,
                    result,
                });
            }));
        }
    }
    fn start(
        &mut self,
        peer: DeviceId,
        payload: Arc<ClipboardPayload>,
        runtime: &RuntimeHandle,
        ui: &UnboundedSender<UiEvent>,
    ) {
        let source_metadata = self
            .metadata
            .get(&payload.sequence)
            .cloned()
            .unwrap_or_else(|| clipboard_bundle::SourceMetadata {
                no_history: true,
                ..Default::default()
            });
        let cancel = self.manual.get(&peer).map(|(_, cancel)| cancel.clone());
        if cancel.as_ref().is_some_and(|c| c.load(Ordering::Acquire)) {
            return;
        }
        // Skip the whole send when the peer already advertised a smaller
        // receive budget, instead of streaming chunks for a guaranteed NACK.
        let budget = self.peers.get(&peer).map(|state| state.max_content);
        if let Some(budget) = budget
            && payload.data.len() as u64 > budget
        {
            let error = format!(
                "内容约 {} MB，超过对端可接收上限 {} MB",
                payload.data.len().div_ceil(1024 * 1024),
                budget.div_ceil(1024 * 1024)
            );
            Self::status(ui, peer, &error);
            self.finish_manual(peer, Err(error), ui);
            return;
        }
        let Some(state) = self.peers.get_mut(&peer) else {
            return;
        };
        if !state.ready {
            state.pending = Some(payload);
            return;
        }
        let Some(remote) = state.remote.clone() else {
            return;
        };
        let source = state.epoch.clone();
        let generation = state.generation;
        let id = epoch();
        let revision = self.revision;
        let (ack_tx, mut ack_rx) = tokio::sync::mpsc::channel(4);
        let runtime = runtime.clone();
        let tx = self.tx.clone();
        let job_id = id.clone();
        let job_epoch = source.clone();
        let task = tokio::spawn(async move {
            let digest = tokio::task::spawn_blocking({
                let payload = payload.clone();
                move || Sha256::digest(&payload.data).to_vec()
            })
            .await;
            let result = async {
                let digest = digest.map_err(|_| "剪贴板校验准备失败".to_owned())?;
                for (attempt, wait) in [1, 2, 4, 4].into_iter().enumerate() {
                    if cancel.as_ref().is_some_and(|c| c.load(Ordering::Acquire)) {
                        return Err("共享已取消".into());
                    }
                    if runtime.control_generation(peer) != Some(generation) {
                        return Err("连接已断开，等待新复制".into());
                    }
                    let chunks = payload.data.len().max(1).div_ceil(wire::CHUNK_BYTES);
                    for part in 0..chunks {
                        if cancel.as_ref().is_some_and(|c| c.load(Ordering::Acquire)) {
                            return Err("共享已取消".into());
                        }
                        let offset = part * wire::CHUNK_BYTES;
                        let end = (offset + wire::CHUNK_BYTES).min(payload.data.len());
                        let frame = SyncMessage::data(
                            source.clone(),
                            remote.clone(),
                            job_id.clone(),
                            revision,
                            payload.mime.clone(),
                            digest.clone(),
                            payload.data.len() as u64,
                            offset as u64,
                            payload.data[offset..end].to_vec(),
                            &source_metadata,
                        );
                        let data = frame.encode_checked().map_err(|e| e.to_string())?;
                        runtime
                            .send_clipboard_on_session(
                                peer,
                                generation,
                                ClipboardPayload::new(wire::MIME, data, revision)
                                    .map_err(|e| e.to_string())?,
                            )
                            .await
                            .map_err(|e| e.to_string())?;
                    }
                    match tokio::time::timeout(Duration::from_secs(wait), ack_rx.recv()).await {
                        Ok(Some(wire::APPLIED)) => return Ok(()),
                        Ok(Some(wire::SUPERSEDED)) => return Err("已被新复制替换".into()),
                        Ok(Some(_)) => return Err("对端未能写入剪贴板".into()),
                        Ok(None) => return Err("同步已取消".into()),
                        Err(_) if attempt < 3 => {}
                        Err(_) => return Err("未收到对端确认，请重新复制".into()),
                    }
                }
                unreachable!()
            }
            .await;
            let _ = tx.send(Work::Sent {
                peer,
                epoch: job_epoch,
                id: job_id,
                revision,
                result,
            });
        });
        state.job = Some(Job {
            id,
            revision,
            ack: ack_tx,
            task,
        });
    }
    pub fn receive(
        &mut self,
        peer: DeviceId,
        bytes: &[u8],
        count: i64,
        runtime: &RuntimeHandle,
        ui: &UnboundedSender<UiEvent>,
        accept_content: bool,
    ) {
        let Ok(frame) = SyncMessage::decode_checked(bytes) else {
            return;
        };
        if frame.kind == wire::DATA && !frame.data_policy_ok() {
            Self::status(ui, peer, wire::POLICY_REJECTED_CONTENT);
            return;
        }
        let reserved = self
            .peers
            .values()
            .filter_map(|p| p.incoming.as_ref())
            .map(|i| i.frame.total as usize)
            .sum::<usize>();
        let Some(state) = self.peers.get_mut(&peer) else {
            return;
        };
        if frame.kind == wire::HELLO {
            if !frame.hello_policy_ok() {
                state.ready = false;
                state.pending = None;
                Self::status(ui, peer, wire::POLICY_REJECTED_HELLO);
                return;
            }
            state.max_content = frame.advertised_max_content();
            let matched = frame.target_epoch == state.epoch;
            let different = state.remote.as_ref() != Some(&frame.source_epoch);
            if matched || state.remote.is_none() {
                if different {
                    if state.remote.is_some() {
                        if let Some(task) = state.decode.take() {
                            task.abort();
                        }
                        state.round += 1;
                        state.job = None;
                        state.pending = None;
                        state.incoming = None;
                        state.receipt = None;
                        state.baseline = count;
                        self.disconnected.push(peer);
                    }
                    state.remote = Some(frame.source_epoch.clone());
                    state.ready = false;
                }
                let was_ready = state.ready;
                state.ready = state.ready || matched;
                if state.ready && !was_ready {
                    Self::status(ui, peer, "已连接，等待新复制");
                }
                let pending = if state.ready {
                    state.pending.take()
                } else {
                    None
                };
                let reply = if different || !was_ready || !matched {
                    Some(SyncMessage::hello(
                        state.epoch.clone(),
                        frame.source_epoch,
                        wire::MAX_BYTES as u64,
                    ))
                } else {
                    None
                };
                if let Some(reply) = reply {
                    self.control(runtime, peer, reply);
                }
                if let Some(payload) = pending {
                    Self::status(ui, peer, "排队中");
                    self.start(peer, payload, runtime, ui);
                }
            } else if frame.target_epoch.is_empty() {
                let reply = SyncMessage::hello(
                    state.epoch.clone(),
                    frame.source_epoch,
                    wire::MAX_BYTES as u64,
                );
                self.control(runtime, peer, reply);
            }
            return;
        }
        if !state.accepts(&frame) {
            return;
        }
        if frame.kind == wire::ACK {
            if let Some(job) = &state.job
                && job.id == frame.sync_id
                && job.revision == frame.revision
            {
                let _ = job.ack.try_send(frame.outcome);
            }
            return;
        }
        if !accept_content {
            self.ack(runtime, peer, &frame, wire::REJECTED);
            return;
        }
        if let Some(outcome) = state.duplicate(&frame) {
            self.ack(runtime, peer, &frame, outcome);
            return;
        }
        if let Some(incoming) = &state.incoming {
            if frame.revision < incoming.frame.revision {
                self.ack(runtime, peer, &frame, wire::SUPERSEDED);
                return;
            }
            if incoming.processing && frame.sync_id == incoming.frame.sync_id {
                return;
            }
        }
        if frame.offset == 0 {
            let previous = state
                .incoming
                .as_ref()
                .map_or(0, |i| i.frame.total as usize);
            if reserved
                .saturating_sub(previous)
                .saturating_add(frame.total as usize)
                > wire::MAX_BYTES
            {
                self.ack(runtime, peer, &frame, wire::REJECTED);
                return;
            }
            let baseline = state
                .incoming
                .as_ref()
                .filter(|i| i.frame.sync_id == frame.sync_id)
                .map_or(count, |i| i.baseline);
            state.incoming = Some(Incoming {
                frame: frame.clone(),
                baseline,
                data: Vec::new(),
                processing: false,
                started: Instant::now(),
            });
        }
        let Some(incoming) = state.incoming.as_mut() else {
            return;
        };
        if incoming.frame.sync_id != frame.sync_id
            || incoming.frame.revision != frame.revision
            || incoming.frame.digest != frame.digest
            || incoming.frame.total != frame.total
            || incoming.frame.mime != frame.mime
            || incoming.frame.source_metadata != frame.source_metadata
            || incoming.data.len() as u64 != frame.offset
        {
            return;
        }
        if incoming.baseline != count {
            state.incoming = None;
            self.ack(runtime, peer, &frame, wire::SUPERSEDED);
            return;
        }
        incoming.data.extend_from_slice(&frame.data);
        if incoming.data.len() as u64 == frame.total {
            incoming.processing = true;
            let data = std::mem::take(&mut incoming.data);
            let baseline = incoming.baseline;
            let tx = self.tx.clone();
            let cpu = self.cpu.clone();
            state.decode = Some(tokio::spawn(async move {
                let Ok(permit) = cpu.acquire_owned().await else {
                    return;
                };
                let metadata = frame.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    if Sha256::digest(&data).as_slice() != metadata.digest {
                        return Err("剪贴板校验失败".into());
                    }
                    let items = if metadata.mime == clipboard_bundle::MIME {
                        Some(clipboard_bundle::decode(&data)?)
                    } else {
                        None
                    };
                    if !matches!(metadata.mime.as_str(), "text/plain" | "image/png")
                        && metadata.mime != clipboard_bundle::MIME
                        && metadata.mime != CLIPBOARD_FILE_MIME
                    {
                        return Err("不支持的剪贴板格式".into());
                    }
                    Ok((
                        ClipboardPayload {
                            mime: metadata.mime,
                            data,
                            sequence: metadata.revision,
                        },
                        items,
                    ))
                })
                .await
                .map_err(|_| "剪贴板解码任务失败".to_owned())
                .and_then(|r| r);
                let _ = tx.send(Work::Decoded {
                    peer,
                    frame,
                    baseline,
                    result,
                });
            }));
        }
    }
    fn ack(&mut self, runtime: &RuntimeHandle, peer: DeviceId, frame: &SyncMessage, outcome: u32) {
        self.control(
            runtime,
            peer,
            SyncMessage::ack(
                frame.target_epoch.clone(),
                frame.source_epoch.clone(),
                frame.sync_id.clone(),
                frame.revision,
                outcome,
            ),
        );
    }
    pub fn finish(&mut self, ready: Ready, outcome: u32, runtime: &RuntimeHandle) {
        if let Some(state) = self.peers.get_mut(&ready.peer) {
            state.receipt = Some((ready.frame.revision, ready.frame.sync_id.clone(), outcome));
            state.incoming = None;
            if let Some(task) = state.decode.take() {
                task.abort();
            }
        }
        self.ack(runtime, ready.peer, &ready.frame, outcome);
    }
    pub fn poll(
        &mut self,
        count: i64,
        runtime: &RuntimeHandle,
        ui: &UnboundedSender<UiEvent>,
        diagnostics: &Diagnostics,
        outgoing: &DeferredSources,
    ) -> Option<Ready> {
        for p in self.peers.values_mut() {
            if p.incoming
                .as_ref()
                .is_some_and(|i| i.started.elapsed() > Duration::from_secs(60))
            {
                p.incoming = None;
                if let Some(task) = p.decode.take() {
                    task.abort();
                }
            }
        }
        while let Ok(work) = self.rx.try_recv() {
            match work {
                Work::Prepared {
                    revision,
                    targets,
                    change_count,
                    explicit,
                    copy_generation,
                    result,
                } => {
                    if copy_generation != self.copy_generation
                        || (!explicit && change_count != count)
                    {
                        continue;
                    }
                    let targets = targets
                        .into_iter()
                        .filter(|(id, g, e, round)| {
                            self.peers.get(id).is_some_and(|p| {
                                p.generation == *g && p.epoch == *e && p.round == *round
                            }) && !self
                                .manual
                                .get(id)
                                .is_some_and(|(_, cancel)| cancel.load(Ordering::Acquire))
                        })
                        .map(|(id, _, _, _)| id)
                        .collect::<Vec<_>>();
                    match result {
                        Ok(Prepared::Native(payload, warning)) => {
                            let payload = Arc::new(payload);
                            for peer in targets {
                                if let Some(warning) = &warning {
                                    self.warnings.insert(peer, warning.clone());
                                    Self::status(
                                        ui,
                                        peer,
                                        &format!("工作流失败：{warning}；正在发送原内容"),
                                    );
                                }
                                self.start(peer, payload.clone(), runtime, ui);
                            }
                        }
                        Ok(Prepared::Files(source, selected)) => {
                            let offers = outgoing
                                .lock()
                                .ok()
                                .map(|mut store| {
                                    file_policy::register_workflow_offers(
                                        &mut store, &source, targets, selected,
                                    )
                                })
                                .unwrap_or_default();
                            for (peer, id) in offers {
                                let frame = ClipboardFileFrame::Deferred {
                                    transfer_id: id,
                                    names: source.names.clone(),
                                    total_bytes: source.total_bytes,
                                    entries: source.deferred_entries().to_vec(),
                                };
                                if let Ok(data) = frame.encode() {
                                    self.start(
                                        peer,
                                        Arc::new(ClipboardPayload {
                                            mime: CLIPBOARD_FILE_MIME.into(),
                                            data,
                                            sequence: revision,
                                        }),
                                        runtime,
                                        ui,
                                    );
                                }
                            }
                        }
                        Err(error) => {
                            for peer in targets {
                                Self::status(ui, peer, &format!("同步失败：{error}"));
                                self.finish_manual(peer, Err(error.clone()), ui);
                            }
                        }
                    }
                }
                Work::Decoded {
                    peer,
                    frame,
                    baseline,
                    result,
                } => {
                    let valid = self.peers.get(&peer).is_some_and(|p| {
                        p.epoch == frame.target_epoch
                            && p.remote.as_ref() == Some(&frame.source_epoch)
                            && p.incoming.as_ref().is_some_and(|i| {
                                i.frame.sync_id == frame.sync_id && i.baseline == baseline
                            })
                    });
                    if !valid {
                        continue;
                    }
                    if baseline != count {
                        self.peers.get_mut(&peer).unwrap().incoming = None;
                        self.ack(runtime, peer, &frame, wire::SUPERSEDED);
                        continue;
                    }
                    match result {
                        Ok((payload, items)) => {
                            return Some(Ready {
                                metadata: serde_json::from_slice(&frame.source_metadata)
                                    .unwrap_or_default(),
                                peer,
                                payload,
                                items,
                                frame,
                            });
                        }
                        Err(_) => {
                            let ready = Ready {
                                metadata: serde_json::from_slice(&frame.source_metadata)
                                    .unwrap_or_default(),
                                peer,
                                payload: ClipboardPayload {
                                    mime: String::new(),
                                    data: Vec::new(),
                                    sequence: 0,
                                },
                                items: None,
                                frame,
                            };
                            self.finish(ready, wire::REJECTED, runtime);
                        }
                    }
                }
                Work::Sent {
                    peer,
                    epoch,
                    id,
                    revision,
                    result,
                } => {
                    let Some(p) = self.peers.get_mut(&peer).filter(|p| {
                        p.epoch == epoch && p.job.as_ref().is_some_and(|j| j.revision == revision)
                    }) else {
                        continue;
                    };
                    p.job = None;
                    let phase = match &result {
                        Ok(()) => self
                            .warnings
                            .remove(&peer)
                            .map(|error| format!("工作流失败：{error}；已发送原内容"))
                            .unwrap_or_else(|| "已同步".to_owned()),
                        Err(error) => format!("同步失败：{error}"),
                    };
                    Self::status(ui, peer, &phase);
                    crate::diagnostics::record!(
                        diagnostics,
                        if result.is_ok() {
                            DiagnosticLevel::Info
                        } else {
                            DiagnosticLevel::Warning
                        },
                        "clipboard",
                        "delivery_result",
                        Some(format!(
                            "peer={peer} generation={} sync={} revision={revision} acknowledged={}",
                            p.generation,
                            hex::encode(&id),
                            result.is_ok()
                        )),
                        None,
                    );
                    self.finish_manual(peer, result, ui);
                }
            }
        }
        None
    }
}

fn encode_snapshot(
    snapshot: PasteboardSnapshot,
    revision: u64,
) -> Result<ClipboardPayload, String> {
    let payload = if !snapshot.items.is_empty() {
        ClipboardPayload {
            mime: clipboard_bundle::MIME.into(),
            data: clipboard_bundle::encode(&snapshot.items)?,
            sequence: revision,
        }
    } else if let Some(png) = snapshot.png {
        ClipboardPayload {
            mime: "image/png".into(),
            data: png,
            sequence: revision,
        }
    } else if let Some(text) = snapshot.text {
        ClipboardPayload {
            mime: "text/plain".into(),
            data: text.into_bytes(),
            sequence: revision,
        }
    } else {
        return Err("剪贴板没有可同步内容".into());
    };
    if payload.data.len() > wire::MAX_BYTES {
        return Err("剪贴板超过容量限制".into());
    }
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn peer(baseline: i64) -> Peer {
        Peer {
            decode: None,
            round: 0,
            pending: None,
            generation: 1,
            epoch: vec![1; 16],
            remote: Some(vec![2; 16]),
            ready: true,
            baseline,
            hello_due: Instant::now(),
            job: None,
            incoming: None,
            receipt: None,
            max_content: wire::MAX_BYTES as u64,
        }
    }
    fn frame() -> SyncMessage {
        SyncMessage {
            kind: wire::DATA,
            source_epoch: vec![2; 16],
            target_epoch: vec![1; 16],
            sync_id: vec![3; 16],
            revision: 7,
            mime: "text/plain".into(),
            digest: Sha256::digest(b"hello").to_vec(),
            total: 5,
            data: b"hello".to_vec(),
            ..Default::default()
        }
    }
    #[test]
    fn reconnect_baseline_does_not_replay_to_a_new_peer() {
        let online = peer(10);
        let reconnected = peer(11);
        assert!(online.should_send(11, false));
        assert!(!reconnected.should_send(11, false));
        assert!(reconnected.should_send(12, false));
        assert!(reconnected.should_send(11, true));
    }
    #[test]
    fn lost_ack_retries_return_receipt_even_after_local_copy() {
        let mut p = peer(10);
        let frame = frame();
        p.receipt = Some((frame.revision, frame.sync_id.clone(), wire::APPLIED));
        p.baseline = 99;
        assert_eq!(p.duplicate(&frame), Some(wire::APPLIED));
        let mut older = frame.clone();
        older.revision -= 1;
        assert_eq!(p.duplicate(&older), Some(wire::SUPERSEDED));
        let mut different = frame;
        different.sync_id = vec![9; 16];
        assert_eq!(p.duplicate(&different), Some(wire::SUPERSEDED));
    }
    #[test]
    fn old_connection_cannot_write_after_epoch_replacement() {
        let mut p = peer(10);
        let frame = frame();
        assert!(p.accepts(&frame));
        p.epoch = vec![8; 16];
        assert!(!p.accepts(&frame));
        p.epoch = vec![1; 16];
        p.remote = Some(vec![9; 16]);
        assert!(!p.accepts(&frame));
    }
    #[test]
    fn new_copy_retires_partial_image_so_retry_cannot_overwrite_it() {
        let id = DeviceId::from_bytes([4; 32]);
        let frame = frame();
        let mut p = peer(10);
        p.incoming = Some(Incoming {
            frame: frame.clone(),
            baseline: 10,
            data: vec![1],
            processing: true,
            started: Instant::now(),
        });
        let mut sync = Sync::default();
        sync.peers.insert(id, p);
        let (ui, _) = futures::channel::mpsc::unbounded();
        sync.invalidate_copy(&ui);
        assert!(sync.peers[&id].incoming.is_none());
        assert_eq!(sync.peers[&id].duplicate(&frame), Some(wire::SUPERSEDED));
    }
}
