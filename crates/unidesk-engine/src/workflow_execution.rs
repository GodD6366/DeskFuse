//! General workflow payloads. All file-changing steps operate on leased private copies.
use super::*;
use std::io::Write;
use unidesk_platform::{WorkflowExecutionPlan, WorkflowOutputMode as PlatformWorkflowOutputMode};

#[derive(Clone)]
enum Payload {
    Text(String),
    Files { paths: Vec<PathBuf>, image: bool },
}
impl Payload {
    fn kind(&self) -> WorkflowContentType {
        match self {
            Self::Text(_) => WorkflowContentType::Text,
            Self::Files { image: true, .. } => WorkflowContentType::Image,
            Self::Files { .. } => WorkflowContentType::Files,
        }
    }
    fn paths(&self) -> Vec<PathBuf> {
        match self {
            Self::Text(_) => Vec::new(),
            Self::Files { paths, .. } => paths.clone(),
        }
    }
}
#[derive(Default)]
struct ImagePolicy {
    quality: u8,
    target: Option<u64>,
    explicit_format: Option<ImageFormat>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    bytes: u64,
    modified: std::time::SystemTime,
    directory: bool,
}
/// Includes directory membership through the full path set, not just root mtime.
type Manifest = BTreeMap<PathBuf, Stamp>;
struct SourceLease {
    bridge: Arc<TokioBridge>,
    id: TransferId,
}
impl Drop for SourceLease {
    fn drop(&mut self) {
        self.bridge.release_transfer_cache_paths(self.id);
    }
}
impl WorkflowService {
    /// Background-only convenience for sharing; never call from the bridge worker.
    pub fn process_share(
        self: &Arc<Self>,
        definition: Workflow,
        snapshot: PasteboardSnapshot,
        cancel: Arc<AtomicBool>,
    ) -> Result<PasteboardSnapshot, String> {
        let id = self.start_definition(
            definition,
            WorkflowInput::Snapshot(snapshot),
            WorkflowOutputMode::ReturnResult,
            0,
            cancel.clone(),
        )?;
        loop {
            check_cancel(&cancel)?;
            let status = self.status(&id)?;
            match status.phase.as_str() {
                "completed" => return self.result_snapshot(&id),
                "cancelled" | "failed" => return Err(status.message),
                _ => thread::sleep(Duration::from_millis(30)),
            }
        }
    }
    pub(super) fn execute_generic(
        &self,
        id: &str,
        workflow: &Workflow,
        input: WorkflowInput,
        output: WorkflowOutputMode,
        target_pid: i32,
        cancel: &Arc<AtomicBool>,
    ) -> Result<(), String> {
        check_cancel(cancel)?;
        let output_port = &self.output_port;
        let planned_output = match output {
            WorkflowOutputMode::Configured => PlatformWorkflowOutputMode::Configured,
            WorkflowOutputMode::Copy => PlatformWorkflowOutputMode::Copy,
            WorkflowOutputMode::ReturnFile => PlatformWorkflowOutputMode::ReturnFile,
            WorkflowOutputMode::ReturnResult => PlatformWorkflowOutputMode::ReturnResult,
        };
        // The synchronized definition is admitted against a fresh local
        // capability snapshot before any private copy or native action starts.
        let _authorized_plan = WorkflowExecutionPlan::new(workflow.clone(), planned_output)
            .map_err(|error| error.to_string())?
            .authorize(&output_port.capabilities())
            .map_err(|error| error.to_string())?;
        let reads_clipboard = matches!(input, WorkflowInput::Clipboard | WorkflowInput::History(_));
        let expected = if matches!(
            output,
            WorkflowOutputMode::ReturnFile | WorkflowOutputMode::ReturnResult
        ) {
            0
        } else {
            output_port
                .clipboard_revision()
                .map_err(|error| error.to_string())?
                .value()
        };
        let snapshot = match input {
            WorkflowInput::Clipboard => self.bridge.workflow_source(None, cancel.clone())?,
            WorkflowInput::History(id) => self.bridge.workflow_source(Some(id), cancel.clone())?,
            WorkflowInput::Snapshot(snapshot) => snapshot,
            WorkflowInput::File(path) => PasteboardSnapshot {
                file_urls: vec![path],
                ..Default::default()
            },
            WorkflowInput::Files(paths) => PasteboardSnapshot {
                file_urls: paths,
                ..Default::default()
            },
            WorkflowInput::Text(text) => PasteboardSnapshot {
                text: Some(text),
                ..Default::default()
            },
        };
        let expected = if reads_clipboard {
            snapshot.change_count
        } else {
            expected
        };
        // Pin inbound caches while the private input copy is being prepared.
        let _source_lease = SourceLease {
            bridge: self.bridge.clone(),
            id: self
                .bridge
                .retain_transfer_cache_paths(snapshot.file_urls.clone())?,
        };
        let sources = snapshot
            .file_urls
            .iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        let before = manifest(&sources, cancel)?;
        let image_bytes = if sources.is_empty() {
            if snapshot.items.len() > 1 {
                return Err("多个原生剪贴板项目请先作为文件共享，不能静默丢弃部分内容".into());
            }
            let has_image = snapshot.png.is_some()
                || snapshot
                    .items
                    .iter()
                    .flat_map(|item| &item.representations)
                    .any(|rep| {
                        [
                            "public.jpeg",
                            "org.webmproject.webp",
                            "public.heic",
                            "public.png",
                            "public.tiff",
                            "com.compuserve.gif",
                        ]
                        .contains(&rep.type_name.as_str())
                    });
            if has_image {
                Some(snapshot_image(&snapshot)?)
            } else {
                None
            }
        } else {
            None
        };
        let input_bytes = if !sources.is_empty() {
            total_bytes(&before)?
        } else if let Some(bytes) = &image_bytes {
            bytes.len() as u64
        } else if let Some(text) = &snapshot.text {
            text.len() as u64
        } else {
            return Err("没有可处理的文字、图片、文件或文件夹".into());
        };
        // Two generations may coexist briefly. Image decoding has its separate 128 MiB limit;
        // arbitrary files are constrained only by the existing cache capacity policy.
        let uses_images = image_bytes.is_some()
            || workflow.steps.iter().any(|s| {
                matches!(
                    s.kind,
                    WorkflowStepKind::Compress
                        | WorkflowStepKind::Resize
                        | WorkflowStepKind::Convert
                )
            });
        let extra = if uses_images {
            MAX_BYTES * 3
        } else {
            16 * 1024 * 1024
        };
        let budget = input_bytes
            .checked_mul(3)
            .and_then(|n| n.checked_add(extra))
            .ok_or("内容容量过大")?;
        let transfer = TransferId::new();
        let cache_root = dirs::cache_dir()
            .ok_or("无法确定工作流缓存目录")?
            .join("com.unidesk.app/clipboard")
            .join(transfer.to_string());
        let lease = self
            .bridge
            .retain_transfer_cache_paths(vec![cache_root.to_string_lossy().into_owned()])?;
        let destination = match self.bridge.workflow_cache_reserve(transfer, budget) {
            Ok(path) => path,
            Err(error) => {
                self.bridge.release_transfer_cache_paths(lease);
                return Err(error);
            }
        };
        let mut workspace = Workspace {
            bridge: self.bridge.clone(),
            transfer,
            lease: Some(lease),
            path: destination.clone(),
            keep: false,
        };
        let initial = destination.join("input");
        fs::create_dir_all(&initial).map_err(|e| e.to_string())?;
        let mut payload = if !sources.is_empty() {
            let mut paths = Vec::new();
            let mut copied = 0;
            for path in &sources {
                let target = unique_path(&initial, file_name(path)?, path.is_dir());
                copy_tree(path, &target, cancel, &mut copied, budget)?;
                paths.push(target);
            }
            Payload::Files {
                paths,
                image: false,
            }
        } else if let Some(bytes) = image_bytes {
            let info = image_processor::inspect_image(&bytes)?;
            let (bytes, format) = if let Some(format) = info.format {
                (bytes, format)
            } else {
                let normalized =
                    image_processor::encode_image(&bytes, ImageFormat::Png, 100, None)?;
                (normalized.data, ImageFormat::Png)
            };
            let path = initial.join(format!("uniDesk-image.{}", format.extension()));
            fs::write(&path, bytes).map_err(|e| e.to_string())?;
            Payload::Files {
                paths: vec![path],
                image: true,
            }
        } else {
            Payload::Text(snapshot.text.ok_or("没有文字内容")?)
        };
        ensure_unchanged(&sources, &before, cancel)?;
        validate_input(workflow.input_type, &payload)?;
        self.update(id, |run| run.input_bytes = input_bytes);
        let mut policy = ImagePolicy {
            quality: 85,
            ..Default::default()
        };
        for (index, step) in workflow.steps.iter().enumerate() {
            check_cancel(cancel)?;
            self.update(id, |run| {
                run.step_index = index as u32;
                run.message = match step.kind {
                    WorkflowStepKind::TextReplace => "替换文字",
                    WorkflowStepKind::TextTrim => "整理文字",
                    WorkflowStepKind::Rename => "批量重命名",
                    WorkflowStepKind::Zip => "打包 ZIP",
                    WorkflowStepKind::Compress => "压缩图片",
                    WorkflowStepKind::Resize => "调整尺寸",
                    WorkflowStepKind::Convert => "转换格式",
                    WorkflowStepKind::SystemShortcut => "等待系统快捷指令",
                    _ => "准备最终输出",
                }
                .into();
            });
            let stage = destination.join(format!("step-{index}"));
            match step.kind {
                WorkflowStepKind::TextReplace | WorkflowStepKind::TextTrim => {
                    let Payload::Text(text) = &mut payload else {
                        return Err("文字步骤仅接受纯文字输入".into());
                    };
                    *text = if step.kind == WorkflowStepKind::TextTrim {
                        text.trim().to_owned()
                    } else {
                        let count = text.matches(&step.find).count() as u64;
                        let projected = (text.len() as u64)
                            .saturating_sub(count.saturating_mul(step.find.len() as u64))
                            .saturating_add(count.saturating_mul(step.replacement.len() as u64));
                        if projected > budget {
                            return Err("文字替换结果超过工作流缓存预留容量".into());
                        }
                        text.replace(&step.find, &step.replacement)
                    };
                }
                WorkflowStepKind::Rename => {
                    let Payload::Files { paths, image } = &mut payload else {
                        return Err("重命名步骤需要文件或文件夹".into());
                    };
                    if *image {
                        return Err("重命名步骤需要文件输入，不接受剪贴板图片".into());
                    }
                    fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
                    let mut renamed = Vec::new();
                    for path in paths.iter() {
                        check_cancel(cancel)?;
                        let directory = path.is_dir();
                        let name = rename_name(path, step, directory)?;
                        let target = unique_path(&stage, &name, directory);
                        fs::rename(path, &target).map_err(|e| e.to_string())?;
                        renamed.push(target);
                    }
                    *paths = renamed;
                }
                WorkflowStepKind::Zip => {
                    let Payload::Files { paths, image } = &mut payload else {
                        return Err("ZIP 步骤需要文件或文件夹".into());
                    };
                    if *image {
                        return Err("ZIP 步骤需要文件输入，不接受剪贴板图片".into());
                    }
                    fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
                    let archive_source = stage.join("contents");
                    fs::create_dir(&archive_source).map_err(|e| e.to_string())?;
                    for path in paths.iter() {
                        let target = unique_path(&archive_source, file_name(path)?, path.is_dir());
                        fs::rename(path, target).map_err(|e| e.to_string())?;
                    }
                    let archive = stage.join("Archive.zip");
                    let mut command = Command::new("/usr/bin/ditto");
                    command
                        .args(["-c", "-k", "--sequesterRsrc"])
                        .arg(&archive_source)
                        .arg(&archive);
                    run_child(command, cancel, &destination, budget, "ZIP 打包")?;
                    fs::remove_dir_all(archive_source).map_err(|e| e.to_string())?;
                    *paths = vec![archive];
                    policy = ImagePolicy {
                        quality: 85,
                        ..Default::default()
                    };
                }
                WorkflowStepKind::Compress
                | WorkflowStepKind::Resize
                | WorkflowStepKind::Convert => {
                    image_step(&mut payload, step, &mut policy, &stage, cancel)?;
                }
                WorkflowStepKind::SystemShortcut => {
                    // A shortcut only sees private copies. Its declared output controls decoding.
                    fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
                    let inputs = match &payload {
                        Payload::Text(text) => {
                            let file = stage.join("input.txt");
                            fs::write(&file, text).map_err(|e| e.to_string())?;
                            vec![file]
                        }
                        Payload::Files { paths, .. } => paths.clone(),
                    };
                    let result_path = stage.join("result");
                    self.external(id, true);
                    let result = shortcut(
                        step,
                        &inputs,
                        &result_path,
                        cancel,
                        &destination,
                        budget,
                        payload.kind(),
                    );
                    self.external(id, false);
                    let next = result?;
                    // Inputs are private and no longer needed once output has been collected.
                    for path in payload.paths() {
                        if path.is_dir() {
                            fs::remove_dir_all(&path).map_err(|e| e.to_string())?;
                        } else if path.exists() {
                            fs::remove_file(&path).map_err(|e| e.to_string())?;
                        }
                    }
                    if matches!(
                        step.shortcut_output,
                        WorkflowContentType::Files | WorkflowContentType::Text
                    ) {
                        policy = ImagePolicy {
                            quality: 85,
                            ..Default::default()
                        };
                    }
                    payload = next;
                }
                WorkflowStepKind::Copy | WorkflowStepKind::Paste => {}
            }
            check_budget(&destination, budget, cancel)?;
        }
        if let Some(target) = policy.target {
            fit_payload_target(
                &mut payload,
                &policy,
                target,
                &destination.join("final-images"),
                cancel,
            )?;
        }
        ensure_unchanged(&sources, &before, cancel)?;
        check_budget(&destination, budget, cancel)?;
        check_cancel(cancel)?;
        let terminal = workflow.steps.last().ok_or("缺少输出步骤")?;
        let will_paste =
            output == WorkflowOutputMode::Configured && terminal.kind == WorkflowStepKind::Paste;
        let returning = matches!(
            output,
            WorkflowOutputMode::ReturnFile | WorkflowOutputMode::ReturnResult
        );
        if will_paste && target_pid <= 0 {
            return Err("没有可用的目标应用，请使用全局快捷键或选择复制输出".into());
        }
        let mut final_snapshot = payload_snapshot(&payload, terminal.output_shape)?;
        // Materialize text as well: deferred file sharing uses output_paths while native
        // clipboard sharing uses the typed snapshot. Never change the snapshot's native form.
        let paths = match &payload {
            Payload::Text(text) => {
                let occupied = total_bytes(&manifest(std::slice::from_ref(&destination), cancel)?)?;
                if occupied
                    .checked_add(text.len() as u64)
                    .is_none_or(|size| size > budget)
                {
                    return Err("最终文字文件超过工作流缓存预留容量".into());
                }
                let path = destination.join("result.txt");
                fs::write(&path, text).map_err(|e| e.to_string())?;
                check_budget(&destination, budget, cancel)?;
                vec![path]
            }
            _ => payload.paths(),
        };
        if output == WorkflowOutputMode::ReturnFile && (paths.len() != 1 || !paths[0].is_file()) {
            return Err("此入口只能返回一个文件；多个文件或文件夹请先添加 ZIP 步骤".into());
        }
        let output_text = final_snapshot.text.clone().unwrap_or_default();
        let output_kind = if final_snapshot.text.is_some() {
            WorkflowContentType::Text
        } else if !final_snapshot.file_urls.is_empty() {
            WorkflowContentType::Files
        } else {
            WorkflowContentType::Image
        };
        let output_bytes = if output_kind == WorkflowContentType::Text {
            output_text.len() as u64
        } else {
            total_bytes(&manifest(&paths, cancel)?)?
        };
        let image_info = if paths.len() == 1 && paths[0].is_file() {
            fs::metadata(&paths[0])
                .ok()
                .filter(|m| m.len() <= MAX_BYTES)
                .and_then(|_| read_image_file(&paths[0]).ok())
                .and_then(|bytes| image_processor::inspect_image(&bytes).ok())
        } else {
            None
        };
        if !returning {
            let count = self.bridge.workflow_commit(
                final_snapshot.clone(),
                expected,
                cancel.clone(),
                if will_paste { target_pid } else { 0 },
            )?;
            final_snapshot.change_count = count;
            workspace.keep = true;
            // Once published, an interrupted paste must not make its clipboard file disappear.
            self.hold_workspace(&mut workspace)?;
            if will_paste {
                let deadline = Instant::now() + Duration::from_secs(3);
                while !output_port.modifiers_released() {
                    check_cancel(cancel)?;
                    if Instant::now() > deadline {
                        return Err("快捷键仍未释放，结果已复制，请手动粘贴".into());
                    }
                    thread::sleep(Duration::from_millis(15));
                }
                check_cancel(cancel)?;
                output_port
                    .paste_if_current(target_pid, unidesk_platform::NativeRevision::new(count))
                    .map_err(|error| error.to_string())?;
            }
        }
        self.update(id, |run| {
            run.output_paths = paths
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            run.output_path = run.output_paths.first().cloned().unwrap_or_default();
            run.output_text = output_text;
            run.output_kind = output_kind;
            run.output_bytes = output_bytes;
            run.output_mime = if output_kind == WorkflowContentType::Text {
                "text/plain".into()
            } else if let Some(format) = image_info.as_ref().and_then(|info| info.format) {
                format.mime().into()
            } else if paths.len() == 1 && paths[0].extension().is_some_and(|e| e == "zip") {
                "application/zip".into()
            } else {
                "application/octet-stream".into()
            };
            if let Some(info) = &image_info {
                run.width = info.width;
                run.height = info.height;
            }
            run.message = if will_paste {
                "已发送粘贴；结果由目标应用接收"
            } else if returning {
                "处理完成"
            } else {
                "已复制处理结果"
            }
            .into();
        });
        if let Ok(mut jobs) = self.jobs.lock()
            && let Some(job) = jobs.get_mut(id)
        {
            job.result = Some(final_snapshot);
        }
        workspace.keep = true;
        self.hold_workspace(&mut workspace)?;
        Ok(())
    }
    fn hold_workspace(&self, workspace: &mut Workspace) -> Result<(), String> {
        if let Some(lease) = workspace.lease {
            self.leases
                .lock()
                .map_err(|_| "缓存租约不可用")?
                .push(Lease {
                    id: lease,
                    until: Instant::now() + Duration::from_secs(120),
                });
            workspace.lease = None;
        }
        Ok(())
    }
}

