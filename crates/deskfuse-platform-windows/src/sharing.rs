//! Shared-engine adapters for the ordinary Windows Default desktop.
//!
//! The current network contract still carries legacy Quartz physical key codes.
//! Conversion is contained here so Windows native capture/injection remains HID-
//! based and Mac-to-Mac behavior is unchanged. Service/session helpers remain a
//! separate boundary; these adapters never claim Winlogon or UAC desktop access.
#![allow(unsafe_code)]

use crate::{
    capture::{
        CaptureError, CaptureEvent as NativeCaptureEvent, CaptureFault, CapturePoll,
        WindowsInputCapture,
    },
    display::{NativeDisplay, native_displays},
    hid::{
        LegacyCapsLock, keyboard_usage_to_legacy_keycode, legacy_keycode_to_keyboard_usage,
        legacy_modifiers_to_hid,
    },
    input::{MouseButton, OrdinaryDesktopInjector, VirtualDesktopPixels},
};
use deskfuse_core::{DisplayDescriptor, InputSessionId, SharedInputEvent};
use deskfuse_input::sharing::HotkeyBindings;
use deskfuse_platform::{
    CaptureEvent, CaptureMode, CaptureSample, PlatformError, PlatformResult, SharedInputCapture,
    SharedInputInjector,
};
use std::{
    collections::BTreeSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime},
};
use tokio::sync::Notify;
use windows_sys::Win32::UI::WindowsAndMessaging::SetCursorPos;

const NATIVE_QUEUE: usize = 2_048;
const INJECTION_QUEUE: usize = 256;
const MAX_LEASE: Duration = Duration::from_millis(1_500);

#[derive(Default)]
struct LeaseState {
    current: Option<InputSessionId>,
    until: Option<(Instant, SystemTime)>,
    retired: BTreeSet<InputSessionId>,
}

#[derive(Default)]
struct SessionLease(Mutex<LeaseState>);
impl SessionLease {
    fn arm(&self, session: InputSessionId, duration: Duration) -> bool {
        let Ok(mut state) = self.0.lock() else {
            return false;
        };
        expire(&mut state);
        if duration.is_zero() || state.retired.contains(&session) {
            return false;
        }
        state.current = Some(session);
        state.until = Some(deadline(duration));
        true
    }
    fn renew(&self, duration: Duration) -> bool {
        let Ok(mut state) = self.0.lock() else {
            return false;
        };
        expire(&mut state);
        if duration.is_zero() || state.current.is_none() || state.until.is_none() {
            return false;
        }
        state.until = Some(deadline(duration));
        true
    }
    fn active(&self, session: InputSessionId) -> bool {
        let Ok(mut state) = self.0.lock() else {
            return false;
        };
        expire(&mut state);
        state.current == Some(session) && state.until.is_some()
    }
    fn remaining(&self) -> Option<Duration> {
        let Ok(mut state) = self.0.lock() else {
            return None;
        };
        expire(&mut state);
        let (monotonic, wall) = state.until?;
        Some(
            monotonic
                .saturating_duration_since(Instant::now())
                .min(wall.duration_since(SystemTime::now()).unwrap_or_default()),
        )
        .filter(|duration| !duration.is_zero())
    }
    fn expired(&self) -> bool {
        self.remaining().is_none()
    }
    fn pause(&self) {
        if let Ok(mut state) = self.0.lock() {
            expire(&mut state);
            state.until = None;
        }
    }
    fn retire(&self) {
        if let Ok(mut state) = self.0.lock() {
            if let Some(current) = state.current {
                state.retired.insert(current);
            }
            state.current = None;
            state.until = None;
        }
    }
}

fn deadline(duration: Duration) -> (Instant, SystemTime) {
    let duration = duration.min(MAX_LEASE);
    (Instant::now() + duration, SystemTime::now() + duration)
}

fn expire(state: &mut LeaseState) {
    if state
        .until
        .is_some_and(|(monotonic, wall)| Instant::now() >= monotonic || SystemTime::now() >= wall)
    {
        if let Some(current) = state.current {
            state.retired.insert(current);
        }
        state.until = None;
    }
}

