use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Workflow {
    pub id: Uuid,
    pub name: String,
    pub share_with_devices: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowOutput {
    pub text: String,
    pub share_with_devices: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum WorkflowError {
    #[error("workflow {0} was not found")]
    NotFound(Uuid),
    #[error("{0}")]
    Failed(String),
}

pub type Result<T> = std::result::Result<T, WorkflowError>;

/// Executes a workflow definition. Native hotkeys stay in the host.
#[derive(Debug, Default)]
pub struct WorkflowExecutor {
    workflows: Vec<Workflow>,
}

impl WorkflowExecutor {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, workflow: Workflow) {
        self.workflows.push(workflow);
    }

    pub fn run(&self, workflow_id: Uuid) -> Result<WorkflowOutput> {
        let workflow = self
            .workflows
            .iter()
            .find(|workflow| workflow.id == workflow_id)
            .ok_or(WorkflowError::NotFound(workflow_id))?;
        Ok(WorkflowOutput {
            text: workflow.name.clone(),
            share_with_devices: workflow.share_with_devices,
        })
    }
}
