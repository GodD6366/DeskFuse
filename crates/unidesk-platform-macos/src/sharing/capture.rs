use super::{Deadline, SYNTHETIC_SHARING_EVENT_MARKER, may_capture};
use crate::{PlatformError, PlatformResult, SYNTHETIC_PASTE_EVENT_MARKER};
use core_foundation::{
    base::TCFType,
    mach_port::CFMachPort,
    runloop::{CFRunLoop, kCFRunLoopDefaultMode},
};
use core_foundation_sys::mach_port::{CFMachPortInvalidate, CFMachPortRef};
use core_graphics::{
    event::{
        CGEvent, CGEventFlags, CGEventMask, CGEventTapLocation, CGEventTapOptions,
        CGEventTapPlacement, CGEventType, EventField,
    },
    event_source::{CGEventSource, CGEventSourceStateID},
    geometry::CGPoint,
    sys::CGEventRef,
};
use std::{
    collections::{BTreeSet, VecDeque},
    ffi::c_void,
    ptr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, Ordering},
        mpsc::{self, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use unidesk_core::SharedInputEvent;
use unidesk_input::sharing::HotkeyBindings;

fn capture_hotkey_matches(
    bindings: &HotkeyBindings,
    controller_enabled: bool,
    code: u16,
    modifiers: u64,
) -> bool {
    HotkeyBindings::is_emergency(code, modifiers)
        || (controller_enabled && bindings.matches(code, modifiers))
}
const QUEUE_SIZE: usize = 512;

fn is_synthetic_input(marker: i64, source_pid: i64) -> bool {
    marker == SYNTHETIC_SHARING_EVENT_MARKER
        || marker == SYNTHETIC_PASTE_EVENT_MARKER
        // Scroll processors may replace our marker on derived events while
        // retaining their original source. Do not route our output back out.
        || source_pid == i64::from(std::process::id())
}

pub use unidesk_platform::{CaptureEvent, CaptureMode, CaptureSample};
struct Shared {
    events: Mutex<VecDeque<CaptureSample>>,
    ready: Arc<tokio::sync::Notify>,
    exclusive: AtomicBool,
    controller_hotkeys: AtomicBool,
    /// Set by the caller before it queues `Command::Exit`, cleared on the tap
    /// thread once the cursor is warped back and re-associated. While set, the
    /// cursor is still frozen at the remote anchor, so motion decoded in this
    /// window describes that anchor and must not reach the consumer.
    restore_pending: AtomicBool,
    stop: AtomicBool,
    fault: AtomicU8,
    deadline: Deadline,
}
impl Shared {
    fn new() -> Self {
        Self {
            events: Mutex::new(VecDeque::with_capacity(QUEUE_SIZE)),
            ready: Arc::new(tokio::sync::Notify::new()),
            exclusive: AtomicBool::new(false),
            controller_hotkeys: AtomicBool::new(false),
            restore_pending: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            fault: AtomicU8::new(0),
            deadline: Deadline::new(),
        }
    }
    fn fail_open(&self, fault: u8) {
        self.deadline.clear();
        self.exclusive.store(false, Ordering::Release);
        self.fault.store(fault, Ordering::Release);
        self.ready.notify_one();
    }
    /// Mode changes invalidate queued coordinates/deltas, but never releases,
    /// buttons, modifiers or scrolling. Called on the tap thread before ACK so
    /// the consumer cannot reinterpret an old phase's motion after switching.
    fn discard_motion(&self) -> PlatformResult<usize> {
        let mut events = self
            .events
            .lock()
            .map_err(|_| PlatformError::InputCaptureUnavailable)?;
        let before = events.len();
        events.retain(|sample| !matches!(sample.event, SharedInputEvent::Motion { .. }));
        Ok(before - events.len())
    }
    /// Only adjacent motion is replaceable. Keep the latest position and all
    /// relative displacement, without crossing a button/key/wheel boundary.
    /// This lock covers bounded in-memory edits only; never native work or I/O.
    fn push(&self, sample: CaptureSample) -> bool {
        let Ok(mut events) = self.events.lock() else {
            self.fail_open(2);
            return false;
        };
        if let Some(previous) = events.back_mut()
            && matches!(previous.event, SharedInputEvent::Motion { .. })
            && matches!(sample.event, SharedInputEvent::Motion { .. })
        {
            let delta_x = previous.delta_x + sample.delta_x;
            let delta_y = previous.delta_y + sample.delta_y;
            let captured_at = previous.captured_at;
            *previous = sample;
            previous.captured_at = captured_at;
            previous.delta_x = delta_x;
            previous.delta_y = delta_y;
        } else {
            if events.len() == QUEUE_SIZE {
                drop(events);
                self.fail_open(2);
                return false;
            }
            events.push_back(sample);
        }
        drop(events);
        self.ready.notify_one();
        true
    }
}
type Reply = SyncSender<PlatformResult<()>>;
enum Command {
    Enter(f64, f64, Reply),
    Exit(Option<(f64, f64)>, Reply),
    Hotkeys(HotkeyBindings),
}
/// Sendable handle; the event tap, callback context, and cursor stay on one thread.
pub struct MacOSSharingCapture {
    shared: Arc<Shared>,
    commands: Option<SyncSender<Command>>,
    thread: Option<JoinHandle<()>>,
}
impl Default for MacOSSharingCapture {
    fn default() -> Self {
        Self::new()
    }
}
impl MacOSSharingCapture {
    pub fn new() -> Self {
        Self {
            shared: Arc::new(Shared::new()),
            commands: None,
            thread: None,
        }
    }
    pub fn start(&mut self) -> PlatformResult<()> {
        if self.thread.is_some() {
            return Ok(());
        }
        if !may_capture() {
            return Err(PlatformError::InputPermissionRequired);
        }
        self.shared = Arc::new(Shared::new());
        let shared = self.shared.clone();
        let (tx, rx) = mpsc::sync_channel(32);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("unidesk-sharing-tap".into())
            .spawn(move || capture_thread(shared, rx, ready_tx))
            .map_err(|_| PlatformError::InputCaptureUnavailable)?;
        match ready_rx.recv_timeout(Duration::from_secs(2)) {
            Ok(Ok(())) => {
                self.commands = Some(tx);
                self.thread = Some(thread);
                Ok(())
            }
            _ => {
                self.shared.stop.store(true, Ordering::Release);
                let _ = thread.join();
                Err(PlatformError::InputCaptureUnavailable)
            }
        }
    }
    pub fn stop(&mut self) -> PlatformResult<()> {
        self.shared.exclusive.store(false, Ordering::Release);
        self.shared.stop.store(true, Ordering::Release);
        self.commands.take();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        Ok(())
    }
    pub fn set_mode(&self, mode: CaptureMode) {
        self.shared
            .exclusive
            .store(mode == CaptureMode::Exclusive, Ordering::Release);
    }
    pub fn set_hotkeys(&self, hotkeys: HotkeyBindings) {
        if let Some(tx) = &self.commands
            && tx.try_send(Command::Hotkeys(hotkeys)).is_err()
        {
            self.shared.fail_open(2);
        }
    }
    pub fn arm_session(&self, session: unidesk_core::InputSessionId, duration: Duration) -> bool {
        self.shared.deadline.arm(session, duration)
    }
    pub fn renew_deadline(&self, duration: Duration) -> bool {
        self.shared.deadline.renew(duration)
    }
    pub fn lease_expired(&self) -> bool {
        self.shared.deadline.expired()
    }
    pub fn set_controller_hotkeys_enabled(&self, enabled: bool) {
        self.shared
            .controller_hotkeys
            .store(enabled, Ordering::Release);
    }

    /// Queue a cursor handover without waiting for the tap thread.
    ///
    /// The coordinator runs on a single-threaded async runtime whose `select!`
    /// cannot poll other branches while one arm is inside a blocking wait, so a
    /// synchronous round trip here froze capture draining and injection
    /// forwarding together. Deskflow has no equivalent: its platform object is
    /// called directly on the owning thread and its transport writes are
    /// buffered. Ordering is preserved through `restore_pending` instead of a
    /// reply, and genuine apply failures still surface through `fail_open`.
    pub fn enter_remote(&self, x: f64, y: f64) -> PlatformResult<()> {
        if !x.is_finite() || !y.is_finite() {
            return Err(PlatformError::InputCaptureUnavailable);
        }
        let started = Instant::now();
        let result = self.request_nowait(|reply| Command::Enter(x, y, reply));
        super::timing::record!(
            "capture_enter_request",
            started.elapsed(),
            format!("ok={}", result.is_ok()),
        );
        result
    }
    pub fn exit_remote(&self, restore: Option<(f64, f64)>) -> PlatformResult<()> {
        // Keep ownership and its deadline until the tap thread applies Exit with its exact
        // restore point. Clearing exclusive (or pausing the deadline) here races
        // the maintenance fallback: exit(None) can make the requested warp a no-op.
        let started = Instant::now();
        self.shared.restore_pending.store(true, Ordering::Release);
        let result = self.request_nowait(|reply| Command::Exit(restore, reply));
        if result.is_err() {
            self.shared.restore_pending.store(false, Ordering::Release);
            self.shared.fail_open(2);
        }
        super::timing::record!(
            "capture_exit_request",
            started.elapsed(),
            format!("ok={}", result.is_ok()),
        );
        result
    }
    /// Best-effort handover: returns only whether the command was queued.
    fn request_nowait(&self, command: impl FnOnce(Reply) -> Command) -> PlatformResult<()> {
        let (reply, _ignored) = mpsc::sync_channel(1);
        self.commands
            .as_ref()
            .ok_or(PlatformError::InputCaptureUnavailable)?
            .try_send(command(reply))
            .map_err(|_| PlatformError::InputCaptureUnavailable)
    }
    /// A retained notification permits wakeups arriving before the consumer
    /// begins waiting. Capture restarts replace the mailbox and notification.
    pub fn event_notification(&self) -> Arc<tokio::sync::Notify> {
        self.shared.ready.clone()
    }
    pub fn next_event(&mut self) -> PlatformResult<Option<CaptureEvent>> {
        let mut events = self
            .shared
            .events
            .lock()
            .map_err(|_| PlatformError::InputCaptureUnavailable)?;
        let fault = self.shared.fault.swap(0, Ordering::AcqRel);
        if fault != 0 {
            events.clear();
            return Ok(Some(match fault {
                1 => CaptureEvent::TapDisabled,
                3 => CaptureEvent::LeaseExpired,
                _ => CaptureEvent::Overflow,
            }));
        }
        let event = events.pop_front().map(CaptureEvent::Input);
        // A bounded consumer turn may stop before draining the mailbox. Preserve
        // a wakeup for the remaining ordered transitions, not a polling timer.
        if !events.is_empty() {
            self.shared.ready.notify_one();
        }
        Ok(event)
    }
}
impl Drop for MacOSSharingCapture {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

type TapCallback = unsafe extern "C" fn(*const c_void, u32, CGEventRef, *mut c_void) -> CGEventRef;
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventTapCreate(
        tap: CGEventTapLocation,
        place: CGEventTapPlacement,
        options: CGEventTapOptions,
        events: CGEventMask,
        callback: TapCallback,
        user_info: *mut c_void,
    ) -> CFMachPortRef;
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
    fn CGEventGetDoubleValueField(event: CGEventRef, field: u32) -> f64;
    fn CGEventGetFlags(event: CGEventRef) -> u64;
    fn CGEventGetLocation(event: CGEventRef) -> CGPoint;
    fn CGEventSourceKeyState(state: CGEventSourceStateID, key: u16) -> bool;
    fn CGEventSourceButtonState(state: CGEventSourceStateID, button: u32) -> bool;
    fn CGEventSourceFlagsState(state: CGEventSourceStateID) -> u64;
    fn CGDisplayHideCursor(display: u32) -> i32;
    fn CGDisplayShowCursor(display: u32) -> i32;
    fn CGAssociateMouseAndMouseCursorPosition(connected: bool) -> i32;
    fn CGWarpMouseCursorPosition(point: CGPoint) -> i32;
    /// Deprecated but still exported; the window server suppresses local pointer
    /// events for this interval after a warp, which is what makes the cursor
    /// hesitate on return. Deskflow drives it to ~zero for the same reason.
    fn CGSetLocalEventsSuppressionInterval(seconds: f64) -> i32;
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum CursorOperation {
    EnableBackground,
    Hide,
    Disassociate,
    /// Deskflow `avoidHesitatingCursor`: keeps the system's post-warp local-event
    /// suppression window minimal while this screen is not the active one.
    AvoidHesitatingCursor,
    /// Deskflow `setZeroSuppressionInterval`: removes that suppression entirely
    /// once the pointer is coupled back to this screen.
    ZeroSuppressionInterval,
    Warp(f64, f64),
    Associate,
    Show,
}
fn apply_cursor_operation(operation: CursorOperation) -> bool {
    // SAFETY: callers supply finite Quartz coordinates. Association restores
    // physical input; Show balances a successful hide owned by CursorGuard.
    unsafe {
        match operation {
            CursorOperation::EnableBackground => super::background_cursor::enable(),
            CursorOperation::Hide => CGDisplayHideCursor(0) == 0,
            CursorOperation::Disassociate => CGAssociateMouseAndMouseCursorPosition(false) == 0,
            CursorOperation::AvoidHesitatingCursor => {
                CGSetLocalEventsSuppressionInterval(0.0001) == 0
            }
            CursorOperation::ZeroSuppressionInterval => {
                CGSetLocalEventsSuppressionInterval(0.0) == 0
            }
            CursorOperation::Warp(x, y) => CGWarpMouseCursorPosition(CGPoint::new(x, y)) == 0,
            CursorOperation::Associate => CGAssociateMouseAndMouseCursorPosition(true) == 0,
            CursorOperation::Show => CGDisplayShowCursor(0) == 0,
        }
    }
}

/// Warp for an explicit same-Mac display-switch hotkey. Unlike exit_remote,
/// this operation intentionally moves the cursor even without a remote session.
pub fn warp_pointer(x: f64, y: f64) -> PlatformResult<()> {
    if !x.is_finite() || !y.is_finite() {
        return Err(PlatformError::InputCaptureUnavailable);
    }
    let warped = apply_cursor_operation(CursorOperation::Warp(x, y));
    let associated = apply_cursor_operation(CursorOperation::Associate);
    if warped && associated {
        Ok(())
    } else {
        Err(PlatformError::InputCaptureUnavailable)
    }
}
struct CursorGuard {
    hidden: bool,
    disassociated: bool,
    anchor: Option<CGPoint>,
}
impl CursorGuard {
    fn new() -> Self {
        Self {
            hidden: false,
            disassociated: false,
            anchor: None,
        }
    }
    fn enter(&mut self, x: f64, y: f64) -> PlatformResult<()> {
        self.enter_with(x, y, apply_cursor_operation)
    }
    fn enter_with(
        &mut self,
        x: f64,
        y: f64,
        mut apply: impl FnMut(CursorOperation) -> bool,
    ) -> PlatformResult<()> {
        if !x.is_finite() || !y.is_finite() {
            return Err(PlatformError::InputCaptureUnavailable);
        }
        self.exit_with(None, &mut apply)?;
        // A successful Quartz hide alone does not hide a background app's
        // cursor. Enable background control before acquiring a hide count.
        // If unsupported, leave local input visible and do not enter exclusive
        // capture; do not pretend that the source pointer disappeared.
        if !apply(CursorOperation::EnableBackground) {
            return Err(PlatformError::Unsupported {
                operation: "macOS background cursor control",
            });
        }
        if !apply(CursorOperation::Hide) {
            return Err(PlatformError::InputCaptureUnavailable);
        }
        self.hidden = true;
        // Before the pointer is frozen off this screen, minimise the window server's
        // post-warp local-event suppression. Deskflow calls this
        // `avoidHesitatingCursor()` right here, between hiding and disassociating.
        apply(CursorOperation::AvoidHesitatingCursor);
        self.disassociated = apply(CursorOperation::Disassociate);
        self.anchor = Some(CGPoint::new(x, y));
        if !apply(CursorOperation::Warp(x, y)) {
            let _ = self.exit_with(None, &mut apply);
            return Err(PlatformError::InputCaptureUnavailable);
        }
        Ok(())
    }
    fn hold(&mut self) -> bool {
        self.hold_with(|point| {
            // SAFETY: anchor was validated before the cursor guard was entered.
            unsafe { CGWarpMouseCursorPosition(point) == 0 }
        })
    }
    fn hold_with(&self, mut warp: impl FnMut(CGPoint) -> bool) -> bool {
        // A disassociated pointer already stays at its anchor while raw deltas
        // continue arriving. Do not warp on every event and maintenance tick.
        if self.disassociated {
            return true;
        }
        if let Some(anchor) = self.anchor {
            return warp(anchor);
        }
        true
    }
    fn exit(&mut self, restore: Option<(f64, f64)>) -> PlatformResult<()> {
        self.exit_with(restore, apply_cursor_operation)
    }
    fn exit_with(
        &mut self,
        restore: Option<(f64, f64)>,
        mut apply: impl FnMut(CursorOperation) -> bool,
    ) -> PlatformResult<()> {
        let was_remote = self.anchor.is_some() || self.hidden || self.disassociated;
        self.anchor = None;
        let mut ok = true;
        if let Some((x, y)) = restore
            && was_remote
            && x.is_finite()
            && y.is_finite()
        {
            ok &= apply(CursorOperation::Warp(x, y));
        }
        // Reassociate AFTER the final warp. A warp after association can leave
        // hardware motion temporarily frozen at the restored edge. Also do this
        // for the fallback that used repeated warps without disassociation.
        if was_remote {
            let restored = apply(CursorOperation::Associate);
            ok &= restored;
            self.disassociated = !restored;
            // Re-coupling alone is not enough: the restore warp above arms the
            // window server's local-event suppression, whose default (0.25 s) is
            // literally a cursor that hesitates after returning to this screen.
            // Deskflow's `enter()` clears it at exactly this point. Best effort:
            // it must not turn a successful restore into a reported failure.
            let _ = apply(CursorOperation::ZeroSuppressionInterval);
        }
        if self.hidden {
            let restored = apply(CursorOperation::Show);
            ok &= restored;
            if restored {
                self.hidden = false;
            }
        }
        if ok {
            Ok(())
        } else {
            Err(PlatformError::InputCaptureUnavailable)
        }
    }
}
impl Drop for CursorGuard {
    fn drop(&mut self) {
        let _ = self.exit(None);
    }
}
// Absolute CGEvent positions can predate a warp. On the local screen use a
// freshly queried cursor and ignore duplicates, including the restoration point.
fn refresh_local_motion(
    sample: &mut CaptureSample,
    live: (f64, f64),
    previous: &mut Option<(f64, f64)>,
) -> bool {
    if !live.0.is_finite() || !live.1.is_finite() || *previous == Some(live) {
        return false;
    }
    let old = previous.replace(live).unwrap_or(live);
    sample.global_x = live.0;
    sample.global_y = live.1;
    sample.delta_x = live.0 - old.0;
    sample.delta_y = live.1 - old.1;
    sample.event = SharedInputEvent::Motion {
        x: live.0,
        y: live.1,
    };
    true
}
struct Context {
    local_position: Option<(f64, f64)>,
    first_motion_after_switch: Option<(Instant, &'static str)>,
    shared: Arc<Shared>,
    cursor: CursorGuard,
    keyboard_source: CGEventSource,
    local_keys: BTreeSet<u16>,
    hotkeys: HotkeyBindings,
    consumed: BTreeSet<u16>,
    last_modifiers: Option<u64>,
    tap: CFMachPortRef,
}
impl Context {
    fn motion_boundary(&mut self, direction: &'static str) -> PlatformResult<()> {
        let discarded = self.shared.discard_motion()?;
        super::timing::record!(
            "capture_motion_boundary",
            Duration::ZERO,
            format!("direction={direction} discarded_motion={discarded}"),
        );
        Ok(())
    }
    fn fail_open(&mut self, fault: u8) {
        self.shared.fail_open(fault);
        let _ = self.exit_remote(None);
        self.consumed.clear();
    }
    fn post_keyboard(&self, event: CGEvent, flags: u64) {
        event.set_flags(CGEventFlags::from_bits_retain(flags));
        event.set_integer_value_field(
            EventField::EVENT_SOURCE_USER_DATA,
            SYNTHETIC_SHARING_EVENT_MARKER,
        );
        // Match the capture tap and Deskflow: synthetic input enters the same
        // HID stage as physical input.
        event.post(CGEventTapLocation::HID);
    }
    fn release_local_keyboard(&mut self) -> PlatformResult<()> {
        for code in std::mem::take(&mut self.local_keys) {
            let event = CGEvent::new_keyboard_event(self.keyboard_source.clone(), code, false)
                .map_err(|()| PlatformError::InputInjectionFailed)?;
            self.post_keyboard(event, self.last_modifiers.unwrap_or(0));
        }
        for code in super::injector::modifier_keycodes(self.last_modifiers.unwrap_or(0)) {
            let event = CGEvent::new_keyboard_event(self.keyboard_source.clone(), code, false)
                .map_err(|()| PlatformError::InputInjectionFailed)?;
            event.set_type(CGEventType::FlagsChanged);
            self.post_keyboard(event, 0);
        }
        Ok(())
    }
    fn exit_remote(&mut self, restore: Option<(f64, f64)>) -> PlatformResult<()> {
        let was_remote = self.cursor.hidden;
        let result = self.cursor.exit(restore);
        if was_remote {
            // This is on the event-tap thread, serialized with decoding. Retain
            // the exact warp destination as the baseline, not the old event's
            // frozen remote anchor. Native queued events use a live query below.
            self.local_position = restore.or_else(|| {
                CGEvent::new(self.keyboard_source.clone())
                    .ok()
                    .map(|event| {
                        let p = event.location();
                        (p.x, p.y)
                    })
            });
            if self.motion_boundary("exit").is_err() {
                self.shared.restore_pending.store(false, Ordering::Release);
                self.shared.fail_open(2);
                return Err(PlatformError::InputCaptureUnavailable);
            }
            for code in super::injector::modifier_keycodes(self.last_modifiers.unwrap_or(0)) {
                if let Ok(event) =
                    CGEvent::new_keyboard_event(self.keyboard_source.clone(), code, true)
                {
                    event.set_type(CGEventType::FlagsChanged);
                    self.post_keyboard(event, self.last_modifiers.unwrap_or(0));
                }
            }
        }
        // Cleared last, after the queued pre-warp motion is purged: only now can
        // a decoded position describe the restored local cursor.
        self.shared.restore_pending.store(false, Ordering::Release);
        result
    }
}
fn capture_thread(shared: Arc<Shared>, commands: mpsc::Receiver<Command>, ready: Reply) {
    super::prioritize_current_thread();
    let runloop = CFRunLoop::get_current();
    let keyboard_source = match CGEventSource::new(CGEventSourceStateID::Private) {
        Ok(source) => source,
        Err(()) => {
            let _ = ready.send(Err(PlatformError::InputCaptureUnavailable));
            return;
        }
    };
    let mut context = Box::new(Context {
        local_position: None,
        first_motion_after_switch: None,
        shared: shared.clone(),
        cursor: CursorGuard::new(),
        keyboard_source,
        local_keys: BTreeSet::new(),
        hotkeys: HotkeyBindings::default(),
        consumed: BTreeSet::new(),
        last_modifiers: None,
        tap: ptr::null_mut(),
    });
    let event_types = [
        CGEventType::MouseMoved,
        CGEventType::LeftMouseDragged,
        CGEventType::RightMouseDragged,
        CGEventType::OtherMouseDragged,
        CGEventType::LeftMouseDown,
        CGEventType::LeftMouseUp,
        CGEventType::RightMouseDown,
        CGEventType::RightMouseUp,
        CGEventType::OtherMouseDown,
        CGEventType::OtherMouseUp,
        CGEventType::KeyDown,
        CGEventType::KeyUp,
        CGEventType::FlagsChanged,
        CGEventType::ScrollWheel,
    ];
    let mask = event_types
        .iter()
        .fold(0_u64, |mask, event| mask | (1_u64 << (*event as u32)));
    // SAFETY: callback ABI matches CGEventTapCallBack using raw u32 event types
    // (including non-event notifications). Box address is stable, and callbacks
    // run only in this thread's run loop until the tap is invalidated below.
    //
    // Prefer the HID stage, which is where Deskflow and this crate's own
    // `input_native.rs` sit. Apple documents the tap locations only as positions
    // in the event pipeline and states that a tap at `kCGHIDEventTap` may be
    // "placed" only by a root process, so on a non-root build creation can fail.
    // Fall back to the session stage instead of losing capture entirely; the
    // level that was actually installed is written to the timing stream.
    let mut raw = unsafe {
        CGEventTapCreate(
            CGEventTapLocation::HID,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::Default,
            mask,
            tap_callback,
            (&mut *context as *mut Context).cast(),
        )
    };
    let mut level = "hid";
    if raw.is_null() {
        level = "session";
        raw = unsafe {
            CGEventTapCreate(
                CGEventTapLocation::Session,
                CGEventTapPlacement::HeadInsertEventTap,
                CGEventTapOptions::Default,
                mask,
                tap_callback,
                (&mut *context as *mut Context).cast(),
            )
        };
    }
    super::timing::record!(
        "capture_tap_level",
        Duration::ZERO,
        format!("level={level}")
    );
    if raw.is_null() {
        let _ = ready.send(Err(PlatformError::InputCaptureUnavailable));
        return;
    }
    // SAFETY: CoreGraphics returned a non-null Create-rule CFMachPort.
    let tap = unsafe { CFMachPort::wrap_under_create_rule(raw) };
    context.tap = raw;
    let source = match tap.create_runloop_source(0) {
        Ok(source) => source,
        Err(()) => {
            // SAFETY: invalidate before the owning callback context is dropped.
            unsafe {
                CFMachPortInvalidate(raw);
            }
            let _ = ready.send(Err(PlatformError::InputCaptureUnavailable));
            return;
        }
    };
    // SAFETY: default mode is a process-lifetime immutable CFString constant.
    unsafe {
        runloop.add_source(&source, kCFRunLoopDefaultMode);
        CGEventTapEnable(raw, true);
    }
    // Seed the physical state before the first edge sample. Enabling sharing
    // while a key or mouse button is already held must not bypass the guard or
    // leave a pre-existing local key-down without its eventual release.
    if let Ok(event) = CGEvent::new(context.keyboard_source.clone()) {
        let position = event.location();
        context.local_position = Some((position.x, position.y));
        // SAFETY: state queries are read-only and accept the HID table constant.
        let modifiers = unsafe { CGEventSourceFlagsState(CGEventSourceStateID::HIDSystemState) };
        context.last_modifiers = Some(modifiers);
        let sample = |event| CaptureSample {
            captured_at: Instant::now(),
            event,
            global_x: position.x,
            global_y: position.y,
            delta_x: 0.0,
            delta_y: 0.0,
        };
        shared.push(sample(SharedInputEvent::Modifiers { modifiers }));
        for code in 0..=255 {
            // SAFETY: code is in Quartz's bounded virtual-key range.
            if unsafe { CGEventSourceKeyState(CGEventSourceStateID::HIDSystemState, code) }
                && ![54, 55, 56, 57, 58, 59, 60, 61, 62, 63].contains(&code)
            {
                context.local_keys.insert(code);
                // Repeat marks a state seed, so a configured hotkey cannot fire
                // merely because it was already held while capture started.
                shared.push(sample(SharedInputEvent::Key {
                    code,
                    pressed: true,
                    modifiers,
                    repeat: true,
                }));
            }
        }
        for button in 0..32 {
            // SAFETY: Quartz supports mouse button indices 0 through 31.
            if unsafe { CGEventSourceButtonState(CGEventSourceStateID::HIDSystemState, button) } {
                shared.push(sample(SharedInputEvent::Button {
                    button: button as u8,
                    pressed: true,
                }));
            }
        }
    }
    // Resolve the private WindowServer cursor entry points now, off the user's
    // first handover, so the first Enter does not block the tap thread while
    // `dlsym` pulls CoreGraphics in.
    super::background_cursor::warm();
    // Start from a zero suppression interval so the first handover is not the one
    // that pays the window server's default 0.25 s of suppressed local events.
    let _ = apply_cursor_operation(CursorOperation::ZeroSuppressionInterval);
    let _ = ready.send(Ok(()));
    while !shared.stop.load(Ordering::Acquire) {
        while let Ok(command) = commands.try_recv() {
            match command {
                Command::Enter(x, y, reply) => {
                    let started = Instant::now();
                    let was_remote = context.cursor.hidden;
                    let result = if shared.deadline.expired() {
                        Err(PlatformError::InputCaptureUnavailable)
                    } else {
                        context
                            .cursor
                            .enter(x, y)
                            .and_then(|()| context.release_local_keyboard())
                    };
                    let result = result.and_then(|()| {
                        if !was_remote {
                            context.local_position = None;
                            context.motion_boundary("enter")?;
                        }
                        Ok(())
                    });
                    shared.exclusive.store(result.is_ok(), Ordering::Release);
                    // An Enter supersedes any Exit that is still pending, so the
                    // motion-drop window never outlives the handover it guards.
                    shared.restore_pending.store(false, Ordering::Release);
                    if result.is_err() {
                        // The caller no longer waits for a reply, so an apply
                        // failure has to reach the coordinator through the fault
                        // channel it already drains.
                        shared.fail_open(1);
                    }
                    super::timing::record!(
                        "capture_enter_native",
                        started.elapsed(),
                        format!(
                            "ok={} disassociated={}",
                            result.is_ok(),
                            context.cursor.disassociated
                        ),
                    );
                    context.first_motion_after_switch =
                        result.as_ref().ok().map(|()| (Instant::now(), "enter"));
                    let _ = reply.send(result);
                }
                Command::Exit(restore, reply) => {
                    let started = Instant::now();
                    let was_remote = context.cursor.hidden;
                    shared.deadline.pause();
                    shared.exclusive.store(false, Ordering::Release);
                    let result = context.exit_remote(restore);
                    super::timing::record!(
                        "capture_exit_native",
                        started.elapsed(),
                        format!("ok={} was_remote={was_remote}", result.is_ok()),
                    );
                    if was_remote && result.is_ok() {
                        context.first_motion_after_switch = Some((Instant::now(), "exit"));
                    }
                    if result.is_err() {
                        // Handover failures must still revoke control; the reply is
                        // no longer awaited, so use the fault channel instead.
                        shared.fail_open(1);
                    }
                    let _ = reply.send(result);
                }
                Command::Hotkeys(keys) => context.hotkeys = keys,
            }
        }
        if shared.exclusive.load(Ordering::Acquire) && shared.deadline.expired() {
            context.fail_open(3);
        }
        if !shared.exclusive.load(Ordering::Acquire) && context.cursor.hidden {
            let _ = context.exit_remote(None);
        }
        if !context.cursor.hold() {
            context.fail_open(1);
        }
        // SAFETY: default mode is process-lifetime and callbacks cannot overlap
        // the mutable context access above because this run loop is single-threaded.
        CFRunLoop::run_in_mode(
            unsafe { kCFRunLoopDefaultMode },
            Duration::from_millis(5),
            true,
        );
    }
    // SAFETY: invalidation synchronously removes the callback source before its
    // Box is dropped. No context or CF object crosses this thread boundary.
    unsafe {
        CFMachPortInvalidate(raw);
    }
    let _ = context.exit_remote(None);
}
unsafe extern "C" fn tap_callback(
    _proxy: *const c_void,
    kind: u32,
    event: CGEventRef,
    user_info: *mut c_void,
) -> CGEventRef {
    if user_info.is_null() {
        return event;
    }
    // SAFETY: owner keeps this stable Context alive through the single-threaded
    // callback and invalidates the source before destroying it.
    let context = unsafe { &mut *user_info.cast::<Context>() };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if kind == CGEventType::TapDisabledByTimeout as u32
            || kind == CGEventType::TapDisabledByUserInput as u32
        {
            context.fail_open(1);
            // SAFETY: context.tap is the retained tap on its owning thread.
            unsafe {
                CGEventTapEnable(context.tap, true);
            }
            return event;
        }
        if event.is_null() {
            return event;
        }
        if context.shared.exclusive.load(Ordering::Acquire) && context.shared.deadline.expired() {
            context.fail_open(3);
            return event;
        }
        // SAFETY: all CGEvent access is after notification/null checks, and the
        // OS retains this event for the entire callback invocation.
        let marker =
            unsafe { CGEventGetIntegerValueField(event, EventField::EVENT_SOURCE_USER_DATA) };
        // SAFETY: event is still non-null and callback-retained.
        let source_pid =
            unsafe { CGEventGetIntegerValueField(event, EventField::EVENT_SOURCE_UNIX_PROCESS_ID) };
        if is_synthetic_input(marker, source_pid) {
            return event;
        }
        // SAFETY: live callback event, as established above.
        let sample = unsafe { decode_event(kind, event) };
        let Some(mut sample) = sample else {
            return event;
        };
        if matches!(sample.event, SharedInputEvent::Motion { .. })
            && context.shared.restore_pending.load(Ordering::Acquire)
        {
            // The consumer already switched back to local, but this thread has
            // not warped the cursor to its restore point yet, so the decoded
            // position still describes the frozen remote anchor. Forwarding it
            // would both misplace the local cursor and re-cross the edge it just
            // came from. Dropping is lossless: the anchor can not move while it
            // is hidden, and the Exit handover discards queued motion anyway.
            return if context.shared.exclusive.load(Ordering::Acquire) {
                ptr::null_mut()
            } else {
                event
            };
        }
        if matches!(sample.event, SharedInputEvent::Motion { .. }) {
            // Native cursor ownership, not the async fail-open flag, determines
            // how this event was captured. This also covers an Exit request that
            // has been sent but has not yet restored/reassociated the cursor.
            let changed = if context.cursor.hidden {
                sample.delta_x != 0.0 || sample.delta_y != 0.0
            } else {
                let Ok(live) = CGEvent::new(context.keyboard_source.clone()) else {
                    context.fail_open(1);
                    return event;
                };
                let p = live.location();
                refresh_local_motion(&mut sample, (p.x, p.y), &mut context.local_position)
            };
            if !changed {
                return if context.shared.exclusive.load(Ordering::Acquire) {
                    ptr::null_mut()
                } else {
                    event
                };
            }
        }
        if matches!(sample.event, SharedInputEvent::Motion { .. })
            && let Some((started, direction)) = context.first_motion_after_switch.take()
        {
            super::timing::record!(
                "capture_first_motion",
                started.elapsed(),
                format!("direction={direction}"),
            );
        }
        // SAFETY: event was checked non-null above and is callback-retained.
        let modifiers = unsafe { CGEventGetFlags(event) };
        if context.last_modifiers != Some(modifiers) {
            context.last_modifiers = Some(modifiers);
            if !context.shared.push(CaptureSample {
                captured_at: sample.captured_at,
                event: SharedInputEvent::Modifiers { modifiers },
                global_x: sample.global_x,
                global_y: sample.global_y,
                delta_x: 0.0,
                delta_y: 0.0,
            }) {
                context.fail_open(2);
                return event;
            }
        }
        let mut hotkey = false;
        if let SharedInputEvent::Key {
            code,
            pressed,
            modifiers,
            ..
        } = sample.event
        {
            if pressed
                && (context.consumed.contains(&code)
                    || capture_hotkey_matches(
                        &context.hotkeys,
                        context.shared.controller_hotkeys.load(Ordering::Acquire),
                        code,
                        modifiers,
                    ))
            {
                context.consumed.insert(code);
                hotkey = true;
            }
            if !pressed && context.consumed.remove(&code) {
                hotkey = true;
            }
            if !context.shared.exclusive.load(Ordering::Acquire) && !hotkey {
                if pressed {
                    context.local_keys.insert(code);
                } else {
                    context.local_keys.remove(&code);
                }
            }
            if pressed && HotkeyBindings::is_emergency(code, modifiers) {
                context.shared.exclusive.store(false, Ordering::Release);
                let _ = context.exit_remote(None);
            }
        }
        if !context.shared.push(sample) {
            context.fail_open(2);
            return event;
        }
        if context.shared.exclusive.load(Ordering::Acquire) {
            if !context.cursor.hold() {
                context.fail_open(1);
                return event;
            }
            return ptr::null_mut();
        }
        if hotkey { ptr::null_mut() } else { event }
    }));
    match result {
        Ok(event) => event,
        Err(_) => {
            context.fail_open(2);
            event
        }
    }
}
/// # Safety
/// `event` is a non-null CGEvent retained by CoreGraphics for this invocation;
/// `kind` has been checked to exclude notification event types.
unsafe fn decode_event(kind: u32, event: CGEventRef) -> Option<CaptureSample> {
    // SAFETY: caller guarantees a live non-null event for all these field reads.
    let (position, modifiers) = unsafe { (CGEventGetLocation(event), CGEventGetFlags(event)) };
    let integer = |field| {
        // SAFETY: event remains retained by the callback for this function.
        unsafe { CGEventGetIntegerValueField(event, field) }
    };
    let double = |field| {
        // SAFETY: event remains retained by the callback for this function.
        unsafe { CGEventGetDoubleValueField(event, field) }
    };
    let mut dx = 0.0;
    let mut dy = 0.0;
    let input = if [
        CGEventType::MouseMoved,
        CGEventType::LeftMouseDragged,
        CGEventType::RightMouseDragged,
        CGEventType::OtherMouseDragged,
    ]
    .iter()
    .any(|t| *t as u32 == kind)
    {
        dx = integer(EventField::MOUSE_EVENT_DELTA_X) as f64;
        dy = integer(EventField::MOUSE_EVENT_DELTA_Y) as f64;
        SharedInputEvent::Motion {
            x: position.x,
            y: position.y,
        }
    } else if [
        CGEventType::LeftMouseDown,
        CGEventType::LeftMouseUp,
        CGEventType::RightMouseDown,
        CGEventType::RightMouseUp,
        CGEventType::OtherMouseDown,
        CGEventType::OtherMouseUp,
    ]
    .iter()
    .any(|t| *t as u32 == kind)
    {
        SharedInputEvent::Button {
            button: integer(EventField::MOUSE_EVENT_BUTTON_NUMBER).clamp(0, 31) as u8,
            pressed: [
                CGEventType::LeftMouseDown,
                CGEventType::RightMouseDown,
                CGEventType::OtherMouseDown,
            ]
            .iter()
            .any(|t| *t as u32 == kind),
        }
    } else if kind == CGEventType::KeyDown as u32 || kind == CGEventType::KeyUp as u32 {
        SharedInputEvent::Key {
            code: integer(EventField::KEYBOARD_EVENT_KEYCODE).clamp(0, 255) as u16,
            pressed: kind == CGEventType::KeyDown as u32,
            modifiers,
            repeat: integer(EventField::KEYBOARD_EVENT_AUTOREPEAT) != 0,
        }
    } else if kind == CGEventType::FlagsChanged as u32 {
        SharedInputEvent::Modifiers { modifiers }
    } else if kind == CGEventType::ScrollWheel as u32 {
        let continuous = integer(EventField::SCROLL_WHEEL_EVENT_IS_CONTINUOUS) != 0;
        let axis = |point, fixed, line| {
            let points = integer(point) as f64;
            if points != 0.0 {
                points
            } else if continuous {
                double(fixed)
            } else {
                integer(line) as f64 * 10.0
            }
        };
        SharedInputEvent::Scroll {
            dx: axis(
                EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_2,
                EventField::SCROLL_WHEEL_EVENT_FIXED_POINT_DELTA_AXIS_2,
                EventField::SCROLL_WHEEL_EVENT_DELTA_AXIS_2,
            ),
            dy: axis(
                EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_1,
                EventField::SCROLL_WHEEL_EVENT_FIXED_POINT_DELTA_AXIS_1,
                EventField::SCROLL_WHEEL_EVENT_DELTA_AXIS_1,
            ),
        }
    } else {
        return None;
    };
    // CGEvent flags remain a full u64 including device-specific modifier bits.
    let _ = CGEventFlags::from_bits_truncate(modifiers);
    Some(CaptureSample {
        captured_at: Instant::now(),
        event: input,
        global_x: position.x,
        global_y: position.y,
        delta_x: dx,
        delta_y: dy,
    })
}

