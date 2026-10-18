//! Typed workflow execution. Owns immutable jobs, cancellation and cache leases;
//! native clients only edit definitions and observe typed results.
use crate::tokio_bridge::TokioBridge;
use crate::workflow_image::{self as image_processor, ProcessedImage};
use deskfuse_core::workflow::*;
use deskfuse_platform::{
    PasteboardItem, PasteboardRepresentation, PasteboardSnapshot, WorkflowExecutionPort,
};
use deskfuse_transfer::TransferId;
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[path = "workflow_execution.rs"]
mod execution;

const MAX_BYTES: u64 = 128 * 1024 * 1024;
#[derive(Clone, Debug)]
pub enum WorkflowInput {
    Clipboard,
    History(u64),
    File(String),
    Files(Vec<String>),
    Text(String),
    Snapshot(PasteboardSnapshot),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkflowOutputMode {
    Configured,
    Copy,
    ReturnFile,
    ReturnResult,
}
#[derive(Clone, Debug)]
pub struct SystemShortcut {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Debug, Default)]
pub struct WorkflowRun {
    pub id: String,
    pub workflow_id: String,
    pub step_index: u32,
    pub phase: String,
    pub message: String,
    pub output_path: String,
    pub output_paths: Vec<String>,
    pub output_text: String,
    pub output_kind: WorkflowContentType,
    pub output_mime: String,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub width: u32,
    pub height: u32,
}
struct Job {
    run: WorkflowRun,
    cancel: Arc<AtomicBool>,
    chain: Vec<String>,
    external: bool,
    independent: bool,
    result: Option<PasteboardSnapshot>,
}
struct Lease {
    id: TransferId,
    until: Instant,
}
pub struct WorkflowService {
    bridge: Arc<TokioBridge>,
    execution: Arc<dyn WorkflowExecutionPort>,
    jobs: Mutex<BTreeMap<String, Job>>,
    handles: Mutex<Vec<thread::JoinHandle<()>>>,
    leases: Mutex<Vec<Lease>>,
    stopped: AtomicBool,
}
impl WorkflowService {
    pub fn new(
        bridge: Arc<TokioBridge>,
        execution: Arc<dyn WorkflowExecutionPort>,
    ) -> Result<Arc<Self>, String> {
        crate::workflow_catalog::global()?;
        Ok(Arc::new(Self {
            bridge,
            execution,
            jobs: Mutex::new(BTreeMap::new()),
            handles: Mutex::new(Vec::new()),
            leases: Mutex::new(Vec::new()),
            stopped: AtomicBool::new(false),
        }))
    }
    pub fn list(&self) -> Result<Vec<Workflow>, String> {
        crate::workflow_catalog::global()?.list()
    }
    pub fn system_shortcuts(&self) -> Result<Vec<SystemShortcut>, String> {
        self.execution
            .list_shortcuts()
            .map_err(|error| error.to_string())
            .map(|shortcuts| {
                shortcuts
                    .into_iter()
                    .map(|shortcut| SystemShortcut {
                        id: shortcut.id,
                        name: shortcut.name,
                    })
                    .collect()
            })
    }
    pub fn save(&self, workflow: Workflow) -> Result<(), String> {
        crate::workflow_catalog::global()?.save(workflow)
    }
    pub fn delete(&self, id: &str) -> Result<(), String> {
        crate::workflow_catalog::global()?.delete(id)
    }
    pub fn status(&self, id: &str) -> Result<WorkflowRun, String> {
        self.reap();
        self.jobs
            .lock()
            .map_err(|_| "流程状态不可用")?
            .get(id)
            .map(|j| j.run.clone())
            .ok_or_else(|| "找不到流程任务".into())
    }
    pub fn cancel(&self, id: &str) -> Result<(), String> {
        let jobs = self.jobs.lock().map_err(|_| "流程状态不可用")?;
        let job = jobs.get(id).ok_or("找不到流程任务")?;
        for candidate in jobs.values() {
            if candidate.chain.starts_with(&job.chain) {
                candidate.cancel.store(true, Ordering::Release);
            }
        }
        Ok(())
    }
    fn update(&self, id: &str, edit: impl FnOnce(&mut WorkflowRun)) {
        if let Ok(mut jobs) = self.jobs.lock()
            && let Some(job) = jobs.get_mut(id)
        {
            edit(&mut job.run);
            self.bridge.workflow_notify(job.run.clone());
        }
    }
    fn external(&self, id: &str, active: bool) {
        if let Ok(mut jobs) = self.jobs.lock()
            && let Some(job) = jobs.get_mut(id)
        {
            job.external = active;
        }
    }
    pub fn start(
        self: &Arc<Self>,
        workflow_id: String,
        input: WorkflowInput,
        output: WorkflowOutputMode,
        target_pid: i32,
    ) -> Result<String, String> {
        if self
            .jobs
            .lock()
            .map_err(|_| "流程状态不可用")?
            .values()
            .any(|job| job.external && job.run.phase == "running")
        {
            return Err(
                "系统快捷指令正在运行，不允许回调或重入 DeskFuse 工作流；请先取消当前流程".into(),
            );
        }
        let workflow = self
            .list()?
            .into_iter()
            .find(|w| w.id == workflow_id && w.enabled)
            .ok_or("流程不存在或已停用")?;
        let workflow = crate::workflow_catalog::global()?.resolve_local(workflow)?;
        self.start_definition(
            workflow,
            input,
            output,
            target_pid,
            Arc::new(AtomicBool::new(false)),
        )
    }
    /// Starts an immutable definition; shared callers supply their own cancellation scope.
    pub fn start_definition(
        self: &Arc<Self>,
        workflow: Workflow,
        input: WorkflowInput,
        output: WorkflowOutputMode,
        target_pid: i32,
        cancel: Arc<AtomicBool>,
    ) -> Result<String, String> {
        if self.stopped.load(Ordering::Acquire) {
            return Err("引擎已停止".into());
        }
        self.reap();
        let workflow_id = workflow.id.clone();
        workflow.validate()?;
        let id = TransferId::new().to_string();
        {
            let mut jobs = self.jobs.lock().map_err(|_| "流程状态不可用")?;
            // Shortcuts does not propagate a trustworthy parent invocation ID through
            // a plain image file. Never infer nesting from another caller's image.
            if output == WorkflowOutputMode::ReturnFile
                && jobs
                    .values()
                    .any(|j| j.external && j.run.phase == "running")
            {
                return Err(
                    "系统快捷指令正在运行，不允许回调或重入 DeskFuse 工作流；请先取消当前流程"
                        .into(),
                );
            }
            for job in jobs.values() {
                if output != WorkflowOutputMode::ReturnResult
                    && !job.independent
                    && job.run.phase == "running"
                {
                    job.cancel.store(true, Ordering::Release);
                }
            }
            let chain = vec![id.clone()];
            jobs.insert(
                id.clone(),
                Job {
                    run: WorkflowRun {
                        id: id.clone(),
                        workflow_id,
                        phase: "running".into(),
                        message: "准备内容…".into(),
                        ..Default::default()
                    },
                    cancel: cancel.clone(),
                    chain,
                    external: false,
                    independent: output == WorkflowOutputMode::ReturnResult,
                    result: None,
                },
            );
            if jobs.len() > 64 {
                let old = jobs
                    .iter()
                    .filter(|(_, j)| j.run.phase != "running")
                    .map(|(id, _)| id.clone())
                    .take(jobs.len() - 64)
                    .collect::<Vec<_>>();
                for id in old {
                    jobs.remove(&id);
                }
            }
        }
        let service = self.clone();
        let job_id = id.clone();
        let handle = thread::Builder::new()
            .name("deskfuse-workflow".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    service.execute(&job_id, &workflow, input, output, target_pid, &cancel)
                }));
                let result = result.unwrap_or_else(|_| Err("流程处理失败，任务已停止".into()));
                service.external(&job_id, false);
                service.update(&job_id, |state| match result {
                    Ok(()) => {
                        state.phase = "completed".into();
                        if state.message.is_empty() {
                            state.message = "处理完成".into();
                        }
                    }
                    Err(error) => {
                        state.phase = if cancel.load(Ordering::Acquire) {
                            "cancelled"
                        } else {
                            "failed"
                        }
                        .into();
                        state.message = error;
                    }
                });
            })
            .map_err(|e| e.to_string());
        match handle {
            Ok(handle) => {
                self.handles
                    .lock()
                    .map_err(|_| "流程线程状态不可用")?
                    .push(handle);
                Ok(id)
            }
            Err(error) => {
                self.update(&id, |r| {
                    r.phase = "failed".into();
                    r.message = error.clone();
                });
                Err(error)
            }
        }
    }
    /// The caller must retain returned cache paths before beginning a long export.
    pub fn result_snapshot(&self, run_id: &str) -> Result<PasteboardSnapshot, String> {
        let jobs = self.jobs.lock().map_err(|_| "流程状态不可用")?;
        let job = jobs.get(run_id).ok_or("找不到流程任务")?;
        if job.run.phase != "completed" {
            return Err(job.run.message.clone());
        }
        let snapshot = job.result.clone().ok_or("流程没有可用结果")?;
        if snapshot.file_urls.iter().any(|p| !Path::new(p).exists()) {
            return Err("流程结果缓存已失效，请重新运行".into());
        }
        Ok(snapshot)
    }
    fn execute(
        &self,
        id: &str,
        workflow: &Workflow,
        input: WorkflowInput,
        output: WorkflowOutputMode,
        target_pid: i32,
        cancel: &Arc<AtomicBool>,
    ) -> Result<(), String> {
        self.execute_generic(id, workflow, input, output, target_pid, cancel)
    }
    fn reap(&self) {
        if let Ok(mut leases) = self.leases.lock() {
            leases.retain(|lease| {
                if Instant::now() > lease.until {
                    self.bridge.release_transfer_cache_paths(lease.id);
                    false
                } else {
                    true
                }
            });
        }
        if let Ok(mut handles) = self.handles.lock() {
            let mut i = 0;
            while i < handles.len() {
                if handles[i].is_finished() {
                    let h = handles.swap_remove(i);
                    let _ = h.join();
                } else {
                    i += 1;
                }
            }
        }
    }
    pub fn shutdown(&self) {
        self.stopped.store(true, Ordering::Release);
        if let Ok(jobs) = self.jobs.lock() {
            for job in jobs.values() {
                job.cancel.store(true, Ordering::Release);
            }
        }
        if let Ok(mut handles) = self.handles.lock() {
            for handle in handles.drain(..) {
                let _ = handle.join();
            }
        }
        if let Ok(mut leases) = self.leases.lock() {
            for lease in leases.drain(..) {
                self.bridge.release_transfer_cache_paths(lease.id);
            }
        }
    }
}
struct Workspace {
    bridge: Arc<TokioBridge>,
    transfer: TransferId,
    lease: Option<TransferId>,
    path: PathBuf,
    keep: bool,
}
impl Drop for Workspace {
    fn drop(&mut self) {
        if !self.keep {
            let _ = fs::remove_dir_all(&self.path);
        }
        let _ = self.bridge.workflow_cache_finish(self.transfer);
        if let Some(id) = self.lease {
            self.bridge.release_transfer_cache_paths(id);
        }
    }
}
fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("流程已取消或被新的操作替代".into())
    } else {
        Ok(())
    }
}
fn safe_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|v| v.to_str())
        .unwrap_or("DeskFuse-image")
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '/' | ':' | '\\'))
        .take(80)
        .collect()
}
fn file_stamp(path: &Path) -> Result<(u64, std::time::SystemTime), String> {
    let metadata = fs::metadata(path).map_err(|_| "源图片文件不可用")?;
    Ok((
        metadata.len(),
        metadata.modified().map_err(|e| e.to_string())?,
    ))
}
fn read_image_file(path: &Path) -> Result<Vec<u8>, String> {
    let before = file_stamp(path)?;
    let metadata = fs::metadata(path).map_err(|_| "图片文件不可用")?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        return Err("请提供单个不超过 128 MiB 的图片文件".into());
    }
    let mut data = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    if data.len() as u64 > MAX_BYTES {
        return Err("图片文件超过处理容量".into());
    }
    if file_stamp(path)? != before || data.len() as u64 != before.0 {
        return Err("读取期间源图片已变化，请重新运行".into());
    }
    Ok(data)
}
fn snapshot_image(snapshot: &PasteboardSnapshot) -> Result<Vec<u8>, String> {
    for kind in [
        "public.jpeg",
        "org.webmproject.webp",
        "public.heic",
        "public.heif",
        "public.png",
        "public.tiff",
        "com.compuserve.gif",
    ] {
        if let Some(rep) = snapshot
            .items
            .iter()
            .flat_map(|i| &i.representations)
            .find(|r| r.type_name == kind)
        {
            if rep.data.len() as u64 > MAX_BYTES {
                return Err("图片超过 128 MiB".into());
            }
            return Ok(rep.data.clone());
        }
    }
    snapshot
        .png
        .clone()
        .filter(|b| b.len() as u64 <= MAX_BYTES)
        .ok_or_else(|| "剪贴板没有可处理的单张图片".into())
}
fn fit_target(
    data: &[u8],
    format: ImageFormat,
    quality: u8,
    target: u64,
    cancel: &AtomicBool,
) -> Result<ProcessedImage, String> {
    let info = image_processor::inspect_image(data)?;
    let original_edge = info.width.max(info.height);
    let floor = original_edge.min(256);
    let mut edge = original_edge;
    for _ in 0..32 {
        check_cancel(cancel)?;
        if format != ImageFormat::Png {
            let mut low = quality.min(40);
            let mut high = quality;
            let mut best = None;
            while low <= high {
                check_cancel(cancel)?;
                let mid = low + (high - low) / 2;
                let image = image_processor::encode_image(data, format, mid, Some(edge))?;
                if image.data.len() as u64 <= target {
                    best = Some(image);
                    low = mid.saturating_add(1);
                } else {
                    if mid == 0 {
                        break;
                    }
                    high = mid - 1;
                }
            }
            if let Some(image) = best {
                return Ok(image);
            }
        } else {
            let image = image_processor::encode_image(data, format, quality, Some(edge))?;
            if image.data.len() as u64 <= target {
                return Ok(image);
            }
        }
        if edge == floor {
            break;
        }
        edge = ((u64::from(edge) * 80 / 100) as u32).max(floor);
    }
    Err("在最低质量和尺寸限制内仍无法达到最终体积，请增大目标上限".into())
}
