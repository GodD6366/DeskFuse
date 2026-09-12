//! Image workflow execution. Owns immutable jobs, cancellation and cache leases;
//! native clients only edit definitions and observe typed results.
use crate::tokio_bridge::TokioBridge;
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use unidesk_core::image_workflow::*;
use unidesk_platform::{Pasteboard, PasteboardItem, PasteboardRepresentation, PasteboardSnapshot};
use unidesk_platform_macos::{
    MacOSPasteboard,
    image_processor::{self, ProcessedImage},
};
use unidesk_store::SQLiteStore;
use unidesk_transfer::TransferId;

const MAX_BYTES: u64 = 128 * 1024 * 1024;
#[derive(Clone, Debug)]
pub enum WorkflowInput {
    Clipboard,
    History(u64),
    File(String),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkflowOutputMode {
    Configured,
    Copy,
    ReturnFile,
}
#[derive(Clone, Debug)]
pub struct SystemShortcut {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Debug, Default)]
pub struct ImageWorkflowRun {
    pub id: String,
    pub workflow_id: String,
    pub step_index: u32,
    pub phase: String,
    pub message: String,
    pub output_path: String,
    pub output_mime: String,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub width: u32,
    pub height: u32,
}
struct Job {
    run: ImageWorkflowRun,
    cancel: Arc<AtomicBool>,
    chain: Vec<String>,
    external: bool,
}
struct Lease {
    id: TransferId,
    until: Instant,
}
pub struct ImageWorkflowService {
    bridge: Arc<TokioBridge>,
    definitions: Mutex<Vec<ImageWorkflow>>,
    jobs: Mutex<BTreeMap<String, Job>>,
    handles: Mutex<Vec<thread::JoinHandle<()>>>,
    leases: Mutex<Vec<Lease>>,
    stopped: AtomicBool,
}
impl ImageWorkflowService {
    pub fn new(bridge: Arc<TokioBridge>) -> Result<Arc<Self>, String> {
        let definitions = SQLiteStore::new()
            .map_err(|e| e.to_string())?
            .image_workflows()
            .map_err(|e| e.to_string())?;
        for definition in &definitions {
            definition.validate()?;
        }
        Ok(Arc::new(Self {
            bridge,
            definitions: Mutex::new(definitions),
            jobs: Mutex::new(BTreeMap::new()),
            handles: Mutex::new(Vec::new()),
            leases: Mutex::new(Vec::new()),
            stopped: AtomicBool::new(false),
        }))
    }
    pub fn list(&self) -> Result<Vec<ImageWorkflow>, String> {
        Ok(self
            .definitions
            .lock()
            .map_err(|_| "流程设置不可用")?
            .clone())
    }
    pub fn save(&self, workflow: ImageWorkflow) -> Result<(), String> {
        workflow.validate()?;
        let mut stored = self.definitions.lock().map_err(|_| "流程设置不可用")?;
        if stored.len() >= 100 && !stored.iter().any(|w| w.id == workflow.id) {
            return Err("最多保存 100 个流程".into());
        }
        if workflow.enabled
            && workflow.shortcut.is_some()
            && stored
                .iter()
                .any(|w| w.enabled && w.id != workflow.id && w.shortcut == workflow.shortcut)
        {
            return Err("该快捷键已被其他流程占用".into());
        }
        let mut next = stored.clone();
        if let Some(w) = next.iter_mut().find(|w| w.id == workflow.id) {
            *w = workflow;
        } else {
            next.push(workflow);
        }
        SQLiteStore::new()
            .map_err(|e| e.to_string())?
            .set_image_workflows(&next)
            .map_err(|e| e.to_string())?;
        *stored = next;
        Ok(())
    }
    pub fn delete(&self, id: &str) -> Result<(), String> {
        let mut stored = self.definitions.lock().map_err(|_| "流程设置不可用")?;
        let next = stored
            .iter()
            .filter(|w| w.id != id)
            .cloned()
            .collect::<Vec<_>>();
        SQLiteStore::new()
            .map_err(|e| e.to_string())?
            .set_image_workflows(&next)
            .map_err(|e| e.to_string())?;
        *stored = next;
        Ok(())
    }
    pub fn status(&self, id: &str) -> Result<ImageWorkflowRun, String> {
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
    fn update(&self, id: &str, edit: impl FnOnce(&mut ImageWorkflowRun)) {
        if let Ok(mut jobs) = self.jobs.lock() {
            if let Some(job) = jobs.get_mut(id) {
                edit(&mut job.run);
                self.bridge.workflow_notify(job.run.clone());
            }
        }
    }
    fn external(&self, id: &str, active: bool) {
        if let Ok(mut jobs) = self.jobs.lock() {
            if let Some(job) = jobs.get_mut(id) {
                job.external = active;
            }
        }
    }
    pub fn start(
        self: &Arc<Self>,
        workflow_id: String,
        input: WorkflowInput,
        output: WorkflowOutputMode,
        target_pid: i32,
    ) -> Result<String, String> {
        if self.stopped.load(Ordering::Acquire) {
            return Err("引擎已停止".into());
        }
        self.reap();
        let workflow = self
            .list()?
            .into_iter()
            .find(|w| w.id == workflow_id && w.enabled)
            .ok_or("流程不存在或已停用")?;
        workflow.validate()?;
        let id = TransferId::new().to_string();
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut jobs = self.jobs.lock().map_err(|_| "流程状态不可用")?;
            // Shortcuts does not propagate a trustworthy parent invocation ID through
            // a plain image file. Never infer nesting from another caller's image.
            if jobs
                .values()
                .any(|j| j.external && j.run.phase == "running")
            {
                return Err(
                    "系统快捷指令正在运行，不允许回调或重入 uniDesk 图片流程；请先取消当前流程"
                        .into(),
                );
            }
            for job in jobs.values() {
                if job.run.phase == "running" {
                    job.cancel.store(true, Ordering::Release);
                }
            }
            let chain = vec![workflow_id.clone()];
            jobs.insert(
                id.clone(),
                Job {
                    run: ImageWorkflowRun {
                        id: id.clone(),
                        workflow_id,
                        phase: "running".into(),
                        message: "准备图片…".into(),
                        ..Default::default()
                    },
                    cancel: cancel.clone(),
                    chain,
                    external: false,
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
            .name("unidesk-image-workflow".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    service.execute(&job_id, &workflow, input, output, target_pid, &cancel)
                }));
                let result = result.unwrap_or_else(|_| Err("图片处理失败，任务已停止".into()));
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
    fn execute(
        &self,
        id: &str,
        workflow: &ImageWorkflow,
        input: WorkflowInput,
        output: WorkflowOutputMode,
        target_pid: i32,
        cancel: &Arc<AtomicBool>,
    ) -> Result<(), String> {
        check_cancel(cancel)?;
        let mut expected = MacOSPasteboard::new()
            .change_count()
            .map_err(|e| e.to_string())?;
        let (mut data, from_file, stem, source_path) = match input {
            WorkflowInput::File(path) => {
                let path = PathBuf::from(path);
                let bytes = read_image_file(&path)?;
                let stem = safe_stem(&path);
                (bytes, true, stem, Some(path))
            }
            source => {
                let snapshot = self.bridge.workflow_source(
                    match source {
                        WorkflowInput::History(id) => Some(id),
                        _ => None,
                    },
                    cancel.clone(),
                )?;
                expected = snapshot.change_count;
                if snapshot.file_urls.len() > 1 || snapshot.items.len() > 1 {
                    return Err("图片流程仅支持单张图片，不支持批量内容".into());
                }
                if let Some(path) = snapshot.file_urls.first() {
                    let path = PathBuf::from(path);
                    let bytes = read_image_file(&path)?;
                    let stem = safe_stem(&path);
                    (bytes, true, stem, Some(path))
                } else {
                    (
                        snapshot_image(&snapshot)?,
                        false,
                        "uniDesk-image".into(),
                        None,
                    )
                }
            }
        };
        let input_len = data.len() as u64;
        let source_fingerprint = source_path.as_ref().map(|p| file_stamp(p)).transpose()?;
        let info = image_processor::inspect_image(&data)?;
        let budget = (u64::from(info.width) * u64::from(info.height) * 8 + input_len + 1024 * 1024)
            .min(MAX_BYTES * 3);
        let transfer = TransferId::new();
        // Register an external lease before making the directory visible to eviction.
        let cache_root = dirs::cache_dir()
            .ok_or("无法确定图片缓存目录")?
            .join("com.unidesk.app/clipboard")
            .join(transfer.to_string());
        let lease = self
            .bridge
            .retain_transfer_cache_paths(vec![cache_root.to_string_lossy().into_owned()])?;
        let destination = match self.bridge.workflow_cache_reserve(transfer, budget) {
            Ok(p) => p,
            Err(e) => {
                self.bridge.release_transfer_cache_paths(lease);
                return Err(e);
            }
        };
        let mut workspace = Workspace {
            bridge: self.bridge.clone(),
            transfer,
            lease: Some(lease),
            path: destination.clone(),
            keep: false,
        };
        fs::create_dir_all(&destination).map_err(|e| e.to_string())?;
        self.update(id, |r| {
            r.input_bytes = input_len;
            r.width = info.width;
            r.height = info.height;
        });
        let mut format = info.format.unwrap_or(if info.has_alpha {
            ImageFormat::Png
        } else {
            ImageFormat::Jpeg
        });
        let mut explicit = false;
        let mut quality = 85;
        let mut target: Option<u64> = None;
        let mut processed: Option<ProcessedImage> = None;
        for (index, step) in workflow.steps.iter().enumerate() {
            check_cancel(cancel)?;
            self.update(id, |r| {
                r.step_index = index as u32;
                r.message = match step.kind {
                    WorkflowStepKind::Compress => "压缩图片",
                    WorkflowStepKind::Resize => "调整尺寸",
                    WorkflowStepKind::Convert => "转换格式",
                    WorkflowStepKind::SystemShortcut => "等待系统快捷指令",
                    _ => "准备最终输出",
                }
                .into();
            });
            match step.kind {
                WorkflowStepKind::Compress => {
                    if let Some(limit) = step.target_bytes {
                        target = Some(target.map_or(limit, |previous| previous.min(limit)));
                        if !explicit {
                            format = if image_processor::inspect_image(&data)?.has_alpha {
                                ImageFormat::Png
                            } else {
                                ImageFormat::Jpeg
                            };
                        }
                    } else {
                        quality = step.quality;
                        if !explicit {
                            format = if image_processor::inspect_image(&data)?.has_alpha {
                                ImageFormat::Png
                            } else {
                                ImageFormat::Jpeg
                            };
                        }
                        let next = image_processor::encode_image(&data, format, quality, None)?;
                        if next.data.len() < data.len() {
                            data = next.data.clone();
                            processed = Some(next);
                        }
                    }
                }
                WorkflowStepKind::Resize => {
                    let current = image_processor::inspect_image(&data)?;
                    let edge = step.max_edge.unwrap_or_else(|| {
                        ((u64::from(current.width.max(current.height))
                            * u64::from(step.scale_percent.unwrap_or(100)))
                            / 100)
                            .max(1) as u32
                    });
                    let next = image_processor::encode_image(&data, format, quality, Some(edge))?;
                    data = next.data.clone();
                    processed = Some(next);
                }
                WorkflowStepKind::Convert => {
                    format = step.format;
                    explicit = true;
                    quality = step.quality;
                    let next = image_processor::encode_image(&data, format, quality, None)?;
                    data = next.data.clone();
                    processed = Some(next);
                }
                WorkflowStepKind::SystemShortcut => {
                    let actual = image_processor::inspect_image(&data)?.format;
                    if actual.is_none() {
                        let normalized =
                            image_processor::encode_image(&data, ImageFormat::Png, 100, None)?;
                        data = normalized.data.clone();
                    }
                    let extension = actual.map(ImageFormat::extension).unwrap_or("png");
                    let source = destination.join(format!("input.{extension}"));
                    let output_path = destination.join("shortcut-output");
                    fs::write(&source, &data).map_err(|e| e.to_string())?;
                    self.external(id, true);
                    let result =
                        run_shortcut(&step.shortcut_id, &source, &output_path, cancel, budget);
                    self.external(id, false);
                    data = result?;
                    let _ = fs::remove_file(&source);
                    let _ = fs::remove_file(&output_path);
                    let info = image_processor::inspect_image(&data)?;
                    format = info.format.unwrap_or(if info.has_alpha {
                        ImageFormat::Png
                    } else {
                        ImageFormat::Jpeg
                    });
                    explicit = true;
                    processed = None;
                }
                WorkflowStepKind::Copy | WorkflowStepKind::Paste => {}
            }
        }
        check_cancel(cancel)?;
        let mut result = if let Some(limit) = target.filter(|n| data.len() as u64 > *n) {
            fit_target(&data, format, quality, limit, cancel)?
        } else if let Some(image) = processed {
            image
        } else {
            // Keep original bytes when already within the target; TIFF is only a compatibility derivative.
            let info = image_processor::inspect_image(&data)?;
            if let Some(actual) = info.format {
                image_processor::processed_representation(data.clone(), actual)?
            } else {
                image_processor::encode_image(&data, format, quality, None)?
            }
        };
        // A final explicit format must not be accidentally replaced by a compatibility TIFF.
        if let Some(limit) = target {
            if result.data.len() as u64 > limit {
                return Err("最终图片无法满足目标体积，请增大上限".into());
            }
        }
        check_cancel(cancel)?;
        if let (Some(path), Some(stamp)) = (&source_path, &source_fingerprint) {
            if &file_stamp(path)? != stamp {
                return Err("源文件已变化，请重新运行流程".into());
            }
        }
        let output_path =
            destination.join(format!("{}-processed.{}", stem, result.format.extension()));
        fs::write(&output_path, &result.data).map_err(|e| e.to_string())?;
        let terminal = workflow.steps.last().ok_or("缺少输出步骤")?;
        let will_paste =
            output == WorkflowOutputMode::Configured && terminal.kind == WorkflowStepKind::Paste;
        if output != WorkflowOutputMode::ReturnFile {
            if will_paste && target_pid <= 0 {
                return Err("没有可用的目标应用，请使用全局快捷键或选择复制输出".into());
            }
            let file_output = terminal.output_shape == ImageOutputShape::File
                || (terminal.output_shape == ImageOutputShape::FollowSource && from_file);
            let snapshot = if file_output {
                PasteboardSnapshot {
                    file_urls: vec![output_path.to_string_lossy().into_owned()],
                    ..Default::default()
                }
            } else {
                PasteboardSnapshot {
                    items: vec![PasteboardItem {
                        representations: vec![
                            PasteboardRepresentation {
                                type_name: result.format.uti().into(),
                                data: std::mem::take(&mut result.data),
                            },
                            PasteboardRepresentation {
                                type_name: "public.tiff".into(),
                                data: std::mem::take(&mut result.tiff),
                            },
                        ],
                    }],
                    ..Default::default()
                }
            };
            let count = self.bridge.workflow_commit(
                snapshot,
                expected,
                cancel.clone(),
                if will_paste { target_pid } else { 0 },
            )?;
            // A successful commit owns the file even if the receiving app later changes focus.
            workspace.keep = true;
            if will_paste {
                let deadline = Instant::now() + Duration::from_secs(3);
                while !unidesk_platform_macos::workflow_modifiers_released() {
                    check_cancel(cancel)?;
                    if Instant::now() > deadline {
                        return Err("快捷键仍未释放，结果已复制，请手动粘贴".into());
                    }
                    thread::sleep(Duration::from_millis(15));
                }
                check_cancel(cancel)?;
                unidesk_platform_macos::workflow_paste_if_current(target_pid, count)?;
            }
        }
        let output_bytes = fs::metadata(&output_path).map_err(|e| e.to_string())?.len();
        self.update(id, |r| {
            r.output_path = output_path.to_string_lossy().into_owned();
            r.output_mime = result.format.mime().into();
            r.output_bytes = output_bytes;
            r.width = result.width;
            r.height = result.height;
            r.message = if will_paste {
                "已发送粘贴；结果由目标应用接收"
            } else if output == WorkflowOutputMode::ReturnFile {
                "处理完成"
            } else {
                "已复制处理结果"
            }
            .into();
        });
        workspace.keep = true;
        if let Some(lease) = workspace.lease.take() {
            self.leases
                .lock()
                .map_err(|_| "缓存租约不可用")?
                .push(Lease {
                    id: lease,
                    until: Instant::now() + Duration::from_secs(120),
                });
        }
        Ok(())
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
        .unwrap_or("uniDesk-image")
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
pub fn system_shortcuts() -> Result<Vec<SystemShortcut>, String> {
    let output = Command::new("/usr/bin/shortcuts")
        .args(["list", "--show-identifiers"])
        .output()
        .map_err(|_| "无法读取系统快捷指令")?;
    if !output.status.success() {
        return Err("读取快捷指令失败，请先打开系统快捷指令应用".into());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let (name, id) = line.rsplit_once(" (")?;
            Some(SystemShortcut {
                name: name.to_owned(),
                id: id.strip_suffix(')')?.to_owned(),
            })
        })
        .collect())
}
fn run_shortcut(
    id: &str,
    input: &Path,
    output: &Path,
    cancel: &AtomicBool,
    budget: u64,
) -> Result<Vec<u8>, String> {
    if !system_shortcuts()?.iter().any(|s| s.id == id) {
        return Err("系统快捷指令已删除，请重新选择".into());
    }
    let mut child = Command::new("/usr/bin/shortcuts")
        .arg("run")
        .arg(id)
        .arg("--input-path")
        .arg(input)
        .arg("--output-path")
        .arg(output)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "无法运行系统快捷指令")?;
    let start = Instant::now();
    loop {
        if cancel.load(Ordering::Acquire) || start.elapsed() > Duration::from_secs(600) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("系统快捷指令已取消或等待超时".into());
        }
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            if !status.success() {
                return Err("系统快捷指令执行失败或已取消".into());
            }
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    let meta = fs::symlink_metadata(output).map_err(|_| "系统快捷指令没有返回图片文件")?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > budget.min(MAX_BYTES) {
        return Err("系统快捷指令必须返回单张有效图片，不能返回目录、多文件或超限内容".into());
    }
    let bytes = read_image_file(output)?;
    image_processor::inspect_image(&bytes)?;
    Ok(bytes)
}
