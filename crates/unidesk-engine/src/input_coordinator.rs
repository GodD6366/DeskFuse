//! Dedicated input orchestration. File reads and clipboard delivery cannot stall
//! this thread; native capture and injection also own fail-safe lease timers.
use crate::platform::{
    PermissionSnapshot, PermissionStatus, create_permission_service, display_topology,
};
use crate::tokio_bridge::UiEvent;
use futures::{StreamExt, channel::mpsc::UnboundedSender};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime},
};
use unidesk_core::{
    DeviceId, DisplayKey, InputActivationId, InputActivationStatus, InputSessionId, InputShareRole,
    InputSharingSettings, InputSharingSnapshot, SharedInputEvent,
};
use unidesk_input::{
    Point, Vector2,
    sharing::{HotkeyBindings, HotkeyChord, SharingAction, SharingController, SharingDisplay},
};
use unidesk_platform::{CaptureEvent, SharedInputCapture, SharedInputInjector, timing};
use unidesk_runtime::{RuntimeCommand, RuntimeEvent, RuntimeHandle};

const LEASE_TTL: Duration = Duration::from_millis(1500);

pub struct InputCoordinator {
    stopping: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
    update_pause: Arc<std::sync::atomic::AtomicU8>,
    remote: Option<unidesk_runtime::input_service::ServiceControl>,
}
impl InputCoordinator {
    pub fn start(
        runtime: RuntimeHandle,
        ui: UnboundedSender<UiEvent>,
        recording: Arc<AtomicBool>,
        emergency: Arc<AtomicBool>,
    ) -> Self {
        if let Some(remote) = runtime.input_service_control() {
            let stopping = Arc::new(AtomicBool::new(false));
            let stop = stopping.clone();
            let channel = remote.clone();
            let worker = thread::Builder::new()
                .name("unidesk-input-ui".into())
                .spawn(move || {
                    let mut previous: Option<unidesk_runtime::input_service::NativeStatus> = None;
                    let mut previous_input: Option<unidesk_core::InputSharingSnapshot> = None;
                    let mut previous_recording = None;
                    let mut previous_pid = 0;
                    while !stop.load(Ordering::Acquire) {
                        let native = channel
                            .status
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .clone();
                        let rec = recording.load(Ordering::Acquire);
                        if (previous_recording != Some(rec) || previous_pid != native.pid)
                            && channel.signal(if rec { b'R' } else { b'r' })
                        {
                            previous_recording = Some(rec);
                            previous_pid = native.pid;
                        }
                        if emergency.swap(false, Ordering::AcqRel) && !channel.signal(b'E') {
                            emergency.store(true, Ordering::Release);
                        }
                        #[cfg(target_os = "macos")]
                        if previous
                            .as_ref()
                            .is_some_and(|p| p.running && !native.running)
                        {
                            let _ =
                                unidesk_platform_macos::sharing::restore_pointer_after_service_exit(
                                );
                        }
                        if previous.as_ref() != Some(&native) {
                            let _ = ui.unbounded_send(UiEvent::InputStatus {
                                error: native.error.clone(),
                            });
                            let _ = ui.unbounded_send(UiEvent::InputLockStatus(native.locked));
                            let _ = ui.unbounded_send(UiEvent::InputRecordingReady(
                                native.recording_ready,
                            ));
                            let permissions = helper_permissions(
                                &native,
                                create_permission_service().snapshot().full_disk_access,
                            );
                            let _ = ui.unbounded_send(UiEvent::PermissionSnapshot(permissions));
                            previous = Some(native);
                        }
                        let input = runtime.input_snapshot();
                        if previous_input.as_ref() != Some(&input) {
                            previous_input = Some(input.clone());
                            let _ =
                                ui.unbounded_send(UiEvent::InputSharingSnapshot(Box::new(input)));
                        }
                        thread::sleep(Duration::from_millis(100));
                    }
                })
                .ok();
            return Self {
                stopping,
                worker,
                update_pause: Arc::new(std::sync::atomic::AtomicU8::new(0)),
                remote: Some(remote),
            };
        }
        Self::start_local(
            runtime,
            ui,
            recording,
            emergency,
            Arc::new(std::sync::atomic::AtomicU8::new(0)),
            Arc::new(AtomicBool::new(false)),
        )
    }
    pub(crate) fn start_local(
        runtime: RuntimeHandle,
        ui: UnboundedSender<UiEvent>,
        recording: Arc<AtomicBool>,
        emergency: Arc<AtomicBool>,
        update_pause: Arc<std::sync::atomic::AtomicU8>,
        stopping: Arc<AtomicBool>,
    ) -> Self {
        let signal = stopping.clone();
        let pause_signal = update_pause.clone();
        let worker = thread::Builder::new()
            .name("unidesk-input".into())
            .spawn(move || {
                unidesk_transport::input_executor::prioritize_current_thread();
                if let Ok(executor) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    executor.block_on(run(runtime, ui, signal, recording, emergency, pause_signal));
                }
            })
            .ok();
        Self {
            stopping,
            worker,
            update_pause,
            remote: None,
        }
    }
    pub fn pause_for_update(&self, manual: bool) -> bool {
        if let Some(remote) = &self.remote {
            remote.signal(if manual { b'M' } else { b'P' });
            return remote.paused();
        }
        // 1 waits for idle; 3 explicitly ends active control on the input worker.
        let requested = if manual { 3 } else { 1 };
        let _ =
            self.update_pause
                .compare_exchange(0, requested, Ordering::AcqRel, Ordering::Acquire);
        if manual {
            let _ = self
                .update_pause
                .compare_exchange(1, 3, Ordering::AcqRel, Ordering::Acquire);
        }
        self.update_pause.load(Ordering::Acquire) == 2
    }
    pub fn resume_after_update(&self) {
        if let Some(remote) = &self.remote {
            remote.signal(b'p');
            return;
        }
        unidesk_runtime::input_service_types::resume_update_pause(&self.update_pause);
    }
}
impl Drop for InputCoordinator {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub use unidesk_input::layout::layout_displays;

#[derive(Clone, Debug)]
struct LeaseProgress {
    session: InputSessionId,
    revision: u64,
    advanced_at: Instant,
    advanced_wall: SystemTime,
}
impl LeaseProgress {
    fn observe(&mut self, session: InputSessionId, revision: u64, now: Instant) -> bool {
        if self.expired(now) || self.session != session {
            return false;
        }
        if revision > self.revision {
            self.session = session;
            self.revision = revision;
            self.advanced_at = now;
            self.advanced_wall = SystemTime::now();
            true
        } else {
            false
        }
    }
    fn remaining(&self, now: Instant) -> Duration {
        let elapsed = now
            .saturating_duration_since(self.advanced_at)
            .max(self.advanced_wall.elapsed().unwrap_or(LEASE_TTL));
        LEASE_TTL.saturating_sub(elapsed)
    }
    fn expired(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.advanced_at) >= LEASE_TTL
            || self
                .advanced_wall
                .elapsed()
                .map_or(true, |elapsed| elapsed >= LEASE_TTL)
    }
}

