//! Independent Finder delivery intentions. Only this Tokio owner mutates jobs;
//! native copy workers return progress/receipts and hold cache leases until exit.
use super::*;
use crate::platform::{
    CapturedFilePaste, DeliveryResult, FilePasteReference, FinderDestination, deliver_finder_files,
};

struct Job {
    id: TransferId,
    reference: FilePasteReference,
    destination: FinderDestination,
    remaining: Option<Vec<PathBuf>>,
    cancel: Arc<AtomicBool>,
    running: bool,
    terminal: bool,
    saved: u64,
    total: u64,
    lease: Option<ShareOutputLease>,
}
struct Ready {
    reference: FilePasteReference,
    paths: Vec<PathBuf>,
}
pub(super) enum DeliveryEvent {
    Progress {
        id: TransferId,
        saved: u64,
        total: u64,
    },
    Finished {
        id: TransferId,
        result: Result<DeliveryResult, String>,
    },
}
pub(super) struct FinderPasteJobs {
    jobs: HashMap<TransferId, Job>,
    ready: HashMap<TransferId, Ready>,
    completed: HashMap<(TransferId, (u64, u64)), std::time::Instant>,
    tx: TokioSender<DeliveryEvent>,
    pub rx: TokioReceiver<DeliveryEvent>,
    leases: ExternalCacheLeases,
}
impl FinderPasteJobs {
    pub fn new(leases: ExternalCacheLeases) -> Self {
        let (tx, rx) = unbounded_channel();
        Self {
            jobs: HashMap::new(),
            ready: HashMap::new(),
            completed: HashMap::new(),
            tx,
            rx,
            leases,
        }
    }
    pub fn ready_reference(&self, pasteboard: &impl Pasteboard) -> Option<FilePasteReference> {
        let marker = pasteboard.read_remote_origin_marker().ok().flatten()?;
        let ready = self
            .ready
            .iter()
            .find(|(id, _)| marker == format!("unidesk-clipboard-file-v1:{id}"))?;
        // Continue intercepting while a delivery of this clipboard is unfinished.
        if !self
            .jobs
            .values()
            .any(|j| j.reference.transfer_id == ready.0.to_string() && !j.terminal)
        {
            return None;
        }
        let mut reference = ready.1.reference.clone();
        reference.change_count = pasteboard.change_count().ok()?;
        Some(reference)
    }
    pub fn has_ready(&self, id: TransferId) -> bool {
        self.ready.contains_key(&id)
    }
    pub fn has_job(&self, id: TransferId) -> bool {
        self.jobs.contains_key(&id)
    }
    pub fn busy(&self) -> bool {
        self.jobs.values().any(|j| j.running || !j.terminal)
    }
    pub fn register(
        &mut self,
        id: TransferId,
        captured: CapturedFilePaste,
        destination: FinderDestination,
        ui: &UnboundedSender<UiEvent>,
        diagnostics: &Diagnostics,
    ) -> Result<(), String> {
        let transfer = parse_transfer(&captured.reference.transfer_id)?;
        self.completed
            .retain(|_, finished| finished.elapsed() < Duration::from_secs(600));
        if self
            .completed
            .get(&(transfer, destination.identity()))
            .is_some_and(|finished| captured.captured_at <= *finished)
        {
            emit_receive_transfer_status(
                ui,
                id,
                captured.reference.name.clone(),
                "已合并到现有粘贴任务".into(),
                TransferState::Completed,
                0,
                0,
            );
            return Ok(());
        }
        if self.jobs.values().any(|j| {
            !j.terminal
                && j.reference.transfer_id == captured.reference.transfer_id
                && j.destination.identity() == destination.identity()
        }) {
            emit_receive_transfer_status(
                ui,
                id,
                captured.reference.name.clone(),
                "已合并到现有粘贴任务".into(),
                TransferState::Completed,
                0,
                0,
            );
            return Ok(());
        }
        let paths = self.ready.get(&transfer).map(|r| r.paths.clone());
        let lease = paths.as_ref().map(|p| self.retain(id, p)).transpose()?;
        let job = Job {
            id,
            reference: captured.reference,
            destination,
            remaining: paths,
            cancel: Arc::new(AtomicBool::new(false)),
            running: false,
            terminal: false,
            saved: 0,
            total: 0,
            lease,
        };
        let _ = ui.unbounded_send(UiEvent::FilePasteLinked {
            paste_id: id,
            transfer_id: transfer,
            can_retry: job.remaining.is_some(),
        });
        emit_receive_transfer_status(
            ui,
            id,
            job.reference.name.clone(),
            "已锁定 Finder 目标，等待文件…".into(),
            TransferState::Preparing,
            0,
            job.reference.total_bytes,
        );
        diagnostics.record(
            DiagnosticLevel::Info,
            "file_paste",
            "requested",
            Some(format!(
                "paste={id} transfer={transfer} pid={} bytes={}",
                captured.target_pid, job.reference.total_bytes
            )),
            None,
        );
        self.jobs.insert(id, job);
        self.start(id, ui, diagnostics);
        Ok(())
    }
    fn retain(&self, id: TransferId, paths: &[PathBuf]) -> Result<ShareOutputLease, String> {
        self.leases.lock().map_err(|_| "缓存租约不可用")?.insert(
            id,
            paths
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
        );
        Ok(ShareOutputLease {
            id,
            leases: self.leases.clone(),
        })
    }
    pub fn received(
        &mut self,
        reference: FilePasteReference,
        paths: Vec<PathBuf>,
        ui: &UnboundedSender<UiEvent>,
        diagnostics: &Diagnostics,
    ) {
        let Ok(transfer) = parse_transfer(&reference.transfer_id) else {
            return;
        };
        let ids = self
            .jobs
            .values()
            .filter(|j| !j.terminal && j.reference.transfer_id == reference.transfer_id)
            .map(|j| j.id)
            .collect::<Vec<_>>();
        for id in ids {
            match self.retain(id, &paths) {
                Ok(lease) => {
                    let job = self.jobs.get_mut(&id).expect("selected job");
                    job.remaining = Some(paths.clone());
                    job.lease = Some(lease);
                    let _ = ui.unbounded_send(UiEvent::FilePasteLinked {
                        paste_id: id,
                        transfer_id: transfer,
                        can_retry: true,
                    });
                    self.start(id, ui, diagnostics);
                }
                Err(error) => self.fail(id, error, ui),
            }
        }
        // Only active/pending clipboard identities need a fast-path source map.
        // Failed delivery jobs retain their own remaining paths and lease.
        self.ready.retain(|id, _| {
            self.jobs
                .values()
                .any(|j| !j.terminal && j.reference.transfer_id == id.to_string())
        });
        self.ready.insert(transfer, Ready { reference, paths });
    }
    fn start(&mut self, id: TransferId, ui: &UnboundedSender<UiEvent>, diagnostics: &Diagnostics) {
        let Some(job) = self.jobs.get_mut(&id) else {
            return;
        };
        if job.running || job.terminal {
            return;
        }
        let Some(paths) = job.remaining.clone() else {
            return;
        };
        job.running = true;
        let destination = job.destination.clone();
        let cancelled = job.cancel.clone();
        let tx = self.tx.clone();
        let worker_lease_id = TransferId::new();
        if let Ok(mut leases) = self.leases.lock() {
            leases.insert(
                worker_lease_id,
                paths
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect(),
            );
        } else {
            job.running = false;
            job.terminal = true;
            emit_receive_transfer_status(
                ui,
                id,
                job.reference.name.clone(),
                "无法保护待保存的缓存文件，请重试".into(),
                TransferState::Failed,
                0,
                job.reference.total_bytes,
            );
            return;
        }
        let worker_lease = ShareOutputLease {
            id: worker_lease_id,
            leases: self.leases.clone(),
        };
        emit_receive_transfer_status(
            ui,
            id,
            job.reference.name.clone(),
            "正在保存到原 Finder 目录…".into(),
            TransferState::Delivering,
            0,
            job.reference.total_bytes,
        );
        diagnostics.record(
            DiagnosticLevel::Info,
            "file_paste",
            "delivery_started",
            Some(format!(
                "paste={id} transfer={} items={}",
                job.reference.transfer_id,
                paths.len()
            )),
            None,
        );
        tokio::spawn(async move {
            let progress = tx.clone();
            let result = tokio::task::spawn_blocking(move || {
                let _worker_lease = worker_lease;
                let last = Mutex::new(std::time::Instant::now() - Duration::from_secs(1));
                deliver_finder_files(&destination, &paths, &cancelled, |saved, total| {
                    if let Ok(mut last) = last.lock()
                        && (saved == total || last.elapsed() >= Duration::from_millis(100))
                    {
                        *last = std::time::Instant::now();
                        let _ = progress.send(DeliveryEvent::Progress { id, saved, total });
                    }
                })
            })
            .await
            .map_err(|_| "保存文件的后台任务异常退出；请检查目标目录后重试".to_owned())
            .and_then(|r| r);
            let _ = tx.send(DeliveryEvent::Finished { id, result });
        });
    }
    pub fn handle(
        &mut self,
        event: DeliveryEvent,
        ui: &UnboundedSender<UiEvent>,
        diagnostics: &Diagnostics,
    ) {
        match event {
            DeliveryEvent::Progress { id, saved, total } => {
                let Some(job) = self.jobs.get_mut(&id) else {
                    return;
                };
                if !job.running || job.terminal {
                    return;
                }
                job.saved = job.saved.max(saved);
                job.total = total;
                emit_receive_transfer_status(
                    ui,
                    id,
                    job.reference.name.clone(),
                    "正在保存到原 Finder 目录…".into(),
                    TransferState::Delivering,
                    job.saved,
                    total,
                );
            }
            DeliveryEvent::Finished { id, result } => {
                let Some(job) = self.jobs.get_mut(&id) else {
                    return;
                };
                job.running = false;
                job.terminal = true;
                let (error, cancelled) = match result {
                    Ok(result) => {
                        if let Some(paths) = &mut job.remaining {
                            paths.retain(|path| {
                                !result.items.iter().any(|item| &item.source == path)
                            });
                        }
                        (result.error, result.cancelled)
                    }
                    Err(error) => (Some(error), job.cancel.load(Ordering::Acquire)),
                };
                let done = job.remaining.as_ref().is_some_and(Vec::is_empty) && error.is_none();
                let phase = if done {
                    TransferState::Completed
                } else if cancelled {
                    TransferState::Cancelled
                } else {
                    TransferState::Failed
                };
                let detail = if done {
                    "已保存到首次粘贴的 Finder 目录".into()
                } else {
                    error.unwrap_or_else(|| "保存已取消；已保存项目保留，可重试剩余项目".into())
                };
                if done {
                    job.saved = job.total;
                    job.lease.take();
                }
                emit_receive_transfer_status(
                    ui,
                    id,
                    job.reference.name.clone(),
                    detail,
                    phase,
                    job.saved,
                    job.total,
                );
                diagnostics.record(
                    if done {
                        DiagnosticLevel::Info
                    } else {
                        DiagnosticLevel::Warning
                    },
                    "file_paste",
                    "delivery_finished",
                    Some(format!(
                        "paste={id} transfer={} phase={phase:?} bytes={} remaining={}",
                        job.reference.transfer_id,
                        job.saved,
                        job.remaining.as_ref().map_or(0, Vec::len)
                    )),
                    None,
                );
                if job.remaining.as_ref().is_some_and(Vec::is_empty) {
                    if let Ok(transfer) = parse_transfer(&job.reference.transfer_id) {
                        let _ = ui.unbounded_send(UiEvent::FilePasteLinked {
                            paste_id: id,
                            transfer_id: transfer,
                            can_retry: false,
                        });
                        self.completed.insert(
                            (transfer, job.destination.identity()),
                            std::time::Instant::now(),
                        );
                    }
                    self.jobs.remove(&id);
                }
            }
        }
    }
    fn fail(&mut self, id: TransferId, error: String, ui: &UnboundedSender<UiEvent>) {
        if let Some(job) = self.jobs.get_mut(&id) {
            job.terminal = true;
            emit_receive_transfer_status(
                ui,
                id,
                job.reference.name.clone(),
                error,
                TransferState::Failed,
                0,
                job.reference.total_bytes,
            );
            if job.remaining.is_none() && !job.running {
                self.jobs.remove(&id);
            }
        }
    }
    pub fn source_failed(&mut self, id: TransferId, ui: &UnboundedSender<UiEvent>) {
        let ids = self
            .jobs
            .values()
            .filter(|j| {
                j.reference.transfer_id == id.to_string() && j.remaining.is_none() && !j.terminal
            })
            .map(|j| j.id)
            .collect::<Vec<_>>();
        for id in ids {
            self.fail(id, "接收未完成或来源已断开；请重新复制文件".into(), ui);
        }
    }
    pub fn reconcile_waiting(
        &mut self,
        policy: &FilePolicy,
        incoming: &HashMap<(DeviceId, TransferId), IncomingFileClipboard>,
        ui: &UnboundedSender<UiEvent>,
    ) {
        let ids = self
            .jobs
            .values()
            .filter(|j| !j.terminal && j.remaining.is_none())
            .filter_map(|j| parse_transfer(&j.reference.transfer_id).ok())
            .filter(|id| {
                !policy.has_requested(*id) && !incoming.keys().any(|(_, transfer)| transfer == id)
            })
            .collect::<Vec<_>>();
        for id in ids {
            self.source_failed(id, ui);
        }
    }
    /// Returns the underlying transfer only when no other destination needs it.
    pub fn cancel(&mut self, id: TransferId, ui: &UnboundedSender<UiEvent>) -> Option<TransferId> {
        let job = self.jobs.get_mut(&id)?;
        job.cancel.store(true, Ordering::Release);
        let transfer = parse_transfer(&job.reference.transfer_id).ok()?;
        if !job.running {
            job.terminal = true;
            emit_receive_transfer_status(
                ui,
                id,
                job.reference.name.clone(),
                "粘贴已取消".into(),
                TransferState::Cancelled,
                job.saved,
                job.total,
            );
        }
        if job.remaining.is_none() && !job.running {
            self.jobs.remove(&id);
        }
        (!self.jobs.values().any(|j| {
            j.reference.transfer_id == transfer.to_string()
                && !j.terminal
                && !j.cancel.load(Ordering::Acquire)
        }))
        .then_some(transfer)
    }
    pub fn retry(
        &mut self,
        id: TransferId,
        ui: &UnboundedSender<UiEvent>,
        diagnostics: &Diagnostics,
    ) -> Result<(), String> {
        let job = self.jobs.get_mut(&id).ok_or("找不到粘贴任务")?;
        if job.running {
            return Ok(());
        }
        if job.remaining.as_ref().is_none_or(Vec::is_empty) {
            return Err("没有可重试的缓存文件，请重新复制".into());
        }
        // A retry is a new attempt/row. Late terminal/progress events can never
        // resurrect the old row, whose state remains final.
        let old = self.jobs.remove(&id).expect("checked job");
        let new_id = TransferId::new();
        let transfer = parse_transfer(&old.reference.transfer_id)?;
        let lease = self.retain(new_id, old.remaining.as_ref().expect("checked remaining"))?;
        let job = Job {
            id: new_id,
            cancel: Arc::new(AtomicBool::new(false)),
            terminal: false,
            saved: 0,
            total: 0,
            lease: Some(lease),
            ..old
        };
        let _ = ui.unbounded_send(UiEvent::FilePasteLinked {
            paste_id: id,
            transfer_id: transfer,
            can_retry: false,
        });
        let _ = ui.unbounded_send(UiEvent::FilePasteLinked {
            paste_id: new_id,
            transfer_id: transfer,
            can_retry: true,
        });
        self.jobs.insert(new_id, job);
        self.start(new_id, ui, diagnostics);
        Ok(())
    }
}
impl Drop for FinderPasteJobs {
    fn drop(&mut self) {
        for job in self.jobs.values() {
            job.cancel.store(true, Ordering::Release);
        }
    }
}
pub(super) fn parse_transfer(value: &str) -> Result<TransferId, String> {
    uuid::Uuid::parse_str(value)
        .map(TransferId::from_uuid)
        .map_err(|_| "文件传输标识无效".into())
}
