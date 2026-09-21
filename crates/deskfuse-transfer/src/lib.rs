//! Transport-independent primitives for DeskFuse file and folder transfers.
//!
//! The crate deliberately contains no UI, socket, async-runtime, or GPUI code.  It
//! owns the protocol's data model and the secure filesystem boundary used by a
//! future runtime/transport implementation.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::derivable_impls,
    clippy::double_must_use,
    clippy::len_without_is_empty,
    clippy::manual_inspect,
    clippy::missing_errors_doc,
    clippy::must_use_candidate,
    clippy::redundant_closure_for_method_calls
)]

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::{Display, Formatter};
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

mod cache;
#[cfg(any(windows, test))]
mod windows_paths;
pub use cache::{CacheCleanup, ExternalCacheLeases, TransferCache};
mod clipboard_file;

pub use clipboard_file::{
    CLIPBOARD_FILE_MIME, ClipboardFileFrame, ClipboardFilePlan, ClipboardSourceFile,
    DeferredClipboardSource,
};

/// The protocol's default chunk size (1 MiB).
pub const DEFAULT_CHUNK_SIZE: usize = 1024 * 1024;
/// Bounded metadata, not a small-folder-only clipboard limit.
pub const MAX_MANIFEST_ENTRIES: usize = 1_000_000;
/// Alias retained for callers that prefer a byte-count constant.
pub const DEFAULT_CHUNK_SIZE_BYTES: u64 = DEFAULT_CHUNK_SIZE as u64;

/// Errors returned by manifest construction and transfer-core operations.
#[derive(Debug)]
pub enum TransferError {
    Io(io::Error),
    InvalidPath(String),
    InvalidManifest(String),
    InvalidChunk(String),
    InvalidBitmap(String),
    HashMismatch {
        expected: Sha256Digest,
        actual: Sha256Digest,
    },
    SymlinkNotAllowed(PathBuf),
    Conflict(PathBuf),
    RetryLimitExceeded(u64),
    OfferConflict(OfferKey),
    InvalidOfferKey {
        expected: OfferKey,
        actual: OfferKey,
    },
    Staging(String),
    State(String),
}

impl Display for TransferError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "I/O error: {error}"),
            Self::InvalidPath(path) => write!(f, "invalid relative path: {path}"),
            Self::InvalidManifest(message) => write!(f, "invalid manifest: {message}"),
            Self::InvalidChunk(message) => write!(f, "invalid chunk: {message}"),
            Self::InvalidBitmap(message) => write!(f, "invalid resume bitmap: {message}"),
            Self::HashMismatch { expected, actual } => {
                write!(f, "SHA-256 mismatch: expected {expected}, got {actual}")
            }
            Self::SymlinkNotAllowed(path) => {
                write!(f, "symlink is not allowed: {}", path.display())
            }
            Self::Conflict(path) => write!(f, "destination conflict: {}", path.display()),
            Self::RetryLimitExceeded(index) => write!(f, "retry limit exceeded for chunk {index}"),
            Self::OfferConflict(key) => {
                write!(f, "offer key already belongs to another transfer: {key}")
            }
            Self::InvalidOfferKey { expected, actual } => {
                write!(f, "invalid offer key: expected {expected}, got {actual}")
            }
            Self::Staging(message) => write!(f, "staging error: {message}"),
            Self::State(message) => write!(f, "invalid transfer state: {message}"),
        }
    }
}

impl std::error::Error for TransferError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for TransferError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

/// A stable UUID-backed transfer identifier.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TransferId(Uuid);

impl TransferId {
    /// Creates a random UUID-backed transfer identifier.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    #[must_use]
    pub const fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    #[must_use]
    pub const fn as_uuid(&self) -> &Uuid {
        &self.0
    }

    #[must_use]
    pub const fn into_uuid(self) -> Uuid {
        self.0
    }

    #[must_use]
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(Uuid::from_bytes(bytes))
    }

    #[must_use]
    pub const fn into_bytes(self) -> [u8; 16] {
        *self.0.as_bytes()
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 16] {
        self.0.as_bytes()
    }

    /// Returns the canonical lower-case UUID representation.
    #[must_use]
    pub fn to_hex(self) -> String {
        self.to_string()
    }
}

impl Default for TransferId {
    fn default() -> Self {
        Self::new()
    }
}

impl Display for TransferId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::str::FromStr for TransferId {
    type Err = TransferError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value)
            .map(Self)
            .map_err(|_| TransferError::InvalidManifest(format!("invalid transfer id: {value}")))
    }
}

impl From<Uuid> for TransferId {
    fn from(value: Uuid) -> Self {
        Self(value)
    }
}

impl Serialize for TransferId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for TransferId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

/// SHA-256 digest used for complete files, chunks, and manifests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Sha256Digest([u8; 32]);

impl Sha256Digest {
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn into_bytes(self) -> [u8; 32] {
        self.0
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Render the canonical lower-case hexadecimal representation.
    #[must_use]
    pub fn to_hex(self) -> String {
        self.to_string()
    }

    pub fn from_hex(value: &str) -> Result<Self, TransferError> {
        if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(TransferError::InvalidManifest(format!(
                "invalid SHA-256 digest: {value}"
            )));
        }
        let mut bytes = [0_u8; 32];
        for (index, pair) in value.as_bytes().as_chunks::<2>().0.iter().enumerate() {
            bytes[index] = (hex_value(pair[0]) << 4) | hex_value(pair[1]);
        }
        Ok(Self(bytes))
    }
}

impl Display for Sha256Digest {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl Serialize for Sha256Digest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Sha256Digest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

impl std::str::FromStr for Sha256Digest {
    type Err = TransferError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::from_hex(value)
    }
}

/// Hashes a byte slice with SHA-256.
#[must_use]
pub fn sha256_bytes(bytes: &[u8]) -> Sha256Digest {
    let digest = Sha256::digest(bytes);
    let mut result = [0_u8; 32];
    result.copy_from_slice(&digest);
    Sha256Digest(result)
}

/// Verifies a byte slice against an expected SHA-256 digest.
pub fn verify_sha256(bytes: &[u8], expected: Sha256Digest) -> Result<(), TransferError> {
    let actual = sha256_bytes(bytes);
    if actual == expected {
        Ok(())
    } else {
        Err(TransferError::HashMismatch { expected, actual })
    }
}

/// A validated path relative to a transfer root.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RelativePath(String);

impl RelativePath {
    /// Validates a path using platform-independent rules.  In particular,
    /// backslashes are rejected even on Unix so a manifest cannot become unsafe
    /// when consumed on Windows later.
    pub fn new(path: impl AsRef<Path>) -> Result<Self, TransferError> {
        let path = path.as_ref();
        let raw = path
            .to_str()
            .ok_or_else(|| TransferError::InvalidPath("non-UTF-8 path".into()))?;
        if raw.is_empty() || raw == "." || raw.starts_with('/') || raw.starts_with('\\') {
            return Err(TransferError::InvalidPath(raw.into()));
        }
        if raw.contains('\\') || raw.contains('\0') {
            return Err(TransferError::InvalidPath(raw.into()));
        }
        // Windows drive prefixes are not represented as Prefix on Unix.
        if raw.as_bytes().get(1).is_some_and(|byte| *byte == b':') {
            return Err(TransferError::InvalidPath(raw.into()));
        }
        let mut components = Vec::new();
        for component in path.components() {
            match component {
                Component::Normal(value) => {
                    let value = value
                        .to_str()
                        .ok_or_else(|| TransferError::InvalidPath(raw.into()))?;
                    if value.is_empty() || value == "." || value == ".." || value.contains('\\') {
                        return Err(TransferError::InvalidPath(raw.into()));
                    }
                    #[cfg(windows)]
                    windows_paths::validate_component(value)?;
                    components.push(value.to_owned());
                }
                Component::CurDir
                | Component::ParentDir
                | Component::RootDir
                | Component::Prefix(_) => {
                    return Err(TransferError::InvalidPath(raw.to_owned()));
                }
            }
        }
        if components.is_empty() {
            return Err(TransferError::InvalidPath(raw.to_owned()));
        }
        Ok(Self(components.join("/")))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn to_path_buf(&self) -> PathBuf {
        PathBuf::from(&self.0)
    }
}

impl Display for RelativePath {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<Path> for RelativePath {
    fn as_ref(&self) -> &Path {
        Path::new(&self.0)
    }
}

impl Serialize for RelativePath {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for RelativePath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// Alias for code that calls a manifest item a manifest entry.
pub type ManifestEntry = TransferEntry;
/// Alias for code that calls an entry's type its kind.
pub type EntryKind = EntryType;
/// Alias for code that calls a SHA-256 digest a content hash.
pub type ContentHash = Sha256Digest;

/// Kind of item represented in a transfer manifest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryType {
    File,
    Directory,
    Symlink,
}

/// One manifest entry.  Directory entries have `hash == None` and `size == 0`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferEntry {
    path: RelativePath,
    entry_type: EntryType,
    size: u64,
    /// Modification time in nanoseconds since Unix epoch.
    mtime: i128,
    hash: Option<Sha256Digest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    link_target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mode: Option<u32>,
}

impl TransferEntry {
    pub fn file(
        path: impl AsRef<Path>,
        size: u64,
        mtime: i128,
        hash: Sha256Digest,
    ) -> Result<Self, TransferError> {
        Ok(Self {
            path: RelativePath::new(path)?,
            entry_type: EntryType::File,
            size,
            mtime,
            hash: Some(hash),
            link_target: None,
            mode: None,
        })
    }

    pub fn directory(path: impl AsRef<Path>, mtime: i128) -> Result<Self, TransferError> {
        Ok(Self {
            path: RelativePath::new(path)?,
            entry_type: EntryType::Directory,
            size: 0,
            mtime,
            hash: None,
            link_target: None,
            mode: None,
        })
    }

    /// Preserve a link as metadata; never read or traverse its target.
    pub fn symlink(
        path: impl AsRef<Path>,
        target: impl AsRef<Path>,
        mtime: i128,
    ) -> Result<Self, TransferError> {
        let mut entry = Self::directory(path, mtime)?;
        entry.entry_type = EntryType::Symlink;
        let target = target
            .as_ref()
            .to_str()
            .ok_or_else(|| TransferError::InvalidPath("符号链接目标不是 UTF-8".into()))?;
        validate_link_target(&entry.path, target)?;
        entry.link_target = Some(target.to_owned());
        Ok(entry)
    }

    pub fn link_target(&self) -> Option<&str> {
        self.link_target.as_deref()
    }

