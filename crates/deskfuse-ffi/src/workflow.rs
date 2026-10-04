//! Typed workflow API; definitions and all execution remain owned by Rust.
// UniFFI exports require owned arguments across the native language boundary.
#![allow(clippy::needless_pass_by_value)]
use super::{Engine, EngineError, Shortcut, failure};
use deskfuse_core::workflow as core_workflow;
use deskfuse_engine::workflow as worker;
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
    TextReplace,
    TextTrim,
    Rename,
    Zip,
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
            WorkflowStepKind::TextReplace => Self::TextReplace,
            WorkflowStepKind::TextTrim => Self::TextTrim,
            WorkflowStepKind::Rename => Self::Rename,
            WorkflowStepKind::Zip => Self::Zip,

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
            core_workflow::WorkflowStepKind::TextReplace => Self::TextReplace,
            core_workflow::WorkflowStepKind::TextTrim => Self::TextTrim,
            core_workflow::WorkflowStepKind::Rename => Self::Rename,
            core_workflow::WorkflowStepKind::Zip => Self::Zip,

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

#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum WorkflowContentType {
    Any,
    Text,
    Image,
    Files,
}
impl From<WorkflowContentType> for core_workflow::WorkflowContentType {
    fn from(v: WorkflowContentType) -> Self {
        match v {
            WorkflowContentType::Any => Self::Any,
            WorkflowContentType::Text => Self::Text,
            WorkflowContentType::Image => Self::Image,
            WorkflowContentType::Files => Self::Files,
        }
    }
}
impl From<core_workflow::WorkflowContentType> for WorkflowContentType {
    fn from(v: core_workflow::WorkflowContentType) -> Self {
        match v {
            core_workflow::WorkflowContentType::Any => Self::Any,
            core_workflow::WorkflowContentType::Text => Self::Text,
            core_workflow::WorkflowContentType::Image => Self::Image,
            core_workflow::WorkflowContentType::Files => Self::Files,
        }
    }
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct WorkflowStep {
    pub kind: WorkflowStepKind,
    pub quality: u8,
    pub target_bytes: Option<u64>,
    pub max_edge: Option<u32>,
    pub scale_percent: Option<u32>,
    pub format: ImageFormat,
    pub shortcut_id: String,
    pub shortcut_name: String,
    pub output_shape: ImageOutputShape,
    pub find: String,
    pub replacement: String,
    pub prefix: String,
    pub suffix: String,
    pub shortcut_output: WorkflowContentType,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct Workflow {
    pub version: u32,
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub shortcut: Option<Shortcut>,
    pub steps: Vec<WorkflowStep>,
    pub input_type: WorkflowContentType,
}
impl From<WorkflowStep> for core_workflow::WorkflowStep {
    fn from(v: WorkflowStep) -> Self {
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
            find: v.find,
            replacement: v.replacement,
            prefix: v.prefix,
            suffix: v.suffix,
            shortcut_output: v.shortcut_output.into(),
        }
    }
}
impl From<core_workflow::WorkflowStep> for WorkflowStep {
    fn from(v: core_workflow::WorkflowStep) -> Self {
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
            find: v.find,
            replacement: v.replacement,
            prefix: v.prefix,
            suffix: v.suffix,
            shortcut_output: v.shortcut_output.into(),
        }
    }
}
impl From<Workflow> for core_workflow::Workflow {
    fn from(v: Workflow) -> Self {
        Self {
            version: v.version,
            id: v.id,
            name: v.name,
            enabled: v.enabled,
            shortcut: v.shortcut.map(|s| deskfuse_core::InputShortcut {
                key_code: s.key_code,
                modifiers: s.modifiers,
            }),
            steps: v.steps.into_iter().map(Into::into).collect(),
            input_type: v.input_type.into(),
        }
    }
}
impl From<core_workflow::Workflow> for Workflow {
    fn from(v: core_workflow::Workflow) -> Self {
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
            input_type: v.input_type.into(),
        }
    }
}
#[derive(Clone, Debug, uniffi::Enum)]
pub enum WorkflowInput {
    Clipboard,
    History { id: u64 },
    File { path: String },
    Files { paths: Vec<String> },
    Text { value: String },
}
#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum WorkflowOutputMode {
    Configured,
    Copy,
    ReturnFile,
    ReturnResult,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct SystemShortcut {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct WorkflowRun {
    pub id: String,
    pub workflow_id: String,
    pub step_index: u32,
    pub phase: String,
    pub message: String,
    pub output_path: String,
    pub output_mime: String,
    pub output_paths: Vec<String>,
    pub output_text: String,
    pub output_kind: WorkflowContentType,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub width: u32,
    pub height: u32,
}
impl From<worker::WorkflowRun> for WorkflowRun {
    fn from(r: worker::WorkflowRun) -> Self {
        Self {
            id: r.id,
            workflow_id: r.workflow_id,
            step_index: r.step_index,
            phase: r.phase,
            message: r.message,
            output_path: r.output_path,
            output_mime: r.output_mime,
            output_paths: r.output_paths,
            output_text: r.output_text,
            output_kind: r.output_kind.into(),
            input_bytes: r.input_bytes,
            output_bytes: r.output_bytes,
            width: r.width,
            height: r.height,
        }
    }
}
#[uniffi::export]
impl Engine {
    /// # Errors
    /// Returns an error if the engine is stopped or stored workflows cannot be read.
    pub fn workflows(&self) -> Result<Vec<Workflow>, EngineError> {
        self.active()?;
        Ok(self
            .workflows
            .list()
            .map_err(failure)?
            .into_iter()
            .map(Into::into)
            .collect())
    }
    /// # Errors
    /// Returns an error for an invalid workflow, reserved shortcut, unavailable engine or persistence failure.
    pub fn save_workflow(&self, workflow: Workflow) -> Result<(), EngineError> {
        self.active()?;
        if workflow.enabled
            && let Some(shortcut) = &workflow.shortcut
        {
            if clipboard_shortcut_conflict(shortcut.key_code, shortcut.modifiers)? {
                return Err(failure("该快捷键已用于唤起剪贴板"));
            }
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
            if shortcut.key_code == 2 && shortcut.modifiers == 0x0012_0000 {
                return Err(failure("该快捷键用于打开文件暂存区"));
            }
        }
        self.workflows.save(workflow.into()).map_err(failure)?;
        self.refresh_workflow_hotkeys();
        Ok(())
    }
    /// # Errors
    /// Returns an error if the engine is stopped or the stored workflow cannot be deleted.
    pub fn delete_workflow(&self, id: String) -> Result<(), EngineError> {
        self.active()?;
        self.workflows.delete(&id).map_err(failure)?;
        self.refresh_workflow_hotkeys();
        Ok(())
    }
    /// # Errors
    /// Returns an error if the engine is stopped or the workflow/input cannot be admitted.
    pub fn run_workflow(
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
            WorkflowInput::Files { paths } => worker::WorkflowInput::Files(paths),
            WorkflowInput::Text { value } => worker::WorkflowInput::Text(value),
        };
        let output = match output {
            WorkflowOutputMode::Configured => worker::WorkflowOutputMode::Configured,
            WorkflowOutputMode::Copy => worker::WorkflowOutputMode::Copy,
            WorkflowOutputMode::ReturnFile => worker::WorkflowOutputMode::ReturnFile,
            WorkflowOutputMode::ReturnResult => worker::WorkflowOutputMode::ReturnResult,
        };
        self.workflows
            .start(workflow_id, input, output, target_pid)
            .map_err(failure)
    }
    /// # Errors
    /// Returns an error if the engine is stopped or the requested run is unavailable.
    pub fn workflow_run(&self, run_id: String) -> Result<WorkflowRun, EngineError> {
        self.active()?;
        self.workflows
            .status(&run_id)
            .map(Into::into)
            .map_err(failure)
    }
    /// # Errors
    /// Returns an error if the engine is stopped or the requested run cannot be cancelled.
    pub fn cancel_workflow(&self, run_id: String) -> Result<(), EngineError> {
        self.active()?;
        self.workflows.cancel(&run_id).map_err(failure)
    }
    pub fn workflow_catalog_json(&self) -> Result<String, EngineError> {
        self.active()?;
        let catalog = deskfuse_engine::workflow_catalog::global().map_err(failure)?;
        let mut view: serde_json::Value =
            serde_json::from_str(&catalog.view_json().map_err(failure)?).map_err(failure)?;
        let workflows = catalog.list().map_err(failure)?;
        let state = self.state.lock().map_err(failure)?;
        view["issues"] = serde_json::Value::Array(
            workflows
                .iter()
                .filter_map(|w| {
                    hotkey_issue(w, &workflows, &state.model.input_sharing.settings.shortcuts)
                        .map(|message| serde_json::json!({"workflow_id":w.id,"message":message}))
                })
                .collect(),
        );
        serde_json::to_string(&view).map_err(failure)
    }
    pub fn save_workflow_rule_json(&self, rule: String) -> Result<(), EngineError> {
        self.active()?;
        let rule: core_workflow::WorkflowRule = serde_json::from_str(&rule).map_err(failure)?;
        deskfuse_engine::workflow_catalog::global()
            .map_err(failure)?
            .save_rule(rule)
            .map_err(failure)
    }
    pub fn delete_workflow_rule(&self, id: String) -> Result<(), EngineError> {
        self.active()?;
        deskfuse_engine::workflow_catalog::global()
            .map_err(failure)?
            .delete_rule(&id)
            .map_err(failure)
    }
    pub fn bind_workflow_shortcut(
        &self,
        workflow_id: String,
        step_index: u32,
        shortcut_id: String,
    ) -> Result<(), EngineError> {
        self.active()?;
        deskfuse_engine::workflow_catalog::global()
            .map_err(failure)?
            .bind_shortcut(&workflow_id, step_index, shortcut_id)
            .map_err(failure)
    }
    pub fn restore_workflow_conflict(&self, id: String) -> Result<(), EngineError> {
        self.active()?;
        deskfuse_engine::workflow_catalog::global()
            .map_err(failure)?
            .restore_conflict(&id)
            .map_err(failure)
    }
    /// Admission returns immediately. Preparation and actual delivery report independent status.
    pub fn share_workflow(
        &self,
        workflow_id: Option<String>,
        history_id: Option<u64>,
        paths: Vec<String>,
        peer_id: String,
        bypass: bool,
    ) -> Result<String, EngineError> {
        self.active()?;
        let peer = deskfuse_core::DeviceId::from_hex(&peer_id).map_err(failure)?;
        let id = deskfuse_transfer::TransferId::new();
        let selection = if bypass {
            deskfuse_engine::workflow_sharing::Selection::Bypass
        } else if let Some(id) = workflow_id {
            deskfuse_engine::workflow_sharing::Selection::Id(id)
        } else {
            deskfuse_engine::workflow_sharing::Selection::Automatic
        };
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.bridge
            .register_share_source(id, cancel.clone())
            .map_err(failure)?;
        let bridge = self.bridge.clone();
        let spawned = std::thread::Builder::new()
            .name("deskfuse-share-source".into())
            .spawn(move || {
                let result = (|| {
                    let snapshot = if paths.is_empty() {
                        bridge.workflow_source(history_id, cancel.clone())?
                    } else {
                        deskfuse_platform::PasteboardSnapshot {
                            file_urls: paths,
                            ..Default::default()
                        }
                    };
                    bridge.share_snapshot(snapshot, peer, selection, id)
                })();
                if let Err(error) = result {
                    bridge.fail_share_source(id, error);
                }
            });
        if let Err(error) = spawned {
            self.bridge.release_share_source(id);
            return Err(failure(error));
        }
        Ok(id.to_string())
    }
    /// # Errors
    /// Returns an error if the engine is stopped or system shortcuts cannot be enumerated.
    pub fn system_shortcuts(&self) -> Result<Vec<SystemShortcut>, EngineError> {
        self.active()?;
        Ok(self
            .workflows
            .system_shortcuts()
            .map_err(failure)?
            .into_iter()
            .map(|s| SystemShortcut {
                id: s.id,
                name: s.name,
            })
            .collect())
    }
}
/// Shared reverse conflict guard for workflow and input shortcut updates.
pub(crate) fn clipboard_shortcut_conflict(
    key_code: u16,
    modifiers: u64,
) -> Result<bool, EngineError> {
    let settings = deskfuse_engine::clipboard_library::global()
        .map_err(failure)?
        .settings()
        .map_err(failure)?;
    Ok(settings.shortcut.enabled
        && settings.shortcut.key_code == key_code
        && settings.shortcut.modifiers == modifiers)
}
pub(crate) fn hotkey_issue(
    workflow: &core_workflow::Workflow,
    workflows: &[core_workflow::Workflow],
    bindings: &deskfuse_core::InputShortcutBindings,
) -> Option<String> {
    if !workflow.enabled {
        return None;
    }
    let shortcut = workflow.shortcut.as_ref()?;
    match clipboard_shortcut_conflict(shortcut.key_code, shortcut.modifiers) {
        Ok(true) => return Some("快捷键与剪贴板唤起冲突，未注册；请修改快捷键".into()),
        Err(_) => return Some("剪贴板快捷键设置读取失败，未注册工作流快捷键".into()),
        Ok(false) => {}
    }
    if [
        &bindings.left,
        &bindings.right,
        &bindings.up,
        &bindings.down,
        &bindings.lock,
        &bindings.emergency,
    ]
    .contains(&shortcut)
        || (shortcut.key_code == 2 && shortcut.modifiers == 0x0012_0000)
    {
        return Some("快捷键与本机系统操作冲突，未注册；流程仍可从菜单运行".into());
    }
    if workflows
        .iter()
        .any(|w| w.enabled && w.id != workflow.id && w.shortcut.as_ref() == Some(shortcut))
    {
        return Some("快捷键与其他工作流冲突，未注册；请修改快捷键".into());
    }
    None
}
pub(crate) fn hotkey_bindings(
    workflows: &[core_workflow::Workflow],
    bindings: &deskfuse_core::InputShortcutBindings,
) -> Vec<deskfuse_platform_macos::WorkflowHotkey> {
    workflows
        .iter()
        .filter(|w| w.enabled && hotkey_issue(w, workflows, bindings).is_none())
        .filter_map(|w| {
            w.shortcut
                .as_ref()
                .map(|s| deskfuse_platform_macos::WorkflowHotkey {
                    id: w.id.clone(),
                    key_code: s.key_code,
                    modifiers: s.modifiers,
                })
        })
        .collect()
}
impl Engine {
    fn refresh_workflow_hotkeys(&self) {
        if let (Ok(workflows), Ok(state), Ok(mut hotkeys)) = (
            self.workflows.list(),
            self.state.lock(),
            self.workflow_hotkeys.lock(),
        ) {
            hotkeys.bindings =
                hotkey_bindings(&workflows, &state.model.input_sharing.settings.shortcuts);
        }
    }
}
