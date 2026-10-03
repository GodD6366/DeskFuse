//! Desktop host assembly. Collaboration owns policy; this crate injects Host Ports.
use deskfuse_collaboration::{CollaborationCore, MemoryTransport};
use deskfuse_platform::ports::{EnginePlatformPort, HostPorts};
use std::{path::PathBuf, sync::Arc};

/// Application-process host. Input capture/injection stays in the input subprocess.
pub struct EngineHost {
    core: CollaborationCore,
    ports: HostPorts,
    workflow_execution: Arc<dyn deskfuse_platform::WorkflowExecutionPort>,
}

impl EngineHost {
    /// Assemble around the existing application-support directory.
    ///
    /// # Errors
    /// Returns a path or native-port failure. This does not start clipboard polling;
    /// [`crate::tokio_bridge::TokioBridge`] still owns the production clipboard loop
    /// until that worker is cut over.
    pub fn assemble() -> Result<Self, String> {
        let data_dir = deskfuse_store::SQLiteStore::default_path()
            .map_err(|error| error.to_string())?
            .parent()
            .map(PathBuf::from)
            .ok_or_else(|| "state directory is missing".to_string())?;
        Self::assemble_in(data_dir)
    }

    /// # Errors
    /// Returns a filesystem or native-port failure.
    pub fn assemble_in(data_dir: PathBuf) -> Result<Self, String> {
        let platform = crate::platform::memory_platform();
        let ports = HostPorts::memory(data_dir.clone());
        Self::assemble_with(data_dir, ports, platform)
    }

    /// Assemble the engine with application-owned native ports.
    ///
    /// The engine never selects an operating-system adapter. The host owns
    /// concrete clipboard, Finder, LAN, input and workflow composition.
    pub fn assemble_with(
        data_dir: PathBuf,
        ports: HostPorts,
        platform: Arc<dyn EnginePlatformPort>,
    ) -> Result<Self, String> {
        std::fs::create_dir_all(&data_dir).map_err(|error| error.to_string())?;
        let transport = Arc::new(MemoryTransport::new());
        Ok(Self {
            core: CollaborationCore::new_desktop(ports.clone(), transport, data_dir),
            ports,
            workflow_execution: platform.workflow_execution(),
        })
    }

    #[must_use]
    pub fn core(&self) -> &CollaborationCore {
        &self.core
    }

    #[must_use]
    pub fn ports(&self) -> &HostPorts {
        &self.ports
    }

    #[must_use]
    pub fn workflow_execution(&self) -> Arc<dyn deskfuse_platform::WorkflowExecutionPort> {
        self.workflow_execution.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assembles_an_idle_desktop_core() {
        let directory = std::env::temp_dir().join(format!("deskfuse-host-{}", std::process::id()));
        let host = EngineHost::assemble_in(directory).expect("host");
        let snapshot = futures::executor::block_on(host.core().snapshot());
        assert!(snapshot.active_connections.is_empty());
        assert!(!snapshot.input_status.lease_active);
    }
}