    fn with_metadata(mut self, metadata: &fs::Metadata) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if self.entry_type != EntryType::Symlink {
                self.mode = Some(metadata.permissions().mode() & 0o777);
            }
        }
        self
    }

    pub(crate) fn prefixed(&self, prefix: &RelativePath) -> Result<Self, TransferError> {
        let mut entry = self.clone();
        entry.path = RelativePath::new(prefix.as_ref().join(self.path.as_ref()))?;
        Ok(entry)
    }

    /// Constructs an entry from its explicit type.  Directories must have
    /// zero size and no hash; files must carry a complete-file hash.
    pub fn new(
        path: impl AsRef<Path>,
        entry_type: EntryType,
        size: u64,
        mtime: i128,
        hash: Option<Sha256Digest>,
    ) -> Result<Self, TransferError> {
        match entry_type {
            EntryType::Symlink => Err(TransferError::InvalidManifest(
                "symbolic link requires a target".into(),
            )),
            EntryType::File => Self::file(
                path,
                size,
                mtime,
                hash.ok_or_else(|| {
                    TransferError::InvalidManifest("file entry has no hash".into())
                })?,
            ),
            EntryType::Directory => {
                if size != 0 || hash.is_some() {
                    return Err(TransferError::InvalidManifest(
                        "directory entry has file metadata".into(),
                    ));
                }
                Self::directory(path, mtime)
            }
        }
    }

    /// Alias for [`Self::file`].
    pub fn new_file(
        path: impl AsRef<Path>,
        size: u64,
        mtime: i128,
        hash: Sha256Digest,
    ) -> Result<Self, TransferError> {
        Self::file(path, size, mtime, hash)
    }

    /// Alias for [`Self::directory`].
    pub fn new_directory(path: impl AsRef<Path>, mtime: i128) -> Result<Self, TransferError> {
        Self::directory(path, mtime)
    }

    #[must_use]
    pub fn path(&self) -> &RelativePath {
        &self.path
    }

    #[must_use]
    pub fn entry_type(&self) -> EntryType {
        self.entry_type
    }

    #[must_use]
    pub fn kind(&self) -> EntryType {
        self.entry_type
    }

    #[must_use]
    pub fn is_file(&self) -> bool {
        self.entry_type == EntryType::File
    }

    #[must_use]
    pub fn is_directory(&self) -> bool {
        self.entry_type == EntryType::Directory
    }

    #[must_use]
    pub fn size(&self) -> u64 {
        self.size
    }

    #[must_use]
    pub fn mtime(&self) -> i128 {
        self.mtime
    }

    #[must_use]
    pub fn hash(&self) -> Option<Sha256Digest> {
        self.hash
    }

    #[must_use]
    pub fn modified_at(&self) -> i128 {
        self.mtime
    }

    #[must_use]
    pub fn modified_system_time(&self) -> SystemTime {
        if self.mtime >= 0 {
            UNIX_EPOCH + Duration::from_nanos(self.mtime.min(u64::MAX as i128) as u64)
        } else {
            UNIX_EPOCH
                .checked_sub(Duration::from_nanos(
                    (-self.mtime).min(u64::MAX as i128) as u64
                ))
                .unwrap_or(UNIX_EPOCH)
        }
    }

    pub fn chunks(&self, chunk_size: usize) -> Result<Vec<ChunkDescriptor>, TransferError> {
        if !self.is_file() {
            return Ok(Vec::new());
        }
        chunk_descriptors(self.size, chunk_size, self.hash)
    }
}

/// A complete, deterministic transfer manifest.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct TransferManifest {
    entries: Vec<TransferEntry>,
    total_bytes: u64,
    file_count: u64,
    directory_count: u64,
}

impl<'de> Deserialize<'de> for TransferManifest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct ManifestWire {
            entries: Vec<TransferEntry>,
            #[serde(default)]
            total_bytes: Option<u64>,
            #[serde(default)]
            file_count: Option<u64>,
            #[serde(default)]
            directory_count: Option<u64>,
        }
        let wire = ManifestWire::deserialize(deserializer)?;
        let manifest = Self::new(wire.entries).map_err(serde::de::Error::custom)?;
        // Derived fields are intentionally recomputed instead of trusting the
        // sender, preventing stale or forged aggregate counts.
        let _ = (wire.total_bytes, wire.file_count, wire.directory_count);
        Ok(manifest)
    }
}

impl TransferManifest {
    pub fn new(mut entries: Vec<TransferEntry>) -> Result<Self, TransferError> {
        if entries.len() > MAX_MANIFEST_ENTRIES {
            return Err(TransferError::InvalidManifest(
                "文件目录超过 100 万项，请分批复制".into(),
            ));
        }
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        let mut paths = BTreeSet::new();
        let mut path_kinds = BTreeMap::new();
        let mut total_bytes: u64 = 0;
        let mut file_count = 0;
        let mut directory_count = 0;
        for entry in &entries {
            if !paths.insert(entry.path.as_str()) {
                return Err(TransferError::InvalidManifest(format!(
                    "duplicate path {}",
                    entry.path
                )));
            }
            let mut prefix = String::new();
            let components = entry.path.as_str().split('/').collect::<Vec<_>>();
            for component in components.iter().take(components.len().saturating_sub(1)) {
                if !prefix.is_empty() {
                    prefix.push('/');
                }
                prefix.push_str(component);
                if path_kinds
                    .get(&prefix)
                    .is_some_and(|kind| *kind != EntryType::Directory)
                {
                    return Err(TransferError::InvalidManifest(format!(
                        "file {} is an ancestor of {}",
                        prefix, entry.path
                    )));
                }
            }
            path_kinds.insert(entry.path.as_str().to_owned(), entry.entry_type);
            if entry.entry_type == EntryType::Symlink {
                if entry.size != 0 || entry.hash.is_some() || entry.mode.is_some() {
                    return Err(TransferError::InvalidManifest(
                        "symbolic link has file metadata".into(),
                    ));
                }
                let target = entry.link_target.as_deref().ok_or_else(|| {
                    TransferError::InvalidManifest("symbolic link has no target".into())
                })?;
                validate_link_target(&entry.path, target)?;
                file_count += 1;
                continue;
            }
            if entry.link_target.is_some() || entry.mode.is_some_and(|mode| mode & !0o777 != 0) {
                return Err(TransferError::InvalidManifest(
                    "invalid file metadata".into(),
                ));
            }
            if entry.is_file() {
                if entry.hash.is_none() {
                    return Err(TransferError::InvalidManifest(format!(
                        "file {} has no hash",
                        entry.path
                    )));
                }
                total_bytes = total_bytes
                    .checked_add(entry.size)
                    .ok_or_else(|| TransferError::InvalidManifest("total size overflow".into()))?;
                file_count += 1;
            } else {
                if entry.size != 0 || entry.hash.is_some() {
                    return Err(TransferError::InvalidManifest(format!(
                        "directory {} has file metadata",
                        entry.path
                    )));
                }
                directory_count += 1;
            }
        }
        let manifest = Self {
            entries,
            total_bytes,
            file_count,
            directory_count,
        };
        #[cfg(windows)]
        windows_paths::validate_manifest(&manifest)?;
        Ok(manifest)
    }

    /// Alias for [`TransferManifest::new`].
    pub fn from_entries(entries: Vec<TransferEntry>) -> Result<Self, TransferError> {
        Self::new(entries)
    }

    /// Builds a manifest without following links. Internal relative links are
    /// retained as metadata; an external or top-level link is rejected.
    pub fn from_path(root: impl AsRef<Path>) -> Result<Self, TransferError> {
        Self::from_path_observed(root.as_ref(), &|| false, &mut |_| {})
    }

    pub(crate) fn from_path_observed(
        root: &Path,
        cancelled: &impl Fn() -> bool,
        progress: &mut impl FnMut(usize),
    ) -> Result<Self, TransferError> {
        if cancelled() {
            return Err(TransferError::State("文件准备已取消".into()));
        }
        let metadata = fs::symlink_metadata(root)?;
        if metadata.file_type().is_symlink() {
            return Err(TransferError::SymlinkNotAllowed(root.to_path_buf()));
        }
        if metadata.is_file() {
            let name = root
                .file_name()
                .ok_or_else(|| TransferError::InvalidManifest("file has no name".into()))?;
            let relative = RelativePath::new(Path::new(name))?;
            let hash = hash_file_cancellable(root, cancelled)?;
            progress(1);
            return Self::new(vec![
                TransferEntry {
                    path: relative,
                    entry_type: EntryType::File,
                    size: metadata.len(),
                    mtime: metadata_mtime(&metadata),
                    hash: Some(hash),
                    link_target: None,
                    mode: None,
                }
                .with_metadata(&metadata),
            ]);
        }
        if !metadata.is_dir() {
            return Err(TransferError::InvalidManifest(format!(
                "unsupported root: {}",
                root.display()
            )));
        }
        let mut entries = Vec::new();
        collect_manifest_entries(root, Path::new(""), &mut entries, cancelled, progress)?;
        Self::new(entries)
    }

    /// Alias for [`TransferManifest::from_path`].
    pub fn from_directory(root: impl AsRef<Path>) -> Result<Self, TransferError> {
        Self::from_path(root)
    }

    /// Alias for [`TransferManifest::from_path`].
    pub fn from_dir(root: impl AsRef<Path>) -> Result<Self, TransferError> {
        Self::from_path(root)
    }

    /// Alias for [`TransferManifest::from_path`].
    pub fn from_local_path(root: impl AsRef<Path>) -> Result<Self, TransferError> {
        Self::from_path(root)
    }

    /// Alias useful to callers that build manifests from a user-selected path.
    pub fn build(root: impl AsRef<Path>) -> Result<Self, TransferError> {
        Self::from_path(root)
    }

    #[must_use]
    pub fn entries(&self) -> &[TransferEntry] {
        &self.entries
    }

    #[must_use]
    pub fn files(&self) -> impl Iterator<Item = &TransferEntry> {
        self.entries.iter().filter(|entry| entry.is_file())
    }

    #[must_use]
    pub fn directories(&self) -> impl Iterator<Item = &TransferEntry> {
        self.entries.iter().filter(|entry| entry.is_directory())
    }

    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    #[must_use]
    pub fn total_size(&self) -> u64 {
        self.total_bytes
    }

    #[must_use]
    pub fn file_count(&self) -> u64 {
        self.file_count
    }

    #[must_use]
    pub fn directory_count(&self) -> u64 {
        self.directory_count
    }

    #[must_use]
    pub fn digest(&self) -> Sha256Digest {
        let mut hasher = Sha256::new();
        for entry in &self.entries {
            hasher.update(entry.path.as_str().as_bytes());
            hasher.update([0]);
            hasher.update([match entry.entry_type {
                EntryType::File => 1,
                EntryType::Directory => 2,
                EntryType::Symlink => 3,
            }]);
            hasher.update(entry.size.to_le_bytes());
            hasher.update(entry.mtime.to_le_bytes());
            if let Some(target) = &entry.link_target {
                hasher.update(target.as_bytes());
            }
            hasher.update([0]);
            hasher.update(entry.mode.unwrap_or_default().to_le_bytes());
            if let Some(hash) = entry.hash {
                hasher.update([1]);
                hasher.update(hash.0);
            } else {
                hasher.update([0]);
            }
        }
        let digest = hasher.finalize();
        let mut bytes = [0_u8; 32];
        bytes.copy_from_slice(&digest);
        Sha256Digest(bytes)
    }

    #[must_use]
    pub fn manifest_hash(&self) -> Sha256Digest {
        self.digest()
    }

    pub fn chunk_plan(
        &self,
        chunk_size: usize,
    ) -> Result<Vec<(RelativePath, ChunkDescriptor)>, TransferError> {
        let mut plan = Vec::new();
        for entry in self.files() {
            for chunk in entry.chunks(chunk_size)? {
                plan.push((entry.path.clone(), chunk));
            }
        }
        Ok(plan)
    }

