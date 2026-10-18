//! File authorization ports. Hosts decide readable sources and writable destinations.
use crate::{PlatformError, PlatformResult};
use async_trait::async_trait;
use std::path::{Path, PathBuf};

/// A host-authorized readable file. Collaboration never opens paths itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorizedFile {
    pub path: PathBuf,
    pub size: u64,
}

impl AuthorizedFile {
    #[must_use]
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".into())
    }
}

/// Grant read access for outbound transfers and a destination directory for inbound ones.
#[async_trait]
pub trait FileAuthorizationPort: Send + Sync {
    async fn authorize_read(&self, paths: &[PathBuf]) -> PlatformResult<Vec<AuthorizedFile>>;

    /// Return the directory that inbound files may be written into.
    async fn authorize_write(&self, names: &[String], total_size: u64) -> PlatformResult<PathBuf>;
}

/// Test double that authorizes any existing path and writes into a given directory.
#[derive(Debug, Clone)]
pub struct MemoryFileAuthorization {
    write_dir: PathBuf,
}

impl MemoryFileAuthorization {
    #[must_use]
    pub fn new(write_dir: impl Into<PathBuf>) -> Self {
        Self {
            write_dir: write_dir.into(),
        }
    }
}

#[async_trait]
impl FileAuthorizationPort for MemoryFileAuthorization {
    async fn authorize_read(&self, paths: &[PathBuf]) -> PlatformResult<Vec<AuthorizedFile>> {
        let mut files = Vec::with_capacity(paths.len());
        for path in paths {
            let metadata = std::fs::metadata(path).map_err(|_| PlatformError::Unsupported {
                operation: "file-read",
            })?;
            if !metadata.is_file() {
                return Err(PlatformError::Unsupported {
                    operation: "file-read",
                });
            }
            files.push(AuthorizedFile {
                path: path.clone(),
                size: metadata.len(),
            });
        }
        Ok(files)
    }

    async fn authorize_write(
        &self,
        _names: &[String],
        _total_size: u64,
    ) -> PlatformResult<PathBuf> {
        if !self.write_dir.as_path().exists() {
            std::fs::create_dir_all(&self.write_dir).map_err(|_| PlatformError::Unsupported {
                operation: "file-write",
            })?;
        }
        Ok(self.write_dir.clone())
    }
}

/// Convenience helper used by file-transfer tests.
pub fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into())
}
