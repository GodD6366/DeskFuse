//! Platform-independent clipboard boundary. Native formats are same-platform extensions.
use crate::{PlatformError, PlatformResult};
use std::sync::atomic::{AtomicI64, Ordering};

/// Private pasteboard type used to label content that uniDesk just wrote on
/// behalf of a peer. The value is only a content digest; it never contains
/// clipboard text, image bytes, or a peer identity.
pub const REMOTE_ORIGIN_MARKER_TYPE: &str = "com.unidesk.clipboard-origin";
/// Apple Universal Clipboard / 接力在系统剪贴板上打的标记。
pub const UNIVERSAL_CLIPBOARD_TYPE: &str = "com.apple.is-remote-clipboard";

/// One native representation of a clipboard item. Bytes stay opaque; the
/// originating application's type identifier is preserved without a whitelist.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PasteboardRepresentation {
    pub type_name: String,
    pub data: Vec<u8>,
}

/// An item can provide several equivalent representations (TIFF, PNG, RTF,
/// HTML, application-specific data). Preserve item boundaries and every format.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PasteboardItem {
    pub representations: Vec<PasteboardRepresentation>,
}

/// A point-in-time snapshot of the pasteboard contents that uniDesk can
/// synchronize. File URLs are kept separate from their Finder-provided text
/// representation so callers can transfer the referenced bytes instead of
/// accidentally creating a `.textClipping` file on the receiving Mac.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PasteboardSnapshot {
    /// Monotonic pasteboard change count.
    pub change_count: i64,
    /// UTF-8 text, if the pasteboard currently provides it.
    pub text: Option<String>,
    /// PNG bytes, if the pasteboard currently provides an image.
    pub png: Option<Vec<u8>>,
    /// File URL strings, if the pasteboard carries file references.
    pub file_urls: Vec<String>,
    /// All materialized native formats, grouped by original pasteboard item.
    pub items: Vec<PasteboardItem>,
}

/// Pasteboard boundary shared by the app worker and deterministic tests.
pub trait Pasteboard: Send + Sync {
    /// Return the current change count.
    ///
    /// # Errors
    /// Returns an unsupported-operation, native clipboard, permission, or provider-state error.
    fn change_count(&self) -> PlatformResult<i64>;

    /// Read the current contents.
    ///
    /// # Errors
    /// Returns an unsupported-operation, native clipboard, permission, or provider-state error.
    fn read(&self) -> PlatformResult<PasteboardSnapshot>;

    /// Replace the pasteboard with `text/plain` content.
    ///
    /// # Errors
    /// Returns an unsupported-operation, native clipboard, permission, or provider-state error.
    fn write_text(&self, text: &str) -> PlatformResult<()>;

    /// Replace the pasteboard with `image/png` content.
    ///
    /// # Errors
    /// Returns an unsupported-operation, native clipboard, permission, or provider-state error.
    fn write_png(&self, bytes: &[u8]) -> PlatformResult<()>;

    /// Replace the pasteboard with local file references.
    ///
    /// # Errors
    /// Returns an unsupported-operation, native clipboard, permission, or provider-state error.
    fn write_file_urls(&self, paths: &[String]) -> PlatformResult<()>;

    /// Remove stale pasteable content while a remote file is materializing.
    /// The private marker carries only its transfer identifier.
    ///
    /// # Errors
    /// Returns an unsupported-operation, native clipboard, permission, or provider-state error.
    fn clear_for_remote_file(&self, marker: &str) -> PlatformResult<()>;

    /// Replace content with all native representations, preserving item boundaries.
    ///
    /// # Errors
    /// Returns an unsupported-operation, native clipboard, permission, or provider-state error.
    fn write_items(&self, items: &[PasteboardItem]) -> PlatformResult<()>;

    /// Read uniDesk's private remote-origin marker, if the current pasteboard
    /// item carries one.
    ///
    /// # Errors
    /// Returns an unsupported-operation, native clipboard, permission, or provider-state error.
    fn read_remote_origin_marker(&self) -> PlatformResult<Option<String>>;

    /// Add a private marker after uniDesk writes remote content. Implementors
    /// must preserve the user-visible pasteboard representation.
    ///
    /// # Errors
    /// Returns an unsupported-operation, native clipboard, permission, or provider-state error.
    fn write_remote_origin_marker(&self, marker: &str) -> PlatformResult<()>;

    /// Whether the current pasteboard declares `type_name`, including types
    /// omitted from [`Self::read`] snapshots as bookkeeping.
    ///
    /// # Errors
    /// Returns an unsupported-operation, native clipboard, permission, or provider-state error.
    fn contains_type(&self, _type_name: &str) -> PlatformResult<bool> {
        Ok(false)
    }
}

/// Deterministic pasteboard used by unit tests.
#[derive(Debug, Default)]
pub struct MemoryPasteboard {
    change_count: AtomicI64,
    state: std::sync::Mutex<MemoryPasteboardState>,
}

#[derive(Debug, Default)]
struct MemoryPasteboardState {
    text: Option<String>,
    png: Option<Vec<u8>>,
    file_urls: Vec<String>,
    remote_origin_marker: Option<String>,
    items: Vec<PasteboardItem>,
    universal_clipboard: bool,
}

