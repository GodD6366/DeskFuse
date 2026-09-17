//! Update extension. The collaboration core and input-only hosts do not depend on this crate.
#![forbid(unsafe_code)]

pub mod downloader;
pub mod installer;

pub use downloader::Downloader;
pub use unidesk_update::{ReleaseCache, UpdateError};

use std::sync::Arc;
use unidesk_collaboration::CollaborationCore;

pub struct UpdateExtension {
    _core: Arc<CollaborationCore>,
}

impl UpdateExtension {
    #[must_use]
    pub fn new(core: Arc<CollaborationCore>) -> Self {
        Self { _core: core }
    }

    pub async fn start(&self) -> unidesk_collaboration::Result<()> {
        Ok(())
    }

    pub async fn stop(&self) {}
}
