//! macOS host implementation for workflow filesystem and process actions.
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use deskfuse_core::workflow::{WorkflowContentType, WorkflowStepKind};
use deskfuse_platform::{
    AuthorizedWorkflowPlan, NativeRevision, PlatformCapabilities, PlatformCapability,
    PlatformError, PlatformResult, WorkflowArchiveRequest, WorkflowCapabilities,
    WorkflowExecutionPort, WorkflowExecutionResult, WorkflowPayload, WorkflowShortcut,
    WorkflowShortcutRequest, WorkflowWorkspaceRequest,
};

/// Host-owned implementation of workflow OS actions. Image decoding and
/// transformation remain in the common engine; this type only touches files,
/// processes, and macOS paste/input APIs.
pub struct MacOSWorkflowExecutionPort;

fn failed() -> PlatformError {
    PlatformError::FfiFailed {
        operation: "macos workflow host action",
    }
}
fn check_budget(root: &Path, budget: u64) -> PlatformResult<()> {
    let mut pending = vec![root.to_owned()];
    let mut total = 0u64;
    while let Some(path) = pending.pop() {
        let metadata = fs::symlink_metadata(&path).map_err(|_| failed())?;
        if metadata.file_type().is_symlink() {
            return Err(failed());
        }
        if metadata.is_dir() {
            for entry in fs::read_dir(&path).map_err(|_| failed())? {
                pending.push(entry.map_err(|_| failed())?.path());
            }
        } else if metadata.is_file() {
            total = total.checked_add(metadata.len()).ok_or_else(failed)?;
            if total > budget {
                return Err(failed());
            }
        } else {
            return Err(failed());
        }
    }
    Ok(())
}
fn copy_tree(
    source: &Path,
    target: &Path,
    cancel: &AtomicBool,
    budget: u64,
) -> PlatformResult<u64> {
    let mut pending = vec![(source.to_owned(), target.to_owned())];
    let mut copied = 0u64;
    while let Some((source, target)) = pending.pop() {
        if cancel.load(Ordering::Acquire) {
            return Err(failed());
        }
        let metadata = fs::symlink_metadata(&source).map_err(|_| failed())?;
        if metadata.file_type().is_symlink() {
            return Err(failed());
        }
        if metadata.is_dir() {
            fs::create_dir(&target).map_err(|_| failed())?;
            for entry in fs::read_dir(&source).map_err(|_| failed())? {
                let entry = entry.map_err(|_| failed())?;
                pending.push((entry.path(), target.join(entry.file_name())));
            }
        } else if metadata.is_file() {
            let mut reader = fs::File::open(&source).map_err(|_| failed())?;
            let mut writer = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
                .map_err(|_| failed())?;
            let mut buffer = vec![0u8; 1024 * 1024];
            loop {
                if cancel.load(Ordering::Acquire) {
                    return Err(failed());
                }
                let count = reader.read(&mut buffer).map_err(|_| failed())?;
                if count == 0 {
                    break;
                }
                copied = copied.checked_add(count as u64).ok_or_else(failed)?;
                if copied > budget {
                    return Err(failed());
                }
                writer.write_all(&buffer[..count]).map_err(|_| failed())?;
            }
        } else {
            return Err(failed());
        }
    }
    Ok(copied)
}
fn run_child(
    mut command: Command,
    cancel: &AtomicBool,
    root: &Path,
    budget: u64,
    operation: &'static str,
) -> PlatformResult<()> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| failed())?;
    let started = Instant::now();
    let mut checked = Instant::now();
    loop {
        if cancel.load(Ordering::Acquire) || started.elapsed() > Duration::from_secs(600) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(PlatformError::FfiFailed { operation });
        }
        if checked.elapsed() > Duration::from_secs(1) {
            checked = Instant::now();
            if check_budget(root, budget).is_err() {
                let _ = child.kill();
                let _ = child.wait();
                return Err(PlatformError::FfiFailed { operation });
            }
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                return status
                    .success()
                    .then_some(())
                    .ok_or(PlatformError::FfiFailed { operation });
            }
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(PlatformError::FfiFailed { operation });
            }
        }
    }
}

impl MacOSWorkflowExecutionPort {
    fn capabilities_snapshot() -> WorkflowCapabilities {
        WorkflowCapabilities {
            steps: vec![
                WorkflowStepKind::Compress,
                WorkflowStepKind::Resize,
                WorkflowStepKind::Convert,
                WorkflowStepKind::SystemShortcut,
                WorkflowStepKind::TextReplace,
                WorkflowStepKind::TextTrim,
                WorkflowStepKind::Rename,
                WorkflowStepKind::Zip,
            ],
            clipboard_write: true,
            guarded_paste: true,
        }
    }
}

