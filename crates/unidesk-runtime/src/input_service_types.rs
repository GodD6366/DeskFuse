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
    pub remote_application_viewer_paused: bool,
    pub remote_application_lease: Option<String>,
    #[serde(default)]
    pub remote_application_request: Option<String>,
    #[serde(default)]
    pub remote_application_error: Option<String>,
    pub error: Option<String>,
    pub persistence_pending: usize,
    pub accessibility: Option<u8>,
    pub input_monitoring: Option<u8>,
}
#[derive(Clone, Default)]
pub struct ServiceControl {
    pub(crate) remote: Arc<Mutex<Option<std::sync::mpsc::SyncSender<RemoteApplicationInput>>>>,
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

/// Separate remote-application lease, delivered to the existing signed helper.
/// Quartz global coordinates and native key codes; never a raw UI event object.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemoteApplicationInput {
    #[serde(default)]
    pub peer: Option<unidesk_core::DeviceId>,
    pub kind: String,
    pub session: String,
    pub generation: u64,
    #[serde(default)]
    pub sequence: u64,
    #[serde(default)]
    pub pid: i32,
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub button: u8,
    #[serde(default)]
    pub key: u16,
    #[serde(default)]
    pub flags: u64,
    #[serde(default)]
    pub repeat: bool,
    #[serde(default)]
    pub dx: f64,
    #[serde(default)]
    pub dy: f64,
}
impl ServiceControl {
    pub fn remote_application(&self, packet: RemoteApplicationInput) -> Result<(), String> {
        self.remote
            .lock()
            .map_err(|_| "输入通道不可用")?
            .as_ref()
            .ok_or("输入进程尚未就绪")?
            .try_send(packet)
            .map_err(|_| "远程输入队列已满或关闭".into())
    }
}

/// Update resumes may only release update-owned states (1=requested,
/// 2=paused, 3=forced). Remote application host/viewer ownership is 4..=7.
/// The updater calls this even when no update is pending.
pub fn resume_update_pause(pause: &std::sync::atomic::AtomicU8) {
    use std::sync::atomic::Ordering;
    let _ = pause.fetch_update(Ordering::AcqRel, Ordering::Acquire, |state| {
        matches!(state, 1..=3).then_some(0)
    });
}
