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
    finishing: bool,
    cleaning: bool,
    pub(super) manifest: TransferManifest,
    pub(super) root_paths: Vec<PathBuf>,
    destination: PathBuf,
    pub(super) name: String,
    pub(super) received_bytes: u64,
    pub(super) pasteboard_change_count: i64,
    pub(super) last_activity: std::time::Instant,
}
impl IncomingFileClipboard {
    pub(super) fn cancel(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        // Wake a worker waiting for the next frame. Queued bodies are dropped by
        // the worker without writing them after it observes cancellation.
        self.sender.take();
    }
}
impl Drop for IncomingFileClipboard {
    fn drop(&mut self) {
        self.cancel();
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
                    let _ = events.tx.send(ReceiveEvent::Progress {
                        key,
                        bytes: received,
                    });
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
            let change_count = pasteboard
                .change_count()
                .map_err(|error| error.to_string())?;
            let destination = cache.destination(id);
            let root_paths = clipboard_root_paths(&destination, &manifest)?;
            // Superseded workers stay in the map and keep their reservations
            // until their completion events prove that all disk work has ended.
            for ((peer, _), state) in incoming.iter_mut() {
                if *peer == peer_id {
                    state.cancel();
                }
            }
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
                    cleaning: false,
                    manifest,
                    root_paths,
                    destination,
                    name: name.clone(),
                    received_bytes: 0,
                    pasteboard_change_count: change_count,
                    last_activity: std::time::Instant::now(),
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
            let sender = state.sender.as_ref().ok_or("接收队列已关闭")?;
            if let Err(error) = sender.try_send(frame) {
                state.cancel();
                return Err(format!("接收队列无法继续写入，请重试：{error}"));
            }
            state.finishing = complete;
            state.last_activity = std::time::Instant::now();
            if complete {
                emit_receive_transfer_status(
                    ui,
                    id,
                    state.name.clone(),
                    "正在校验文件…".into(),
                    TransferState::Transferring,
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
                incoming.received_bytes = bytes;
                if !incoming.cancelled.load(Ordering::Acquire) {
                    emit_receive_transfer_status(
                        ui,
                        key.1,
                        incoming.name.clone(),
                        "正在接收文件…".into(),
                        TransferState::Transferring,
                        bytes,
                        incoming.manifest.total_bytes(),
                    );
                }
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
            == Some(format!("unidesk-clipboard-file-v1:{}", key.1));
    let eligible = !incoming.cancelled.load(Ordering::Acquire)
        && owns_generation
        && state.pending_incoming_file.load(Ordering::Acquire);
    let committed = result.is_ok();
    let result = if committed && !eligible {
        Err("剪贴板或粘贴请求已变化，本次接收已取消".into())
    } else {
        result
    };
    // A successful commit which became stale must be removed off-thread before
    // releasing its reservation. A failed worker already did its own cleanup.
    let result = result.and_then(|()| {
        pasteboard
            .write_file_urls(
                &incoming
                    .root_paths
                    .iter()
                    .map(|path| path.to_string_lossy().into_owned())
                    .collect::<Vec<_>>(),
            )
            .map_err(|error| error.to_string())
    });
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
    let publish = result;
    state.cache.finish(key.1);
    match publish {
        Ok(()) => {
            state.pending_incoming_file.store(false, Ordering::Release);
            let target = state
                .paste_requested
                .swap(false, Ordering::AcqRel)
                .then(|| state.paste_target_pid.swap(0, Ordering::AcqRel));
            if let Err(error) = pasteboard
                .write_remote_origin_marker(&format!("unidesk-clipboard-file-v1:{}", key.1))
            {
                emit_clipboard_error(ui, diagnostics, error.to_string());
            }
            *state.last_change_count = pasteboard.change_count().ok();
            if let Ok(snapshot) = pasteboard.read() {
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
            diagnostics.record(
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
            diagnostics.record(
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
