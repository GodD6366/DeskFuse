//! Framing and source planning for seamless file clipboard transfers.

use super::{
    ChunkDescriptor, RelativePath, TransferEntry, TransferError, TransferId, TransferManifest,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// MIME marker carried inside the authenticated clipboard channel.
pub const CLIPBOARD_FILE_MIME: &str = "application/x-deskfuse-file-clipboard;version=1";
const MAGIC: &[u8; 8] = b"DFFILE1\0";
const OFFER: u8 = 1;
const CHUNK: u8 = 2;
const COMPLETE: u8 = 3;
const CANCEL: u8 = 4;
const ACK: u8 = 5;
const DEFERRED: u8 = 6;
const REQUEST: u8 = 7;
const REQUESTED_OFFER: u8 = 8;
const RECEIPT: u8 = 11;
const COMPRESSED_DEFERRED: u8 = 12;
const COMPRESSED_OFFER: u8 = 9;
const COMPRESSED_REQUESTED_OFFER: u8 = 10;
const MAX_MANIFEST_BYTES: usize = 256 * 1024 * 1024;
const MAX_OFFER_BYTES: usize = 16 * 1024 * 1024 - 1024;
const CHUNK_SIZE: usize = 1024 * 1024;

/// One local file and the chunk plan used to stream it without buffering the
/// complete clipboard transfer in memory.
#[derive(Clone, Debug)]
pub struct ClipboardSourceFile {
    source_path: PathBuf,
    relative_path: RelativePath,
    chunks: Vec<ChunkDescriptor>,
}

impl ClipboardSourceFile {
    #[must_use]
    pub fn source_path(&self) -> &Path {
        &self.source_path
    }

    #[must_use]
    pub fn relative_path(&self) -> &RelativePath {
        &self.relative_path
    }

    #[must_use]
    pub fn chunks(&self) -> &[ChunkDescriptor] {
        &self.chunks
    }
}

/// A validated manifest plus local source paths for one Finder copy action.
#[derive(Clone, Debug)]
pub struct ClipboardFilePlan {
    transfer_id: TransferId,
    manifest: TransferManifest,
    sources: Vec<ClipboardSourceFile>,
}

impl ClipboardFilePlan {
    /// Build one manifest while retaining every copied top-level file or
    /// directory. Internal relative links are preserved, never followed.
    pub fn from_paths(paths: &[PathBuf]) -> Result<Self, TransferError> {
        Self::from_paths_with_transfer_id(paths, TransferId::new())
    }

    /// Build a plan for a caller-provided ID so a background job can publish
    /// progress and accept cancellation before manifest scanning finishes.
    pub fn from_paths_with_transfer_id(
        paths: &[PathBuf],
        transfer_id: TransferId,
    ) -> Result<Self, TransferError> {
        Self::from_paths_observed(paths, transfer_id, || false, |_| {})
    }

    pub fn from_paths_observed(
        paths: &[PathBuf],
        transfer_id: TransferId,
        cancelled: impl Fn() -> bool,
        mut progress: impl FnMut(usize),
    ) -> Result<Self, TransferError> {
        if paths.is_empty() {
            return Err(TransferError::InvalidManifest(
                "file clipboard has no paths".into(),
            ));
        }
        let mut entries = Vec::new();
        let mut source_pairs = Vec::new();
        for source_root in paths {
            let metadata = fs::symlink_metadata(source_root)?;
            if metadata.file_type().is_symlink() {
                return Err(TransferError::SymlinkNotAllowed(source_root.clone()));
            }
            let root_name = source_root.file_name().ok_or_else(|| {
                TransferError::InvalidManifest(format!(
                    "clipboard path has no file name: {}",
                    source_root.display()
                ))
            })?;
            let root_relative = RelativePath::new(Path::new(root_name))?;
            let base = entries.len();
            let single =
                TransferManifest::from_path_observed(source_root, &cancelled, &mut |count| {
                    progress(base + count)
                })?;
            if metadata.is_file() {
                let entry = single.entries().first().ok_or_else(|| {
                    TransferError::InvalidManifest("file manifest is empty".into())
                })?;
                entries.push(entry.clone());
                source_pairs.push((entry.clone(), source_root.clone()));
                continue;
            }

            entries.push(TransferEntry::directory(&root_relative, 0)?);
            for entry in single.entries() {
                let copied = entry.prefixed(&root_relative)?;
                if entry.is_file() {
                    source_pairs.push((copied.clone(), source_root.join(entry.path().as_ref())));
                }
                entries.push(copied);
            }
        }
        let manifest = TransferManifest::new(entries)?;
        let sources = source_pairs
            .into_iter()
            .map(|(entry, source_path)| {
                Ok(ClipboardSourceFile {
                    source_path,
                    relative_path: entry.path().clone(),
                    chunks: entry.chunks(CHUNK_SIZE)?,
                })
            })
            .collect::<Result<Vec<_>, TransferError>>()?;
        Ok(Self {
            transfer_id,
            manifest,
            sources,
        })
    }

    #[must_use]
    pub const fn transfer_id(&self) -> TransferId {
        self.transfer_id
    }

    #[must_use]
    pub fn manifest(&self) -> &TransferManifest {
        &self.manifest
    }

    #[must_use]
    pub fn sources(&self) -> &[ClipboardSourceFile] {
        &self.sources
    }
}

/// One bounded frame in a file clipboard transfer.
#[derive(Clone, Debug)]
pub enum ClipboardFileFrame {
    /// A clipboard announcement; carries no content and creates no destination files.
    /// `entries` is optional on the wire for compatibility with the original
    /// names/bytes-only deferred frame. New senders include the complete tree;
    /// old frames decode to synthetic top-level file entries.
    Deferred {
        transfer_id: TransferId,
        names: Vec<String>,
        total_bytes: u64,
        entries: Vec<DeferredFileEntry>,
    },
    /// Explicit demand from the target after paste or permitted small-file prefetch.
    Request {
        transfer_id: TransferId,
    },
    /// Materialized response to a deferred request; valid until that transfer is cancelled.
    RequestedOffer {
        transfer_id: TransferId,
        manifest: TransferManifest,
    },
    Offer {
        transfer_id: TransferId,
        manifest: TransferManifest,
    },
    Chunk {
        transfer_id: TransferId,
        path: RelativePath,
        descriptor: ChunkDescriptor,
        bytes: Vec<u8>,
    },
    Complete {
        transfer_id: TransferId,
    },
    /// Cancel a transfer whose contents have been superseded or explicitly
    /// dismissed. The reason is metadata only and bounded at decode time.
    Cancel {
        transfer_id: TransferId,
        reason: String,
    },
    /// The receiver sends this only after every file hash has been verified,
    /// the staging directory has committed, and the received URLs have been
    /// installed on its pasteboard. It is therefore the sender's success
    /// boundary, rather than merely "last chunk was written".
    Ack {
        transfer_id: TransferId,
    },
    /// Cumulative queue acceptance; not an integrity/success acknowledgement.
    Receipt {
        transfer_id: TransferId,
        accepted_frames: u64,
    },
}

impl ClipboardFileFrame {
    #[must_use]
    pub const fn transfer_id(&self) -> TransferId {
        match self {
            Self::Deferred { transfer_id, .. }
            | Self::Request { transfer_id }
            | Self::RequestedOffer { transfer_id, .. }
            | Self::Offer { transfer_id, .. }
            | Self::Chunk { transfer_id, .. }
            | Self::Complete { transfer_id }
            | Self::Cancel { transfer_id, .. }
            | Self::Ack { transfer_id }
            | Self::Receipt { transfer_id, .. } => *transfer_id,
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>, TransferError> {
        let mut output = Vec::new();
        output.extend_from_slice(MAGIC);
        match self {
            Self::Deferred {
                transfer_id,
                names,
                total_bytes,
                entries,
            } => {
                validate_deferred_names(names)?;
                // Preserve the original tuple wire shape for callers that do
                // not have tree metadata; this lets old peers continue to
                // consume compatibility announcements.
                if entries.is_empty() {
                    output.push(DEFERRED);
                    output.extend_from_slice(transfer_id.as_bytes());
                    output.extend_from_slice(
                        &serde_json::to_vec(&(names, total_bytes))
                            .map_err(|e| TransferError::InvalidManifest(e.to_string()))?,
                    );
                    return Ok(output);
                }
                validate_deferred_entries(names, *total_bytes, entries)?;
                let wire = DeferredWire {
                    names,
                    total_bytes: *total_bytes,
                    entries,
                };
                let json = serde_json::to_vec(&wire)
                    .map_err(|e| TransferError::InvalidManifest(e.to_string()))?;
                let compressed = json.len() > 64 * 1024;
                let body = if compressed {
                    let mut encoder =
                        flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
                    encoder.write_all(&json)?;
                    encoder.finish()?
                } else {
                    json
                };
                if body.len() > MAX_OFFER_BYTES {
                    return Err(TransferError::InvalidManifest(
                        "延迟文件清单过大，请分批复制".into(),
                    ));
                }
                output.push(if compressed {
                    COMPRESSED_DEFERRED
                } else {
                    DEFERRED
                });
                output.extend_from_slice(transfer_id.as_bytes());
                output.extend_from_slice(&body);
            }
            Self::Request { transfer_id } => {
                output.push(REQUEST);
                output.extend_from_slice(transfer_id.as_bytes());
            }
            Self::RequestedOffer {
                transfer_id,
                manifest,
            }
            | Self::Offer {
                transfer_id,
                manifest,
            } => {
                let json = serde_json::to_vec(manifest)
                    .map_err(|error| TransferError::InvalidManifest(error.to_string()))?;
                if json.len() > MAX_MANIFEST_BYTES {
                    return Err(TransferError::InvalidManifest(
                        "文件清单超过 256 MiB，请分批复制".into(),
                    ));
                }
                let compressed = json.len() > 64 * 1024;
                let body = if compressed {
                    let mut encoder =
                        flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
                    encoder.write_all(&json)?;
                    encoder.finish()?
                } else {
                    json
                };
                if body.len() > MAX_OFFER_BYTES {
                    return Err(TransferError::InvalidManifest(
                        "压缩后的文件清单仍过大，请分批复制".into(),
                    ));
                }
                output.push(
                    match (matches!(self, Self::RequestedOffer { .. }), compressed) {
                        (true, true) => COMPRESSED_REQUESTED_OFFER,
                        (false, true) => COMPRESSED_OFFER,
                        (true, false) => REQUESTED_OFFER,
                        (false, false) => OFFER,
                    },
                );
                output.extend_from_slice(transfer_id.as_bytes());
                output.extend_from_slice(&body);
            }
            Self::Chunk {
                transfer_id,
                path,
                descriptor,
                bytes,
            } => {
                if bytes.len() != descriptor.length as usize {
                    return Err(TransferError::InvalidChunk(
                        "clipboard chunk length does not match descriptor".into(),
                    ));
                }
                let path = path.as_str().as_bytes();
                let path_len = u32::try_from(path.len()).map_err(|_| {
                    TransferError::InvalidChunk("clipboard chunk path is too long".into())
                })?;
                output.push(CHUNK);
                output.extend_from_slice(transfer_id.as_bytes());
                output.extend_from_slice(&path_len.to_be_bytes());
                output.extend_from_slice(path);
                output.extend_from_slice(&descriptor.index.to_be_bytes());
                output.extend_from_slice(&descriptor.offset.to_be_bytes());
                output.extend_from_slice(&descriptor.length.to_be_bytes());
                output.extend_from_slice(bytes);
            }
            Self::Complete { transfer_id } => {
                output.push(COMPLETE);
                output.extend_from_slice(transfer_id.as_bytes());
            }
            Self::Cancel {
                transfer_id,
                reason,
            } => {
                if reason.len() > 1024 {
                    return Err(TransferError::InvalidChunk(
                        "clipboard cancellation reason is too long".into(),
                    ));
                }
                output.push(CANCEL);
                output.extend_from_slice(transfer_id.as_bytes());
                output.extend_from_slice(reason.as_bytes());
            }
            Self::Receipt {
                transfer_id,
                accepted_frames,
            } => {
                output.push(RECEIPT);
                output.extend_from_slice(transfer_id.as_bytes());
                output.extend_from_slice(&accepted_frames.to_be_bytes());
            }
            Self::Ack { transfer_id } => {
                output.push(ACK);
                output.extend_from_slice(transfer_id.as_bytes());
            }
        }
        Ok(output)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, TransferError> {
        if bytes.len() < MAGIC.len() + 1 + 16 || &bytes[..MAGIC.len()] != MAGIC {
            return Err(TransferError::InvalidChunk(
                "invalid file clipboard frame".into(),
            ));
        }
        let kind = bytes[MAGIC.len()];
        let mut id = [0_u8; 16];
        id.copy_from_slice(&bytes[MAGIC.len() + 1..MAGIC.len() + 17]);
        let transfer_id = TransferId::from_bytes(id);
        let body = &bytes[MAGIC.len() + 17..];
        match kind {
            DEFERRED | COMPRESSED_DEFERRED if body.len() <= MAX_OFFER_BYTES => {
                let mut decoded = Vec::new();
                let json = if kind == COMPRESSED_DEFERRED {
                    flate2::read::ZlibDecoder::new(body)
                        .take(MAX_MANIFEST_BYTES as u64 + 1)
                        .read_to_end(&mut decoded)?;
                    if decoded.len() > MAX_MANIFEST_BYTES {
                        return Err(TransferError::InvalidManifest(
                            "expanded deferred file manifest is too large".into(),
                        ));
                    }
                    decoded.as_slice()
                } else {
                    body
                };
                let (names, total_bytes, entries) =
                    match serde_json::from_slice::<DeferredWireOwned>(json) {
                        Ok(wire) => (wire.names, wire.total_bytes, wire.entries),
                        // The original protocol encoded exactly `(names, total_bytes)`.
                        Err(_) => {
                            let (names, total_bytes): (Vec<String>, u64) =
                                serde_json::from_slice(json)
                                    .map_err(|e| TransferError::InvalidManifest(e.to_string()))?;
                            let entries = names
                                .iter()
                                .enumerate()
                                .map(|(lindex, name)| DeferredFileEntry {
                                    path: RelativePath::new(name).expect("validated deferred name"),
                                    kind: DeferredEntryKind::File,
                                    size: if lindex == 0 { total_bytes } else { 0 },
                                    lindex: lindex as u32,
                                })
                                .collect();
                            (names, total_bytes, entries)
                        }
                    };
                validate_deferred_names(&names)?;
                validate_deferred_entries(&names, total_bytes, &entries)?;
                Ok(Self::Deferred {
                    transfer_id,
                    names,
                    total_bytes,
                    entries,
                })
            }
            REQUEST if body.is_empty() => Ok(Self::Request { transfer_id }),
            OFFER | REQUESTED_OFFER | COMPRESSED_OFFER | COMPRESSED_REQUESTED_OFFER => {
                if body.len() > MAX_OFFER_BYTES {
                    return Err(TransferError::InvalidManifest(
                        "file offer is too large".into(),
                    ));
                }
                let mut decoded = Vec::new();
                let json = if matches!(kind, COMPRESSED_OFFER | COMPRESSED_REQUESTED_OFFER) {
                    flate2::read::ZlibDecoder::new(body)
                        .take(MAX_MANIFEST_BYTES as u64 + 1)
                        .read_to_end(&mut decoded)?;
                    if decoded.len() > MAX_MANIFEST_BYTES {
                        return Err(TransferError::InvalidManifest(
                            "expanded file manifest is too large".into(),
                        ));
                    }
                    decoded.as_slice()
                } else {
                    body
                };
                let manifest = serde_json::from_slice(json)
                    .map_err(|error| TransferError::InvalidManifest(error.to_string()))?;
                Ok(
                    if matches!(kind, REQUESTED_OFFER | COMPRESSED_REQUESTED_OFFER) {
                        Self::RequestedOffer {
                            transfer_id,
                            manifest,
                        }
                    } else {
                        Self::Offer {
                            transfer_id,
                            manifest,
                        }
                    },
                )
            }
            CHUNK => decode_chunk(transfer_id, body),
            COMPLETE if body.is_empty() => Ok(Self::Complete { transfer_id }),
            CANCEL if body.len() <= 1024 => {
                let reason = std::str::from_utf8(body).map_err(|_| {
                    TransferError::InvalidChunk("cancel reason is not UTF-8".into())
                })?;
                Ok(Self::Cancel {
                    transfer_id,
                    reason: reason.to_owned(),
                })
            }
            ACK if body.is_empty() => Ok(Self::Ack { transfer_id }),
            RECEIPT if body.len() == 8 => Ok(Self::Receipt {
                transfer_id,
                accepted_frames: u64::from_be_bytes(body.try_into().expect("checked receipt size")),
            }),
            _ => Err(TransferError::InvalidChunk(
                "unknown or malformed file clipboard frame".into(),
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeferredEntryKind {
    File,
    Directory,
    Symlink,
}

/// Metadata advertised before content is requested. Paths are relative to the
/// copied top-level roots and `lindex` is a deterministic, zero-based ordinal
/// used by Windows `FileContents`; it never changes after a request.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeferredFileEntry {
    pub path: RelativePath,
    pub kind: DeferredEntryKind,
    pub size: u64,
    pub lindex: u32,
}

#[derive(Serialize)]
struct DeferredWire<'a> {
    names: &'a [String],
    total_bytes: u64,
    entries: &'a [DeferredFileEntry],
}

#[derive(Deserialize)]
struct DeferredWireOwned {
    names: Vec<String>,
    total_bytes: u64,
    entries: Vec<DeferredFileEntry>,
}

fn validate_deferred_entries(
    names: &[String],
    total_bytes: u64,
    entries: &[DeferredFileEntry],
) -> Result<(), TransferError> {
    if entries.is_empty() {
        return Err(TransferError::InvalidManifest(
            "deferred clipboard manifest has no entries".into(),
        ));
    }
    if entries.len() > super::MAX_MANIFEST_ENTRIES {
        return Err(TransferError::InvalidManifest(
            "deferred clipboard manifest has too many entries".into(),
        ));
    }
    let mut paths = std::collections::BTreeSet::new();
    let mut bytes = 0_u64;
    for (index, entry) in entries.iter().enumerate() {
        if entry.lindex != index as u32 || !paths.insert(entry.path.as_str()) {
            return Err(TransferError::InvalidManifest(
                "deferred clipboard manifest has unstable indexes or duplicate paths".into(),
            ));
        }
        if entry.kind == DeferredEntryKind::File {
            bytes = bytes.checked_add(entry.size).ok_or_else(|| {
                TransferError::InvalidManifest("deferred clipboard size overflow".into())
            })?;
        } else if entry.size != 0 {
            return Err(TransferError::InvalidManifest(
                "deferred directory has non-zero size".into(),
            ));
        }
    }
    if bytes != total_bytes {
        return Err(TransferError::InvalidManifest(
            "deferred file bytes do not match manifest".into(),
        ));
    }
    let roots = entries
        .iter()
        .filter_map(|entry| entry.path.as_str().split('/').next())
        .collect::<std::collections::BTreeSet<_>>();
    if roots.len() != names.len() || names.iter().any(|name| !roots.contains(name.as_str())) {
        return Err(TransferError::InvalidManifest(
            "deferred manifest roots do not match names".into(),
        ));
    }
    Ok(())
}

fn validate_deferred_names(names: &[String]) -> Result<(), TransferError> {
    if names.is_empty()
        || names.len() > 1024
        || names.iter().any(|name| {
            name.is_empty()
                || name.len() > 1024
                || name.contains('/')
                || name == "."
                || name == ".."
                || name.chars().any(char::is_control)
        })
    {
        return Err(TransferError::InvalidManifest(
            "invalid deferred clipboard names".into(),
        ));
    }
    Ok(())
}

/// A metadata-only snapshot of local source trees. Building this never opens
/// file contents, hashes or packages them; content reads begin after demand.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeferredClipboardSource {
    pub roots: Vec<PathBuf>,
    pub names: Vec<String>,
    pub total_bytes: u64,
    entries: std::sync::Arc<Vec<SourceMetadata>>,
    deferred_entries: std::sync::Arc<Vec<DeferredFileEntry>>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
struct SourceMetadata {
    path: PathBuf,
    size: u64,
    directory: bool,
    link_target: Option<PathBuf>,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    modified: (i64, i64),
    #[cfg(windows)]
    stamp: deskfuse_platform_windows::filesystem::SourceStamp,
}
impl DeferredClipboardSource {
    pub fn inspect(roots: Vec<PathBuf>) -> Result<Self, TransferError> {
        Self::inspect_cancellable(roots, || false)
    }
    pub fn inspect_cancellable(
        roots: Vec<PathBuf>,
        cancelled: impl Fn() -> bool,
    ) -> Result<Self, TransferError> {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        let mut names = Vec::new();
        let mut stack = roots.clone();
        let mut entries = Vec::new();
        let mut total_bytes = 0_u64;
        for root in &roots {
            names.push(
                root.file_name()
                    .and_then(|name| name.to_str())
                    .ok_or_else(|| TransferError::InvalidPath("invalid source name".into()))?
                    .to_owned(),
            );
        }
        validate_deferred_names(&names)?;
        let mut sorted_names = names.clone();
        sorted_names.sort();
        sorted_names.dedup();
        if sorted_names.len() != names.len() {
            return Err(TransferError::InvalidManifest(
                "duplicate top-level source name".into(),
            ));
        }
        while let Some(path) = stack.pop() {
            if cancelled() {
                return Err(TransferError::State("文件目录扫描已取消".into()));
            }
            #[cfg(windows)]
            let (metadata, stamp) = deskfuse_platform_windows::filesystem::source_stamp(&path)?;
            #[cfg(not(windows))]
            let metadata = fs::symlink_metadata(&path)?;
            let link_target = if metadata.file_type().is_symlink() {
                let root = roots
                    .iter()
                    .filter(|root| path.starts_with(root))
                    .max_by_key(|root| root.components().count())
                    .expect("source descendant");
                if &path == root {
                    return Err(TransferError::SymlinkNotAllowed(path));
                }
                let relative =
                    RelativePath::new(path.strip_prefix(root).expect("source descendant"))?;
                let target = fs::read_link(&path)?;
                super::validate_link_target(
                    &relative,
                    target.to_str().ok_or_else(|| {
                        TransferError::InvalidPath("符号链接目标不是 UTF-8".into())
                    })?,
                )?;
                Some(target)
            } else {
                None
            };
            if !metadata.is_file() && !metadata.is_dir() && link_target.is_none() {
                return Err(TransferError::InvalidPath("unsupported source type".into()));
            }
            if entries.len() >= super::MAX_MANIFEST_ENTRIES {
                return Err(TransferError::InvalidManifest(
                    "文件目录超过 100 万项，请分批复制".into(),
                ));
            }
            if metadata.is_file() {
                total_bytes = total_bytes.checked_add(metadata.len()).ok_or_else(|| {
                    TransferError::InvalidManifest("clipboard size overflow".into())
                })?;
            }
            entries.push(SourceMetadata {
                path: path.clone(),
                size: metadata.len(),
                directory: metadata.is_dir(),
                link_target,
                #[cfg(unix)]
                device: metadata.dev(),
                #[cfg(unix)]
                inode: metadata.ino(),
                #[cfg(unix)]
                modified: (metadata.mtime(), metadata.mtime_nsec()),
                #[cfg(windows)]
                stamp,
            });
            if metadata.is_dir() {
                for entry in fs::read_dir(path)? {
                    stack.push(entry?.path());
                }
            }
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        let mut deferred_entries = entries
            .iter()
            .map(|entry| {
                let root = roots
                    .iter()
                    .filter(|root| entry.path.starts_with(root))
                    .max_by_key(|root| root.components().count())
                    .expect("source descendant");
                let root_name = root.file_name().expect("validated source name");
                let relative = if &entry.path == root {
                    PathBuf::from(root_name)
                } else {
                    PathBuf::from(root_name)
                        .join(entry.path.strip_prefix(root).expect("descendant"))
                };
                let path = RelativePath::new(relative).expect("validated source relative path");
                let kind = if entry.link_target.is_some() {
                    DeferredEntryKind::Symlink
                } else if entry.directory {
                    DeferredEntryKind::Directory
                } else {
                    DeferredEntryKind::File
                };
                DeferredFileEntry {
                    path,
                    kind,
                    size: if kind == DeferredEntryKind::File {
                        entry.size
                    } else {
                        0
                    },
                    lindex: 0,
                }
            })
            .collect::<Vec<_>>();
        deferred_entries.sort_by(|left, right| left.path.cmp(&right.path));
        for (index, entry) in deferred_entries.iter_mut().enumerate() {
            entry.lindex = u32::try_from(index)
                .map_err(|_| TransferError::InvalidManifest("deferred lindex overflow".into()))?;
        }
        validate_deferred_entries(&names, total_bytes, &deferred_entries)?;
        Ok(Self {
            roots,
            names,
            total_bytes,
            entries: std::sync::Arc::new(entries),
            deferred_entries: std::sync::Arc::new(deferred_entries),
        })
    }
    /// Metadata entries with deterministic paths and OLE indexes.
    #[must_use]
    pub fn deferred_entries(&self) -> &[DeferredFileEntry] {
        &self.deferred_entries
    }

    /// Reject deleted, replaced, resized, edited, or renamed sources before any
    /// content is read. Hash verification still protects the eventual transfer.
    /// Do not compare Unix ctime: Finder tags, quarantine attributes and chmod
    /// change it without changing file contents. Identity, size, mtime and the
    /// complete tree still have to match, including after manifest hashing.
    pub fn validate_unchanged(&self) -> Result<(), TransferError> {
        self.validate_unchanged_cancellable(|| false)
    }
    pub fn validate_unchanged_cancellable(
        &self,
        cancelled: impl Fn() -> bool,
    ) -> Result<(), TransferError> {
        if Self::inspect_cancellable(self.roots.clone(), cancelled)? != *self {
            return Err(TransferError::State("源文件已更改，请重新复制".into()));
        }
        Ok(())
    }
}

fn decode_chunk(transfer_id: TransferId, body: &[u8]) -> Result<ClipboardFileFrame, TransferError> {
    if body.len() < 4 {
        return Err(TransferError::InvalidChunk("truncated chunk path".into()));
    }
    let path_len = u32::from_be_bytes(body[..4].try_into().expect("four bytes")) as usize;
    let metadata_end = 4_usize
        .checked_add(path_len)
        .and_then(|value| value.checked_add(20))
        .ok_or_else(|| TransferError::InvalidChunk("chunk header overflow".into()))?;
    if body.len() < metadata_end {
        return Err(TransferError::InvalidChunk("truncated chunk header".into()));
    }
    let path_text = std::str::from_utf8(&body[4..4 + path_len])
        .map_err(|_| TransferError::InvalidChunk("chunk path is not UTF-8".into()))?;
    let path = RelativePath::new(path_text)?;
    let mut cursor = 4 + path_len;
    let index = u64::from_be_bytes(body[cursor..cursor + 8].try_into().expect("eight bytes"));
    cursor += 8;
    let offset = u64::from_be_bytes(body[cursor..cursor + 8].try_into().expect("eight bytes"));
    cursor += 8;
    let length = u32::from_be_bytes(body[cursor..cursor + 4].try_into().expect("four bytes"));
    cursor += 4;
    let payload = &body[cursor..];
    if payload.len() != length as usize {
        return Err(TransferError::InvalidChunk(
            "file clipboard chunk payload length mismatch".into(),
        ));
    }
    Ok(ClipboardFileFrame::Chunk {
        transfer_id,
        path,
        descriptor: ChunkDescriptor::new(index, offset, length)?,
        bytes: payload.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ConflictStrategy, FileReceiver, read_chunk};

    #[test]
    fn file_frames_round_trip() {
        let id = TransferId::new();
        let path = RelativePath::new("folder/file.txt").expect("path");
        let frame = ClipboardFileFrame::Chunk {
            transfer_id: id,
            path,
            descriptor: ChunkDescriptor::new(4, 12, 3).expect("descriptor"),
            bytes: b"abc".to_vec(),
        };
        let decoded = ClipboardFileFrame::decode(&frame.encode().expect("encode")).expect("decode");
        match decoded {
            ClipboardFileFrame::Chunk {
                transfer_id,
                descriptor,
                bytes,
                ..
            } => {
                assert_eq!(transfer_id, id);
                assert_eq!(descriptor.offset, 12);
                assert_eq!(bytes, b"abc");
            }
            _ => panic!("wrong frame"),
        }
    }

    #[test]
    fn cancellation_frame_round_trips_and_rejects_an_oversized_reason() {
        let id = TransferId::new();
        let frame = ClipboardFileFrame::Cancel {
            transfer_id: id,
            reason: "superseded by newer clipboard content".to_owned(),
        };
        assert!(matches!(
            ClipboardFileFrame::decode(&frame.encode().expect("encode")).expect("decode"),
            ClipboardFileFrame::Cancel { transfer_id, reason }
                if transfer_id == id && reason == "superseded by newer clipboard content"
        ));
        let oversized = ClipboardFileFrame::Cancel {
            transfer_id: id,
            reason: "x".repeat(1025),
        };
        assert!(oversized.encode().is_err());
    }

    #[test]
    fn acknowledgement_frame_round_trips() {
        let id = TransferId::new();
        let encoded = ClipboardFileFrame::Ack { transfer_id: id }
            .encode()
            .expect("encode");
        assert!(matches!(
            ClipboardFileFrame::decode(&encoded).expect("decode"),
            ClipboardFileFrame::Ack { transfer_id } if transfer_id == id
        ));
    }

    #[test]
    fn caller_supplied_transfer_id_survives_folder_planning() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("Copied Folder");
        fs::create_dir(&source).expect("source folder");
        fs::write(source.join("note.txt"), b"actual file contents").expect("source file");
        let transfer_id = TransferId::new();

        let plan = ClipboardFilePlan::from_paths_with_transfer_id(
            std::slice::from_ref(&source),
            transfer_id,
        )
        .expect("plan");
        assert_eq!(plan.transfer_id(), transfer_id);
    }

    #[test]
    fn folder_clipboard_round_trip_writes_real_file_bytes() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("Copied Folder");
        fs::create_dir(&source).expect("source folder");
        fs::write(source.join("note.txt"), b"actual file contents").expect("source file");

        let plan = ClipboardFilePlan::from_paths(std::slice::from_ref(&source)).expect("plan");
        assert!(
            plan.manifest()
                .entries()
                .iter()
                .any(|entry| entry.path().as_str() == "Copied Folder/note.txt")
        );

        let offer = ClipboardFileFrame::Offer {
            transfer_id: plan.transfer_id(),
            manifest: plan.manifest().clone(),
        };
        let manifest = match ClipboardFileFrame::decode(&offer.encode().expect("offer encode"))
            .expect("offer decode")
        {
            ClipboardFileFrame::Offer { manifest, .. } => manifest,
            _ => panic!("wrong offer frame"),
        };
        let destination = temp.path().join("received");
        let receiver = FileReceiver::new(&destination).expect("receiver");
        let mut staging = receiver
            .begin(plan.transfer_id(), &manifest)
            .expect("staging");
        for source_file in plan.sources() {
            for descriptor in source_file.chunks() {
                let bytes = read_chunk(source_file.source_path(), *descriptor).expect("read chunk");
                let encoded = ClipboardFileFrame::Chunk {
                    transfer_id: plan.transfer_id(),
                    path: source_file.relative_path().clone(),
                    descriptor: *descriptor,
                    bytes,
                }
                .encode()
                .expect("chunk encode");
                match ClipboardFileFrame::decode(&encoded).expect("chunk decode") {
                    ClipboardFileFrame::Chunk {
                        path,
                        descriptor,
                        bytes,
                        ..
                    } => staging
                        .write_chunk(&path, descriptor, &bytes)
                        .expect("write chunk"),
                    _ => panic!("wrong chunk frame"),
                }
            }
        }
        for entry in manifest.files() {
            staging.complete_file(entry).expect("complete file");
        }
        receiver
            .commit_staging(staging, ConflictStrategy::Overwrite)
            .expect("commit");

        assert_eq!(
            fs::read(destination.join("Copied Folder/note.txt")).expect("received file"),
            b"actual file contents"
        );
    }
    #[test]
    fn deferred_metadata_and_request_round_trip_without_file_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("large.bin");
        let file = fs::File::create(&path).unwrap();
        file.set_len(8 * 1024 * 1024 * 1024).unwrap();
        drop(file);
        let source = DeferredClipboardSource::inspect(vec![path.clone()]).unwrap();
        assert_eq!(source.total_bytes, 8 * 1024 * 1024 * 1024);
        let id = TransferId::new();
        let frame = ClipboardFileFrame::Deferred {
            transfer_id: id,
            names: source.names.clone(),
            total_bytes: source.total_bytes,
            entries: source.deferred_entries().to_vec(),
        };
        let encoded = frame.encode().unwrap();
        assert!(
            encoded.len() < 512,
            "deferred metadata frame unexpectedly large: {} bytes",
            encoded.len()
        );
        assert!(
            matches!(ClipboardFileFrame::decode(&encoded).unwrap(),ClipboardFileFrame::Deferred {transfer_id,total_bytes,..} if transfer_id==id && total_bytes==source.total_bytes)
        );
        assert!(
            matches!(ClipboardFileFrame::decode(&ClipboardFileFrame::Request {transfer_id:id}.encode().unwrap()).unwrap(),ClipboardFileFrame::Request {transfer_id} if transfer_id==id)
        );
        source.validate_unchanged().unwrap();
        fs::write(&path, b"new").unwrap();
        assert!(source.validate_unchanged().is_err());
        fs::remove_file(path).unwrap();
        assert!(source.validate_unchanged().is_err());
    }
    #[cfg(windows)]
    #[test]
    fn windows_paths_source_snapshot_detects_same_size_replacement() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("source.txt");
        fs::write(&path, b"old").unwrap();
        let source = DeferredClipboardSource::inspect(vec![path.clone()]).unwrap();
        source.validate_unchanged().unwrap();
        // Keep the original inode allocated so the filesystem cannot reuse its ID.
        fs::rename(&path, temp.path().join("previous.txt")).unwrap();
        fs::write(&path, b"new").unwrap();
        assert!(source.validate_unchanged().is_err());
        let replacement = DeferredClipboardSource::inspect(vec![path.clone()]).unwrap();
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(
                std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000),
            )
            .unwrap();
        assert!(replacement.validate_unchanged().is_err());
        let directory = DeferredClipboardSource::inspect(vec![temp.path().to_owned()]).unwrap();
        directory.validate_unchanged().unwrap();
        fs::write(temp.path().join("additional.txt"), b"x").unwrap();
        assert!(directory.validate_unchanged().is_err());
    }
}
