//! Native admission to ordinary-desktop input adapters, not peer authorization.
use crate::DesktopState;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputBlockerKind {
    AccessibilityRequired,
    InputMonitoringRequired,
    InactiveDesktop,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputBlocker {
    pub kind: InputBlockerKind,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputCapability {
    /// Native prerequisites permit attempting adapter startup. Only successful
    /// capture/inject startup proves readiness; this is not a network grant.
    Allowed,
    Blocked(InputBlocker),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputEnvironmentSnapshot {
    pub desktop: DesktopState,
    pub capture: InputCapability,
    pub inject: InputCapability,
}
impl InputEnvironmentSnapshot {
    pub fn blocker(&self) -> Option<&InputBlocker> {
        match (&self.capture, &self.inject) {
            (InputCapability::Blocked(blocker), _) | (_, InputCapability::Blocked(blocker)) => {
                Some(blocker)
            }
            _ => None,
        }
    }

    pub fn blocked(desktop: DesktopState, kind: InputBlockerKind, reason: impl Into<String>) -> Self {
        let capability = InputCapability::Blocked(InputBlocker { kind, reason: reason.into() });
        Self { desktop, capture: capability.clone(), inject: capability }
    }
}

/// Probe in the process/session that owns the native adapters. A UI process
/// must not substitute its own permissions for its helper's observations.
/// Snapshots are ephemeral: consumers must reject stale observations and still
/// retain lease, peer admission, startup and asynchronous-fault checks.
/// No implementation of this port grants Winlogon, UAC, SAS or credential input.
pub trait InputEnvironmentPort: Send + Sync {
    fn snapshot(&self) -> InputEnvironmentSnapshot;
}
