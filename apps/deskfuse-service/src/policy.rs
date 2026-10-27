//! Pure session gate. Session identifiers come only from kernel/token queries,
//! never from the IPC request. A session event invalidates all outstanding work.
use serde::{Deserialize, Serialize};

/// User-consented relaxation of the session gate. Defaults are strict: a locked
/// console or a remote (RDP) session is rejected unless the user enabled the
/// corresponding switch from a fully present local session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionPolicy {
    /// Keep serving clipboard/file/status while the console session is locked.
    #[serde(default)]
    pub allow_locked: bool,
    /// Allow sessions whose protocol is not the physical console (RDP).
    #[serde(default)]
    pub allow_remote: bool,
}
impl SessionPolicy {
    pub const STRICT: Self = Self {
        allow_locked: false,
        allow_remote: false,
    };
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientSession {
    pub pipe_session: u32,
    pub token_session: u32,
    pub logon_id: u64,
    pub user_sid: [u8; 68],
    pub active_console_session: u32,
    pub interactive: bool,
    pub unlocked: bool,
    pub physical_console: bool,
    pub local_system: bool,
}
impl ClientSession {
    #[must_use]
    pub fn allowed(self, policy: SessionPolicy) -> bool {
        self.pipe_session != 0
            && self.pipe_session != u32::MAX
            && self.pipe_session == self.token_session
            && self.pipe_session == self.active_console_session
            && self.interactive
            && !self.local_system
            && (self.unlocked || policy.allow_locked)
            && (self.physical_console || policy.allow_remote)
    }
    /// Input injection, new pairing trust and policy changes always require the
    /// physically present, unlocked console session regardless of policy.
    #[must_use]
    pub fn fully_present(self) -> bool {
        self.allowed(SessionPolicy::STRICT)
    }
}
#[must_use]
pub fn same_session(
    before: ClientSession,
    after: ClientSession,
    epoch: u64,
    current_epoch: u64,
    policy: SessionPolicy,
) -> bool {
    before.allowed(policy) && after.allowed(policy) && before == after && epoch == current_epoch
}
#[cfg(test)]
mod tests {
    use super::*;
    fn active() -> ClientSession {
        ClientSession {
            pipe_session: 1,
            token_session: 1,
            logon_id: 100,
            user_sid: [0; 68],
            active_console_session: 1,
            interactive: true,
            unlocked: true,
            physical_console: true,
            local_system: false,
        }
    }
    #[test]
    fn only_current_unlocked_interactive_session_is_allowed() {
        assert!(active().allowed(SessionPolicy::STRICT));
        for bad in [
            ClientSession {
                pipe_session: 0,
                ..active()
            },
            ClientSession {
                token_session: 2,
                ..active()
            },
            ClientSession {
                active_console_session: 2,
                ..active()
            },
            ClientSession {
                interactive: false,
                ..active()
            },
            ClientSession {
                unlocked: false,
                ..active()
            },
            ClientSession {
                physical_console: false,
                ..active()
            },
            ClientSession {
                local_system: true,
                ..active()
            },
        ] {
            assert!(!bad.allowed(SessionPolicy::STRICT));
        }
    }
    #[test]
    fn policy_relaxes_only_the_named_session_fact() {
        let locked = ClientSession {
            unlocked: false,
            ..active()
        };
        let remote = ClientSession {
            physical_console: false,
            ..active()
        };
        let allow_locked = SessionPolicy {
            allow_locked: true,
            allow_remote: false,
        };
        let allow_remote = SessionPolicy {
            allow_locked: false,
            allow_remote: true,
        };
        assert!(locked.allowed(allow_locked));
        assert!(!locked.allowed(allow_remote));
        assert!(!locked.fully_present());
        assert!(remote.allowed(allow_remote));
        assert!(!remote.allowed(allow_locked));
        assert!(!remote.fully_present());
        // Policy never relaxes identity, interactivity or console ownership.
        let wrong_user = ClientSession {
            token_session: 2,
            unlocked: false,
            ..active()
        };
        assert!(!wrong_user.allowed(SessionPolicy {
            allow_locked: true,
            allow_remote: true,
        }));
    }
    #[test]
    fn lock_logoff_switch_and_epoch_change_revoke_inflight_request() {
        assert!(same_session(
            active(),
            active(),
            4,
            4,
            SessionPolicy::STRICT
        ));
        assert!(!same_session(
            active(),
            active(),
            4,
            5,
            SessionPolicy::STRICT
        ));
        assert!(!same_session(
            active(),
            ClientSession {
                logon_id: 101,
                ..active()
            },
            4,
            4,
            SessionPolicy::STRICT
        ));
        assert!(!same_session(
            active(),
            ClientSession {
                unlocked: false,
                ..active()
            },
            4,
            4,
            SessionPolicy::STRICT
        ));
        assert!(!same_session(
            active(),
            ClientSession {
                pipe_session: 2,
                token_session: 2,
                active_console_session: 2,
                ..active()
            },
            4,
            4,
            SessionPolicy::STRICT
        ));
    }
    #[test]
    fn relaxed_session_stays_comparable_across_request() {
        let policy = SessionPolicy {
            allow_locked: true,
            allow_remote: false,
        };
        let locked = ClientSession {
            unlocked: false,
            ..active()
        };
        assert!(same_session(locked, locked, 4, 4, policy));
        assert!(!same_session(locked, locked, 4, 4, SessionPolicy::STRICT));
    }
}
