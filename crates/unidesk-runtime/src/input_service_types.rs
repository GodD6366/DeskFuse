//! Shared status vocabulary; non-Unix clients do not launch the macOS service.
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
#[cfg(unix)]
use std::{io::Write, os::unix::net::UnixStream};
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NativeStatus {
    pub running: bool,
    pub pid: u32,
    pub locked: bool,
    pub recording_ready: bool,
    pub update_paused: bool,
    pub error: Option<String>,
    pub persistence_pending: usize,
    pub accessibility: Option<u8>,
    pub input_monitoring: Option<u8>,
}
#[derive(Clone, Default)]
pub struct ServiceControl {
    #[cfg(unix)]
    pub(crate) urgent: Arc<Mutex<Option<UnixStream>>>,
    pub status: Arc<Mutex<NativeStatus>>,
    pub diagnostics: Arc<Mutex<String>>,
    pub(crate) sequence: Arc<Mutex<u64>>,
}
impl ServiceControl {
    pub fn signal(&self, byte: u8) -> bool {
        #[cfg(unix)]
        {
            self.urgent
                .lock()
                .ok()
                .and_then(|mut s| s.as_mut().map(|s| s.write_all(&[byte]).is_ok()))
                .unwrap_or(false)
        }
        #[cfg(not(unix))]
        {
            let _ = byte;
            false
        }
    }

    pub fn paused(&self) -> bool {
        self.status
            .lock()
            .is_ok_and(|s| s.running && s.update_paused)
    }
}
