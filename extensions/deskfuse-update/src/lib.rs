//! Update extension. The collaboration core and input-only hosts do not depend on this crate.
#![forbid(unsafe_code)]

pub mod downloader;
pub mod installer;

pub use deskfuse_update::{ReleaseCache, UpdateError};
pub use downloader::Downloader;

use deskfuse_collaboration::CollaborationCore;
use std::sync::Arc;

pub struct UpdateExtension {
    _core: Arc<CollaborationCore>,
}

impl UpdateExtension {
    #[must_use]
    pub fn new(core: Arc<CollaborationCore>) -> Self {
        Self { _core: core }
    }

    pub async fn start(&self) -> deskfuse_collaboration::Result<()> {
        Ok(())
    }

    pub async fn stop(&self) {}
}