pub struct WindowsSharingCapture {
    native: Option<WindowsInputCapture>,
    notification: Arc<Notify>,
    lease: SessionLease,
    last_position: Option<(f64, f64)>,
    caps_lock: LegacyCapsLock,
    /// Cached adapter settings applied to every native capture instance:
    /// `set_hotkeys`/`set_controller_hotkeys_enabled` can run before `start`.
    hotkeys: Mutex<HotkeyBindings>,
    controller_hotkeys: AtomicBool,
}
impl WindowsSharingCapture {
    #[must_use]
    pub fn new() -> Self {
        Self {
            native: None,
            notification: Arc::new(Notify::new()),
            lease: SessionLease::default(),
            last_position: None,
            caps_lock: LegacyCapsLock::default(),
            hotkeys: Mutex::new(HotkeyBindings::default()),
            controller_hotkeys: AtomicBool::new(false),
        }
    }
    fn apply_hotkey_settings(&self, native: &WindowsInputCapture) {
        if let Ok(hotkeys) = self.hotkeys.lock() {
            native.set_hotkeys(hotkeys.clone());
        }
        native.set_controller_hotkeys_enabled(self.controller_hotkeys.load(Ordering::Acquire));
    }
}
impl Default for WindowsSharingCapture {
    fn default() -> Self {
        Self::new()
    }
}
impl SharedInputCapture for WindowsSharingCapture {
    fn start(&mut self) -> PlatformResult<()> {
        if self.native.is_some() {
            return Ok(());
        }
        let native = WindowsInputCapture::start_with_notification(
            NATIVE_QUEUE,
            Arc::clone(&self.notification),
        )
        .map_err(|_| PlatformError::InputCaptureUnavailable)?;
        self.apply_hotkey_settings(&native);
        self.caps_lock = LegacyCapsLock::new(native.initial_caps_lock());
        self.native = Some(native);
        Ok(())
    }
    fn stop(&mut self) -> PlatformResult<()> {
        self.lease.retire();
        if let Some(mut native) = self.native.take() {
            native
                .stop()
                .map_err(|_| PlatformError::InputCaptureUnavailable)?;
        }
        self.last_position = None;
        Ok(())
    }
    fn set_mode(&self, _mode: CaptureMode) {}
    fn set_hotkeys(&self, hotkeys: HotkeyBindings) {
        if let Ok(mut guard) = self.hotkeys.lock() {
            *guard = hotkeys.clone();
        }
        if let Some(native) = &self.native {
            native.set_hotkeys(hotkeys);
        }
    }
    fn arm_session(&self, session: InputSessionId, duration: Duration) -> bool {
        self.lease.arm(session, duration)
    }
    fn renew_deadline(&self, duration: Duration) -> bool {
        if !self.lease.renew(duration) {
            return false;
        }
        self.native.as_ref().is_none_or(|native| {
            !native.status().suppressing || native.renew_suppression_lease(duration).is_ok()
        })
    }
    fn lease_expired(&self) -> bool {
        self.lease.expired()
            || self
                .native
                .as_ref()
                .is_some_and(|native| native.status().suppressing && !native.status().thread_alive)
    }
    fn set_controller_hotkeys_enabled(&self, enabled: bool) {
        self.controller_hotkeys.store(enabled, Ordering::Release);
        if let Some(native) = &self.native {
            native.set_controller_hotkeys_enabled(enabled);
        }
    }
    fn enter_remote(&self, _x: f64, _y: f64) -> PlatformResult<()> {
        let remaining = self
            .lease
            .remaining()
            .ok_or(PlatformError::InputCaptureUnavailable)?;
        let native = self
            .native
            .as_ref()
            .ok_or(PlatformError::InputCaptureUnavailable)?;
        match native.begin_suppression_lease(remaining) {
            Ok(()) => Ok(()),
            Err(CaptureError::SuppressionBlockedByHeldInput) => {
                // Directional hotkeys require held modifiers, and edge switches
                // can happen mid-drag: release what local applications already
                // saw (mirrors macOS `release_local_keyboard`), then the lease
                // can start without stranding those keys on the local machine.
                let held = native.held_physical_input();
                if held.usages.is_empty() && held.buttons == 0 {
                    return Err(PlatformError::InputCaptureUnavailable);
                }
                let mut injector = OrdinaryDesktopInjector::new()
                    .map_err(|_| PlatformError::InputCaptureUnavailable)?;
                injector
                    .release_local_physical(&held.usages, held.buttons)
                    .map_err(|_| PlatformError::InputCaptureUnavailable)?;
                native.clear_held_physical(&held);
                native
                    .begin_suppression_lease(remaining)
                    .map_err(|_| PlatformError::InputCaptureUnavailable)
            }
            Err(_) => Err(PlatformError::InputCaptureUnavailable),
        }
    }
    fn exit_remote(&self, restore: Option<(f64, f64)>) -> PlatformResult<()> {
        if let Some(native) = &self.native {
            native.stop_suppression();
        }
        self.lease.pause();
        if let Some((x, y)) = restore {
            warp_pointer(x, y)?;
        }
        Ok(())
    }
    fn event_notification(&self) -> Arc<Notify> {
        Arc::clone(&self.notification)
    }
    fn next_event(&mut self) -> PlatformResult<Option<CaptureEvent>> {
        let native = self
            .native
            .as_ref()
            .ok_or(PlatformError::InputCaptureUnavailable)?;
        let event = match native.poll() {
            CapturePoll::Idle => return Ok(None),
            CapturePoll::Stopped => CaptureEvent::TapDisabled,
            CapturePoll::Fault(CaptureFault::QueueOverflow) => CaptureEvent::Overflow,
            CapturePoll::Fault(_) => CaptureEvent::TapDisabled,
            CapturePoll::Event(event) => self.convert(event)?,
        };
        Ok(Some(event))
    }
}
impl WindowsSharingCapture {
    fn convert(&mut self, event: NativeCaptureEvent) -> PlatformResult<CaptureEvent> {
        let ((global_x, global_y), raw) = match event {
            NativeCaptureEvent::MouseMove { x, y, raw } => (physical_to_global(x, y)?, raw),
            NativeCaptureEvent::MouseButton { x, y, .. }
            | NativeCaptureEvent::Scroll { x, y, .. } => (physical_to_global(x, y)?, None),
            NativeCaptureEvent::Key { .. } | NativeCaptureEvent::Modifiers(_) => {
                (self.last_position.unwrap_or_default(), None)
            }
        };
        let previous = self.last_position.replace((global_x, global_y));
        let (delta_x, delta_y) = previous
            .map(|(x, y)| (global_x - x, global_y - y))
            .unwrap_or_default();
        let event = match event {
            NativeCaptureEvent::MouseMove { .. } => SharedInputEvent::Motion {
                x: global_x,
                y: global_y,
            },
            NativeCaptureEvent::MouseButton { button, down, .. } => SharedInputEvent::Button {
                button: button_number(button),
                pressed: down,
            },
            NativeCaptureEvent::Scroll {
                vertical,
                horizontal,
                ..
            } => SharedInputEvent::Scroll {
                dx: f64::from(horizontal),
                dy: f64::from(vertical),
            },
            NativeCaptureEvent::Key {
                usage,
                down,
                repeat,
                modifiers,
            } => {
                self.caps_lock.observe_key(usage, down, repeat);
                SharedInputEvent::Key {
                    code: keyboard_usage_to_legacy_keycode(usage)
                        .ok_or(PlatformError::InputCaptureUnavailable)?,
                    pressed: down,
                    modifiers: self.caps_lock.flags(modifiers),
                    repeat,
                }
            }
            NativeCaptureEvent::Modifiers(modifiers) => SharedInputEvent::Modifiers {
                modifiers: self.caps_lock.flags(modifiers),
            },
        };
        Ok(CaptureEvent::Input(CaptureSample {
            captured_at: Instant::now(),
            event,
            global_x,
            global_y,
            delta_x,
            delta_y,
            raw_dx: raw.map(|(dx, _)| f64::from(dx)).unwrap_or(0.0),
            raw_dy: raw.map(|(_, dy)| f64::from(dy)).unwrap_or(0.0),
            has_raw: raw.is_some(),
        }))
    }
}

