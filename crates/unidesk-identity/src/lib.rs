//! Device identity and trust management for uniDesk.
//!
//! The local identity is an Ed25519 key pair.  The private key never leaves the
//! identity provider; callers receive a public device id and detached
//! signatures which can be verified independently.

#![deny(unsafe_code)]
#![allow(clippy::missing_errors_doc)]

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::{WindowsServiceDataDirectory, WindowsServiceIdentityProvider};

mod network;
pub use network::*;

use std::collections::BTreeMap;
use std::fmt;
use std::ops::{Deref, DerefMut, Index, IndexMut};
use std::sync::{Arc, Mutex};

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// The size, in bytes, of a device id/fingerprint and Ed25519 secret key.
pub const KEY_BYTES: usize = 32;

/// Stable identifier for a device, computed by `unidesk-core` as the SHA-256
/// digest of its Ed25519 verifying key.
pub use unidesk_core::DeviceId;

/// A stable, display-oriented fingerprint for a device public key.
///
/// Fingerprints deliberately use the same SHA-256 material as [`DeviceId`],
/// but are kept as a distinct type to prevent accidentally using a human
/// display value where a protocol device id is required.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct Fingerprint([u8; KEY_BYTES]);

impl Fingerprint {
    #[must_use]
    pub const fn from_bytes(bytes: [u8; KEY_BYTES]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; KEY_BYTES] {
        &self.0
    }

    #[must_use]
    pub const fn into_bytes(self) -> [u8; KEY_BYTES] {
        self.0
    }

    #[must_use]
    pub fn to_hex(self) -> String {
        hex_encode(&self.0)
    }

    pub fn from_hex(value: &str) -> Result<Self, IdentityError> {
        Ok(Self(parse_hex(value)?))
    }

    /// Compute a fingerprint from an Ed25519 public key.
    pub fn from_public_key<P: AsRef<[u8]>>(public_key: P) -> Result<Self, IdentityError> {
        let public_key = public_key.as_ref();
        let public_key: [u8; KEY_BYTES] =
            public_key
                .try_into()
                .map_err(|_| IdentityError::InvalidKeyLength {
                    expected: KEY_BYTES,
                    actual: public_key.len(),
                })?;
        Ok(Self(digest_public_key(&public_key)))
    }
}

impl From<DeviceId> for Fingerprint {
    fn from(device_id: DeviceId) -> Self {
        Self::from_bytes(device_id.into_bytes())
    }
}

impl From<Fingerprint> for DeviceId {
    fn from(fingerprint: Fingerprint) -> Self {
        DeviceId::from_bytes(fingerprint.into_bytes())
    }
}

impl fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Fingerprint").field(&self.to_hex()).finish()
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl AsRef<[u8]> for Fingerprint {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

/// Signature bytes produced by [`LocalIdentity::sign`].
#[derive(Clone, Eq, PartialEq)]
pub struct IdentitySignature([u8; Signature::BYTE_SIZE]);

impl IdentitySignature {
    #[must_use]
    pub const fn from_bytes(bytes: [u8; Signature::BYTE_SIZE]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; Signature::BYTE_SIZE] {
        &self.0
    }

    #[must_use]
    pub const fn into_bytes(self) -> [u8; Signature::BYTE_SIZE] {
        self.0
    }
}

impl fmt::Debug for IdentitySignature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("IdentitySignature")
            .field(&hex_encode(&self.0))
            .finish()
    }
}

impl AsRef<[u8]> for IdentitySignature {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl TryFrom<&[u8]> for IdentitySignature {
    type Error = IdentityError;

    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        let bytes: [u8; Signature::BYTE_SIZE] =
            bytes
                .try_into()
                .map_err(|_| IdentityError::InvalidSignatureLength {
                    expected: Signature::BYTE_SIZE,
                    actual: bytes.len(),
                })?;
        Ok(Self(bytes))
    }
}

impl Deref for IdentitySignature {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for IdentitySignature {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Index<usize> for IdentitySignature {
    type Output = u8;