fn file_name(path: &Path) -> Result<&str, String> {
    path.file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty() && *n != "." && *n != "..")
        .ok_or_else(|| "文件名称无效".into())
}
fn unique_path(parent: &Path, name: &str, directory: bool) -> PathBuf {
    let path = Path::new(name);
    let (stem, extension) = if directory {
        (name, None)
    } else {
        (
            path.file_stem().and_then(|s| s.to_str()).unwrap_or(name),
            path.extension().and_then(|s| s.to_str()),
        )
    };
    let mut candidate = parent.join(name);
    let mut n = 2;
    while candidate.exists() {
        let numbered = format!("{stem} ({n})");
        candidate = parent
            .join(extension.map_or_else(|| numbered.clone(), |ext| format!("{numbered}.{ext}")));
        n += 1;
    }
    candidate
}
fn rename_name(path: &Path, step: &WorkflowStep, directory: bool) -> Result<String, String> {
    let original = file_name(path)?;
    let stem = if directory {
        original
    } else {
        path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(original)
    };
    let changed = if step.find.is_empty() {
        stem.to_owned()
    } else {
        stem.replace(&step.find, &step.replacement)
    };
    let mut name = format!("{}{}{}", step.prefix, changed, step.suffix);
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\0']) {
        return Err("重命名结果不是有效文件名称".into());
    }
    if !directory && let Some(extension) = path.extension().and_then(|e| e.to_str()) {
        name.push('.');
        name.push_str(extension);
    }
    if name.len() > 255 {
        return Err("重命名结果超过文件系统名称长度限制".into());
    }
    Ok(name)
}
fn manifest(paths: &[PathBuf], cancel: &AtomicBool) -> Result<Manifest, String> {
    let mut result = BTreeMap::new();
    let mut pending = paths.to_vec();
    while let Some(path) = pending.pop() {
        check_cancel(cancel)?;
        let meta = fs::symlink_metadata(&path).map_err(|_| "源文件不可用或已变化")?;
        if meta.file_type().is_symlink() || !(meta.is_dir() || meta.is_file()) {
            return Err("工作流不处理符号链接或特殊文件".into());
        }
        if meta.is_dir() {
            for entry in fs::read_dir(&path).map_err(|e| e.to_string())? {
                pending.push(entry.map_err(|e| e.to_string())?.path());
            }
        }
        result.insert(
            path,
            Stamp {
                bytes: if meta.is_file() { meta.len() } else { 0 },
                modified: meta.modified().map_err(|e| e.to_string())?,
                directory: meta.is_dir(),
            },
        );
    }
    Ok(result)
}
fn total_bytes(manifest: &Manifest) -> Result<u64, String> {
    manifest.values().try_fold(0u64, |total, value| {
        total
            .checked_add(value.bytes)
            .ok_or_else(|| "内容容量过大".into())
    })
}
fn ensure_unchanged(
    paths: &[PathBuf],
    before: &Manifest,
    cancel: &AtomicBool,
) -> Result<(), String> {
    if &manifest(paths, cancel)? != before {
        return Err("源文件已变化，请重新运行流程".into());
    }
    Ok(())
}
fn check_budget(root: &Path, budget: u64, cancel: &AtomicBool) -> Result<(), String> {
    if total_bytes(&manifest(&[root.to_owned()], cancel)?)? > budget {
        return Err("处理结果超过工作流缓存预留容量".into());
    }
    Ok(())
}
fn copy_tree(
    source: &Path,
    target: &Path,
    cancel: &AtomicBool,
    copied: &mut u64,
    budget: u64,
) -> Result<(), String> {
    let mut pending = vec![(source.to_owned(), target.to_owned())];
    while let Some((source, target)) = pending.pop() {
        check_cancel(cancel)?;
        let metadata = fs::symlink_metadata(&source).map_err(|e| e.to_string())?;
        if metadata.file_type().is_symlink() {
            return Err("工作流不处理符号链接".into());
        }
        if metadata.is_dir() {
            fs::create_dir(&target).map_err(|e| e.to_string())?;
            for entry in fs::read_dir(&source).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                pending.push((entry.path(), target.join(entry.file_name())));
            }
        } else if metadata.is_file() {
            let mut reader = fs::File::open(&source).map_err(|e| e.to_string())?;
            let mut writer = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
                .map_err(|e| e.to_string())?;
            let mut buf = vec![0u8; 1024 * 1024];
            loop {
                check_cancel(cancel)?;
                let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                *copied = copied.checked_add(n as u64).ok_or("内容容量过大")?;
                if *copied > budget {
                    return Err("文件副本超过缓存预留容量".into());
                }
                writer.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            }
        } else {
            return Err("工作流不处理特殊文件".into());
        }
    }
    Ok(())
}
fn validate_input(kind: WorkflowContentType, payload: &Payload) -> Result<(), String> {
    if kind == WorkflowContentType::Any || kind == payload.kind() {
        return Ok(());
    }
    if kind == WorkflowContentType::Image
        && let Payload::Files { paths, .. } = payload
    {
        for path in paths {
            image_processor::inspect_image(&read_image_file(path)?)?;
        }
        return Ok(());
    }
    Err("流程声明的输入类型与当前内容不兼容".into())
}
fn image_step(
    payload: &mut Payload,
    step: &WorkflowStep,
    policy: &mut ImagePolicy,
    stage: &Path,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let Payload::Files { paths, .. } = payload else {
        return Err("图片步骤需要图片输入".into());
    };
    fs::create_dir_all(stage).map_err(|e| e.to_string())?;
    if step.kind == WorkflowStepKind::Convert {
        policy.explicit_format = Some(step.format);
        policy.quality = step.quality;
    }
    if step.kind == WorkflowStepKind::Compress {
        if let Some(target) = step.target_bytes {
            policy.target = Some(policy.target.map_or(target, |old| old.min(target)));
        } else {
            policy.quality = step.quality;
        }
    }
    let mut next_paths = Vec::new();
    for path in paths.iter() {
        check_cancel(cancel)?;
        // Fails the complete batch for folders/non-images; no implicit recursive image selection.
        let bytes = read_image_file(path)?;
        let info = image_processor::inspect_image(&bytes)?;
        let original_format = info.format.unwrap_or(if info.has_alpha {
            ImageFormat::Png
        } else {
            ImageFormat::Jpeg
        });
        let format = policy
            .explicit_format
            .unwrap_or(if step.kind == WorkflowStepKind::Compress {
                if info.has_alpha {
                    ImageFormat::Png
                } else {
                    ImageFormat::Jpeg
                }
            } else {
                original_format
            });
        let edge = if step.kind == WorkflowStepKind::Resize {
            Some(step.max_edge.unwrap_or_else(|| {
                ((u64::from(info.width.max(info.height))
                    * u64::from(step.scale_percent.unwrap_or(100)))
                    / 100)
                    .max(1) as u32
            }))
        } else {
            None
        };
        let result = if step.kind == WorkflowStepKind::Compress && step.target_bytes.is_some() {
            None
        } else {
            Some(image_processor::encode_image(
                &bytes,
                format,
                policy.quality,
                edge,
            )?)
        };
        let (bytes, actual_format) = match result {
            Some(result)
                if step.kind != WorkflowStepKind::Compress || result.data.len() < bytes.len() =>
            {
                (result.data, format)
            }
            _ if info.format.is_none() => {
                let normalized =
                    image_processor::encode_image(&bytes, ImageFormat::Png, 100, None)?;
                (normalized.data, ImageFormat::Png)
            }
            _ => (bytes, original_format),
        };
        let name = format!("{}.{}", safe_stem(path), actual_format.extension());
        let target = unique_path(stage, &name, false);
        fs::write(&target, bytes).map_err(|e| e.to_string())?;
        next_paths.push(target);
    }
    for path in paths.iter() {
        fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    *paths = next_paths;
    Ok(())
}
fn fit_payload_target(
    payload: &mut Payload,
    policy: &ImagePolicy,
    target: u64,
    stage: &Path,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let Payload::Files { paths, .. } = payload else {
        return Ok(());
    };
    fs::create_dir_all(stage).map_err(|e| e.to_string())?;
    let mut next_paths = Vec::new();
    for path in paths.iter() {
        let bytes = read_image_file(path)?;
        let info = image_processor::inspect_image(&bytes)?;
        if bytes.len() as u64 <= target {
            next_paths.push(path.clone());
            continue;
        }
        let format = policy
            .explicit_format
            .or(info.format)
            .unwrap_or(if info.has_alpha {
                ImageFormat::Png
            } else {
                ImageFormat::Jpeg
            });
        let result = fit_target(&bytes, format, policy.quality, target, cancel)?;
        let output = unique_path(
            stage,
            &format!("{}.{}", safe_stem(path), result.format.extension()),
            false,
        );
        fs::write(&output, result.data).map_err(|e| e.to_string())?;
        fs::remove_file(path).map_err(|e| e.to_string())?;
        next_paths.push(output);
    }
    *paths = next_paths;
    Ok(())
}
fn payload_snapshot(
    payload: &Payload,
    shape: ImageOutputShape,
) -> Result<PasteboardSnapshot, String> {
    match payload {
        Payload::Text(text) => Ok(PasteboardSnapshot {
            text: Some(text.clone()),
            ..Default::default()
        }),
        Payload::Files { paths, image } => {
            if (*image && shape != ImageOutputShape::File) || shape == ImageOutputShape::Image {
                if paths.len() != 1 {
                    return Err("图片输出形式只能用于一张图片；批量结果请选择文件输出".into());
                }
                let bytes = read_image_file(&paths[0])?;
                let info = image_processor::inspect_image(&bytes)?;
                let result = if let Some(format) = info.format {
                    image_processor::processed_representation(bytes, format)?
                } else {
                    image_processor::encode_image(&bytes, ImageFormat::Png, 100, None)?
                };
                Ok(PasteboardSnapshot {
                    items: vec![PasteboardItem {
                        representations: vec![
                            PasteboardRepresentation {
                                type_name: result.format.uti().into(),
                                data: result.data,
                            },
                            PasteboardRepresentation {
                                type_name: "public.tiff".into(),
                                data: result.tiff,
                            },
                        ],
                    }],
                    ..Default::default()
                })
            } else {
                Ok(PasteboardSnapshot {
                    file_urls: paths
                        .iter()
                        .map(|p| p.to_string_lossy().into_owned())
                        .collect(),
                    ..Default::default()
                })
            }
        }
    }
}
fn run_child(
    mut command: Command,
    cancel: &AtomicBool,
    root: &Path,
    budget: u64,
    label: &str,
) -> Result<(), String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("{label}无法启动：{e}"))?;
    let started = Instant::now();
    let mut checked = Instant::now();
    loop {
        let invalid = if cancel.load(Ordering::Acquire) {
            Some("流程已取消".to_owned())
        } else if started.elapsed() > Duration::from_secs(600) {
            Some(format!("{label}等待超时"))
        } else if checked.elapsed() > Duration::from_secs(1) {
            checked = Instant::now();
            check_budget(root, budget, cancel).err()
        } else {
            None
        };
        if let Some(error) = invalid {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    Ok(())
                } else {
                    Err(format!("{label}执行失败或已取消"))
                };
            }
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.to_string());
            }
        }
    }
}
fn shortcut(
    step: &WorkflowStep,
    inputs: &[PathBuf],
    output: &Path,
    cancel: &AtomicBool,
    root: &Path,
    budget: u64,
    input_kind: WorkflowContentType,
) -> Result<Payload, String> {
    if !system_shortcuts()?.iter().any(|s| s.id == step.shortcut_id) {
        return Err("系统快捷指令已删除或本机未绑定，请重新选择".into());
    }
    let mut command = Command::new("/usr/bin/shortcuts");
    command
        .arg("run")
        .arg(&step.shortcut_id)
        .arg("--input-path")
        .args(inputs)
        .arg("--output-path")
        .arg(output);
    let kind = if step.shortcut_output == WorkflowContentType::Any {
        input_kind
    } else {
        step.shortcut_output
    };
    if kind == WorkflowContentType::Text {
        command.args(["--output-type", "public.utf8-plain-text"]);
    } else if kind == WorkflowContentType::Files {
        fs::create_dir(output).map_err(|e| e.to_string())?;
    }
    run_child(command, cancel, root, budget, "系统快捷指令")?;
    let metadata =
        fs::symlink_metadata(output).map_err(|_| "系统快捷指令没有返回声明类型的内容")?;
    if metadata.file_type().is_symlink() {
        return Err("系统快捷指令不能返回符号链接".into());
    }
    match kind {
        WorkflowContentType::Text => {
            if !metadata.is_file() || metadata.len() > budget {
                return Err("系统快捷指令必须返回有效文字文件".into());
            }
            let bytes = fs::read(output).map_err(|e| e.to_string())?;
            Ok(Payload::Text(
                String::from_utf8(bytes).map_err(|_| "快捷指令没有返回 UTF-8 文字")?,
            ))
        }
        WorkflowContentType::Image => {
            let bytes = read_image_file(output)?;
            let info = image_processor::inspect_image(&bytes)?;
            let (bytes, format) = if let Some(format) = info.format {
                (bytes, format)
            } else {
                (
                    image_processor::encode_image(&bytes, ImageFormat::Png, 100, None)?.data,
                    ImageFormat::Png,
                )
            };
            let named = output.with_extension(format.extension());
            fs::write(&named, bytes).map_err(|e| e.to_string())?;
            fs::remove_file(output).map_err(|e| e.to_string())?;
            Ok(Payload::Files {
                paths: vec![named],
                image: true,
            })
        }
        WorkflowContentType::Files | WorkflowContentType::Any => {
            manifest(&[output.to_owned()], cancel)?;
            let mut paths = if metadata.is_dir() {
                fs::read_dir(output)
                    .map_err(|e| e.to_string())?
                    .map(|entry| entry.map(|e| e.path()).map_err(|e| e.to_string()))
                    .collect::<Result<Vec<_>, String>>()?
            } else {
                vec![output.to_owned()]
            };
            paths.sort();
            if paths.is_empty() {
                return Err("系统快捷指令没有返回文件".into());
            }
            Ok(Payload::Files {
                paths,
                image: false,
            })
        }
    }
}
