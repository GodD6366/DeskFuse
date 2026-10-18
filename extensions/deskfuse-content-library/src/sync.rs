//! Library replication. History items never enter the system clipboard from this path.
use deskfuse_collaboration::CollaborationCore;
use deskfuse_store_library::LibraryStore;
use std::sync::Arc;

/// Optional peer replication of library metadata. Disabled until a host opts in.
pub struct LibraryReplication {
    enabled: bool,
    _core: Arc<CollaborationCore>,
    _store: LibraryStore,
}

impl LibraryReplication {
    #[must_use]
    pub fn new(core: Arc<CollaborationCore>, store: LibraryStore) -> Self {
        Self {
            enabled: false,
            _core: core,
            _store: store,
        }
    }

    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn enable(&mut self) {
        self.enabled = true;
    }

    pub async fn start(&self) -> deskfuse_collaboration::Result<()> {
        Ok(())
    }
}
