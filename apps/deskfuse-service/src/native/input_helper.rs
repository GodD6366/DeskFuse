use super::security;
use crate::{
    desktop_protocol::{self as ipc, InputDelivery, InputState},
    protocol,
};
use deskfuse_core::{
    DeviceId, InputSessionId, InputShareRole, InputSharingSettings, InputSharingSnapshot,
    SharedInputEvent,
};
use deskfuse_input::{
    Point, Vector2,
    layout::layout_displays,
    sharing::{HotkeyBindings, HotkeyChord, SharingAction, SharingController, SharingDisplay},
};
use deskfuse_platform::{CaptureEvent, SharedInputCapture, SharedInputInjector};
use deskfuse_platform_windows::{
    WindowsSharingCapture, WindowsSharingInjector,
    capture::{CaptureFault, CapturePoll, WindowsInputCapture},
    display_topology,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    os::windows::io::AsRawHandle,
    time::{Duration, Instant},
};
use tokio::time::{sleep, timeout};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
const INPUT_LEASE: Duration = Duration::from_millis(1_500);
/// One controller IPC request per drained capture batch.
const MAX_BATCH_EVENTS: usize = 256;
const LEASE_TTL: Duration = Duration::from_millis(1_500);
/// Display topology is probed on this interval; a change restarts the helper
/// so the supervisor relaunches it with a fresh `RegisterInput` snapshot.
const TOPOLOGY_PROBE_INTERVAL: Duration = Duration::from_secs(2);

fn input_desktop() -> ipc::InputDesktop {
    match deskfuse_platform_windows::session::session_capabilities().desktop {
        deskfuse_platform::DesktopState::UserDesktop => ipc::InputDesktop::Default,
        deskfuse_platform::DesktopState::Locked => ipc::InputDesktop::Locked,
        deskfuse_platform::DesktopState::LoginOrSecureDesktop
        | deskfuse_platform::DesktopState::InactiveSession
        | deskfuse_platform::DesktopState::RemoteSession => ipc::InputDesktop::Protected,
        deskfuse_platform::DesktopState::Unavailable => ipc::InputDesktop::Unavailable,
    }
}

/// Run the fixed UIAccess input helper. It owns no identity, trust or network
/// runtime and accepts no path, shell or installation command. UIAccess lets the
/// signed helper cross UIPI for elevated applications; secure Winlogon desktops
/// remain session-owned by the service and are never entered by this process. It
/// injects remote input (receiver) and captures local input (controller); the
/// service runtime stays the sole authority for both directions.
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

/// A runtime-granted control session, mirroring the runtime lease locally.
#[derive(Clone)]
struct Grant {
    session: InputSessionId,
    expires: Instant,
}
impl Grant {
    fn remaining(&self) -> Duration {
        self.expires.saturating_duration_since(Instant::now())
    }
    fn expired(&self) -> bool {
        Instant::now() >= self.expires
    }
}

/// Controller-side state, mirroring the macOS input coordinator. All edge
/// math runs locally through the cross-platform `SharingController`; only the
/// session protocol crosses the service IPC.
struct Controller {
    local: Option<DeviceId>,
    policy: SharingController,
    capture: Box<dyn SharedInputCapture>,
    capture_started: bool,
    anchor: (f64, f64),
    grants: BTreeMap<InputSessionId, Grant>,
    retired: BTreeSet<InputSessionId>,
    outbound: Option<Grant>,
    last_revision: u64,
    configured: Option<InputSharingSettings>,
    layout: Vec<SharingDisplay>,
    /// (peer, granted control session, ready) rows mirrored from the state.
    links: Vec<(DeviceId, Option<InputSessionId>, bool)>,
    pending: Vec<SharedInputEvent>,
    pending_relative: Option<(f64, f64)>,
    pending_target: Option<(InputSessionId, String)>,
    batch: u64,
    started: Instant,
}

impl Controller {
    fn new() -> Self {
        Self {
            local: None,
            policy: SharingController::new(DeviceId::from_bytes([0; 32])),
            capture: Box::new(WindowsSharingCapture::new()),
            capture_started: false,
            anchor: (0.0, 0.0),
            grants: BTreeMap::new(),
            retired: BTreeSet::new(),
            outbound: None,
            last_revision: 0,
            configured: None,
            layout: Vec::new(),
            links: Vec::new(),
            pending: Vec::new(),
            pending_relative: None,
            pending_target: None,
            batch: 0,
            started: Instant::now(),
        }
    }

