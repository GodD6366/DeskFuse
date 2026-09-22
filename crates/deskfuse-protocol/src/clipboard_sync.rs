//! Acknowledged clipboard protocol, carried inside ClipboardUpdate. No legacy fallback.
use crate::ProtocolError;
use prost::Message;
pub const MIME: &str = "application/x-deskfuse-clipboard-sync-v2";
pub const CHUNK_BYTES: usize = 1024 * 1024;
pub const MAX_BYTES: usize = 257 * 1024 * 1024;
pub const HELLO: u32 = 0;
pub const DATA: u32 = 1;
pub const ACK: u32 = 2;
pub const APPLIED: u32 = 1;
pub const REJECTED: u32 = 2;
pub const SUPERSEDED: u32 = 3;
/// Source-policy generation every host must advertise on HELLO and DATA.
/// Frames built by [`SyncMessage::hello`] / [`SyncMessage::data`] always carry
/// it, so hosts never hand-roll the value and drift apart.
pub const POLICY_VERSION: u32 = 1;
/// Rejection reason a policy-unaware peer earns on the receiving host.
pub const POLICY_REJECTED_HELLO: &str = "对端需升级以支持历史排除策略";
pub const POLICY_REJECTED_CONTENT: &str = "剪贴板来源策略无效，已拒绝接收";