fn button_number(button: MouseButton) -> u8 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Right => 1,
        MouseButton::Middle => 2,
        MouseButton::Back => 3,
        MouseButton::Forward => 4,
    }
}
fn native_button(button: u8) -> Option<MouseButton> {
    Some(match button {
        0 => MouseButton::Left,
        1 => MouseButton::Right,
        2 => MouseButton::Middle,
        3 => MouseButton::Back,
        4 => MouseButton::Forward,
        _ => return None,
    })
}

fn physical_to_global(x: i32, y: i32) -> PlatformResult<(f64, f64)> {
    for display in native_displays()? {
        if let Some((local_x, local_y)) = display.physical_to_local(f64::from(x), f64::from(y)) {
            return Ok((
                display.descriptor.frame_x + local_x,
                display.descriptor.frame_y + local_y,
            ));
        }
    }
    Err(PlatformError::DisplayUnavailable)
}

fn virtual_desktop(displays: &[NativeDisplay]) -> PlatformResult<VirtualDesktopPixels> {
    let left = displays
        .iter()
        .map(|display| display.physical_left)
        .min()
        .ok_or(PlatformError::DisplayUnavailable)?;
    let top = displays
        .iter()
        .map(|display| display.physical_top)
        .min()
        .ok_or(PlatformError::DisplayUnavailable)?;
    let right = displays
        .iter()
        .map(|display| i64::from(display.physical_left) + i64::from(display.physical_width))
        .max()
        .ok_or(PlatformError::DisplayUnavailable)?;
    let bottom = displays
        .iter()
        .map(|display| i64::from(display.physical_top) + i64::from(display.physical_height))
        .max()
        .ok_or(PlatformError::DisplayUnavailable)?;
    Ok(VirtualDesktopPixels {
        left,
        top,
        width: u32::try_from(right - i64::from(left))
            .map_err(|_| PlatformError::DisplayUnavailable)?,
        height: u32::try_from(bottom - i64::from(top))
            .map_err(|_| PlatformError::DisplayUnavailable)?,
    })
}