    fn restore_local(&mut self) {
        self.outbound = None;
        self.policy.reclaim();
        if self.capture_started {
            let _ = self.capture.exit_remote(Some(self.anchor));
        }
    }
}

async fn run(parent: ParentProcess) -> io::Result<()> {
    let topology = display_topology().map_err(io::Error::other)?;
    let injector = WindowsSharingInjector::new().map_err(io::Error::other)?;
    let emergency_capture = WindowsInputCapture::start(2_048).map_err(io::Error::other)?;
    let mut controller = Controller::new();
    let mut request_id = 0_u64;
    let mut cursor = 0_u64;
    let mut current_session = None;
    let mut registered = false;
    let mut session_epoch = 0_u64;
    let mut failures = 0_u8;
    let mut topology_probe = Instant::now();
    loop {
        if !parent.alive()? {
            let _ = injector.release_all_nowait();
            stop_controller(&mut controller);
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
                Ok(ipc::Reply::InputRegistered {
                    session_epoch: epoch,
                }) => {
                    registered = true;
                    session_epoch = epoch;
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
        let stages = deskfuse_core::take_pending_stages();
        let reply = match exchange(
            &mut request_id,
            ipc::Command::PollInput {
                session_epoch,
                after_sequence: cursor,
                desktop: input_desktop(),
                stages,
            },
        )
        .await
        {
            Ok(reply) => reply,
            Err(_) => {
                deskfuse_core::add_pending_stages(stages);
                registered = false;
                current_session = None;
                let _ = injector.release_all_nowait();
                stop_controller(&mut controller);
                sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let ipc::Reply::InputPoll {
            deliveries,
            sequence,
            state,
        } = reply
        else {
            // 服务没收下这批计数。加回去，等下一次成功的轮询。
            deskfuse_core::add_pending_stages(stages);
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
                InputDelivery::ControlGranted {
                    session_id,
                    lease_ms,
                    ..
                } => {
                    let session = parse_session(&session_id)?;
                    controller.retired.remove(&session);
                    controller.grants.insert(
                        session,
                        Grant {
                            session,
                            expires: Instant::now()
                                + Duration::from_millis(lease_ms.clamp(1, 60_000)),
                        },
                    );
                    Ok(())
                }
                InputDelivery::ControlDenied { .. } => {
                    // A denial is peer state, not a helper fault: keep the
                    // helper alive so the user can retry after the peer
                    // re-enables input sharing.
                    controller.grants.clear();
                    controller.restore_local();
                    // The bounded reason is already mirrored by the service in
                    // the snapshot's activation state; nothing further to do.
                    Ok(())
                }
                InputDelivery::ControlReleased {
                    session_id, reason, ..
                } => {
                    if let Ok(session) = parse_session(&session_id) {
                        if !reason.contains("left display") {
                            controller.retired.insert(session);
                        }
                        if controller
                            .outbound
                            .as_ref()
                            .is_some_and(|grant| grant.session == session)
                        {
                            controller.restore_local();
                        }
                    } else {
                        // A wildcard release clears the whole controller state.
                        controller.restore_local();
                    }
                    Ok(())
                }
            };
            if let Err(error) = result {
                let _ = injector.release_all_nowait();
                let _ = exchange(
                    &mut request_id,
                    ipc::Command::InputFault {
                        message: error.to_string().chars().take(256).collect(),
                    },
                )
                .await;
                return Err(error);
            }
        }
        cursor = sequence;

        // Controller maintenance from the latest state view.
        match maintain_controller(&mut controller, &state, &topology) {
            Ok(Some(released)) => {
                let _ = exchange(
                    &mut request_id,
                    ipc::Command::ReleaseInputControl {
                        session_id: released.to_string(),
                    },
                )
                .await;
            }
            Ok(None) => {}
            Err(error) => {
                let _ = injector.release_all_nowait();
                return report_failure(&mut request_id, &error).await;
            }
        }

        // Drain captured local input while this desktop is a controller.
        if controller.capture_started
            && let Some(local) = controller.local
        {
            let mut actions = Vec::new();
            for _ in 0..MAX_BATCH_EVENTS {
                match controller.capture.next_event() {
                    Ok(Some(CaptureEvent::Input(sample))) => {
                        if !controller.policy.is_remote() {
                            controller.anchor = (sample.global_x, sample.global_y);
                        }
                        let global = Point::new(sample.global_x, sample.global_y);
                        let now_ms = controller.started.elapsed().as_millis() as u64;
                        let (has_raw, raw_dx, raw_dy) =
                            (sample.has_raw, sample.raw_dx, sample.raw_dy);
                        let mut routed = controller.policy.handle_input(
                            sample.event,
                            global,
                            Vector2::new(sample.delta_x, sample.delta_y),
                            now_ms,
                        );
                        attach_captured_relative(&mut routed, has_raw, raw_dx, raw_dy);
                        deskfuse_core::record_input_stage(
                            deskfuse_core::InputStage::CaptureQueue,
                            u64::try_from(sample.captured_at.elapsed().as_micros())
                                .unwrap_or(u64::MAX),
                        );
                        actions.extend(routed);
                    }
                    Ok(None) => break,
                    Ok(Some(_)) | Err(_) => {
                        let _ = injector.release_all_nowait();
                        return report_failure(&mut request_id, "输入捕获中断，已恢复本机输入")
                            .await;
                    }
                }
            }
            if let Err(error) =
                apply_actions(&mut controller, &mut request_id, actions, local).await
            {
                let _ = injector.release_all_nowait();
                return report_failure(&mut request_id, &error.to_string()).await;
            }
            // Flush the routed batch collected during this iteration.
            if controller.outbound.is_some() {
                flush_pending(&mut controller, &mut request_id).await;
            } else {
                controller.pending.clear();
                controller.pending_target = None;
            }
        }

        loop {
            match emergency_capture.poll() {
                CapturePoll::Event(_) => {}
                CapturePoll::Idle => break,
                CapturePoll::Fault(CaptureFault::EmergencyReclaim) => {
                    let _ = injector.release_all_nowait();
                    if let Some(outbound) = controller.outbound.take() {
                        let _ = exchange(
                            &mut request_id,
                            ipc::Command::ReleaseInputControl {
                                session_id: outbound.session.to_string(),
                            },
                        )
                        .await;
                    }
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
        if topology_probe.elapsed() >= TOPOLOGY_PROBE_INTERVAL {
            topology_probe = Instant::now();
            // Monitor hot-plug and DPI changes: coordinate mapping reads live
            // native geometry, but the registered topology and per-batch target
            // lookup still use the startup snapshot, so exit and let the
            // supervisor relaunch with a fresh registration.
            if let Ok(current) = display_topology()
                && current != topology
            {
                let _ = exchange(
                    &mut request_id,
                    ipc::Command::InputFault {
                        message: "display topology changed; input helper restarting".into(),
                    },
                )
                .await;
                let _ = injector.release_all_nowait();
                stop_controller(&mut controller);
                return Err(io::Error::other(
                    "display topology changed; helper restarting",
                ));
            }
        }
        sleep(Duration::from_millis(8)).await;
    }
}

/// Rebuilds layout, hotkeys and the capture lifecycle from the state view.
/// Returns a control session the caller must release when the configuration
/// changed while this desktop was controlling, mirroring the coordinator's
/// conservative reclaim (never keep a remote session across a layout change).
fn maintain_controller(
    controller: &mut Controller,
    state: &InputState,
    topology: &deskfuse_core::DisplayTopology,
) -> Result<Option<InputSessionId>, String> {
    let Some(settings) = state.settings.clone() else {
        return Ok(None);
    };
    if let Ok(local) = state.local_device_id.parse::<DeviceId>()
        && controller.local.is_none()
    {
        controller.local = Some(local);
        controller.policy = SharingController::new(local);
    }
    let Some(local) = controller.local else {
        return Ok(None);
    };
    let controller_enabled = settings.enabled && settings.role == InputShareRole::Controller;
    let mut snapshot = InputSharingSnapshot {
        settings: settings.clone(),
        local_topology: Some(topology.clone()),
        ..InputSharingSnapshot::default()
    };
    snapshot.remote_topologies = state
        .remote_topologies
        .iter()
        .filter_map(|view| {
            view.peer_id
                .parse::<DeviceId>()
                .ok()
                .map(|peer| (peer, view.topology.clone()))
        })
        .collect();
    snapshot.links = state
        .links
        .iter()
        .filter_map(|link| {
            let peer = link.peer_id.parse::<DeviceId>().ok()?;
            Some(deskfuse_core::InputPeerStatus {
                peer_id: peer,
                ready: link.ready,
                control_session: link
                    .control_session
                    .as_deref()
                    .and_then(|id| id.parse::<uuid::Uuid>().ok())
                    .map(InputSessionId),
                lease_revision: link.lease_revision,
                context: None,
                reason: None,
                motion_mode: link.motion_mode.clone(),
                motion_fallback_reason: link.motion_fallback_reason.clone(),
            })
        })
        .collect();
    controller.links = state
        .links
        .iter()
        .filter_map(|link| {
            let peer = link.peer_id.parse::<DeviceId>().ok()?;
            let session = link
                .control_session
                .as_deref()
                .and_then(|id| id.parse::<uuid::Uuid>().ok())
                .map(InputSessionId);
            Some((peer, session, link.ready))
        })
        .collect();
    let mut release_on_change = None;
    if controller.configured.as_ref() != Some(&settings) {
        if let Some(outbound) = controller.outbound.take() {
            controller.policy.reclaim();
            let _ = controller.capture.exit_remote(Some(controller.anchor));
            controller.retired.insert(outbound.session);
            release_on_change = Some(outbound.session);
        }
        let layout = layout_displays(&snapshot, local, true);
        if !layout.is_empty() {
            controller.policy.set_layout(layout.clone())?;
        }
        controller.layout = layout;
        let bindings = hotkey_bindings(&settings);
        controller.policy.set_mode(settings.switch_mode);
        controller.policy.set_hotkeys(bindings.clone());
        controller
            .policy
            .set_disabled_corners(settings.disabled_corners, f64::from(settings.corner_size));
        controller.capture.set_hotkeys(bindings);
        controller
            .capture
            .set_controller_hotkeys_enabled(controller_enabled);
        controller.configured = Some(settings);
    }
    if controller_enabled && !controller.capture_started {
        controller
            .capture
            .start()
            .map_err(|error| format!("无法启动本机输入捕获：{error}"))?;
        controller.capture_started = true;
    } else if !controller_enabled && controller.capture_started {
        stop_controller(controller);
    }
    // Renew the outbound lease from the runtime's link revision.
    let outbound_session = controller.outbound.as_ref().map(|grant| grant.session);
    if let Some(session) = outbound_session
        && let Some(link) = state
            .links
            .iter()
            .find(|link| link.control_session.as_deref() == Some(session.to_string().as_str()))
    {
        if link.ready {
            if link.lease_revision != controller.last_revision {
                controller.last_revision = link.lease_revision;
                controller.outbound = controller.outbound.take().map(|mut grant| {
                    grant.expires = Instant::now() + LEASE_TTL;
                    grant
                });
                if !controller.capture.renew_deadline(LEASE_TTL) {
                    controller.restore_local();
                }
            }
        } else if controller.outbound.as_ref().is_some_and(Grant::expired) {
            controller.restore_local();
        }
    } else if controller.outbound.as_ref().is_some_and(Grant::expired) {
        controller.restore_local();
    }
    // Drop grants the runtime has already superseded.
    controller.grants.retain(|_, grant| !grant.expired());
    Ok(release_on_change)
}

/// 把设备计数挂到本样本的第一条移动上。超限时保留原来的累计，不把像素差当成设备计数。
fn attach_captured_relative(actions: &mut [SharingAction], has_raw: bool, dx: f64, dy: f64) {
    if !has_raw {
        return;
    }
    if !deskfuse_core::relative_delta_ok(dx) || !deskfuse_core::relative_delta_ok(dy) {
        return;
    }
    let Some(SharingAction::Route { relative, .. }) = actions.iter_mut().find(|action| {
        matches!(
            action,
            SharingAction::Route {
                event: SharedInputEvent::Motion { .. },
                ..
            }
        )
    }) else {
        return;
    };
    *relative = match *relative {
        None => Some((dx, dy)),
        Some((left, right)) => {
            let (sum_x, sum_y) = (left + dx, right + dy);
            if deskfuse_core::relative_delta_ok(sum_x) && deskfuse_core::relative_delta_ok(sum_y) {
                Some((sum_x, sum_y))
            } else {
                Some((left, right))
            }
        }
    };
}

/// 累加同一批次的相对位移。和超限时返回错误，调用方先发出当前批次。
fn merge_relative(current: Option<(f64, f64)>, dx: f64, dy: f64) -> Result<Option<(f64, f64)>, ()> {
    if !deskfuse_core::relative_delta_ok(dx) || !deskfuse_core::relative_delta_ok(dy) {
        return Ok(current);
    }
    match current {
        None => Ok(Some((dx, dy))),
        Some((left, right)) => {
            let (sum_x, sum_y) = (left + dx, right + dy);
            if deskfuse_core::relative_delta_ok(sum_x) && deskfuse_core::relative_delta_ok(sum_y) {
                Ok(Some((sum_x, sum_y)))
            } else {
                Err(())
            }
        }
    }
}

/// Sends the pending routed batch to the service. Batch identifiers are
/// per-session monotonic; the service drops replays and the runtime validates
/// every event against the live lease.
async fn flush_pending(controller: &mut Controller, request_id: &mut u64) {
    let Some((session, target)) = controller.pending_target.clone() else {
        return;
    };
    if controller.pending.is_empty() {
        controller.pending_target = None;
        controller.pending_relative = None;
        return;
    }
    controller.batch = controller.batch.wrapping_add(1).max(1);
    let batch = controller.batch;
    let events = std::mem::take(&mut controller.pending);
    let relative = controller.pending_relative.take();
    controller.pending_target = None;
    let reply = exchange(
        request_id,
        ipc::Command::SendInputEvents {
            session_id: session.to_string(),
            target_display: target,
            batch,
            events,
            relative,
        },
    )
    .await;
    if !matches!(reply, Ok(ipc::Reply::Queued { .. })) {
        controller.restore_local();
    }
}

/// Applies crossing/routing actions exactly like the macOS coordinator, with
/// the service IPC replacing the in-process runtime channel.
async fn apply_actions(
    controller: &mut Controller,
    request_id: &mut u64,
    actions: Vec<SharingAction>,
    local: DeviceId,
) -> Result<(), io::Error> {
    for action in actions {
        match action {
            SharingAction::Crossed { from, to, position } => {
                let changing_device = from.device_id != to.device_id;
                if changing_device && let Some(outbound) = controller.outbound.take() {
                    let _ = exchange(
                        request_id,
                        ipc::Command::LeaveInputControl {
                            session_id: outbound.session.to_string(),
                        },
                    )
                    .await;
                }
                if to.device_id == local {
                    let restore = controller
                        .layout
                        .iter()
                        .find(|display| display.key == to)
                        .map(|display| {
                            (
                                display.display.frame_x + position.x,
                                display.display.frame_y + position.y,
                            )
                        });
                    if changing_device {
                        controller.outbound = None;
                        // No policy reclaim here: the macOS coordinator lets
                        // the next local sample re-anchor the active display,
                        // preserving dwell/double-tap attempt state.
                        let _ = controller.capture.exit_remote(restore);
                    }
                } else {
                    // Entering a remote display. The link names the granted
                    // session; a live runtime grant must still arm the capture.
                    let Some(session) = controller
                        .links
                        .iter()
                        .find(|(peer, session, ready)| {
                            *peer == to.device_id && *ready && session.is_some()
                        })
                        .and_then(|(_, session, _)| *session)
                    else {
                        controller.policy.reclaim();
                        continue;
                    };
                    let Some(grant) = controller
                        .grants
                        .get(&session)
                        .filter(|grant| !grant.expired())
                        .cloned()
                    else {
                        controller.policy.reclaim();
                        continue;
                    };
                    if controller
                        .outbound
                        .as_ref()
                        .is_none_or(|outbound| outbound.session != session)
                    {
                        if controller.retired.contains(&session)
                            || !controller.capture.arm_session(session, grant.remaining())
                        {
                            controller.retired.insert(session);
                            let _ = exchange(
                                request_id,
                                ipc::Command::ReleaseInputControl {
                                    session_id: session.to_string(),
                                },
                            )
                            .await;
                            controller.policy.reclaim();
                            continue;
                        }
                        controller.outbound = Some(grant);
                    }
                    let entered = exchange(
                        request_id,
                        ipc::Command::EnterInputControl {
                            session_id: session.to_string(),
                            target_display: to.display_id.0.clone(),
                            x: position.x,
                            y: position.y,
                        },
                    )
                    .await;
                    match entered {
                        Ok(ipc::Reply::Queued { .. }) => {
                            if changing_device
                                && let Err(error) = controller
                                    .capture
                                    .enter_remote(controller.anchor.0, controller.anchor.1)
                            {
                                // The runtime already entered remote control; without
                                // local suppression the same input would drive both
                                // machines. Release the session and reclaim, like the
                                // macOS coordinator, instead of dropping the error.
                                if let Some(outbound) = controller.outbound.take() {
                                    let _ = exchange(
                                        request_id,
                                        ipc::Command::ReleaseInputControl {
                                            session_id: outbound.session.to_string(),
                                        },
                                    )
                                    .await;
                                }
                                controller.restore_local();
                                let _ = exchange(request_id, ipc::Command::ReclaimInput).await;
                                return Err(io::Error::other(format!(
                                    "进入远端输入控制失败：{error}"
                                )));
                            }
                        }
                        Ok(_) => {}
                        Err(_) => {
                            controller.outbound = None;
                            controller.policy.reclaim();
                            return Err(io::Error::other("进入远端输入控制失败"));
                        }
                    }
                }
            }
            SharingAction::Route {
                target,
                event,
                relative,
            } => {
                let Some(outbound) = controller.outbound.clone() else {
                    continue;
                };
                if outbound.expired() {
                    controller.restore_local();
                    continue;
                }
                let key = (outbound.session, target.display_id.0.clone());
                if controller
                    .pending_target
                    .as_ref()
                    .is_some_and(|existing| *existing != key)
                {
                    flush_pending(controller, request_id).await;
                }
                if let Some((dx, dy)) = relative {
                    match merge_relative(controller.pending_relative, dx, dy) {
                        Ok(next) => controller.pending_relative = next,
                        Err(()) => {
                            if controller.pending_relative.is_some() {
                                flush_pending(controller, request_id).await;
                            }
                            controller.pending_relative = if deskfuse_core::relative_delta_ok(dx)
                                && deskfuse_core::relative_delta_ok(dy)
                            {
                                Some((dx, dy))
                            } else {
                                None
                            };
                        }
                    }
                }
                controller.pending_target = Some(key);
                controller.pending.push(event);
            }
            SharingAction::Reclaim => {
                if let Some(outbound) = controller.outbound.take() {
                    let _ = exchange(
                        request_id,
                        ipc::Command::ReleaseInputControl {
                            session_id: outbound.session.to_string(),
                        },
                    )
                    .await;
                }
                controller.restore_local();
                let _ = exchange(request_id, ipc::Command::ReclaimInput).await;
            }
            SharingAction::LockChanged(_) => {}
        }
    }
    Ok(())
}

pub(crate) fn hotkey_bindings(settings: &InputSharingSettings) -> HotkeyBindings {
    let shortcuts = &settings.shortcuts;
    let chord = |shortcut: &deskfuse_core::InputShortcut| HotkeyChord {
        code: shortcut.key_code,
        modifiers: shortcut.modifiers,
    };
    HotkeyBindings {
        directional: [
            chord(&shortcuts.left),
            chord(&shortcuts.right),
            chord(&shortcuts.up),
            chord(&shortcuts.down),
        ],
        lock: chord(&shortcuts.lock),
        emergency: chord(&shortcuts.emergency),
    }
}

fn stop_controller(controller: &mut Controller) {
    controller.outbound = None;
    controller.grants.clear();
    controller.policy.reclaim();
    if controller.capture_started {
        let _ = controller.capture.exit_remote(None);
        let _ = controller.capture.stop();
        controller.capture_started = false;
    }
}

pub(crate) fn parse_session(value: &str) -> io::Result<InputSessionId> {
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
