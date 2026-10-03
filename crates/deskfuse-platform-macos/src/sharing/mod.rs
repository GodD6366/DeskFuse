//! Production input-sharing boundary. Native objects never leave their owning
//! threads. Capture and injection each enforce a deadline independently of GPUI.
#![allow(unsafe_code)]
mod background_cursor;
mod capture;
mod environment;
mod injector;
pub use environment::MacOSInputEnvironment;
pub mod timing;
use crate::{PlatformError, PlatformResult};
pub use capture::{
    CaptureEvent, CaptureMode, CaptureSample, MacOSSharingCapture,
    restore_pointer_after_service_exit, warp_pointer,
};
pub use injector::MacOSSharingInjector;
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

/// Distinct from the file-paste marker; every synthetic sharing event is tagged.
pub const SYNTHETIC_SHARING_EVENT_MARKER: i64 = 0x5544_5348;
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightPostEventAccess() -> u8;
}
fn may_post() -> bool {
    // SAFETY: the preflight function has no arguments and does not request TCC.
    unsafe { CGPreflightPostEventAccess() != 0 }
}

/// The input helper runs Tokio, not an AppKit application run loop. Query the
/// WindowServer process state synchronously instead of NSWorkspace's cached
/// running-application properties, which update on AppKit run-loop turns.
pub fn frontmost_pid() -> i32 {
    #[repr(C)]
    struct ProcessSerialNumber {
        high: u32,
        low: u32,
    }
    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn GetFrontProcess(process: *mut ProcessSerialNumber) -> i32;
        fn GetProcessPID(process: *const ProcessSerialNumber, pid: *mut i32) -> i32;
    }
    let mut process = ProcessSerialNumber { high: 0, low: 0 };
    let mut pid = 0;
    // SAFETY: both legacy public Process Manager APIs receive valid stack
    // output pointers. An unavailable foreground process fails closed.
    unsafe {
        if GetFrontProcess(&mut process) == 0 && GetProcessPID(&process, &mut pid) == 0 {
            return pid.max(0);
        }
    }
    0
}

pub fn can_post_input() -> bool {
    may_post()
}
fn may_capture() -> bool {
    // The production capture uses CGEventTapOptions::Default so it can
    // suppress local input while controlling a peer. macOS authorizes this
    // modifying tap with Accessibility, not the listen-only keyboard grant.
    // The actual CGEventTapCreate result remains the final authority.
    may_post()
}
#[repr(C)]
struct MachTimebase {
    numer: u32,
    denom: u32,
}
unsafe extern "C" {
    fn mach_continuous_time() -> u64;
    fn mach_timebase_info(info: *mut MachTimebase) -> i32;
}
#[derive(Default)]
struct DeadlineState {
    current: Option<deskfuse_core::InputSessionId>,
    paused: bool,
    retired: std::collections::BTreeSet<deskfuse_core::InputSessionId>,
}
struct Deadline {
    until: AtomicU64,
    state: std::sync::Mutex<DeadlineState>,
}
impl Deadline {
    fn new() -> Self {
        Self {
            until: AtomicU64::new(0),
            state: std::sync::Mutex::new(DeadlineState::default()),
        }
    }
    fn now_millis() -> u64 {
        static TIMEBASE: std::sync::OnceLock<(u32, u32)> = std::sync::OnceLock::new();
        let &(numer, denom) = TIMEBASE.get_or_init(|| {
            let mut info = MachTimebase { numer: 0, denom: 0 };
            // SAFETY: Mach writes two u32 fields to the valid stack struct.
            if unsafe { mach_timebase_info(&mut info) } == 0 && info.denom != 0 {
                (info.numer, info.denom)
            } else {
                (0, 1)
            }
        });
        // SAFETY: no arguments; unlike uptime this system clock includes sleep.
        let ticks = unsafe { mach_continuous_time() };
        if numer == 0 {
            return u64::MAX;
        }
        ((u128::from(ticks) * u128::from(numer)) / u128::from(denom) / 1_000_000)
            .min(u128::from(u64::MAX)) as u64
    }
    fn arm(&self, session: deskfuse_core::InputSessionId, duration: Duration) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        let now = Self::now_millis();
        if duration.is_zero() || now == u64::MAX {
            return false;
        }
        if let Some(previous) = state.current
            && !state.paused
            && now >= self.until.load(Ordering::Acquire)
        {
            state.retired.insert(previous);
        }
        if state.retired.contains(&session) {
            return false;
        }
        state.current = Some(session);
        state.paused = false;
        self.until.store(
            now.saturating_add(duration.min(Duration::from_millis(1500)).as_millis() as u64),
            Ordering::Release,
        );
        true
    }
    /// Renewal cannot revive an elapsed epoch, even if the watchdog thread
    /// has not been scheduled to observe the elapsed deadline yet.
    fn renew(&self, duration: Duration) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        let now = Self::now_millis();
        if state.paused
            || state.current.is_none()
            || now >= self.until.load(Ordering::Acquire)
            || duration.is_zero()
        {
            if !state.paused
                && let Some(session) = state.current
            {
                state.retired.insert(session);
            }
            self.until.store(0, Ordering::Release);
            return false;
        }
        self.until.store(
            now.saturating_add(duration.min(Duration::from_millis(1500)).as_millis() as u64),
            Ordering::Release,
        );
        true
    }
    fn expired(&self) -> bool {
        Self::now_millis() >= self.until.load(Ordering::Acquire)
    }
    /// Leave pauses native injection without revoking a still-valid network
    /// lease. Re-entry can explicitly arm that session again.
    fn pause(&self) {
        if let Ok(mut state) = self.state.lock() {
            if !state.paused
                && self.expired()
                && let Some(session) = state.current
            {
                state.retired.insert(session);
            }
            state.paused = true;
        }
        self.until.store(0, Ordering::Release);
    }
    /// Errors/TTL expiration revoke the epoch; that session ID stays retired.
    fn clear(&self) {
        if let Ok(mut state) = self.state.lock() {
            if let Some(session) = state.current {
                state.retired.insert(session);
            }
            state.current = None;
            state.paused = false;
        }
        self.until.store(0, Ordering::Release);
    }
}