    /// Returns the default 1 MiB chunk plan.
    pub fn default_chunk_plan(
        &self,
    ) -> Result<Vec<(RelativePath, ChunkDescriptor)>, TransferError> {
        self.chunk_plan(DEFAULT_CHUNK_SIZE)
    }
}

/// A chunk's byte range and optional expected digest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkDescriptor {
    pub index: u64,
    pub offset: u64,
    pub length: u32,
    pub hash: Option<Sha256Digest>,
}

impl ChunkDescriptor {
    pub fn new(index: u64, offset: u64, length: u32) -> Result<Self, TransferError> {
        if length == 0 {
            return Err(TransferError::InvalidChunk(
                "chunk length must be non-zero".into(),
            ));
        }
        if offset.checked_add(length as u64).is_none() {
            return Err(TransferError::InvalidChunk(
                "chunk range overflows u64".into(),
            ));
        }
        Ok(Self {
            index,
            offset,
            length,
            hash: None,
        })
    }

    #[must_use]
    pub fn with_hash(mut self, hash: Sha256Digest) -> Self {
        self.hash = Some(hash);
        self
    }

    #[must_use]
    pub const fn end_offset(&self) -> u64 {
        self.offset.saturating_add(self.length as u64)
    }

    /// Validates that this descriptor's range is representable and non-empty.
    pub fn validate(&self) -> Result<(), TransferError> {
        if self.length == 0 {
            return Err(TransferError::InvalidChunk(
                "chunk length must be non-zero".into(),
            ));
        }
        self.offset
            .checked_add(self.length as u64)
            .ok_or_else(|| TransferError::InvalidChunk("chunk range overflows u64".into()))?;
        Ok(())
    }

    #[must_use]
    pub const fn len(&self) -> u32 {
        self.length
    }
}

/// Creates fixed-size chunk descriptors for a file.
pub fn chunk_descriptors(
    size: u64,
    chunk_size: usize,
    whole_file_hash: Option<Sha256Digest>,
) -> Result<Vec<ChunkDescriptor>, TransferError> {
    if chunk_size == 0 || chunk_size > u32::MAX as usize {
        return Err(TransferError::InvalidChunk(format!(
            "invalid chunk size {chunk_size}"
        )));
    }
    if size == 0 {
        return Ok(Vec::new());
    }
    let count = size.div_ceil(chunk_size as u64);
    let capacity = usize::try_from(count)
        .map_err(|_| TransferError::InvalidChunk("too many chunks for this platform".into()))?;
    let mut chunks = Vec::with_capacity(capacity);
    for index in 0..count {
        let offset = index * chunk_size as u64;
        let length = (size - offset).min(chunk_size as u64) as u32;
        let mut descriptor = ChunkDescriptor::new(index, offset, length)?;
        // The complete-file digest cannot be used as a chunk digest; retain it
        // only on the final descriptor as a useful end-to-end hint would be
        // misleading.  Chunk hashes are filled by `read_chunk` below.
        if count == 1 {
            descriptor.hash = whole_file_hash;
        }
        chunks.push(descriptor);
    }
    Ok(chunks)
}

/// Creates descriptors with a per-chunk SHA-256 hash from an in-memory file.
pub fn chunk_descriptors_for_data(
    data: &[u8],
    chunk_size: usize,
) -> Result<Vec<ChunkDescriptor>, TransferError> {
    let mut descriptors = chunk_descriptors(data.len() as u64, chunk_size, None)?;
    for descriptor in &mut descriptors {
        let start = descriptor.offset as usize;
        let end = start + descriptor.length as usize;
        descriptor.hash = Some(sha256_bytes(&data[start..end]));
    }
    Ok(descriptors)
}

/// Alias for [`chunk_descriptors`].
pub fn chunks_for_size(
    size: u64,
    chunk_size: usize,
) -> Result<Vec<ChunkDescriptor>, TransferError> {
    chunk_descriptors(size, chunk_size, None)
}

/// Reads and hashes one chunk from a local file.
pub fn read_chunk(
    path: impl AsRef<Path>,
    descriptor: ChunkDescriptor,
) -> Result<Vec<u8>, TransferError> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(descriptor.offset))?;
    let mut bytes = vec![0_u8; descriptor.length as usize];
    file.read_exact(&mut bytes)?;
    if let Some(expected) = descriptor.hash {
        verify_sha256(&bytes, expected)?;
    }
    Ok(bytes)
}

/// Resume state for a transfer's chunk indexes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeBitmap {
    total_chunks: u64,
    words: Vec<u64>,
}

impl ResumeBitmap {
    pub fn new(total_chunks: u64) -> Self {
        let word_count = total_chunks.div_ceil(64) as usize;
        Self {
            total_chunks,
            words: vec![0; word_count],
        }
    }

    pub fn from_completed(
        total_chunks: u64,
        completed: impl IntoIterator<Item = u64>,
    ) -> Result<Self, TransferError> {
        let mut bitmap = Self::new(total_chunks);
        for index in completed {
            bitmap.mark_complete(index)?;
        }
        Ok(bitmap)
    }

    #[must_use]
    pub const fn total_chunks(&self) -> u64 {
        self.total_chunks
    }

    pub fn mark_complete(&mut self, index: u64) -> Result<bool, TransferError> {
        self.check_index(index)?;
        let word = index as usize / 64;
        let bit = 1_u64 << (index % 64);
        let was_new = self.words[word] & bit == 0;
        self.words[word] |= bit;
        Ok(was_new)
    }

    pub fn mark_missing(&mut self, index: u64) -> Result<bool, TransferError> {
        self.check_index(index)?;
        let word = index as usize / 64;
        let bit = 1_u64 << (index % 64);
        let was_complete = self.words[word] & bit != 0;
        self.words[word] &= !bit;
        Ok(was_complete)
    }

    /// Alias for [`Self::mark_complete`].
    pub fn set(&mut self, index: u64) -> Result<bool, TransferError> {
        self.mark_complete(index)
    }

    /// Alias for [`Self::mark_missing`].
    pub fn clear(&mut self, index: u64) -> Result<bool, TransferError> {
        self.mark_missing(index)
    }

    #[must_use]
    pub fn is_complete(&self, index: u64) -> bool {
        if index >= self.total_chunks {
            return false;
        }
        self.words[index as usize / 64] & (1_u64 << (index % 64)) != 0
    }

    #[must_use]
    pub fn completed_count(&self) -> u64 {
        self.words
            .iter()
            .map(|word| u64::from(word.count_ones()))
            .sum()
    }

    #[must_use]
    pub fn missing_count(&self) -> u64 {
        self.total_chunks - self.completed_count()
    }

    #[must_use]
    pub fn is_complete_all(&self) -> bool {
        self.completed_count() == self.total_chunks
    }

    #[must_use]
    pub fn missing_indices(&self) -> impl Iterator<Item = u64> + '_ {
        (0..self.total_chunks).filter(|index| !self.is_complete(*index))
    }

    #[must_use]
    pub fn as_words(&self) -> &[u64] {
        &self.words
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.words.len() * 8);
        for word in &self.words {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        bytes
    }

    pub fn from_bytes(total_chunks: u64, bytes: &[u8]) -> Result<Self, TransferError> {
        let expected = total_chunks.div_ceil(64) as usize * 8;
        if bytes.len() != expected {
            return Err(TransferError::InvalidBitmap(format!(
                "expected {expected} bytes, got {}",
                bytes.len()
            )));
        }
        let mut words = Vec::with_capacity(expected / 8);
        for chunk in bytes.as_chunks::<8>().0 {
            let mut word = [0_u8; 8];
            word.copy_from_slice(chunk);
            words.push(u64::from_le_bytes(word));
        }
        if !total_chunks.is_multiple_of(64) {
            let valid = total_chunks % 64;
            let mask = (1_u64 << valid) - 1;
            if words.last().is_some_and(|word| *word & !mask != 0) {
                return Err(TransferError::InvalidBitmap(
                    "bits beyond total_chunks are set".into(),
                ));
            }
        }
        Ok(Self {
            total_chunks,
            words,
        })
    }

    fn check_index(&self, index: u64) -> Result<(), TransferError> {
        if index >= self.total_chunks {
            Err(TransferError::InvalidBitmap(format!(
                "chunk index {index} out of range"
            )))
        } else {
            Ok(())
        }
    }
}

/// A resumable transfer plan derived from a bitmap.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumePlan {
    bitmap: ResumeBitmap,
}

impl ResumePlan {
    #[must_use]
    pub fn new(total_chunks: u64) -> Self {
        Self {
            bitmap: ResumeBitmap::new(total_chunks),
        }
    }

    #[must_use]
    pub fn from_bitmap(bitmap: ResumeBitmap) -> Self {
        Self { bitmap }
    }

    #[must_use]
    pub fn bitmap(&self) -> &ResumeBitmap {
        &self.bitmap
    }

    #[must_use]
    pub fn total_chunks(&self) -> u64 {
        self.bitmap.total_chunks()
    }

    #[must_use]
    pub fn completed_count(&self) -> u64 {
        self.bitmap.completed_count()
    }

    #[must_use]
    pub fn missing(&self) -> impl Iterator<Item = u64> + '_ {
        self.bitmap.missing_indices()
    }

    #[must_use]
    pub fn next_batch(&self, max_chunks: usize) -> Vec<u64> {
        self.missing().take(max_chunks).collect()
    }

    pub fn mark_complete(&mut self, index: u64) -> Result<bool, TransferError> {
        self.bitmap.mark_complete(index)
    }

    pub fn mark_missing(&mut self, index: u64) -> Result<bool, TransferError> {
        self.bitmap.mark_missing(index)
    }
}

/// Lifecycle state of a transfer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferState {
    Offered,
    Accepted,
    Transferring,
    Verifying,
    Completed,
    Paused,
    Failed(String),
    Cancelled,
}

impl Default for TransferState {
    fn default() -> Self {
        Self::Offered
    }
}

impl TransferState {
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed(_) | Self::Cancelled)
    }
}

/// Progress snapshot suitable for UI or telemetry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferProgress {
    pub state: TransferState,
    pub total_bytes: u64,
    pub transferred_bytes: u64,
    pub total_chunks: u64,
    pub completed_chunks: u64,
}

impl TransferProgress {
    #[must_use]
    pub fn new(total_bytes: u64, total_chunks: u64) -> Self {
        Self {
            state: TransferState::Offered,
            total_bytes,
            transferred_bytes: 0,
            total_chunks,
            completed_chunks: 0,
        }
    }

    #[must_use]
    pub fn fraction(&self) -> f64 {
        if self.total_bytes == 0 {
            1.0
        } else {
            self.transferred_bytes as f64 / self.total_bytes as f64
        }
    }

    #[must_use]
    pub fn percent(&self) -> f64 {
        self.fraction() * 100.0
    }

    pub fn apply_chunk(&mut self, length: u64) {
        self.transferred_bytes = self
            .transferred_bytes
            .saturating_add(length)
            .min(self.total_bytes);
        self.completed_chunks = self
            .completed_chunks
            .saturating_add(1)
            .min(self.total_chunks);
    }
}

