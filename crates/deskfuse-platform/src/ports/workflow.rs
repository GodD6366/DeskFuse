//! Local workflow admission. Sync transports definitions/rules, never authorization.
use super::{
    ApplyResult, AuthorizedFile, CapturedContent, ContentData, ContentToApply, NativeRevision,
};
use crate::{BoundaryErrorCode, PlatformError, PlatformResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::PathBuf, sync::atomic::AtomicBool};
use deskfuse_core::workflow::{Workflow, WorkflowContentType, WorkflowStepKind};

/// Clipboard lifecycle owned by the common state machine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClipboardState {
    Idle,
    Monitoring,
    Captured { revision: NativeRevision },
    Applying { expected_revision: NativeRevision },
    Applied { revision: NativeRevision },
    Superseded { current_revision: NativeRevision },
    Failed { code: BoundaryErrorCode },
}
impl Default for ClipboardState {
    fn default() -> Self {
        Self::Idle
    }
}
impl ClipboardState {
    pub fn start_monitoring(&mut self) -> PlatformResult<()> {
        if matches!(
            self,
            Self::Idle | Self::Applied { .. } | Self::Superseded { .. }
        ) {
            *self = Self::Monitoring;
            Ok(())
        } else {
            Err(PlatformError::InvalidState {
                operation: "start clipboard monitoring",
            })
        }
    }
    pub fn captured(&mut self, revision: NativeRevision) -> PlatformResult<()> {
        if matches!(
            self,
            Self::Monitoring | Self::Applied { .. } | Self::Superseded { .. }
        ) {
            *self = Self::Captured { revision };
            Ok(())
        } else {
            Err(PlatformError::InvalidState {
                operation: "capture clipboard",
            })
        }
    }
    pub fn begin_apply(&mut self, expected_revision: NativeRevision) -> PlatformResult<()> {
        if matches!(
            self,
            Self::Captured { .. } | Self::Monitoring | Self::Applied { .. }
        ) {
            *self = Self::Applying { expected_revision };
            Ok(())
        } else {
            Err(PlatformError::InvalidState {
                operation: "apply clipboard",
            })
        }
    }
    pub fn applied(&mut self, result: ApplyResult) -> PlatformResult<()> {
        match result {
            ApplyResult::Applied { new_revision } => {
                *self = Self::Applied {
                    revision: new_revision,
                }
            }
            ApplyResult::Superseded { current_revision } => {
                *self = Self::Superseded { current_revision }
            }
            ApplyResult::Rejected { .. } => {
                *self = Self::Failed {
                    code: BoundaryErrorCode::Native,
                }
            }
        }
        Ok(())
    }
    pub fn fail(&mut self, code: BoundaryErrorCode) {
        *self = Self::Failed { code };
    }
}

/// File transfer lifecycle. Materialization and publication are separate so a
/// partially received file can never be exposed as a pasteable object.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FileTransferState {
    Idle,
    AwaitingAuthorization,
    Receiving { received_bytes: u64 },
    Materialized { paths: Vec<std::path::PathBuf> },
    Published,
    Failed { code: BoundaryErrorCode },
}
impl Default for FileTransferState {
    fn default() -> Self {
        Self::Idle
    }
}
impl FileTransferState {
    pub fn authorize(&mut self) -> PlatformResult<()> {
        if matches!(self, Self::Idle) {
            *self = Self::AwaitingAuthorization;
            Ok(())
        } else {
            Err(PlatformError::InvalidState {
                operation: "authorize file transfer",
            })
        }
    }
    pub fn receive(&mut self, received_bytes: u64) -> PlatformResult<()> {
        if matches!(self, Self::AwaitingAuthorization | Self::Receiving { .. }) {
            *self = Self::Receiving { received_bytes };
            Ok(())
        } else {
            Err(PlatformError::InvalidState {
                operation: "receive file transfer",
            })
        }
    }
    pub fn materialize(&mut self, paths: Vec<std::path::PathBuf>) -> PlatformResult<()> {
        if matches!(self, Self::Receiving { .. }) && !paths.is_empty() {
            *self = Self::Materialized { paths };
            Ok(())
        } else {
            Err(PlatformError::InvalidState {
                operation: "materialize file transfer",
            })
        }
    }
    pub fn publish(&mut self) -> PlatformResult<()> {
        if matches!(self, Self::Materialized { .. }) {
            *self = Self::Published;
            Ok(())
        } else {
            Err(PlatformError::InvalidState {
                operation: "publish file transfer",
            })
        }
    }
    pub fn fail(&mut self, code: BoundaryErrorCode) {
        *self = Self::Failed { code };
    }
}

