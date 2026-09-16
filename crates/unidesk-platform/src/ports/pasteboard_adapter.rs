//! Bridge the existing synchronous [`Pasteboard`] trait onto Host Ports.
use super::{
    ApplyResult, CaptureSource, CapturedContent, ClipboardApplyPort, ClipboardCapturePort,
    ContentData, ContentToApply, ContentType, NativeRevision,
};
use crate::{Pasteboard, PlatformResult, REMOTE_ORIGIN_MARKER_TYPE};
use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Wraps a [`Pasteboard`] so collaboration can capture and apply through Host Ports.
#[derive(Debug)]
pub struct PasteboardPort<P: Pasteboard> {
    pasteboard: P,
    remote_write: AtomicBool,
}

impl<P: Pasteboard> PasteboardPort<P> {
    #[must_use]
    pub fn new(pasteboard: P) -> Self {
        Self {
            pasteboard,
            remote_write: AtomicBool::new(false),
        }
    }

    #[must_use]
    pub fn inner(&self) -> &P {
        &self.pasteboard
    }

    fn captured_at_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0)
    }
}

fn content_from_snapshot(snapshot: &crate::PasteboardSnapshot) -> ContentData {
    let mut data = if !snapshot.file_urls.is_empty() {
        ContentData {
            mime: "application/x-file-urls".into(),
            bytes: Vec::new(),
            file_urls: snapshot.file_urls.clone(),
            items: Vec::new(),
        }
    } else if let Some(png) = &snapshot.png {
        ContentData {
            mime: "image/png".into(),
            bytes: png.clone(),
            file_urls: Vec::new(),
            items: Vec::new(),
        }
    } else if let Some(text) = &snapshot.text {
        ContentData::text(text.clone())
    } else {
        ContentData::default()
    };
    data.items = snapshot.items.clone();
    data
}

/// Apply remote clipboard content through the Host Ports contract.
///
/// Hosts that already hold a [`Pasteboard`] (the engine worker) use this
/// instead of calling `write_*` directly, so revision checks and remote-origin
/// marking stay in one place.
pub fn apply_clipboard(
    pasteboard: &impl Pasteboard,
    content: ContentToApply,
    expected_revision: NativeRevision,
) -> PlatformResult<ApplyResult> {
    let current = pasteboard.change_count()?;
    if current != expected_revision.value() {
        return Ok(ApplyResult::Superseded {
            current_revision: NativeRevision(current),
        });
    }
    if !content.data.items.is_empty() {
        pasteboard.write_items(&content.data.items)?;
    } else if !content.data.file_urls.is_empty() {
        pasteboard.write_file_urls(&content.data.file_urls)?;
    } else if content.data.mime == "image/png" && !content.data.bytes.is_empty() {
        pasteboard.write_png(&content.data.bytes)?;
    } else if let Some(text) = content.data.as_text() {
        pasteboard.write_text(text)?;
    } else if !content.data.bytes.is_empty() {
        pasteboard.write_text(&String::from_utf8_lossy(&content.data.bytes))?;
    } else {
        return Ok(ApplyResult::Rejected {
            reason: "empty clipboard payload".into(),
        });
    }
    if let Some(marker) = &content.remote_origin_marker {
        pasteboard.write_remote_origin_marker(marker)?;
    } else {
        pasteboard.write_remote_origin_marker(REMOTE_ORIGIN_MARKER_TYPE)?;
    }
    Ok(ApplyResult::Applied {
        new_revision: NativeRevision(pasteboard.change_count()?),
    })
}

#[async_trait]
impl<P: Pasteboard + Sync> ClipboardCapturePort for PasteboardPort<P> {
    async fn start_monitoring(&self) -> PlatformResult<()> {
        Ok(())
    }

    async fn capture(&self) -> PlatformResult<CapturedContent> {
        let snapshot = self.pasteboard.read()?;
        let marked_remote = self.pasteboard.read_remote_origin_marker()?.is_some()
            || self.remote_write.swap(false, Ordering::AcqRel);
        let data = content_from_snapshot(&snapshot);
        let types = if data.mime.is_empty() {
            Vec::new()
        } else {
            vec![ContentType(data.mime.clone())]
        };
        Ok(CapturedContent {
            revision: NativeRevision(snapshot.change_count),
            types,
            data,
            source: if marked_remote {
                CaptureSource::Remote
            } else {
                CaptureSource::User
            },
            captured_at_ms: Self::captured_at_ms(),
        })
    }

    async fn stop_monitoring(&self) -> PlatformResult<()> {
        Ok(())
    }
}

#[async_trait]
impl<P: Pasteboard + Sync> ClipboardApplyPort for PasteboardPort<P> {
    async fn apply(
        &self,
        content: ContentToApply,
        expected_revision: NativeRevision,
    ) -> PlatformResult<ApplyResult> {
        let result = apply_clipboard(&self.pasteboard, content, expected_revision)?;
        if matches!(result, ApplyResult::Applied { .. }) {
            self.remote_write.store(true, Ordering::Release);
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryPasteboard;
    use crate::ports::ContentToApply;

    #[tokio::test]
    async fn apply_is_superseded_when_revision_moves() {
        let port = PasteboardPort::new(MemoryPasteboard::new());
        port.inner().write_text("local").unwrap();
        let result = port
            .apply(
                ContentToApply {
                    data: ContentData::text("remote"),
                    remote_origin_marker: Some("op".into()),
                },
                NativeRevision(0),
            )
            .await
            .unwrap();
        assert!(matches!(result, ApplyResult::Superseded { .. }));
        assert_eq!(port.inner().read().unwrap().text.as_deref(), Some("local"));
    }

    #[tokio::test]
    async fn apply_marks_capture_as_remote() {
        let port = PasteboardPort::new(MemoryPasteboard::new());
        let applied = port
            .apply(
                ContentToApply {
                    data: ContentData::text("from peer"),
                    remote_origin_marker: Some("op".into()),
                },
                NativeRevision(0),
            )
            .await
            .unwrap();
        assert!(matches!(applied, ApplyResult::Applied { .. }));
        let captured = port.capture().await.unwrap();
        assert_eq!(captured.source, CaptureSource::Remote);
        assert_eq!(captured.data.as_text(), Some("from peer"));
    }

    #[test]
    fn apply_clipboard_writes_native_items() {
        let pasteboard = MemoryPasteboard::new();
        let items = vec![crate::PasteboardItem {
            representations: vec![crate::PasteboardRepresentation {
                type_name: "public.utf8-plain-text".into(),
                data: b"native".to_vec(),
            }],
        }];
        let result = apply_clipboard(
            &pasteboard,
            ContentToApply {
                data: ContentData {
                    mime: "text/plain".into(),
                    bytes: b"native".to_vec(),
                    file_urls: Vec::new(),
                    items: items.clone(),
                },
                remote_origin_marker: Some("op".into()),
            },
            NativeRevision(0),
        )
        .unwrap();
        assert!(matches!(result, ApplyResult::Applied { .. }));
        assert_eq!(pasteboard.read().unwrap().items, items);
    }
}