/// Retry behavior for the ACK window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub retry_after: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            retry_after: Duration::from_millis(500),
        }
    }
}

impl RetryPolicy {
    #[must_use]
    pub const fn new(max_retries: u32, retry_after: Duration) -> Self {
        Self {
            max_retries,
            retry_after,
        }
    }
}

#[derive(Clone, Debug)]
struct PendingAck {
    descriptor: ChunkDescriptor,
    attempts: u32,
    sent_at: SystemTime,
}

/// Sliding ACK/retry window.  It only tracks protocol state; sending is owned
/// by the future transport implementation.
#[derive(Clone, Debug)]
pub struct AckWindow {
    capacity: usize,
    policy: RetryPolicy,
    pending: BTreeMap<u64, PendingAck>,
}

impl AckWindow {
    #[must_use]
    pub fn new(capacity: usize, policy: RetryPolicy) -> Self {
        Self {
            capacity: capacity.max(1),
            policy,
            pending: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn with_max_retries(capacity: usize, max_retries: u32) -> Self {
        Self::new(
            capacity,
            RetryPolicy::new(max_retries, RetryPolicy::default().retry_after),
        )
    }

    #[must_use]
    pub fn policy(&self) -> RetryPolicy {
        self.policy
    }

    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    #[must_use]
    pub fn in_flight(&self) -> usize {
        self.pending.len()
    }

    #[must_use]
    pub fn can_send(&self) -> bool {
        self.pending.len() < self.capacity
    }

    pub fn register_sent(&mut self, descriptor: ChunkDescriptor) -> Result<(), TransferError> {
        if !self.can_send() {
            return Err(TransferError::State("ACK window is full".into()));
        }
        if self.pending.contains_key(&descriptor.index) {
            return Err(TransferError::State(format!(
                "chunk {} is already pending",
                descriptor.index
            )));
        }
        self.pending.insert(
            descriptor.index,
            PendingAck {
                descriptor,
                attempts: 0,
                sent_at: SystemTime::now(),
            },
        );
        Ok(())
    }

    pub fn acknowledge(&mut self, index: u64) -> Option<ChunkDescriptor> {
        self.pending
            .remove(&index)
            .map(|pending| pending.descriptor)
    }

    pub fn ack(&mut self, index: u64) -> Option<ChunkDescriptor> {
        self.acknowledge(index)
    }

    pub fn negative_acknowledge(&mut self, index: u64) -> Result<ChunkDescriptor, TransferError> {
        let pending = self
            .pending
            .get_mut(&index)
            .ok_or_else(|| TransferError::State(format!("unknown chunk {index}")))?;
        if pending.attempts >= self.policy.max_retries {
            return Err(TransferError::RetryLimitExceeded(index));
        }
        pending.attempts += 1;
        pending.sent_at = SystemTime::now();
        Ok(pending.descriptor)
    }

    pub fn nack(&mut self, index: u64) -> Result<ChunkDescriptor, TransferError> {
        self.negative_acknowledge(index)
    }

    #[must_use]
    pub fn retry_count(&self, index: u64) -> Option<u32> {
        self.pending.get(&index).map(|pending| pending.attempts)
    }

    /// Returns due chunks without removing them.  The caller should retransmit
    /// the returned descriptors and call [`Self::record_retry`] afterwards.
    #[must_use]
    pub fn due_retries(&self, now: SystemTime) -> Vec<ChunkDescriptor> {
        self.pending
            .values()
            .filter(|pending| {
                now.duration_since(pending.sent_at).unwrap_or_default() >= self.policy.retry_after
            })
            .filter(|pending| pending.attempts < self.policy.max_retries)
            .map(|pending| pending.descriptor)
            .collect()
    }

    pub fn record_retry(&mut self, index: u64) -> Result<(), TransferError> {
        let pending = self
            .pending
            .get_mut(&index)
            .ok_or_else(|| TransferError::State(format!("unknown chunk {index}")))?;
        if pending.attempts >= self.policy.max_retries {
            return Err(TransferError::RetryLimitExceeded(index));
        }
        pending.attempts += 1;
        pending.sent_at = SystemTime::now();
        Ok(())
    }

    #[must_use]
    pub fn pending(&self) -> impl Iterator<Item = ChunkDescriptor> + '_ {
        self.pending.values().map(|pending| pending.descriptor)
    }
}

/// Conflict policy applied when committing staged files.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConflictStrategy {
    #[default]
    Rename,
    Overwrite,
    Skip,
}

/// Alias for callers that name a conflict strategy a policy.
pub type ConflictPolicy = ConflictStrategy;

/// Result of resolving one destination path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConflictResolution {
    Write(PathBuf),
    Skip(PathBuf),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PathState {
    Absent,
    Present,
}

/// Performs an lstat-style existence check without turning permission or other
/// filesystem failures into a false "not found" result.
fn path_state(path: &Path) -> Result<PathState, TransferError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(PathState::Present),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(PathState::Absent),
        Err(error) => Err(TransferError::Io(error)),
    }
}

/// Chooses a safe destination path according to the conflict policy.
pub fn resolve_conflict(
    path: impl AsRef<Path>,
    strategy: ConflictStrategy,
) -> Result<ConflictResolution, TransferError> {
    let path = path.as_ref();
    if path_state(path)? == PathState::Absent {
        return Ok(ConflictResolution::Write(path.to_path_buf()));
    }
    match strategy {
        ConflictStrategy::Overwrite => Ok(ConflictResolution::Write(path.to_path_buf())),
        ConflictStrategy::Skip => Ok(ConflictResolution::Skip(path.to_path_buf())),
        ConflictStrategy::Rename => {
            let parent = path.parent().unwrap_or_else(|| Path::new("."));
            let file_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| TransferError::Conflict(path.to_path_buf()))?;
            let (stem, extension) = match path.extension().and_then(|ext| ext.to_str()) {
                Some(extension) if !extension.is_empty() => {
                    let suffix = format!(".{extension}");
                    (&file_name[..file_name.len() - suffix.len()], suffix)
                }
                _ => (file_name, String::new()),
            };
            for number in 1_u64.. {
                let candidate = parent.join(format!("{stem} ({number}){extension}"));
                if path_state(&candidate)? == PathState::Absent {
                    return Ok(ConflictResolution::Write(candidate));
                }
            }
            unreachable!("u64 range is finite but practically unexhaustible")
        }
    }
}

/// An idempotency key for a transfer offer.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord,
)]
pub struct OfferKey(Sha256Digest);

impl OfferKey {
    #[must_use]
    pub fn new(transfer_id: TransferId, manifest_digest: Sha256Digest) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(transfer_id.0);
        hasher.update(manifest_digest.0);
        let digest = hasher.finalize();
        let mut bytes = [0_u8; 32];
        bytes.copy_from_slice(&digest);
        Self(Sha256Digest(bytes))
    }

    #[must_use]
    pub const fn digest(&self) -> Sha256Digest {
        self.0
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        self.0.as_bytes()
    }

    #[must_use]
    pub fn to_hex(self) -> String {
        self.to_string()
    }
}

impl Display for OfferKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::str::FromStr for OfferKey {
    type Err = TransferError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(Self(Sha256Digest::from_hex(value)?))
    }
}

/// A complete offer sent before any chunks are transferred.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferOffer {
    pub transfer_id: TransferId,
    pub manifest: TransferManifest,
    pub offer_key: OfferKey,
}

impl TransferOffer {
    #[must_use]
    pub fn new(transfer_id: TransferId, manifest: TransferManifest) -> Self {
        let offer_key = OfferKey::new(transfer_id, manifest.digest());
        Self {
            transfer_id,
            manifest,
            offer_key,
        }
    }

    #[must_use]
    pub fn generated(manifest: TransferManifest) -> Self {
        Self::new(TransferId::new(), manifest)
    }

    #[must_use]
    pub fn idempotency_key(&self) -> OfferKey {
        self.offer_key
    }
}

/// Outcome of processing an offer.  Duplicate offers are acknowledged without
/// creating another staging directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OfferOutcome {
    Accepted(TransferId),
    Duplicate(TransferId),
}

/// In-memory idempotency registry used by a transport/runtime implementation.
#[derive(Clone, Debug, Default)]
pub struct OfferRegistry {
    offers: HashMap<OfferKey, (TransferId, Sha256Digest)>,
}

impl OfferRegistry {
    pub fn accept(&mut self, offer: &TransferOffer) -> Result<OfferOutcome, TransferError> {
        // `offer_key` is a derived idempotency value, not an authority supplied by
        // the peer.  Recompute it from the authenticated protocol fields before
        // looking in (or mutating) the registry so forged keys cannot alias a real
        // transfer or create an uncheckable registry entry.
        let expected = OfferKey::new(offer.transfer_id, offer.manifest.digest());
        if offer.offer_key != expected {
            return Err(TransferError::InvalidOfferKey {
                expected,
                actual: offer.offer_key,
            });
        }
        if let Some((transfer_id, digest)) = self.offers.get(&expected).copied() {
            if digest != offer.manifest.digest() || transfer_id != offer.transfer_id {
                return Err(TransferError::OfferConflict(expected));
            }
            return Ok(OfferOutcome::Duplicate(transfer_id));
        }
        self.offers
            .insert(expected, (offer.transfer_id, offer.manifest.digest()));
        Ok(OfferOutcome::Accepted(offer.transfer_id))
    }

    #[must_use]
    pub fn contains(&self, key: OfferKey) -> bool {
        self.offers.contains_key(&key)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.offers.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.offers.is_empty()
    }
}

/// Abstract staging/commit boundary for receiving implementations.
///
/// Implementors should write chunks into a private staging area, verify each
/// file's expected digest, and only expose data after `commit` succeeds.  The
/// trait is synchronous on purpose: a runtime may wrap it in `spawn_blocking`
/// or its equivalent without coupling this crate to an async executor.
pub trait AtomicReceiver {
    type Staging;
    type Error: std::error::Error + Send + Sync + 'static;

    fn begin(
        &self,
        transfer_id: TransferId,
        manifest: &TransferManifest,
    ) -> Result<Self::Staging, Self::Error>;
    fn write_chunk(
        &self,
        staging: &mut Self::Staging,
        path: &RelativePath,
        descriptor: ChunkDescriptor,
        bytes: &[u8],
    ) -> Result<(), Self::Error>;
    fn complete_file(
        &self,
        staging: &mut Self::Staging,
        entry: &TransferEntry,
    ) -> Result<(), Self::Error>;
    fn commit(
        self,
        staging: Self::Staging,
        strategy: ConflictStrategy,
    ) -> Result<CommitReport, Self::Error>;
    fn abort(&self, staging: Self::Staging) -> Result<(), Self::Error>;
}

/// Summary returned after committing a staged transfer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommitReport {
    pub written: Vec<PathBuf>,
    pub skipped: Vec<PathBuf>,
}

#[cfg(unix)]
mod secure_fs {
    use super::{RelativePath, TransferEntry, TransferError, hash_reader};
    use rustix::fd::OwnedFd;
    use rustix::fs::{AtFlags, FileType, Mode, OFlags, open, openat, renameat, statat, unlinkat};
    use std::fs::File;
    use std::io::{self, ErrorKind};
    use std::path::Path;
    use std::sync::Arc;

