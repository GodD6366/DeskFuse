//! Native macOS global input capture and injection.
//!
//! Capture uses a Accessibility-authorized `CGEventTap` on a dedicated Core Foundation run
//! loop and requires the macOS **Accessibility** TCC grant. Injection posts
//! synthetic `CGEvent`s to the HID tap location and requires the **辅助功能 /
//! Accessibility** TCC grant. Both boundaries fail explicitly rather than
//! silently pretending an event reached the operating system.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use core_foundation::base::TCFType;
use core_foundation::runloop::{CFRunLoop, kCFRunLoopCommonModes};
use core_foundation_sys::mach_port::CFMachPortInvalidate;
use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    CGEventType, CGMouseButton, EventField, ScrollEventUnit,
};

/// Marker placed on UniDesk's synthetic Command-V events so the interception
/// tap can distinguish them from a user's first paste request.
pub const SYNTHETIC_PASTE_EVENT_MARKER: i64 = 0x5544_5041;

/// Post Command-V after an intercepted remote-file preparation completes.
#[allow(unsafe_code)]
pub fn post_paste_shortcut() -> PlatformResult<()> {
    if unsafe { CGPreflightPostEventAccess() } == 0 {
        return Err(PlatformError::InputPermissionRequired);
    }
    let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
        .map_err(|()| PlatformError::InputInjectionFailed)?;
    for pressed in [true, false] {
        let event = CGEvent::new_keyboard_event(source.clone(), 9, pressed)
            .map_err(|()| PlatformError::InputInjectionFailed)?;
        event.set_flags(CGEventFlags::CGEventFlagCommand);
        event.set_integer_value_field(
            EventField::EVENT_SOURCE_USER_DATA,
            SYNTHETIC_PASTE_EVENT_MARKER,
        );
        event.post(CGEventTapLocation::Session);
    }
    Ok(())
}
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::CGPoint;

use crate::{
    ButtonState, GlobalInputCapture, InputEvent, InputInjector, KeyCode, MouseButton,
    PlatformError, PlatformResult,
};

#[allow(unsafe_code)]
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    /// Returns whether the process may post synthetic HID events
    /// (Accessibility / 辅助功能).
    fn CGPreflightPostEventAccess() -> u8;
}

/// Native macOS input injector backed by `CGEvent` HID posting.
///
/// A fresh `CGEventSource` is created per posted event: the source is a
/// process-wide HID state handle and this keeps the injector `Send` without
/// asserting thread safety for an opaque Core Graphics object.
#[derive(Clone, Copy, Debug, Default)]
pub struct MacOSInputInjector;

impl MacOSInputInjector {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    fn source() -> PlatformResult<CGEventSource> {
        CGEventSource::new(CGEventSourceStateID::HIDSystemState)
            .map_err(|()| PlatformError::InputInjectionFailed)
    }
}