struct State {
    last_capture_at: Option<Instant>,
    crossing_started: Option<Instant>,
    last_slow_capture: Option<Instant>,
    ui: UnboundedSender<UiEvent>,
    local: DeviceId,
    policy: SharingController,
    capture: Box<dyn SharedInputCapture>,
    capture_started: bool,
    injector: Option<Box<dyn SharedInputInjector>>,
    snapshot: InputSharingSnapshot,
    effective: InputSharingSettings,
    last_report: Option<NativeReport>,
    layout: Vec<SharingDisplay>,
    grants: BTreeMap<InputSessionId, LeaseProgress>,
    retired: BTreeSet<InputSessionId>,
    paused_activations: BTreeSet<InputActivationId>,
    outbound: Option<LeaseProgress>,
    inbound: Option<LeaseProgress>,
    anchor: (f64, f64),
    available: bool,
    last_error: Option<String>,
    observer_thread: Option<Arc<thread::Thread>>,
    /// Positions this machine forwarded to a peer, and positions it injected for a
    /// peer. Motion is forwarded as absolute positions and every stage keeps only
    /// the newest one, so the *update rate and its regularity* is what a user
    /// perceives as smoothness; comparing both sides localises where the rate is
    /// lost. Counts and timings only, never coordinates.
    out_motion: MotionCadence,
    in_motion: MotionCadence,
    cadence_at: Instant,
}
/// Bounded so a once-per-second summary stays cheap even at 1000 Hz sampling.
const CADENCE_GAPS: usize = 1024;
const CADENCE_WINDOW: Duration = Duration::from_secs(1);
#[derive(Default)]
struct MotionCadence {
    count: u64,
    last: Option<Instant>,
    gaps_ms: Vec<f32>,
}
impl MotionCadence {
    fn observe(&mut self, samples: u64, now: Instant) {
        if samples == 0 {
            return;
        }
        if let Some(previous) = self.last
            && self.gaps_ms.len() < CADENCE_GAPS
        {
            self.gaps_ms
                .push(now.saturating_duration_since(previous).as_secs_f32() * 1000.0);
        }
        self.last = Some(now);
        self.count = self.count.saturating_add(samples);
    }
    /// Returns `(samples, p50_ms, p99_ms, max_ms)` for the window and resets it.
    fn take(&mut self) -> Option<(u64, f32, f32, f32)> {
        if self.count == 0 {
            return None;
        }
        let count = std::mem::take(&mut self.count);
        self.last = None;
        let mut gaps = std::mem::take(&mut self.gaps_ms);
        gaps.sort_by(f32::total_cmp);
        let at = |quantile: f64| -> f32 {
            if gaps.is_empty() {
                return 0.0;
            }
            // Nearest-rank, so a two-sample window reports the lower value for
            // p50 and the upper one for p99 instead of the same element.
            let rank = (quantile * gaps.len() as f64).ceil() as usize;
            gaps[rank.saturating_sub(1).min(gaps.len() - 1)]
        };
        let max = gaps.last().copied().unwrap_or_default();
        Some((count, at(0.5), at(0.99), max))
    }
}
impl State {
    fn error(&mut self, ui: &UnboundedSender<UiEvent>, message: String) {
        if self.last_error.as_ref() != Some(&message) {
            self.last_error = Some(message.clone());
            let _ = ui.unbounded_send(UiEvent::InputStatus {
                error: Some(message),
            });
        }
    }
    async fn reclaim(&mut self, runtime: &RuntimeHandle, restore: Option<(f64, f64)>) {
        #[cfg(target_os = "macos")]
        unidesk_platform_macos::sharing::microphone_shortcut::cancel();
        self.crossing_started = None;
        let outbound = self.outbound.take();
        let inbound = self.inbound.take();
        self.policy.reclaim();
        let _ = self.ui.unbounded_send(UiEvent::InputLockStatus(false));
        let _ = self
            .capture
            .exit_remote(if outbound.is_some() { restore } else { None });
        if let Some(injector) = &self.injector {
            let _ = injector.release_all_nowait();
        }
        for lease in [outbound, inbound].into_iter().flatten() {
            self.retired.insert(lease.session);
            let _ = runtime
                .send(RuntimeCommand::ReleaseInputControl {
                    session_id: lease.session,
                })
                .await;
        }
    }
    async fn pause_managed(&mut self, runtime: &RuntimeHandle) {
        self.available = false;
        self.last_report = None;
        if let Some(managed) = &self.snapshot.managed_control {
            self.paused_activations.insert(managed.activation_id);
        }
        // This local intent targets the activation current when the actor handles
        // it, including a replacement that arrived after our last snapshot.
        let _ = runtime
            .send(RuntimeCommand::PauseCurrentManagedActivation)
            .await;
    }

    async fn reset_adapters(&mut self, runtime: &RuntimeHandle) {
        self.available = false;
        self.last_report = None;
        let _ = runtime
            .send(RuntimeCommand::SetInputAvailability {
                available: false,
                reason: Some("正在切换本机输入状态".into()),
            })
            .await;
        if let Some(managed) = &self.snapshot.managed_control {
            let _ = runtime
                .send(RuntimeCommand::SetManagedInputAvailability {
                    activation_id: managed.activation_id,
                    status: InputActivationStatus::Preparing,
                    reason: Some("正在切换本机输入状态".into()),
                })
                .await;
        }
        self.reclaim(runtime, Some(self.anchor)).await;
        let _ = self.capture.stop();
        self.capture_started = false;
        self.injector = None;
    }

    async fn native_failed(&mut self, runtime: &RuntimeHandle, reason: String) {
        self.available = false;
        self.last_report = None;
        let snapshot = self.snapshot.clone();
        self.report_native(
            runtime,
            &snapshot,
            InputActivationStatus::Unavailable,
            Some(reason),
        )
        .await;
        self.reclaim(runtime, Some(self.anchor)).await;
        let _ = self.capture.stop();
        self.capture_started = false;
        self.injector = None;
    }

    async fn report_native(
        &mut self,
        runtime: &RuntimeHandle,
        snapshot: &InputSharingSnapshot,
        status: InputActivationStatus,
        reason: Option<String>,
    ) {
        let live = runtime.input_snapshot();
        if !native_report_is_current(snapshot, &live) {
            self.available = false;
            return;
        }
        let report = if let Some(managed) = &snapshot.managed_control {
            if managed.status == InputActivationStatus::PausedByUser
                || self.paused_activations.contains(&managed.activation_id)
            {
                self.available = false;
                return;
            }
            NativeReport::Managed {
                activation_id: managed.activation_id,
                status,
                reason,
            }
        } else {
            NativeReport::Local {
                available: status == InputActivationStatus::Ready,
                reason,
            }
        };
        self.available = status == InputActivationStatus::Ready;
        if self.last_report.as_ref() != Some(&report) {
            if runtime.send(report.command()).await.is_ok() {
                self.last_report = Some(report);
            }
            if self.available {
                self.last_error = None;
                let _ = self.ui.unbounded_send(UiEvent::InputStatus { error: None });
            }
        }
    }

    async fn actions(
        &mut self,
        actions: Vec<SharingAction>,
        runtime: &RuntimeHandle,
        ui: &UnboundedSender<UiEvent>,
    ) {
        for action in actions {
            let crossing = matches!(&action, SharingAction::Crossed { .. });
            let action_started = (crossing && timing::enabled()).then(Instant::now);
            let result = match action {
                SharingAction::Reclaim => {
                    self.pause_managed(runtime).await;
                    self.reclaim(runtime, Some(self.anchor)).await;
                    Ok(())
                }
                SharingAction::LockChanged(locked) => {
                    let _ = ui.unbounded_send(UiEvent::InputLockStatus(locked));
                    Ok(())
                }
                SharingAction::Crossed { from, to, position } => {
                    self.crossing_started = action_started;
                    let link = self
                        .snapshot
                        .links
                        .iter()
                        .find(|link| link.peer_id == to.device_id);
                    timing::record!(
                        "crossing_begin",
                        self.last_capture_at
                            .map_or(Duration::ZERO, |captured| captured.elapsed()),
                        format!(
                            "from={} to={} session={:?} mode={:?} motion_mode={:?} capture_age_available={}",
                            from.device_id,
                            to.device_id,
                            link.and_then(|link| link.control_session),
                            self.effective.switch_mode,
                            link.and_then(|link| link.motion_mode.as_deref()),
                            self.last_capture_at.is_some()
                        ),
                    );
                    let changing_device = from.device_id != to.device_id;
                    if changing_device && let Some(lease) = self.outbound.take() {
                        let _ = runtime
                            .send(RuntimeCommand::LeaveInputControl {
                                session_id: lease.session,
                            })
                            .await;
                    }
                    if to.device_id == self.local {
                        let restore = self.layout.iter().find(|d| d.key == to).map(|d| {
                            (
                                d.display.frame_x + position.x,
                                d.display.frame_y + position.y,
                            )
                        });
                        if changing_device {
                            self.capture.exit_remote(restore).map_err(|e| e.to_string())
                        } else if let Some((x, y)) = restore {
                            crate::platform::warp_pointer(x, y).map_err(|e| e.to_string())
                        } else {
                            Ok(())
                        }
                    } else if let Some(link) = self.snapshot.links.iter().find(|l| {
                        l.peer_id == to.device_id && l.ready && l.control_session.is_some()
                    }) {
                        let session = link.control_session.expect("filtered granted session");
                        if self
                            .outbound
                            .as_ref()
                            .is_none_or(|lease| lease.session != session)
                        {
                            let Some(progress) = self
                                .grants
                                .get(&session)
                                .filter(|p| !p.expired(Instant::now()))
                                .cloned()
                            else {
                                timing::record!(
                                    "crossing_aborted",
                                    action_started.map_or(Duration::ZERO, |at| at.elapsed()),
                                    "reason=missing_or_expired_grant".into(),
                                );
                                self.reclaim(runtime, Some(self.anchor)).await;
                                return;
                            };
                            if self.retired.contains(&session)
                                || !self
                                    .capture
                                    .arm_session(session, progress.remaining(Instant::now()))
                            {
                                timing::record!(
                                    "crossing_aborted",
                                    action_started.map_or(Duration::ZERO, |at| at.elapsed()),
                                    "reason=retired_or_capture_arm_failed".into(),
                                );
                                self.retired.insert(session);
                                let _ = runtime
                                    .send(RuntimeCommand::ReleaseInputControl {
                                        session_id: session,
                                    })
                                    .await;
                                self.reclaim(runtime, Some(self.anchor)).await;
                                return;
                            }
                            self.outbound = Some(progress);
                        }
                        let enqueue_started = Instant::now();
                        let queued = runtime
                            .send(RuntimeCommand::EnterInputControl {
                                session_id: session,
                                target_display: to.display_id,
                                x: position.x,
                                y: position.y,
                            })
                            .await;
                        timing::record!(
                            "enter_command_enqueue",
                            enqueue_started.elapsed(),
                            format!("session={session:?} ok={}", queued.is_ok()),
                        );
                        match queued {
                            Ok(()) if changing_device => self
                                .capture
                                .enter_remote(self.anchor.0, self.anchor.1)
                                .map_err(|e| e.to_string()),
                            Ok(()) => Ok(()),
                            Err(error) => Err(error.to_string()),
                        }
                    } else {
                        Err("控制租约已失效".into())
                    }
                }
                SharingAction::Route { target, event } => {
                    if let Some(lease) = &self.outbound {
                        let first_motion = matches!(&event, SharedInputEvent::Motion { .. })
                            .then(|| self.crossing_started.take())
                            .flatten();
                        let queued = runtime
                            .try_send(RuntimeCommand::SendSharedInput {
                                session_id: lease.session,
                                target_display: target.display_id,
                                events: vec![event],
                            })
                            .map_err(|e| e.to_string());
                        if let Some(started) = first_motion {
                            timing::record!(
                                "crossing_first_motion_queued",
                                started.elapsed(),
                                format!("session={:?} ok={}", lease.session, queued.is_ok()),
                            );
                        }
                        queued
                    } else {
                        Ok(())
                    }
                }
            };
            if crossing {
                timing::record!(
                    "crossing_complete",
                    action_started.map_or(Duration::ZERO, |at| at.elapsed()),
                    format!("ok={}", result.is_ok()),
                );
            }
            if let Err(error) = result {
                self.error(ui, error);
                self.reclaim(runtime, Some(self.anchor)).await;
                break;
            }
        }
    }
}

