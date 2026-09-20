//! Windows SCM entry, authenticated session-bound IPC and service-owned networking.
#![allow(unsafe_code)]
mod clipboard_codec;
mod clipboard_sync;
mod desktop;
mod input_helper;
mod network;
mod remote_app;
mod security;
use crate::{
    policy::same_session,
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

#[derive(Default)]
struct Control {
    stopping: AtomicBool,
    epoch: AtomicU64,
    desktop: Mutex<desktop::State>,
}
static CONTROL: OnceLock<Arc<Control>> = OnceLock::new();
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
    // Claim namespace before touching durable state; squatting/second service fails closed.
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
                connected = listener.connect(), if clients.len() < 8 => {
                    if let Err(error) = connected { break Err(error); }
                    // Keep a listening instance owned at all times, so a client cannot
                    // create a competing namespace between completed connections.
                    let next = match security::create_pipe(protocol::PIPE_NAME, false) {
                        Ok(next) => next,
                        Err(error) => break Err(error),
                    };
                    let connected = std::mem::replace(&mut listener, next);
                    let network = network.clone();
                    let control = control.clone();
                    clients.spawn(async move {
                        // Read/write deadline and at most eight clients. Authentication
                        // uses synchronous local Win32 queries (no await while impersonating).
                        let _ = timeout(Duration::from_secs(3), handle_client(connected, &network, &control)).await;
                    });
                }
                connected = desktop_listener.connect(), if clients.len() < 8 => {
                    if let Err(error) = connected { break Err(error); }
                    let next = match security::create_pipe(crate::desktop_protocol::PIPE_NAME, false) {
                        Ok(next) => next, Err(error) => break Err(error),
                    };
                    let connected = std::mem::replace(&mut desktop_listener, next);
                    let network = network.clone();
                    let control = control.clone();
                    clients.spawn(async move {
                        let _ = timeout(Duration::from_secs(3), desktop::handle(connected, &network, &control)).await;
                    });
                }
                input_event = input_events.next() => {
                    let Some(input_event) = input_event else {
                        break Err(io::Error::other("input event stream closed"));
                    };
                    if let Ok(mut state) = control.desktop.lock() {
                        state.input_event(input_event, &network, &control);
                    }
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
                    if let Err(error) = network.sync_input_authority().await {
                        break Err(error);
                    }
                }
                _ = clients.join_next(), if !clients.is_empty() => {},
                _ = desktop_tick.tick() => {
                    if let Ok(mut state) = control.desktop.lock() { state.tick(&network, &control); }
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
    let before = security::authenticate(pipe.as_raw_handle())?;
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
    let after = security::authenticate(pipe.as_raw_handle())?;
    if control.stopping.load(Ordering::Acquire)
        || !same_session(before, after, epoch, control.epoch.load(Ordering::Acquire))
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