impl unidesk_platform::SharedInputCapture for MacOSSharingCapture {
    fn start(&mut self) -> PlatformResult<()> {
        MacOSSharingCapture::start(self)
    }
    fn stop(&mut self) -> PlatformResult<()> {
        MacOSSharingCapture::stop(self)
    }
    fn set_mode(&self, mode: CaptureMode) {
        MacOSSharingCapture::set_mode(self, mode)
    }
    fn set_hotkeys(&self, hotkeys: HotkeyBindings) {
        MacOSSharingCapture::set_hotkeys(self, hotkeys)
    }
    fn arm_session(&self, session: unidesk_core::InputSessionId, duration: Duration) -> bool {
        MacOSSharingCapture::arm_session(self, session, duration)
    }
    fn renew_deadline(&self, duration: Duration) -> bool {
        MacOSSharingCapture::renew_deadline(self, duration)
    }
    fn lease_expired(&self) -> bool {
        MacOSSharingCapture::lease_expired(self)
    }
    fn set_controller_hotkeys_enabled(&self, enabled: bool) {
        MacOSSharingCapture::set_controller_hotkeys_enabled(self, enabled)
    }
    fn enter_remote(&self, x: f64, y: f64) -> PlatformResult<()> {
        MacOSSharingCapture::enter_remote(self, x, y)
    }
    fn exit_remote(&self, restore: Option<(f64, f64)>) -> PlatformResult<()> {
        MacOSSharingCapture::exit_remote(self, restore)
    }
    fn event_notification(&self) -> Arc<tokio::sync::Notify> {
        MacOSSharingCapture::event_notification(self)
    }
    fn next_event(&mut self) -> PlatformResult<Option<CaptureEvent>> {
        MacOSSharingCapture::next_event(self)
    }
}

