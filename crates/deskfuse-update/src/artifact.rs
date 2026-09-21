use crate::{ArtifactDescriptor, ReleaseVersion, Target, UpdateError, VerifiedRelease};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

/// Integrity evidence at verification time; installers MUST reverify immediately
/// before use and keep their private staging directory protected from replacement.
#[derive(Clone, Debug)]
pub struct VerifiedArtifact {
    pub(crate) release: ReleaseVersion,
    pub(crate) descriptor: ArtifactDescriptor,
    path: PathBuf,
}
impl VerifiedArtifact {
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
    #[must_use]
    pub fn descriptor(&self) -> &ArtifactDescriptor {
        &self.descriptor
    }
    #[must_use]
    pub fn release(&self) -> &ReleaseVersion {
        &self.release
    }
}

///
/// # Errors
/// Rejects missing, non-regular, incompatible or corrupt artifacts and filesystem errors.
pub fn verify_artifact(
    release: &VerifiedRelease,
    target: Target,
    path: &Path,
) -> Result<VerifiedArtifact, UpdateError> {
    let descriptor = release.artifact(target).ok_or(UpdateError::Incompatible)?;
    // Do not follow a caller-selected symlink to a different source artifact.
    let metadata = path.symlink_metadata()?;
    if !metadata.is_file() || metadata.len() != descriptor.size {
        return Err(UpdateError::ArtifactMismatch);
    }
    let file = File::open(path)?;
    if file.metadata()?.len() != descriptor.size {
        return Err(UpdateError::ArtifactMismatch);
    }
    let mut file = file.take(descriptor.size + 1);
    let mut digest = Sha256::new();
    let mut buffer = vec![0; 64 * 1024];
    let mut bytes = 0u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
        bytes += read as u64;
    }
    if bytes != descriptor.size || hex::encode(digest.finalize()) != descriptor.sha256 {
        return Err(UpdateError::ArtifactMismatch);
    }
    Ok(VerifiedArtifact {
        release: release.manifest().release.clone(),
        descriptor: descriptor.clone(),
        path: path.to_path_buf(),
    })
}
