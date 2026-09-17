use crate::{MAX_MANIFEST_BYTES, UPDATE_CACHE_LIMIT, UpdateError};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use uuid::Uuid;

const SIGNATURE_DOMAIN: &[u8] = b"unidesk/release-manifest/v1\0";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseVersion {
    pub version: Version,
    pub build_number: u64,
    pub release_id: Uuid,
}
impl ReleaseVersion {
    #[must_use]
    pub fn precedence(&self, other: &Self) -> Ordering {
        self.version
            .cmp_precedence(&other.version)
            .then(self.build_number.cmp(&other.build_number))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    MacosArm64,
    WindowsX64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageKind {
    MacosAppZip,
    MacosDmg,
    WindowsNsisExe,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactDescriptor {
    pub target: Target,
    pub kind: PackageKind,
    /// A single file name, never a path supplied by a remote process.
    pub file_name: String,
    pub size: u64,
    pub sha256: String,
    pub minimum_os: Version,
}
impl ArtifactDescriptor {
    ///
    /// # Errors
    /// Rejects malformed release metadata, encoding errors, or an invalid pinned-key signature.
    pub fn validate(&self) -> Result<(), UpdateError> {
        let extension = match (self.target, self.kind) {
            (Target::MacosArm64, PackageKind::MacosAppZip) => ".zip",
            (Target::MacosArm64, PackageKind::MacosDmg) => ".dmg",
            (Target::WindowsX64, PackageKind::WindowsNsisExe) => ".exe",
            _ => return Err(UpdateError::Invalid("target/package mismatch")),
        };
        if self.size == 0 || self.size > UPDATE_CACHE_LIMIT {
            return Err(UpdateError::Invalid("artifact size"));
        }
        if self.file_name.len() > 128
            || !self.file_name.starts_with("uniDesk-")
            || !self.file_name.ends_with(extension)
            || !self
                .file_name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
            || self.file_name.contains("..")
        {
            return Err(UpdateError::Invalid("artifact filename"));
        }
        if self.sha256.len() != 64
            || !self
                .sha256
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err(UpdateError::Invalid("artifact SHA-256"));
        }
        if !self.minimum_os.pre.is_empty() || !self.minimum_os.build.is_empty() {
            return Err(UpdateError::Invalid("minimum OS version"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseManifest {
    pub schema_version: u32,
    pub application_id: String,
    pub release: ReleaseVersion,
    pub protocol_major: u32,
    pub minimum_update_protocol: u32,
    /// Strict target ordering makes the signature payload deterministic.
    pub artifacts: Vec<ArtifactDescriptor>,
}
impl ReleaseManifest {
    ///
    /// # Errors
    /// Rejects malformed release metadata, encoding errors, or an invalid pinned-key signature.
    pub fn validate(&self) -> Result<(), UpdateError> {
        if self.schema_version != 1
            || self.application_id != "com.unidesk.app"
            || self.release.build_number == 0
            || self.release.release_id.is_nil()
            || self.protocol_major == 0
            || self.minimum_update_protocol == 0
            || !self.release.version.build.is_empty()
        {
            return Err(UpdateError::Invalid("release header"));
        }
        if self.artifacts.is_empty() || self.artifacts.len() > 2 {
            return Err(UpdateError::Invalid("artifact count"));
        }
        for artifact in &self.artifacts {
            artifact.validate()?;
            if artifact.kind == PackageKind::MacosDmg {
                return Err(UpdateError::Invalid("LAN manifest package kind"));
            }
        }
        if self
            .artifacts
            .windows(2)
            .any(|w| w[0].target >= w[1].target)
        {
            return Err(UpdateError::Invalid("artifact target order or duplicate"));
        }
        Ok(())
    }
    fn signing_bytes(&self) -> Result<Vec<u8>, UpdateError> {
        self.validate()?;
        let payload = serde_json::to_vec(self)?;
        if payload.len() > MAX_MANIFEST_BYTES {
            return Err(UpdateError::Invalid("manifest bounds"));
        }
        let mut bytes = SIGNATURE_DOMAIN.to_vec();
        bytes.extend(payload);
        Ok(bytes)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedReleaseManifest {
    pub manifest: ReleaseManifest,
    /// Hex Ed25519 signature. The verifying key is deliberately NOT network-provided.
    pub signature: String,
}
impl SignedReleaseManifest {
    ///
    /// # Errors
    /// Rejects malformed release metadata, encoding errors, or an invalid pinned-key signature.
    pub fn sign(manifest: ReleaseManifest, key: &SigningKey) -> Result<Self, UpdateError> {
        let signature = hex::encode(key.sign(&manifest.signing_bytes()?).to_bytes());
        Ok(Self {
            manifest,
            signature,
        })
    }
    ///
    /// # Errors
    /// Rejects malformed release metadata, encoding errors, or an invalid pinned-key signature.
    pub fn from_slice(bytes: &[u8]) -> Result<Self, UpdateError> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(UpdateError::Invalid("signed manifest bounds"));
        }
        Ok(serde_json::from_slice(bytes)?)
    }
    ///
    /// # Errors
    /// Rejects malformed release metadata, encoding errors, or an invalid pinned-key signature.
    pub fn verify(&self, pinned_key: &VerifyingKey) -> Result<VerifiedRelease, UpdateError> {
        let bytes = self.manifest.signing_bytes()?;
        if self.signature.len() != 128 {
            return Err(UpdateError::Signature);
        }
        let signature = hex::decode(&self.signature).map_err(|_| UpdateError::Signature)?;
        let signature = Signature::from_slice(&signature).map_err(|_| UpdateError::Signature)?;
        pinned_key
            .verify_strict(&bytes, &signature)
            .map_err(|_| UpdateError::Signature)?;
        Ok(VerifiedRelease(self.clone()))
    }
}

/// Construction requires the locally pinned release key, not a peer identity key.
#[derive(Clone, Debug)]
pub struct VerifiedRelease(SignedReleaseManifest);
impl VerifiedRelease {
    #[must_use]
    pub fn manifest(&self) -> &ReleaseManifest {
        &self.0.manifest
    }
    #[must_use]
    pub fn signed(&self) -> &SignedReleaseManifest {
        &self.0
    }
    #[must_use]
    pub fn artifact(&self, target: Target) -> Option<&ArtifactDescriptor> {
        self.manifest()
            .artifacts
            .iter()
            .find(|a| a.target == target)
    }
}
