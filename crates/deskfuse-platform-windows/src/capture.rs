//! Ordinary Default-desktop capture with bounded exclusive handoff.
//! Dedicated low-level hooks forward through a nonblocking queue. Exclusive
//! suppression starts only when no physical key/button already delivered locally
//! is held, and expires after at most 1.5 seconds without renewal. Queue faults,
//! desktop changes, callback failures and the emergency chord always fail open.
//!
//! Always poll faults before consuming further input and release the remote
//! injector/control lease on any fault. Faults are latched outside the event
//! queue, so losing a key-up to overflow can never silently leave control active.
//! Restart this capture object after a fault; old queued events are not replayed.
//! Thread heartbeat proves only message-loop liveness: Windows can silently
//! remove a timed-out low-level hook without providing a detection API.

use crate::{hid::HidModifiers, input::MouseButton};
use std::{
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
        mpsc::{Receiver, SyncSender, TryRecvError, TrySendError},
    },
    time::{Duration, Instant, SystemTime},
};
use tokio::sync::Notify;

const HEARTBEAT_TIMEOUT_MS: u64 = 750;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureEvent {
    /// Modifier usages have their own event; do not feed them to `injector.keyboard`.
    Modifiers(HidModifiers),
    Key {
        usage: u16,
        down: bool,
        repeat: bool,
        modifiers: HidModifiers,
    },
    /// Per-monitor-aware physical screen coordinates, including negative origins.
    MouseMove { x: i32, y: i32 },
    MouseButton {
        button: MouseButton,
        down: bool,
        x: i32,
        y: i32,
    },
    /// Native wheel units (120/notch); positive means up/right.
    Scroll {
        vertical: i32,
        horizontal: i32,
        x: i32,
        y: i32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum CaptureFault {
    QueueOverflow = 1,
    ConsumerDisconnected = 2,
    EmergencyReclaim = 3,
    DesktopChanged = 5,
    MessageLoopFailed = 6,
    CallbackFailed = 7,
    ThreadStopped = 8,
    HeartbeatTimedOut = 9,
    UnsupportedKeyboardMapping = 10,
    SecureAttentionRequested = 11,
}
impl CaptureFault {
    fn from_byte(value: u8) -> Option<Self> {
        Some(match value {
            0 => return None,
            1 => Self::QueueOverflow,
            2 => Self::ConsumerDisconnected,
            3 => Self::EmergencyReclaim,
            5 => Self::DesktopChanged,
            6 => Self::MessageLoopFailed,
            8 => Self::ThreadStopped,
            9 => Self::HeartbeatTimedOut,
            10 => Self::UnsupportedKeyboardMapping,
            11 => Self::SecureAttentionRequested,
            _ => Self::CallbackFailed,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureError {
    InvalidQueueCapacity,
    AlreadyRunning,
    DesktopUnavailable,
    NativeCall {
        operation: &'static str,
        code: u32,
    },
    ThreadStartFailed,
    StartupTimedOut,
    /// Exclusive capture cannot start while input already delivered locally is held.
    SuppressionBlockedByHeldInput,
    Faulted(CaptureFault),
    Stopped,
}
impl fmt::Display for CaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for CaptureError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub enum CapturePoll {
    Event(CaptureEvent),
    /// Latched notification, independent of queue capacity. Release remote input.
    Fault(CaptureFault),
    Idle,
    Stopped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureMode {
    ObserverOnly,
    Exclusive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub struct CaptureStatus {
    pub mode: CaptureMode,
    pub thread_alive: bool,
    pub suppressing: bool,
    pub fault: Option<CaptureFault>,
}

struct SuppressionState {
    until: Option<(Instant, SystemTime)>,
    physical_keys: [bool; 256],
    physical_buttons: u8,
}
impl Default for SuppressionState {
    fn default() -> Self {
        Self {
            until: None,
            physical_keys: [false; 256],
            physical_buttons: 0,
        }
    }
}
impl SuppressionState {
    fn expire(&mut self) {
        if self.until.is_some_and(|(monotonic, wall)| {
            Instant::now() >= monotonic || SystemTime::now() >= wall
        }) {
            self.until = None;
        }
    }
    fn active(&mut self) -> bool {
        self.expire();
        self.until.is_some()
    }
    fn held(&self) -> bool {
        self.physical_keys.iter().any(|down| *down) || self.physical_buttons != 0
    }
}

struct Shared {
    origin: Instant,
    stop: AtomicBool,
    alive: AtomicBool,
    heartbeat_ms: AtomicU64,
    fault: AtomicU8,
    suppression: Mutex<SuppressionState>,
    notification: Arc<Notify>,
}
impl Shared {
    fn new() -> Self {
        Self {
            origin: Instant::now(),
            stop: AtomicBool::new(false),
            alive: AtomicBool::new(false),
            heartbeat_ms: AtomicU64::new(0),
            fault: AtomicU8::new(0),
            suppression: Mutex::new(SuppressionState::default()),
            notification: Arc::new(Notify::new()),
        }
    }
    fn now_ms(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
    fn fault(&self) -> Option<CaptureFault> {
        CaptureFault::from_byte(self.fault.load(Ordering::Acquire))
    }
    fn trip(&self, fault: CaptureFault) {
        if let Ok(mut suppression) = self.suppression.lock() {
            suppression.until = None;
        }
        // The fault latch is independent of queue capacity and cannot be renewed away.
        let _ = self
            .fault
            .compare_exchange(0, fault as u8, Ordering::AcqRel, Ordering::Acquire);
        self.notification.notify_one();
    }
    fn request_suppression(&self, duration: Duration) -> Result<(), CaptureError> {
        let Ok(mut suppression) = self.suppression.lock() else {
            self.trip(CaptureFault::CallbackFailed);
            return Err(CaptureError::Faulted(CaptureFault::CallbackFailed));
        };
        suppression.expire();
        if duration.is_zero() || suppression.held() {
            return Err(CaptureError::SuppressionBlockedByHeldInput);
        }
        let duration = duration.min(Duration::from_millis(1_500));
        suppression.until = Some((Instant::now() + duration, SystemTime::now() + duration));
        Ok(())
    }
    fn renew_suppression(&self, duration: Duration) -> bool {
        let Ok(mut suppression) = self.suppression.lock() else {
            self.trip(CaptureFault::CallbackFailed);
            return false;
        };
        if !suppression.active() || duration.is_zero() {
            suppression.until = None;
            return false;
        }
        let duration = duration.min(Duration::from_millis(1_500));
        suppression.until = Some((Instant::now() + duration, SystemTime::now() + duration));
        true
    }
    fn stop_suppression(&self) {
        if let Ok(mut suppression) = self.suppression.lock() {
            suppression.until = None;
        }
    }
    fn observe_key(&self, usage: u16, down: bool) -> bool {
        let Ok(mut suppression) = self.suppression.try_lock() else {
            self.trip(CaptureFault::CallbackFailed);
            return false;
        };
        let active = suppression.active();
        if let Some(key) = suppression.physical_keys.get_mut(usize::from(usage)) {
            *key = down;
        }
        active
    }
    fn observe_button(&self, button: MouseButton, down: bool) -> bool {
        let Ok(mut suppression) = self.suppression.try_lock() else {
            self.trip(CaptureFault::CallbackFailed);
            return false;
        };
        let active = suppression.active();
        let bit = match button {
            MouseButton::Left => 1,
            MouseButton::Right => 2,
            MouseButton::Middle => 4,
            MouseButton::Back => 8,
            MouseButton::Forward => 16,
        };
        if down {
            suppression.physical_buttons |= bit;
        } else {
            suppression.physical_buttons &= !bit;
        }
        active
    }
    fn should_suppress(&self) -> bool {
        self.suppression
            .try_lock()
            .map(|mut suppression| suppression.active())
            .unwrap_or_else(|_| {
                self.trip(CaptureFault::CallbackFailed);
                false
            })
    }
    fn status(&self) -> CaptureStatus {
        let now = self.now_ms();
        let alive = self.alive.load(Ordering::Acquire);
        if alive
            && now.saturating_sub(self.heartbeat_ms.load(Ordering::Acquire)) > HEARTBEAT_TIMEOUT_MS
        {
            self.trip(CaptureFault::HeartbeatTimedOut);
        }
        let suppressing = self
            .suppression
            .lock()
            .map(|mut suppression| suppression.active())
            .unwrap_or(false);
        CaptureStatus {
            mode: if suppressing {
                CaptureMode::Exclusive
            } else {
                CaptureMode::ObserverOnly
            },
            thread_alive: alive,
            suppressing,
            fault: self.fault(),
        }
    }
    fn publish(&self, sender: &SyncSender<CaptureEvent>, event: CaptureEvent) -> bool {
        if self.fault().is_some() || self.stop.load(Ordering::Acquire) {
            return false;
        }
        match sender.try_send(event) {
            Ok(()) => {
                self.notification.notify_one();
                true
            }
            Err(TrySendError::Full(_)) => {
                self.trip(CaptureFault::QueueOverflow);
                false
            }
            Err(TrySendError::Disconnected(_)) => {
                self.trip(CaptureFault::ConsumerDisconnected);
                false
            }
        }
    }
    fn poll(&self, receiver: &Receiver<CaptureEvent>) -> CapturePoll {
        if let Some(fault) = self.status().fault {
            return CapturePoll::Fault(fault);
        }
        if self.stop.load(Ordering::Acquire) {
            return CapturePoll::Stopped;
        }
        let result = match receiver.try_recv() {
            Ok(event) => CapturePoll::Event(event),
            Err(TryRecvError::Empty) => CapturePoll::Idle,
            Err(TryRecvError::Disconnected) => {
                self.trip(CaptureFault::ThreadStopped);
                CapturePoll::Fault(CaptureFault::ThreadStopped)
            }
        };
        // Do not deliver a queued down after another callback reported lost input.
        self.fault().map_or(result, CapturePoll::Fault)
    }
}

fn emergency_chord(usage: u16, down: bool, modifiers: HidModifiers) -> bool {
    down && usage == 0x29
        && modifiers.control()
        && modifiers.alt()
        && modifiers.0 & (HidModifiers::LEFT_GUI | HidModifiers::RIGHT_GUI) != 0
}

#[cfg(windows)]
pub use native::WindowsInputCapture;

#[cfg(windows)]
#[allow(unsafe_code)]
mod native {
    use super::{
        AtomicBool, CaptureError, CaptureEvent, CaptureFault, CapturePoll, CaptureStatus, Duration,
        HidModifiers, Instant, MouseButton, Notify, Ordering, Receiver, Shared, SyncSender,
        emergency_chord,
    };
    use crate::{
        hid::{ScanCode, scan_code_to_keyboard_usage},
        input::is_own_injected_event,
    };
    use deskfuse_platform::DesktopState;
    use std::sync::Arc;
    use std::{
        cell::RefCell,
        ptr::null_mut,
        sync::mpsc::sync_channel,
        thread::{self, JoinHandle},
    };
    use windows_sys::Win32::{
        Foundation::{GetLastError, LPARAM, LRESULT, WAIT_FAILED, WPARAM},
        System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
        UI::{
            HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext},
            Input::KeyboardAndMouse::{
                GetAsyncKeyState, VK_LBUTTON, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN,
                VK_MBUTTON, VK_PAUSE, VK_RBUTTON, VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN,
                VK_XBUTTON1, VK_XBUTTON2,
            },
            WindowsAndMessaging::{
                CallNextHookEx, DispatchMessageW, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT,
                LLKHF_EXTENDED, LLKHF_INJECTED, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT,
                MWMO_INPUTAVAILABLE, MsgWaitForMultipleObjectsEx, PM_NOREMOVE, PM_REMOVE,
                PeekMessageW, PostThreadMessageW, QS_ALLINPUT, SetWindowsHookExW, TranslateMessage,
                UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_APP, WM_KEYDOWN, WM_KEYUP,
                WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL,
                WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_QUIT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN,
                WM_SYSKEYUP, WM_XBUTTONDOWN, WM_XBUTTONUP,
            },
        },
    };

    static CAPTURE_ACTIVE: AtomicBool = AtomicBool::new(false);
    const STOP_MESSAGE: u32 = WM_APP + 0x234;
    thread_local! {
        static CONTEXT: RefCell<Option<HookContext>> = const { RefCell::new(None) };
        static SHARED: RefCell<Option<Arc<Shared>>> = const { RefCell::new(None) };
    }

    pub struct WindowsInputCapture {
        shared: Arc<Shared>,
        receiver: Receiver<CaptureEvent>,
        thread: Option<JoinHandle<()>>,
        thread_id: u32,
    }
    impl WindowsInputCapture {
        /// Install real hooks on a dedicated ordinary-user Default-desktop thread.
        /// Observation starts immediately; exclusive suppression requires a bounded lease.
        /// # Errors
        /// Returns invalid configuration, a native failure, or a latched capture fault.
        pub fn start(queue_capacity: usize) -> Result<Self, CaptureError> {
            Self::start_with_notification(queue_capacity, Arc::new(Notify::new()))
        }
        pub fn start_with_notification(
            queue_capacity: usize,
            notification: Arc<Notify>,
        ) -> Result<Self, CaptureError> {
            if !(16..=16_384).contains(&queue_capacity) {
                return Err(CaptureError::InvalidQueueCapacity);
            }
            if CAPTURE_ACTIVE
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                return Err(CaptureError::AlreadyRunning);
            }
            let mut shared = Shared::new();
            shared.notification = notification;
            let shared = Arc::new(shared);
            let worker_shared = Arc::clone(&shared);
            let (sender, receiver) = sync_channel(queue_capacity);
            let (ready_sender, ready_receiver) = sync_channel(1);
            let Ok(thread) = thread::Builder::new()
                .name("deskfuse-input-capture".into())
                .spawn(move || {
                    let _exit = ThreadExit(Arc::clone(&worker_shared));
                    SHARED.with(|cell| *cell.borrow_mut() = Some(Arc::clone(&worker_shared)));
                    let result = run_capture(&worker_shared, sender, &ready_sender);
                    if let Err(error) = result {
                        let _ = ready_sender.try_send(Err(error));
                    }
                    CONTEXT.with(|cell| *cell.borrow_mut() = None);
                    SHARED.with(|cell| *cell.borrow_mut() = None);
                })
            else {
                CAPTURE_ACTIVE.store(false, Ordering::Release);
                return Err(CaptureError::ThreadStartFailed);
            };
            let thread_id = match ready_receiver.recv_timeout(Duration::from_secs(5)) {
                Ok(Ok(id)) => id,
                Ok(Err(error)) => {
                    shared.stop.store(true, Ordering::Release);
                    let _ = thread.join();
                    return Err(error);
                }
                Err(_) => {
                    // A late worker remains observer-only. Detach instead of
                    // hanging construction on an OS call; it checks stop on return.
                    shared.stop.store(true, Ordering::Release);
                    return Err(CaptureError::StartupTimedOut);
                }
            };
            Ok(Self {
                shared,
                receiver,
                thread: Some(thread),
                thread_id,
            })
        }
        pub fn poll(&self) -> CapturePoll {
            self.shared.poll(&self.receiver)
        }
        pub fn status(&self) -> CaptureStatus {
            if self.thread.as_ref().is_some_and(JoinHandle::is_finished)
                && !self.shared.stop.load(Ordering::Acquire)
            {
                self.shared.trip(CaptureFault::ThreadStopped);
            }
            self.shared.status()
        }

        /// Begin bounded exclusive capture after verifying that no physical key or
        /// mouse button already delivered locally is held.
        /// # Errors
        /// Returns a fault or `SuppressionBlockedByHeldInput` when handoff is unsafe.
        pub fn begin_suppression_lease(&self, duration: Duration) -> Result<(), CaptureError> {
            self.shared.request_suppression(duration)
        }
        /// Renew an already-active lease. An elapsed lease cannot be revived.
        /// # Errors
        /// Returns `Stopped` after expiry; callers must obtain a fresh control session.
        pub fn renew_suppression_lease(&self, duration: Duration) -> Result<(), CaptureError> {
            if self.shared.renew_suppression(duration) {
                Ok(())
            } else {
                Err(CaptureError::Stopped)
            }
        }
        pub fn stop_suppression(&self) {
            self.shared.stop_suppression();
        }
        pub fn notification(&self) -> Arc<Notify> {
            Arc::clone(&self.shared.notification)
        }

        /// Stop observation and uninstall the owned hooks.
        /// # Errors
        /// Returns a worker-thread failure if shutdown could not join cleanly.
        pub fn stop(&mut self) -> Result<(), CaptureError> {
            self.shared.stop_suppression();
            self.shared.stop.store(true, Ordering::Release);
            // SAFETY: scalar message to the worker's own queue. A finished thread
            // may reject it; the worker also wakes/checks stop every 50ms.
            unsafe {
                PostThreadMessageW(self.thread_id, STOP_MESSAGE, 0, 0);
            }
            if let Some(thread) = self.thread.take()
                && thread.join().is_err()
            {
                return Err(CaptureError::Faulted(CaptureFault::ThreadStopped));
            }
            Ok(())
        }
    }
    impl Drop for WindowsInputCapture {
        fn drop(&mut self) {
            let _ = self.stop();
        }
    }

    struct ThreadExit(Arc<Shared>);
    impl Drop for ThreadExit {
        fn drop(&mut self) {
            self.0.alive.store(false, Ordering::Release);
            if !self.0.stop.load(Ordering::Acquire) {
                self.0.trip(CaptureFault::ThreadStopped);
            }
            CAPTURE_ACTIVE.store(false, Ordering::Release);
        }
    }
    struct Hook(HHOOK);
    impl Drop for Hook {
        fn drop(&mut self) {
            // SAFETY: owned hook installed on this still-live worker thread.
            unsafe {
                UnhookWindowsHookEx(self.0);
            }
        }
    }
    fn native_error(operation: &'static str) -> CaptureError {
        // SAFETY: immediate thread-local last-error read; no preconditions.
        CaptureError::NativeCall {
            operation,
            code: unsafe { GetLastError() },
        }
    }
    fn ordinary_desktop() -> bool {
        crate::session::session_capabilities().desktop == DesktopState::UserDesktop
    }

    fn run_capture(
        shared: &Arc<Shared>,
        sender: SyncSender<CaptureEvent>,
        ready: &SyncSender<Result<u32, CaptureError>>,
    ) -> Result<(), CaptureError> {
        if !ordinary_desktop() {
            return Err(CaptureError::DesktopUnavailable);
        }
        // SAFETY: changes only this dedicated thread's DPI context, never global state.
        if unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) }
            .is_null()
        {
            return Err(native_error("SetThreadDpiAwarenessContext"));
        }
        let mut message = MSG::default();
        // SAFETY: live writable MSG; null HWND selects this thread's queue.
        unsafe {
            PeekMessageW(&raw mut message, null_mut(), 0, 0, PM_NOREMOVE);
        }
        if shared.stop.load(Ordering::Acquire) {
            return Err(CaptureError::Stopped);
        }
        CONTEXT.with(|cell| *cell.borrow_mut() = Some(HookContext::new(shared, sender)));
        // SAFETY: null requests the borrowed module handle of the current executable.
        let module = unsafe { GetModuleHandleW(std::ptr::null()) };
        if module.is_null() {
            return Err(native_error("GetModuleHandleW"));
        }
        // SAFETY: callbacks have HOOKPROC ABI/static lifetime; LL hooks run on the
        // installing thread with its message loop. Module remains loaded.
        let keyboard = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), module, 0) };
        if keyboard.is_null() {
            return Err(native_error("SetWindowsHookExW keyboard"));
        }
        let _keyboard = Hook(keyboard);
        // SAFETY: same lifetime/ABI argument as keyboard hook above.
        let mouse = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), module, 0) };
        if mouse.is_null() {
            return Err(native_error("SetWindowsHookExW mouse"));
        }
        let _mouse = Hook(mouse);
        shared
            .heartbeat_ms
            .store(shared.now_ms(), Ordering::Release);
        shared.alive.store(true, Ordering::Release);
        // SAFETY: GetCurrentThreadId takes no pointers and returns this live worker.
        let id = unsafe { GetCurrentThreadId() };
        if ready.try_send(Ok(id)).is_err() {
            return Err(CaptureError::Stopped);
        }
        let mut last_desktop_probe = Instant::now();
        while !shared.stop.load(Ordering::Acquire) {
            shared
                .heartbeat_ms
                .store(shared.now_ms(), Ordering::Release);
            if shared.fault().is_some() {
                break;
            }
            if last_desktop_probe.elapsed() >= Duration::from_millis(100) {
                if !ordinary_desktop() {
                    shared.trip(CaptureFault::DesktopChanged);
                    break;
                }
                last_desktop_probe = Instant::now();
            }
            // Bound work per iteration so queued application messages cannot starve
            // desktop/stop checks. PeekMessage dispatches LL sent messages.
            for _ in 0..64 {
                // SAFETY: writable MSG and own thread queue; native pointers never escape.
                if unsafe { PeekMessageW(&raw mut message, null_mut(), 0, 0, PM_REMOVE) } == 0 {
                    break;
                }
                if message.message == STOP_MESSAGE || message.message == WM_QUIT {
                    shared.stop.store(true, Ordering::Release);
                    break;
                }
                // SAFETY: MSG initialized by PeekMessageW; standard loop dispatch.
                unsafe {
                    TranslateMessage(&raw const message);
                    DispatchMessageW(&raw const message);
                }
            }
            if shared.stop.load(Ordering::Acquire) {
                break;
            }
            // SAFETY: zero handles permits null pointer; finite wait services queued
            // input and avoids a polling spin. No callback ever waits on this call.
            if unsafe {
                MsgWaitForMultipleObjectsEx(
                    0,
                    std::ptr::null(),
                    50,
                    QS_ALLINPUT,
                    MWMO_INPUTAVAILABLE,
                )
            } == WAIT_FAILED
            {
                shared.trip(CaptureFault::MessageLoopFailed);
                return Err(native_error("MsgWaitForMultipleObjectsEx"));
            }
        }
        Ok(())
    }

    struct HookContext {
        shared: Arc<Shared>,
        sender: SyncSender<CaptureEvent>,
        keys: [bool; 256],
    }
    impl HookContext {
        fn new(shared: &Arc<Shared>, sender: SyncSender<CaptureEvent>) -> Self {
            let mut context = Self {
                shared: Arc::clone(shared),
                sender,
                keys: [false; 256],
            };
            for (index, vk) in [
                VK_LCONTROL,
                VK_LSHIFT,
                VK_LMENU,
                VK_LWIN,
                VK_RCONTROL,
                VK_RSHIFT,
                VK_RMENU,
                VK_RWIN,
            ]
            .into_iter()
            .enumerate()
            {
                // SAFETY: reads scalar virtual-key state, no injection or pointers.
                context.keys[0xe0 + index] = unsafe { GetAsyncKeyState(i32::from(vk)) < 0 };
            }
            if let Ok(mut suppression) = context.shared.suppression.lock() {
                suppression.physical_keys = context.keys;
                for (button, vk) in [
                    (MouseButton::Left, VK_LBUTTON),
                    (MouseButton::Right, VK_RBUTTON),
                    (MouseButton::Middle, VK_MBUTTON),
                    (MouseButton::Back, VK_XBUTTON1),
                    (MouseButton::Forward, VK_XBUTTON2),
                ] {
                    // SAFETY: reads scalar virtual-key state, no injection or pointers.
                    if unsafe { GetAsyncKeyState(i32::from(vk)) < 0 } {
                        suppression.physical_buttons |= match button {
                            MouseButton::Left => 1,
                            MouseButton::Right => 2,
                            MouseButton::Middle => 4,
                            MouseButton::Back => 8,
                            MouseButton::Forward => 16,
                        };
                    }
                }
            }
            context
        }

        fn modifiers(&self) -> HidModifiers {
            HidModifiers((0..8).fold(0, |bits, index| {
                bits | if self.keys[0xe0 + index] {
                    1 << index
                } else {
                    0
                }
            }))
        }
        fn emit(&self, event: CaptureEvent, suppress: bool) -> bool {
            self.shared.publish(&self.sender, event) && suppress
        }
        fn keyboard(&mut self, message: u32, data: &KBDLLHOOKSTRUCT) -> bool {
            // Capture physical keys only. Other accessibility/automation injection
            // is neither forwarded nor suppressed, and our marker never loops.
            if is_own_injected_event(data.dwExtraInfo) || data.flags & LLKHF_INJECTED != 0 {
                return false;
            }
            let down = match message {
                WM_KEYDOWN | WM_SYSKEYDOWN => true,
                WM_KEYUP | WM_SYSKEYUP => false,
                _ => return false,
            };
            let usage = if data.vkCode == u32::from(VK_PAUSE) {
                None
            } else {
                u16::try_from(data.scanCode).ok().and_then(|code| {
                    scan_code_to_keyboard_usage(ScanCode {
                        code,
                        extended: data.flags & LLKHF_EXTENDED != 0,
                    })
                })
            };
            let Some(usage) = usage else {
                self.shared.trip(CaptureFault::UnsupportedKeyboardMapping);
                return false;
            };
            let repeat = down && self.keys[usize::from(usage)];
            self.keys[usize::from(usage)] = down;
            let modifiers = self.modifiers();
            if emergency_chord(usage, down, modifiers) {
                self.shared.trip(CaptureFault::EmergencyReclaim);
                return false;
            }
            if modifiers.control() && modifiers.alt() && (self.keys[0x4c] || self.keys[0x63]) {
                self.shared.trip(CaptureFault::SecureAttentionRequested);
                return false;
            }
            let suppress = self.shared.observe_key(usage, down);
            if (0xe0..=0xe7).contains(&usage) {
                self.emit(CaptureEvent::Modifiers(modifiers), suppress)
            } else {
                self.emit(
                    CaptureEvent::Key {
                        usage,
                        down,
                        repeat,
                        modifiers,
                    },
                    suppress,
                )
            }
        }
        fn mouse(&self, message: u32, data: &MSLLHOOKSTRUCT) -> bool {
            if is_own_injected_event(data.dwExtraInfo) || data.flags & LLMHF_INJECTED != 0 {
                return false;
            }
            let x = data.pt.x;
            let y = data.pt.y;
            let event = match message {
                WM_MOUSEMOVE => CaptureEvent::MouseMove { x, y },
                WM_MOUSEWHEEL => CaptureEvent::Scroll {
                    vertical: i32::from(i16::from_le_bytes([
                        data.mouseData.to_le_bytes()[2],
                        data.mouseData.to_le_bytes()[3],
                    ])),
                    horizontal: 0,
                    x,
                    y,
                },
                WM_MOUSEHWHEEL => CaptureEvent::Scroll {
                    vertical: 0,
                    horizontal: i32::from(i16::from_le_bytes([
                        data.mouseData.to_le_bytes()[2],
                        data.mouseData.to_le_bytes()[3],
                    ])),
                    x,
                    y,
                },
                _ => {
                    let (button, down) = match message {
                        WM_LBUTTONDOWN => (MouseButton::Left, true),
                        WM_LBUTTONUP => (MouseButton::Left, false),
                        WM_RBUTTONDOWN => (MouseButton::Right, true),
                        WM_RBUTTONUP => (MouseButton::Right, false),
                        WM_MBUTTONDOWN => (MouseButton::Middle, true),
                        WM_MBUTTONUP => (MouseButton::Middle, false),
                        WM_XBUTTONDOWN | WM_XBUTTONUP => {
                            let button = match data.mouseData >> 16 {
                                1 => MouseButton::Back,
                                2 => MouseButton::Forward,
                                _ => return false,
                            };
                            (button, message == WM_XBUTTONDOWN)
                        }
                        _ => return false,
                    };
                    CaptureEvent::MouseButton { button, down, x, y }
                }
            };
            let suppress = match event {
                CaptureEvent::MouseButton { button, down, .. } => {
                    self.shared.observe_button(button, down)
                }
                _ => self.shared.should_suppress(),
            };
            self.emit(event, suppress)
        }
    }

    fn dispatch(callback: impl FnOnce(&mut HookContext) -> bool) -> bool {
        let shared = SHARED.with(|cell| cell.borrow().clone());
        let Some(shared) = shared else {
            return false;
        };
        if shared.fault().is_some() || shared.stop.load(Ordering::Acquire) {
            return false;
        }
        let mut suppress = false;
        // Never allow Rust panic/reentrant mutable access across the Win32 ABI.
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            CONTEXT.with(|cell| {
                let Ok(mut context) = cell.try_borrow_mut() else {
                    shared.trip(CaptureFault::CallbackFailed);
                    return;
                };
                if let Some(context) = context.as_mut() {
                    suppress = callback(context);
                }
            });
        }))
        .is_err()
        {
            shared.trip(CaptureFault::CallbackFailed);
            return false;
        }
        suppress && shared.fault().is_none()
    }
    unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == HC_ACTION.cast_signed()
            && lparam != 0
            && let Ok(message) = u32::try_from(wparam)
        {
            // SAFETY: Windows guarantees KBDLLHOOKSTRUCT for HC_ACTION, valid for
            // callback duration. We only borrow; no pointer is retained.
            let data = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
            if dispatch(|context| context.keyboard(message, data)) {
                return 1;
            }
        }
        // SAFETY: pass original hook arguments unchanged; null handle is ignored.
        unsafe { CallNextHookEx(null_mut(), code, wparam, lparam) }
    }
    unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == HC_ACTION.cast_signed()
            && lparam != 0
            && let Ok(message) = u32::try_from(wparam)
        {
            // SAFETY: Windows supplies MSLLHOOKSTRUCT for HC_ACTION for this call.
            let data = unsafe { &*(lparam as *const MSLLHOOKSTRUCT) };
            if dispatch(|context| context.mouse(message, data)) {
                return 1;
            }
        }
        // SAFETY: original arguments unchanged; no retained native pointers.
        unsafe { CallNextHookEx(null_mut(), code, wparam, lparam) }
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        use std::sync::mpsc::TryRecvError;

        // Construct callback state only. No hooks, GetAsyncKeyState, capture thread,
        // suppression, or SendInput calls are performed by these tests.
        fn context(capacity: usize) -> (HookContext, Receiver<CaptureEvent>) {
            let shared = Arc::new(Shared::new());
            shared.alive.store(true, Ordering::Release);
            let (sender, receiver) = sync_channel(capacity);
            (
                HookContext {
                    shared,
                    sender,
                    keys: [false; 256],
                },
                receiver,
            )
        }
        fn raw_key(scan: u32) -> KBDLLHOOKSTRUCT {
            KBDLLHOOKSTRUCT {
                scanCode: scan,
                ..KBDLLHOOKSTRUCT::default()
            }
        }
        #[test]
        fn callback_maps_physical_keys_and_independent_modifiers() {
            let (mut context, receiver) = context(8);
            let control = KBDLLHOOKSTRUCT {
                flags: LLKHF_EXTENDED,
                ..raw_key(0x1d)
            };
            assert!(!context.keyboard(WM_KEYDOWN, &control));
            assert_eq!(
                receiver.try_recv(),
                Ok(CaptureEvent::Modifiers(HidModifiers(
                    HidModifiers::RIGHT_CONTROL
                )))
            );
            assert!(!context.keyboard(WM_KEYDOWN, &raw_key(0x1e)));
            assert_eq!(
                receiver.try_recv(),
                Ok(CaptureEvent::Key {
                    usage: 4,
                    down: true,
                    repeat: false,
                    modifiers: HidModifiers(HidModifiers::RIGHT_CONTROL)
                })
            );
            assert!(!context.keyboard(WM_KEYDOWN, &raw_key(0x1e)));
            assert!(matches!(
                receiver.try_recv(),
                Ok(CaptureEvent::Key { repeat: true, .. })
            ));
            assert!(!context.keyboard(WM_KEYUP, &raw_key(0x1e)));
            assert!(matches!(
                receiver.try_recv(),
                Ok(CaptureEvent::Key {
                    usage: 4,
                    down: false,
                    ..
                })
            ));
        }
        #[test]
        fn injected_input_is_not_forwarded_or_suppressed() {
            let (mut context, receiver) = context(8);
            let own = KBDLLHOOKSTRUCT {
                dwExtraInfo: crate::input::INPUT_INJECTION_MARKER,
                ..raw_key(0x1e)
            };
            assert!(!context.keyboard(WM_KEYDOWN, &own));
            let foreign = KBDLLHOOKSTRUCT {
                flags: LLKHF_INJECTED,
                ..raw_key(0x1e)
            };
            assert!(!context.keyboard(WM_KEYDOWN, &foreign));
            assert_eq!(receiver.try_recv(), Err(TryRecvError::Empty));
        }
        #[test]
        fn callback_overflow_of_key_up_latches_reclaim() {
            let (mut context, receiver) = context(1);
            assert!(!context.keyboard(WM_KEYDOWN, &raw_key(0x1e)));
            assert!(!context.keyboard(WM_KEYUP, &raw_key(0x1e)));
            assert_eq!(
                context.shared.poll(&receiver),
                CapturePoll::Fault(CaptureFault::QueueOverflow)
            );
            assert!(!context.shared.status().suppressing);
        }
        #[test]
        fn observe_ctrl_down_then_suppression_request_never_swallows_ctrl_up() {
            let (mut context, receiver) = context(8);
            assert!(!context.keyboard(WM_KEYDOWN, &raw_key(0x1d)));
            assert_eq!(
                context
                    .shared
                    .request_suppression(Duration::from_millis(500)),
                Err(CaptureError::SuppressionBlockedByHeldInput)
            );
            assert!(!context.keyboard(WM_KEYUP, &raw_key(0x1d)));
            assert_eq!(
                receiver.try_recv(),
                Ok(CaptureEvent::Modifiers(HidModifiers(
                    HidModifiers::LEFT_CONTROL
                )))
            );
            assert_eq!(
                receiver.try_recv(),
                Ok(CaptureEvent::Modifiers(HidModifiers(0)))
            );
            assert!(!context.shared.status().suppressing);
        }

        #[test]
        fn observe_button_down_then_suppression_request_never_swallows_button_up() {
            let (context, receiver) = context(8);
            let mouse = MSLLHOOKSTRUCT::default();
            assert!(!context.mouse(WM_LBUTTONDOWN, &mouse));
            assert_eq!(
                context
                    .shared
                    .request_suppression(Duration::from_millis(500)),
                Err(CaptureError::SuppressionBlockedByHeldInput)
            );
            assert!(!context.mouse(WM_LBUTTONUP, &mouse));
            assert!(matches!(
                receiver.try_recv(),
                Ok(CaptureEvent::MouseButton {
                    button: MouseButton::Left,
                    down: true,
                    ..
                })
            ));
            assert!(matches!(
                receiver.try_recv(),
                Ok(CaptureEvent::MouseButton {
                    button: MouseButton::Left,
                    down: false,
                    ..
                })
            ));
        }

        #[test]
        fn keys_held_before_capture_cannot_become_suppression_owned() {
            let (mut context, receiver) = context(8);
            // Model the startup snapshot of a Ctrl already delivered locally.
            context.keys[0xe0] = true;
            context.shared.suppression.lock().unwrap().physical_keys[0xe0] = true;
            assert_eq!(
                context
                    .shared
                    .request_suppression(Duration::from_millis(500)),
                Err(CaptureError::SuppressionBlockedByHeldInput)
            );
            assert!(!context.keyboard(WM_KEYUP, &raw_key(0x1d)));
            assert_eq!(
                receiver.try_recv(),
                Ok(CaptureEvent::Modifiers(HidModifiers(0)))
            );
        }

        #[test]
        fn exclusive_capture_suppresses_only_successfully_forwarded_events() {
            let (mut context, receiver) = context(1);
            assert_eq!(
                context
                    .shared
                    .request_suppression(Duration::from_millis(500)),
                Ok(())
            );
            assert!(context.keyboard(WM_KEYDOWN, &raw_key(0x1e)));
            assert!(!context.keyboard(WM_KEYUP, &raw_key(0x1e)));
            assert_eq!(
                context.shared.poll(&receiver),
                CapturePoll::Fault(CaptureFault::QueueOverflow)
            );
            assert!(!context.shared.status().suppressing);
        }

        #[test]
        fn emergency_and_pause_are_not_forwarded_as_ordinary_keys() {
            let (mut context, receiver) = context(8);
            context.keys[0xe0] = true;
            context.keys[0xe2] = true;
            context.keys[0xe3] = true;
            assert!(!context.keyboard(WM_KEYDOWN, &raw_key(1)));
            assert_eq!(
                context.shared.poll(&receiver),
                CapturePoll::Fault(CaptureFault::EmergencyReclaim)
            );
            let (mut pause_context, _) = self::context(8);
            let pause = KBDLLHOOKSTRUCT {
                vkCode: u32::from(VK_PAUSE),
                ..raw_key(0x45)
            };
            assert!(!pause_context.keyboard(WM_KEYDOWN, &pause));
            assert_eq!(
                pause_context.shared.fault(),
                Some(CaptureFault::UnsupportedKeyboardMapping)
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::sync_channel;
    fn key(down: bool) -> CaptureEvent {
        CaptureEvent::Key {
            usage: 4,
            down,
            repeat: false,
            modifiers: HidModifiers(0),
        }
    }
    #[test]
    fn suppression_is_bounded_and_cannot_start_with_held_input() {
        let shared = Shared::new();
        shared.alive.store(true, Ordering::Release);
        assert_eq!(
            shared.request_suppression(Duration::ZERO),
            Err(CaptureError::SuppressionBlockedByHeldInput)
        );
        assert_eq!(
            shared.request_suppression(Duration::from_millis(500)),
            Ok(())
        );
        assert_eq!(shared.status().mode, CaptureMode::Exclusive);
        assert!(shared.status().suppressing);
        assert!(shared.renew_suppression(Duration::from_millis(500)));
        shared.stop_suppression();
        assert_eq!(shared.status().mode, CaptureMode::ObserverOnly);
        assert!(!shared.renew_suppression(Duration::from_millis(500)));
        shared.suppression.lock().unwrap().physical_buttons = 1;
        assert_eq!(
            shared.request_suppression(Duration::from_millis(500)),
            Err(CaptureError::SuppressionBlockedByHeldInput)
        );
    }
    #[test]
    fn lost_key_up_has_out_of_band_fault_and_discards_queued_down() {
        let shared = Shared::new();
        shared.alive.store(true, Ordering::Release);
        let (sender, receiver) = sync_channel(1);
        assert!(shared.publish(&sender, key(true)));
        assert!(!shared.publish(&sender, key(false)));
        assert_eq!(
            shared.poll(&receiver),
            CapturePoll::Fault(CaptureFault::QueueOverflow)
        );
        assert_eq!(
            shared.poll(&receiver),
            CapturePoll::Fault(CaptureFault::QueueOverflow)
        );
        assert!(!shared.publish(&sender, key(true)));
        assert!(!shared.status().suppressing);
    }
    #[test]
    fn consumer_disconnect_and_stop_never_suppress() {
        let shared = Shared::new();
        shared.alive.store(true, Ordering::Release);
        let (sender, receiver) = sync_channel(1);
        drop(receiver);
        assert!(!shared.publish(&sender, key(false)));
        assert_eq!(shared.fault(), Some(CaptureFault::ConsumerDisconnected));
        assert!(!shared.status().suppressing);
        shared.stop.store(true, Ordering::Release);
        assert!(!shared.status().suppressing);
    }
    #[test]
    fn emergency_requires_ctrl_alt_gui_escape_on_down() {
        let modifiers = HidModifiers(
            HidModifiers::RIGHT_CONTROL | HidModifiers::LEFT_ALT | HidModifiers::RIGHT_GUI,
        );
        assert!(emergency_chord(0x29, true, modifiers));
        assert!(!emergency_chord(0x29, false, modifiers));
        assert!(!emergency_chord(4, true, modifiers));
        assert!(!emergency_chord(
            0x29,
            true,
            HidModifiers(modifiers.0 & !HidModifiers::RIGHT_GUI)
        ));
    }
}
