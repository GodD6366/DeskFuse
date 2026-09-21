//! Each transfer has one bounded serial disk worker. Only the Tokio owner may
//! publish a verified result to the OS clipboard or release its reservation.
use super::*;

const RECEIVE_QUEUE_FRAMES: usize = 32;
type ReceiveKey = (DeviceId, TransferId);

#[derive(Clone)]
pub(super) struct ReceiveEventSender {
    pub(super) tx: TokioSender<ReceiveEvent>,
    #[cfg(test)]
    pub(super) gate: Option<Arc<ReceiveTestGate>>,
}
pub(super) fn receive_event_channel() -> (ReceiveEventSender, TokioReceiver<ReceiveEvent>) {
    let (tx, rx) = unbounded_channel();
    (
        ReceiveEventSender {
            tx,
            #[cfg(test)]
            gate: None,
        },
        rx,
    )
}
pub(super) enum ReceiveEvent {
    Progress {
        key: ReceiveKey,
        bytes: u64,
    },
    Finished {
        key: ReceiveKey,
        result: Result<(), String>,
    },
    Cleaned {
        key: ReceiveKey,
        error: String,
        cleanup: Result<(), String>,
    },
}
pub(super) struct IncomingFileClipboard {
    sender: Option<tokio::sync::mpsc::Sender<ClipboardFileFrame>>,
    cancelled: Arc<AtomicBool>,
    pub(super) finishing: bool,
    pending_frame: Option<ClipboardFileFrame>,
    accepted_frames: u64,
    cleaning: bool,
    pub(super) manifest: TransferManifest,
    pub(super) root_paths: Vec<PathBuf>,
    destination: PathBuf,
    pub(super) name: String,
    pub(super) received_bytes: u64,
    pub(super) pasteboard_change_count: i64,
    pub(super) last_activity: std::time::Instant,
    last_progress: std::time::Instant,
}
impl IncomingFileClipboard {
    pub(super) fn receipt(&self, transfer_id: TransferId) -> Option<ClipboardFileFrame> {
        (!self.cancelled.load(Ordering::Acquire) && self.pending_frame.is_none()).then_some(
            ClipboardFileFrame::Receipt {
                transfer_id,
                accepted_frames: self.accepted_frames,
            },
        )
    }
    pub(super) fn is_backpressured(&self) -> bool {
        self.pending_frame.is_some()
    }
    pub(super) fn cancel(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        // Wake a worker waiting for the next frame. Queued bodies are dropped by
        // the worker without writing them after it observes cancellation.
        self.sender.take();
        self.pending_frame.take();
    }
}
impl Drop for IncomingFileClipboard {
    fn drop(&mut self) {
        self.cancel();
    }
}
pub(super) async fn wait_for_receive_capacity(
    incoming: &HashMap<ReceiveKey, IncomingFileClipboard>,
) -> (
    ReceiveKey,
    Result<tokio::sync::mpsc::OwnedPermit<ClipboardFileFrame>, String>,
) {
    let pending = incoming.iter().find_map(|(key, state)| {
        state.pending_frame.as_ref()?;
        Some((*key, state.sender.as_ref()?.clone()))
    });
    let Some((key, sender)) = pending else {
        return std::future::pending().await;
    };
    (
        key,
        sender
            .reserve_owned()
            .await
            .map_err(|error| error.to_string()),
    )
}

pub(super) fn flush_pending_receive(
    key: ReceiveKey,
    permit: Result<tokio::sync::mpsc::OwnedPermit<ClipboardFileFrame>, String>,
    incoming: &mut HashMap<ReceiveKey, IncomingFileClipboard>,
    ui: &UnboundedSender<UiEvent>,
) {
    let Some(state) = incoming.get_mut(&key) else {
        return;
    };
    match permit {
        Ok(permit) => {
            if let Some(frame) = state.pending_frame.take() {
                permit.send(frame);
                state.accepted_frames = state.accepted_frames.saturating_add(1);
            }
        }
        Err(error) => {
            state.cancel();
            emit_receive_transfer_status(
                ui,
                key.1,
                state.name.clone(),
                format!("文件磁盘接收已结束：{error}"),
                TransferState::Failed,
                state.received_bytes,
                state.manifest.total_bytes(),
            );
        }
    }
}

