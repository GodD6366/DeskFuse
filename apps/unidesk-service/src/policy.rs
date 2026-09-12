//! Pure session gate. Session identifiers come only from kernel/token queries,
//! never from the IPC request. A session event invalidates all outstanding work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientSession {
    pub pipe_session: u32,
    pub token_session: u32,
    pub logon_id: u64,
    pub active_console_session: u32,
    pub interactive: bool,
    pub unlocked: bool,
    pub local_system: bool,
}
impl ClientSession {
    #[must_use]
    pub fn allowed(self) -> bool {
        self.pipe_session != 0
            && self.pipe_session != u32::MAX
            && self.pipe_session == self.token_session
            && self.pipe_session == self.active_console_session
            && self.interactive
            && self.unlocked
            && !self.local_system
    }
}
#[must_use]
pub fn same_session(
    before: ClientSession,
    after: ClientSession,
    epoch: u64,
    current_epoch: u64,
) -> bool {
    before.allowed() && after.allowed() && before == after && epoch == current_epoch
}
#[cfg(test)]
mod tests {
    use super::*;
    fn active() -> ClientSession {
        ClientSession {
            pipe_session: 1,
            token_session: 1,
            logon_id: 100,
            active_console_session: 1,
            interactive: true,
            unlocked: true,
            local_system: false,
        }
    }
    #[test]
    fn only_current_unlocked_interactive_session_is_allowed() {
        assert!(active().allowed());
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
                local_system: true,
                ..active()
            },
        ] {
            assert!(!bad.allowed());
        }
    }
    #[test]
    fn lock_logoff_switch_and_epoch_change_revoke_inflight_request() {
        assert!(same_session(active(), active(), 4, 4));
        assert!(!same_session(active(), active(), 4, 5));
        assert!(!same_session(
            active(),
            ClientSession {
                logon_id: 101,
                ..active()
            },
            4,
            4
        ));
        assert!(!same_session(
            active(),
            ClientSession {
                unlocked: false,
                ..active()
            },
            4,
            4
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
            4
        ));
    }
}