impl InputInjector for MacOSInputInjector {
    #[allow(unsafe_code)]
    fn inject(&mut self, event: InputEvent) -> PlatformResult<()> {
        if unsafe { CGPreflightPostEventAccess() } == 0 {
            return Err(PlatformError::InputPermissionRequired);
        }
        match event {
            InputEvent::MouseMoved { x, y } => {
                let source = Self::source()?;
                let event = CGEvent::new_mouse_event(
                    source,
                    CGEventType::MouseMoved,
                    CGPoint::new(x, y),
                    CGMouseButton::Left,
                )
                .map_err(|()| PlatformError::InputInjectionFailed)?;
                event.set_integer_value_field(
                    EventField::EVENT_SOURCE_USER_DATA,
                    crate::sharing::SYNTHETIC_SHARING_EVENT_MARKER,
                );
                event.post(CGEventTapLocation::HID);
                Ok(())
            }
            InputEvent::MouseButton { button, state } => {
                let pressed = state == ButtonState::Pressed;
                let (event_type, native_button, explicit_number) = match (button, pressed) {
                    (MouseButton::Left, true) => {
                        (CGEventType::LeftMouseDown, CGMouseButton::Left, None)
                    }
                    (MouseButton::Left, false) => {
                        (CGEventType::LeftMouseUp, CGMouseButton::Left, None)
                    }
                    (MouseButton::Right, true) => {
                        (CGEventType::RightMouseDown, CGMouseButton::Right, None)
                    }
                    (MouseButton::Right, false) => {
                        (CGEventType::RightMouseUp, CGMouseButton::Right, None)
                    }
                    (MouseButton::Middle, true) => (
                        CGEventType::OtherMouseDown,
                        CGMouseButton::Center,
                        Some(2_i64),
                    ),
                    (MouseButton::Middle, false) => (
                        CGEventType::OtherMouseUp,
                        CGMouseButton::Center,
                        Some(2_i64),
                    ),
                    (MouseButton::Other(number), true) => (
                        CGEventType::OtherMouseDown,
                        CGMouseButton::Center,
                        Some(i64::from(number)),
                    ),
                    (MouseButton::Other(number), false) => (
                        CGEventType::OtherMouseUp,
                        CGMouseButton::Center,
                        Some(i64::from(number)),
                    ),
                };
                let position = CGPoint::new(0.0, 0.0);
                let source = Self::source()?;
                let event = CGEvent::new_mouse_event(source, event_type, position, native_button)
                    .map_err(|()| PlatformError::InputInjectionFailed)?;
                if let Some(number) = explicit_number {
                    event.set_integer_value_field(EventField::MOUSE_EVENT_BUTTON_NUMBER, number);
                }
                event.set_integer_value_field(
                    EventField::EVENT_SOURCE_USER_DATA,
                    crate::sharing::SYNTHETIC_SHARING_EVENT_MARKER,
                );
                event.post(CGEventTapLocation::HID);
                Ok(())
            }
            InputEvent::Key {
                code: KeyCode::Raw(keycode),
                state,
            } => {
                let pressed = state == ButtonState::Pressed;
                let source = Self::source()?;
                let event = CGEvent::new_keyboard_event(source, keycode, pressed)
                    .map_err(|()| PlatformError::InputInjectionFailed)?;
                event.set_integer_value_field(
                    EventField::EVENT_SOURCE_USER_DATA,
                    crate::sharing::SYNTHETIC_SHARING_EVENT_MARKER,
                );
                event.post(CGEventTapLocation::HID);
                Ok(())
            }
            InputEvent::Key {
                code: KeyCode::Character(_),
                ..
            } => Err(PlatformError::Unsupported {
                operation: "character-key injection (capture emits raw key codes)",
            }),
            InputEvent::Scroll { delta_x, delta_y } => {
                let source = Self::source()?;
                let event = CGEvent::new_scroll_event(
                    source,
                    ScrollEventUnit::LINE,
                    2,
                    delta_y.round() as i32,
                    delta_x.round() as i32,
                    0,
                )
                .map_err(|()| PlatformError::InputInjectionFailed)?;
                event.set_integer_value_field(
                    EventField::EVENT_SOURCE_USER_DATA,
                    crate::sharing::SYNTHETIC_SHARING_EVENT_MARKER,
                );
                event.post(CGEventTapLocation::HID);
                Ok(())
            }
        }
    }
}

/// Native macOS global input capture backed by a Accessibility-authorized `CGEventTap`.
pub struct MacOSGlobalInputCapture {
    events: std::sync::mpsc::Receiver<InputEvent>,
    runloop: Arc<Mutex<Option<CFRunLoop>>>,
    thread: Option<JoinHandle<()>>,
    active: AtomicBool,
}

impl Default for MacOSGlobalInputCapture {
    fn default() -> Self {
        Self::new()
    }
}

impl MacOSGlobalInputCapture {
    #[must_use]
    pub fn new() -> Self {
        Self {
            events: std::sync::mpsc::channel().1,
            runloop: Arc::new(Mutex::new(None)),
            thread: None,
            active: AtomicBool::new(false),
        }
    }
}