/// Reposition only on the ordinary Default desktop using current native topology.
#[allow(clippy::cast_possible_truncation)]
pub fn warp_pointer(x: f64, y: f64) -> PlatformResult<()> {
    let displays = native_displays()?;
    let native = displays
        .iter()
        .find(|native| {
            native
                .descriptor
                .contains(x - native.descriptor.frame_x, y - native.descriptor.frame_y)
        })
        .ok_or(PlatformError::DisplayUnavailable)?;
    let local_x = x - native.descriptor.frame_x;
    let local_y = y - native.descriptor.frame_y;
    let (physical_x, physical_y) = native
        .local_to_physical(local_x, local_y)
        .ok_or(PlatformError::DisplayUnavailable)?;
    // SAFETY: SetCursorPos consumes bounded scalar virtual-desktop coordinates.
    if unsafe { SetCursorPos(physical_x.round() as i32, physical_y.round() as i32) } == 0 {
        return Err(PlatformError::InputInjectionFailed);
    }
    Ok(())
}

pub fn session_is_active() -> bool {
    crate::session::session_capabilities().user_content_allowed
}

pub struct WindowsPermissionService;
impl deskfuse_platform::PermissionService for WindowsPermissionService {
    fn snapshot(&self) -> deskfuse_platform::PermissionSnapshot {
        let status = if session_is_active() {
            deskfuse_platform::PermissionStatus::Granted
        } else {
            deskfuse_platform::PermissionStatus::NeedsUserAction
        };
        deskfuse_platform::PermissionSnapshot::new(
            status,
            deskfuse_platform::PermissionStatus::Unknown,
            status,
        )
    }
    fn open_system_settings(&self, _kind: deskfuse_platform::PermissionKind) -> PlatformResult<()> {
        Err(PlatformError::Unsupported {
            operation: "Windows input service settings",
        })
    }
}