/// Capability names shared by FFI and desktop IPC adapters.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub enum PlatformCapability {
    ClipboardRead,
    ClipboardWrite,
    FileDataObject,
    WorkflowText,
    WorkflowImage,
    WorkflowFiles,
    SystemShortcut,
    FinderPaste,
    GuardedPaste,
    OleDataObject,
}
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlatformCapabilities {
    enabled: BTreeSet<PlatformCapability>,
}
impl PlatformCapability {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ClipboardRead => "clipboard-read",
            Self::ClipboardWrite => "clipboard-write",
            Self::FileDataObject => "file-data-object",
            Self::WorkflowText => "workflow-text",
            Self::WorkflowImage => "workflow-image",
            Self::WorkflowFiles => "workflow-files",
            Self::SystemShortcut => "system-shortcut",
            Self::FinderPaste => "finder-paste",
            Self::GuardedPaste => "guarded-paste",
            Self::OleDataObject => "ole-data-object",
        }
    }
}

impl PlatformCapabilities {
    #[must_use]
    pub fn new(capabilities: impl IntoIterator<Item = PlatformCapability>) -> Self {
        Self {
            enabled: capabilities.into_iter().collect(),
        }
    }
    #[must_use]
    pub fn supports(&self, capability: PlatformCapability) -> bool {
        self.enabled.contains(&capability)
    }
    pub fn require(&self, capability: PlatformCapability) -> PlatformResult<()> {
        self.supports(capability)
            .then_some(())
            .ok_or(PlatformError::CapabilityUnavailable {
                capability: capability.as_str(),
            })
    }
}

/// Observed executor support, not user preferences or peer advertisements.
/// Empty/unknown snapshots deny every operation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowCapabilities {
    pub steps: Vec<WorkflowStepKind>,
    pub clipboard_write: bool,
    pub guarded_paste: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkflowOutputMode {
    Configured,
    Copy,
    ReturnFile,
    ReturnResult,
}

/// Validated immutable definition plus local output policy. Deliberately not
/// deserializable: a synchronized model must pass validation/admission locally.
#[derive(Clone, Debug)]
pub struct WorkflowExecutionPlan {
    workflow: Workflow,
    output: WorkflowOutputMode,
}

impl WorkflowExecutionPlan {
    pub fn new(workflow: Workflow, output: WorkflowOutputMode) -> PlatformResult<Self> {
        workflow
            .validate()
            .map_err(|message| PlatformError::InvalidRequest { message })?;
        if !workflow.enabled {
            return Err(PlatformError::InvalidRequest {
                message: "工作流已停用".into(),
            });
        }
        Ok(Self { workflow, output })
    }

    /// Called only on an authenticated local invocation or a locally selected
    /// sender-side rule, after resolving local shortcut bindings. This checks
    /// executor support; it does not grant peer trust or file access.
    pub fn authorize(
        self,
        capabilities: &WorkflowCapabilities,
    ) -> PlatformResult<AuthorizedWorkflowPlan> {
        for step in &self.workflow.steps {
            if matches!(step.kind, WorkflowStepKind::Copy | WorkflowStepKind::Paste) {
                continue; // Output override may return data without clipboard I/O.
            }
            if !capabilities.steps.contains(&step.kind) {
                return Err(PlatformError::CapabilityUnavailable {
                    capability: step_capability(step.kind),
                });
            }
        }
        let returning = matches!(
            self.output,
            WorkflowOutputMode::ReturnFile | WorkflowOutputMode::ReturnResult
        );
        if !returning && !capabilities.clipboard_write {
            return Err(PlatformError::CapabilityUnavailable {
                capability: "clipboard-write",
            });
        }
        if self.output == WorkflowOutputMode::Configured
            && self
                .workflow
                .steps
                .last()
                .is_some_and(|s| s.kind == WorkflowStepKind::Paste)
            && !capabilities.guarded_paste
        {
            return Err(PlatformError::CapabilityUnavailable {
                capability: "guarded-paste",
            });
        }
        Ok(AuthorizedWorkflowPlan(self))
    }
}

