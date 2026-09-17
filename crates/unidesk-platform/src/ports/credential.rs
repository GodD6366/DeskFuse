//! Host-owned credential persistence. Collaboration never chooses a secret path.
use crate::{PlatformError, PlatformResult};
use async_trait::async_trait;
use std::path::PathBuf;

/// Load and store the local identity secret. Implementations may use a file or Keychain.
#[async_trait]
pub trait CredentialStorePort: Send + Sync {
    async fn load_identity(&self) -> PlatformResult<Vec<u8>>;
    async fn store_identity(&self, bytes: &[u8]) -> PlatformResult<()>;
}

/// File-backed identity used by tests and by hosts that have not moved to Keychain.
#[derive(Debug, Clone)]
pub struct MemoryCredentialStore {
    path: PathBuf,
}

impl MemoryCredentialStore {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

#[async_trait]
impl CredentialStorePort for MemoryCredentialStore {
    async fn load_identity(&self) -> PlatformResult<Vec<u8>> {
        std::fs::read(&self.path).map_err(|_| PlatformError::Unsupported {
            operation: "credential-load",
        })
    }

    async fn store_identity(&self, bytes: &[u8]) -> PlatformResult<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|_| PlatformError::Unsupported {
                operation: "credential-store",
            })?;
        }
        std::fs::write(&self.path, bytes).map_err(|_| PlatformError::Unsupported {
            operation: "credential-store",
        })
    }
}