fn active_layout_changed(
    previous: &[SharingDisplay],
    next: &[SharingDisplay],
    active: Option<&DisplayKey>,
) -> bool {
    active.is_some_and(|key| {
        previous.iter().find(|d| &d.key == key) != next.iter().find(|d| &d.key == key)
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum NativeReport {
    Local {
        available: bool,
        reason: Option<String>,
    },
    Managed {
        activation_id: InputActivationId,
        status: InputActivationStatus,
        reason: Option<String>,
    },
}
impl NativeReport {
    fn command(&self) -> RuntimeCommand {
        match self {
            Self::Local { available, reason } => RuntimeCommand::SetInputAvailability {
                available: *available,
                reason: reason.clone(),
            },
            Self::Managed {
                activation_id,
                status,
                reason,
            } => RuntimeCommand::SetManagedInputAvailability {
                activation_id: *activation_id,
                status: *status,
                reason: reason.clone(),
            },
        }
    }
}

fn activation_identity(snapshot: &InputSharingSnapshot) -> Option<(InputActivationId, DeviceId)> {
    snapshot
        .managed_control
        .as_ref()
        .map(|managed| (managed.activation_id, managed.controller))
}
fn latch_emergency(
    requested: &AtomicBool,
    snapshot: &InputSharingSnapshot,
    paused: &mut BTreeSet<InputActivationId>,
) -> bool {
    if !requested.swap(false, Ordering::AcqRel) {
        return false;
    }
    if let Some(managed) = &snapshot.managed_control {
        paused.insert(managed.activation_id);
    }
    true
}
fn effective_native_settings(
    snapshot: &InputSharingSnapshot,
    paused: &BTreeSet<InputActivationId>,
) -> InputSharingSettings {
    let mut effective = snapshot.effective_settings();
    if snapshot
        .managed_control
        .as_ref()
        .is_some_and(|managed| paused.contains(&managed.activation_id))
    {
        effective.enabled = false;
        effective.role = InputShareRole::Disabled;
    }
    effective
}
// Geometry changes rebuild routing, not permission adapters or activation leases.
fn native_settings_equal(a: &InputSharingSettings, b: &InputSharingSettings) -> bool {
    // Neither layout coordinates nor remembered combinations change permissions.
    // Compare by reference rather than cloning the placement history on each tick.
    a.shortcuts == b.shortcuts
        && a.disabled_corners == b.disabled_corners
        && a.corner_size == b.corner_size
        && a.enabled == b.enabled
        && a.role == b.role
        && a.authorized_controller == b.authorized_controller
        && a.participants == b.participants
        && a.switch_mode == b.switch_mode
}
fn native_configuration_changed(
    previous: &InputSharingSnapshot,
    next: &InputSharingSnapshot,
) -> bool {
    activation_identity(previous) != activation_identity(next)
        || !native_settings_equal(&previous.effective_settings(), &next.effective_settings())
}
fn native_report_is_current(
    observed: &InputSharingSnapshot,
    current: &InputSharingSnapshot,
) -> bool {
    activation_identity(observed) == activation_identity(current)
        && native_settings_equal(
            &observed.effective_settings(),
            &current.effective_settings(),
        )
}
fn native_phase(
    permission: PermissionSnapshot,
    session_active: bool,
    _recording: bool,
    adapters_ready: bool,
    error: Option<String>,
) -> (InputActivationStatus, Option<String>) {
    use InputActivationStatus::*;
    if permission.accessibility() != PermissionStatus::Granted {
        return (WaitingAccessibility, Some("等待辅助功能权限".into()));
    }
    // Production sharing uses a modifying Default event tap, authorized by
    // Accessibility. Input Monitoring only gates listen-only keyboard taps;
    // its diagnostic status must not prevent creating our active tap/injector.
    if !session_active {
        return (LockedSession, Some("等待解锁本机桌面".into()));
    }
    if let Some(error) = error {
        return (Unavailable, Some(error));
    }
    if adapters_ready {
        (Ready, None)
    } else {
        (Preparing, Some("正在准备本机输入服务".into()))
    }
}

fn hotkey_bindings(settings: &InputSharingSettings) -> HotkeyBindings {
    let shortcuts = &settings.shortcuts;
    let chord = |shortcut: &unidesk_core::InputShortcut| HotkeyChord {
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
    }
}

fn native_available(
    settings: &unidesk_core::InputSharingSettings,
    permitted: bool,
    capture_started: bool,
    injector_ready: bool,
) -> bool {
    settings.enabled
        && settings.role != InputShareRole::Disabled
        && permitted
        && capture_started
        && injector_ready
}

const NATIVE_OBSERVATION_MAX_AGE: Duration = Duration::from_secs(1);

#[derive(Clone)]
struct NativeObservation {
    permission: PermissionSnapshot,
    session_active: bool,
    topology: Result<unidesk_core::DisplayTopology, String>,
    started_at: Instant,
    started_wall: SystemTime,
}
impl NativeObservation {
    fn permits_input(&self) -> bool {
        self.fresh()
            && self.permission.accessibility() == PermissionStatus::Granted
            && self.session_active
            && self.topology.is_ok()
    }
    fn fresh(&self) -> bool {
        self.started_at.elapsed() < NATIVE_OBSERVATION_MAX_AGE
            && self
                .started_wall
                .elapsed()
                .is_ok_and(|age| age < NATIVE_OBSERVATION_MAX_AGE)
    }
}

type NativeObserver = (
    tokio::sync::watch::Receiver<Option<NativeObservation>>,
    Option<thread::JoinHandle<()>>,
    Option<Arc<thread::Thread>>,
);

/// Native permission/session/display queries may block in macOS. Keep them on
/// their own thread and publish only the latest observation, never an input
/// work item. A stalled observer fails closed through the freshness check.
fn native_observer(stopping: Arc<AtomicBool>) -> NativeObserver {
    let (tx, rx) = tokio::sync::watch::channel(None);
    let worker = thread::Builder::new()
        .name("unidesk-input-observer".into())
        .spawn(move || {
            let permissions = create_permission_service();
            let mut last_topology: Option<(
                Instant,
                Result<unidesk_core::DisplayTopology, String>,
            )> = None;
            while !stopping.load(Ordering::Acquire) {
                let started_at = Instant::now();
                let started_wall = SystemTime::now();
                let topology = match last_topology.as_ref() {
                    Some((at, value))
                        if at.elapsed() < Duration::from_millis(500) && value.is_ok() =>
                    {
                        value.clone()
                    }
                    _ => {
                        let next = display_topology().map_err(|error| error.to_string());
                        last_topology = Some((started_at, next.clone()));
                        next
                    }
                };
                let observation = NativeObservation {
                    permission: permissions.snapshot(),
                    session_active: crate::platform::session_is_active(),
                    topology,
                    started_at,
                    started_wall,
                };
                if tx.send(Some(observation)).is_err() {
                    break;
                }
                // 辅助功能用廉价 AX 查询保持 100ms；显示器枚举更重，上面单独节流。
                thread::park_timeout(Duration::from_millis(100));
            }
        })
        .ok();
    let thread_handle = worker.as_ref().map(|w| Arc::new(w.thread().clone()));
    (rx, worker, thread_handle)
}

/// Shutdown must not wait for a stuck macOS status query. The caller has already
/// set stopping and released native input; a detached observer exits when its
/// current query returns. It owns no active capture/injection state.
fn finish_native_observer(observer: Option<thread::JoinHandle<()>>) {
    if let Some(observer) = observer {
        observer.thread().unpark();
        if observer.is_finished() {
            let _ = observer.join();
        }
    }
}

async fn run(
    runtime: RuntimeHandle,
    ui: UnboundedSender<UiEvent>,
    stopping: Arc<AtomicBool>,
    recording: Arc<AtomicBool>,
    emergency: Arc<AtomicBool>,
    update_pause: Arc<std::sync::atomic::AtomicU8>,
) {
    let local = runtime.snapshot_view().snapshot.local_device_id;
    let mut events = runtime.input_event_stream();
    let mut state = State {
        last_capture_at: None,
        crossing_started: None,
        last_slow_capture: None,
        ui: ui.clone(),
        local,
        policy: SharingController::new(local),
        capture: crate::platform::create_capture(),
        capture_started: false,
        injector: None,
        snapshot: Default::default(),
        effective: Default::default(),
        last_report: None,
        layout: vec![],
        grants: BTreeMap::new(),
        retired: BTreeSet::new(),
        paused_activations: BTreeSet::new(),
        outbound: None,
        inbound: None,
        anchor: (0.0, 0.0),
        available: false,
        last_error: None,
        observer_thread: None,
        out_motion: MotionCadence::default(),
        in_motion: MotionCadence::default(),
        cadence_at: Instant::now(),
    };
    let (mut observations, observer, observer_thread) = native_observer(stopping.clone());
    state.observer_thread = observer_thread;
    let mut observer_open = observer.is_some();
    let mut previous_permission = None;
    let mut previous_ui_snapshot: Option<InputSharingSnapshot> = None;
    let mut recording_active = false;
    let started = Instant::now();
    let mut next_refresh = Instant::now();
    let mut remote_application_paused = false;
    // Housekeeping/dwell/lease checks only. Mouse delivery is notification-driven.
    let mut timer = tokio::time::interval(Duration::from_millis(20));
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        #[cfg(target_os = "macos")]
        unidesk_platform_macos::sharing::microphone_shortcut::set_route(
            state.local,
            state.policy.active_display().map(|display| display.device_id),
            (state.effective.role == InputShareRole::Controlled)
                .then_some(state.effective.authorized_controller)
                .flatten(),
            state.inbound.as_ref().is_some_and(|lease| !lease.expired(Instant::now())),
            state.available && state.effective.enabled && !recording.load(Ordering::Acquire)
                && observations.borrow().as_ref().is_some_and(NativeObservation::permits_input),
        );
        if stopping.load(Ordering::Acquire) {
            break;
        }
        let pause = update_pause.load(Ordering::Acquire);
        if pause == 4 || pause == 6 {
            // Stopping native adapters alone leaves idle network grants marked
            // Ready. Their local watchdogs expire while this loop is suspended,
            // preventing sharing from recovering after the canvas releases us.
            // Retire the old grants and negotiate fresh ones on native Ready.
            let snapshot = runtime.input_snapshot();
            state
                .report_native(
                    &runtime,
                    &snapshot,
                    InputActivationStatus::Preparing,
                    Some("远程应用暂时占用输入，退出后自动恢复".into()),
                )
                .await;
            state.available = false;
            state.retired.extend(state.grants.keys().copied());
            state.grants.clear();
            state.reclaim(&runtime, Some(state.anchor)).await;
            let _ = state.capture.stop();
            state.capture_started = false;
            if let Some(injector) = state.injector.take() {
                let _ = injector.release_all();
            }
            let _ = update_pause.compare_exchange(
                pause,
                pause + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
        }
        if matches!(pause, 4..=7) {
            if !remote_application_paused {
                timing::record(
                    "remote_sharing_pause",
                    Duration::ZERO,
                    format!("pause={pause}"),
                );
                remote_application_paused = true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
            next_refresh = Instant::now();
            state.last_report = None;
            continue;
        }
        if remote_application_paused {
            remote_application_paused = false;
            next_refresh = Instant::now();
            timer.reset_immediately();
            timing::record(
                "remote_sharing_resume",
                Duration::ZERO,
                format!("pause={pause}"),
            );
        }
        if pause == 3 || (pause != 0 && state.outbound.is_none() && state.inbound.is_none()) {
            if pause == 1 || pause == 3 {
                state.reclaim(&runtime, Some(state.anchor)).await;
                let _ = state.capture.stop();
                state.capture_started = false;
                let _ = runtime
                    .send(RuntimeCommand::SetInputAvailability {
                        available: false,
                        reason: Some("正在安装更新".into()),
                    })
                    .await;
                update_pause.store(2, Ordering::Release);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
            next_refresh = Instant::now();
            state.last_report = None;
            continue;
        }
        let capture_ready = state.capture.event_notification();
        let injection_fault = state
            .injector
            .as_ref()
            .map(|injector| injector.error_notification());
        tokio::select! {
            biased;
            event = events.next() => {
                let Some(event)=event else {break;};
                match event {
                    RuntimeEvent::InputDeliveryInvalidated {reason} => {
                        state.error(&ui,reason.clone());
                        state.native_failed(&runtime,reason).await;
                        next_refresh=Instant::now();
                    }
                    RuntimeEvent::InputSharingSettingsChanged(_) => {
                        // Also emitted when settings validation/persistence fails:
                        // re-evaluate the native gate instead of retaining a cached true.
                        state.reclaim(&runtime,Some(state.anchor)).await;
                        state.available=false;
                        state.last_report=None;
                        next_refresh=Instant::now();
                    }
                    RuntimeEvent::RefreshNativeObserver => {
                        // User requested immediate permission check (e.g., clicked refresh button)
                        if let Some(thread) = &state.observer_thread {
                            thread.unpark();
                        }
                        next_refresh=Instant::now();
                    }
                    RuntimeEvent::SharedInputReceived {session_id,target_display,state_epoch,events,..} => {
                        let receive_arrived = Instant::now();
                        // Runtime has already checked token, role, transport identity,
                        // topology generation, sequence and lease before this event.
                        if !observations.borrow().as_ref().is_some_and(NativeObservation::permits_input) || state.retired.contains(&session_id) || !state.available || !state.effective.enabled || state.effective.role!=InputShareRole::Controlled {continue;}
                        let live=runtime.input_snapshot();
                        if !native_report_is_current(&state.snapshot,&live) || !live.effective_settings().enabled {continue;}
                        let Some(link)=live.links.iter().find(|l| l.control_session==Some(session_id) && l.ready) else {continue;};
                        let Some(display)=live.local_topology.as_ref().and_then(|t|t.displays.iter().find(|d| d.id==target_display)) else {continue;};
                        if state.inbound.as_ref().is_none_or(|p|p.session!=session_id) {
                            let entry_started = Instant::now();
                            timing::record!("receive_entry", Duration::ZERO, format!("session={session_id:?} epoch={state_epoch:?} motion_mode={:?}", link.motion_mode));
                            let now=Instant::now();
                            let progress=state.grants.entry(session_id).or_insert(LeaseProgress{session:session_id,revision:link.lease_revision,advanced_at:now,advanced_wall:SystemTime::now()});
                            if progress.expired(now) {state.retired.insert(session_id);let _=runtime.send(RuntimeCommand::ReleaseInputControl{session_id}).await;continue;}
                            progress.observe(session_id,link.lease_revision,now);
                            if let Some(injector)=&state.injector {
                                let _=injector.release_all_nowait();
                                let armed = injector.arm_session(session_id,progress.remaining(Instant::now()));
                                timing::record!("receive_native_arm", entry_started.elapsed(), format!("session={session_id:?} ok={armed}"));
                                if !armed {state.retired.insert(session_id);let _=runtime.send(RuntimeCommand::ReleaseInputControl{session_id}).await;continue;}
                            }
                            state.inbound=Some(progress.clone());
                        }
                        if state.inbound.as_ref().is_some_and(|p|p.expired(Instant::now())) || state.injector.as_ref().is_none_or(|i|i.lease_expired()) {
                            state.retired.insert(session_id);state.reclaim(&runtime,Some(state.anchor)).await;continue;
                        }
                        let precheck_time = receive_arrived.elapsed();
                        if timing::enabled() {
                            let inbound_motion = events
                                .iter()
                                .filter(|event| matches!(event, SharedInputEvent::Motion { .. }))
                                .count() as u64;
                            if inbound_motion > 0 {
                                state.in_motion.observe(inbound_motion, Instant::now());
                            }
                        }
                        if let Some(injector)=&state.injector {
                            let enqueue_start = Instant::now();
                            let error=injector.enqueue_batch(session_id,display,state_epoch,&events).err();
                            let enqueue_time = enqueue_start.elapsed();
                            if timing::enabled() && (precheck_time >= Duration::from_millis(2) || enqueue_time >= Duration::from_millis(2)) {
                                timing::record!("receive_slow_path", receive_arrived.elapsed(), format!("session={session_id:?} precheck={}us enqueue={}us events={}", precheck_time.as_micros(), enqueue_time.as_micros(), events.len()));
                            }
                            if let Some(error)=error {state.error(&ui,error.to_string());state.native_failed(&runtime,error.to_string()).await;next_refresh=Instant::now();}
                        }
                    }
                    RuntimeEvent::InputControlReleased {session_id,reason,..} => {
                        if !reason.contains("left display") { state.retired.insert(session_id); }
                        if state.outbound.as_ref().is_some_and(|p|p.session==session_id) { state.outbound=None; state.policy.reclaim(); let _=ui.unbounded_send(UiEvent::InputLockStatus(false)); let _=state.capture.exit_remote(Some(state.anchor)); }
                        if state.inbound.as_ref().is_some_and(|p|p.session==session_id) { state.inbound=None; if let Some(injector)=&state.injector {let _=injector.release_all_nowait();} }
                    }
                    RuntimeEvent::InputLinkUnavailable {peer_id,..} => {
                        let active=[state.outbound.as_ref(),state.inbound.as_ref()].into_iter().flatten().any(|lease|state.snapshot.links.iter().any(|link|link.peer_id==peer_id && link.control_session==Some(lease.session)));
                        if active {state.reclaim(&runtime,Some(state.anchor)).await;}
                    }
                    RuntimeEvent::InputReceived {..} => {state.error(&ui,"已拒绝旧版未授权输入协议".into());}
                    _=>{}
                }
            }
            changed = observations.changed(), if observer_open => {
                observer_open = changed.is_ok();
                next_refresh = Instant::now();
                timer.reset_immediately();
            }
            _ = async { match &injection_fault {
                Some(signal) => signal.notified().await,
                None => std::future::pending::<()>().await,
            }} => {
                if let Some(error) = state.injector.as_ref().and_then(|injector| injector.take_error()) {
                    state.error(&ui, error.to_string());
                    state.native_failed(&runtime, error.to_string()).await;
                    next_refresh = Instant::now();
                }
            }
            _ = capture_ready.notified() => {
                // Do not route a pending sample after a user pause/emergency or
                // while the observer is stale. Maintenance applies the new gate.
                if emergency.load(Ordering::Acquire)
                    || !state.available || !state.effective.enabled
                    || !state.capture_started
                    || !observations.borrow().as_ref().is_some_and(NativeObservation::permits_input)
                {
                    timer.reset_immediately();
                    continue;
                }
                let live = runtime.input_snapshot();
                if !native_report_is_current(&state.snapshot, &live)
                    || effective_native_settings(&live, &state.paused_activations) != state.effective
                {
                    timer.reset_immediately();
                    continue;
                }
                for _ in 0..256 {
                    match state.capture.next_event() {
                        Ok(Some(CaptureEvent::Input(sample))) => {
                            state.last_capture_at = Some(sample.captured_at);
                            if timing::enabled() {
                                let age = sample.captured_at.elapsed();
                                if age >= Duration::from_millis(16)
                                    && state.last_slow_capture.is_none_or(|last| last.elapsed() >= Duration::from_secs(1))
                                {
                                    state.last_slow_capture = Some(Instant::now());
                                    timing::record!("capture_queue_slow", age, format!("remote={}", state.policy.is_remote()));
                                }
                            }
                            if !state.policy.is_remote() { state.anchor=(sample.global_x,sample.global_y); }
                            if (recording_active || state.effective.role != InputShareRole::Controller)
                                && !matches!(&sample.event, SharedInputEvent::Key {code,pressed:true,modifiers,..} if HotkeyBindings::is_emergency(*code,*modifiers)) {continue;}
                            let global = Point::new(sample.global_x, sample.global_y);
                            let actions = state.policy.handle_input(sample.event, global, Vector2::new(sample.delta_x,sample.delta_y), started.elapsed().as_millis() as u64);
                            // Count exactly the positions this machine hands to the
                            // peer, so the peer's inbound summary can be compared
                            // against it one-for-one.
                            if timing::enabled() {
                                let forwarded = actions
                                    .iter()
                                    .filter(|action| {
                                        matches!(
                                            action,
                                            SharingAction::Route {
                                                event: SharedInputEvent::Motion { .. },
                                                ..
                                            }
                                        )
                                    })
                                    .count() as u64;
                                if forwarded > 0 {
                                    state.out_motion.observe(forwarded, Instant::now());
                                }
                            }
                            state.actions(actions,&runtime,&ui).await;
                            if !state.available || emergency.load(Ordering::Acquire) { break; }
                        }
                        Ok(None) => break,
                        Ok(Some(_)) | Err(_) => {
                            let reason = "输入捕获中断，已恢复本机输入".to_owned();
                            state.error(&ui,reason.clone());
                            state.native_failed(&runtime,reason).await;
                            next_refresh=Instant::now();
                            break;
                        }
                    }
                }
            }
            _ = timer.tick() => {
                let now = Instant::now();
                let mut snapshot = runtime.input_snapshot();
                let effective = effective_native_settings(&snapshot, &state.paused_activations);
                if latch_emergency(&emergency, &snapshot, &mut state.paused_activations) {
                    state.snapshot = snapshot.clone();
                    state.pause_managed(&runtime).await;
                    state.reclaim(&runtime,Some(state.anchor)).await;
                    let _ = state.capture.stop(); state.capture_started=false; state.injector=None;
                    next_refresh=now;
                    continue;
                }
                let recording_requested=recording.load(Ordering::Acquire);
                if recording_requested != recording_active {
                    recording_active=recording_requested;
                    // Recording consumes shortcuts in the app, not the network
                    // keyboard. Keep inbound injection and its lease alive.
                    if recording_active && state.policy.is_remote() {
                        state.reclaim(&runtime, Some(state.anchor)).await;
                    }
                    state.capture.set_controller_hotkeys_enabled(!recording_active && effective.role==InputShareRole::Controller);
                    let _=ui.unbounded_send(UiEvent::InputRecordingReady(recording_active));
                    next_refresh=now;
                }
                for progress in state.grants.values() { if progress.expired(now) { state.retired.insert(progress.session); } }
                if let Some(progress)=&state.outbound && (progress.expired(now)||state.capture.lease_expired()) {state.retired.insert(progress.session);}
                if let Some(progress)=&state.inbound && (progress.expired(now)||state.injector.as_ref().is_none_or(|i|i.lease_expired())) {state.retired.insert(progress.session);}
                if [state.outbound.as_ref(),state.inbound.as_ref()].into_iter().flatten().any(|p|state.retired.contains(&p.session)) {state.error(&ui,"控制租约已过期，已恢复本机输入".into());state.reclaim(&runtime,Some(state.anchor)).await;}
                let configuration_changed = native_configuration_changed(&state.snapshot, &snapshot) || !native_settings_equal(&state.effective, &effective);
                if configuration_changed {
                    state.reset_adapters(&runtime).await;
                    next_refresh=now;
                }
                if now >= next_refresh {
                    next_refresh = now + Duration::from_millis(250);
                    let observation = observations.borrow_and_update().clone().filter(NativeObservation::fresh);
                    let permission = observation.as_ref().map(|sample| sample.permission).unwrap_or_default();
                    let session_active = observation.as_ref().is_some_and(|sample| sample.session_active);
                    let (initial_phase, _) = native_phase(permission,session_active,recording_active,false,None);
                    let permitted = initial_phase == InputActivationStatus::Preparing;
                    let mut adapter_error = None;
                    match observation.map(|sample| sample.topology)
                        .unwrap_or_else(|| Err("原生输入状态检查未完成或已超时".into())) {
                        Ok(topology) => {
                            if snapshot.local_topology.as_ref() != Some(&topology) {
                                state.reclaim(&runtime, Some(state.anchor)).await;
                                if let Err(error) = runtime.send(RuntimeCommand::PublishDisplayTopology(topology)).await { adapter_error=Some(error.to_string()); }
                                let refreshed = runtime.input_snapshot();
                                if !native_report_is_current(&snapshot, &refreshed) { next_refresh=now; continue; }
                                snapshot=refreshed;
                                // The same activation was verified above; retain any local emergency pause overlay.
                            }
                        }
                        Err(error) => adapter_error=Some(error.to_string()),
                    }
                    if effective.enabled && permitted && adapter_error.is_none() {
                        if !state.capture_started {
                            match state.capture.start() {
                                Ok(()) => {
                                    state.capture_started=true;
                                    // A permission grant or native failure can restart
                                    // capture without changing the saved settings.
                                    // Every new tap needs its role and custom hotkeys.
                                    state.capture.set_hotkeys(hotkey_bindings(&effective));
                                    state.capture.set_controller_hotkeys_enabled(!recording_active && effective.role==InputShareRole::Controller);
                                }
                                Err(error)=>adapter_error=Some(error.to_string())
                            }
                        }
                        if state.injector.is_none() && adapter_error.is_none() {
                            match crate::platform::create_injector() {Ok(injector)=>state.injector=Some(injector),Err(error)=>adapter_error=Some(error.to_string())}
                        }
                    }
                    if let Some(error)=&adapter_error {state.error(&ui,error.clone());}
                    let adapters_ready=native_available(&effective,permitted,state.capture_started,state.injector.is_some());
                    let (status, reason) = if effective.enabled {
                        native_phase(permission,session_active,recording_active,adapters_ready,adapter_error)
                    } else {
                        (InputActivationStatus::Unavailable,Some("本机主控未启用，等待设备网络中的主控".into()))
                    };
                    state.report_native(&runtime,&snapshot,status,reason).await;
                    if recording_active {let _=ui.unbounded_send(UiEvent::InputRecordingReady(true));}
                    if previous_ui_snapshot.as_ref() != Some(&snapshot) {
                        previous_ui_snapshot = Some(snapshot.clone());
                        let _ = ui.unbounded_send(UiEvent::InputSharingSnapshot(Box::new(snapshot.clone())));
                    }
                    if previous_permission != Some(permission) {
                        let _=ui.unbounded_send(UiEvent::PermissionSnapshot(permission));
                        previous_permission=Some(permission);
                        // Permission changed: immediately check again to avoid waiting for the next 100ms tick
                        if let Some(thread) = &state.observer_thread {
                            thread.unpark();
                        }
                    }
                    let _ = ui.unbounded_send(UiEvent::InputLockStatus(state.policy.is_locked()));
                }
                state.grants.retain(|session,_| snapshot.links.iter().any(|l|l.control_session==Some(*session)));
                for link in &mut snapshot.links {
                    if let Some(session)=link.control_session {
                        let progress=state.grants.entry(session).or_insert(LeaseProgress{session,revision:link.lease_revision,advanced_at:now,advanced_wall:SystemTime::now()});
                        if !state.retired.contains(&session) {progress.observe(session,link.lease_revision,now);}
                        if state.retired.contains(&session) || progress.expired(now) {link.ready=false;link.reason=Some("控制租约未续期".into());}
                    }
                }
                let enabled = effective.enabled && state.available;
                let mut native_snapshot = snapshot.clone();
                native_snapshot.settings = effective.clone();
                let layout = layout_displays(&native_snapshot,local,true);
                if configuration_changed || layout != state.layout || (!enabled && (state.capture_started || state.outbound.is_some() || state.inbound.is_some())) {
                    if configuration_changed || !enabled || active_layout_changed(&state.layout,&layout,state.policy.active_display()) {
                        state.reclaim(&runtime, Some(state.anchor)).await;
                    }
                    if !enabled && state.capture_started { let _ = state.capture.stop(); state.capture_started=false; }
                    if !layout.is_empty() && let Err(error)=state.policy.set_layout(layout.clone()) {state.error(&ui,error);}
                    if configuration_changed {
                    let bindings=hotkey_bindings(&effective);
                    state.policy.set_mode(effective.switch_mode);
                    state.policy.set_hotkeys(bindings.clone());
                    state.policy.set_disabled_corners(effective.disabled_corners, f64::from(effective.corner_size));
                    state.capture.set_hotkeys(bindings);
                    state.capture.set_controller_hotkeys_enabled(!recording_active && effective.role==InputShareRole::Controller);
                    }
                    state.layout=layout;
                }
                state.effective=effective;
                state.snapshot=snapshot;
                let mut expired=false;
                for (lease, outbound) in [(&mut state.outbound,true),(&mut state.inbound,false)] {
                    if let Some(progress)=lease {
                        if let Some(link)=state.snapshot.links.iter().find(|l|l.control_session==Some(progress.session) && l.ready) {
                            if progress.expired(now) || state.retired.contains(&progress.session) {expired=true;}
                            else if progress.observe(progress.session,link.lease_revision,now) {
                                if outbound {expired|=!state.capture.renew_deadline(LEASE_TTL);} else if let Some(injector)=&state.injector {expired|=!injector.renew_deadline(LEASE_TTL);}
                            }
                        } else {expired=true;}
                    }
                }
                if expired {state.error(&ui,"控制租约失效，已恢复本机输入".into());state.reclaim(&runtime,Some(state.anchor)).await;}
                // Once-per-second cadence summary: the delivered update rate and its
                // tail is what smoothness actually is, and these two records are the
                // only place it is observable from an exported diagnostics file.
                if !timing::enabled() {
                    // Discard partial windows, never report the disabled interval as a stall.
                    state.out_motion = MotionCadence::default();
                    state.in_motion = MotionCadence::default();
                    state.cadence_at = now;
                } else if now.saturating_duration_since(state.cadence_at) >= CADENCE_WINDOW {
                    state.cadence_at = now;
                    if let Some((count, p50, p99, max)) = state.out_motion.take() {
                        timing::record!(
                            "motion_out_summary",
                            Duration::ZERO,
                            format!("count={count}/s gap_p50_ms={p50:.1} gap_p99_ms={p99:.1} gap_max_ms={max:.1}"),
                        );
                    }
                    if let Some((count, p50, p99, max)) = state.in_motion.take() {
                        timing::record!(
                            "motion_in_summary",
                            Duration::ZERO,
                            format!("count={count}/s gap_p50_ms={p50:.1} gap_p99_ms={p99:.1} gap_max_ms={max:.1}"),
                        );
                    }
                }
                if enabled && state.capture_started {
                    let actions=state.policy.tick(started.elapsed().as_millis() as u64);
                    state.actions(actions,&runtime,&ui).await;
                }
            }

        }
    }
    stopping.store(true, Ordering::Release);
    state.reclaim(&runtime, Some(state.anchor)).await;
    let _ = state.capture.stop();
    finish_native_observer(observer);
    let _ = runtime
        .send(RuntimeCommand::SetInputAvailability {
            available: false,
            reason: Some("本机输入协调器已停止".into()),
        })
        .await;
}

pub(crate) fn helper_permissions(
    native: &unidesk_runtime::input_service::NativeStatus,
    full_disk_access: PermissionStatus,
) -> PermissionSnapshot {
    fn decode(value: Option<u8>) -> PermissionStatus {
        match value {
            Some(1) => PermissionStatus::Granted,
            Some(2) => PermissionStatus::Denied,
            Some(3) => PermissionStatus::NeedsUserAction,
            _ => PermissionStatus::Unknown,
        }
    }
    PermissionSnapshot {
        accessibility: decode(native.accessibility),
        input_monitoring: decode(native.input_monitoring),
        full_disk_access,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shutdown_does_not_join_a_blocked_native_observer() {
        let (release, query) = std::sync::mpsc::channel();
        let observer = thread::spawn(move || {
            let _ = query.recv();
        });
        let (done, completed) = std::sync::mpsc::channel();
        let shutdown = thread::spawn(move || {
            finish_native_observer(Some(observer));
            let _ = done.send(());
        });
        let result = completed.recv_timeout(Duration::from_secs(1));
        // Always unblock the test worker, even if shutdown regresses to join.
        let _ = release.send(());
        shutdown.join().unwrap();
        assert!(result.is_ok());
    }
    #[test]
    fn stale_or_sleep_spanning_native_observation_cannot_enable_input() {
        let mut observation = NativeObservation {
            permission: PermissionSnapshot::all_unknown(),
            session_active: false,
            topology: Err("not sampled".into()),
            started_at: Instant::now(),
            started_wall: SystemTime::now(),
        };
        assert!(observation.fresh());
        assert!(!observation.permits_input());
        observation.started_wall = SystemTime::now() - Duration::from_secs(2);
        assert!(!observation.fresh());
        observation.started_wall = SystemTime::now();
        observation.started_at = Instant::now() - Duration::from_secs(2);
        assert!(!observation.fresh());
    }
    fn managed_snapshot(status: InputActivationStatus) -> InputSharingSnapshot {
        use unidesk_core::{
            ConnectionContext, InterfaceKind, LinkScope, ManagedInputControl, TransportSessionId,
        };
        InputSharingSnapshot {
            managed_control: Some(ManagedInputControl {
                activation_id: InputActivationId::new(),
                controller: device(9),
                context: ConnectionContext {
                    session_id: TransportSessionId::new(),
                    local_addr: "192.0.2.1:42424".parse().unwrap(),
                    remote_addr: "192.0.2.2:42424".parse().unwrap(),
                    interface_name: Some("en0".into()),
                    interface_kind: InterfaceKind::WiFi,
                    scope: LinkScope::Lan,
                },
                status,
                reason: None,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn managed_identity_changes_rebuild_adapters_but_status_updates_do_not() {
        let observed = managed_snapshot(InputActivationStatus::Preparing);
        let persisted = observed.settings.clone();
        let effective = observed.effective_settings();
        assert!(effective.enabled);
        assert_eq!(effective.role, InputShareRole::Controlled);
        assert_eq!(observed.settings, persisted);
        assert!(!observed.settings.enabled);
        let mut ready = observed.clone();
        ready.managed_control.as_mut().unwrap().status = InputActivationStatus::Ready;
        ready.managed_control.as_mut().unwrap().reason = Some("native ready".into());
        assert!(!native_configuration_changed(&observed, &ready));
        assert!(native_report_is_current(&observed, &ready));
        let mut replaced = ready.clone();
        replaced.managed_control.as_mut().unwrap().activation_id = InputActivationId::new();
        assert!(native_configuration_changed(&ready, &replaced));
        assert!(
            !native_report_is_current(&ready, &replaced),
            "old adapter Ready cannot acknowledge a new activation"
        );
        replaced = ready.clone();
        replaced.managed_control.as_mut().unwrap().controller = device(8);
        assert!(native_configuration_changed(&ready, &replaced));
        assert!(!native_report_is_current(&ready, &replaced));
        replaced = ready.clone();
        replaced.managed_control = None;
        assert!(native_configuration_changed(&ready, &replaced));
    }

    #[test]
    fn stale_ui_emergency_intent_pauses_the_current_activation() {
        let old_ui = managed_snapshot(InputActivationStatus::Ready);
        let current = managed_snapshot(InputActivationStatus::Ready);
        let old_id = old_ui.managed_control.as_ref().unwrap().activation_id;
        let current_id = current.managed_control.as_ref().unwrap().activation_id;
        assert_ne!(old_id, current_id);
        // The UI conveys only the emergency intent, never its cached old_id.
        let requested = AtomicBool::new(true);
        let mut paused = BTreeSet::new();
        assert!(latch_emergency(&requested, &current, &mut paused));
        assert!(paused.contains(&current_id));
        assert!(!paused.contains(&old_id));
        assert!(!effective_native_settings(&current, &paused).enabled);
        assert!(!latch_emergency(&requested, &current, &mut paused));
    }

    #[test]
    fn emergency_pause_cannot_be_undone_by_an_old_ready_snapshot() {
        let snapshot = managed_snapshot(InputActivationStatus::Ready);
        let id = snapshot.managed_control.as_ref().unwrap().activation_id;
        let paused = BTreeSet::from([id]);
        assert!(!effective_native_settings(&snapshot, &paused).enabled);
        assert_eq!(
            effective_native_settings(&snapshot, &paused).role,
            InputShareRole::Disabled
        );
        let mut fresh = snapshot.clone();
        fresh.managed_control.as_mut().unwrap().activation_id = InputActivationId::new();
        assert!(effective_native_settings(&fresh, &paused).enabled);
        assert_eq!(fresh.settings, snapshot.settings);
    }

    #[test]
    fn managed_native_reports_permissions_and_locks_with_the_exact_activation() {
        use InputActivationStatus::*;
        let mut permission = PermissionSnapshot::all_unknown();
        assert_eq!(
            native_phase(permission, true, false, false, None).0,
            WaitingAccessibility
        );
        permission.accessibility = PermissionStatus::Granted;
        assert_eq!(
            native_phase(permission, false, false, false, None).0,
            LockedSession
        );
        assert_eq!(
            native_phase(permission, true, false, false, None).0,
            Preparing
        );
        assert_eq!(native_phase(permission, true, false, true, None).0, Ready);
        assert_eq!(native_phase(permission, true, true, true, None).0, Ready);
        assert_eq!(
            native_phase(permission, true, false, true, Some("tap failed".into())).0,
            Unavailable
        );
        let activation_id = InputActivationId::new();
        let report = NativeReport::Managed {
            activation_id,
            status: WaitingAccessibility,
            reason: Some("permission".into()),
        };
        assert!(
            matches!(report.command(),RuntimeCommand::SetManagedInputAvailability {activation_id:id,status:WaitingAccessibility,..} if id==activation_id)
        );
        assert_ne!(
            report,
            NativeReport::Managed {
                activation_id,
                status: LockedSession,
                reason: Some("session locked".into())
            }
        );
    }

    #[test]
    fn active_tap_does_not_require_input_monitoring_but_must_really_start() {
        use InputActivationStatus::*;
        for input_monitoring in [
            PermissionStatus::Denied,
            PermissionStatus::Unknown,
            PermissionStatus::NeedsUserAction,
            PermissionStatus::Granted,
        ] {
            let permission = PermissionSnapshot {
                accessibility: PermissionStatus::Granted,
                input_monitoring,
                ..PermissionSnapshot::all_unknown()
            };
            // AX authorizes trying the modifying tap. It does not claim that
            // native creation or injection has succeeded.
            assert_eq!(
                native_phase(permission, true, false, false, None).0,
                Preparing
            );
            assert_eq!(native_phase(permission, true, false, true, None).0, Ready);
            assert_eq!(
                native_phase(permission, true, false, false, Some("tap failed".into())),
                (Unavailable, Some("tap failed".into()))
            );
            let denied_ax = PermissionSnapshot {
                accessibility: PermissionStatus::NeedsUserAction,
                ..permission
            };
            assert_eq!(
                native_phase(denied_ax, true, false, true, None).0,
                WaitingAccessibility
            );
        }
    }

    #[test]
    fn native_eligibility_waits_for_both_adapters_and_enabled_role() {
        let mut settings = unidesk_core::InputSharingSettings::default();
        assert!(!native_available(&settings, true, true, true));
        settings.enabled = true;
        settings.role = InputShareRole::Controller;
        assert!(!native_available(&settings, true, false, true));
        assert!(!native_available(&settings, true, true, false));
        assert!(!native_available(&settings, false, true, true));
        assert!(native_available(&settings, true, true, true));
    }

    #[test]
    fn unchanged_lease_snapshot_never_extends_native_deadline() {
        let now = Instant::now();
        let session = InputSessionId::new();
        let mut progress = LeaseProgress {
            session,
            revision: 1,
            advanced_at: now,
            advanced_wall: SystemTime::now(),
        };
        assert!(!progress.observe(session, 1, now + Duration::from_millis(1000)));
        assert!(progress.expired(now + LEASE_TTL));
        assert!(!progress.observe(session, 2, now + LEASE_TTL));
        assert!(progress.expired(now + LEASE_TTL));
    }
    #[test]
    fn sleep_expiry_is_checked_before_new_revision_can_refresh_progress() {
        let now = Instant::now();
        let session = InputSessionId::new();
        let mut progress = LeaseProgress {
            session,
            revision: 1,
            advanced_at: now,
            advanced_wall: SystemTime::now() - LEASE_TTL,
        };
        assert!(progress.expired(now));
        assert!(!progress.observe(session, 8, now));
        assert_eq!(progress.revision, 1);
        assert!(!progress.observe(InputSessionId::new(), 9, now));
    }
    #[test]
    fn unrelated_peer_readiness_does_not_move_or_reclaim_the_active_display() {
        use unidesk_core::{DisplayTopology, InputPeerStatus};
        let local = device(1);
        let mut peers = [device(2), device(3)];
        peers.sort();
        let [before, active] = peers;
        let mut snapshot = InputSharingSnapshot {
            local_topology: Some(DisplayTopology {
                generation: 1,
                displays: vec![display("local", 0.0, 0.0, 100)],
            }),
            ..Default::default()
        };
        snapshot.settings.participants = peers.to_vec();
        for peer in peers {
            snapshot.remote_topologies.insert(
                peer,
                DisplayTopology {
                    generation: 1,
                    displays: vec![display("remote", 0.0, 0.0, 100)],
                },
            );
            snapshot.links.push(InputPeerStatus {
                motion_mode: None,
                motion_fallback_reason: None,
                peer_id: peer,
                context: None,
                ready: true,
                reason: None,
                control_session: Some(InputSessionId::new()),
                lease_revision: 1,
            });
        }
        let previous = layout_displays(&snapshot, local, true);
        let key = DisplayKey {
            device_id: active,
            display_id: "remote".into(),
        };
        snapshot
            .links
            .iter_mut()
            .find(|l| l.peer_id == before)
            .unwrap()
            .ready = false;
        let next = layout_displays(&snapshot, local, true);
        assert_eq!(previous.iter().find(|d| d.key == key).unwrap().x, 200.0);
        assert_eq!(next.iter().find(|d| d.key == key).unwrap().x, 200.0);
        assert!(!active_layout_changed(&previous, &next, Some(&key)));
        snapshot
            .links
            .iter_mut()
            .find(|l| l.peer_id == active)
            .unwrap()
            .ready = false;
        assert!(active_layout_changed(
            &next,
            &layout_displays(&snapshot, local, true),
            Some(&key)
        ));
    }
    fn device(seed: u8) -> DeviceId {
        DeviceId::from_public_key([seed; 32]).expect("device")
    }
    fn display(id: &str, x: f64, y: f64, width: u32) -> unidesk_core::DisplayDescriptor {
        unidesk_core::DisplayDescriptor {
            id: id.into(),
            frame_x: x,
            frame_y: y,
            width,
            height: 100,
            scale_factor: 2.0,
            rotation_degrees: 0,
            is_primary: x == 0.0,
        }
    }
    #[test]
    fn group_layout_keeps_native_negative_offsets_and_excludes_ungranted_peers() {
        use unidesk_core::{DisplayPlacementSetting, DisplayTopology, InputPeerStatus};
        let local = device(1);
        let peer = device(2);
        let mut snapshot = InputSharingSnapshot {
            local_topology: Some(DisplayTopology {
                generation: 1,
                displays: vec![
                    display("local", -100.0, 20.0, 100),
                    display("secondary", 0.0, 20.0, 200),
                ],
            }),
            ..Default::default()
        };
        snapshot.remote_topologies.insert(
            peer,
            DisplayTopology {
                generation: 2,
                displays: vec![display("remote", 0.0, 0.0, 100)],
            },
        );
        snapshot.settings.participants.push(peer);
        snapshot.settings.placements.push(DisplayPlacementSetting {
            display: DisplayKey {
                device_id: local,
                display_id: "local".into(),
            },
            x: 500.0,
            y: -200.0,
        });
        let all = layout_displays(&snapshot, local, false);
        assert_eq!((all[0].x, all[0].y), (500.0, -200.0));
        assert_eq!((all[1].x, all[1].y), (600.0, -200.0));
        assert_eq!(layout_displays(&snapshot, local, true).len(), 2);
        snapshot.links.push(InputPeerStatus {
            motion_mode: None,
            motion_fallback_reason: None,
            peer_id: peer,
            context: None,
            ready: true,
            reason: None,
            control_session: Some(InputSessionId::new()),
            lease_revision: 1,
        });
        assert_eq!(layout_displays(&snapshot, local, true).len(), 3);
    }
    #[test]
    fn native_coordinates_still_select_a_translated_local_screen() {
        let local = device(3);
        let peer = device(4);
        let mut policy = SharingController::new(local);
        policy
            .set_layout(vec![
                SharingDisplay {
                    key: DisplayKey {
                        device_id: local,
                        display_id: "left".into(),
                    },
                    display: display("left", -100.0, 20.0, 100),
                    x: 500.0,
                    y: -200.0,
                    ready: true,
                },
                SharingDisplay {
                    key: DisplayKey {
                        device_id: peer,
                        display_id: "right".into(),
                    },
                    display: display("right", 0.0, 0.0, 100),
                    x: 600.0,
                    y: -200.0,
                    ready: true,
                },
            ])
            .unwrap();
        policy.handle_input(
            SharedInputEvent::Motion { x: -50.0, y: 60.0 },
            Point::new(-50.0, 60.0),
            Vector2::new(1.0, 0.0),
            0,
        );
        assert_eq!(policy.active_display().unwrap().device_id, local);
        let actions = policy.handle_input(
            SharedInputEvent::Motion { x: -1.0, y: 60.0 },
            Point::new(-1.0, 60.0),
            Vector2::new(10.0, 0.0),
            1,
        );
        assert!(
            actions
                .iter()
                .any(|a| matches!(a,SharingAction::Crossed{to,..} if to.device_id==peer))
        );
        policy.reclaim();
        assert!(!policy.is_remote());
        assert!(!policy.is_locked());
    }
    #[test]
    fn no_topology_means_no_fabricated_display() {
        let local = DeviceId::from_public_key([1; 32]).expect("device");
        assert!(layout_displays(&InputSharingSnapshot::default(), local, false).is_empty());
    }

    #[test]
    fn motion_cadence_summarises_rate_and_gap_tail() {
        let mut cadence = MotionCadence::default();
        assert!(cadence.take().is_none(), "an idle window must not emit");
        let start = Instant::now();
        for index in 0..10u32 {
            cadence.observe(1, start + Duration::from_millis(u64::from(index) * 8));
        }
        // One window: 10 samples over eight-millisecond spacing.
        let (count, p50, p99, max) = cadence.take().expect("window");
        assert_eq!(count, 10);
        assert!((p50 - 8.0).abs() < 0.01, "p50 {p50}");
        assert!((p99 - 8.0).abs() < 0.01, "p99 {p99}");
        assert!((max - 8.0).abs() < 0.01, "max {max}");
        assert!(cadence.take().is_none(), "take() must reset the window");
        // A stalled sample must show up in the tail, not the median.
        cadence.observe(1, start);
        cadence.observe(1, start + Duration::from_millis(8));
        cadence.observe(1, start + Duration::from_millis(108));
        let (count, p50, p99, max) = cadence.take().expect("window");
        assert_eq!(count, 3);
        assert!((p50 - 8.0).abs() < 0.01, "p50 {p50}");
        assert!(p99 > 90.0, "p99 {p99}");
        assert!((max - 100.0).abs() < 0.01, "max {max}");
    }
}
