//! Native adapters implement these contracts; shared coordination owns policy and leases.
use crate::{PlatformError, PlatformResult};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Notify;
use unidesk_core::{DisplayDescriptor, InputSessionId, SharedInputEvent};
use unidesk_input::sharing::HotkeyBindings;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureMode {
    Observe,
    Exclusive,
}
#[derive(Clone, Debug, PartialEq)]
pub struct CaptureSample {
    pub captured_at: Instant,
    pub event: SharedInputEvent,
    pub global_x: f64,
    pub global_y: f64,
    pub delta_x: f64,
    pub delta_y: f64,
}
#[derive(Clone, Debug, PartialEq)]
pub enum CaptureEvent {
    Input(CaptureSample),
    TapDisabled,
    Overflow,
    LeaseExpired,
}

pub trait SharedInputCapture: Send {
    /// # Errors
    /// Returns an error when native capture cannot be installed or permission is missing.
    fn start(&mut self) -> PlatformResult<()>;
    /// # Errors
    /// Returns an error when the native capture backend cannot shut down cleanly.
    fn stop(&mut self) -> PlatformResult<()>;
    fn set_mode(&self, mode: CaptureMode);
    fn set_hotkeys(&self, hotkeys: HotkeyBindings);
    fn arm_session(&self, session: InputSessionId, duration: Duration) -> bool;
    fn renew_deadline(&self, duration: Duration) -> bool;
    fn lease_expired(&self) -> bool;
    fn set_controller_hotkeys_enabled(&self, enabled: bool);
    /// # Errors
    /// Returns an error when cursor capture or positioning fails.
    fn enter_remote(&self, x: f64, y: f64) -> PlatformResult<()>;
    /// # Errors
    /// Returns an error when releasing capture or restoring the cursor fails.
    fn exit_remote(&self, restore: Option<(f64, f64)>) -> PlatformResult<()>;
    fn event_notification(&self) -> Arc<Notify>;
    /// # Errors
    /// Returns a native capture or event-queue error.
    fn next_event(&mut self) -> PlatformResult<Option<CaptureEvent>>;
}
pub trait SharedInputInjector: Send {
    fn arm_session(&self, session: InputSessionId, duration: Duration) -> bool;
    fn renew_deadline(&self, duration: Duration) -> bool;
    fn lease_expired(&self) -> bool;
    /// # Errors
    /// Returns an error for an invalid session, unsupported display or failed native injection.
    fn enqueue_batch(
        &self,
        session: InputSessionId,
        display: &DisplayDescriptor,
        state_epoch: u64,
        events: &[SharedInputEvent],
    ) -> PlatformResult<()>;
    fn error_notification(&self) -> Arc<Notify>;
    fn take_error(&self) -> Option<PlatformError>;
    /// # Errors
    /// Returns an error when the native backend cannot release held input state.
    fn release_all(&self) -> PlatformResult<()>;
}
