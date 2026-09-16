use super::security;
use crate::{
    desktop_protocol::{self as ipc, InputDelivery},
    protocol,
};
use std::{io, os::windows::io::AsRawHandle, time::Duration};
use tokio::time::{sleep, timeout};
use unidesk_core::InputSessionId;
use unidesk_platform::SharedInputInjector;
use unidesk_platform_windows::{
    WindowsSharingInjector,
    capture::{CaptureFault, CapturePoll, WindowsInputCapture},
    display_topology,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
const INPUT_LEASE: Duration = Duration::from_millis(1_500);

/// Run the fixed ordinary-desktop input helper. It owns no identity, trust or
/// network runtime and accepts no path, shell or installation command.
pub fn run_input_helper(parent_pid: u32) -> io::Result<()> {
    let parent = ParentProcess::open(parent_pid)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(run(parent))
}

struct ParentProcess(HANDLE);
impl ParentProcess {
    fn open(pid: u32) -> io::Result<Self> {
        // SAFETY: fixed process ID and SYNCHRONIZE-only access; no handle inheritance.
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(handle))
    }
    fn alive(&self) -> io::Result<bool> {
        // SAFETY: owned process handle remains valid for this zero-timeout wait.
        match unsafe { WaitForSingleObject(self.0, 0) } {
            WAIT_TIMEOUT => Ok(true),
            WAIT_OBJECT_0 => Ok(false),
            WAIT_FAILED => Err(io::Error::last_os_error()),
            _ => Err(io::Error::other("unexpected parent process wait result")),
        }
    }
}
impl Drop for ParentProcess {
    fn drop(&mut self) {
        // SAFETY: this helper uniquely owns the OpenProcess handle.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

async fn run(parent: ParentProcess) -> io::Result<()> {
    let topology = display_topology().map_err(io::Error::other)?;
    let injector = WindowsSharingInjector::new().map_err(io::Error::other)?;
    let emergency_capture = WindowsInputCapture::start(2_048).map_err(io::Error::other)?;
    let mut request_id = 0_u64;
    let mut cursor = 0_u64;
    let mut current_session = None;
    let mut registered = false;
    let mut failures = 0_u8;
    loop {
        if !parent.alive()? {
            let _ = injector.release_all_nowait();
            return Err(io::Error::other("desktop parent process exited"));
        }
        if !registered {
            match exchange(
                &mut request_id,
                ipc::Command::RegisterInput {
                    topology: topology.clone(),
                },
            )
            .await
            {
                Ok(ipc::Reply::Queued { .. }) => {
                    registered = true;
                    cursor = 0;
                    failures = 0;
                }
                Ok(_) | Err(_) => {
                    failures = failures.saturating_add(1);
                    if failures >= 20 {
                        return Err(io::Error::other("desktop input session unavailable"));
                    }
                    sleep(Duration::from_millis(250)).await;
                    continue;
                }
            }
        }
        let reply = match exchange(
            &mut request_id,
            ipc::Command::PollInput {
                after_sequence: cursor,
            },
        )
        .await
        {
            Ok(reply) => reply,
            Err(_) => {
                registered = false;
                current_session = None;
                let _ = injector.release_all_nowait();
                sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let ipc::Reply::InputPoll {
            deliveries,
            sequence,
            ..
        } = reply
        else {
            registered = false;
            continue;
        };
        for delivery in deliveries {
            let result = match delivery {
                InputDelivery::Lease { session_id, .. } => {
                    let session = parse_session(&session_id)?;
                    if current_session == Some(session) {
                        injector
                            .renew_deadline(INPUT_LEASE)
                            .then_some(())
                            .ok_or_else(|| io::Error::other("input lease expired before renewal"))
                    } else {
                        let _ = injector.release_all_nowait();
                        if injector.arm_session(session, INPUT_LEASE) {
                            current_session = Some(session);
                            Ok(())
                        } else {
                            Err(io::Error::other("input lease rejected"))
                        }
                    }
                }
                InputDelivery::Batch {
                    session_id,
                    target_display,
                    state_epoch,
                    events,
                    ..
                } => {
                    let session = parse_session(&session_id)?;
                    if current_session != Some(session) {
                        let _ = injector.release_all_nowait();
                        if !injector.arm_session(session, INPUT_LEASE) {
                            return report_failure(
                                &mut request_id,
                                "input batch belongs to an expired lease",
                            )
                            .await;
                        }
                        current_session = Some(session);
                    } else if !injector.renew_deadline(INPUT_LEASE) {
                        return report_failure(&mut request_id, "input lease renewal failed").await;
                    }
                    let display = topology
                        .displays
                        .iter()
                        .find(|display| display.id.as_str() == target_display)
                        .ok_or_else(|| io::Error::other("target display is unavailable"))?;
                    injector
                        .enqueue_batch(session, display, state_epoch, &events)
                        .map_err(io::Error::other)
                }
                InputDelivery::Release { .. } => {
                    current_session = None;
                    injector.release_all_nowait().map_err(io::Error::other)
                }
            };
            if let Err(error) = result {
                let _ = injector.release_all_nowait();
                return report_failure(&mut request_id, &error.to_string()).await;
            }
        }
        cursor = sequence;
        loop {
            match emergency_capture.poll() {
                CapturePoll::Event(_) => {}
                CapturePoll::Idle => break,
                CapturePoll::Fault(CaptureFault::EmergencyReclaim) => {
                    let _ = injector.release_all_nowait();
                    let _ = exchange(&mut request_id, ipc::Command::ReclaimInput).await;
                    return Err(io::Error::other(
                        "input reclaimed by Windows emergency chord",
                    ));
                }
                CapturePoll::Fault(_) | CapturePoll::Stopped => {
                    let _ = injector.release_all_nowait();
                    return report_failure(
                        &mut request_id,
                        "Windows emergency input observer stopped",
                    )
                    .await;
                }
            }
        }
        if let Some(error) = injector.take_error() {
            let _ = injector.release_all_nowait();
            return report_failure(&mut request_id, &error.to_string()).await;
        }
        sleep(Duration::from_millis(8)).await;
    }
}

fn parse_session(value: &str) -> io::Result<InputSessionId> {
    value
        .parse::<uuid::Uuid>()
        .map(InputSessionId)
        .map_err(|_| io::Error::other("invalid input session identifier"))
}

async fn report_failure(request_id: &mut u64, message: &str) -> io::Result<()> {
    let _ = exchange(
        request_id,
        ipc::Command::InputFault {
            message: message.chars().take(256).collect(),
        },
    )
    .await;
    Err(io::Error::other("Windows input helper stopped safely"))
}

async fn exchange(request_id: &mut u64, command: ipc::Command) -> io::Result<ipc::Reply> {
    *request_id = (*request_id).saturating_add(1).max(1);
    let id = *request_id;
    timeout(REQUEST_TIMEOUT, async move {
        let mut pipe = security::open_client(ipc::PIPE_NAME)?;
        security::verify_server(pipe.as_raw_handle())?;
        protocol::write_frame_limited(
            &mut pipe,
            &ipc::Request {
                version: ipc::VERSION,
                request_id: id,
                command,
            },
            ipc::MAX_FRAME,
        )
        .await?;
        let response: ipc::Response =
            protocol::read_frame_limited(&mut pipe, ipc::MAX_FRAME).await?;
        if response.version != ipc::VERSION || response.request_id != id {
            return Err(io::Error::other("desktop input protocol mismatch"));
        }
        use tokio::io::AsyncWriteExt;
        pipe.write_all(&[0x06]).await?;
        match response.result {
            ipc::Reply::Rejected { message, .. } => Err(io::Error::other(message)),
            ipc::Reply::UnsupportedVersion => {
                Err(io::Error::other("desktop input protocol unsupported"))
            }
            reply => Ok(reply),
        }
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "desktop input request timed out"))?
}