    pub(crate) type Dir = Arc<OwnedFd>;

    const DIRECTORY_FLAGS: OFlags = OFlags::RDONLY
        .union(OFlags::DIRECTORY)
        .union(OFlags::CLOEXEC)
        .union(OFlags::NOFOLLOW);
    const FILE_READ_FLAGS: OFlags = OFlags::RDONLY
        .union(OFlags::CLOEXEC)
        .union(OFlags::NOFOLLOW);
    const FILE_WRITE_FLAGS: OFlags = OFlags::RDWR
        .union(OFlags::CREATE)
        .union(OFlags::CLOEXEC)
        .union(OFlags::NOFOLLOW);
    const EMPTY_FILE_FLAGS: OFlags = OFlags::WRONLY
        .union(OFlags::CREATE)
        .union(OFlags::TRUNC)
        .union(OFlags::CLOEXEC)
        .union(OFlags::NOFOLLOW);

    fn io_result<T>(result: rustix::io::Result<T>) -> io::Result<T> {
        result.map_err(io::Error::from)
    }

    fn is_missing(error: &io::Error) -> bool {
        error.kind() == ErrorKind::NotFound
    }

    fn is_exists(error: &io::Error) -> bool {
        error.kind() == ErrorKind::AlreadyExists
    }

    pub(crate) fn open_root(path: &Path) -> io::Result<Dir> {
        // macOS exposes /var as a compatibility symlink to /private/var.  The
        // destination is caller-selected (not manifest-controlled), so resolve
        // only that outer root once, then perform the security-sensitive walk
        // with O_NOFOLLOW at every resolved component.
        let resolved = if path.parent().is_none() {
            std::fs::canonicalize(path)?
        } else {
            let parent = path.parent().unwrap_or_else(|| Path::new("."));
            let parent = std::fs::canonicalize(parent)?;
            let leaf = path.file_name().ok_or_else(|| {
                io::Error::new(
                    ErrorKind::InvalidInput,
                    "destination has no final component",
                )
            })?;
            parent.join(leaf)
        };
        let mut current = if resolved.is_absolute() {
            Arc::new(io_result(open("/", DIRECTORY_FLAGS, Mode::empty()))?)
        } else {
            Arc::new(io_result(open(".", DIRECTORY_FLAGS, Mode::empty()))?)
        };
        for component in resolved.components() {
            let name = match component {
                std::path::Component::RootDir | std::path::Component::CurDir => continue,
                std::path::Component::Normal(name) => name.to_str().ok_or_else(|| {
                    io::Error::new(ErrorKind::InvalidInput, "destination is not UTF-8")
                })?,
                std::path::Component::ParentDir | std::path::Component::Prefix(_) => {
                    return Err(io::Error::new(
                        ErrorKind::InvalidInput,
                        "destination must not contain parent or prefix components",
                    ));
                }
            };
            match open_dir_at(&current, name) {
                Ok(next) => current = next,
                Err(error) if is_missing(&error) => {
                    match io_result(rustix::fs::mkdirat(
                        current.as_ref(),
                        name,
                        Mode::from_raw_mode(0o700),
                    )) {
                        Ok(()) => {}
                        Err(error) if is_exists(&error) => {}
                        Err(error) => return Err(error),
                    }
                    current = open_dir_at(&current, name)?;
                }
                Err(error) => return Err(error),
            }
        }
        Ok(current)
    }

    fn open_dir_at(parent: &Dir, name: &str) -> io::Result<Dir> {
        Ok(Arc::new(io_result(openat(
            parent.as_ref(),
            name,
            DIRECTORY_FLAGS,
            Mode::empty(),
        ))?))
    }

    fn open_dir_path(root: &Dir, path: &RelativePath) -> io::Result<Dir> {
        let mut current = root.clone();
        for component in path.as_str().split('/') {
            current = open_dir_at(&current, component)?;
        }
        Ok(current)
    }

    pub(crate) fn open_parent(root: &Dir, path: &RelativePath) -> io::Result<(Dir, String)> {
        let Some((parent, leaf)) = path.as_str().rsplit_once('/') else {
            return Ok((root.clone(), path.as_str().to_owned()));
        };
        let parent = RelativePath::new(parent).map_err(|error| match error {
            TransferError::InvalidPath(_) => io::Error::new(ErrorKind::InvalidInput, error),
            other => io::Error::other(other),
        })?;
        Ok((open_dir_path(root, &parent)?, leaf.to_owned()))
    }

    pub(crate) fn create_staging(root: &Dir, name: &str) -> io::Result<Dir> {
        match io_result(rustix::fs::mkdirat(
            root.as_ref(),
            name,
            Mode::from_raw_mode(0o700),
        )) {
            Ok(()) => {}
            Err(error) if is_exists(&error) => {}
            Err(error) => return Err(error),
        }
        open_dir_at(root, name)
    }

