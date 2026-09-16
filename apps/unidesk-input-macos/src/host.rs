//! Input-only assembly. Clipboard, files, library, workflow and web stay out.
use std::{path::PathBuf, sync::Arc, time::Duration};
use unidesk_collaboration::{CollaborationCore, MemoryTransport};
use unidesk_platform::ports::HostPorts;

pub struct InputHost {
    pub core: CollaborationCore,
}

impl InputHost {
    pub fn assemble(data_dir: PathBuf) -> Self {
        let ports = HostPorts::input_only(&data_dir);
        let transport = Arc::new(MemoryTransport::new());
        Self {
            core: CollaborationCore::new_input_only(ports, transport, data_dir),
        }
    }

    pub async fn start(&self) -> unidesk_collaboration::Result<()> {
        self.core.start().await
    }

    pub async fn stop(&self) -> unidesk_collaboration::Result<()> {
        self.core.stop().await
    }

    pub fn watchdog_interval() -> Duration {
        Duration::from_secs(5)
    }
}