impl std::fmt::Debug for MacOSGlobalInputCapture {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MacOSGlobalInputCapture")
            .field("active", &self.active.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl GlobalInputCapture for MacOSGlobalInputCapture {
    #[allow(unsafe_code)]
    fn start(&mut self) -> PlatformResult<()> {
        if self.active.load(Ordering::Relaxed) {
            return Ok(());
        }
        if unsafe { CGPreflightPostEventAccess() } == 0 {
            return Err(PlatformError::InputPermissionRequired);
        }
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        let runloop = Arc::clone(&self.runloop);
        let thread = thread::Builder::new()
            .name("unidesk-input-tap".to_owned())
            .spawn(move || capture_thread(runloop, ready_tx, event_tx))
            .map_err(|_| PlatformError::InputCaptureUnavailable)?;
        match ready_rx.recv_timeout(Duration::from_secs(3)) {
            Ok(Ok(())) => {
                self.events = event_rx;
                self.thread = Some(thread);
                self.active.store(true, Ordering::Relaxed);
                Ok(())
            }
            Ok(Err(())) | Err(_) => {
                let _ = thread.join();
                Err(PlatformError::InputCaptureUnavailable)
            }
        }
    }

    fn stop(&mut self) -> PlatformResult<()> {
        if !self.active.swap(false, Ordering::Relaxed) {
            return Ok(());
        }
        if let Some(runloop) = self.runloop.lock().ok().and_then(|guard| guard.clone()) {
            runloop.stop();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        Ok(())
    }

    fn is_active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }

    fn next_event(&mut self) -> PlatformResult<Option<InputEvent>> {
        if !self.active.load(Ordering::Relaxed) {
            return Ok(None);
        }
        match self.events.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(std::sync::mpsc::TryRecvError::Empty) => Ok(None),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Err(PlatformError::InputCaptureUnavailable)
            }
        }
    }
}

