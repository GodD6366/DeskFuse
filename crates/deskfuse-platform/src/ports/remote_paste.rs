//! Command-V intent for remote clipboard content that is still in flight.
//!
//! The native keyboard boundary only records intent (non-blocking `try_lock`);
//! the engine decides when the content has landed and whether to replay.
use std::sync::Mutex;

#[derive(Default)]
struct State {
    /// Local pasteboard change count the in-flight content will replace.
    baseline: Option<i64>,
    /// Foreground application captured at the suppressed Command-V.
    target_pid: Option<i32>,
}

/// Thread-safe state shared by a host's native paste interception.
#[derive(Default)]
pub struct RemoteClipboardPaste {
    state: Mutex<State>,
}

impl RemoteClipboardPaste {
    /// Mark content in flight against `baseline`, or nothing in flight.
    pub fn set_pending(&self, baseline: Option<i64>) {
        if let Ok(mut state) = self.state.lock() {
            state.baseline = baseline;
        }
    }

    /// Record paste intent when the local pasteboard is still the one the
    /// in-flight content will replace. Never blocks the event tap.
    pub fn request(&self, pid: i32, change_count: i64) -> bool {
        let Ok(mut state) = self.state.try_lock() else {
            return false;
        };
        if state.baseline != Some(change_count) {
            return false;
        }
        state.target_pid = Some(pid);
        true
    }

    pub fn requested(&self) -> bool {
        self.state
            .lock()
            .is_ok_and(|state| state.target_pid.is_some())
    }

    /// End the in-flight phase and take any captured intent. Call only after
    /// the pasteboard write so a Command-V either sees the new change count or
    /// is captured here.
    pub fn complete(&self) -> Option<i32> {
        let mut state = self.state.lock().ok()?;
        state.baseline = None;
        state.target_pid.take()
    }

    /// Drop the captured intent while the transfer continues (user cancelled).
    pub fn cancel(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.target_pid = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intent_requires_matching_baseline_and_ends_on_complete() {
        let paste = RemoteClipboardPaste::default();
        assert!(!paste.request(7, 10));
        paste.set_pending(Some(10));
        assert!(
            !paste.request(7, 11),
            "a newer local copy must paste normally"
        );
        assert!(paste.request(7, 10));
        assert_eq!(paste.complete(), Some(7));
        assert!(!paste.request(7, 10), "completed content pastes normally");
        assert_eq!(paste.complete(), None);
    }
}
