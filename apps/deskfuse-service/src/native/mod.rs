//! Windows SCM entry, authenticated session-bound IPC and service-owned networking.
#![allow(unsafe_code)]
mod clipboard_codec;
mod clipboard_sync;
pub(crate) mod desktop;
pub(super) mod file_transfer;
pub(crate) mod input_helper;
mod input_security;
pub(super) mod lan_web;
mod network;
mod remote_app;
mod security;
mod session_agent;
mod wifi;
mod workflow;
use crate::{
    policy::{SessionPolicy, same_session},
    protocol::{self, Command, Reply, Request, Response, Status},
};
use futures::StreamExt;
use network::Network;
use std::{
    io,
    os::windows::io::AsRawHandle,
    ptr,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    net::windows::named_pipe::NamedPipeServer,
    task::JoinSet,
    time::{sleep, timeout},
};
use windows_sys::Win32::{
    Foundation::{ERROR_CALL_NOT_IMPLEMENTED, ERROR_SERVICE_SPECIFIC_ERROR},
    System::Services::{
        RegisterServiceCtrlHandlerExW, SERVICE_ACCEPT_SESSIONCHANGE, SERVICE_ACCEPT_SHUTDOWN,
        SERVICE_ACCEPT_STOP, SERVICE_CONTROL_INTERROGATE, SERVICE_CONTROL_SESSIONCHANGE,
        SERVICE_CONTROL_SHUTDOWN, SERVICE_CONTROL_STOP, SERVICE_RUNNING, SERVICE_START_PENDING,
        SERVICE_STATUS, SERVICE_STATUS_HANDLE, SERVICE_STOP_PENDING, SERVICE_STOPPED,
        SERVICE_TABLE_ENTRYW, SERVICE_WIN32_OWN_PROCESS, SetServiceStatus,
        StartServiceCtrlDispatcherW,
    },
};

/// Concurrent authenticated client connections across both listeners. The
/// helper's persistent desktop connection holds one for its whole lifetime.
const MAX_CLIENTS: usize = 16;

pub(super) struct Control {
    stopping: AtomicBool,
    epoch: AtomicU64,
    desktop: Mutex<desktop::State>,
    /// Cached session-gate relaxation; refreshed from the durable store per
    /// desktop request and written through on change. Strict until then.
    allow_locked_session: AtomicBool,
    allow_remote_session: AtomicBool,
    /// Input-delivery revision. The helper's push task waits on it, so a
    /// delivery reaches the helper without waiting for its next poll.
    input_push: tokio::sync::watch::Sender<u64>,
}
impl Default for Control {
    fn default() -> Self {
        Self {
            stopping: AtomicBool::default(),
            epoch: AtomicU64::default(),
            desktop: Mutex::default(),
            allow_locked_session: AtomicBool::default(),
            allow_remote_session: AtomicBool::default(),
            input_push: tokio::sync::watch::channel(0).0,
        }
    }
}
impl Control {
    /// Wake the helper's push task. Cheap and lossless: the revision is stored,
    /// so a signal raised before the task waits is still observed.
    pub(super) fn signal_input_push(&self) {
        self.input_push
            .send_modify(|revision| *revision = revision.wrapping_add(1));
    }
    pub(super) fn session_policy(&self) -> SessionPolicy {
        SessionPolicy {
            allow_locked: self.allow_locked_session.load(Ordering::Relaxed),
            allow_remote: self.allow_remote_session.load(Ordering::Relaxed),
        }
    }
    pub(super) fn set_session_policy(&self, policy: SessionPolicy) {
        self.allow_locked_session
            .store(policy.allow_locked, Ordering::Relaxed);
        self.allow_remote_session
            .store(policy.allow_remote, Ordering::Relaxed);
    }
    /// Reload the cached policy from the durable store; keep the cache on error.
    pub(super) fn refresh_session_policy(&self, network: &Network) {
        if let Ok(Some(json)) = network.store.session_policy_json()
            && let Ok(policy) = serde_json::from_str::<SessionPolicy>(&json)
        {
            self.set_session_policy(policy);
        }
    }
}
static CONTROL: OnceLock<Arc<Control>> = OnceLock::new();