    fn index(&self, index: usize) -> &Self::Output {
        &self.0[index]
    }
}

impl IndexMut<usize> for IdentitySignature {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.0[index]
    }
}

impl AsMut<[u8]> for IdentitySignature {
    fn as_mut(&mut self) -> &mut [u8] {
        &mut self.0
    }
}

/// Errors returned by identity and trust-store operations.
#[derive(Debug, Error)]
pub enum IdentityError {
    #[error("Windows service identity operation failed: {0}")]
    WindowsService(String),
    #[error("identity persistence is unsupported on this platform")]
    UnsupportedPlatform,
    #[error("identity file operation failed: {0}")]
    File(#[from] std::io::Error),
    #[error("invalid Ed25519 key material: expected {expected} bytes, got {actual}")]
    InvalidKeyLength { expected: usize, actual: usize },
    #[error("invalid hexadecimal value")]
    InvalidHex,
    #[error("invalid signature encoding")]
    InvalidSignature,
    #[error("invalid Ed25519 signature: expected {expected} bytes, got {actual}")]
    InvalidSignatureLength { expected: usize, actual: usize },
    #[error("signature verification failed")]
    VerificationFailed,
    #[error("trust store operation failed: {0}")]
    TrustStore(String),
    #[error("concurrent identity operation was poisoned")]
    Poisoned,
}

/// A local Ed25519 identity.
///
/// The [`SigningKey`] is intentionally private.  This prevents callers from
/// exporting or replacing key material through the normal API while still
/// allowing detached signing and public verification.
pub struct LocalIdentity {
    signing_key: SigningKey,
    device_id: DeviceId,
    fingerprint: Fingerprint,
}

impl fmt::Debug for LocalIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalIdentity")
            .field("device_id", &self.device_id)
            .field("fingerprint", &self.fingerprint)
            .finish_non_exhaustive()
    }
}

impl LocalIdentity {
    /// Generate a new random identity.
    #[must_use]
    pub fn generate() -> Self {
        Self::from_signing_key(SigningKey::generate(&mut OsRng))
    }

