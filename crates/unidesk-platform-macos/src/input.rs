use std::collections::VecDeque;

use crate::{PlatformError, PlatformResult};

/// Mouse buttons used by the platform boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Other(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ButtonState {
    Pressed,
    Released,
}

/// A framework-independent input event.  Coordinates use the platform's
/// global display coordinate space and are intentionally not tied to a window.
#[derive(Debug, Clone, PartialEq)]
pub enum InputEvent {
    MouseMoved {
        x: f64,
        y: f64,
    },
    MouseButton {
        button: MouseButton,
        state: ButtonState,
    },
    Key {
        code: KeyCode,
        state: ButtonState,
    },
    Scroll {
        delta_x: f64,
        delta_y: f64,
    },
}

/// Small key-code boundary.  The native implementation can extend this enum
/// without exposing CoreGraphics or a UI framework to the rest of the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyCode {
    Character(char),
    Raw(u16),
}

/// Future global input capture boundary.
pub trait GlobalInputCapture: Send {
    fn start(&mut self) -> PlatformResult<()>;
    fn stop(&mut self) -> PlatformResult<()>;
    fn is_active(&self) -> bool;

    /// Poll one captured event.  Returning `None` means that no event is
    /// available now; implementations need not block.
    fn next_event(&mut self) -> PlatformResult<Option<InputEvent>>;
}

/// Future input injection boundary.  A real implementation must be explicit
/// and permission-aware; this crate's test implementation never posts events.
pub trait InputInjector: Send {
    fn inject(&mut self, event: InputEvent) -> PlatformResult<()>;
}

/// Explicit no-op capture implementation.  It does not install a hook or
/// read from the operating system.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopGlobalInputCapture {
    active: bool,
}

impl NoopGlobalInputCapture {
    pub const fn new() -> Self {
        Self { active: false }
    }
}

impl GlobalInputCapture for NoopGlobalInputCapture {
    fn start(&mut self) -> PlatformResult<()> {
        self.active = true;
        Ok(())
    }

    fn stop(&mut self) -> PlatformResult<()> {
        self.active = false;
        Ok(())
    }

    fn is_active(&self) -> bool {
        self.active
    }

    fn next_event(&mut self) -> PlatformResult<Option<InputEvent>> {
        Ok(None)
    }
}

/// In-memory capture used by tests.  Events must be explicitly queued by the
/// test; no native event tap is created.
#[derive(Debug, Default, Clone)]
pub struct MemoryGlobalInputCapture {
    active: bool,
    events: VecDeque<InputEvent>,
}

impl MemoryGlobalInputCapture {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_event(&mut self, event: InputEvent) {
        self.events.push_back(event);
    }

    pub fn pending_events(&self) -> usize {
        self.events.len()
    }

    pub fn clear(&mut self) {
        self.events.clear();
    }
}

impl GlobalInputCapture for MemoryGlobalInputCapture {
    fn start(&mut self) -> PlatformResult<()> {
        self.active = true;
        Ok(())
    }

    fn stop(&mut self) -> PlatformResult<()> {
        self.active = false;
        Ok(())
    }

    fn is_active(&self) -> bool {
        self.active
    }

    fn next_event(&mut self) -> PlatformResult<Option<InputEvent>> {
        if self.active {
            Ok(self.events.pop_front())
        } else {
            Ok(None)
        }
    }
}

/// Explicit no-op input injector.  It always returns an error so callers
/// cannot accidentally believe that an event reached the OS.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopInputInjector;

impl NoopInputInjector {
    pub const fn new() -> Self {
        Self
    }
}

impl InputInjector for NoopInputInjector {
    fn inject(&mut self, _event: InputEvent) -> PlatformResult<()> {
        Err(PlatformError::InputInjectionDisabled)
    }
}

/// In-memory injector for tests.  It records attempted events for assertions,
/// then returns an error to make it explicit that no native input was sent.
#[derive(Debug, Default, Clone)]
pub struct MemoryInputInjector {
    attempted_events: Vec<InputEvent>,
}

impl MemoryInputInjector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn attempted_events(&self) -> &[InputEvent] {
        &self.attempted_events
    }

    pub fn clear(&mut self) {
        self.attempted_events.clear();
    }

    /// Record an event without going through the `InputInjector` contract.
    /// This is useful when a test is checking translation rather than testing
    /// the intentionally-disabled injection path.
    pub fn record(&mut self, event: InputEvent) {
        self.attempted_events.push(event);
    }
}

impl InputInjector for MemoryInputInjector {
    fn inject(&mut self, event: InputEvent) -> PlatformResult<()> {
        self.record(event);
        Err(PlatformError::InputInjectionDisabled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_capture_only_yields_events_when_started() {
        let mut capture = MemoryGlobalInputCapture::new();
        capture.push_event(InputEvent::MouseMoved { x: 1.0, y: 2.0 });
        assert_eq!(capture.next_event().expect("memory capture works"), None);
        capture.start().expect("memory capture works");
        assert_eq!(
            capture.next_event().expect("memory capture works"),
            Some(InputEvent::MouseMoved { x: 1.0, y: 2.0 })
        );
    }

    #[test]
    fn memory_injector_records_but_never_injects() {
        let mut injector = MemoryInputInjector::new();
        let event = InputEvent::MouseButton {
            button: MouseButton::Left,
            state: ButtonState::Pressed,
        };
        assert_eq!(
            injector.inject(event.clone()),
            Err(PlatformError::InputInjectionDisabled)
        );
        assert_eq!(injector.attempted_events(), &[event]);
    }
}