fn remove_receive_destination(path: &std::path::Path) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
        Ok(metadata) if metadata.file_type().is_symlink() => Err("拒绝清理符号链接接收目录".into()),
        Ok(_) => std::fs::remove_dir_all(path).map_err(|error| error.to_string()),
    }
}
fn run_receive_disk_worker(
    key: ReceiveKey,
    destination: PathBuf,
    manifest: TransferManifest,
    mut frames: tokio::sync::mpsc::Receiver<ClipboardFileFrame>,
    cancelled: Arc<AtomicBool>,
    events: ReceiveEventSender,
) -> Result<(), String> {
    let result = (|| {
        if cancelled.load(Ordering::Acquire) {
            return Err("接收已取消".into());
        }
        let receiver = clipboard_file_receiver(&destination)?;
        let mut staging = receiver
            .begin(key.1, &manifest)
            .map_err(|error| error.to_string())?;
        let mut received = 0_u64;
        let mut last_progress = std::time::Instant::now();
        while let Some(frame) = frames.blocking_recv() {
            if cancelled.load(Ordering::Acquire) {
                return Err("接收已取消".into());
            }
            match frame {
                ClipboardFileFrame::Chunk {
                    path,
                    descriptor,
                    bytes,
                    ..
                } => {
                    staging
                        .write_chunk(&path, descriptor, &bytes)
                        .map_err(|error| error.to_string())?;
                    received = received.saturating_add(u64::from(descriptor.length));
                    if last_progress.elapsed() >= Duration::from_millis(100) {
                        let _ = events.tx.send(ReceiveEvent::Progress {
                            key,
                            bytes: received,
                        });
                        last_progress = std::time::Instant::now();
                    }
                }
                ClipboardFileFrame::Complete { .. } => {
                    #[cfg(test)]
                    if let Some(gate) = &events.gate {
                        gate.wait_in_disk_worker();
                    }
                    if cancelled.load(Ordering::Acquire) {
                        return Err("接收已取消".into());
                    }
                    // commit_staging performs the complete-file hashes once.
                    // Do not call complete_file beforehand and hash every byte twice.
                    receiver
                        .commit_staging(staging, ConflictStrategy::Overwrite)
                        .map_err(|error| error.to_string())?;
                    if cancelled.load(Ordering::Acquire) {
                        return Err("接收已取消".into());
                    }
                    return Ok(());
                }
                _ => return Err("接收队列包含无效文件帧".into()),
            }
        }
        Err("接收已取消或连接已关闭".into())
    })();
    if result.is_err() {
        remove_receive_destination(&destination).map_err(|cleanup| {
            format!(
                "{}；临时文件清理失败：{cleanup}",
                result.as_ref().unwrap_err()
            )
        })?;
    }
    result
}
#[cfg(test)]
pub(super) fn apply_remote_file_clipboard(
    pasteboard: &impl Pasteboard,
    bytes: &[u8],
    incoming: &mut HashMap<ReceiveKey, IncomingFileClipboard>,
    cache: &mut TransferCache,
    peer_id: DeviceId,
    events: &ReceiveEventSender,
    ui: &UnboundedSender<UiEvent>,
) -> Result<(), String> {
    let frame = ClipboardFileFrame::decode(bytes).map_err(|error| error.to_string())?;
    apply_remote_file_clipboard_frame(pasteboard, frame, incoming, cache, peer_id, events, ui)
}