/// One-shot recovery after the input child has exited. No event tap or lease is
/// created here, and no cursor warp is performed.
pub fn restore_pointer_after_service_exit() -> bool {
    apply_cursor_operation(CursorOperation::Associate)
        && apply_cursor_operation(CursorOperation::ZeroSuppressionInterval)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transformed_local_output_is_not_recaptured() {
        let own_pid = i64::from(std::process::id());
        assert!(is_synthetic_input(SYNTHETIC_SHARING_EVENT_MARKER, 0));
        assert!(is_synthetic_input(SYNTHETIC_PASTE_EVENT_MARKER, 0));
        assert!(is_synthetic_input(0x1234_5678, own_pid));
        assert!(is_synthetic_input(0, own_pid));
        assert!(!is_synthetic_input(0, 0));
        assert!(!is_synthetic_input(0x1234_5678, own_pid + 1));
    }

    fn sample(event: SharedInputEvent) -> CaptureSample {
        CaptureSample {
            captured_at: Instant::now(),
            event,
            global_x: 10.0,
            global_y: 20.0,
            delta_x: 1.0,
            delta_y: 2.0,
        }
    }
    #[test]
    fn old_event_coordinates_do_not_undo_local_restore() {
        let mut previous = Some((98.0, 50.0));
        let mut old = sample(SharedInputEvent::Motion { x: 99.0, y: 50.0 });
        old.global_x = 99.0;
        old.global_y = 50.0;
        old.delta_x = -120.0;
        assert!(!refresh_local_motion(&mut old, (98.0, 50.0), &mut previous));
        assert!(refresh_local_motion(&mut old, (97.0, 50.0), &mut previous));
        assert_eq!(old.global_x, 97.0);
        assert_eq!(old.delta_x, -1.0);
        assert_eq!(old.delta_y, 0.0);
        assert_eq!(old.event, SharedInputEvent::Motion { x: 97.0, y: 50.0 });
        // A real reversal back to the edge is not delayed or suppressed.
        assert!(refresh_local_motion(&mut old, (99.0, 50.0), &mut previous));
        assert_eq!(old.delta_x, 2.0);
    }
    #[test]
    fn motion_boundary_retains_all_ordered_non_motion_events() {
        let shared = Shared::new();
        let ordered = vec![
            SharedInputEvent::Button {
                button: 0,
                pressed: true,
            },
            SharedInputEvent::Scroll { dx: 0.0, dy: 3.0 },
            SharedInputEvent::Key {
                code: 1,
                pressed: false,
                modifiers: 0,
                repeat: false,
            },
            SharedInputEvent::Modifiers { modifiers: 0 },
            SharedInputEvent::Button {
                button: 0,
                pressed: false,
            },
        ];
        for event in &ordered {
            assert!(shared.push(sample(SharedInputEvent::Motion { x: 99.0, y: 50.0 })));
            assert!(shared.push(sample(event.clone())));
        }
        assert_eq!(shared.discard_motion().unwrap(), ordered.len());
        assert_eq!(
            shared
                .events
                .lock()
                .unwrap()
                .iter()
                .map(|s| s.event.clone())
                .collect::<Vec<_>>(),
            ordered
        );
        assert!(shared.push(sample(SharedInputEvent::Motion { x: 98.0, y: 50.0 })));
        assert_eq!(shared.events.lock().unwrap().back().unwrap().delta_x, 1.0);
    }
    #[test]
    fn exit_request_is_queued_without_waiting_for_the_tap_thread() {
        let mut capture = MacOSSharingCapture::new();
        let shared = capture.shared.clone();
        shared.exclusive.store(true, Ordering::Release);
        assert!(shared.deadline.arm(
            unidesk_core::InputSessionId::new(),
            Duration::from_millis(1500)
        ));
        let (tx, rx) = mpsc::sync_channel(1);
        capture.commands = Some(tx);
        // The coordinator runs on a single-threaded async runtime whose `select!`
        // cannot service capture or injection while one arm waits, so this call
        // must return as soon as the handover is queued.
        capture.exit_remote(Some((98.0, 50.0))).unwrap();
        assert!(
            shared.restore_pending.load(Ordering::Acquire),
            "the handover window must gate decoded motion until Exit is applied"
        );
        let Command::Exit(restore, reply) = rx.try_recv().unwrap() else {
            panic!("expected Exit");
        };
        assert_eq!(restore, Some((98.0, 50.0)));
        assert!(
            shared.exclusive.load(Ordering::Acquire),
            "ownership must remain native until Exit is applied"
        );
        assert!(
            !shared.deadline.expired(),
            "pending Exit must not manufacture a lease expiry"
        );
        // Emulate only the command ACK, never install a native tap or grant
        // permissions. The reply receiver is intentionally already gone.
        assert!(reply.send(Ok(())).is_err());
        shared.deadline.pause();
        shared.exclusive.store(false, Ordering::Release);
        shared.restore_pending.store(false, Ordering::Release);
    }
    #[test]
    fn tap_thread_closes_the_handover_window() {
        let Ok(keyboard_source) = CGEventSource::new(CGEventSourceStateID::Private) else {
            return;
        };
        let shared = Arc::new(Shared::new());
        shared.restore_pending.store(true, Ordering::Release);
        let mut context = Context {
            local_position: None,
            first_motion_after_switch: None,
            shared: shared.clone(),
            cursor: CursorGuard::new(),
            keyboard_source,
            local_keys: BTreeSet::new(),
            hotkeys: HotkeyBindings::default(),
            consumed: BTreeSet::new(),
            last_modifiers: None,
            tap: std::ptr::null_mut(),
        };
        // A cursor that was never hidden still ends the window, so a failed or
        // self-healing Exit can never leave motion dropped forever.
        context.exit_remote(None).unwrap();
        assert!(!shared.restore_pending.load(Ordering::Acquire));
    }
    #[test]
    fn background_cursor_is_enabled_before_each_balanced_hide() {
        let mut cursor = std::mem::ManuallyDrop::new(CursorGuard::new());
        let mut calls = Vec::new();
        // Multiple sessions cannot accumulate hide counts or depend on which
        // application currently has focus.
        for _ in 0..3 {
            cursor
                .enter_with(4.0, 5.0, |operation| {
                    calls.push(operation);
                    true
                })
                .unwrap();
            assert!(cursor.hidden && cursor.disassociated);
            cursor
                .exit_with(Some((40.0, 50.0)), |operation| {
                    calls.push(operation);
                    true
                })
                .unwrap();
            assert_eq!(
                calls,
                vec![
                    CursorOperation::EnableBackground,
                    CursorOperation::Hide,
                    CursorOperation::AvoidHesitatingCursor,
                    CursorOperation::Disassociate,
                    CursorOperation::Warp(4.0, 5.0),
                    CursorOperation::Warp(40.0, 50.0),
                    CursorOperation::Associate,
                    CursorOperation::ZeroSuppressionInterval,
                    CursorOperation::Show,
                ]
            );
            assert!(!cursor.hidden && !cursor.disassociated && cursor.anchor.is_none());
            calls.clear();
        }
    }

    #[test]
    fn unavailable_background_cursor_leaves_local_input_untouched() {
        for failed in [CursorOperation::EnableBackground, CursorOperation::Hide] {
            let mut cursor = std::mem::ManuallyDrop::new(CursorGuard::new());
            let mut calls = Vec::new();
            assert!(
                cursor
                    .enter_with(4.0, 5.0, |operation| {
                        calls.push(operation);
                        operation != failed
                    })
                    .is_err()
            );
            assert_eq!(calls.last(), Some(&failed));
            assert!(!calls.contains(&CursorOperation::Disassociate));
            assert!(!cursor.hidden && !cursor.disassociated && cursor.anchor.is_none());
            cursor
                .exit_with(None, |_| {
                    panic!("failed entry acquired no cursor ownership")
                })
                .unwrap();
        }
    }

    #[test]
    fn failed_entry_warp_balances_hide_and_restores_mouse_association() {
        let mut cursor = std::mem::ManuallyDrop::new(CursorGuard::new());
        let mut calls = Vec::new();
        assert!(
            cursor
                .enter_with(4.0, 5.0, |operation| {
                    calls.push(operation);
                    !matches!(operation, CursorOperation::Warp(..))
                })
                .is_err()
        );
        assert_eq!(
            &calls[calls.len() - 3..],
            &[
                CursorOperation::Associate,
                CursorOperation::ZeroSuppressionInterval,
                CursorOperation::Show
            ]
        );
        assert!(!cursor.hidden && !cursor.disassociated && cursor.anchor.is_none());
    }

    #[test]
    fn cursor_restore_warps_before_reassociating_physical_motion() {
        let mut cursor = std::mem::ManuallyDrop::new(CursorGuard {
            hidden: true,
            disassociated: true,
            anchor: Some(CGPoint::new(4.0, 5.0)),
        });
        let mut calls = Vec::new();
        cursor
            .exit_with(Some((40.0, 50.0)), |call| {
                calls.push(call);
                true
            })
            .unwrap();
        assert_eq!(
            calls,
            vec![
                CursorOperation::Warp(40.0, 50.0),
                CursorOperation::Associate,
                CursorOperation::ZeroSuppressionInterval,
                CursorOperation::Show
            ]
        );
        calls.clear();
        cursor
            .exit_with(Some((80.0, 90.0)), |call| {
                calls.push(call);
                true
            })
            .unwrap();
        assert!(
            calls.is_empty(),
            "idle cleanup cannot move or show a cursor again"
        );
    }

    #[test]
    fn disassociated_cursor_does_not_warp_on_every_event() {
        let mut cursor = std::mem::ManuallyDrop::new(CursorGuard {
            hidden: true,
            disassociated: true,
            anchor: Some(CGPoint::new(4.0, 5.0)),
        });
        let mut warps = 0;
        for _ in 0..1000 {
            assert!(cursor.hold_with(|_| {
                warps += 1;
                true
            }));
        }
        assert_eq!(warps, 0);
        cursor.disassociated = false;
        assert!(
            !cursor.hold_with(|_| false),
            "fallback warp failure must fail open"
        );
    }

    #[test]
    fn cursor_restore_failures_still_release_and_retry_native_ownership() {
        let mut cursor = std::mem::ManuallyDrop::new(CursorGuard {
            hidden: true,
            disassociated: true,
            anchor: Some(CGPoint::new(4.0, 5.0)),
        });
        let mut calls = Vec::new();
        assert!(
            cursor
                .exit_with(Some((40.0, 50.0)), |call| {
                    calls.push(call);
                    false
                })
                .is_err()
        );
        assert_eq!(
            calls,
            vec![
                CursorOperation::Warp(40.0, 50.0),
                CursorOperation::Associate,
                CursorOperation::ZeroSuppressionInterval,
                CursorOperation::Show
            ]
        );
        assert!(cursor.disassociated && cursor.hidden);
        calls.clear();
        cursor
            .exit_with(None, |call| {
                calls.push(call);
                true
            })
            .unwrap();
        assert_eq!(
            calls,
            vec![
                CursorOperation::Associate,
                CursorOperation::ZeroSuppressionInterval,
                CursorOperation::Show
            ]
        );
        assert!(!cursor.disassociated && !cursor.hidden);
    }

    #[test]
    fn fallback_cursor_restoration_reassociates_and_rejects_nonfinite_warps() {
        for restore in [Some((f64::NAN, 5.0)), None] {
            let mut cursor = std::mem::ManuallyDrop::new(CursorGuard {
                hidden: true,
                disassociated: false,
                anchor: Some(CGPoint::new(4.0, 5.0)),
            });
            let mut calls = Vec::new();
            cursor
                .exit_with(restore, |call| {
                    calls.push(call);
                    true
                })
                .unwrap();
            assert_eq!(
                calls,
                vec![
                    CursorOperation::Associate,
                    CursorOperation::ZeroSuppressionInterval,
                    CursorOperation::Show
                ]
            );
        }
    }

    #[test]
    fn motion_coalesces_but_button_boundaries_survive() {
        let mut c = MacOSSharingCapture::new();
        let s = &c.shared;
        assert!(s.push(sample(SharedInputEvent::Motion { x: 1.0, y: 2.0 })));
        assert!(s.push(sample(SharedInputEvent::Motion { x: 2.0, y: 3.0 })));
        assert!(s.push(sample(SharedInputEvent::Button {
            button: 0,
            pressed: true
        })));
        assert!(s.push(sample(SharedInputEvent::Motion { x: 3.0, y: 4.0 })));
        let Some(CaptureEvent::Input(first)) = c.next_event().unwrap() else {
            panic!("motion")
        };
        assert_eq!(first.delta_x, 2.0);
        assert!(matches!(
            c.next_event().unwrap(),
            Some(CaptureEvent::Input(CaptureSample {
                event: SharedInputEvent::Button { .. },
                ..
            }))
        ));
        assert!(c.next_event().unwrap().is_some());
    }
    #[test]
    fn motion_burst_keeps_one_sample_and_all_displacement() {
        let mut capture = MacOSSharingCapture::new();
        for i in 0..10_000 {
            assert!(capture.shared.push(sample(SharedInputEvent::Motion {
                x: f64::from(i),
                y: 0.0
            })));
        }
        assert_eq!(capture.shared.events.lock().unwrap().len(), 1);
        let Some(CaptureEvent::Input(motion)) = capture.next_event().unwrap() else {
            panic!("motion")
        };
        assert_eq!(motion.delta_x, 10_000.0);
        assert_eq!(motion.delta_y, 20_000.0);
        assert_eq!(motion.event, SharedInputEvent::Motion { x: 9999.0, y: 0.0 });
        assert_eq!(capture.shared.fault.load(Ordering::Acquire), 0);
    }
    #[tokio::test]
    async fn capture_and_fault_wake_without_polling_and_without_lost_wakeup() {
        let mut capture = MacOSSharingCapture::new();
        let ready = capture.event_notification();
        // Notification before awaiting must also work.
        capture
            .shared
            .push(sample(SharedInputEvent::Motion { x: 1.0, y: 0.0 }));
        tokio::time::timeout(Duration::from_millis(100), ready.notified())
            .await
            .unwrap();
        assert!(capture.next_event().unwrap().is_some());
        let waiter = ready.notified();
        tokio::pin!(waiter);
        assert!(!waiter.as_mut().enable());
        capture.shared.fail_open(1);
        tokio::time::timeout(Duration::from_millis(100), waiter)
            .await
            .unwrap();
        assert_eq!(
            capture.next_event().unwrap(),
            Some(CaptureEvent::TapDisabled)
        );
    }
    #[tokio::test]
    async fn partial_drain_retains_wakeup_for_ordered_events() {
        let mut capture = MacOSSharingCapture::new();
        for pressed in [true, false] {
            capture
                .shared
                .push(sample(SharedInputEvent::Button { button: 0, pressed }));
        }
        let ready = capture.event_notification();
        ready.notified().await;
        assert!(capture.next_event().unwrap().is_some());
        tokio::time::timeout(Duration::from_millis(100), ready.notified())
            .await
            .unwrap();
        assert!(capture.next_event().unwrap().is_some());
        assert_eq!(capture.next_event().unwrap(), None);
    }
    #[test]
    fn queue_overflow_fails_open_instead_of_losing_key_up() {
        let s = Shared::new();
        s.exclusive.store(true, Ordering::Release);
        for _ in 0..QUEUE_SIZE {
            assert!(s.push(sample(SharedInputEvent::Button {
                button: 0,
                pressed: true
            })));
        }
        assert!(!s.push(sample(SharedInputEvent::Button {
            button: 0,
            pressed: false
        })));
        assert!(!s.exclusive.load(Ordering::Acquire));
        assert_eq!(s.fault.load(Ordering::Acquire), 2);
    }
    fn decoded(event: &CGEvent) -> CaptureSample {
        use foreign_types::ForeignType;
        // SAFETY: test owns the retained event, and passes its real event type.
        unsafe { decode_event(event.get_type() as u32, event.as_ptr()) }.expect("captured event")
    }
    #[test]
    fn quartz_integer_raw_motion_fields_are_preserved() {
        let source = CGEventSource::new(CGEventSourceStateID::Private).unwrap();
        let event = CGEvent::new_mouse_event(
            source,
            CGEventType::LeftMouseDragged,
            CGPoint::new(-850.0, 42.0),
            core_graphics::event::CGMouseButton::Left,
        )
        .unwrap();
        event.set_integer_value_field(EventField::MOUSE_EVENT_DELTA_X, -7);
        event.set_integer_value_field(EventField::MOUSE_EVENT_DELTA_Y, 11);
        let sample = decoded(&event);
        assert_eq!(
            (
                sample.global_x,
                sample.global_y,
                sample.delta_x,
                sample.delta_y
            ),
            (-850.0, 42.0, -7.0, 11.0)
        );
    }
    #[test]
    fn quartz_line_wheel_falls_back_and_smooth_wheel_keeps_pixels() {
        let source = CGEventSource::new(CGEventSourceStateID::Private).unwrap();
        let event = CGEvent::new_scroll_event(
            source,
            core_graphics::event::ScrollEventUnit::LINE,
            2,
            2,
            -1,
            0,
        )
        .unwrap();
        event.set_integer_value_field(EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_1, 0);
        event.set_integer_value_field(EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_2, 0);
        event.set_integer_value_field(EventField::SCROLL_WHEEL_EVENT_IS_CONTINUOUS, 0);
        assert_eq!(
            decoded(&event).event,
            SharedInputEvent::Scroll {
                dx: -10.0,
                dy: 20.0
            }
        );
        event.set_integer_value_field(EventField::SCROLL_WHEEL_EVENT_IS_CONTINUOUS, 1);
        event.set_double_value_field(
            EventField::SCROLL_WHEEL_EVENT_FIXED_POINT_DELTA_AXIS_1,
            0.25,
        );
        event.set_double_value_field(
            EventField::SCROLL_WHEEL_EVENT_FIXED_POINT_DELTA_AXIS_2,
            -0.5,
        );
        assert_eq!(
            decoded(&event).event,
            SharedInputEvent::Scroll { dx: -0.5, dy: 0.25 }
        );
        event.set_integer_value_field(EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_1, 17);
        assert_eq!(
            decoded(&event).event,
            SharedInputEvent::Scroll { dx: -0.5, dy: 17.0 }
        );
    }
    #[test]
    fn quartz_key_repeat_and_flags_changed_are_captured() {
        let source = CGEventSource::new(CGEventSourceStateID::Private).unwrap();
        let event = CGEvent::new_keyboard_event(source, 0, true).unwrap();
        let flags = (1 << 17) | 4 | (1 << 20) | 8;
        event.set_flags(CGEventFlags::from_bits_retain(flags));
        event.set_integer_value_field(EventField::KEYBOARD_EVENT_AUTOREPEAT, 1);
        assert_eq!(
            decoded(&event).event,
            SharedInputEvent::Key {
                code: 0,
                pressed: true,
                modifiers: flags,
                repeat: true
            }
        );
        event.set_type(CGEventType::FlagsChanged);
        assert_eq!(
            decoded(&event).event,
            SharedInputEvent::Modifiers { modifiers: flags }
        );
    }
    #[test]
    fn controlled_role_only_consumes_fixed_emergency_chord() {
        let keys = HotkeyBindings::default();
        let flags = unidesk_input::sharing::SHARING_MODIFIERS;
        assert!(capture_hotkey_matches(&keys, false, 53, flags));
        for code in [123, 124, 125, 126, 37] {
            assert!(!capture_hotkey_matches(&keys, false, code, flags));
            assert!(capture_hotkey_matches(&keys, true, code, flags));
        }
    }
}
