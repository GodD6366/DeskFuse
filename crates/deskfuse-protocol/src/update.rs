//! Dedicated update frames. Validation here is structural, never release authorization.
use crate::ProtocolError;
use prost::Message;

pub const MAX_UPDATE_CHUNK: usize = 64 * 1024;
pub const MAX_RELEASE_INVENTORY: usize = 16;
pub const MAX_RELEASE_MANIFEST: usize = 64 * 1024;
const MAX_ARTIFACT_SIZE: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Clone, PartialEq, Message)]
pub struct ReleaseInventory {
    #[prost(string, tag = "1")]
    pub installed_version: String,
    #[prost(uint64, tag = "2")]
    pub installed_build: u64,
    /// Empty on bootstrap installations without release metadata.
    #[prost(bytes = "vec", tag = "3")]
    pub installed_release_id: Vec<u8>,
    #[prost(uint32, tag = "4")]
    pub target: u32,
    /// Each entry is a bounded signed manifest; the receiver verifies its pinned key.
    #[prost(bytes = "vec", repeated, tag = "5")]
    pub signed_manifests: Vec<Vec<u8>>,
    /// GitHub-channel self-packages. JSON `PeerAppArtifact` values.
    /// Trust is Developer ID of the contained app, never LAN Ed25519.
    #[prost(bytes = "vec", repeated, tag = "6")]
    pub github_artifacts: Vec<Vec<u8>>,
}
#[derive(Clone, PartialEq, Message)]
pub struct UpdateRequest {
    #[prost(bytes = "vec", tag = "1")]
    pub transaction_id: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub release_id: Vec<u8>,
    #[prost(uint32, tag = "3")]
    pub target: u32,
    #[prost(bytes = "vec", tag = "4")]
    pub sha256: Vec<u8>,
    /// Pull starting at this offset; a new transaction restarts at offset zero.
    #[prost(uint64, tag = "5")]
    pub offset: u64,
    /// 本次请求希望连续收到的最大字节数，应答在同一条流上按 `UpdateChunk`
    /// 逐帧写出直到满足或到达文件末尾。`0` 表示只返回一个块。
    #[prost(uint64, tag = "6")]
    pub max_bytes: u64,
}
#[derive(Clone, PartialEq, Message)]
#[prost(skip_debug)]
pub struct UpdateChunk {
    #[prost(bytes = "vec", tag = "1")]
    pub transaction_id: Vec<u8>,
    #[prost(uint64, tag = "2")]
    pub offset: u64,
    #[prost(bytes = "vec", tag = "3")]
    pub data: Vec<u8>,
}
impl std::fmt::Debug for UpdateChunk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpdateChunk")
            .field("transaction_id", &self.transaction_id)
            .field("offset", &self.offset)
            .field("bytes", &self.data.len())
            .finish()
    }
}
#[derive(Clone, PartialEq, Message)]
pub struct UpdateCancel {
    #[prost(bytes = "vec", tag = "1")]
    pub transaction_id: Vec<u8>,
    /// Stable reason code, never arbitrary diagnostic content.
    #[prost(uint32, tag = "2")]
    pub reason: u32,
}
#[derive(Clone, PartialEq, Message)]
pub struct UpdateStatus {
    #[prost(bytes = "vec", tag = "1")]
    pub transaction_id: Vec<u8>,
    #[prost(uint32, tag = "2")]
    pub phase: u32,
    #[prost(uint64, tag = "3")]
    pub received_bytes: u64,
    #[prost(uint64, tag = "4")]
    pub total_bytes: u64,
}
#[derive(Clone, PartialEq, Message)]
pub struct UpdateReceipt {
    #[prost(bytes = "vec", tag = "1")]
    pub transaction_id: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub release_id: Vec<u8>,
    #[prost(string, tag = "3")]
    pub installed_version: String,
    #[prost(uint64, tag = "4")]
    pub installed_build: u64,
    #[prost(uint32, tag = "5")]
    pub target: u32,
    #[prost(bytes = "vec", tag = "6")]
    pub sha256: Vec<u8>,
}

/// Ask a paired device to run its normal update check and installation flow.
/// The receiver still chooses and verifies the release locally.
#[derive(Clone, PartialEq, Message)]
pub struct UpdateCommand {
    /// Stable action code. `1` means download and install a compatible update.
    #[prost(uint32, tag = "1")]
    pub action: u32,
}

/// Acknowledgement for [`UpdateCommand`].
#[derive(Clone, PartialEq, Message)]
pub struct UpdateCommandAck {
    /// `1` accepted, `2` unsupported, `3` busy/unavailable.
    #[prost(uint32, tag = "1")]
    pub status: u32,
}
fn id(value: &[u8]) -> bool {
    value.len() == 16 && value.iter().any(|v| *v != 0)
}
fn target(value: u32) -> bool {
    matches!(value, 1 | 2)
}
fn version(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b".-+".contains(&c))
}

pub(crate) fn validate(message: &crate::Message) -> Result<(), ProtocolError> {
    use crate::Message as M;
    let valid = match message {
        M::ReleaseInventory(v) => {
            version(&v.installed_version)
                && target(v.target)
                && (v.installed_release_id.is_empty() || id(&v.installed_release_id))
                && v.signed_manifests.len() <= MAX_RELEASE_INVENTORY
                && v.signed_manifests
                    .iter()
                    .all(|m| !m.is_empty() && m.len() <= MAX_RELEASE_MANIFEST)
                && v.github_artifacts.len() <= MAX_RELEASE_INVENTORY
                && v.github_artifacts
                    .iter()
                    .all(|m| !m.is_empty() && m.len() <= MAX_RELEASE_MANIFEST)
        }
        M::UpdateRequest(v) => {
            id(&v.transaction_id)
                && id(&v.release_id)
                && target(v.target)
                && v.sha256.len() == 32
                && v.offset < MAX_ARTIFACT_SIZE
                && v.max_bytes <= MAX_ARTIFACT_SIZE
        }
        M::UpdateChunk(v) => {
            id(&v.transaction_id)
                && !v.data.is_empty()
                && v.data.len() <= MAX_UPDATE_CHUNK
                && v.offset
                    .checked_add(v.data.len() as u64)
                    .is_some_and(|end| end <= MAX_ARTIFACT_SIZE)
        }
        M::UpdateCancel(v) => id(&v.transaction_id) && (1..=6).contains(&v.reason),
        M::UpdateStatus(v) => {
            id(&v.transaction_id)
                && (1..=12).contains(&v.phase)
                && v.total_bytes > 0
                && v.total_bytes <= MAX_ARTIFACT_SIZE
                && v.received_bytes <= v.total_bytes
        }
        M::UpdateReceipt(v) => {
            id(&v.transaction_id)
                && id(&v.release_id)
                && target(v.target)
                && version(&v.installed_version)
                && v.installed_build > 0
                && v.sha256.len() == 32
        }
        M::UpdateCommand(v) => v.action == 1,
        M::UpdateCommandAck(v) => (1..=3).contains(&v.status),
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(ProtocolError::InvalidPayload(
            "update frame bounds or metadata",
        ))
    }
}