pub(super) fn apply_remote_file_clipboard_frame(
    pasteboard: &impl Pasteboard,
    frame: ClipboardFileFrame,
    incoming: &mut HashMap<ReceiveKey, IncomingFileClipboard>,
    cache: &mut TransferCache,
    peer_id: DeviceId,
    events: &ReceiveEventSender,
    ui: &UnboundedSender<UiEvent>,
) -> Result<(), String> {
    let id = frame.transfer_id();
    let key = (peer_id, id);
    match frame {
        ClipboardFileFrame::Offer { manifest, .. }
        | ClipboardFileFrame::RequestedOffer { manifest, .. } => {
            if incoming
                .keys()
                .any(|(peer, transfer)| *transfer == id && *peer != peer_id)
            {
                return Err("传输标识与另一个设备的活动任务冲突".into());
            }
            if let Some(existing) = incoming.get(&key) {
                return if existing.cancelled.load(Ordering::Acquire) {
                    Err("旧接收任务正在退出，请重新复制文件".into())
                } else {
                    Ok(())
                };
            }
            let change_count = if pasteboard.read_remote_origin_marker().ok().flatten()
                == Some(format!("deskfuse-clipboard-file-v1:{id}"))
            {
                pasteboard
                    .change_count()
                    .map_err(|error| error.to_string())?
            } else {
                // Detached requested responses own no current clipboard generation.
                -1
            };
            let destination = cache.destination(id);
            let root_paths = clipboard_root_paths(&destination, &manifest)?;
            cache.reserve(id, manifest.total_bytes())?;
            let (sender, frames) = tokio::sync::mpsc::channel(RECEIVE_QUEUE_FRAMES);
            let cancelled = Arc::new(AtomicBool::new(false));
            let name = clipboard_transfer_name(&root_paths);
            let total = manifest.total_bytes();
            let worker_manifest = manifest.clone();
            let worker_destination = destination.clone();
            let worker_cancelled = cancelled.clone();
            let worker_events = events.clone();
            let completion = events.tx.clone();
            tokio::spawn(async move {
                let cleanup_destination = worker_destination.clone();
                let result = tokio::task::spawn_blocking(move || {
                    run_receive_disk_worker(
                        key,
                        worker_destination,
                        worker_manifest,
                        frames,
                        worker_cancelled,
                        worker_events,
                    )
                })
                .await;
                let result = match result {
                    Ok(result) => result,
                    Err(error) => {
                        let failure = format!("文件接收后台任务失败：{error}");
                        let cleanup = tokio::task::spawn_blocking(move || {
                            remove_receive_destination(&cleanup_destination)
                        })
                        .await;
                        Err(match cleanup {
                            Ok(Ok(())) => failure,
                            _ => format!("{failure}；临时文件清理失败"),
                        })
                    }
                };
                // Awaiting the join guarantees reservation release happens only
                // after the worker, its file handles and any cleanup have ended.
                let _ = completion.send(ReceiveEvent::Finished { key, result });
            });
            incoming.insert(
                key,
                IncomingFileClipboard {
                    sender: Some(sender),
                    cancelled,
                    finishing: false,
                    pending_frame: None,
                    accepted_frames: 1,
                    cleaning: false,
                    manifest,
                    root_paths,
                    destination,
                    name: name.clone(),
                    received_bytes: 0,
                    pasteboard_change_count: change_count,
                    last_activity: std::time::Instant::now(),
                    last_progress: std::time::Instant::now(),
                },
            );
            emit_receive_transfer_status(
                ui,
                id,
                name,
                "正在接收文件…".into(),
                TransferState::Transferring,
                0,
                total,
            );
        }
        ClipboardFileFrame::Chunk { .. } | ClipboardFileFrame::Complete { .. } => {
            let state = incoming
                .get_mut(&key)
                .ok_or_else(|| format!("未找到文件传输 {id} 的清单"))?;
            if state.cancelled.load(Ordering::Acquire) {
                return Err("接收已取消，等待磁盘任务退出".into());
            }
            if state.finishing {
                return if matches!(frame, ClipboardFileFrame::Complete { .. }) {
                    Ok(())
                } else {
                    Err("文件已经进入校验阶段".into())
                };
            }
            let complete = matches!(frame, ClipboardFileFrame::Complete { .. });
            let chunk_bytes = match &frame {
                ClipboardFileFrame::Chunk { bytes, .. } => bytes.len() as u64,
                _ => 0,
            };
            let sender = state.sender.as_ref().ok_or("接收队列已关闭")?;
            match sender.try_send(frame) {
                Ok(()) => {
                    state.accepted_frames = state.accepted_frames.saturating_add(1);
                }
                Err(tokio::sync::mpsc::error::TrySendError::Full(frame))
                    if state.pending_frame.is_none() =>
                {
                    // Hold exactly one bounded frame. The engine stops draining
                    // network events until this FIFO slot is sent, but continues
                    // servicing cancellation, disk completion and permission ticks.
                    state.pending_frame = Some(frame);
                }
                Err(error) => {
                    state.cancel();
                    return Err(format!("接收队列已关闭：{error}"));
                }
            }
            let first_chunk = state.received_bytes == 0 && chunk_bytes > 0;
            state.received_bytes = state.received_bytes.saturating_add(chunk_bytes);
            state.finishing = complete;
            state.last_activity = std::time::Instant::now();
            if complete
                || first_chunk
                || state.last_progress.elapsed() >= Duration::from_millis(100)
            {
                state.last_progress = std::time::Instant::now();
                emit_receive_transfer_status(
                    ui,
                    id,
                    state.name.clone(),
                    if complete {
                        "正在校验文件…"
                    } else {
                        "正在接收文件…"
                    }
                    .into(),
                    if complete {
                        TransferState::Verifying
                    } else {
                        TransferState::Transferring
                    },
                    state.received_bytes,
                    state.manifest.total_bytes(),
                );
            }
        }
        ClipboardFileFrame::Cancel { .. } => {
            if let Some(state) = incoming.get_mut(&key) {
                state.cancel();
            }
        }
        _ => {}
    }
    Ok(())
}
pub(super) fn handle_receive_event(
    event: ReceiveEvent,
    pasteboard: &impl Pasteboard,
    state: &mut RemoteClipboardState<'_>,
    ui: &UnboundedSender<UiEvent>,
    diagnostics: &Diagnostics,
) -> Option<(DeviceId, TransferId)> {
    let (key, result) = match event {
        ReceiveEvent::Progress { key, bytes } => {
            if let Some(incoming) = state.incoming_files.get_mut(&key) {
                // Display network receipt immediately. Disk acknowledgements
                // may lag; never reset the displayed progress to an older value.
                incoming.received_bytes = incoming.received_bytes.max(bytes);
            }
            return None;
        }
        ReceiveEvent::Finished { key, result } => (key, result),
        ReceiveEvent::Cleaned {
            key,
            error,
            cleanup,
        } => (
            key,
            Err(match cleanup {
                Ok(()) => error,
                Err(cleanup) => format!("{error}；临时文件清理失败：{cleanup}"),
            }),
        ),
    };
    let incoming = state.incoming_files.get_mut(&key)?;
    let owns_generation = pasteboard.change_count().ok() == Some(incoming.pasteboard_change_count)
        && pasteboard.read_remote_origin_marker().ok().flatten()
            == Some(format!("deskfuse-clipboard-file-v1:{}", key.1));
    let mut eligible = !incoming.cancelled.load(Ordering::Acquire)
        && owns_generation
        && state.pending_incoming_file.load(Ordering::Acquire);
    let committed = result.is_ok();
    let result = if committed && incoming.cancelled.load(Ordering::Acquire) {
        Err("接收已取消".into())
    } else {
        result
    };
    // Clipboard replacement removes only the publication/replay intent. Keep
    // verified files in the cache; explicit cancellation still removes them.
    if result.is_ok()
        && eligible
        && let Err(error) = pasteboard.write_file_urls(
            &incoming
                .root_paths
                .iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
        )
    {
        // A Finder delivery owns verified cache files independently of the
        // system clipboard. Publication failure must not delete its source.
        eligible = false;
        emit_clipboard_error(ui, diagnostics, error.to_string());
    }
    if committed
        && !incoming.cleaning
        && let Err(error) = &result
    {
        incoming.cleaning = true;
        incoming.cancel();
        let path = incoming.destination.clone();
        let tx = state.receive_events.tx.clone();
        let error = error.clone();
        tokio::spawn(async move {
            let cleanup = tokio::task::spawn_blocking(move || remove_receive_destination(&path))
                .await
                .map_err(|error| error.to_string())
                .and_then(|result| result);
            let _ = tx.send(ReceiveEvent::Cleaned {
                key,
                error,
                cleanup,
            });
        });
        return None;
    }
    let mut incoming = state
        .incoming_files
        .remove(&key)
        .expect("event matched live receiver");
    incoming.sender.take();
    let file_count = incoming.manifest.file_count();
    let total = incoming.manifest.total_bytes();
    let metadata = take_file_source(key.0, key.1);
    let publish = result;
    state.cache.finish(key.1);
    diagnostics.record(
        if publish.is_ok() {
            DiagnosticLevel::Info
        } else {
            DiagnosticLevel::Warning
        },
        "file_transfer",
        "receive_finalized",
        Some(format!(
            "peer={} transfer={} bytes={total} verified={} clipboard_published={eligible}",
            key.0.to_hex(),
            key.1,
            publish.is_ok()
        )),
        None,
    );
    match publish {
        Ok(()) => {
            if !eligible {
                let mut snapshot = PasteboardSnapshot {
                    file_urls: incoming
                        .root_paths
                        .iter()
                        .map(|path| path.to_string_lossy().into_owned())
                        .collect(),
                    ..Default::default()
                };
                clipboard_bundle::attach_source_metadata(&mut snapshot, &metadata);
                state
                    .history
                    .record(snapshot, format!("远端 {}", key.0.to_hex()), ui);
                emit_receive_transfer_status(
                    ui,
                    key.1,
                    std::mem::take(&mut incoming.name),
                    "已校验并缓存；剪贴板已变化，可从历史记录使用".into(),
                    TransferState::Completed,
                    total,
                    total,
                );
                return Some(key);
            }
            state.pending_incoming_file.store(false, Ordering::Release);
            let target = state
                .paste_requested
                .swap(false, Ordering::AcqRel)
                .then(|| state.paste_target_pid.swap(0, Ordering::AcqRel));
            if let Err(error) = pasteboard
                .write_remote_origin_marker(&format!("deskfuse-clipboard-file-v1:{}", key.1))
            {
                emit_clipboard_error(ui, diagnostics, error.to_string());
            }
            *state.last_change_count = pasteboard.change_count().ok();
            if let Ok(mut snapshot) = pasteboard.read() {
                clipboard_bundle::attach_source_metadata(&mut snapshot, &metadata);
                state
                    .history
                    .record(snapshot, format!("远端 {}", key.0.to_hex()), ui);
            }
            if let (Some(target_pid), Some(expected_change_count)) =
                (target, *state.last_change_count)
            {
                let _ = ui.unbounded_send(UiEvent::PasteReady {
                    target_pid,
                    expected_change_count,
                });
            }
            emit_receive_transfer_status(
                ui,
                key.1,
                std::mem::take(&mut incoming.name),
                "已校验并写入本机剪贴板".into(),
                TransferState::Completed,
                total,
                total,
            );
            crate::diagnostics::record!(
                diagnostics,
                DiagnosticLevel::Info,
                "clipboard",
                "file_received",
                Some(format!(
                    "peer={} transfer={} files={file_count} bytes={total}",
                    key.0.to_hex(),
                    key.1
                )),
                None,
            );
            let _ = ui.unbounded_send(UiEvent::ClipboardStatus {
                enabled: true,
                last_mime: Some(CLIPBOARD_FILE_MIME.into()),
                last_len: usize::try_from(total).unwrap_or(usize::MAX),
                last_sequence: None,
                error: None,
            });
            Some(key)
        }
        Err(error) => {
            if owns_generation {
                state.pending_incoming_file.store(false, Ordering::Release);
                state.paste_requested.store(false, Ordering::Release);
                state.paste_target_pid.store(0, Ordering::Release);
            }
            emit_receive_transfer_status(
                ui,
                key.1,
                std::mem::take(&mut incoming.name),
                error.clone(),
                if incoming.cancelled.load(Ordering::Acquire) {
                    TransferState::Cancelled
                } else {
                    TransferState::Failed
                },
                incoming.received_bytes,
                total,
            );
            crate::diagnostics::record!(
                diagnostics,
                DiagnosticLevel::Error,
                "clipboard",
                "file_apply_failed",
                Some(format!("peer={} {error}", key.0.to_hex())),
                None,
            );
            emit_clipboard_error(ui, diagnostics, error);
            None
        }
    }
}