#[async_trait]
impl WorkflowExecutionPort for MacOSWorkflowExecutionPort {
    async fn execute(
        &self,
        _plan: &AuthorizedWorkflowPlan,
        _payload: WorkflowPayload,
    ) -> PlatformResult<WorkflowExecutionResult> {
        Err(PlatformError::Unsupported {
            operation: "macos workflow execution requires engine orchestration",
        })
    }

    async fn capabilities(&self) -> PlatformResult<PlatformCapabilities> {
        Ok(PlatformCapabilities::new([
            PlatformCapability::ClipboardRead,
            PlatformCapability::ClipboardWrite,
            PlatformCapability::WorkflowText,
            PlatformCapability::WorkflowImage,
            PlatformCapability::WorkflowFiles,
            PlatformCapability::SystemShortcut,
            PlatformCapability::FinderPaste,
            PlatformCapability::GuardedPaste,
        ]))
    }

    fn workflow_capabilities(&self) -> WorkflowCapabilities {
        Self::capabilities_snapshot()
    }

    fn clipboard_revision(&self) -> PlatformResult<NativeRevision> {
        use deskfuse_platform::Pasteboard;
        crate::MacOSPasteboard::new()
            .change_count()
            .map(NativeRevision::new)
    }

    fn modifiers_released(&self) -> bool {
        crate::workflow_modifiers_released()
    }

    fn paste_if_current(&self, target_pid: i32, revision: NativeRevision) -> PlatformResult<()> {
        crate::workflow_paste_if_current(target_pid, revision.value()).map_err(|_| failed())
    }

    fn create_workspace(&self, path: &Path) -> PlatformResult<()> {
        fs::create_dir_all(path).map_err(|_| failed())
    }

    fn copy_workspace(
        &self,
        request: &WorkflowWorkspaceRequest,
        cancel: &AtomicBool,
    ) -> PlatformResult<u64> {
        copy_tree(&request.source, &request.target, cancel, request.budget)
    }

    fn rename_path(&self, source: &Path, target: &Path) -> PlatformResult<()> {
        fs::rename(source, target).map_err(|_| failed())
    }

    fn move_path(&self, source: &Path, target: &Path) -> PlatformResult<()> {
        fs::rename(source, target).map_err(|_| failed())
    }

    fn remove_path(&self, path: &Path) -> PlatformResult<()> {
        let metadata = fs::symlink_metadata(path).map_err(|_| failed())?;
        if metadata.file_type().is_symlink() {
            return Err(failed());
        }
        if metadata.is_dir() {
            fs::remove_dir_all(path).map_err(|_| failed())
        } else {
            fs::remove_file(path).map_err(|_| failed())
        }
    }

    fn archive_zip(
        &self,
        request: &WorkflowArchiveRequest,
        cancel: &AtomicBool,
    ) -> PlatformResult<()> {
        let mut command = Command::new("/usr/bin/ditto");
        command
            .args(["-c", "-k", "--sequesterRsrc"])
            .arg(&request.source)
            .arg(&request.archive);
        run_child(
            command,
            cancel,
            &request.root,
            request.budget,
            "macos workflow zip",
        )
    }

    fn list_shortcuts(&self) -> PlatformResult<Vec<WorkflowShortcut>> {
        let output = Command::new("/usr/bin/shortcuts")
            .args(["list", "--show-identifiers"])
            .output()
            .map_err(|_| failed())?;
        if !output.status.success() {
            return Err(failed());
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| {
                let (name, id) = line.rsplit_once(" (")?;
                Some(WorkflowShortcut {
                    name: name.to_owned(),
                    id: id.strip_suffix(')')?.to_owned(),
                })
            })
            .collect())
    }

    fn run_shortcut(
        &self,
        request: &WorkflowShortcutRequest,
        cancel: &AtomicBool,
    ) -> PlatformResult<()> {
        if !self
            .list_shortcuts()?
            .iter()
            .any(|shortcut| shortcut.id == request.id)
        {
            return Err(failed());
        }
        let kind = if request.output_type == WorkflowContentType::Any {
            request.input_type
        } else {
            request.output_type
        };
        let mut command = Command::new("/usr/bin/shortcuts");
        command
            .args(["run", &request.id, "--input-path"])
            .args(&request.inputs)
            .args(["--output-path"])
            .arg(&request.output);
        if kind == WorkflowContentType::Text {
            command.args(["--output-type", "public.utf8-plain-text"]);
        } else if kind == WorkflowContentType::Files {
            fs::create_dir(&request.output).map_err(|_| failed())?;
        }
        run_child(
            command,
            cancel,
            &request.root,
            request.budget,
            "macos workflow shortcut",
        )
    }
}
