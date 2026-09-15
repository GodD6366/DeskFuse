//! The event tap freezes the file reference and queues a paste intention.
//! The engine shows path acquisition before starting an on-demand native query.
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Clone, Debug)]
pub struct FilePasteReference {
    pub transfer_id: String,
    pub peer_id: String,
    pub change_count: i64,
    pub name: String,
    pub total_bytes: u64,
}

pub struct CapturedFilePaste {
    pub captured_at: Instant,
    pub reference: FilePasteReference,
    pub target_pid: i32,
}
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
        let capture = Arc::new(Self {
            reference: Mutex::new(None),
            events,
        });
        Ok((capture, receiver))
    }
    pub fn update(&self, reference: Option<FilePasteReference>) {
        if let Ok(mut current) = self.reference.lock() {
            *current = reference;
        }
    }
    pub fn has_reference(&self) -> bool {
        self.reference.try_lock().is_ok_and(|r| r.is_some())
    }
    /// Queue immediately without AX, AppleScript, filesystem work or an expiring
    /// directory cache on the keyboard callback.
    pub fn request(&self, pid: i32) -> bool {
        let Some(reference) = self.reference.try_lock().ok().and_then(|r| r.clone()) else {
            return false;
        };
        self.events
            .send(CapturedFilePaste {
                captured_at: Instant::now(),
                reference,
                target_pid: pid,
            })
            .is_ok()
    }
}
