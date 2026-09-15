//! Process-local diagnostic collection policy. Never persisted or shared with peers.
use std::sync::atomic::{AtomicBool, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Hot paths check this before formatting, sampling clocks, or locking log stores.
#[inline]
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Applies only to this process; a new app launch always starts with collection off.
pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}
