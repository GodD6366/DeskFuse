//! Opaque content handles. Collaboration APIs never return clipboard bodies to UI snapshots.
use deskfuse_platform::ports::ContentData;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Stable handle for a captured or received clipboard body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentHandle {
    pub id: Uuid,
    pub mime: String,
    pub size: u64,
    pub digest: [u8; 32],
}

impl ContentHandle {
    #[must_use]
    pub fn from_data(data: &ContentData) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(data.mime.as_bytes());
        hasher.update(&data.bytes);
        for url in &data.file_urls {
            hasher.update(url.as_bytes());
        }
        for item in &data.items {
            for representation in &item.representations {
                hasher.update(representation.type_name.as_bytes());
                hasher.update(&representation.data);
            }
        }
        let digest = hasher.finalize();
        let mut bytes = [0_u8; 32];
        bytes.copy_from_slice(&digest);
        Self {
            id: Uuid::new_v4(),
            mime: data.mime.clone(),
            size: data.bytes.len() as u64,
            digest: bytes,
        }
    }
}
