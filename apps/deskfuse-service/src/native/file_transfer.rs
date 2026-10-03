//! Desktop-service file clipboard integration: authenticated frame delivery
//! over the runtime clipboard session, plus the local computer display name
//! shared with macOS (hostname, never used as identity).

use deskfuse_core::DeviceId;
use deskfuse_runtime::RuntimeHandle;
use deskfuse_transfer::{ClipboardFileFrame, FileBridge, FileFrameSink};
use std::io;
use std::sync::{Arc, OnceLock};
use tokio::sync::mpsc;

const MAX_PENDING_FRAMES: usize = 256;

static SINK: OnceLock<FileSink> = OnceLock::new();
static CONSUMER: std::sync::Mutex<Option<mpsc::Receiver<(DeviceId, ClipboardFileFrame)>>> =
    std::sync::Mutex::new(None);
static CACHE_ROOT: OnceLock<std::path::PathBuf> = OnceLock::new();

/// The process-wide frame sink; worker threads keep it alive across session
/// resets. The consumer receiver is handed to the runtime exactly once.
#[must_use]
pub fn global_sink() -> &'static FileSink {
    SINK.get_or_init(|| {
        let (sink, consumer) = FileSink::new();
        if let Ok(mut guard) = CONSUMER.lock() {
            *guard = Some(consumer);
        }
        sink
    })
}

struct SinkCore {
    runtime: OnceLock<RuntimeHandle>,
    queue: mpsc::Sender<(DeviceId, ClipboardFileFrame)>,
}

/// Fire-and-forget frame sink shared by the desktop bridge and its worker
/// threads. The consumer task owns the runtime handle and writes each frame as
/// a direct `CLIPBOARD_FILE_MIME` clipboard payload, like the macOS engine's
/// `send_file_clipboard_frame`.
#[derive(Clone)]
pub struct FileSink {
    core: Arc<SinkCore>,
}

impl FileSink {
    #[must_use]
    pub fn new() -> (Self, mpsc::Receiver<(DeviceId, ClipboardFileFrame)>) {
        let (queue, consumer) = mpsc::channel(MAX_PENDING_FRAMES);
        (
            Self {
                core: Arc::new(SinkCore {
                    runtime: OnceLock::new(),
                    queue,
                }),
            },
            consumer,
        )
    }

    /// Activates delivery once the runtime exists; earlier frames were queued.
    pub fn attach_runtime(&self, runtime: RuntimeHandle) {
        let _ = self.core.runtime.set(runtime);
    }
}

impl Default for FileSink {
    fn default() -> Self {
        Self::new().0
    }
}

impl FileFrameSink for FileSink {
    fn send(&self, peer: DeviceId, frame: ClipboardFileFrame) -> Result<(), String> {
        if self.core.runtime.get().is_none() {
            return Err("文件通道尚未就绪".into());
        }
        match self.core.queue.try_send((peer, frame)) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => Err("文件通道队列已满".into()),
            Err(mpsc::error::TrySendError::Closed(_)) => Err("文件通道已关闭".into()),
        }
    }
}

/// Consumes queued frames and delivers them over the runtime. Frames must
/// ride the peer's current control generation; a stale generation is dropped
/// because the receiving side would refuse it anyway.
pub fn spawn_consumer(
    runtime: RuntimeHandle,
    mut consumer: mpsc::Receiver<(DeviceId, ClipboardFileFrame)>,
) {
    tokio::spawn(async move {
        while let Some((peer, frame)) = consumer.recv().await {
            let Ok(data) = frame.encode() else { continue };
            let Ok(payload) = deskfuse_core::ClipboardPayload::new(
                deskfuse_transfer::CLIPBOARD_FILE_MIME,
                data,
                0,
            ) else {
                continue;
            };
            let Some(generation) = runtime.control_generation(peer) else {
                continue;
            };
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                runtime.send_clipboard_on_session(peer, generation, payload),
            )
            .await;
        }
    });
}

/// The local computer's DNS hostname, matching the macOS engine's
/// `/bin/hostname` display label. Never used as identity, only for display.
#[must_use]
pub fn computer_display_name() -> String {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::SystemInformation::{
            ComputerNameDnsHostname, GetComputerNameExW,
        };
        let mut buffer = [0_u16; 256];
        let mut size = u32::try_from(buffer.len()).unwrap_or(0);
        // SAFETY: writable buffer with explicit size; the call writes at most
        // `size` UTF-16 units plus a terminator.
        let ok =
            unsafe { GetComputerNameExW(ComputerNameDnsHostname, buffer.as_mut_ptr(), &mut size) };
        if ok != 0 && size > 0 {
            let len = (size as usize).min(buffer.len());
            let name = String::from_utf16_lossy(&buffer[..len.min(buffer.len().saturating_sub(1))]);
            let trimmed = name.trim_end_matches('\0').trim().to_owned();
            let bounded = trimmed.chars().take(64).collect::<String>();
            if !bounded.is_empty() && !bounded.chars().any(char::is_control) {
                return bounded;
            }
        }
    }
    "DeskFuse Windows".to_owned()
}