    /// Ensure a directory path exists, returning the paths created by this call.
    /// If a later component fails, undo directories created by this call before
    /// returning so callers never lose track of partial work.
    pub(crate) fn ensure_dir(root: &Dir, path: &RelativePath) -> io::Result<Vec<String>> {
        let mut current = root.clone();
        let mut prefix = String::new();
        let mut created = Vec::new();
        for component in path.as_str().split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(component);
            match open_dir_at(&current, component) {
                Ok(next) => current = next,
                Err(error) if is_missing(&error) => {
                    if let Err(error) = io_result(rustix::fs::mkdirat(
                        current.as_ref(),
                        component,
                        Mode::from_raw_mode(0o700),
                    )) && !is_exists(&error)
                    {
                        remove_created(root, &created)?;
                        return Err(error);
                    }
                    match open_dir_at(&current, component) {
                        Ok(next) => {
                            current = next;
                            created.push(prefix.clone());
                        }
                        Err(error) => {
                            remove_created(root, &created)?;
                            return Err(error);
                        }
                    }
                }
                Err(error) => {
                    remove_created(root, &created)?;
                    return Err(error);
                }
            }
        }
        Ok(created)
    }

    fn remove_created(root: &Dir, created: &[String]) -> io::Result<()> {
        for path in created.iter().rev() {
            let relative = RelativePath::new(path)
                .map_err(|error| io::Error::new(ErrorKind::InvalidInput, error))?;
            let Ok((parent, leaf)) = open_parent(root, &relative) else {
                continue;
            };
            match io_result(unlinkat(parent.as_ref(), leaf, AtFlags::REMOVEDIR)) {
                Ok(()) => {}
                Err(error) if is_missing(&error) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum Node {
        Missing,
        File,
        Directory,
        Symlink,
        Other,
    }

    pub(crate) fn node_at(root: &Dir, path: &RelativePath) -> io::Result<Node> {
        let (parent, leaf) = match open_parent(root, path) {
            Ok(value) => value,
            Err(error) if is_missing(&error) => return Ok(Node::Missing),
            Err(error) => return Err(error),
        };
        match io_result(statat(parent.as_ref(), leaf, AtFlags::SYMLINK_NOFOLLOW)) {
            Ok(stat) => {
                let file_type = FileType::from_raw_mode(stat.st_mode);
                Ok(if file_type.is_file() {
                    Node::File
                } else if file_type.is_dir() {
                    Node::Directory
                } else if file_type.is_symlink() {
                    Node::Symlink
                } else {
                    Node::Other
                })
            }
            Err(error) if is_missing(&error) => Ok(Node::Missing),
            Err(error) => Err(error),
        }
    }

    pub(crate) fn create_empty_file(root: &Dir, path: &RelativePath) -> io::Result<()> {
        let (parent, leaf) = open_parent_for_create(root, path)?;
        let _file = io_result(openat(
            parent.as_ref(),
            leaf,
            EMPTY_FILE_FLAGS,
            Mode::from_raw_mode(0o600),
        ))?;
        Ok(())
    }

    fn open_parent_for_create(root: &Dir, path: &RelativePath) -> io::Result<(Dir, String)> {
        let Some((parent, _leaf)) = path.as_str().rsplit_once('/') else {
            return Ok((root.clone(), path.as_str().to_owned()));
        };
        let parent = RelativePath::new(parent)
            .map_err(|error| io::Error::new(ErrorKind::InvalidInput, error))?;
        ensure_dir(root, &parent)?;
        open_parent(root, path)
    }

    pub(crate) fn create_symlink(
        root: &Dir,
        path: &RelativePath,
        target: &str,
    ) -> Result<(), TransferError> {
        let (parent, leaf) = open_parent_for_create(root, path).map_err(TransferError::Io)?;
        io_result(rustix::fs::symlinkat(target, parent.as_ref(), leaf)).map_err(TransferError::Io)
    }

    pub(crate) fn write_chunk(
        root: &Dir,
        path: &RelativePath,
        offset: u64,
        bytes: &[u8],
    ) -> io::Result<()> {
        let (parent, leaf) = open_parent_for_create(root, path)?;
        let file = io_result(openat(
            parent.as_ref(),
            leaf,
            FILE_WRITE_FLAGS,
            Mode::from_raw_mode(0o600),
        ))?;
        let mut written = 0;
        while written < bytes.len() {
            let count = io_result(rustix::io::pwrite(
                &file,
                &bytes[written..],
                offset.checked_add(written as u64).ok_or_else(|| {
                    io::Error::new(ErrorKind::InvalidInput, "chunk offset overflow")
                })?,
            ))?;
            if count == 0 {
                return Err(io::Error::new(ErrorKind::WriteZero, "short chunk write"));
            }
            written += count;
        }
        Ok(())
    }

    pub(crate) fn file_hash_and_size(
        root: &Dir,
        entry: &TransferEntry,
    ) -> Result<(), TransferError> {
        let (parent, leaf) = open_parent(root, entry.path()).map_err(TransferError::Io)?;
        let file = io_result(openat(
            parent.as_ref(),
            leaf,
            FILE_READ_FLAGS,
            Mode::empty(),
        ))
        .map_err(TransferError::Io)?;
        let stat = io_result(rustix::fs::fstat(&file)).map_err(TransferError::Io)?;
        let file_type = FileType::from_raw_mode(stat.st_mode);
        if !file_type.is_file() || stat.st_size < 0 || stat.st_size as u64 != entry.size() {
            return Err(TransferError::Staging(format!(
                "staged file has wrong size or type: {}",
                entry.path()
            )));
        }
        let file: File = file.into();
        let actual = hash_reader(&file).map_err(TransferError::Io)?;
        let expected = entry.hash().ok_or_else(|| {
            TransferError::InvalidManifest(format!("file {} has no hash", entry.path()))
        })?;
        if actual == expected {
            if let Some(mode) = entry.mode {
                io_result(rustix::fs::fchmod(
                    &file,
                    Mode::from_raw_mode((mode & 0o777) as _),
                ))
                .map_err(TransferError::Io)?;
            }
            file.sync_all().map_err(TransferError::Io)?;
            Ok(())
        } else {
            Err(TransferError::HashMismatch { expected, actual })
        }
    }

    pub(crate) fn rename(
        old_root: &Dir,
        old_path: &RelativePath,
        new_root: &Dir,
        new_path: &RelativePath,
    ) -> io::Result<()> {
        let (old_parent, old_leaf) = open_parent(old_root, old_path)?;
        let (new_parent, new_leaf) = open_parent(new_root, new_path)?;
        io_result(renameat(
            old_parent.as_ref(),
            old_leaf,
            new_parent.as_ref(),
            new_leaf,
        ))
    }

    pub(crate) fn remove_file(root: &Dir, path: &RelativePath) -> io::Result<()> {
        let (parent, leaf) = open_parent(root, path)?;
        io_result(unlinkat(parent.as_ref(), leaf, AtFlags::empty()))
    }

    pub(crate) fn remove_dir(root: &Dir, path: &RelativePath) -> io::Result<()> {
        let (parent, leaf) = open_parent(root, path)?;
        io_result(unlinkat(parent.as_ref(), leaf, AtFlags::REMOVEDIR))
    }

    fn remove_known(root: &Dir, path: &RelativePath, directory: bool) -> io::Result<()> {
        let result = if directory {
            remove_dir(root, path)
        } else {
            remove_file(root, path)
        };
        match result {
            Ok(()) => Ok(()),
            Err(error) if is_missing(&error) => Ok(()),
            Err(error) => Err(error),
        }
    }

    fn remove_symlink_if_present(root: &Dir, path: &RelativePath) -> io::Result<()> {
        let (parent, leaf) = match open_parent(root, path) {
            Ok(value) => value,
            Err(error) if is_missing(&error) => return Ok(()),
            Err(error) => return Err(error),
        };
        let state = match io_result(statat(parent.as_ref(), &leaf, AtFlags::SYMLINK_NOFOLLOW)) {
            Ok(stat) => FileType::from_raw_mode(stat.st_mode),
            Err(error) if is_missing(&error) => return Ok(()),
            Err(error) => return Err(error),
        };
        if state.is_symlink() {
            match io_result(unlinkat(parent.as_ref(), leaf, AtFlags::empty())) {
                Ok(()) => Ok(()),
                Err(error) if is_missing(&error) => Ok(()),
                Err(error) => Err(error),
            }
        } else {
            Ok(())
        }
    }

    pub(crate) fn remove_staging(
        destination: &Dir,
        root: &Dir,
        name: &str,
        entries: &[TransferEntry],
    ) -> Result<(), TransferError> {
        let mut files = Vec::new();
        let mut directories = std::collections::BTreeSet::new();
        for entry in entries {
            if entry.is_file() {
                files.push(entry.path().clone());
                let mut parent = entry.path().as_str().rsplit_once('/').map(|(p, _)| p);
                while let Some(path) = parent {
                    directories.insert(path.to_owned());
                    parent = path.rsplit_once('/').map(|(p, _)| p);
                }
            } else {
                directories.insert(entry.path().as_str().to_owned());
            }
        }
        files.sort_by(|left, right| {
            right
                .as_str()
                .split('/')
                .count()
                .cmp(&left.as_str().split('/').count())
                .then_with(|| right.cmp(left))
        });
        let mut directories = directories
            .into_iter()
            .filter_map(|path| RelativePath::new(path).ok())
            .collect::<Vec<_>>();
        directories.sort_by(|left, right| {
            right
                .as_str()
                .split('/')
                .count()
                .cmp(&left.as_str().split('/').count())
                .then_with(|| right.cmp(left))
        });
        // Remove attacker-inserted symlink components before trying to remove
        // files below them.  This is anchored to the root fd and unlinks the
        // symlink itself; it never follows the link.
        for path in &directories {
            remove_symlink_if_present(root, path).map_err(TransferError::Io)?;
        }
        for path in files {
            remove_known(root, &path, false).map_err(TransferError::Io)?;
        }
        for path in directories {
            remove_known(root, &path, true).map_err(TransferError::Io)?;
        }
        match io_result(unlinkat(destination.as_ref(), name, AtFlags::REMOVEDIR)) {
            Ok(()) => Ok(()),
            Err(error) if is_missing(&error) => Ok(()),
            Err(error) => Err(TransferError::Io(error)),
        }
    }
}

/// Filesystem receiver with a private staging directory and a directory-handle
/// anchored commit path.  Unix builds use `openat`/`renameat` with `O_NOFOLLOW`
/// for every manifest-controlled component; other platforms fail closed.
#[derive(Clone, Debug)]
pub struct FileReceiver {
    destination: PathBuf,
    #[cfg(unix)]
    destination_fd: secure_fs::Dir,
}

impl FileReceiver {
    pub fn new(destination: impl Into<PathBuf>) -> Result<Self, TransferError> {
        let destination = destination.into();
        #[cfg(not(unix))]
        {
            let _ = destination;
            return Err(TransferError::Staging(
                "secure filesystem receiver is unsupported on this platform".into(),
            ));
        }
        #[cfg(unix)]
        {
            // Validate the final root entry without following it.  Ancestor
            // compatibility links (such as macOS /var) are resolved once by
            // open_root, while the resulting descriptor walk remains no-follow.
            match path_state(&destination)? {
                PathState::Absent => {}
                PathState::Present => {
                    let metadata = fs::symlink_metadata(&destination)?;
                    if metadata.file_type().is_symlink() || !metadata.is_dir() {
                        return Err(TransferError::Staging(format!(
                            "destination is not a real directory: {}",
                            destination.display()
                        )));
                    }
                }
            }
            let destination_fd = secure_fs::open_root(&destination).map_err(|error| {
                TransferError::Staging(format!(
                    "destination is not a real directory: {} ({error})",
                    destination.display()
                ))
            })?;
            Ok(Self {
                destination,
                destination_fd,
            })
        }
    }

    #[must_use]
    pub fn destination(&self) -> &Path {
        &self.destination
    }

    /// Starts or resumes the private staging area for a transfer.
    pub fn begin(
        &self,
        transfer_id: TransferId,
        manifest: &TransferManifest,
    ) -> Result<FileStaging, TransferError> {
        self.begin_transfer(transfer_id, manifest)
    }

    pub fn begin_transfer(
        &self,
        transfer_id: TransferId,
        manifest: &TransferManifest,
    ) -> Result<FileStaging, TransferError> {
        <Self as AtomicReceiver>::begin(self, transfer_id, manifest)
    }

    /// Commits a staging area using a journal which restores every destination
    /// mutation if any later operation fails.
    pub fn commit_staging(
        self,
        staging: FileStaging,
        strategy: ConflictStrategy,
    ) -> Result<CommitReport, TransferError> {
        <Self as AtomicReceiver>::commit(self, staging, strategy)
    }

    /// Aborts a staging area and removes its private files.
    pub fn abort_staging(&self, staging: FileStaging) -> Result<(), TransferError> {
        <Self as AtomicReceiver>::abort(self, staging)
    }
}

#[derive(Debug)]
pub struct FileStaging {
    root: PathBuf,
    manifest: TransferManifest,
    transfer_id: TransferId,
    #[cfg(unix)]
    root_fd: secure_fs::Dir,
    #[cfg(unix)]
    destination_fd: secure_fs::Dir,
    #[cfg(unix)]
    staging_name: String,
}

impl FileStaging {
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn manifest(&self) -> &TransferManifest {
        &self.manifest
    }

    pub fn write_chunk(
        &mut self,
        path: &RelativePath,
        descriptor: ChunkDescriptor,
        bytes: &[u8],
    ) -> Result<(), TransferError> {
        if bytes.len() != descriptor.length as usize {
            return Err(TransferError::InvalidChunk(format!(
                "chunk {} has {} bytes, expected {}",
                descriptor.index,
                bytes.len(),
                descriptor.length
            )));
        }
        descriptor.validate()?;
        let entry = self
            .manifest
            .entries()
            .binary_search_by(|entry| entry.path().cmp(path))
            .ok()
            .map(|index| &self.manifest.entries()[index])
            .ok_or_else(|| {
                TransferError::InvalidChunk(format!("path {path} is not in the manifest"))
            })?;
        let end = descriptor
            .offset
            .checked_add(descriptor.length as u64)
            .ok_or_else(|| TransferError::InvalidChunk("chunk range overflows u64".into()))?;
        if !entry.is_file() || end > entry.size() {
            return Err(TransferError::InvalidChunk(format!(
                "chunk {} is outside file {path}",
                descriptor.index
            )));
        }
        if let Some(expected) = descriptor.hash {
            verify_sha256(bytes, expected)?;
        }
        #[cfg(not(unix))]
        {
            let _ = (path, descriptor, bytes);
            return Err(TransferError::Staging(
                "secure filesystem receiver is unsupported on this platform".into(),
            ));
        }
        #[cfg(unix)]
        secure_fs::write_chunk(&self.root_fd, path, descriptor.offset, bytes)
            .map_err(TransferError::Io)
    }

    pub fn complete_file(&self, entry: &TransferEntry) -> Result<(), TransferError> {
        let expected = entry.hash().ok_or_else(|| {
            TransferError::InvalidManifest(format!(
                "directory {} cannot be completed as a file",
                entry.path()
            ))
        })?;
        #[cfg(not(unix))]
        {
            let _ = expected;
            return Err(TransferError::Staging(
                "secure filesystem receiver is unsupported on this platform".into(),
            ));
        }
        #[cfg(unix)]
        {
            secure_fs::file_hash_and_size(&self.root_fd, entry)?;
            let _ = expected;
            Ok(())
        }
    }

    fn cleanup(self) -> Result<(), TransferError> {
        #[cfg(not(unix))]
        {
            return Err(TransferError::Staging(
                "secure filesystem receiver is unsupported on this platform".into(),
            ));
        }
        #[cfg(unix)]
        secure_fs::remove_staging(
            &self.destination_fd,
            &self.root_fd,
            &self.staging_name,
            self.manifest.entries(),
        )
    }
}

impl AtomicReceiver for FileReceiver {
    type Staging = FileStaging;
    type Error = TransferError;

    fn begin(
        &self,
        transfer_id: TransferId,
        manifest: &TransferManifest,
    ) -> Result<Self::Staging, Self::Error> {
        if manifest
            .entries()
            .iter()
            .any(|entry| entry.path().as_str().is_empty())
        {
            return Err(TransferError::InvalidManifest("empty path".into()));
        }
        #[cfg(not(unix))]
        {
            let _ = (transfer_id, manifest);
            return Err(TransferError::Staging(
                "secure filesystem receiver is unsupported on this platform".into(),
            ));
        }
        #[cfg(unix)]
        {
            let staging_name = format!(".deskfuse-staging-{transfer_id}");
            let root_fd = secure_fs::create_staging(&self.destination_fd, &staging_name)?;
            let root = self.destination.join(&staging_name);
            let staging = FileStaging {
                root,
                manifest: manifest.clone(),
                transfer_id,
                root_fd,
                destination_fd: self.destination_fd.clone(),
                staging_name,
            };
            let empty_paths = staging
                .manifest
                .files()
                .filter(|entry| entry.size() == 0)
                .map(|entry| entry.path().clone())
                .collect::<Vec<_>>();
            for path in empty_paths {
                if let Err(error) = secure_fs::create_empty_file(&staging.root_fd, &path) {
                    let _ = staging.cleanup();
                    return Err(TransferError::Io(error));
                }
            }
            Ok(staging)
        }
    }

    fn write_chunk(
        &self,
        staging: &mut Self::Staging,
        path: &RelativePath,
        descriptor: ChunkDescriptor,
        bytes: &[u8],
    ) -> Result<(), Self::Error> {
        staging.write_chunk(path, descriptor, bytes)
    }

    fn complete_file(
        &self,
        staging: &mut Self::Staging,
        entry: &TransferEntry,
    ) -> Result<(), Self::Error> {
        staging.complete_file(entry)
    }

    fn commit(
        self,
        staging: Self::Staging,
        strategy: ConflictStrategy,
    ) -> Result<CommitReport, Self::Error> {
        #[cfg(not(unix))]
        {
            let _ = (staging, strategy);
            return Err(TransferError::Staging(
                "secure filesystem receiver is unsupported on this platform".into(),
            ));
        }
        #[cfg(unix)]
        commit_secure(self.destination_fd, self.destination, staging, strategy)
    }

    fn abort(&self, staging: Self::Staging) -> Result<(), Self::Error> {
        staging.cleanup()
    }
}

#[cfg(unix)]
#[derive(Debug)]
enum JournalEntry {
    CreatedFile(RelativePath),
    ReplacedFile {
        target: RelativePath,
        backup: RelativePath,
    },
    CreatedDirectory(RelativePath),
    ReplacedDirectory {
        target: RelativePath,
        backup: RelativePath,
    },
}

#[cfg(unix)]
fn destination_path(destination: &Path, path: &RelativePath) -> PathBuf {
    destination.join(path.as_ref())
}

#[cfg(unix)]
fn is_skipped_path(path: &RelativePath, skipped: &BTreeSet<String>) -> bool {
    skipped.iter().any(|prefix| {
        path.as_str() == prefix
            || path
                .as_str()
                .strip_prefix(prefix)
                .is_some_and(|suffix| suffix.starts_with('/'))
    })
}

#[cfg(unix)]
fn remap_renamed_path(
    path: &RelativePath,
    renamed_prefixes: &BTreeMap<String, RelativePath>,
) -> Result<RelativePath, TransferError> {
    let Some((source, target)) = renamed_prefixes
        .iter()
        .filter(|(source, _)| {
            path.as_str() == source.as_str()
                || path
                    .as_str()
                    .strip_prefix(source.as_str())
                    .is_some_and(|suffix| suffix.starts_with('/'))
        })
        .max_by_key(|(source, _)| source.len())
    else {
        return Ok(path.clone());
    };
    let suffix = &path.as_str()[source.len()..];
    RelativePath::new(format!("{}{}", target.as_str(), suffix))
}

#[cfg(unix)]
fn secure_conflict(
    destination: &secure_fs::Dir,
    path: &RelativePath,
    strategy: ConflictStrategy,
    reserved: &BTreeSet<String>,
) -> Result<(RelativePath, bool), TransferError> {
    let node = secure_fs::node_at(destination, path).map_err(TransferError::Io)?;
    if node == secure_fs::Node::Missing {
        return Ok((path.clone(), false));
    }
    match strategy {
        ConflictStrategy::Overwrite => Ok((path.clone(), false)),
        ConflictStrategy::Skip => Ok((path.clone(), true)),
        ConflictStrategy::Rename => {
            let (parent, file_name) = path
                .as_str()
                .rsplit_once('/')
                .map_or(("", path.as_str()), |(parent, name)| (parent, name));
            let (stem, extension) = match file_name.rsplit_once('.') {
                Some((stem, extension)) if !stem.is_empty() && !extension.is_empty() => {
                    (stem, format!(".{extension}"))
                }
                _ => (file_name, String::new()),
            };
            for number in 1_u64.. {
                let candidate_name = format!("{stem} ({number}){extension}");
                let candidate_string = if parent.is_empty() {
                    candidate_name
                } else {
                    format!("{parent}/{candidate_name}")
                };
                let candidate = RelativePath::new(candidate_string)?;
                if !reserved.contains(candidate.as_str())
                    && secure_fs::node_at(destination, &candidate).map_err(TransferError::Io)?
                        == secure_fs::Node::Missing
                {
                    return Ok((candidate, false));
                }
            }
            unreachable!("u64 range is finite but practically unexhaustible")
        }
    }
}

#[cfg(unix)]
fn backup_name(
    destination: &secure_fs::Dir,
    target: &RelativePath,
    transfer_id: TransferId,
    reserved: &BTreeSet<String>,
) -> Result<RelativePath, TransferError> {
    let (parent, _) = target
        .as_str()
        .rsplit_once('/')
        .map_or(("", target.as_str()), |(parent, leaf)| (parent, leaf));
    for number in 0_u64.. {
        let name = format!(".deskfuse-backup-{transfer_id}-{number}");
        let candidate_string = if parent.is_empty() {
            name
        } else {
            format!("{parent}/{name}")
        };
        let candidate = RelativePath::new(candidate_string)?;
        if !reserved.contains(candidate.as_str())
            && secure_fs::node_at(destination, &candidate).map_err(TransferError::Io)?
                == secure_fs::Node::Missing
        {
            return Ok(candidate);
        }
    }
    unreachable!("u64 range is finite but practically unexhaustible")
}

#[cfg(unix)]
fn add_created_directories(
    destination: &secure_fs::Dir,
    base: &Path,
    created: Vec<String>,
    journal: &mut Vec<JournalEntry>,
    report: &mut CommitReport,
) -> Result<(), TransferError> {
    for path in created {
        let relative = RelativePath::new(path)?;
        journal.push(JournalEntry::CreatedDirectory(relative.clone()));
        report.written.push(destination_path(base, &relative));
    }
    let _ = destination;
    Ok(())
}

#[cfg(unix)]
fn rollback_journal(
    destination: &secure_fs::Dir,
    journal: &[JournalEntry],
) -> Result<(), TransferError> {
    for entry in journal.iter().rev() {
        match entry {
            JournalEntry::CreatedFile(path) => match secure_fs::remove_file(destination, path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(TransferError::Io(error)),
            },
            JournalEntry::ReplacedFile { target, backup } => {
                match secure_fs::remove_file(destination, target) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(TransferError::Io(error)),
                }
                secure_fs::rename(destination, backup, destination, target)
                    .map_err(TransferError::Io)?;
            }
            JournalEntry::CreatedDirectory(path) => {
                match secure_fs::remove_dir(destination, path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(TransferError::Io(error)),
                }
            }
            JournalEntry::ReplacedDirectory { target, backup } => {
                match secure_fs::remove_dir(destination, target) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(TransferError::Io(error)),
                }
                secure_fs::rename(destination, backup, destination, target)
                    .map_err(TransferError::Io)?;
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
fn commit_secure(
    destination: secure_fs::Dir,
    destination_path_buf: PathBuf,
    staging: FileStaging,
    strategy: ConflictStrategy,
) -> Result<CommitReport, TransferError> {
    for entry in staging.manifest.entries() {
        RelativePath::new(entry.path())?;
    }
    // Verify every staged file while it is still anchored to the staging fd.
    // This keeps later publication operations free of content-validation exits.
    for entry in staging.manifest.files() {
        staging.complete_file(entry)?;
    }

    for entry in staging
        .manifest
        .entries()
        .iter()
        .filter(|entry| entry.link_target.is_some())
    {
        secure_fs::create_symlink(
            &staging.root_fd,
            entry.path(),
            entry.link_target.as_deref().expect("validated link"),
        )?;
    }
    let mut report = CommitReport::default();
    let mut journal = Vec::new();
    let mut reserved = BTreeSet::new();
    let mut skipped_directories = BTreeSet::new();
    let mut renamed_prefixes = BTreeMap::new();
    let result = (|| {
        for entry in staging.manifest.directories() {
            let original_path = RelativePath::new(entry.path())?;
            if is_skipped_path(&original_path, &skipped_directories) {
                continue;
            }
            let path = remap_renamed_path(&original_path, &renamed_prefixes)?;
            let node = secure_fs::node_at(&destination, &path).map_err(TransferError::Io)?;
            let target = if matches!(node, secure_fs::Node::Missing | secure_fs::Node::Directory) {
                path.clone()
            } else {
                let (target, skip) = secure_conflict(&destination, &path, strategy, &reserved)?;
                if skip {
                    skipped_directories.insert(original_path.as_str().to_owned());
                    report
                        .skipped
                        .push(destination_path(&destination_path_buf, &path));
                    continue;
                }
                if strategy == ConflictStrategy::Overwrite && target == path {
                    let backup =
                        backup_name(&destination, &target, staging.transfer_id, &reserved)?;
                    secure_fs::rename(&destination, &target, &destination, &backup)
                        .map_err(TransferError::Io)?;
                    journal.push(JournalEntry::ReplacedDirectory {
                        target: target.clone(),
                        backup,
                    });
                }
                target
            };
            if target != original_path {
                renamed_prefixes.insert(original_path.as_str().to_owned(), target.clone());
            }
            reserved.insert(target.as_str().to_owned());
            let created =
                secure_fs::ensure_dir(&destination, &target).map_err(TransferError::Io)?;
            add_created_directories(
                &destination,
                &destination_path_buf,
                created,
                &mut journal,
                &mut report,
            )?;
        }

        for entry in staging
            .manifest
            .entries()
            .iter()
            .filter(|entry| !entry.is_directory())
        {
            let original_path = RelativePath::new(entry.path())?;
            if is_skipped_path(&original_path, &skipped_directories) {
                report
                    .skipped
                    .push(destination_path(&destination_path_buf, &original_path));
                continue;
            }
            let path = remap_renamed_path(&original_path, &renamed_prefixes)?;
            let (target, skip) = secure_conflict(&destination, &path, strategy, &reserved)?;
            if skip {
                report
                    .skipped
                    .push(destination_path(&destination_path_buf, &target));
                continue;
            }
            let parent = target
                .as_str()
                .rsplit_once('/')
                .map(|(parent, _)| parent.to_owned());
            if let Some(parent) = parent {
                let parent = RelativePath::new(parent)?;
                let created =
                    secure_fs::ensure_dir(&destination, &parent).map_err(TransferError::Io)?;
                add_created_directories(
                    &destination,
                    &destination_path_buf,
                    created,
                    &mut journal,
                    &mut report,
                )?;
            }
            let target_node =
                secure_fs::node_at(&destination, &target).map_err(TransferError::Io)?;
            if target_node == secure_fs::Node::Directory {
                return Err(TransferError::Conflict(destination_path(
                    &destination_path_buf,
                    &target,
                )));
            }
            let backup = if strategy == ConflictStrategy::Overwrite
                && target_node != secure_fs::Node::Missing
            {
                let backup = backup_name(&destination, &target, staging.transfer_id, &reserved)?;
                secure_fs::rename(&destination, &target, &destination, &backup)
                    .map_err(TransferError::Io)?;
                // Record the backup before publishing the new file.  A failed
                // source rename must still restore the old destination entry.
                journal.push(JournalEntry::ReplacedFile {
                    target: target.clone(),
                    backup: backup.clone(),
                });
                Some(backup)
            } else {
                None
            };
            secure_fs::rename(&staging.root_fd, &original_path, &destination, &target)
                .map_err(TransferError::Io)?;
            if backup.is_none() {
                journal.push(JournalEntry::CreatedFile(target.clone()));
            }
            reserved.insert(target.as_str().to_owned());
            report
                .written
                .push(destination_path(&destination_path_buf, &target));
        }

        staging.cleanup()?;
        for entry in &journal {
            match entry {
                JournalEntry::ReplacedFile { backup, .. }
                | JournalEntry::ReplacedDirectory { backup, .. } => {
                    match secure_fs::remove_file(&destination, backup) {
                        Ok(()) => {}
                        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                        Err(error) => return Err(TransferError::Io(error)),
                    }
                }
                JournalEntry::CreatedFile(_) | JournalEntry::CreatedDirectory(_) => {}
            }
        }
        Ok(report)
    })();

    match result {
        Ok(report) => Ok(report),
        Err(error) => match rollback_journal(&destination, &journal) {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(TransferError::Staging(format!(
                "commit failed: {error}; rollback failed: {rollback_error}"
            ))),
        },
    }
}

/// Transport-facing engine abstraction.  A future runtime can implement this
/// trait and map the methods to TCP/QUIC/Bluetooth/etc.; no transport details
/// leak into the manifest or filesystem code.
pub trait TransferEngine {
    type Error: std::error::Error + Send + Sync + 'static;

    fn offer(&mut self, offer: TransferOffer) -> Result<OfferOutcome, Self::Error>;
    fn state(&self, transfer_id: TransferId) -> Option<TransferState>;
    fn progress(&self, transfer_id: TransferId) -> Option<TransferProgress>;
    fn acknowledge(&mut self, transfer_id: TransferId, chunk_index: u64)
    -> Result<(), Self::Error>;
    fn cancel(&mut self, transfer_id: TransferId) -> Result<(), Self::Error>;
}

/// Small in-memory reference engine useful for tests and runtime integration.
#[derive(Debug, Default)]
pub struct InMemoryTransferEngine {
    offers: OfferRegistry,
    records: HashMap<TransferId, (TransferState, TransferProgress)>,
}

impl InMemoryTransferEngine {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn offer_registry(&self) -> &OfferRegistry {
        &self.offers
    }
}

impl TransferEngine for InMemoryTransferEngine {
    type Error = TransferError;

    fn offer(&mut self, offer: TransferOffer) -> Result<OfferOutcome, Self::Error> {
        let outcome = self.offers.accept(&offer)?;
        self.records.entry(offer.transfer_id).or_insert_with(|| {
            let total_chunks = offer
                .manifest
                .files()
                .map(|entry| entry.size().div_ceil(DEFAULT_CHUNK_SIZE_BYTES))
                .sum();
            (
                TransferState::Accepted,
                TransferProgress {
                    state: TransferState::Accepted,
                    total_bytes: offer.manifest.total_bytes(),
                    transferred_bytes: 0,
                    total_chunks,
                    completed_chunks: 0,
                },
            )
        });
        Ok(outcome)
    }

    fn state(&self, transfer_id: TransferId) -> Option<TransferState> {
        self.records
            .get(&transfer_id)
            .map(|record| record.0.clone())
    }

    fn progress(&self, transfer_id: TransferId) -> Option<TransferProgress> {
        self.records
            .get(&transfer_id)
            .map(|record| record.1.clone())
    }

    fn acknowledge(
        &mut self,
        transfer_id: TransferId,
        chunk_index: u64,
    ) -> Result<(), Self::Error> {
        let (state, progress) = self
            .records
            .get_mut(&transfer_id)
            .ok_or_else(|| TransferError::State(format!("unknown transfer {transfer_id}")))?;
        if state.is_terminal() {
            return Err(TransferError::State(
                "cannot ACK a terminal transfer".into(),
            ));
        }
        if chunk_index >= progress.total_chunks {
            return Err(TransferError::InvalidChunk(format!(
                "chunk index {chunk_index} out of range"
            )));
        }
        progress.completed_chunks = progress.completed_chunks.max(chunk_index + 1);
        *state = TransferState::Transferring;
        progress.state = state.clone();
        Ok(())
    }

    fn cancel(&mut self, transfer_id: TransferId) -> Result<(), Self::Error> {
        let (state, progress) = self
            .records
            .get_mut(&transfer_id)
            .ok_or_else(|| TransferError::State(format!("unknown transfer {transfer_id}")))?;
        if !state.is_terminal() {
            *state = TransferState::Cancelled;
            progress.state = TransferState::Cancelled;
        }
        Ok(())
    }
}

fn collect_manifest_entries(
    root: &Path,
    relative: &Path,
    entries: &mut Vec<TransferEntry>,
    cancelled: &impl Fn() -> bool,
    progress: &mut impl FnMut(usize),
) -> Result<(), TransferError> {
    let directory = if relative.as_os_str().is_empty() {
        root.to_path_buf()
    } else {
        secure_join(root, &RelativePath::new(relative)?)?
    };
    let mut children = fs::read_dir(&directory)?.collect::<Result<Vec<_>, io::Error>>()?;
    children.sort_by_key(|entry| entry.file_name());
    for child in children {
        if cancelled() {
            return Err(TransferError::State("文件准备已取消".into()));
        }
        let path = child.path();
        let metadata = fs::symlink_metadata(&path)?;
        let child_relative = if relative.as_os_str().is_empty() {
            PathBuf::from(child.file_name())
        } else {
            relative.join(child.file_name())
        };
        let safe = RelativePath::new(&child_relative)?;
        if entries.len() >= MAX_MANIFEST_ENTRIES {
            return Err(TransferError::InvalidManifest(
                "文件目录超过 100 万项，请分批复制".into(),
            ));
        }
        if metadata.file_type().is_symlink() {
            entries.push(TransferEntry::symlink(
                &safe,
                fs::read_link(&path)?,
                metadata_mtime(&metadata),
            )?);
        } else if metadata.is_dir() {
            entries.push(
                TransferEntry::directory(&safe, metadata_mtime(&metadata))?
                    .with_metadata(&metadata),
            );
            collect_manifest_entries(root, &child_relative, entries, cancelled, progress)?;
        } else if metadata.is_file() {
            entries.push(
                TransferEntry::file(
                    &safe,
                    metadata.len(),
                    metadata_mtime(&metadata),
                    hash_file_cancellable(&path, cancelled)?,
                )?
                .with_metadata(&metadata),
            );
        } else {
            return Err(TransferError::InvalidManifest(format!(
                "unsupported filesystem entry: {}",
                path.display()
            )));
        }
        progress(entries.len());
    }
    Ok(())
}

/// Validate lexically without ever following a local or remote link.
pub(crate) fn validate_link_target(path: &RelativePath, target: &str) -> Result<(), TransferError> {
    let mut depth = path.as_str().split('/').count().saturating_sub(1);
    if target.is_empty() || target.contains('\0') || target.contains('\\') {
        return Err(TransferError::InvalidPath("符号链接目标无效".into()));
    }
    for component in Path::new(target).components() {
        match component {
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::ParentDir if depth > 0 => depth -= 1,
            _ => {
                return Err(TransferError::InvalidPath(
                    "符号链接指向所选目录之外，请包含其所在项目目录后重新复制".into(),
                ));
            }
        }
    }
    Ok(())
}

fn secure_join(root: &Path, relative: &RelativePath) -> Result<PathBuf, TransferError> {
    let path = relative.to_path_buf();
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(TransferError::InvalidPath(relative.to_string()));
    }
    let mut current = root.to_path_buf();
    for component in path.components() {
        let Component::Normal(part) = component else {
            return Err(TransferError::InvalidPath(relative.to_string()));
        };
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(TransferError::SymlinkNotAllowed(current));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(TransferError::Io(error)),
        }
    }
    Ok(root.join(path))
}

fn hash_reader(mut reader: impl Read) -> io::Result<Sha256Digest> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 128 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = hasher.finalize();
    let mut bytes = [0_u8; 32];
    bytes.copy_from_slice(&digest);
    Ok(Sha256Digest(bytes))
}

fn hash_file_cancellable(
    path: impl AsRef<Path>,
    cancelled: &impl Fn() -> bool,
) -> Result<Sha256Digest, TransferError> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        if cancelled() {
            return Err(TransferError::State("文件准备已取消".into()));
        }
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(Sha256Digest(hasher.finalize().into()))
}

