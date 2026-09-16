use std::{path::PathBuf, sync::Arc};
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
}
