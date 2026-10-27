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
use tokio::{
    net::windows::named_pipe::NamedPipeClient,
    time::{sleep, timeout},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT},
    Security::{GetTokenInformation, TOKEN_QUERY, TokenUIAccess},
    System::Threading::{
        GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    },
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
const INPUT_LEASE: Duration = Duration::from_millis(1_500);
/// One controller IPC request per drained capture batch.
const MAX_BATCH_EVENTS: usize = 256;
const LEASE_TTL: Duration = Duration::from_millis(1_500);
/// Retry interval after a local capture fault. Local capture only serves the
/// controller direction; injection keeps running meanwhile.
const CAPTURE_RETRY: Duration = Duration::from_secs(3);
/// Display topology is probed on this interval; a change restarts the helper
/// so the supervisor relaunches it with a fresh `RegisterInput` snapshot.
const TOPOLOGY_PROBE_INTERVAL: Duration = Duration::from_secs(2);
/// 连续注册失败的上限。服务端在满载时可能主动关闭超额连接，这是可恢复的
/// 瞬时状态：给足约 15 秒再去让监督进程重启，而不是 5 秒就自杀。
const REGISTER_FAILURES: u8 = 60;

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
    // Temporary diagnosis: helper exits with no stderr output on this machine.
    let debug = std::env::temp_dir().join("deskfuse-helper-debug.log");
    let log = |step: &str, detail: String| {
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&debug)
        {
            use std::io::Write;
            let _ = writeln!(file, "{step}: {detail}");
        }
    };
    log("start", format!("parent {parent_pid}"));
    let parent = match ParentProcess::open(parent_pid) {
        Ok(parent) => parent,
        Err(error) => {
            log("parent_open_failed", error.to_string());
            return Err(error);
        }
    };
    log("parent_opened", String::new());
    {
        // Temporary diagnosis: confirm whether Windows granted the UIAccess flag.
        let mut token: HANDLE = std::ptr::null_mut();
        // SAFETY: pseudo handle of the current process and writable token out param.
        if unsafe { OpenProcessToken(unsafe { GetCurrentProcess() }, TOKEN_QUERY, &mut token) } != 0
        {
            let mut flag = 0_u32;
            let mut needed = 0_u32;
            let status = unsafe {
                GetTokenInformation(
                    token,
                    TokenUIAccess,
                    std::ptr::from_mut(&mut flag).cast(),
                    4,
                    &mut needed,
                )
            };
            log("uiaccess_query", format!("status {status} flag {flag}"));
            // SAFETY: token opened above.
            unsafe { CloseHandle(token) };
        } else {
            log("uiaccess_query", "open_failed".into());
        }
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    log("runtime_built", String::new());
    let result = runtime.block_on(run(parent));
    match &result {
        Ok(()) => log("run_ok", String::new()),
        Err(error) => log("run_failed", error.to_string()),
    }
    result
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
    /// Backoff after a capture the OS refused or revoked, so a broken local
    /// capture degrades only the controller direction instead of exiting the
    /// helper and taking remote-input injection down with it.
    capture_retry_at: Option<Instant>,
    /// Capture fault awaiting one report to the service; replayed only when it
    /// changes, because every `InputFault` clears the input heartbeat.
    pending_fault: Option<String>,
    reported_fault: Option<String>,
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
            capture_retry_at: None,
            pending_fault: None,
            reported_fault: None,
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
    let debug = std::env::temp_dir().join("deskfuse-helper-debug.log");
    let topology = match display_topology() {
        Ok(topology) => {
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&debug)
            {
                use std::io::Write;
                let _ = writeln!(file, "topology_ok: {} displays", topology.displays.len());
                for d in &topology.displays {
                    let _ = writeln!(
                        file,
                        "  {} {}x{} scale {:.2} rot {} primary {} panel {:?}",
                        d.id.0,
                        d.width,
                        d.height,
                        d.scale_factor,
                        d.rotation_degrees,
                        d.is_primary,
                        (d.physical_width_mm, d.physical_height_mm)
                    );
                }
            }
            topology
        }
        Err(error) => {
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&debug)
            {
                use std::io::Write;
                let _ = writeln!(file, "topology_failed: {error:?}");
            }
            return Err(io::Error::other(error));
        }
    };
    let injector = WindowsSharingInjector::new().map_err(io::Error::other)?;
    // Observation-only: the emergency watchdog must not take the exclusive
    // capture slot the sharing controller needs when this desktop is a controller.
    let emergency_capture = WindowsInputCapture::start_observer(2_048).map_err(io::Error::other)?;
    let mut controller = Controller::new();
    let mut client = Client::new();
    let mut cursor = 0_u64;
    let mut current_session = None;
    let mut registered = false;
    let mut session_epoch = 0_u64;
    let mut failures = 0_u8;
    let mut poll_failures = 0_u32;
    let mut topology_probe = Instant::now();
    let mut poll_due = Instant::now();
    loop {
        if !parent.alive()? {
            let _ = injector.release_all_nowait();
            stop_controller(&mut controller);
            return Err(io::Error::other("desktop parent process exited"));
        }
        if !registered {
            match client
                .exchange(ipc::Command::RegisterInput {
                    topology: topology.clone(),
                })
                .await
            {
                Ok(ipc::Reply::InputRegistered {
                    session_epoch: epoch,
                }) => {
                    registered = true;
                    session_epoch = epoch;
                    cursor = 0;
                    failures = 0;
                    // 上一代的推送是过期的，不能跨注册使用。
                    client.reset_pushes();
                    // 声明推送支持。旧服务会拒绝这条命令，helper 于是继续按
                    // 8ms 轮询——推送是加速，不是可用性的前提。
                    client.push_enabled = client
                        .exchange(ipc::Command::EnableInputPush)
                        .await
                        .is_ok_and(|reply| matches!(reply, ipc::Reply::Queued { .. }));
                }
                Ok(_) => {
                    failures = failures.saturating_add(1);
                    if let Ok(mut file) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&debug)
                    {
                        use std::io::Write;
                        let _ = writeln!(file, "register_unexpected: non-registered reply");
                    }
                    if failures >= REGISTER_FAILURES {
                        return Err(io::Error::other("desktop input session unavailable"));
                    }
                    sleep(Duration::from_millis(250)).await;
                    continue;
                }
                Err(error) => {
                    failures = failures.saturating_add(1);
                    if let Ok(mut file) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&debug)
                    {
                        use std::io::Write;
                        let _ = writeln!(file, "register_rejected: {error}");
                    }
                    if failures >= REGISTER_FAILURES {
                        return Err(io::Error::other("desktop input session unavailable"));
                    }
                    sleep(Duration::from_millis(250)).await;
                    continue;
                }
            }
        }
        // 服务端推送的投递不等下一次轮询，先落地。
        let pushed = client.take_pushed();
        if !pushed.is_empty() {
            if let Err(error) = apply_deliveries(
                pushed,
                &injector,
                &mut controller,
                &mut current_session,
                &topology,
                &mut cursor,
                &mut client,
            )
            .await
            {
                let _ = injector.release_all_nowait();
                return Err(error);
            }
        }
        let mut state_update = None;
        if poll_due.elapsed() >= poll_interval(&client, &controller, current_session) {
            poll_due = Instant::now();
            let stages = deskfuse_core::take_pending_stages();
            match client
                .exchange(ipc::Command::PollInput {
                    session_epoch,
                    after_sequence: cursor,
                    desktop: input_desktop(),
                    stages,
                })
                .await
            {
                Ok(ipc::Reply::InputPoll {
                    deliveries, state, ..
                }) => {
                    poll_failures = 0;
                    if let Err(error) = apply_deliveries(
                        deliveries,
                        &injector,
                        &mut controller,
                        &mut current_session,
                        &topology,
                        &mut cursor,
                        &mut client,
                    )
                    .await
                    {
                        let _ = injector.release_all_nowait();
                        return Err(error);
                    }
                    state_update = Some(state);
                }
                Ok(_) => {
                    // 服务没收下这批计数。加回去，等下一次成功的轮询。
                    deskfuse_core::add_pending_stages(stages);
                    registered = false;
                    continue;
                }
                Err(error) => {
                    deskfuse_core::add_pending_stages(stages);
                    current_session = None;
                    let _ = injector.release_all_nowait();
                    poll_failures = poll_failures.saturating_add(1);
                    // A slow service is transport noise, not a lost
                    // registration: re-registering here would clear the service's
                    // heartbeat and make every input capability flicker to 未就绪.
                    if is_registration_lost(&error) || poll_failures >= 20 {
                        registered = false;
                        stop_controller(&mut controller);
                        sleep(Duration::from_millis(100)).await;
                    } else {
                        let backoff = 8 + 4 * u64::from(poll_failures.min(8));
                        sleep(Duration::from_millis(backoff)).await;
                    }
                    continue;
                }
            }
        }

        // Controller maintenance from the latest state view.
        if let Some(state) = state_update {
            match maintain_controller(&mut controller, &state, &topology) {
                Ok(Some(released)) => {
                    let _ = client
                        .exchange(ipc::Command::ReleaseInputControl {
                            session_id: released.to_string(),
                        })
                        .await;
                }
                Ok(None) => {}
                Err(error) => {
                    let _ = injector.release_all_nowait();
                    return report_failure(&mut client, &error).await;
                }
            }
        }
        if let Some(fault) = controller.pending_fault.clone()
            && controller.reported_fault.as_deref() != Some(fault.as_str())
        {
            controller.reported_fault = Some(fault.clone());
            let _ = client
                .exchange(ipc::Command::InputFault {
                    message: fault.chars().take(256).collect(),
                })
                .await;
        }

        // Drain captured local input while this desktop is a controller.
        if controller.capture_started
            && let Some(local) = controller.local
        {
            let mut actions = Vec::new();
            let mut capture_fault = None;
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
                    Ok(Some(_)) => {
                        capture_fault = Some("本机输入捕获被系统停用，已停止共享控制".to_string());
                        break;
                    }
                    Err(error) => {
                        capture_fault = Some(format!("本机输入捕获中断，已恢复本机输入：{error}"));
                        break;
                    }
                }
            }
            if let Some(reason) = capture_fault {
                // Local capture is controller-side only: release what this desktop
                // was driving, keep injecting remote input, and retry the tap.
                let _ = injector.release_all_nowait();
                if let Some(outbound) = controller.outbound.take() {
                    let _ = client
                        .exchange(ipc::Command::ReleaseInputControl {
                            session_id: outbound.session.to_string(),
                        })
                        .await;
                }
                stop_controller(&mut controller);
                controller.capture_retry_at = Some(Instant::now() + CAPTURE_RETRY);
                controller.pending_fault = Some(reason);
                let _ = client.exchange(ipc::Command::ReclaimInput).await;
                continue;
            }
            if let Err(error) = apply_actions(&mut controller, &mut client, actions, local).await {
                let _ = injector.release_all_nowait();
                return report_failure(&mut client, &error.to_string()).await;
            }
            // Flush the routed batch collected during this iteration.
            if controller.outbound.is_some() {
                flush_pending(&mut controller, &mut client).await;
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
                        let _ = client
                            .exchange(ipc::Command::ReleaseInputControl {
                                session_id: outbound.session.to_string(),
                            })
                            .await;
                    }
                    let _ = client.exchange(ipc::Command::ReclaimInput).await;
                    return Err(io::Error::other(
                        "input reclaimed by Windows emergency chord",
                    ));
                }
                CapturePoll::Fault(_) | CapturePoll::Stopped => {
                    let _ = injector.release_all_nowait();
                    return report_failure(&mut client, "Windows emergency input observer stopped")
                        .await;
                }
            }
        }
        if let Some(error) = injector.take_error() {
            let _ = injector.release_all_nowait();
            return report_failure(&mut client, &error.to_string()).await;
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
                let _ = client
                    .exchange(ipc::Command::InputFault {
                        message: "display topology changed; input helper restarting".into(),
                    })
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

/// 有活动会话时按 8ms 轮询状态。空闲时降频，由推送负责低延迟投递；服务端对
/// 推送和轮询用同一个序号，所以降频只影响状态新鲜度，不影响输入延迟。
///
/// 空闲降频省下的是服务端每秒重建上百次输入状态。会话开始的那一次投递会晚
/// 一个空闲周期到，之后立刻回到 8ms——授权总是发生在用户真的开始操作之前。
///
/// 没有推送可用时保持 8ms：那时轮询是唯一的投递路径。
fn poll_interval(
    client: &Client,
    controller: &Controller,
    current_session: Option<InputSessionId>,
) -> Duration {
    let active =
        current_session.is_some() || controller.outbound.is_some() || !controller.grants.is_empty();
    if !client.push_enabled || active {
        Duration::from_millis(8)
    } else {
        Duration::from_millis(25)
    }
}

/// 落地一批投递，来源可以是轮询回复或服务端推送。
///
/// 推送和轮询会带同一批投递：推送取「服务端还没交付的」，轮询取「helper 还没
/// 落地的」，两个区间可以重叠。按 `cursor` 去重后每条投递只执行一次，重复的
/// 输入事件因此不会变成两次按键。
async fn apply_deliveries(
    deliveries: Vec<InputDelivery>,
    injector: &WindowsSharingInjector,
    controller: &mut Controller,
    current_session: &mut Option<InputSessionId>,
    topology: &deskfuse_core::DisplayTopology,
    cursor: &mut u64,
    client: &mut Client,
) -> io::Result<()> {
    for delivery in deliveries {
        if delivery.sequence() <= *cursor {
            continue;
        }
        *cursor = delivery.sequence();
        let result = match delivery {
            InputDelivery::Lease { session_id, .. } => {
                let session = parse_session(&session_id)?;
                if *current_session == Some(session) {
                    injector
                        .renew_deadline(INPUT_LEASE)
                        .then_some(())
                        .ok_or_else(|| io::Error::other("input lease expired before renewal"))
                } else {
                    let _ = injector.release_all_nowait();
                    if injector.arm_session(session, INPUT_LEASE) {
                        *current_session = Some(session);
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
                if *current_session != Some(session) {
                    let _ = injector.release_all_nowait();
                    if !injector.arm_session(session, INPUT_LEASE) {
                        return report_failure(client, "input batch belongs to an expired lease")
                            .await;
                    }
                    *current_session = Some(session);
                } else if !injector.renew_deadline(INPUT_LEASE) {
                    return report_failure(client, "input lease renewal failed").await;
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
                *current_session = None;
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
                        expires: Instant::now() + Duration::from_millis(lease_ms.clamp(1, 60_000)),
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
            let _ = client
                .exchange(ipc::Command::InputFault {
                    message: error.to_string().chars().take(256).collect(),
                })
                .await;
            return Err(error);
        }
    }
    Ok(())
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
        if controller
            .capture_retry_at
            .is_none_or(|retry_at| Instant::now() >= retry_at)
        {
            match controller.capture.start() {
                Ok(()) => {
                    controller.capture_started = true;
                    controller.capture_retry_at = None;
                    controller.pending_fault = None;
                    controller.reported_fault = None;
                }
                Err(error) => {
                    controller.capture_retry_at = Some(Instant::now() + CAPTURE_RETRY);
                    controller.pending_fault = Some(format!("无法启动本机输入捕获：{error}"));
                }
            }
        }
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
async fn flush_pending(controller: &mut Controller, client: &mut Client) {
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
    let reply = client
        .exchange(ipc::Command::SendInputEvents {
            session_id: session.to_string(),
            target_display: target,
            batch,
            events,
            relative,
        })
        .await;
    if !matches!(reply, Ok(ipc::Reply::Queued { .. })) {
        controller.restore_local();
    }
}

/// Applies crossing/routing actions exactly like the macOS coordinator, with
/// the service IPC replacing the in-process runtime channel.
async fn apply_actions(
    controller: &mut Controller,
    client: &mut Client,
    actions: Vec<SharingAction>,
    local: DeviceId,
) -> Result<(), io::Error> {
    for action in actions {
        match action {
            SharingAction::Crossed { from, to, position } => {
                let changing_device = from.device_id != to.device_id;
                if changing_device && let Some(outbound) = controller.outbound.take() {
                    let _ = client
                        .exchange(ipc::Command::LeaveInputControl {
                            session_id: outbound.session.to_string(),
                        })
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
                            let _ = client
                                .exchange(ipc::Command::ReleaseInputControl {
                                    session_id: session.to_string(),
                                })
                                .await;
                            controller.policy.reclaim();
                            continue;
                        }
                        controller.outbound = Some(grant);
                    }
                    let entered = client
                        .exchange(ipc::Command::EnterInputControl {
                            session_id: session.to_string(),
                            target_display: to.display_id.0.clone(),
                            x: position.x,
                            y: position.y,
                        })
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
                                    let _ = client
                                        .exchange(ipc::Command::ReleaseInputControl {
                                            session_id: outbound.session.to_string(),
                                        })
                                        .await;
                                }
                                controller.restore_local();
                                let _ = client.exchange(ipc::Command::ReclaimInput).await;
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
                    flush_pending(controller, client).await;
                }
                if let Some((dx, dy)) = relative {
                    match merge_relative(controller.pending_relative, dx, dy) {
                        Ok(next) => controller.pending_relative = next,
                        Err(()) => {
                            if controller.pending_relative.is_some() {
                                flush_pending(controller, client).await;
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
                    let _ = client
                        .exchange(ipc::Command::ReleaseInputControl {
                            session_id: outbound.session.to_string(),
                        })
                        .await;
                }
                controller.restore_local();
                let _ = client.exchange(ipc::Command::ReclaimInput).await;
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

/// Service-side rejections that invalidate the registration, as opposed to a
/// transport error (busy pipe, timeout) that only needs a retry.
fn is_registration_lost(error: &io::Error) -> bool {
    let message = error.to_string();
    message.contains("session_expired")
        || message.contains("desktop_unavailable")
        || message.contains("input_unregistered")
        || message.contains("desktop input protocol")
}

async fn report_failure(client: &mut Client, message: &str) -> io::Result<()> {
    // Temporary diagnosis: name the fault the helper reports before exiting.
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(std::env::temp_dir().join("deskfuse-helper-debug.log"))
    {
        use std::io::Write;
        let _ = writeln!(file, "report_failure: {message}");
    }
    let _ = client
        .exchange(ipc::Command::InputFault {
            message: message.chars().take(256).collect(),
        })
        .await;
    Err(io::Error::other("Windows input helper stopped safely"))
}

/// The helper's single authenticated pipe.
///
/// The connection outlives every request. The service listens with one pipe
/// instance at a time, so a connect per poll made this helper lose the pipe to
/// ordinary clients and spend its life on `ERROR_PIPE_BUSY`; one connection for
/// the process removes the race instead of retrying against it.
struct Client {
    link: Option<NamedPipeClient>,
    request_id: u64,
    /// 服务端推送的投递，由主循环落地。
    pushed: Vec<InputDelivery>,
    pushed_sequence: u64,
    /// 服务端接受了推送声明。旧服务会拒绝它，于是继续按轮询节奏工作。
    /// 断线不重置：重连后服务端仍按同一个 helper 进程推送。
    push_enabled: bool,
}

impl Client {
    fn new() -> Self {
        Self {
            link: None,
            request_id: 0,
            pushed: Vec::new(),
            pushed_sequence: 0,
            push_enabled: false,
        }
    }

    /// 断开只丢这条连接；下一次请求会重新建立。推送支持是服务的属性，
    /// 不随连接失效，重连后服务端会按同一个 helper 进程恢复推送。
    fn disconnect(&mut self) {
        self.link = None;
    }

    fn take_pushed(&mut self) -> Vec<InputDelivery> {
        std::mem::take(&mut self.pushed)
    }

    /// 重新注册后上一代的推送和序号都作废：服务端可能已经重置过序号。
    fn reset_pushes(&mut self) {
        self.pushed.clear();
        self.pushed_sequence = 0;
    }

    async fn exchange(&mut self, command: ipc::Command) -> io::Result<ipc::Reply> {
        self.request_id = self.request_id.saturating_add(1).max(1);
        let id = self.request_id;
        if self.link.is_none() {
            let pipe = security::open_client(ipc::PIPE_NAME)?;
            security::verify_server(pipe.as_raw_handle())?;
            self.link = Some(pipe);
        }
        let outcome = timeout(REQUEST_TIMEOUT, self.round_trip(id, command)).await;
        match outcome {
            Ok(Ok(reply)) => Ok(reply),
            Ok(Err(error)) => {
                self.disconnect();
                Err(error)
            }
            Err(_) => {
                self.disconnect();
                Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "desktop input request timed out",
                ))
            }
        }
    }

    async fn round_trip(&mut self, id: u64, command: ipc::Command) -> io::Result<ipc::Reply> {
        let Some(mut pipe) = self.link.take() else {
            return Err(io::Error::other("desktop input link is closed"));
        };
        let result = pump(
            &mut pipe,
            &mut self.pushed,
            &mut self.pushed_sequence,
            id,
            command,
        )
        .await;
        if result.is_ok() {
            self.link = Some(pipe);
        }
        result
    }
}

/// One request/response round trip. Frames the service pushes while this request
/// is in flight are collected, not mistaken for the reply.
async fn pump(
    pipe: &mut NamedPipeClient,
    pushed: &mut Vec<InputDelivery>,
    pushed_sequence: &mut u64,
    id: u64,
    command: ipc::Command,
) -> io::Result<ipc::Reply> {
    protocol::write_frame_limited(
        pipe,
        &ipc::Request {
            version: ipc::VERSION,
            request_id: id,
            command,
        },
        ipc::MAX_FRAME,
    )
    .await?;
    loop {
        let response: ipc::Response = protocol::read_frame_limited(pipe, ipc::MAX_FRAME).await?;
        if response.version != ipc::VERSION {
            return Err(io::Error::other("desktop input protocol mismatch"));
        }
        if response.request_id == ipc::PUSH_REQUEST_ID {
            // 推送不回执：回执字节只对应请求回复，保持一一对应才不会错位。
            if let ipc::Reply::InputPush {
                deliveries,
                sequence,
            } = response.result
                && sequence > *pushed_sequence
            {
                *pushed_sequence = sequence;
                pushed.extend(deliveries);
            }
            continue;
        }
        if response.request_id != id {
            return Err(io::Error::other("desktop input protocol mismatch"));
        }
        use tokio::io::AsyncWriteExt;
        pipe.write_all(&[0x06]).await?;
        return match response.result {
            ipc::Reply::Rejected { message, .. } => Err(io::Error::other(message)),
            ipc::Reply::UnsupportedVersion => {
                Err(io::Error::other("desktop input protocol unsupported"))
            }
            reply => Ok(reply),
        };
    }
}