fn step_capability(kind: WorkflowStepKind) -> &'static str {
    match kind {
        WorkflowStepKind::TextReplace | WorkflowStepKind::TextTrim => "workflow-text",
        WorkflowStepKind::Compress | WorkflowStepKind::Resize | WorkflowStepKind::Convert => {
            "workflow-image"
        }
        WorkflowStepKind::Rename | WorkflowStepKind::Zip => "workflow-files",
        WorkflowStepKind::SystemShortcut => "system-shortcut",
        WorkflowStepKind::Copy => "clipboard-write",
        WorkflowStepKind::Paste => "guarded-paste",
    }
}

/// No public constructor or deserializer: native execution receives only a
/// validated, capability-admitted local plan, never a wire-provided token.
#[derive(Clone, Debug)]
pub struct AuthorizedWorkflowPlan(WorkflowExecutionPlan);
impl AuthorizedWorkflowPlan {
    pub fn workflow(&self) -> &Workflow {
        &self.0.workflow
    }
    pub fn output(&self) -> WorkflowOutputMode {
        self.0.output
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkflowPayload {
    Text(String),
    Image(Vec<u8>),
    Files(Vec<AuthorizedFile>),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkflowExecutionResult {
    pub payload: WorkflowPayload,
    pub message: String,
}
/// Typed requests for host-owned workflow file and process operations. The
/// engine owns workflow policy and image calculation; the host owns all OS
/// filesystem/process effects.
#[derive(Clone, Debug)]
pub struct WorkflowWorkspaceRequest {
    pub source: PathBuf,
    pub target: PathBuf,
    pub budget: u64,
}
#[derive(Clone, Debug)]
pub struct WorkflowArchiveRequest {
    pub source: PathBuf,
    pub archive: PathBuf,
    pub root: PathBuf,
    pub budget: u64,
}
#[derive(Clone, Debug)]
pub struct WorkflowShortcutRequest {
    pub id: String,
    pub inputs: Vec<PathBuf>,
    pub output: PathBuf,
    pub output_type: WorkflowContentType,
    pub input_type: WorkflowContentType,
    pub root: PathBuf,
    pub budget: u64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkflowShortcut {
    pub id: String,
    pub name: String,
}
#[async_trait]
pub trait WorkflowExecutionPort: Send + Sync {
    async fn execute(
        &self,
        plan: &AuthorizedWorkflowPlan,
        payload: WorkflowPayload,
    ) -> PlatformResult<WorkflowExecutionResult>;
    async fn capabilities(&self) -> PlatformResult<PlatformCapabilities>;

    fn workflow_capabilities(&self) -> WorkflowCapabilities;
    fn clipboard_revision(&self) -> PlatformResult<super::NativeRevision>;
    fn modifiers_released(&self) -> bool;
    fn paste_if_current(
        &self,
        target_pid: i32,
        revision: super::NativeRevision,
    ) -> PlatformResult<()>;
    fn create_workspace(&self, path: &std::path::Path) -> PlatformResult<()>;
    fn copy_workspace(
        &self,
        request: &WorkflowWorkspaceRequest,
        cancel: &AtomicBool,
    ) -> PlatformResult<u64>;
    fn rename_path(&self, source: &std::path::Path, target: &std::path::Path)
    -> PlatformResult<()>;
    fn move_path(&self, source: &std::path::Path, target: &std::path::Path) -> PlatformResult<()>;
    fn remove_path(&self, path: &std::path::Path) -> PlatformResult<()>;
    fn archive_zip(
        &self,
        request: &WorkflowArchiveRequest,
        cancel: &AtomicBool,
    ) -> PlatformResult<()>;
    fn list_shortcuts(&self) -> PlatformResult<Vec<WorkflowShortcut>>;
    fn run_shortcut(
        &self,
        request: &WorkflowShortcutRequest,
        cancel: &AtomicBool,
    ) -> PlatformResult<()>;
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileDataObjectHandle {
    pub id: String,
    pub files: Vec<AuthorizedFile>,
}
#[async_trait]
pub trait FileDataObjectPort: Send + Sync {
    async fn create_file_data_object(
        &self,
        files: Vec<AuthorizedFile>,
    ) -> PlatformResult<FileDataObjectHandle>;
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkflowInput {
    pub mime: String,
    pub bytes: Vec<u8>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum PlatformCommand {
    /// Legacy request retained for older clients. Hosts must use the host
    /// request variants below when native clipboard data is required.
    ReadClipboard,
    WriteClipboard {
        content: ContentToApply,
        expected_revision: NativeRevision,
    },
    CreateFileDataObject {
        paths: Vec<String>,
    },
    ExecuteWorkflow {
        workflow_id: String,
    },
    /// Authenticated WPF host report containing the bounded native capture.
    ReadClipboardHost {
        content: CapturedContent,
    },
    /// Authenticated WPF host receipt after the native write has completed.
    WriteClipboardHost {
        content: ContentToApply,
        expected_revision: NativeRevision,
    },
    /// Workflow input is supplied by the interactive host; the service owns
    /// model validation and execution but never touches the user clipboard.
    ExecuteWorkflowHost {
        workflow_id: String,
        input: Vec<WorkflowInput>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum PlatformEvent {
    ClipboardCaptured(CapturedContent),
    ClipboardApplied(ApplyResult),
    FileDataObjectCreated {
        id: String,
    },
    Failed {
        code: BoundaryErrorCode,
        message: String,
    },
    WorkflowCompleted {
        workflow_id: String,
        message: String,
        content: Option<ContentData>,
    },
    WorkflowFailed {
        workflow_id: String,
        code: BoundaryErrorCode,
    },
    /// Typed completion for command bridges. Legacy string responses may
    /// remain at an outer IPC adapter, but new consumers use this code.
    CommandFailed {
        request_id: u64,
        code: BoundaryErrorCode,
        message: String,
    },
}

/// OS-specific workflow output actions. Clipboard writes remain serialized by
/// the engine's existing ClipboardApplyPort; this port only observes revision
/// and posts a guarded paste to the original foreground target.
pub trait WorkflowOutputPort: Send + Sync {
    fn capabilities(&self) -> WorkflowCapabilities;
    fn clipboard_revision(&self) -> PlatformResult<super::NativeRevision>;
    fn modifiers_released(&self) -> bool;
    fn paste_if_current(
        &self,
        target_pid: i32,
        revision: super::NativeRevision,
    ) -> PlatformResult<()>;
}

/// Safe default for hosts that do not expose a native workflow output.
#[derive(Debug, Default)]
pub struct UnavailableWorkflowOutputPort;

impl WorkflowOutputPort for UnavailableWorkflowOutputPort {
    fn capabilities(&self) -> WorkflowCapabilities {
        WorkflowCapabilities::default()
    }

    fn clipboard_revision(&self) -> PlatformResult<super::NativeRevision> {
        Err(PlatformError::Unsupported {
            operation: "workflow clipboard revision",
        })
    }

    fn modifiers_released(&self) -> bool {
        false
    }

    fn paste_if_current(
        &self,
        _target_pid: i32,
        _revision: super::NativeRevision,
    ) -> PlatformResult<()> {
        Err(PlatformError::Unsupported {
            operation: "workflow guarded paste",
        })
    }
}
