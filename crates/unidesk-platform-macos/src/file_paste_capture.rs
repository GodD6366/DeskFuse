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
    pub fn has_reference_at(&self, change_count: i64) -> bool {
        self.reference.try_lock().is_ok_and(|reference| {
            reference
                .as_ref()
                .is_some_and(|r| r.change_count == change_count)
        })
    }
    /// Queue immediately without AX, AppleScript, filesystem work or an expiring
    /// directory cache on the keyboard callback.
    pub fn request(&self, pid: i32, change_count: i64) -> bool {
        let Some(reference) = self.reference.try_lock().ok().and_then(|r| r.clone()) else {
            return false;
        };
        // The engine's cached reference can lag behind a local screenshot/copy.
        // Freeze only the generation observed at key-down, not an old offer.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn reference() -> FilePasteReference {
        FilePasteReference {
            transfer_id: "remote-transfer".into(),
            peer_id: "remote-peer".into(),
            change_count: 10,
            name: "file.png".into(),
            total_bytes: 1,
        }
    }

    #[test]
    fn local_copy_does_not_capture_stale_remote_reference() {
        let (capture, mut events) = FilePasteCapture::new().unwrap();
        capture.update(Some(reference()));
        assert!(capture.has_reference()); // Engine has not polled the local copy yet.
        assert!(!capture.has_reference_at(11)); // Includes held Command-V repeats.
        assert!(!capture.request(42, 11));
        assert!(events.try_recv().is_err());
    }

    #[test]
    fn valid_key_down_keeps_its_reference_after_a_new_copy() {
        let (capture, mut events) = FilePasteCapture::new().unwrap();
        capture.update(Some(reference()));
        assert!(capture.has_reference_at(10));
        assert!(capture.request(42, 10));
        capture.update(None);
        let captured = events.try_recv().unwrap();
        assert_eq!(captured.reference.change_count, 10);
        assert_eq!(captured.reference.transfer_id, "remote-transfer");
        assert_eq!(captured.target_pid, 42);
        assert!(!capture.request(42, 11));
        assert!(events.try_recv().is_err());
    }
}