    /// Reconstruct an identity from exactly 32 private-key bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, IdentityError> {
        let bytes: [u8; KEY_BYTES] =
            bytes
                .try_into()
                .map_err(|_| IdentityError::InvalidKeyLength {
                    expected: KEY_BYTES,
                    actual: bytes.len(),
                })?;
        Ok(Self::from_signing_key(SigningKey::from_bytes(&bytes)))
    }

    /// Alias for [`Self::from_bytes`] that makes the key purpose explicit.
    pub fn from_secret_bytes(bytes: &[u8]) -> Result<Self, IdentityError> {
        Self::from_bytes(bytes)
    }

    /// Return a copy of the raw private key bytes for a persistence provider.
    ///
    /// This is intentionally crate-visible: applications should use an
    /// [`IdentityProvider`] rather than persisting these bytes themselves.
    pub(crate) fn secret_bytes(&self) -> [u8; KEY_BYTES] {
        self.signing_key.to_bytes()
    }

    /// Return the stable SHA-256 device id.
    #[must_use]
    pub const fn device_id(&self) -> DeviceId {
        self.device_id
    }

    /// Return the public-key fingerprint.
    #[must_use]
    pub const fn fingerprint(&self) -> Fingerprint {
        self.fingerprint
    }

    /// Return the Ed25519 verifying key.
    #[must_use]
    pub fn verifying_key(&self) -> VerifyingKey {
        self.signing_key.verifying_key()
    }

    /// Return the raw 32-byte public key.
    #[must_use]
    pub fn verifying_key_bytes(&self) -> [u8; KEY_BYTES] {
        self.verifying_key().to_bytes()
    }

    /// Return the raw public key bytes (alias for [`Self::verifying_key_bytes`]).
    #[must_use]
    pub fn public_key_bytes(&self) -> [u8; KEY_BYTES] {
        self.verifying_key_bytes()
    }

    /// Alias for [`Self::device_id`], useful when building wire identities.
    #[must_use]
    pub const fn id(&self) -> DeviceId {
        self.device_id()
    }

    /// Alias for [`Self::public_key_bytes`].
    #[must_use]
    pub fn public_key(&self) -> [u8; KEY_BYTES] {
        self.public_key_bytes()
    }

    /// Sign a message using Ed25519.
    #[must_use]
    pub fn sign(&self, message: &[u8]) -> IdentitySignature {
        IdentitySignature(self.signing_key.sign(message).to_bytes())
    }

    /// Verify a signature against this identity's public key using the strict
    /// Ed25519 verifier.  Any byte container of exactly 64 bytes is accepted.
    pub fn verify<S: AsRef<[u8]>>(
        &self,
        message: &[u8],
        signature: &S,
    ) -> Result<(), IdentityError> {
        let signature = signature.as_ref();
        let signature: [u8; Signature::BYTE_SIZE] =
            signature
                .try_into()
                .map_err(|_| IdentityError::InvalidSignatureLength {
                    expected: Signature::BYTE_SIZE,
                    actual: signature.len(),
                })?;
        self.verifying_key()
            .verify_strict(message, &Signature::from_bytes(&signature))
            .map_err(|_| IdentityError::VerificationFailed)
    }

    /// Return whether a signature verifies, without discarding the strict
    /// verifier used by [`Self::verify`].
    #[must_use]
    pub fn verify_bool<S: AsRef<[u8]>>(&self, message: &[u8], signature: &S) -> bool {
        self.verify(message, signature).is_ok()
    }

    /// Verify a detached signature against a raw public key.
    pub fn verify_with_key(
        verifying_key: &[u8],
        message: &[u8],
        signature: &[u8],
    ) -> Result<(), IdentityError> {
        let verifying_key: [u8; KEY_BYTES] =
            verifying_key
                .try_into()
                .map_err(|_| IdentityError::InvalidKeyLength {
                    expected: KEY_BYTES,
                    actual: verifying_key.len(),
                })?;
        let signature: [u8; Signature::BYTE_SIZE] =
            signature
                .try_into()
                .map_err(|_| IdentityError::InvalidSignatureLength {
                    expected: Signature::BYTE_SIZE,
                    actual: signature.len(),
                })?;
        let key = VerifyingKey::from_bytes(&verifying_key)
            .map_err(|_| IdentityError::InvalidSignature)?;
        key.verify_strict(message, &Signature::from_bytes(&signature))
            .map_err(|_| IdentityError::VerificationFailed)
    }

    /// Verify a detached signature using a raw Ed25519 public key.
    pub fn verify_detached(
        verifying_key: &[u8],
        message: &[u8],
        signature: &[u8],
    ) -> Result<(), IdentityError> {
        Self::verify_with_key(verifying_key, message, signature)
    }

    fn from_signing_key(signing_key: SigningKey) -> Self {
        let verifying_key = signing_key.verifying_key().to_bytes();
        let digest = digest_public_key(&verifying_key);
        // `DeviceId` is SHA-256(verifying key), matching the core contract.
        let device_id = DeviceId::from_bytes(digest);
        Self {
            signing_key,
            device_id,
            fingerprint: Fingerprint(digest),
        }
    }
}

/// Source of the local device identity.
pub trait IdentityProvider: Send + Sync {
    /// Load the existing identity, creating one atomically when absent.
    fn load_or_create(&self) -> Result<LocalIdentity, IdentityError>;
}

/// An in-memory identity provider, useful for tests and ephemeral sessions.
#[derive(Clone, Default)]
pub struct MemoryIdentityProvider {
    inner: Arc<Mutex<Option<[u8; KEY_BYTES]>>>,
}

impl fmt::Debug for MemoryIdentityProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoryIdentityProvider")
            .field("initialized", &self.has_identity())
            .finish()
    }
}

impl MemoryIdentityProvider {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create an in-memory provider seeded with an existing secret key.
    pub fn from_secret_bytes(bytes: &[u8]) -> Result<Self, IdentityError> {
        let bytes: [u8; KEY_BYTES] =
            bytes
                .try_into()
                .map_err(|_| IdentityError::InvalidKeyLength {
                    expected: KEY_BYTES,
                    actual: bytes.len(),
                })?;
        Ok(Self {
            inner: Arc::new(Mutex::new(Some(bytes))),
        })
    }

