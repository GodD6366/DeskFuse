use crate::UpdateError;
use serde::Serialize;
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::Path,
};

/// # Errors
/// Returns a verification, storage, or platform operation failure.
pub fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<(), UpdateError> {
    let parent = path.parent().ok_or(UpdateError::Invalid("metadata path"))?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".metadata-{}", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(&serde_json::to_vec(value)?)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    let _ = fs::remove_file(temporary);
    result
}

/// # Errors
/// Returns a verification, storage, or platform operation failure.
pub fn pinned_release_key() -> Result<ed25519_dalek::VerifyingKey, UpdateError> {
    let bytes =
        hex::decode(include_str!("../../../resources/unidesk-release-public-key.hex").trim())
            .map_err(|_| UpdateError::Invalid("release public key"))?;
    ed25519_dalek::VerifyingKey::from_bytes(
        &bytes
            .try_into()
            .map_err(|_| UpdateError::Invalid("release public key"))?,
    )
    .map_err(|_| UpdateError::Invalid("release public key"))
}
