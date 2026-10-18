use deskfuse_collaboration::{CollaborationCore, MemoryTransport};
use deskfuse_platform::ports::HostPorts;
use std::{path::PathBuf, sync::Arc};

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
}
