use std::fs::{self, File};
use std::io::{ErrorKind, Read, Write};
use std::path::PathBuf;

use super::{IdentityError, IdentityProvider, KEY_BYTES, LocalIdentity};

/// Persistent local identity. Existing invalid keys are never overwritten.
#[derive(Clone, Debug)]
pub struct FileIdentityProvider {
    path: PathBuf,
}

impl FileIdentityProvider {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn load_or_create(&self) -> Result<LocalIdentity, IdentityError> {
        <Self as IdentityProvider>::load_or_create(self)
    }

    fn read(&self) -> Result<LocalIdentity, IdentityError> {
        let metadata = fs::symlink_metadata(&self.path)?;
        if !metadata.file_type().is_file() {
            return Err(std::io::Error::new(
                ErrorKind::InvalidData,
                "identity must be a regular file",
            )
            .into());
        }
        let file = File::open(&self.path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        let mut secret = Vec::with_capacity(KEY_BYTES + 1);
        file.take((KEY_BYTES + 1) as u64).read_to_end(&mut secret)?;
        LocalIdentity::from_bytes(&secret)
    }
}

impl IdentityProvider for FileIdentityProvider {
    fn load_or_create(&self) -> Result<LocalIdentity, IdentityError> {
        match self.read() {
            Ok(identity) => return Ok(identity),
            Err(IdentityError::File(error)) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let parent = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::Path::new("."));
        fs::create_dir_all(parent)?;
        let identity = LocalIdentity::generate();
        // Publish only a complete, synced key; concurrent processes read the winner.
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            temporary
                .as_file()
                .set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        temporary.write_all(&identity.secret_bytes())?;
        temporary.as_file().sync_all()?;
        match temporary.persist_noclobber(&self.path) {
            Ok(_) => {
                #[cfg(unix)]
                File::open(parent)?.sync_all()?;
                Ok(identity)
            }
            Err(error) if error.error.kind() == ErrorKind::AlreadyExists => self.read(),
            Err(error) => Err(error.error.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_preserves_identity_and_private_permissions() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("data/identity.key");
        let first = FileIdentityProvider::new(&path).load_or_create().unwrap();
        let second = FileIdentityProvider::new(&path).load_or_create().unwrap();
        assert_eq!(first.device_id(), second.device_id());
        assert!(second.verify(b"restart", &first.sign(b"restart")).is_ok());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn invalid_file_is_not_replaced() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("identity.key");
        fs::write(&path, b"invalid").unwrap();
        assert!(FileIdentityProvider::new(&path).load_or_create().is_err());
        assert_eq!(fs::read(path).unwrap(), b"invalid");
    }

    #[test]
    fn concurrent_creation_uses_one_identity() {
        let directory = tempfile::tempdir().unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let path = directory.path().join("identity.key");
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    FileIdentityProvider::new(path)
                        .load_or_create()
                        .unwrap()
                        .device_id()
                })
            })
            .collect();
        let identities: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();
        assert!(identities.iter().all(|identity| *identity == identities[0]));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_is_rejected_without_modifying_target() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target");
        let path = directory.path().join("identity.key");
        fs::write(&target, [7; KEY_BYTES]).unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(FileIdentityProvider::new(path).load_or_create().is_err());
        assert_eq!(fs::read(target).unwrap(), [7; KEY_BYTES]);
    }
}
