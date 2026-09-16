//! Workflow extension. The collaboration core does not depend on this crate.
#![forbid(unsafe_code)]

pub mod executor;
pub mod sharing;

pub use executor::{Workflow, WorkflowError, WorkflowExecutor, WorkflowOutput};

use std::sync::Arc;
use unidesk_collaboration::CollaborationCore;
use uuid::Uuid;

pub trait HotkeyManager: Send + Sync {
    fn bind(&self, workflow_id: Uuid, combination: &str) -> Result<(), String>;
}

pub struct NoopHotkeyManager;

impl HotkeyManager for NoopHotkeyManager {
    fn bind(&self, _workflow_id: Uuid, _combination: &str) -> Result<(), String> {
        Ok(())
    }
}

pub struct WorkflowExtension {
    core: Arc<CollaborationCore>,
    executor: WorkflowExecutor,
}

impl WorkflowExtension {
    pub fn new(core: Arc<CollaborationCore>, _hotkeys: Arc<dyn HotkeyManager>) -> Self {
        Self {
            core,
            executor: WorkflowExecutor::new(),
        }
    }

    pub async fn start(&self) -> unidesk_collaboration::Result<()> {
        Ok(())
    }

    pub async fn stop(&self) {}

    pub fn executor_mut(&mut self) -> &mut WorkflowExecutor {
        &mut self.executor
    }

    pub async fn execute(
        &self,
        workflow_id: Uuid,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let result = self.executor.run(workflow_id)?;
        sharing::share_output(&self.core, &result).await?;
        Ok(())
    }
}