    /// Load the identity, creating it while holding the provider lock.
    pub fn load_or_create(&self) -> Result<LocalIdentity, IdentityError> {
        <Self as IdentityProvider>::load_or_create(self)
    }

    #[must_use]
    pub fn has_identity(&self) -> bool {
        self.inner.lock().is_ok_and(|guard| guard.is_some())
    }

    pub fn clear(&self) -> Result<(), IdentityError> {
        self.inner
            .lock()
            .map_err(|_| IdentityError::Poisoned)?
            .take();
        Ok(())
    }
}

impl IdentityProvider for MemoryIdentityProvider {
    fn load_or_create(&self) -> Result<LocalIdentity, IdentityError> {
        let mut guard = self.inner.lock().map_err(|_| IdentityError::Poisoned)?;
        if let Some(bytes) = *guard {
            return LocalIdentity::from_bytes(&bytes);
        }
        let identity = LocalIdentity::generate();
        *guard = Some(identity.secret_bytes());
        Ok(identity)
    }
}

/// A trusted remote device record.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TrustRecord {
    pub device_id: DeviceId,
    pub fingerprint: Fingerprint,
    pub revoked: bool,
}

impl TrustRecord {
    #[must_use]
    pub const fn new(device_id: DeviceId, fingerprint: Fingerprint) -> Self {
        Self {
            device_id,
            fingerprint,
            revoked: false,
        }
    }

    #[must_use]
    pub const fn revoked(device_id: DeviceId, fingerprint: Fingerprint) -> Self {
        Self {
            device_id,
            fingerprint,
            revoked: true,
        }
    }
}

/// Persistent or in-memory trusted-device registry.
pub trait TrustStore: Send + Sync {
    /// Insert or replace a trusted record.
    fn upsert(&self, record: TrustRecord) -> Result<(), IdentityError>;
    /// Find one record by device id.
    fn get(&self, device_id: &DeviceId) -> Result<Option<TrustRecord>, IdentityError>;
    /// Return all records, in deterministic device-id order.
    fn list(&self) -> Result<Vec<TrustRecord>, IdentityError>;
    /// Mark a device as revoked. Missing devices are not an error.
    fn revoke(&self, device_id: &DeviceId) -> Result<(), IdentityError>;
    /// Test whether a device is currently trusted (exists and is not revoked).
    fn is_trusted(&self, device_id: &DeviceId) -> Result<bool, IdentityError> {
        Ok(self.get(device_id)?.is_some_and(|record| !record.revoked))
    }

    /// Convenience alias for [`Self::upsert`].
    fn trust(&self, record: TrustRecord) -> Result<(), IdentityError> {
        self.upsert(record)
    }

    /// Convenience alias for [`Self::list`].
    fn all(&self) -> Result<Vec<TrustRecord>, IdentityError> {
        self.list()
    }
}

/// A lock-protected in-memory trust store.
#[derive(Clone, Default)]
pub struct MemoryTrustStore {
    records: Arc<Mutex<BTreeMap<DeviceId, TrustRecord>>>,
}

impl fmt::Debug for MemoryTrustStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoryTrustStore")
            .field(
                "records",
                &self.records.lock().map_or(0, |records| records.len()),
            )
            .finish()
    }
}

impl MemoryTrustStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or replace a trusted device record.
    pub fn upsert(&self, record: TrustRecord) -> Result<(), IdentityError> {
        <Self as TrustStore>::upsert(self, record)
    }

    /// Find a record by device id.
    pub fn get(&self, device_id: &DeviceId) -> Result<Option<TrustRecord>, IdentityError> {
        <Self as TrustStore>::get(self, device_id)
    }

    /// Return records in deterministic device-id order.
    pub fn list(&self) -> Result<Vec<TrustRecord>, IdentityError> {
        <Self as TrustStore>::list(self)
    }

    /// Mark a device as revoked.
    pub fn revoke(&self, device_id: &DeviceId) -> Result<(), IdentityError> {
        <Self as TrustStore>::revoke(self, device_id)
    }

    /// Test whether a device exists and has not been revoked.
    pub fn is_trusted(&self, device_id: &DeviceId) -> Result<bool, IdentityError> {
        <Self as TrustStore>::is_trusted(self, device_id)
    }
}