pub fn create_permission_service() -> Box<dyn deskfuse_platform::PermissionService> {
    Box::new(WindowsPermissionService)
}

struct InjectionError {
    value: Mutex<Option<PlatformError>>,
    notification: Arc<Notify>,
}
impl InjectionError {
    fn publish(&self, error: PlatformError) {
        if let Ok(mut value) = self.value.lock() {
            *value = Some(error);
        }
        self.notification.notify_one();
    }
}

enum InjectionCommand {
    Batch {
        session: InputSessionId,
        display: DisplayDescriptor,
        state_epoch: u64,
        events: Vec<SharedInputEvent>,
    },
    Release(Option<SyncSender<PlatformResult<()>>>),
    Stop,
}

pub struct WindowsSharingInjector {
    sender: SyncSender<InjectionCommand>,
    worker: Option<JoinHandle<()>>,
    lease: Arc<SessionLease>,
    error: Arc<InjectionError>,
}
impl WindowsSharingInjector {
    pub fn new() -> PlatformResult<Self> {
        let (sender, receiver) = sync_channel(INJECTION_QUEUE);
        let (ready_sender, ready_receiver) = sync_channel(1);
        let lease = Arc::new(SessionLease::default());
        let worker_lease = Arc::clone(&lease);
        let error = Arc::new(InjectionError {
            value: Mutex::new(None),
            notification: Arc::new(Notify::new()),
        });
        let worker_error = Arc::clone(&error);
        let worker = thread::Builder::new()
            .name("deskfuse-input-injector".into())
            .spawn(move || injection_worker(receiver, worker_lease, worker_error, ready_sender))
            .map_err(|_| PlatformError::InputInjectionFailed)?;
        match ready_receiver.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(Self {
                sender,
                worker: Some(worker),
                lease,
                error,
            }),
            _ => {
                let _ = worker.join();
                Err(PlatformError::InputInjectionFailed)
            }
        }
    }
}
impl SharedInputInjector for WindowsSharingInjector {
    fn arm_session(&self, session: InputSessionId, duration: Duration) -> bool {
        self.lease.arm(session, duration)
    }
    fn renew_deadline(&self, duration: Duration) -> bool {
        self.lease.renew(duration)
    }
    fn lease_expired(&self) -> bool {
        self.lease.expired()
    }
    fn enqueue_batch(
        &self,
        session: InputSessionId,
        display: &DisplayDescriptor,
        state_epoch: u64,
        events: &[SharedInputEvent],
    ) -> PlatformResult<()> {
        if !self.lease.active(session) || events.is_empty() {
            return Err(PlatformError::InputInjectionFailed);
        }
        self.sender
            .try_send(InjectionCommand::Batch {
                session,
                display: display.clone(),
                state_epoch,
                events: events.to_vec(),
            })
            .map_err(|error| {
                if matches!(error, TrySendError::Full(_)) {
                    self.lease.retire();
                }
                PlatformError::InputInjectionFailed
            })
    }
    fn error_notification(&self) -> Arc<Notify> {
        Arc::clone(&self.error.notification)
    }
    fn take_error(&self) -> Option<PlatformError> {
        self.error.value.lock().ok()?.take()
    }
    fn release_all(&self) -> PlatformResult<()> {
        let (sender, receiver) = sync_channel(1);
        self.sender
            .try_send(InjectionCommand::Release(Some(sender)))
            .map_err(|_| PlatformError::InputInjectionFailed)?;
        receiver
            .recv_timeout(Duration::from_millis(500))
            .map_err(|_| PlatformError::InputInjectionFailed)??;
        self.lease.pause();
        Ok(())
    }
    fn release_all_nowait(&self) -> PlatformResult<()> {
        self.sender
            .try_send(InjectionCommand::Release(None))
            .map_err(|_| PlatformError::InputInjectionFailed)?;
        self.lease.pause();
        Ok(())
    }
}
impl Drop for WindowsSharingInjector {
    fn drop(&mut self) {
        self.lease.retire();
        let _ = self.sender.send(InjectionCommand::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn injection_worker(
    receiver: Receiver<InjectionCommand>,
    lease: Arc<SessionLease>,
    error: Arc<InjectionError>,
    ready: SyncSender<PlatformResult<()>>,
) {
    let mut injector = match OrdinaryDesktopInjector::new() {
        Ok(injector) => {
            let _ = ready.send(Ok(()));
            injector
        }
        Err(_) => {
            let _ = ready.send(Err(PlatformError::InputInjectionFailed));
            return;
        }
    };
    let mut state_epoch = 0;
    let mut scroll_residual = (0.0, 0.0);
    let mut motion_residual = (0.0, 0.0);
    loop {
        let command = match receiver.recv_timeout(Duration::from_millis(20)) {
            Ok(command) => command,
            Err(RecvTimeoutError::Timeout) => {
                if lease.expired() && injector.held_input_count() > 0 {
                    let _ = injector.release_all();
                }
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let result = match command {
            InjectionCommand::Batch {
                session,
                display,
                state_epoch: incoming_epoch,
                events,
            } => {
                if !lease.active(session) {
                    let _ = injector.release_all();
                    Ok(())
                } else if incoming_epoch < state_epoch {
                    Err(PlatformError::InputInjectionFailed)
                } else {
                    state_epoch = incoming_epoch;
                    inject_batch(
                        &mut injector,
                        &lease,
                        session,
                        &display,
                        &events,
                        &mut scroll_residual,
                        &mut motion_residual,
                    )
                }
            }
            InjectionCommand::Release(reply) => {
                motion_residual = (0.0, 0.0);
                let result = injector
                    .release_all()
                    .map_err(|_| PlatformError::InputInjectionFailed);
                if let Some(reply) = reply {
                    let _ = reply.send(result.clone());
                }
                state_epoch = 0;
                result
            }
            InjectionCommand::Stop => {
                let _ = injector.release_all();
                break;
            }
        };
        if let Err(failure) = result {
            motion_residual = (0.0, 0.0);
            lease.retire();
            let _ = injector.release_all();
            error.publish(failure);
        }
    }
}

#[allow(clippy::cast_possible_truncation)]
fn inject_batch(
    injector: &mut OrdinaryDesktopInjector,
    lease: &SessionLease,
    session: InputSessionId,
    display: &DisplayDescriptor,
    events: &[SharedInputEvent],
    scroll_residual: &mut (f64, f64),
    motion_residual: &mut (f64, f64),
) -> PlatformResult<()> {
    let displays = native_displays()?;
    let native = displays
        .iter()
        .find(|native| native.descriptor.id == display.id)
        .ok_or(PlatformError::DisplayUnavailable)?;
    let desktop = virtual_desktop(&displays)?;
    for event in events {
        if !lease.active(session) {
            return Err(PlatformError::InputInjectionFailed);
        }
        event
            .validate()
            .map_err(|_| PlatformError::InputInjectionFailed)?;
        let started = Instant::now();
        match event {
            SharedInputEvent::Motion { x, y } => {
                let (physical_x, physical_y) = native
                    .local_to_physical(*x, *y)
                    .ok_or(PlatformError::InputInjectionFailed)?;
                injector
                    .move_absolute(
                        desktop,
                        physical_x.round() as i32,
                        physical_y.round() as i32,
                    )
                    .map_err(|_| PlatformError::InputInjectionFailed)?;
            }
            SharedInputEvent::Button { button, pressed } => {
                injector
                    .mouse_button(
                        native_button(*button).ok_or(PlatformError::InputInjectionFailed)?,
                        *pressed,
                    )
                    .map_err(|_| PlatformError::InputInjectionFailed)?;
            }
            SharedInputEvent::Key {
                code,
                pressed,
                modifiers,
                ..
            } => {
                injector
                    .set_modifiers(legacy_modifiers_to_hid(*modifiers))
                    .and_then(|()| {
                        legacy_keycode_to_keyboard_usage(*code)
                            .ok_or(crate::input::InputError::UnsupportedUsage(*code))
                            .and_then(|usage| injector.keyboard(usage, *pressed).map(|_| ()))
                    })
                    .map_err(|_| PlatformError::InputInjectionFailed)?;
            }
            SharedInputEvent::Modifiers { modifiers } => injector
                .set_modifiers(legacy_modifiers_to_hid(*modifiers))
                .map_err(|_| PlatformError::InputInjectionFailed)?,
            SharedInputEvent::RelativeMotion { dx, dy } => {
                let total_x = motion_residual.0 + dx;
                let total_y = motion_residual.1 + dy;
                let step_x = total_x.round();
                let step_y = total_y.round();
                let next = (total_x - step_x, total_y - step_y);
                let ix = step_x as i32;
                let iy = step_y as i32;
                if ix != 0 || iy != 0 {
                    injector
                        .move_relative(ix, iy)
                        .map_err(|_| PlatformError::InputInjectionFailed)?;
                }
                *motion_residual = next;
            }
            SharedInputEvent::Scroll { dx, dy } => {
                scroll_residual.0 += dx;
                scroll_residual.1 += dy;
                let horizontal = scroll_residual.0.round();
                let vertical = scroll_residual.1.round();
                scroll_residual.0 -= horizontal;
                scroll_residual.1 -= vertical;
                injector
                    .scroll(vertical as i32, horizontal as i32)
                    .map_err(|_| PlatformError::InputInjectionFailed)?;
            }
        }
        // 慢注入计数和阶段均值各记一份：均值会把少数几次长卡顿摊平到看不见。
        let inject_micros = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
        deskfuse_core::note_injection_duration(inject_micros);
        deskfuse_core::record_input_stage(deskfuse_core::InputStage::Inject, inject_micros);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_capture_preserves_lock_and_physical_flags_through_the_adapter() {
        use crate::hid::HidModifiers;
        let mut capture = WindowsSharingCapture::new();
        for (usage, down, repeat, expected_flags) in [
            (0x39, true, false, 0x10080),
            (0x39, true, true, 0x10080),
            (0x39, false, false, 0x10000),
            (0x04, true, false, 0x10000),
            (0x04, false, false, 0x10000),
            (0x39, true, false, 0x80),
            (0x39, false, false, 0),
        ] {
            let event = capture
                .convert(NativeCaptureEvent::Key {
                    usage,
                    down,
                    repeat,
                    modifiers: HidModifiers(0),
                })
                .unwrap();
            let CaptureEvent::Input(sample) = event else {
                panic!("expected key");
            };
            assert_eq!(
                sample.event,
                SharedInputEvent::Key {
                    code: keyboard_usage_to_legacy_keycode(usage).unwrap(),
                    pressed: down,
                    repeat,
                    modifiers: expected_flags,
                }
            );
        }
    }
    #[test]
    fn expired_session_cannot_be_rearmed() {
        let lease = SessionLease::default();
        let session = InputSessionId::new();
        assert!(lease.arm(session, Duration::from_millis(10)));
        lease.0.lock().unwrap().until = Some((
            Instant::now() - Duration::from_millis(1),
            SystemTime::now() - Duration::from_millis(1),
        ));
        assert!(lease.expired());
        assert!(!lease.arm(session, Duration::from_millis(10)));
        assert!(lease.arm(InputSessionId::new(), Duration::from_millis(10)));
    }

    #[test]
    fn button_mapping_is_explicit_and_bounded() {
        for button in 0..=4 {
            assert_eq!(button_number(native_button(button).unwrap()), button);
        }
        assert_eq!(native_button(5), None);
    }
}