/// Process-wide handle for async desktop-state updates (file publication
/// results). Returns `None` before SCM dispatch started.
pub(super) fn desktop_control() -> Option<&'static Arc<Control>> {
    CONTROL.get()
}
pub use input_helper::run_input_helper;

/// Enter SCM dispatch; running this directly never silently starts a console service.
/// # Errors
/// Returns SCM connection/registration errors; no automatic privilege escalation.
pub fn run_dispatcher() -> io::Result<()> {
    CONTROL
        .set(Arc::new(Control::default()))
        .map_err(|_| io::Error::other("service already initialized"))?;
    let mut name = security::wide(protocol::SERVICE_NAME);
    let table = [
        SERVICE_TABLE_ENTRYW {
            lpServiceName: name.as_mut_ptr(),
            lpServiceProc: Some(service_main),
        },
        SERVICE_TABLE_ENTRYW {
            lpServiceName: ptr::null_mut(),
            lpServiceProc: None,
        },
    ];
    // SAFETY: terminated two-entry table and string live throughout the blocking dispatcher.
    if unsafe { StartServiceCtrlDispatcherW(table.as_ptr()) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
unsafe extern "system" fn handler(
    control: u32,
    _event: u32,
    _data: *mut std::ffi::c_void,
    _context: *mut std::ffi::c_void,
) -> u32 {
    let Some(state) = CONTROL.get() else {
        return ERROR_CALL_NOT_IMPLEMENTED;
    };
    match control {
        SERVICE_CONTROL_STOP | SERVICE_CONTROL_SHUTDOWN => {
            state.stopping.store(true, Ordering::Release);
            state.epoch.fetch_add(1, Ordering::AcqRel);
            if let Ok(mut desktop) = state.desktop.lock() {
                desktop.clear();
            }
            0
        }
        SERVICE_CONTROL_SESSIONCHANGE => {
            state.epoch.fetch_add(1, Ordering::AcqRel);
            if let Ok(mut desktop) = state.desktop.lock() {
                desktop.clear();
            }
            0
        }
        SERVICE_CONTROL_INTERROGATE => 0,
        _ => ERROR_CALL_NOT_IMPLEMENTED,
    }
}
struct StatusHandle(SERVICE_STATUS_HANDLE);
impl StatusHandle {
    fn report(&self, state: u32, checkpoint: u32, failure_code: u32) -> io::Result<()> {
        let status = SERVICE_STATUS {
            dwServiceType: SERVICE_WIN32_OWN_PROCESS,
            dwCurrentState: state,
            dwControlsAccepted: if state == SERVICE_RUNNING {
                SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SHUTDOWN | SERVICE_ACCEPT_SESSIONCHANGE
            } else {
                0
            },
            dwWin32ExitCode: if failure_code != 0 {
                ERROR_SERVICE_SPECIFIC_ERROR
            } else {
                0
            },
            dwServiceSpecificExitCode: failure_code,
            dwCheckPoint: checkpoint,
            dwWaitHint: if matches!(state, SERVICE_START_PENDING | SERVICE_STOP_PENDING) {
                15_000
            } else {
                0
            },
        };
        // SAFETY: registration returned this status handle; status struct is valid for the call.
        if unsafe { SetServiceStatus(self.0, &raw const status) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}
unsafe extern "system" fn service_main(_argc: u32, _argv: *mut windows_sys::core::PWSTR) {
    // No FFI unwind: faults report a stopped service; no user content in logs.
    let name = security::wide(protocol::SERVICE_NAME);
    // SAFETY: callback ABI matches SCM; no custom context; name is terminated.
    let handle =
        unsafe { RegisterServiceCtrlHandlerExW(name.as_ptr(), Some(handler), ptr::null()) };
    if handle.is_null() {
        return;
    }
    let status = StatusHandle(handle);
    let result = std::panic::catch_unwind(|| {
        status.report(SERVICE_START_PENDING, 1, 0)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(serve(&status))
    });
    let code = match result {
        Ok(Ok(())) => 0,
        Ok(Err(error)) => error
            .raw_os_error()
            .and_then(|code| u32::try_from(code).ok())
            .unwrap_or(1),
        Err(_) => 2,
    };
    // SCM stores a stage/system code, never a user-content-bearing error string.
    let _ = status.report(SERVICE_STOPPED, 0, code);
}
async fn serve(status: &StatusHandle) -> io::Result<()> {
    let control = CONTROL
        .get()
        .ok_or_else(|| io::Error::other("missing service control"))?
        .clone();
    // Claim the namespace before touching durable state; a squatter or a second
    // service fails closed. Each pipe then keeps its own accept task, so pipe
    // connects are never starved by work in the service loop and a client always
    // finds a listening instance (ERROR_PIPE_BUSY otherwise).
    let mut listener = security::create_pipe(protocol::PIPE_NAME, true)?;
    let mut desktop_listener = security::create_pipe(crate::desktop_protocol::PIPE_NAME, true)?;
    let start = timeout(Duration::from_mins(1), Network::start());
    tokio::pin!(start);
    let mut checkpoint = 1;
    let network = Arc::new(loop {
        tokio::select! {
            result = &mut start => break result.map_err(|_| io::Error::from_raw_os_error(27))??,
            () = sleep(Duration::from_secs(2)) => {
                checkpoint += 1;
                status.report(SERVICE_START_PENDING, checkpoint, 0)?;
            }
        }
    });
    let running_report = status.report(SERVICE_RUNNING, 0, 0);
    let mut clients = JoinSet::new();
    // 连接编号。helper 的长连接长期占用一个名额，上限因此留出余量。
    let links = AtomicU64::new(0);
    let mut events = network.runtime.subscribe_events();
    let mut input_events = network.input_runtime.handle.input_event_stream();
    let mut input_sync_tick = tokio::time::interval(Duration::from_secs(1));
    let mut desktop_tick = tokio::time::interval(Duration::from_millis(200));
    let outcome = if let Err(error) = running_report {
        Err(error)
    } else {
        loop {
            if control.stopping.load(Ordering::Acquire) {
                break Ok(());
            }
            tokio::select! {
                connected = listener.connect() => {
                    if let Err(error) = connected { break Err(error); }
                    // 监听实例必须始终保持。池满时若连 accept 一起停掉，管道就没有
                    // 可连接实例，所有客户端——包括正要释放名额的那些——都会拿到
                    // ERROR_PIPE_BUSY。超出名额就立刻关闭这条连接，让调用方按 EOF 重试。
                    let next = match security::create_pipe(protocol::PIPE_NAME, false) {
                        Ok(next) => next,
                        Err(error) => break Err(error),
                    };
                    let connected = std::mem::replace(&mut listener, next);
                    if clients.len() >= MAX_CLIENTS {
                        continue;
                    }
                    let network = network.clone();
                    let control = control.clone();
                    clients.spawn(async move {
                        // Read/write deadline and a bounded client count. Authentication
                        // uses synchronous local Win32 queries (no await while impersonating).
                        let _ = timeout(Duration::from_secs(3), handle_client(connected, &network, &control)).await;
                    });
                }
                connected = desktop_listener.connect() => {
                    if let Err(error) = connected { break Err(error); }
                    let next = match security::create_pipe(crate::desktop_protocol::PIPE_NAME, false) {
                        Ok(next) => next, Err(error) => break Err(error),
                    };
                    let connected = std::mem::replace(&mut desktop_listener, next);
                    if clients.len() >= MAX_CLIENTS {
                        continue;
                    }
                    let network = network.clone();
                    let control = control.clone();
                    // 桌面连接不再按请求计时：helper 的长连接存活整个进程生命周期。
                    // 首帧和空闲上限由 `desktop::handle` 自己约束。
                    let link = links.fetch_add(1, Ordering::AcqRel) + 1;
                    clients.spawn(async move {
                        let _ = desktop::handle(connected, &network, control, link).await;
                    });
                }
                input_event = input_events.next() => {
                    let Some(input_event) = input_event else {
                        break Err(io::Error::other("input event stream closed"));
                    };
                    if let Ok(mut state) = control.desktop.lock() {
                        state.input_event(input_event, &network, &control);
                    }
                    control.signal_input_push();
                }
                event = events.recv() => {
                    if let Ok(mut state) = control.desktop.lock() {
                        match event {
                            Ok(event) => state.event(event, &network, &control),
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => state.lagged(),
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => break Err(io::Error::other("network event stream closed")),
                        }
                    }
                }
                _ = input_sync_tick.tick() => {
                    network.schedule_input_authority();
                }
                _ = clients.join_next(), if !clients.is_empty() => {},
                _ = desktop_tick.tick() => {
                    if let Ok(mut state) = control.desktop.lock() { state.tick(&network, &control); }
                    control.signal_input_push();
                    network.serve_remote_applications();
                },
            }
        }
    };
    // Even a status-report failure must go through the full runtime shutdown.
    let stop_report = status.report(SERVICE_STOP_PENDING, 1, 0);
    control.epoch.fetch_add(1, Ordering::AcqRel);
    drop(listener);
    drop(desktop_listener);
    if let Ok(mut state) = control.desktop.lock() {
        state.clear();
    }
    clients.abort_all();
    while clients.join_next().await.is_some() {}
    // Stop the input-only listener before the ordinary runtime and directory lease.
    if let Ok(result) = timeout(
        Duration::from_secs(10),
        network.input_runtime.handle.shutdown_and_wait(),
    )
    .await
    {
        result.map_err(io::Error::other)?;
    } else {
        let _ = status.report(SERVICE_STOPPED, 0, 30);
        std::process::abort();
    }
    // Stop listeners and runtime before dropping its pinned SQLite directory.
    if let Ok(result) = timeout(Duration::from_secs(10), network.runtime.shutdown_and_wait()).await
    {
        result.map_err(io::Error::other)?;
    } else {
        // A timed-out join owns a still-running blocking task. Do not drop
        // the directory lease while that actor could write, or hang in
        // Tokio runtime Drop. Terminate all service threads together.
        let _ = status.report(SERVICE_STOPPED, 0, 30);
        std::process::abort();
    }
    outcome.and(stop_report)
}
async fn handle_client(
    mut pipe: NamedPipeServer,
    network: &Network,
    control: &Control,
) -> io::Result<()> {
    // Named-pipe impersonation applies to the most recently READ message.
    let request: Request = protocol::read_frame(&mut pipe).await?;
    let epoch = control.epoch.load(Ordering::Acquire);
    control.refresh_session_policy(network);
    let policy = control.session_policy();
    let before = security::authenticate(pipe.as_raw_handle(), policy)?;
    let result = if request.version == protocol::VERSION {
        match request.command {
            Command::Status => {
                let snapshot = network.runtime.snapshot();
                Reply::Status(Status {
                    package_version: env!("CARGO_PKG_VERSION").into(),
                    session_epoch: epoch,
                    client_session_id: before.pipe_session,
                    network_running: snapshot.sequence > 0
                        && network.runtime.subscribe().has_changed().is_ok(),
                    local_device_id: snapshot.local_device_id.to_string(),
                    control_port: network.control_port,
                    peer_count: snapshot.peers.len(),
                    desktop_bridge_available: true,
                    input_helper_available: true,
                    installer_available: false,
                })
            }
        }
    } else {
        Reply::UnsupportedVersion
    };
    let after = security::authenticate(pipe.as_raw_handle(), policy)?;
    if control.stopping.load(Ordering::Acquire)
        || !same_session(
            before,
            after,
            epoch,
            control.epoch.load(Ordering::Acquire),
            policy,
        )
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "session changed",
        ));
    }
    // Metadata only. Future user-content streams require per-message generation
    // routing; this check is not a guarantee that a desktop cannot lock during write.
    protocol::write_frame(
        &mut pipe,
        &Response {
            version: protocol::VERSION,
            request_id: request.request_id,
            result,
        },
    )
    .await
}

/// Query metadata from the genuine SCM-owned service; never starts a fallback runtime.
/// # Errors
/// Service absent, noninteractive/locked session, timeout, protocol/peer mismatch.
pub async fn query_status() -> io::Result<Response> {
    timeout(Duration::from_secs(4), async {
        let mut pipe = security::open_client(protocol::PIPE_NAME)?;
        security::verify_server(pipe.as_raw_handle())?;
        let request = Request {
            version: protocol::VERSION,
            request_id: 1,
            command: Command::Status,
        };
        protocol::write_frame(&mut pipe, &request).await?;
        let response: Response = protocol::read_frame(&mut pipe).await?;
        if response.version != protocol::VERSION || response.request_id != request.request_id {
            return Err(io::Error::other("service protocol mismatch"));
        }
        Ok(response)
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "service IPC timeout"))?
}
