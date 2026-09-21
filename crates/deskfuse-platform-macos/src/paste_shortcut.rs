//! Active macOS Command-V interception for pending remote file clipboards.
//!
//! Unlike the general remote-input capture this is an active session event tap:
//! it may suppress the user's first paste while remote file bytes are still
//! materializing. Synthetic paste events carry a private marker and always pass.

#![allow(unsafe_code)]

use std::{
    ffi::c_void,
    ptr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI32, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use core_foundation::{
    base::TCFType,
    mach_port::CFMachPort,
    runloop::{CFRunLoop, kCFRunLoopCommonModes},
};
use core_foundation_sys::mach_port::{CFMachPortInvalidate, CFMachPortRef};
use core_graphics::{
    event::{
        CGEventFlags, CGEventMask, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
        CGEventType, EventField,
    },
    event_source::CGEventSourceStateID,
    sys::CGEventRef,
};

use crate::{PlatformError, PlatformResult, SYNTHETIC_PASTE_EVENT_MARKER};

type CGEventTapProxy = *const c_void;
type CGEventTapCallback =
    unsafe extern "C" fn(CGEventTapProxy, u32, CGEventRef, *mut c_void) -> CGEventRef;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventTapCreate(
        tap: CGEventTapLocation,
        place: CGEventTapPlacement,
        options: CGEventTapOptions,
        events_of_interest: CGEventMask,
        callback: CGEventTapCallback,
        user_info: *mut c_void,
    ) -> CFMachPortRef;
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
    fn CGEventGetFlags(event: CGEventRef) -> u64;
    fn CGEventSourceFlagsState(state: CGEventSourceStateID) -> u64;
}

/// A physical key chord. `modifiers` uses CoreGraphics flag bits.
#[derive(Clone, Debug)]
pub struct WorkflowHotkey {
    pub id: String,
    pub key_code: u16,
    pub modifiers: u64,
}

/// Updated by the engine without restarting the native event tap.
#[derive(Default)]
pub struct WorkflowHotkeys {
    pub bindings: Vec<WorkflowHotkey>,
    pub recording: bool,
}

/// Sent once on physical key-down; processing happens outside the callback.
#[derive(Clone, Debug)]
pub struct WorkflowHotkeyEvent {
    pub id: String,
    pub target_pid: i32,
}

fn workflow_modifier_mask() -> u64 {
    (CGEventFlags::CGEventFlagCommand
        | CGEventFlags::CGEventFlagControl
        | CGEventFlags::CGEventFlagAlternate
        | CGEventFlags::CGEventFlagShift)
        .bits()
}

/// Whether physical command/control/option/shift keys have all been released.
/// Call immediately before replay as well as while waiting, off the event tap.
pub fn workflow_modifiers_released() -> bool {
    // SAFETY: reads the HID state; no pointers or owned resources are involved.
    let flags = unsafe { CGEventSourceFlagsState(CGEventSourceStateID::HIDSystemState) };
    flags & workflow_modifier_mask() == 0
}

/// Current foreground application's PID, or zero when no application is active.
pub fn workflow_frontmost_pid() -> i32 {
    objc2_app_kit::NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .map(|application| application.processIdentifier())
        .filter(|pid| *pid > 0)
        .unwrap_or(0)
}

/// Replay only while the processed clipboard and original target remain current.
/// Call from the workflow worker, never from the native event-tap callback.
pub fn workflow_paste_if_current(
    target_pid: i32,
    expected_change_count: i64,
) -> Result<(), String> {
    use crate::Pasteboard;

    if target_pid <= 0 || workflow_frontmost_pid() != target_pid {
        return Err("目标应用已切换，已取消自动粘贴。".to_owned());
    }
    if !workflow_modifiers_released() {
        return Err("快捷键修饰键尚未释放，未执行自动粘贴。".to_owned());
    }
    let current = crate::MacOSPasteboard::new()
        .change_count()
        .map_err(|error| format!("无法确认当前剪贴板：{error}"))?;
    if current != expected_change_count {
        return Err("剪贴板内容已变化，已取消自动粘贴。".to_owned());
    }
    // Pasteboard access can cross processes; check focus again after it returns.
    if workflow_frontmost_pid() != target_pid {
        return Err("目标应用已切换，已取消自动粘贴。".to_owned());
    }
    crate::post_paste_shortcut().map_err(|error| match error {
        PlatformError::InputPermissionRequired => {
            "无法自动粘贴：请在系统设置的隐私与安全性中为 DeskFuse 开启辅助功能权限。".to_owned()
        }
        other => format!("无法自动粘贴：{other}"),
    })
}