#[cfg(test)]
#[derive(Default)]
pub(super) struct ReceiveTestGate {
    pub(super) entered: AtomicBool,
    released: Mutex<bool>,
    changed: Condvar,
}
#[cfg(test)]
impl ReceiveTestGate {
    fn wait_in_disk_worker(&self) {
        self.entered.store(true, Ordering::Release);
        let released = self.released.lock().unwrap();
        drop(
            self.changed
                .wait_while(released, |released| !*released)
                .unwrap(),
        );
    }
    pub(super) fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.changed.notify_all();
    }
}

// Source policy is keyed by the authenticated peer plus transfer UUID, rather
// than the mutable pasteboard. Deferred completion can happen long after copy.
type SourcePolicies = std::collections::HashMap<
    (DeviceId, TransferId),
    (std::time::Instant, clipboard_bundle::SourceMetadata),
>;
fn file_sources() -> &'static Mutex<SourcePolicies> {
    static SOURCES: std::sync::OnceLock<Mutex<SourcePolicies>> = std::sync::OnceLock::new();
    SOURCES.get_or_init(|| Mutex::new(HashMap::new()))
}
pub(super) fn remember_file_source(
    peer: DeviceId,
    id: TransferId,
    metadata: clipboard_bundle::SourceMetadata,
) {
    if let Ok(mut sources) = file_sources().lock() {
        sources.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(24 * 60 * 60));
        if sources.len() >= 4096 {
            return;
        }
        sources.insert((peer, id), (std::time::Instant::now(), metadata));
    }
}
fn take_file_source(peer: DeviceId, id: TransferId) -> clipboard_bundle::SourceMetadata {
    file_sources()
        .lock()
        .ok()
        .and_then(|mut sources| sources.remove(&(peer, id)))
        .map(|(_, source)| source)
        // Missing/expired metadata must not turn an excluded copy into history.
        .unwrap_or_else(|| clipboard_bundle::SourceMetadata {
            no_history: true,
            ..Default::default()
        })
}
