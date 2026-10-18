//! Input lease policy. Native posting stays in the host; this crate never captures HID.
use crate::{
    api::{CollaborationEvent, InputStatus},
    error::{CollaborationError, Result},
};
use deskfuse_core::{DeviceId, SharedInputEvent};
use deskfuse_platform::ports::InputExecutionPort;
use std::{collections::BTreeSet, sync::Arc, time::Duration};
use tokio::sync::{RwLock, mpsc};

#[derive(Default)]
struct LeaseState {
    controller: Option<DeviceId>,
    pressed_keys: BTreeSet<u16>,
    generation: u64,
}

/// Coordinates a single active input lease and releases keys on expiry.
pub struct InputCoordinator {
    execution: Arc<dyn InputExecutionPort>,
    state: Arc<RwLock<LeaseState>>,
    events: mpsc::UnboundedSender<CollaborationEvent>,
}

impl InputCoordinator {
    pub fn new(
        execution: Arc<dyn InputExecutionPort>,
        events: mpsc::UnboundedSender<CollaborationEvent>,
    ) -> Self {
        Self {
            execution,
            state: Arc::new(RwLock::new(LeaseState::default())),
            events,
        }
    }

    pub async fn status(&self) -> InputStatus {
        let state = self.state.read().await;
        InputStatus {
            controller: state.controller,
            lease_active: state.controller.is_some(),
        }
    }

    pub async fn has_active_lease(&self) -> bool {
        self.state.read().await.controller.is_some()
    }

    pub async fn pressed_keys(&self) -> BTreeSet<u16> {
        self.state.read().await.pressed_keys.clone()
    }

    pub async fn grant_lease(&self, device_id: DeviceId, duration: Duration) -> Result<()> {
        let generation = {
            let mut state = self.state.write().await;
            state.controller = Some(device_id);
            state.pressed_keys.clear();
            state.generation += 1;
            state.generation
        };
        let _ = self
            .events
            .send(CollaborationEvent::InputControlGranted { device_id });
        self.spawn_watchdog(generation, duration);
        Ok(())
    }

    pub async fn release_lease(&self, reason: impl Into<String>) -> Result<()> {
        self.expire(reason.into()).await
    }

    pub async fn handle_input(&self, event: SharedInputEvent) -> Result<()> {
        if !self.has_active_lease().await {
            return Err(CollaborationError::LeaseExpired);
        }
        if let SharedInputEvent::Key { code, pressed, .. } = event {
            let mut state = self.state.write().await;
            if pressed {
                state.pressed_keys.insert(code);
            } else {
                state.pressed_keys.remove(&code);
            }
        }
        self.execution.execute(event).await?;
        Ok(())
    }

    fn spawn_watchdog(&self, generation: u64, duration: Duration) {
        let state = Arc::clone(&self.state);
        let execution = Arc::clone(&self.execution);
        let events = self.events.clone();
        tokio::spawn(async move {
            tokio::time::sleep(duration).await;
            let controller = {
                let mut lease = state.write().await;
                if lease.generation != generation || lease.controller.is_none() {
                    return;
                }
                lease.controller.take();
                lease.pressed_keys.clear();
                lease.controller
            };
            let _ = execution.release_all().await;
            let _ = events.send(CollaborationEvent::InputControlRevoked {
                reason: "lease expired".into(),
            });
            let _ = controller;
        });
    }

    async fn expire(&self, reason: String) -> Result<()> {
        {
            let mut state = self.state.write().await;
            state.controller = None;
            state.pressed_keys.clear();
            state.generation += 1;
        }
        self.execution.release_all().await?;
        let _ = self
            .events
            .send(CollaborationEvent::InputControlRevoked { reason });
        Ok(())
    }
}