struct TapContext {
    file_capture: Option<Arc<crate::FilePasteCapture>>,
    pending_file: Arc<AtomicBool>,
    paste_requested: Arc<AtomicBool>,
    target_pid: Arc<AtomicI32>,
    tap: Mutex<CFMachPortRef>,
    workflows: Arc<Mutex<WorkflowHotkeys>>,
    workflow_events: std::sync::mpsc::Sender<WorkflowHotkeyEvent>,
    captured_keys: [AtomicBool; 256],
}

// All mutable cross-thread state is atomic or mutex-protected. The mach-port
// pointer is only dereferenced by CoreGraphics while the owning thread keeps
// its retained CFMachPort alive.
unsafe impl Send for TapContext {}
unsafe impl Sync for TapContext {}

/// Owns the active event tap and its dedicated run-loop thread.
pub struct MacOSPasteShortcutMonitor {
    runloop: Arc<Mutex<Option<CFRunLoop>>>,
    thread: Option<JoinHandle<()>>,
}

impl MacOSPasteShortcutMonitor {
    /// Start intercepting Command-V only while `pending_file` is true.
    pub fn start(
        pending_file: Arc<AtomicBool>,
        paste_requested: Arc<AtomicBool>,
        target_pid: Arc<AtomicI32>,
    ) -> PlatformResult<Self> {
        let (events, _) = std::sync::mpsc::channel();
        Self::start_with_workflows(
            pending_file,
            paste_requested,
            target_pid,
            Arc::new(Mutex::new(WorkflowHotkeys::default())),
            events,
        )
    }

    /// Shares the pending-file paste tap with configurable image workflows.
    pub fn start_with_workflows(
        pending_file: Arc<AtomicBool>,
        paste_requested: Arc<AtomicBool>,
        target_pid: Arc<AtomicI32>,
        workflows: Arc<Mutex<WorkflowHotkeys>>,
        workflow_events: std::sync::mpsc::Sender<WorkflowHotkeyEvent>,
    ) -> PlatformResult<Self> {
        Self::start_with_file_capture(
            pending_file,
            paste_requested,
            target_pid,
            workflows,
            workflow_events,
            None,
        )
    }

    pub fn start_with_file_capture(
        pending_file: Arc<AtomicBool>,
        paste_requested: Arc<AtomicBool>,
        target_pid: Arc<AtomicI32>,
        workflows: Arc<Mutex<WorkflowHotkeys>>,
        workflow_events: std::sync::mpsc::Sender<WorkflowHotkeyEvent>,
        file_capture: Option<Arc<crate::FilePasteCapture>>,
    ) -> PlatformResult<Self> {
        let runloop = Arc::new(Mutex::new(None));
        let runloop_for_thread = Arc::clone(&runloop);
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("deskfuse-paste-tap".to_owned())
            .spawn(move || {
                paste_tap_thread(
                    runloop_for_thread,
                    ready_tx,
                    pending_file,
                    paste_requested,
                    target_pid,
                    workflows,
                    workflow_events,
                    file_capture,
                );
            })
            .map_err(|_| PlatformError::InputCaptureUnavailable)?;

        match ready_rx.recv_timeout(Duration::from_secs(2)) {
            Ok(Ok(())) => Ok(Self {
                runloop,
                thread: Some(thread),
            }),
            Ok(Err(())) | Err(_) => {
                let _ = thread.join();
                Err(PlatformError::InputCaptureUnavailable)
            }
        }
    }

