//! Clipboard capture and apply ports. Hosts implement these; collaboration owns policy.
use crate::{PlatformError, PlatformResult};
use async_trait::async_trait;

/// Opaque native pasteboard revision (change count on macOS/Windows).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NativeRevision(pub i64);

impl NativeRevision {
    #[must_use]
    pub const fn new(value: i64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn value(self) -> i64 {
        self.0
    }
}

/// How the current pasteboard contents were produced.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CaptureSource {
    /// A local user or application wrote the pasteboard.
    User,
    /// uniDesk wrote remote content; collaboration must not echo it back.
    Remote,
    #[default]
    Unknown,
}

/// A MIME-like type identifier preserved from the native pasteboard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentType(pub String);

/// Opaque clipboard body. Collaboration never logs these bytes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContentData {
    pub mime: String,
    pub bytes: Vec<u8>,
    pub file_urls: Vec<String>,
    /// Native representations, preserved when the host can round-trip them.
    pub items: Vec<crate::PasteboardItem>,
}

impl ContentData {
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            mime: "text/plain".into(),
            bytes: text.into_bytes(),
            file_urls: Vec::new(),
            items: Vec::new(),
        }
    }

    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        if self.mime == "text/plain" {
            std::str::from_utf8(&self.bytes).ok()
        } else {
            None
        }
    }
}

/// Point-in-time capture used as the collaboration input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapturedContent {
    pub revision: NativeRevision,
    pub types: Vec<ContentType>,
    pub data: ContentData,
    pub source: CaptureSource,
    /// Host-monotonic milliseconds. Used to drop delayed remote frames.
    pub captured_at_ms: u64,
}

impl CapturedContent {
    #[must_use]
    pub fn text(revision: NativeRevision, text: impl Into<String>, source: CaptureSource) -> Self {
        Self::text_at(revision, text, source, 0)
    }

    #[must_use]
    pub fn text_at(
        revision: NativeRevision,
        text: impl Into<String>,
        source: CaptureSource,
        captured_at_ms: u64,
    ) -> Self {
        let data = ContentData::text(text);
        Self {
            revision,
            types: vec![ContentType("text/plain".into())],
            data,
            source,
            captured_at_ms,
        }
    }
}

/// Payload the host should attempt to write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentToApply {
    pub data: ContentData,
    pub remote_origin_marker: Option<String>,
}

/// Result of a native apply attempt. Collaboration decides whether to ACK.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApplyResult {
    Applied { new_revision: NativeRevision },
    Superseded { current_revision: NativeRevision },
    Rejected { reason: String },
}

/// Listen for native clipboard changes.
#[async_trait]
pub trait ClipboardCapturePort: Send + Sync {
    /// Begin observing the system pasteboard.
    async fn start_monitoring(&self) -> PlatformResult<()>;

    /// Read the current pasteboard. Unchanged contents still return the current revision.
    async fn capture(&self) -> PlatformResult<CapturedContent>;

    /// Stop observing the system pasteboard.
    async fn stop_monitoring(&self) -> PlatformResult<()>;
}

/// Write remote clipboard contents onto the native pasteboard.
#[async_trait]
pub trait ClipboardApplyPort: Send + Sync {
    /// Apply `content` only if the native revision still matches `expected_revision`.
    async fn apply(
        &self,
        content: ContentToApply,
        expected_revision: NativeRevision,
    ) -> PlatformResult<ApplyResult>;
}

#[derive(Debug)]
struct MemoryClipboardState {
    revision: i64,
    data: ContentData,
    source: CaptureSource,
    remote_origin_marker: Option<String>,
}

impl Default for MemoryClipboardState {
    fn default() -> Self {
        Self {
            revision: 0,
            data: ContentData::default(),
            source: CaptureSource::Unknown,
            remote_origin_marker: None,
        }
    }
}

/// In-memory clipboard used by tests and host-free cores.
#[derive(Debug, Default)]
pub struct MemoryClipboard {
    state: std::sync::Mutex<MemoryClipboardState>,
}

impl MemoryClipboard {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Simulate a local user copy. Tests drive collaboration through this.
    pub fn capture_text(&self, text: impl Into<String>) -> PlatformResult<CapturedContent> {
        let mut state = self.lock()?;
        state.revision += 1;
        state.data = ContentData::text(text);
        state.source = CaptureSource::User;
        state.remote_origin_marker = None;
        Ok(CapturedContent::text_at(
            NativeRevision(state.revision),
            String::from_utf8_lossy(&state.data.bytes).into_owned(),
            CaptureSource::User,
            state.revision as u64,
        ))
    }

    pub fn apply_count(&self) -> PlatformResult<i64> {
        Ok(self.lock()?.revision)
    }

    pub fn current_text(&self) -> PlatformResult<Option<String>> {
        Ok(self.lock()?.data.as_text().map(str::to_owned))
    }

    fn lock(&self) -> PlatformResult<std::sync::MutexGuard<'_, MemoryClipboardState>> {
        self.state.lock().map_err(|_| PlatformError::StatePoisoned {
            provider: "MemoryClipboard",
        })
    }
}

#[async_trait]
impl ClipboardCapturePort for MemoryClipboard {
    async fn start_monitoring(&self) -> PlatformResult<()> {
        Ok(())
    }

    async fn capture(&self) -> PlatformResult<CapturedContent> {
        let state = self.lock()?;
        Ok(CapturedContent {
            revision: NativeRevision(state.revision),
            types: vec![ContentType(state.data.mime.clone())],
            data: state.data.clone(),
            source: state.source,
            captured_at_ms: state.revision as u64,
        })
    }

    async fn stop_monitoring(&self) -> PlatformResult<()> {
        Ok(())
    }
}

#[async_trait]
impl ClipboardApplyPort for MemoryClipboard {
    async fn apply(
        &self,
        content: ContentToApply,
        expected_revision: NativeRevision,
    ) -> PlatformResult<ApplyResult> {
        let mut state = self.lock()?;
        if state.revision != expected_revision.value() {
            return Ok(ApplyResult::Superseded {
                current_revision: NativeRevision(state.revision),
            });
        }
        if content.data.mime.is_empty() && content.data.bytes.is_empty() {
            return Ok(ApplyResult::Rejected {
                reason: "empty clipboard payload".into(),
            });
        }
        state.revision += 1;
        state.data = content.data;
        state.source = CaptureSource::Remote;
        state.remote_origin_marker = content.remote_origin_marker;
        Ok(ApplyResult::Applied {
            new_revision: NativeRevision(state.revision),
        })
    }
}
