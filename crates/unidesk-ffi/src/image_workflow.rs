//! Typed workflow API; definitions and all execution remain owned by Rust.
use super::*;
use unidesk_core::image_workflow as core_workflow;
use unidesk_engine::image_workflow as worker;
#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Webp,
    Heic,
}
impl From<ImageFormat> for core_workflow::ImageFormat {
    fn from(v: ImageFormat) -> Self {
        match v {
            ImageFormat::Png => Self::Png,
            ImageFormat::Jpeg => Self::Jpeg,
            ImageFormat::Webp => Self::Webp,
            ImageFormat::Heic => Self::Heic,
        }
    }
}
impl From<core_workflow::ImageFormat> for ImageFormat {
    fn from(v: core_workflow::ImageFormat) -> Self {
        match v {
            core_workflow::ImageFormat::Png => Self::Png,
            core_workflow::ImageFormat::Jpeg => Self::Jpeg,
            core_workflow::ImageFormat::Webp => Self::Webp,
            core_workflow::ImageFormat::Heic => Self::Heic,
        }
    }
}
#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum WorkflowStepKind {
    Compress,
    Resize,
    Convert,
    SystemShortcut,
    Copy,
    Paste,
}
impl From<WorkflowStepKind> for core_workflow::WorkflowStepKind {
    fn from(v: WorkflowStepKind) -> Self {
        match v {
            WorkflowStepKind::Compress => Self::Compress,
            WorkflowStepKind::Resize => Self::Resize,
            WorkflowStepKind::Convert => Self::Convert,
            WorkflowStepKind::SystemShortcut => Self::SystemShortcut,
            WorkflowStepKind::Copy => Self::Copy,
            WorkflowStepKind::Paste => Self::Paste,
        }
    }
}
impl From<core_workflow::WorkflowStepKind> for WorkflowStepKind {
    fn from(v: core_workflow::WorkflowStepKind) -> Self {
        match v {
            core_workflow::WorkflowStepKind::Compress => Self::Compress,
            core_workflow::WorkflowStepKind::Resize => Self::Resize,
            core_workflow::WorkflowStepKind::Convert => Self::Convert,
            core_workflow::WorkflowStepKind::SystemShortcut => Self::SystemShortcut,
            core_workflow::WorkflowStepKind::Copy => Self::Copy,
            core_workflow::WorkflowStepKind::Paste => Self::Paste,
        }
    }
}
#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum ImageOutputShape {
    FollowSource,
    Image,
    File,
}
impl From<ImageOutputShape> for core_workflow::ImageOutputShape {
    fn from(v: ImageOutputShape) -> Self {
        match v {
            ImageOutputShape::FollowSource => Self::FollowSource,
            ImageOutputShape::Image => Self::Image,
            ImageOutputShape::File => Self::File,
        }
    }
}
impl From<core_workflow::ImageOutputShape> for ImageOutputShape {
    fn from(v: core_workflow::ImageOutputShape) -> Self {
        match v {
            core_workflow::ImageOutputShape::FollowSource => Self::FollowSource,
            core_workflow::ImageOutputShape::Image => Self::Image,
            core_workflow::ImageOutputShape::File => Self::File,
        }
    }
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct ImageWorkflowStep {
    pub kind: WorkflowStepKind,
    pub quality: u8,
    pub target_bytes: Option<u64>,
    pub max_edge: Option<u32>,
    pub scale_percent: Option<u32>,
    pub format: ImageFormat,
    pub shortcut_id: String,
    pub shortcut_name: String,
    pub output_shape: ImageOutputShape,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ImageWorkflow {
    pub version: u32,
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub shortcut: Option<Shortcut>,
    pub steps: Vec<ImageWorkflowStep>,
}
impl From<ImageWorkflowStep> for core_workflow::ImageWorkflowStep {
    fn from(v: ImageWorkflowStep) -> Self {
        Self {
            kind: v.kind.into(),
            quality: v.quality,
            target_bytes: v.target_bytes,
            max_edge: v.max_edge,
            scale_percent: v.scale_percent,
            format: v.format.into(),
            shortcut_id: v.shortcut_id,
            shortcut_name: v.shortcut_name,
            output_shape: v.output_shape.into(),
        }
    }
}
impl From<core_workflow::ImageWorkflowStep> for ImageWorkflowStep {
    fn from(v: core_workflow::ImageWorkflowStep) -> Self {
        Self {
            kind: v.kind.into(),
            quality: v.quality,
            target_bytes: v.target_bytes,
            max_edge: v.max_edge,
            scale_percent: v.scale_percent,
            format: v.format.into(),
            shortcut_id: v.shortcut_id,
            shortcut_name: v.shortcut_name,
            output_shape: v.output_shape.into(),
        }
    }
}
impl From<ImageWorkflow> for core_workflow::ImageWorkflow {
    fn from(v: ImageWorkflow) -> Self {
        Self {
            version: v.version,
            id: v.id,
            name: v.name,
            enabled: v.enabled,
            shortcut: v.shortcut.map(|s| unidesk_core::InputShortcut {
                key_code: s.key_code,
                modifiers: s.modifiers,
            }),
            steps: v.steps.into_iter().map(Into::into).collect(),
        }
    }
}
impl From<core_workflow::ImageWorkflow> for ImageWorkflow {
    fn from(v: core_workflow::ImageWorkflow) -> Self {
        Self {
            version: v.version,
            id: v.id,
            name: v.name,
            enabled: v.enabled,
            shortcut: v.shortcut.map(|s| Shortcut {
                key_code: s.key_code,
                modifiers: s.modifiers,
            }),
            steps: v.steps.into_iter().map(Into::into).collect(),
        }
    }
}
#[derive(Clone, Debug, uniffi::Enum)]
pub enum WorkflowInput {
    Clipboard,
    History { id: u64 },
    File { path: String },
}
#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum WorkflowOutputMode {
    Configured,
    Copy,
    ReturnFile,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct SystemShortcut {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Debug, uniffi::Record)]
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
impl From<worker::ImageWorkflowRun> for ImageWorkflowRun {
    fn from(r: worker::ImageWorkflowRun) -> Self {
        Self {
            id: r.id,
            workflow_id: r.workflow_id,
            step_index: r.step_index,
            phase: r.phase,
            message: r.message,
            output_path: r.output_path,
            output_mime: r.output_mime,
            input_bytes: r.input_bytes,
            output_bytes: r.output_bytes,
            width: r.width,
            height: r.height,
        }
    }
}
#[uniffi::export]
impl Engine {
    pub fn image_workflows(&self) -> Result<Vec<ImageWorkflow>, EngineError> {
        self.active()?;
        Ok(self
            .workflows
            .list()
            .map_err(failure)?
            .into_iter()
            .map(Into::into)
            .collect())
    }
    pub fn save_image_workflow(&self, workflow: ImageWorkflow) -> Result<(), EngineError> {
        self.active()?;
        if workflow.enabled {
            if let Some(shortcut) = &workflow.shortcut {
                let state = self.state.lock().map_err(failure)?;
                let settings = &state.model.input_sharing.settings.shortcuts;
                if [
                    &settings.left,
                    &settings.right,
                    &settings.up,
                    &settings.down,
                    &settings.lock,
                ]
                .iter()
                .any(|s| s.key_code == shortcut.key_code && s.modifiers == shortcut.modifiers)
                {
                    return Err(failure("该快捷键已被键鼠共享占用"));
                }
                if shortcut.key_code == 2 && shortcut.modifiers == 0x120000 {
                    return Err(failure("该快捷键用于打开文件暂存区"));
                }
            }
        }
        self.workflows.save(workflow.into()).map_err(failure)?;
        self.refresh_workflow_hotkeys();
        Ok(())
    }
    pub fn delete_image_workflow(&self, id: String) -> Result<(), EngineError> {
        self.active()?;
        self.workflows.delete(&id).map_err(failure)?;
        self.refresh_workflow_hotkeys();
        Ok(())
    }
    pub fn run_image_workflow(
        &self,
        workflow_id: String,
        input: WorkflowInput,
        output: WorkflowOutputMode,
        target_pid: i32,
    ) -> Result<String, EngineError> {
        self.active()?;
        let input = match input {
            WorkflowInput::Clipboard => worker::WorkflowInput::Clipboard,
            WorkflowInput::History { id } => worker::WorkflowInput::History(id),
            WorkflowInput::File { path } => worker::WorkflowInput::File(path),
        };
        let output = match output {
            WorkflowOutputMode::Configured => worker::WorkflowOutputMode::Configured,
            WorkflowOutputMode::Copy => worker::WorkflowOutputMode::Copy,
            WorkflowOutputMode::ReturnFile => worker::WorkflowOutputMode::ReturnFile,
        };
        self.workflows
            .start(workflow_id, input, output, target_pid)
            .map_err(failure)
    }
    pub fn image_workflow_run(&self, run_id: String) -> Result<ImageWorkflowRun, EngineError> {
        self.active()?;
        self.workflows
            .status(&run_id)
            .map(Into::into)
            .map_err(failure)
    }
    pub fn cancel_image_workflow(&self, run_id: String) -> Result<(), EngineError> {
        self.active()?;
        self.workflows.cancel(&run_id).map_err(failure)
    }
    pub fn system_shortcuts(&self) -> Result<Vec<SystemShortcut>, EngineError> {
        self.active()?;
        Ok(worker::system_shortcuts()
            .map_err(failure)?
            .into_iter()
            .map(|s| SystemShortcut {
                id: s.id,
                name: s.name,
            })
            .collect())
    }
}
impl Engine {
    fn refresh_workflow_hotkeys(&self) {
        if let (Ok(workflows), Ok(mut hotkeys)) =
            (self.workflows.list(), self.workflow_hotkeys.lock())
        {
            hotkeys.bindings = workflows
                .into_iter()
                .filter(|w| w.enabled)
                .filter_map(|w| {
                    w.shortcut.map(|s| unidesk_platform_macos::WorkflowHotkey {
                        id: w.id,
                        key_code: s.key_code,
                        modifiers: s.modifiers,
                    })
                })
                .collect();
        }
    }
}