impl MemoryPasteboard {
    #[must_use]
    pub fn new() -> Self {
        Self {
            change_count: AtomicI64::new(0),
            state: std::sync::Mutex::new(MemoryPasteboardState::default()),
        }
    }

    /// 测试用：把当前内容标成接力回写，不新增一代剪贴板。
    pub fn mark_universal_clipboard(&self) -> PlatformResult<()> {
        self.state
            .lock()
            .map_err(|_| PlatformError::StatePoisoned {
                provider: "MemoryPasteboard",
            })?
            .universal_clipboard = true;
        Ok(())
    }
}

impl Pasteboard for MemoryPasteboard {
    fn change_count(&self) -> PlatformResult<i64> {
        Ok(self.change_count.load(Ordering::Relaxed))
    }

    fn read(&self) -> PlatformResult<PasteboardSnapshot> {
        let state = self
            .state
            .lock()
            .map_err(|_| PlatformError::StatePoisoned {
                provider: "MemoryPasteboard",
            })?;
        Ok(PasteboardSnapshot {
            change_count: self.change_count.load(Ordering::Relaxed),
            text: state.text.clone(),
            png: state.png.clone(),
            file_urls: state.file_urls.clone(),
            items: state
                .items
                .iter()
                .filter_map(|item| {
                    let representations = item
                        .representations
                        .iter()
                        .filter(|representation| {
                            representation.type_name != REMOTE_ORIGIN_MARKER_TYPE
                                && representation.type_name != UNIVERSAL_CLIPBOARD_TYPE
                        })
                        .cloned()
                        .collect::<Vec<_>>();
                    (!representations.is_empty()).then_some(PasteboardItem { representations })
                })
                .collect(),
        })
    }

    fn write_text(&self, text: &str) -> PlatformResult<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| PlatformError::StatePoisoned {
                provider: "MemoryPasteboard",
            })?;
        state.text = Some(text.to_owned());
        state.png = None;
        state.file_urls.clear();
        state.remote_origin_marker = None;
        state.items.clear();
        state.universal_clipboard = false;
        self.change_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn write_png(&self, bytes: &[u8]) -> PlatformResult<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| PlatformError::StatePoisoned {
                provider: "MemoryPasteboard",
            })?;
        state.text = None;
        state.png = Some(bytes.to_vec());
        state.file_urls.clear();
        state.remote_origin_marker = None;
        state.items.clear();
        state.universal_clipboard = false;
        self.change_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn write_file_urls(&self, paths: &[String]) -> PlatformResult<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| PlatformError::StatePoisoned {
                provider: "MemoryPasteboard",
            })?;
        state.text = None;
        state.png = None;
        state.file_urls = paths.to_vec();
        state.remote_origin_marker = None;
        state.items.clear();
        state.universal_clipboard = false;
        self.change_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn clear_for_remote_file(&self, marker: &str) -> PlatformResult<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| PlatformError::StatePoisoned {
                provider: "MemoryPasteboard",
            })?;
        *state = MemoryPasteboardState {
            remote_origin_marker: Some(marker.to_owned()),
            ..Default::default()
        };
        self.change_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn write_items(&self, items: &[PasteboardItem]) -> PlatformResult<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| PlatformError::StatePoisoned {
                provider: "MemoryPasteboard",
            })?;
        state.text = items
            .iter()
            .flat_map(|item| &item.representations)
            .find(|r| r.type_name == "public.utf8-plain-text")
            .and_then(|r| String::from_utf8(r.data.clone()).ok());
        state.png = items
            .iter()
            .flat_map(|item| &item.representations)
            .find(|r| r.type_name == "public.png")
            .map(|r| r.data.clone());
        state.file_urls.clear();
        state.items = items.to_vec();
        state.remote_origin_marker = items
            .iter()
            .flat_map(|item| &item.representations)
            .find(|representation| representation.type_name == REMOTE_ORIGIN_MARKER_TYPE)
            .and_then(|representation| String::from_utf8(representation.data.clone()).ok());
        state.universal_clipboard = items
            .iter()
            .flat_map(|item| &item.representations)
            .any(|r| r.type_name == UNIVERSAL_CLIPBOARD_TYPE);
        self.change_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn read_remote_origin_marker(&self) -> PlatformResult<Option<String>> {
        Ok(self
            .state
            .lock()
            .map_err(|_| PlatformError::StatePoisoned {
                provider: "MemoryPasteboard",
            })?
            .remote_origin_marker
            .clone())
    }

    fn write_remote_origin_marker(&self, marker: &str) -> PlatformResult<()> {
        self.state
            .lock()
            .map_err(|_| PlatformError::StatePoisoned {
                provider: "MemoryPasteboard",
            })?
            .remote_origin_marker = Some(marker.to_owned());
        self.change_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn contains_type(&self, type_name: &str) -> PlatformResult<bool> {
        let state = self
            .state
            .lock()
            .map_err(|_| PlatformError::StatePoisoned {
                provider: "MemoryPasteboard",
            })?;
        if type_name == UNIVERSAL_CLIPBOARD_TYPE && state.universal_clipboard {
            return Ok(true);
        }
        Ok(state
            .items
            .iter()
            .flat_map(|item| &item.representations)
            .any(|rep| rep.type_name == type_name))
    }
}
