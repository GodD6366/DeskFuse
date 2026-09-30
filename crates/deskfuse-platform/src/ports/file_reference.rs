//! Typed file-reference capture shared by Finder and the collaboration engine.
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Immutable metadata for a remote file reference frozen at paste time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilePasteReference {
    pub transfer_id: String,
    pub peer_id: String,
    pub change_count: i64,
    pub name: String,
    pub total_bytes: u64,
}

/// A file-reference paste request captured by the native keyboard boundary.
pub struct CapturedFilePaste {
    pub captured_at: Instant,
    pub reference: FilePasteReference,
    pub target_pid: i32,
}

/// Thread-safe state shared by a host's native paste interception.
pub struct FilePasteCapture {
    reference: Mutex<Option<FilePasteReference>>,
    events: tokio::sync::mpsc::UnboundedSender<CapturedFilePaste>,
}

impl FilePasteCapture {
    pub fn new() -> Result<
        (
            Arc<Self>,
            tokio::sync::mpsc::UnboundedReceiver<CapturedFilePaste>,
        ),
        String,
    > {
        let (events, receiver) = tokio::sync::mpsc::unbounded_channel();
        Ok((
            Arc::new(Self {
                reference: Mutex::new(None),
                events,
            }),
            receiver,
        ))
    }

    pub fn update(&self, reference: Option<FilePasteReference>) {
        if let Ok(mut current) = self.reference.lock() {
            *current = reference;
        }
    }

    pub fn has_reference(&self) -> bool {
        self.reference.try_lock().is_ok_and(|r| r.is_some())
    }

    pub fn has_reference_at(&self, change_count: i64) -> bool {
        self.reference.try_lock().is_ok_and(|reference| {
            reference
                .as_ref()
                .is_some_and(|reference| reference.change_count == change_count)
        })
    }

    /// Queue immediately without native queries or filesystem work.
    pub fn request(&self, pid: i32, change_count: i64) -> bool {
        let Some(reference) = self.reference.try_lock().ok().and_then(|r| r.clone()) else {
            return false;
        };
        if reference.change_count != change_count {
            return false;
        }
        self.events
            .send(CapturedFilePaste {
                captured_at: Instant::now(),
                reference,
                target_pid: pid,
            })
            .is_ok()
    }
}
