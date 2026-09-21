//! Content-library extension. The collaboration core does not depend on this crate.
#![forbid(unsafe_code)]

pub mod indexer;
pub mod search;
pub mod sync;

pub use indexer::{Indexer, title_for};
pub use sync::LibraryReplication;

use deskfuse_collaboration::{CollaborationCore, CollaborationEvent, ContentHandle};
use deskfuse_store_library::LibraryStore;
use std::{path::PathBuf, sync::Arc};

pub struct ContentLibraryExtension {
    store: LibraryStore,
    indexer: Indexer,
    sync: LibraryReplication,
}

impl ContentLibraryExtension {
    pub fn open(
        core: Arc<CollaborationCore>,
        db_path: PathBuf,
    ) -> deskfuse_store_library::Result<Self> {
        let store = LibraryStore::open(db_path)?;
        let indexer = Indexer::new(store.contents.clone());
        let sync = LibraryReplication::new(core, store.clone());
        Ok(Self {
            store,
            indexer,
            sync,
        })
    }

    pub fn memory(core: Arc<CollaborationCore>) -> deskfuse_store_library::Result<Self> {
        let store = LibraryStore::memory()?;
        let indexer = Indexer::new(store.contents.clone());
        let sync = LibraryReplication::new(core, store.clone());
        Ok(Self {
            store,
            indexer,
            sync,
        })
    }

    pub async fn start(&self) -> deskfuse_collaboration::Result<()> {
        self.sync.start().await
    }

    pub async fn stop(&self) {}

    pub fn on_event(&self, event: &CollaborationEvent) {
        if let CollaborationEvent::ClipboardApplied { .. } = event {
            // Indexing of bodies happens when the host supplies a handle via `index_handle`.
        }
    }

    pub fn index_handle(
        &self,
        handle: &ContentHandle,
        now: i64,
    ) -> deskfuse_store_library::Result<()> {
        self.indexer.index(handle, now)
    }

    #[must_use]
    pub fn store(&self) -> &LibraryStore {
        &self.store
    }
}