fn metadata_mtime(metadata: &fs::Metadata) -> i128 {
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |duration| {
            i128::try_from(duration.as_nanos()).unwrap_or(i128::MAX)
        })
}

fn hex_value(value: u8) -> u8 {
    match value {
        b'0'..=b'9' => value - b'0',
        b'a'..=b'f' => value - b'a' + 10,
        b'A'..=b'F' => value - b'A' + 10,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn manifest_contains_directories_and_files() {
        let root = tempdir().unwrap();
        fs::create_dir(root.path().join("nested")).unwrap();
        fs::write(root.path().join("nested/file.txt"), b"hello").unwrap();
        let manifest = TransferManifest::from_directory(root.path()).unwrap();
        assert_eq!(manifest.file_count(), 1);
        assert_eq!(manifest.directory_count(), 1);
        let file = manifest.files().next().unwrap();
        assert_eq!(file.path().as_str(), "nested/file.txt");
        assert_eq!(file.size(), 5);
        assert_eq!(file.hash(), Some(sha256_bytes(b"hello")));
    }

    #[test]
    fn path_attacks_are_rejected() {
        for path in [
            "../escape",
            "/absolute",
            "a/../../escape",
            r"a\..\escape",
            "C:\\escape",
        ] {
            assert!(RelativePath::new(path).is_err(), "{path}");
        }
    }

    #[test]
    fn resume_bitmap_round_trips() {
        let mut bitmap = ResumeBitmap::new(130);
        bitmap.mark_complete(0).unwrap();
        bitmap.mark_complete(65).unwrap();
        let encoded = bitmap.to_bytes();
        let decoded = ResumeBitmap::from_bytes(130, &encoded).unwrap();
        assert_eq!(bitmap, decoded);
        assert_eq!(decoded.missing_count(), 128);
    }

    #[test]
    fn duplicate_offers_are_idempotent() {
        let manifest = TransferManifest::new(vec![
            TransferEntry::file("a", 1, 0, sha256_bytes(b"a")).unwrap(),
        ])
        .unwrap();
        let offer = TransferOffer::new(TransferId::from_bytes([7; 16]), manifest);
        let mut registry = OfferRegistry::default();
        assert_eq!(
            registry.accept(&offer).unwrap(),
            OfferOutcome::Accepted(offer.transfer_id)
        );
        assert_eq!(
            registry.accept(&offer).unwrap(),
            OfferOutcome::Duplicate(offer.transfer_id)
        );
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn hash_mismatch_is_reported() {
        let expected = sha256_bytes(b"good");
        let error = verify_sha256(b"bad", expected).unwrap_err();
        assert!(matches!(error, TransferError::HashMismatch { .. }));
    }

    #[test]
    fn rename_conflicts_preserve_extension() {
        let root = tempdir().unwrap();
        let target = root.path().join("file.txt");
        fs::write(&target, b"old").unwrap();
        let resolved = resolve_conflict(&target, ConflictStrategy::Rename).unwrap();
        assert_eq!(
            resolved,
            ConflictResolution::Write(root.path().join("file (1).txt"))
        );
    }
}
