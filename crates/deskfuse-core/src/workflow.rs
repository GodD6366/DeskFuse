//! Typed workflow definitions shared by native clients and the engine.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Webp,
    Heic,
}
impl ImageFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Webp => "webp",
            Self::Heic => "heic",
        }
    }
    pub fn uti(self) -> &'static str {
        match self {
            Self::Png => "public.png",
            Self::Jpeg => "public.jpeg",
            Self::Webp => "org.webmproject.webp",
            Self::Heic => "public.heic",
        }
    }
    pub fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Webp => "image/webp",
            Self::Heic => "image/heic",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageOutputShape {
    FollowSource,
    Image,
    File,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkflowContentType {
    #[default]
    Any,
    Text,
    Image,
    Files,
}

/// A flat parameter record keeps native step editing and persisted JSON explicit.
#[derive(Clone, Debug, Serialize, Deserialize)]
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
    #[serde(default)]
    pub find: String,
    #[serde(default)]
    pub replacement: String,
    #[serde(default)]
    pub prefix: String,
    #[serde(default)]
    pub suffix: String,
    #[serde(default)]
    pub shortcut_output: WorkflowContentType,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Workflow {
    pub version: u32,
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub shortcut: Option<crate::InputShortcut>,
    pub steps: Vec<WorkflowStep>,
    #[serde(default)]
    pub input_type: WorkflowContentType,
}
impl Workflow {
    pub fn validate(&self) -> Result<(), String> {
        if !matches!(self.version, 1 | 2)
            || self.id.is_empty()
            || self.id.len() > 128
            || self.name.trim().is_empty()
            || self.name.chars().count() > 80
        {
            return Err("流程名称或版本无效".into());
        }
        if self.steps.is_empty() || self.steps.len() > 32 {
            return Err("流程需要 1–32 个步骤".into());
        }
        if let Some(s) = &self.shortcut
            && (s.key_code > 127
                || s.modifiers & !0x1e0000 != 0
                || s.modifiers & 0x1c0000 == 0
                || (s.key_code == 9 && s.modifiers == 0x100000)
                || (s.key_code == 53 && s.modifiers & 0x1c0000 == 0x1c0000))
        {
            return Err("快捷键必须含 ⌘/⌥/⌃，且不能占用普通粘贴或紧急收回".into());
        }
        let mut current_type = self.input_type;
        for (i, s) in self.steps.iter().enumerate() {
            let accepts = match s.kind {
                WorkflowStepKind::TextReplace | WorkflowStepKind::TextTrim => matches!(
                    current_type,
                    WorkflowContentType::Any | WorkflowContentType::Text
                ),
                WorkflowStepKind::Rename | WorkflowStepKind::Zip => matches!(
                    current_type,
                    WorkflowContentType::Any | WorkflowContentType::Files
                ),
                WorkflowStepKind::Compress
                | WorkflowStepKind::Resize
                | WorkflowStepKind::Convert => matches!(
                    current_type,
                    WorkflowContentType::Any
                        | WorkflowContentType::Image
                        | WorkflowContentType::Files
                ),
                _ => true,
            };
            if !accepts {
                return Err(format!("第 {} 步不接受前一步的内容类型", i + 1));
            }
            current_type = match s.kind {
                WorkflowStepKind::TextReplace | WorkflowStepKind::TextTrim => {
                    WorkflowContentType::Text
                }
                WorkflowStepKind::Rename | WorkflowStepKind::Zip => WorkflowContentType::Files,
                WorkflowStepKind::SystemShortcut => s.shortcut_output,
                _ => current_type,
            };
            if [
                s.find.as_str(),
                s.replacement.as_str(),
                s.prefix.as_str(),
                s.suffix.as_str(),
            ]
            .iter()
            .any(|v| v.len() > 16384)
            {
                return Err("步骤文字参数过长".into());
            }
            if s.kind == WorkflowStepKind::TextReplace && s.find.is_empty() {
                return Err("查找文字不能为空".into());
            }
            if s.kind == WorkflowStepKind::Rename
                && [s.replacement.as_str(), s.prefix.as_str(), s.suffix.as_str()]
                    .iter()
                    .any(|v| v.contains('/') || v.contains('\0'))
            {
                return Err("文件名称不能包含路径分隔符".into());
            }
            let output = matches!(s.kind, WorkflowStepKind::Copy | WorkflowStepKind::Paste);
            if output != (i + 1 == self.steps.len()) {
                return Err("流程必须且只能以一个复制或粘贴步骤结束".into());
            }
            if !(1..=100).contains(&s.quality)
                || s.target_bytes
                    .is_some_and(|n| !(1024..=128 * 1024 * 1024).contains(&n))
                || s.max_edge.is_some_and(|n| n == 0 || n > 16384)
                || s.scale_percent.is_some_and(|n| n == 0 || n > 100)
            {
                return Err("图片参数超出允许范围".into());
            }
            if s.kind == WorkflowStepKind::Resize
                && (s.max_edge.is_some() == s.scale_percent.is_some())
            {
                return Err("尺寸步骤请选择最长边或比例中的一种".into());
            }
            if s.kind == WorkflowStepKind::SystemShortcut
                && (s.shortcut_id.is_empty() || s.shortcut_id.len() > 256)
            {
                return Err("请选择系统快捷指令".into());
            }
        }
        Ok(())
    }
}

/// Synced rule; only the identified sender executes it. Empty target means all peers.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkflowRule {
    pub id: String,
    pub enabled: bool,
    pub sender_id: String,
    pub target_id: String,
    pub content_type: WorkflowContentType,
    pub workflow_id: String,
    pub position: u32,
}
impl WorkflowRule {
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty()
            || self.id.len() > 128
            || self.workflow_id.is_empty()
            || self.workflow_id.len() > 128
            || self.sender_id.is_empty()
            || self.sender_id.len() > 128
            || self.target_id.len() > 128
        {
            return Err("共享规则无效".into());
        }
        Ok(())
    }
}
