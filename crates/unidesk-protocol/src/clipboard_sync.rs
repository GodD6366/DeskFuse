//! Acknowledged clipboard protocol, carried inside ClipboardUpdate. No legacy fallback.
use crate::ProtocolError;
use prost::Message;
pub const MIME: &str = "application/x-unidesk-clipboard-sync-v2";
pub const CHUNK_BYTES: usize = 1024 * 1024;
pub const MAX_BYTES: usize = 257 * 1024 * 1024;
pub const HELLO: u32 = 0;
pub const DATA: u32 = 1;
pub const ACK: u32 = 2;
pub const APPLIED: u32 = 1;
pub const REJECTED: u32 = 2;
pub const SUPERSEDED: u32 = 3;
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
}