impl Drop for MacOSGlobalInputCapture {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[allow(unsafe_code)]
fn capture_thread(
    runloop_state: Arc<Mutex<Option<CFRunLoop>>>,
    ready: std::sync::mpsc::Sender<Result<(), ()>>,
    events: std::sync::mpsc::Sender<InputEvent>,
) {
    let runloop = CFRunLoop::get_current();
    let event_types = vec![
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
        CGEventType::ScrollWheel,
    ];
    let tap = match CGEventTap::new(
        CGEventTapLocation::Session,
        CGEventTapPlacement::HeadInsertEventTap,
        CGEventTapOptions::Default,
        event_types,
        move |_proxy, event_type, event| {
            if let Some(event) = convert_captured_event(event_type, event) {
                // A full channel is not fatal: capture degrades by dropping
                // the sample, exactly like a lagging remote-input consumer.
                let _ = events.send(event);
            }
            None
        },
    ) {
        Ok(tap) => tap,
        Err(()) => {
            let _ = ready.send(Err(()));
            return;
        }
    };
    let source = match tap.mach_port.create_runloop_source(0) {
        Ok(source) => source,
        Err(()) => {
            let _ = ready.send(Err(()));
            return;
        }
    };
    // SAFETY: `kCFRunLoopCommonModes` is a well-known immutable Core
    // Foundation string constant used by every run loop mode.
    unsafe {
        runloop.add_source(&source, kCFRunLoopCommonModes);
    }
    tap.enable();
    if let Ok(mut guard) = runloop_state.lock() {
        *guard = Some(runloop.clone());
    }
    let _ = ready.send(Ok(()));
    CFRunLoop::run_current();
    // Invalidating the event-tap mach port disables the tap before its
    // callback storage is dropped, so the system can never call into a
    // dangling closure after `stop`.
    // SAFETY: `mach_port` is the valid CFMachPort owned by `tap`; invalidation
    // is idempotent and safe to call exactly once during teardown.
    unsafe {
        CFMachPortInvalidate(tap.mach_port.as_concrete_TypeRef());
    }
}

fn convert_captured_event(event_type: CGEventType, event: &CGEvent) -> Option<InputEvent> {
    let marker = event.get_integer_value_field(EventField::EVENT_SOURCE_USER_DATA);
    if marker == SYNTHETIC_PASTE_EVENT_MARKER
        || marker == crate::sharing::SYNTHETIC_SHARING_EVENT_MARKER
    {
        return None;
    }
    match event_type {
        CGEventType::MouseMoved
        | CGEventType::LeftMouseDragged
        | CGEventType::RightMouseDragged
        | CGEventType::OtherMouseDragged => {
            let location = event.location();
            Some(InputEvent::MouseMoved {
                x: location.x,
                y: location.y,
            })
        }
        CGEventType::LeftMouseDown => Some(InputEvent::MouseButton {
            button: MouseButton::Left,
            state: ButtonState::Pressed,
        }),
        CGEventType::LeftMouseUp => Some(InputEvent::MouseButton {
            button: MouseButton::Left,
            state: ButtonState::Released,
        }),
        CGEventType::RightMouseDown => Some(InputEvent::MouseButton {
            button: MouseButton::Right,
            state: ButtonState::Pressed,
        }),
        CGEventType::RightMouseUp => Some(InputEvent::MouseButton {
            button: MouseButton::Right,
            state: ButtonState::Released,
        }),
        CGEventType::OtherMouseDown => {
            let number = event.get_integer_value_field(EventField::MOUSE_EVENT_BUTTON_NUMBER);
            let button = if number == 2 {
                MouseButton::Middle
            } else {
                MouseButton::Other(number.clamp(0, u16::MAX as i64) as u16)
            };
            Some(InputEvent::MouseButton {
                button,
                state: ButtonState::Pressed,
            })
        }
        CGEventType::OtherMouseUp => {
            let number = event.get_integer_value_field(EventField::MOUSE_EVENT_BUTTON_NUMBER);
            let button = if number == 2 {
                MouseButton::Middle
            } else {
                MouseButton::Other(number.clamp(0, u16::MAX as i64) as u16)
            };
            Some(InputEvent::MouseButton {
                button,
                state: ButtonState::Released,
            })
        }
        CGEventType::KeyDown => {
            let keycode = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE);
            Some(InputEvent::Key {
                code: KeyCode::Raw(keycode.clamp(0, u16::MAX as i64) as u16),
                state: ButtonState::Pressed,
            })
        }
        CGEventType::KeyUp => {
            let keycode = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE);
            Some(InputEvent::Key {
                code: KeyCode::Raw(keycode.clamp(0, u16::MAX as i64) as u16),
                state: ButtonState::Released,
            })
        }
        CGEventType::ScrollWheel => Some(InputEvent::Scroll {
            delta_y: event
                .get_double_value_field(EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_1),
            delta_x: event
                .get_double_value_field(EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_2),
        }),
        _ => None,
    }
}

#[cfg(test)]
#[allow(unsafe_code)]
mod tests {
    use super::*;

    #[test]
    fn injector_is_unavailable_without_accessibility_permission() {
        // The permission probe is environment-dependent; the injector must
        // never claim success. `new` can succeed without the grant but
        // `inject` must refuse unless the OS says posting is allowed.
        let mut injector = MacOSInputInjector::new();
        if unsafe { CGPreflightPostEventAccess() } == 0 {
            assert_eq!(
                injector.inject(InputEvent::MouseMoved { x: 0.0, y: 0.0 }),
                Err(PlatformError::InputPermissionRequired)
            );
        }
    }

    #[test]
    fn character_key_injection_is_explicitly_unsupported() {
        let mut injector = MacOSInputInjector::new();
        if unsafe { CGPreflightPostEventAccess() } == 0 {
            return;
        }
        assert!(matches!(
            injector.inject(InputEvent::Key {
                code: KeyCode::Character('a'),
                state: ButtonState::Pressed,
            }),
            Err(PlatformError::Unsupported { .. })
        ));
    }
}