fn permission_result() -> PlatformResult<()> {
    if may_post() {
        Ok(())
    } else {
        Err(PlatformError::InputPermissionRequired)
    }
}

/// Whether this process's session owns the unlocked console. Missing or
/// unreadable session dictionaries are treated as inactive.
pub fn session_is_active() -> bool {
    use core_foundation::{
        base::TCFType, boolean::CFBoolean, dictionary::CFDictionary, string::CFString,
    };
    use core_foundation_sys::{
        base::CFTypeRef,
        dictionary::{CFDictionaryGetValue, CFDictionaryRef},
    };
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGSessionCopyCurrentDictionary() -> CFDictionaryRef;
    }
    // SAFETY: Copy returns a nullable owned dictionary for the current session.
    let raw = unsafe { CGSessionCopyCurrentDictionary() };
    if raw.is_null() {
        return false;
    }
    // SAFETY: raw is non-null and follows the Copy rule.
    let dict: CFDictionary = unsafe { CFDictionary::wrap_under_create_rule(raw) };
    let value = |name: &str| -> Option<bool> {
        let key = CFString::new(name);
        // SAFETY: dictionary and key are valid retained CF objects. The returned
        // value is borrowed, used only while the dictionary remains alive.
        let raw = unsafe { CFDictionaryGetValue(dict.as_concrete_TypeRef(), key.as_CFTypeRef()) };
        if raw.is_null() {
            return None;
        }
        // SAFETY: compare the dynamic CF type before wrapping it as a boolean.
        if unsafe { core_foundation_sys::base::CFGetTypeID(raw as CFTypeRef) }
            != CFBoolean::type_id()
        {
            return None;
        }
        // SAFETY: the checked boolean value is retained under the Get rule.
        Some(unsafe { CFBoolean::wrap_under_get_rule(raw.cast()) }.into())
    };
    value("kCGSSessionOnConsoleKey") == Some(true)
        && value("CGSSessionScreenIsLocked") != Some(true)
}

// Capture and injection must stay responsive while the app is in the background.
fn prioritize_current_thread() {
    use objc2_foundation::{NSQualityOfService, NSThread};
    NSThread::currentThread().setQualityOfService(NSQualityOfService::UserInteractive);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expired_epoch_cannot_be_renewed_or_rearmed() {
        let d = Deadline::new();
        let id = deskfuse_core::InputSessionId::new();
        assert!(!d.renew(Duration::from_millis(1500)));
        assert!(d.arm(id, Duration::from_secs(30)));
        assert!(d.until.load(Ordering::Acquire) <= Deadline::now_millis() + 1500);
        d.until
            .store(Deadline::now_millis().saturating_sub(1), Ordering::Release);
        assert!(!d.renew(Duration::from_millis(1500)));
        assert!(!d.arm(id, Duration::from_millis(1500)));
        assert!(d.arm(
            deskfuse_core::InputSessionId::new(),
            Duration::from_millis(1500)
        ));
    }
    #[test]
    fn leave_reentry_keeps_valid_network_session_but_expired_leave_does_not() {
        let d = Deadline::new();
        let id = deskfuse_core::InputSessionId::new();
        assert!(d.arm(id, Duration::from_millis(1500)));
        d.pause();
        assert!(d.arm(id, Duration::from_millis(1500)));
        d.until
            .store(Deadline::now_millis().saturating_sub(1), Ordering::Release);
        d.pause();
        assert!(!d.arm(id, Duration::from_millis(1500)));
    }
    #[test]
    fn revocation_retires_session_and_old_session_cannot_follow_new_epoch() {
        let d = Deadline::new();
        let old = deskfuse_core::InputSessionId::new();
        assert!(d.arm(old, Duration::from_millis(1500)));
        d.clear();
        assert!(d.arm(
            deskfuse_core::InputSessionId::new(),
            Duration::from_millis(1500)
        ));
        assert!(!d.arm(old, Duration::from_millis(1500)));
    }
    #[test]
    fn controller_can_visit_multiple_preleased_peers_and_return() {
        let d = Deadline::new();
        let a = deskfuse_core::InputSessionId::new();
        let b = deskfuse_core::InputSessionId::new();
        assert!(d.arm(a, Duration::from_millis(1500)));
        d.pause();
        assert!(d.arm(b, Duration::from_millis(1500)));
        d.pause();
        assert!(d.arm(a, Duration::from_millis(1500)));
    }
}