/// Source policy carried beside every DATA frame; never clipboard content.
/// Defined here so the macOS engine and the Windows service cannot drift on
/// field names: a frame rejected by one host must be rejected by the other.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SourceMetadata {
    pub app_bundle_id: Option<String>,
    pub device_id: Option<String>,
    pub created_at_ms: Option<u64>,
    pub no_history: bool,
    /// Exact decoded image hash, including single-image file representations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_digest: Option<String>,
}
#[derive(Clone, PartialEq, Message)]
pub struct SyncMessage {
    #[prost(uint32, tag = "1")]
    pub kind: u32,
    #[prost(bytes = "vec", tag = "2")]
    pub source_epoch: Vec<u8>,
    #[prost(bytes = "vec", tag = "3")]
    pub target_epoch: Vec<u8>,
    #[prost(bytes = "vec", tag = "4")]
    pub sync_id: Vec<u8>,
    #[prost(uint64, tag = "5")]
    pub revision: u64,
    #[prost(string, tag = "6")]
    pub mime: String,
    #[prost(bytes = "vec", tag = "7")]
    pub digest: Vec<u8>,
    #[prost(uint64, tag = "8")]
    pub total: u64,
    #[prost(uint64, tag = "9")]
    pub offset: u64,
    #[prost(bytes = "vec", tag = "10")]
    pub data: Vec<u8>,
    /// Policy-aware peers must advertise version 1 before exchanging content.
    #[prost(uint32, tag = "12")]
    pub policy_version: u32,
    /// Source metadata, never clipboard content.
    #[prost(bytes = "vec", tag = "13")]
    pub source_metadata: Vec<u8>,
    /// Receive budget the HELLO sender can accept, in bytes. Absent (0) on
    /// older peers; receivers then assume the full wire limit.
    #[prost(uint64, tag = "14")]
    pub max_content_bytes: u64,
    #[prost(uint32, tag = "11")]
    pub outcome: u32,
}
impl SyncMessage {
    pub fn decode_checked(bytes: &[u8]) -> Result<Self, ProtocolError> {
        if bytes.len() > CHUNK_BYTES + 8192 {
            return Err(ProtocolError::InvalidPayload(
                "clipboard sync frame too large",
            ));
        }
        let message = Self::decode(bytes)
            .map_err(|_| ProtocolError::InvalidPayload("invalid clipboard sync frame"))?;
        message.validate()?;
        Ok(message)
    }
    pub fn encode_checked(&self) -> Result<Vec<u8>, ProtocolError> {
        self.validate()?;
        Ok(self.encode_to_vec())
    }
    fn validate(&self) -> Result<(), ProtocolError> {
        let valid = self.source_metadata.len() <= 4096
            && self.source_epoch.len() == 16
            && match self.kind {
                HELLO => {
                    (self.target_epoch.is_empty() || self.target_epoch.len() == 16)
                        && self.data.is_empty()
                }
                DATA => {
                    self.target_epoch.len() == 16
                        && self.sync_id.len() == 16
                        && self.revision > 0
                        && !self.mime.is_empty()
                        && self.mime.len() <= 128
                        && self.digest.len() == 32
                        && self.total <= MAX_BYTES as u64
                        && self.data.len() <= CHUNK_BYTES
                        && self
                            .offset
                            .checked_add(self.data.len() as u64)
                            .is_some_and(|end| end <= self.total)
                        && (self.total == 0 || !self.data.is_empty())
                }
                ACK => {
                    self.target_epoch.len() == 16
                        && self.sync_id.len() == 16
                        && self.revision > 0
                        && matches!(self.outcome, APPLIED | REJECTED | SUPERSEDED)
                        && self.data.is_empty()
                }
                _ => false,
            };
        if valid {
            Ok(())
        } else {
            Err(ProtocolError::InvalidPayload(
                "clipboard sync fields invalid",
            ))
        }
    }
    /// Every host builds HELLO through here so the policy generation and the
    /// advertised receive budget can never be forgotten by one side.
    pub fn hello(source_epoch: Vec<u8>, target_epoch: Vec<u8>, max_content_bytes: u64) -> Self {
        Self {
            kind: HELLO,
            policy_version: POLICY_VERSION,
            max_content_bytes,
            source_epoch,
            target_epoch,
            ..Default::default()
        }
    }
    pub fn ack(
        source_epoch: Vec<u8>,
        target_epoch: Vec<u8>,
        sync_id: Vec<u8>,
        revision: u64,
        outcome: u32,
    ) -> Self {
        Self {
            kind: ACK,
            source_epoch,
            target_epoch,
            sync_id,
            revision,
            outcome,
            ..Default::default()
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub fn data(
        source_epoch: Vec<u8>,
        target_epoch: Vec<u8>,
        sync_id: Vec<u8>,
        revision: u64,
        mime: String,
        digest: Vec<u8>,
        total: u64,
        offset: u64,
        data: Vec<u8>,
        metadata: &SourceMetadata,
    ) -> Self {
        Self {
            kind: DATA,
            policy_version: POLICY_VERSION,
            source_metadata: serde_json::to_vec(metadata).unwrap_or_default(),
            source_epoch,
            target_epoch,
            sync_id,
            revision,
            mime,
            digest,
            total,
            offset,
            data,
            ..Default::default()
        }
    }
    /// Shared gate: a HELLO from a peer that never heard of source policies
    /// is a peer whose DATA frames would drift, so the handshake stops here.
    pub fn hello_policy_ok(&self) -> bool {
        self.policy_version == POLICY_VERSION
    }
    /// Shared gate: DATA must carry the current policy generation *and*
    /// decodable source metadata, checked identically by both hosts.
    pub fn data_policy_ok(&self) -> bool {
        self.policy_version == POLICY_VERSION
            && serde_json::from_slice::<SourceMetadata>(&self.source_metadata).is_ok()
    }
    /// Effective receive budget a HELLO advertises; legacy peers that predate
    /// the field are assumed to accept the full wire limit.
    pub fn advertised_max_content(&self) -> u64 {
        if self.max_content_bytes == 0 {
            MAX_BYTES as u64
        } else {
            self.max_content_bytes
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_cross_epoch_and_oversized_chunks() {
        let mut m = SyncMessage {
            kind: DATA,
            source_epoch: vec![1; 16],
            target_epoch: vec![2; 16],
            sync_id: vec![3; 16],
            revision: 1,
            mime: "text/plain".into(),
            digest: vec![0; 32],
            total: 1,
            data: vec![1],
            ..Default::default()
        };
        assert!(m.encode_checked().is_ok());
        m.target_epoch.clear();
        assert!(m.encode_checked().is_err());
        m.target_epoch = vec![2; 16];
        m.offset = u64::MAX;
        assert!(m.encode_checked().is_err());
    }
    #[test]
    fn constructors_carry_policy_and_budget_across_the_wire() {
        let hello = SyncMessage::hello(vec![1; 16], vec![2; 16], 16 * 1024 * 1024);
        assert!(hello.hello_policy_ok());
        assert_eq!(hello.advertised_max_content(), 16 * 1024 * 1024);
        let round = SyncMessage::decode_checked(&hello.encode_checked().unwrap()).unwrap();
        assert!(round.hello_policy_ok());
        assert_eq!(round.advertised_max_content(), 16 * 1024 * 1024);

        let metadata = SourceMetadata {
            app_bundle_id: Some("com.example.app".into()),
            no_history: true,
            ..Default::default()
        };
        let data = SyncMessage::data(
            vec![1; 16],
            vec![2; 16],
            vec![3; 16],
            7,
            "text/plain".into(),
            vec![0; 32],
            1,
            0,
            vec![9],
            &metadata,
        );
        assert!(data.data_policy_ok());
        let round = SyncMessage::decode_checked(&data.encode_checked().unwrap()).unwrap();
        assert!(round.data_policy_ok());
        let decoded: SourceMetadata = serde_json::from_slice(&round.source_metadata).unwrap();
        assert_eq!(decoded.app_bundle_id.as_deref(), Some("com.example.app"));
        assert!(decoded.no_history);

        let ack = SyncMessage::ack(vec![1; 16], vec![2; 16], vec![3; 16], 7, REJECTED);
        assert!(SyncMessage::decode_checked(&ack.encode_checked().unwrap()).is_ok());
    }
    #[test]
    fn policy_gates_reject_legacy_peers_and_undecodable_metadata() {
        let mut hello = SyncMessage::hello(vec![1; 16], Vec::new(), 0);
        hello.policy_version = 0;
        assert!(!hello.hello_policy_ok());
        assert!(!SyncMessage::default().hello_policy_ok());

        let metadata = SourceMetadata::default();
        let mut data = SyncMessage::data(
            vec![1; 16],
            vec![2; 16],
            vec![3; 16],
            1,
            "text/plain".into(),
            vec![0; 32],
            1,
            0,
            vec![9],
            &metadata,
        );
        assert!(data.data_policy_ok());
        data.policy_version = 0;
        assert!(!data.data_policy_ok());
        let mut no_metadata = SyncMessage::data(
            vec![1; 16],
            vec![2; 16],
            vec![3; 16],
            1,
            "text/plain".into(),
            vec![0; 32],
            1,
            0,
            vec![9],
            &metadata,
        );
        no_metadata.source_metadata.clear();
        assert!(!no_metadata.data_policy_ok());
    }
    #[test]
    fn legacy_hello_without_budget_falls_back_to_the_wire_limit() {
        let hello = SyncMessage::hello(vec![1; 16], Vec::new(), 0);
        assert_eq!(hello.advertised_max_content(), MAX_BYTES as u64);
    }
}