/// Creates (or re-secures) the transfer cache directory. The service keeps
/// full control; the interactive user receives read+execute so the desktop
/// process can copy verified staged files into the user's own destination.
/// No write, delete or take-ownership right is granted to the user.
pub(super) fn open_transfer_cache(path: &std::path::Path) -> io::Result<()> {
    const DIRECTORY_SDDL: &str = "O:SYG:SYD:P(A;CI;FA;;;SY)(A;CI;FA;;;BA)(A;CI;0x120089;;;IU)";
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, LocalFree};
        use windows_sys::Win32::Security::Authorization::SE_FILE_OBJECT;
        use windows_sys::Win32::Security::Authorization::{
            ConvertStringSecurityDescriptorToSecurityDescriptorW, SetNamedSecurityInfoW,
        };
        use windows_sys::Win32::Security::{
            DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, SECURITY_ATTRIBUTES,
        };
        use windows_sys::Win32::Storage::FileSystem::CreateDirectoryW;
        let Some(path_text) = path.to_str() else {
            return Err(io::Error::other("transfer cache path is not UTF-16-safe"));
        };
        let mut wide = wide_string(path_text);
        let sddl = wide_string(DIRECTORY_SDDL);
        let mut descriptor = std::ptr::null_mut();
        // SAFETY: terminated SDDL string; revision 1; owned wrapper frees it.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        struct Owned(*mut std::ffi::c_void);
        impl Drop for Owned {
            fn drop(&mut self) {
                // SAFETY: allocated by the SDDL conversion above.
                unsafe { LocalFree(self.0) };
            }
        }
        let owned = Owned(descriptor);
        let attributes = SECURITY_ATTRIBUTES {
            nLength: u32::try_from(std::mem::size_of::<SECURITY_ATTRIBUTES>())
                .map_err(|_| io::Error::other("security attributes size overflow"))?,
            lpSecurityDescriptor: descriptor.cast(),
            bInheritHandle: 0,
        };
        // SAFETY: name and descriptor are NUL-terminated/live for the call.
        if unsafe { CreateDirectoryW(wide.as_ptr(), &attributes) } == 0 {
            // ERROR_ALREADY_EXISTS keeps the old DACL; SetNamedSecurityInfoW
            // below re-applies this build's descriptor either way.
            let code = unsafe { GetLastError() };
            if code != ERROR_ALREADY_EXISTS {
                return Err(io::Error::from_raw_os_error(code as i32));
            }
        }
        // Re-apply the protected DACL so the ACE set always matches this
        // build, even after user-session changes or a service upgrade.
        // SAFETY: NUL-terminated path and live self-relative descriptor.
        if unsafe {
            SetNamedSecurityInfoW(
                wide.as_mut_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                descriptor.cast(),
                std::ptr::null(),
            )
        } != 0
        {
            return Err(io::Error::other("transfer cache DACL update failed"));
        }
        drop(owned);
    }
    Ok(())
}

fn wide_string(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
/// Service-owned transfer cache root, fixed beneath the protected service data
/// directory. Per-transfer directories live beneath it and are cleaned by the
/// bridge on every terminal state.
pub fn set_cache_root(root: std::path::PathBuf) {
    let _ = CACHE_ROOT.set(root);
}

fn cache_root() -> std::path::PathBuf {
    CACHE_ROOT
        .get()
        .cloned()
        .unwrap_or_else(|| std::env::temp_dir().join("deskfuse-service-transfer"))
}

/// One bridge per desktop session; every snapshot replaces it. The global sink
/// survives session resets so worker threads can still report cancellation.
#[must_use]
pub fn session_bridge() -> FileBridge {
    FileBridge::with_default_store(cache_root(), Arc::new(global_sink().clone()))
}

/// Activates frame delivery and starts the single consumer task. Called once
/// per service process after the runtime exists.
pub fn start_delivery(runtime: RuntimeHandle) {
    global_sink().attach_runtime(runtime.clone());
    if let Some(consumer) = CONSUMER.lock().ok().and_then(|mut guard| guard.take()) {
        spawn_consumer(runtime, consumer);
    }
}
