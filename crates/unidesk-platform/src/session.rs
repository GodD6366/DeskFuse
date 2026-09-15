//! Platform-neutral, fail-closed session and desktop capability reporting.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DesktopState {
    UserDesktop,
    Locked,
    /// Winlogon covers both sign-in and the UAC secure desktop; the GUI cannot
    /// safely distinguish these simply by reading a desktop name.
    LoginOrSecureDesktop,
    InactiveSession,
    RemoteSession,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SessionBlocker {
    SessionQueryFailed,
    SessionNotActive,
    SessionLocked,
    RemoteSession,
    InputDesktopUnavailable,
    DifferentDesktop,
    ProtectedDesktop,
    /// Full remote-input capability is gated on the service authorization and
    /// desktop-specific helper, neither of which this read-only probe provides.
    AuthorizedInputServiceRequired,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
// Independent native facts, not alternative mutually exclusive states.
#[allow(clippy::struct_excessive_bools)]
pub struct SessionCapabilities {
    pub session_id: Option<u32>,
    pub desktop: DesktopState,
    /// This only indicates that local clipboard/file work is permissible for
    /// the current user session. It does not grant access to another user's data.
    pub user_content_allowed: bool,
    pub remote_input_allowed: bool,
    pub login_input_available: bool,
    pub secure_desktop_input_available: bool,
    pub blockers: Vec<SessionBlocker>,
    pub native_error: Option<u32>,
}

impl SessionCapabilities {
    /// Construct a fail-closed capability snapshot after a native probe failed.
    /// No user-content or input capability is granted. The authorized input
    /// service requirement remains visible alongside the specific probe blocker.
    #[must_use]
    pub fn unavailable(
        session_id: Option<u32>,
        blocker: SessionBlocker,
        error: Option<u32>,
    ) -> Self {
        Self {
            session_id,
            desktop: DesktopState::Unavailable,
            user_content_allowed: false,
            remote_input_allowed: false,
            login_input_available: false,
            secure_desktop_input_available: false,
            blockers: vec![blocker, SessionBlocker::AuthorizedInputServiceRequired],
            native_error: error,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unavailable_never_grants_content_or_remote_input() {
        let c = SessionCapabilities::unavailable(Some(1), SessionBlocker::SessionLocked, Some(5));
        assert!(!c.user_content_allowed);
        assert!(!c.remote_input_allowed);
        assert!(!c.login_input_available);
        assert!(!c.secure_desktop_input_available);
        assert!(
            c.blockers
                .contains(&SessionBlocker::AuthorizedInputServiceRequired)
        );
    }
}
