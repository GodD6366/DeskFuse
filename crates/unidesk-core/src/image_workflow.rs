//! Persisted, versioned image workflows shared by native clients and the engine.
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
    Copy,
    Paste,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageOutputShape {
    FollowSource,
    Image,
    File,
}
/// A flat parameter record keeps native step editing and persisted JSON explicit.
#[derive(Clone, Debug, Serialize, Deserialize)]
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
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImageWorkflow {
    pub version: u32,
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub shortcut: Option<crate::InputShortcut>,
    pub steps: Vec<ImageWorkflowStep>,
}
impl ImageWorkflow {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1
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
        for (i, s) in self.steps.iter().enumerate() {
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
