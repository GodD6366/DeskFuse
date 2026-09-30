//! Fail-closed contract for a future Windows secure-session agent.
//!
//! The service runs in Session 0 and cannot turn its ordinary desktop helper
//! into a Winlogon/UAC agent.  This module is the only service-side state that
//! may expose such capabilities.  It starts unavailable and can only be
//! admitted by a separately authenticated agent channel after every binding
//! fact is checked.  No SAS or credential operation is represented here.

use crate::desktop_protocol::SessionAgentState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AgentDesktop {
    Locked,
    UacSecure,
    Login,
}

/// Facts that must come from the service-authenticated agent channel. Values
/// supplied by the ordinary desktop pipe or a peer are not sufficient.
#[derive(Clone, Copy, Debug)]
pub(super) struct AgentObservation {
    pub service_epoch: u64,
    pub agent_epoch: u64,
    pub session_id: u32,
    pub active_console_session: u32,
    pub token_session: u32,
    pub process_authenticated: bool,
    pub signed_installation: bool,
    pub explicit_user_authorization: bool,
    pub desktop: AgentDesktop,
    pub can_input: bool,
}

#[derive(Clone)]
pub(super) struct SessionAgent {
    state: SessionAgentState,
}

impl Default for SessionAgent {
    fn default() -> Self {
        let mut agent = Self {
            state: SessionAgentState::default(),
        };
        agent.clear("no authenticated secure session agent is installed");
        agent
    }
}

impl SessionAgent {
    pub(super) fn snapshot(&self) -> SessionAgentState {
        self.state.clone()
    }

    /// Drop all agent authority on epoch, session, process or desktop changes.
    pub(super) fn clear(&mut self, reason: &'static str) {
        self.state = SessionAgentState {
            reason: Some(reason.into()),
            ..SessionAgentState::default()
        };
    }

    /// Admit only a complete, explicit authorization record. In particular,
    /// an authenticated process alone is not authorization and an agent cannot
    /// claim SAS support. The service currently has no production agent
    /// channel, so this method is intentionally not called by ordinary IPC.
    pub(super) fn admit(&mut self, observation: AgentObservation) {
        let session_bound = observation.session_id == observation.active_console_session
            && observation.session_id == observation.token_session
            && observation.service_epoch == observation.agent_epoch;
        let authorized = session_bound
            && observation.process_authenticated
            && observation.signed_installation
            && observation.explicit_user_authorization
            && observation.can_input;
        if !authorized {
            self.clear("secure session agent authorization is incomplete");
            return;
        }
        self.state = SessionAgentState {
            connected: true,
            authorized: true,
            secure_desktop_input: matches!(
                observation.desktop,
                AgentDesktop::Locked | AgentDesktop::UacSecure
            ),
            locked_desktop_input: matches!(observation.desktop, AgentDesktop::Locked),
            login_input: matches!(observation.desktop, AgentDesktop::Login),
            // Windows owns Ctrl+Alt+Del/SAS. Even an authorized agent never
            // turns this into a capability or an input event.
            secure_attention_sequence: false,
            reason: Some(
                match observation.desktop {
                    AgentDesktop::Locked => "authorized lock-screen agent",
                    AgentDesktop::UacSecure => "authorized UAC secure-desktop agent",
                    AgentDesktop::Login => "authorized login agent",
                }
                .into(),
            ),
        };
    }

    pub(super) fn secure_desktop_input_available(&self) -> bool {
        self.state.secure_desktop_input
    }

    pub(super) fn login_input_available(&self) -> bool {
        self.state.login_input
    }

    pub(super) fn secure_attention_sequence_available(&self) -> bool {
        self.state.secure_attention_sequence
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(explicit_user_authorization: bool) -> AgentObservation {
        AgentObservation {
            service_epoch: 9,
            agent_epoch: 9,
            session_id: 3,
            active_console_session: 3,
            token_session: 3,
            process_authenticated: true,
            signed_installation: true,
            explicit_user_authorization,
            desktop: AgentDesktop::Locked,
            can_input: true,
        }
    }

    #[test]
    fn agent_starts_fail_closed() {
        let agent = SessionAgent::default();
        let state = agent.snapshot();
        assert!(!state.connected);
        assert!(!state.authorized);
        assert!(!state.secure_desktop_input);
        assert!(!state.login_input);
        assert!(!state.secure_attention_sequence);
    }

    #[test]
    fn authenticated_process_without_user_authorization_is_rejected() {
        let mut agent = SessionAgent::default();
        agent.admit(observation(false));
        assert!(!agent.snapshot().authorized);
        assert!(!agent.secure_desktop_input_available());
        assert!(!agent.login_input_available());
        assert!(!agent.secure_attention_sequence_available());
    }

    #[test]
    fn locked_agent_never_gets_sas_capability() {
        let mut agent = SessionAgent::default();
        agent.admit(observation(true));
        let state = agent.snapshot();
        assert!(state.authorized);
        assert!(state.secure_desktop_input);
        assert!(state.locked_desktop_input);
        assert!(!state.login_input);
        assert!(!state.secure_attention_sequence);
    }
}
