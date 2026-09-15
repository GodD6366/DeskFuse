//! Exact-byte offline release cache. Call on an I/O worker, never the UI thread.
//! The caller must provide an application-owned directory with platform ACLs.
use crate::{
    MAX_MANIFEST_BYTES, SignedReleaseManifest, Target, UPDATE_CACHE_LIMIT, UpdateError,
    VerifiedArtifact, VerifiedRelease, verify_artifact,
};
use ed25519_dalek::VerifyingKey;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;

/// Owns an exclusive cross-process lock for the lifetime of the cache.
/// Dropping it releases the lock; clients must not serve paths after releasing it.
pub struct ReleaseCache {
    root: PathBuf,
    key: VerifyingKey,
    limit: u64,
    _lock: File,
}

#[derive(Debug)]
pub struct CachedArtifact {
    pub release: VerifiedRelease,
    pub artifact: VerifiedArtifact,
}

impl ReleaseCache {
    /// Open the separate update cache. No clipboard cache or user files are touched.
    ///
    /// # Errors
    /// Rejects symlink roots, invalid limits, busy caches and filesystem failures.
    pub fn open(root: &Path, key: VerifyingKey, limit: u64) -> Result<Self, UpdateError> {
        if limit == 0 || limit > UPDATE_CACHE_LIMIT {
            return Err(UpdateError::Invalid("cache limit"));
        }
        fs::create_dir_all(root)?;
        if !root.symlink_metadata()?.is_dir() {
            return Err(UpdateError::Invalid("cache root"));
        }
        let lock_path = root.join("cache.lock");
        if lock_path.exists() && !lock_path.symlink_metadata()?.is_file() {
            return Err(UpdateError::Invalid("cache lock"));
        }
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)?;
        lock.try_lock()
            .map_err(|_| UpdateError::Invalid("cache busy"))?;
        Ok(Self {
            root: root.to_path_buf(),
            key,
            limit,
            _lock: lock,
        })
    }

    /// Reverify signatures and original bytes after every reopen/before advertising.
    /// Incomplete staging directories are never advertised.
    ///
    /// # Errors
    /// Fails closed on corrupt committed entries or filesystem failures.
    pub fn inventory(&self) -> Result<Vec<CachedArtifact>, UpdateError> {
        let mut result = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            if !entry.file_name().to_string_lossy().starts_with("entry-") {
                continue;
            }
            if !entry.file_type()?.is_dir() {
                return Err(UpdateError::Invalid("cache entry"));
            }
            let dir = entry.path();
            let manifest_path = dir.join("manifest.json");
            if !manifest_path.symlink_metadata()?.is_file() {
                return Err(UpdateError::Invalid("cached manifest"));
            }
            let mut bytes = Vec::new();
            File::open(manifest_path)?
                .take(MAX_MANIFEST_BYTES as u64 + 1)
                .read_to_end(&mut bytes)?;
            let release = SignedReleaseManifest::from_slice(&bytes)?.verify(&self.key)?;
            // Only actually present, verified target packages are advertised.
            let mut found = false;
            for descriptor in &release.manifest().artifacts {
                let path = dir.join(&descriptor.file_name);
                if path.try_exists()? {
                    let artifact = verify_artifact(&release, descriptor.target, &path)?;
                    result.push(CachedArtifact {
                        release: release.clone(),
                        artifact,
                    });
                    found = true;
                }
            }
            if !found {
                return Err(UpdateError::Invalid("empty cache entry"));
            }
        }
        Ok(result)
    }

    /// Make room before downloading, preserving installed, candidate and serving releases.
    /// # Errors
    /// Fails if pinned entries leave insufficient room or filesystem operations fail.
    pub fn reserve(
        &self,
        bytes: u64,
        protected: &std::collections::BTreeSet<Uuid>,
    ) -> Result<(), UpdateError> {
        let mut entries = self.inventory()?;
        entries.sort_by(|a, b| a.artifact.release().precedence(b.artifact.release()));
        let mut used = directory_size(&self.root)?;
        for entry in entries {
            if used.saturating_add(bytes) <= self.limit {
                return Ok(());
            }
            if protected.contains(&entry.artifact.release().release_id) {
                continue;
            }
            let dir = entry
                .artifact
                .path()
                .parent()
                .ok_or(UpdateError::Invalid("cache entry path"))?;
            let size = directory_size(dir)?;
            fs::remove_dir_all(dir)?;
            used = used.saturating_sub(size);
        }
        if used.saturating_add(bytes) > self.limit {
            return Err(UpdateError::Invalid(
                "update cache full with protected releases",
            ));
        }
        Ok(())
    }

    /// Import one target's original bytes, independent of the host OS. Publication
    /// is a directory rename only after both payload and signed manifest are synced.
    /// Budget exhaustion preserves existing packages (eviction requires installer pins).
    ///
    /// # Errors
    /// Rejects untrusted/corrupt packages, version conflicts, capacity and I/O failures.
    pub fn import(
        &self,
        manifest: &[u8],
        target: Target,
        source: &Path,
    ) -> Result<CachedArtifact, UpdateError> {
        let release = SignedReleaseManifest::from_slice(manifest)?.verify(&self.key)?;
        let source_artifact = verify_artifact(&release, target, source)?;
        let descriptor = source_artifact.descriptor();
        let inventory = self.inventory()?;
        for cached in &inventory {
            let previous = &cached.release.manifest().release;
            let next = &release.manifest().release;
            if previous.version == next.version
                && previous.build_number == next.build_number
                && cached.release.signed() != release.signed()
            {
                return Err(UpdateError::Conflict);
            }
            if cached.release.signed() == release.signed()
                && cached.artifact.descriptor().target == target
            {
                return Ok(CachedArtifact {
                    release,
                    artifact: cached.artifact.clone(),
                });
            }
        }
        let used = directory_size(&self.root)?;
        if used
            .saturating_add(descriptor.size)
            .saturating_add(manifest.len() as u64)
            > self.limit
        {
            return Err(UpdateError::Invalid("update cache full"));
        }
        let stage = self.root.join(format!("stage-{}", Uuid::new_v4()));
        fs::create_dir(&stage)?;
        let result = (|| {
            let staged_payload = stage.join(&descriptor.file_name);
            let mut input = File::open(source)?.take(descriptor.size + 1);
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&staged_payload)?;
            let size = std::io::copy(&mut input, &mut output)?;
            output.sync_all()?;
            if size != descriptor.size {
                return Err(UpdateError::ArtifactMismatch);
            }
            verify_artifact(&release, target, &staged_payload)?;
            let mut metadata = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(stage.join("manifest.json"))?;
            metadata.write_all(manifest)?;
            metadata.sync_all()?;
            drop(metadata);
            drop(output);
            let mut identity = Sha256::new();
            identity.update(serde_json::to_vec(release.signed())?);
            identity.update(descriptor.file_name.as_bytes());
            let destination = self
                .root
                .join(format!("entry-{}", hex::encode(identity.finalize())));
            fs::rename(&stage, &destination)?;
            let artifact =
                verify_artifact(&release, target, &destination.join(&descriptor.file_name))?;
            Ok(CachedArtifact { release, artifact })
        })();
        if stage.exists() {
            let _ = fs::remove_dir_all(stage);
        }
        result
    }
}

fn directory_size(path: &Path) -> Result<u64, UpdateError> {
    let mut total = 0u64;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            return Err(UpdateError::Invalid("cache symlink"));
        }
        let size = if kind.is_dir() {
            directory_size(&entry.path())?
        } else {
            entry.metadata()?.len()
        };
        total = total
            .checked_add(size)
            .ok_or(UpdateError::Invalid("cache size overflow"))?;
    }
    Ok(total)
}