impl TrustStore for MemoryTrustStore {
    fn upsert(&self, record: TrustRecord) -> Result<(), IdentityError> {
        self.records
            .lock()
            .map_err(|_| IdentityError::Poisoned)?
            .insert(record.device_id, record);
        Ok(())
    }

    fn get(&self, device_id: &DeviceId) -> Result<Option<TrustRecord>, IdentityError> {
        Ok(self
            .records
            .lock()
            .map_err(|_| IdentityError::Poisoned)?
            .get(device_id)
            .cloned())
    }

    fn list(&self) -> Result<Vec<TrustRecord>, IdentityError> {
        Ok(self
            .records
            .lock()
            .map_err(|_| IdentityError::Poisoned)?
            .values()
            .cloned()
            .collect())
    }

    fn revoke(&self, device_id: &DeviceId) -> Result<(), IdentityError> {
        if let Some(record) = self
            .records
            .lock()
            .map_err(|_| IdentityError::Poisoned)?
            .get_mut(device_id)
        {
            record.revoked = true;
        }
        Ok(())
    }
}

mod file;
pub use file::FileIdentityProvider;

fn digest_public_key(verifying_key: &[u8; KEY_BYTES]) -> [u8; KEY_BYTES] {
    Sha256::digest(verifying_key).into()
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn parse_hex(value: &str) -> Result<[u8; KEY_BYTES], IdentityError> {
    if value.len() != KEY_BYTES * 2 {
        return Err(IdentityError::InvalidHex);
    }
    let mut output = [0; KEY_BYTES];
    for (index, chunk) in value.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let high = hex_digit(chunk[0]).ok_or(IdentityError::InvalidHex)?;
        let low = hex_digit(chunk[1]).ok_or(IdentityError::InvalidHex)?;
        output[index] = high << 4 | low;
    }
    Ok(output)
}

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_identity_is_stable_from_secret_bytes() {
        let first = LocalIdentity::generate();
        let second = LocalIdentity::from_bytes(&first.secret_bytes()).expect("valid key");
        assert_eq!(first.device_id(), second.device_id());
        assert_eq!(first.fingerprint(), second.fingerprint());
        assert_eq!(first.verifying_key_bytes(), second.verifying_key_bytes());
    }

    #[test]
    fn memory_provider_load_or_create_is_stable() {
        let provider = MemoryIdentityProvider::new();
        let first = provider.load_or_create().expect("identity");
        let second = provider.load_or_create().expect("identity");
        assert_eq!(first.device_id(), second.device_id());
        assert_eq!(first.sign(b"hello"), second.sign(b"hello"));
    }

    #[test]
    fn signatures_reject_tampering() {
        let identity = LocalIdentity::generate();
        let signature = identity.sign(b"message");
        assert!(identity.verify(b"message", &signature).is_ok());
        assert!(matches!(
            identity.verify(b"tampered", &signature),
            Err(IdentityError::VerificationFailed)
        ));
    }

    #[test]
    fn memory_trust_store_revoke() {
        let identity = LocalIdentity::generate();
        let store = MemoryTrustStore::new();
        let record = TrustRecord::new(identity.device_id(), identity.fingerprint());
        store.upsert(record).expect("upsert");
        assert!(store.is_trusted(&identity.device_id()).expect("lookup"));
        store.revoke(&identity.device_id()).expect("revoke");
        assert!(!store.is_trusted(&identity.device_id()).expect("lookup"));
        assert!(
            store
                .get(&identity.device_id())
                .expect("lookup")
                .expect("record")
                .revoked
        );
    }
}