    /// Stop the event tap and wait for its run loop to exit.
    pub fn stop(&mut self) {
        if let Some(runloop) = self.runloop.lock().ok().and_then(|guard| guard.clone()) {
            runloop.stop();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for MacOSPasteShortcutMonitor {
    fn drop(&mut self) {
        self.stop();
    }
}

#[allow(unsafe_code, clippy::too_many_arguments)]
fn paste_tap_thread(
    runloop_state: Arc<Mutex<Option<CFRunLoop>>>,
    ready: std::sync::mpsc::SyncSender<Result<(), ()>>,
    pending_file: Arc<AtomicBool>,
    paste_requested: Arc<AtomicBool>,
    target_pid: Arc<AtomicI32>,
    workflows: Arc<Mutex<WorkflowHotkeys>>,
    workflow_events: std::sync::mpsc::Sender<WorkflowHotkeyEvent>,
    file_capture: Option<Arc<crate::FilePasteCapture>>,
) {
    let runloop = CFRunLoop::get_current();
    let context = Box::new(TapContext {
        file_capture,
        pending_file,
        paste_requested,
        target_pid,
        tap: Mutex::new(ptr::null_mut()),
        workflows,
        workflow_events,
        captured_keys: std::array::from_fn(|_| AtomicBool::new(false)),
    });
    let context_ptr = Box::into_raw(context);
    let event_mask =
        (1_u64 << (CGEventType::KeyDown as u32)) | (1_u64 << (CGEventType::KeyUp as u32));
    // SAFETY: callback and context remain alive until after the tap is
    // invalidated and the run loop exits below.
    let tap_ref = unsafe {
        CGEventTapCreate(
            CGEventTapLocation::Session,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::Default,
            event_mask,
            paste_tap_callback,
            context_ptr.cast(),
        )
    };
    if tap_ref.is_null() {
        // SAFETY: CoreGraphics retained no callback context when tap creation
        // failed, so ownership returns directly to this thread.
        unsafe { drop(Box::from_raw(context_ptr)) };
        let _ = ready.send(Err(()));
        return;
    }
    // SAFETY: tap_ref follows the Create rule and is non-null.
    let tap = unsafe { CFMachPort::wrap_under_create_rule(tap_ref) };
    if let Ok(mut stored_tap) = unsafe { &*context_ptr }.tap.lock() {
        *stored_tap = tap_ref;
    }
    let source = match tap.create_runloop_source(0) {
        Ok(source) => source,
        Err(()) => {
            // SAFETY: invalidation disables callbacks before freeing context.
            unsafe {
                CFMachPortInvalidate(tap.as_concrete_TypeRef());
                drop(Box::from_raw(context_ptr));
            }
            let _ = ready.send(Err(()));
            return;
        }
    };
    // SAFETY: common modes is a process-lifetime Core Foundation constant.
    unsafe { runloop.add_source(&source, kCFRunLoopCommonModes) };
    // SAFETY: the retained tap remains alive through this run loop.
    unsafe { CGEventTapEnable(tap_ref, true) };
    if let Ok(mut slot) = runloop_state.lock() {
        *slot = Some(runloop.clone());
    }
    let _ = ready.send(Ok(()));
    CFRunLoop::run_current();
    // SAFETY: stop the callback source before releasing its Box context.
    unsafe {
        CFMachPortInvalidate(tap.as_concrete_TypeRef());
        drop(Box::from_raw(context_ptr));
    }
}

#[allow(unsafe_code)]
unsafe extern "C" fn paste_tap_callback(
    _proxy: CGEventTapProxy,
    event_type: u32,
    event: CGEventRef,
    user_info: *mut c_void,
) -> CGEventRef {
    if user_info.is_null() {
        return event;
    }
    // SAFETY: the owner keeps TapContext alive until the tap is invalidated.
    let context = unsafe { &*(user_info.cast::<TapContext>()) };
    if event_type == CGEventType::TapDisabledByTimeout as u32
        || event_type == CGEventType::TapDisabledByUserInput as u32
    {
        if let Ok(tap) = context.tap.try_lock()
            && !tap.is_null()
        {
            // SAFETY: the mutex stores the currently retained event tap.
            unsafe { CGEventTapEnable(*tap, true) };
        }
        return event;
    }
    if event.is_null()
        || (event_type != CGEventType::KeyDown as u32 && event_type != CGEventType::KeyUp as u32)
    {
        return event;
    }

    // SAFETY: CoreGraphics supplied a live event for the duration of callback.
    let marker = unsafe { CGEventGetIntegerValueField(event, EventField::EVENT_SOURCE_USER_DATA) };
    let keycode = unsafe { CGEventGetIntegerValueField(event, EventField::KEYBOARD_EVENT_KEYCODE) };
    let flags = CGEventFlags::from_bits_truncate(unsafe { CGEventGetFlags(event) });
    if marker == SYNTHETIC_PASTE_EVENT_MARKER {
        return event;
    }
    // Sharing injections remain valid remote-file paste intent below, but must
    // never start a local image workflow (nor consume a physical key-up).
    let source_pid =
        unsafe { CGEventGetIntegerValueField(event, EventField::EVENT_SOURCE_UNIX_PROCESS_ID) };
    let physical = marker != crate::sharing::SYNTHETIC_SHARING_EVENT_MARKER
        && source_pid != i64::from(std::process::id());
    if physical
        && let Ok(index) = usize::try_from(keycode)
        && let Some(captured) = context.captured_keys.get(index)
    {
        if event_type == CGEventType::KeyUp as u32 {
            return if captured.swap(false, Ordering::AcqRel) {
                ptr::null_mut()
            } else {
                event
            };
        }
        // Consume repeats even if modifiers or bindings changed mid-chord.
        if captured.load(Ordering::Acquire) {
            return ptr::null_mut();
        }
        let modifiers = flags.bits() & workflow_modifier_mask();
        let ordinary_paste = keycode == 9 && modifiers == CGEventFlags::CGEventFlagCommand.bits();
        let autorepeat =
            unsafe { CGEventGetIntegerValueField(event, EventField::KEYBOARD_EVENT_AUTOREPEAT) }
                != 0;
        if !ordinary_paste && !autorepeat {
            // Never wait on settings/UI work in the event tap. Drop the guard
            // before consulting AppKit or sending a request to the engine.
            let id = context.workflows.try_lock().ok().and_then(|state| {
                if state.recording {
                    return None;
                }
                state
                    .bindings
                    .iter()
                    .find(|binding| {
                        i64::from(binding.key_code) == keycode
                            && binding.modifiers & workflow_modifier_mask() == modifiers
                            && modifiers != 0
                    })
                    .map(|binding| binding.id.clone())
            });
            if let Some(id) = id {
                let event_pid = unsafe {
                    CGEventGetIntegerValueField(event, EventField::EVENT_TARGET_UNIX_PROCESS_ID)
                };
                let target_pid = paste_target_pid(event_pid, || {
                    objc2_app_kit::NSWorkspace::sharedWorkspace()
                        .frontmostApplication()
                        .map(|application| application.processIdentifier())
                });
                if context
                    .workflow_events
                    .send(WorkflowHotkeyEvent { id, target_pid })
                    .is_ok()
                {
                    captured.store(true, Ordering::Release);
                    return ptr::null_mut();
                }
            }
        }
    }
    if event_type != CGEventType::KeyDown as u32 {
        return event;
    }
    if !should_intercept_paste(
        keycode,
        flags,
        marker,
        context.pending_file.load(Ordering::Acquire)
            || context
                .file_capture
                .as_ref()
                .is_some_and(|capture| capture.has_reference()),
    ) {
        return event;
    }

    if let Some(capture) = &context.file_capture {
        let pid = workflow_frontmost_pid();
        let autorepeat =
            unsafe { CGEventGetIntegerValueField(event, EventField::KEYBOARD_EVENT_AUTOREPEAT) }
                != 0;
        let pasteboard = objc2_app_kit::NSPasteboard::generalPasteboard();
        let change_count = pasteboard.changeCount() as i64;
        let captured_reference = if autorepeat {
            capture.has_reference_at(change_count)
        } else {
            capture.request(pid, change_count)
        };
        if captured_reference {
            if physical
                && let Some(captured) = usize::try_from(keycode)
                    .ok()
                    .and_then(|i| context.captured_keys.get(i))
            {
                captured.store(true, Ordering::Release);
            }
            return ptr::null_mut();
        }
        // During remote-reference installation the engine may not yet have
        // published a capture. Only suppress that empty remote pasteboard;
        // a stale pending flag must never swallow a local screenshot or text.
        if context.pending_file.load(Ordering::Acquire) {
            let marker_type =
                objc2_foundation::NSString::from_str(crate::REMOTE_ORIGIN_MARKER_TYPE);
            let remote_file = pasteboard
                .stringForType(&marker_type)
                .is_some_and(|marker| {
                    marker
                        .to_string()
                        .starts_with("deskfuse-clipboard-file-v1:")
                });
            if remote_file && pasteboard.changeCount() as i64 == change_count {
                return ptr::null_mut();
            }
        }
        return event;
    }

    if !context.paste_requested.load(Ordering::Acquire) {
        let event_pid =
            unsafe { CGEventGetIntegerValueField(event, EventField::EVENT_TARGET_UNIX_PROCESS_ID) };
        // Session-posted keyboard-sharing events may not have a target PID
        // assigned yet. Capture the foreground application now, not after the
        // transfer, so the existing replay focus/generation checks stay valid.
        let pid = paste_target_pid(event_pid, || {
            objc2_app_kit::NSWorkspace::sharedWorkspace()
                .frontmostApplication()
                .map(|application| application.processIdentifier())
        });
        context.target_pid.store(pid, Ordering::Release);
        context.paste_requested.store(true, Ordering::Release);
    }
    // A null event tells CoreGraphics to suppress the physical Command-V.
    ptr::null_mut()
}

fn paste_target_pid(event_pid: i64, foreground: impl FnOnce() -> Option<i32>) -> i32 {
    // Session-injected events can retain a sender/previous target PID. The
    // foreground application at interception is where this paste was intended.
    foreground()
        .filter(|pid| *pid > 0)
        .or_else(|| i32::try_from(event_pid).ok().filter(|pid| *pid > 0))
        .unwrap_or(0)
}

fn should_intercept_paste(
    keycode: i64,
    flags: CGEventFlags,
    marker: i64,
    pending_file: bool,
) -> bool {
    pending_file
        && marker != SYNTHETIC_PASTE_EVENT_MARKER
        // Shared-keyboard events are user paste intent too. Only our completed
        // paste replay bypasses interception; input capture separately ignores
        // both markers to prevent sending injected events back to the peer.
        && keycode == 9
        && flags.contains(CGEventFlags::CGEventFlagCommand)
        && !flags.intersects(CGEventFlags::CGEventFlagControl | CGEventFlags::CGEventFlagAlternate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use foreign_types::ForeignType;

    #[test]
    fn session_paste_without_target_remembers_foreground_at_request_time() {
        assert_eq!(paste_target_pid(0, || Some(42)), 42);
        assert_eq!(paste_target_pid(43, || Some(42)), 42);
        assert_eq!(paste_target_pid(43, || None), 43);
        assert_eq!(paste_target_pid(0, || None), 0);
        assert_eq!(paste_target_pid(0, || Some(-1)), 0);
    }

    #[test]
    fn shared_keyboard_paste_requests_pending_file_once() {
        use core_graphics::{
            event::CGEvent,
            event_source::{CGEventSource, CGEventSourceStateID},
        };

        let context = TapContext {
            file_capture: None,
            pending_file: Arc::new(AtomicBool::new(true)),
            paste_requested: Arc::new(AtomicBool::new(false)),
            target_pid: Arc::new(AtomicI32::new(0)),
            tap: Mutex::new(ptr::null_mut()),
            workflows: Arc::new(Mutex::new(WorkflowHotkeys::default())),
            workflow_events: std::sync::mpsc::channel().0,
            captured_keys: std::array::from_fn(|_| AtomicBool::new(false)),
        };
        let make_event = |marker, pid| {
            let source = CGEventSource::new(CGEventSourceStateID::Private).unwrap();
            let event = CGEvent::new_keyboard_event(source, 9, true).unwrap();
            event.set_flags(CGEventFlags::CGEventFlagCommand);
            event.set_integer_value_field(EventField::EVENT_SOURCE_USER_DATA, marker);
            event.set_integer_value_field(EventField::EVENT_TARGET_UNIX_PROCESS_ID, pid);
            event
        };
        let context_ptr = (&context as *const TapContext).cast_mut().cast();
        let mut first_target = None;
        for pid in [42, 43] {
            let event = make_event(crate::sharing::SYNTHETIC_SHARING_EVENT_MARKER, pid);
            // SAFETY: both the event and context outlive this synchronous call;
            // no event is posted and no native event tap is installed.
            let result = unsafe {
                paste_tap_callback(
                    ptr::null(),
                    CGEventType::KeyDown as u32,
                    event.as_ptr(),
                    context_ptr,
                )
            };
            assert!(
                result.is_null(),
                "shared Command-V must wait for file bytes"
            );
            assert!(context.paste_requested.load(Ordering::Acquire));
            let target = context.target_pid.load(Ordering::Acquire);
            assert!(target > 0);
            assert_eq!(*first_target.get_or_insert(target), target);
        }
        let event = make_event(SYNTHETIC_PASTE_EVENT_MARKER, 42);
        assert_eq!(
            event.get_integer_value_field(EventField::EVENT_SOURCE_USER_DATA),
            SYNTHETIC_PASTE_EVENT_MARKER,
        );
        // SAFETY: same local live event/context; this only calls the callback.
        let replay = unsafe {
            paste_tap_callback(
                ptr::null(),
                CGEventType::KeyDown as u32,
                event.as_ptr(),
                context_ptr,
            )
        };
        assert_eq!(replay, event.as_ptr(), "completed paste replay must pass");
    }

    #[test]
    fn intercepts_only_plain_command_v_while_file_is_pending() {
        let command = CGEventFlags::CGEventFlagCommand;
        assert!(should_intercept_paste(9, command, 0, true));
        assert!(!should_intercept_paste(9, command, 0, false));
        assert!(!should_intercept_paste(
            9,
            command | CGEventFlags::CGEventFlagAlternate,
            0,
            true
        ));
        assert!(!should_intercept_paste(
            9,
            command,
            SYNTHETIC_PASTE_EVENT_MARKER,
            true
        ));
        assert!(!should_intercept_paste(8, command, 0, true));
    }
}
