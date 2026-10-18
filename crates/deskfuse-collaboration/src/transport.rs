//! Outbound collaboration transport. Hosts inject a real QUIC session; tests inject a memory bus.
use crate::{api::OperationId, error::Result, file_transfer::FileManifest};
use async_trait::async_trait;
use deskfuse_core::DeviceId;
use deskfuse_platform::ports::ContentData;
use deskfuse_transfer::TransferId;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::sync::mpsc;

/// Clipboard body sent to a connected peer. Bytes stay off UI snapshots.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteContent {
    pub generation: u64,
    pub operation_id: OperationId,
    pub data: ContentData,
    pub captured_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum OutboundMessage {
    Clipboard(RemoteContent),
    CancelClipboard {
        operation_id: OperationId,
    },
    FileManifest {
        transfer_id: TransferId,
        manifest: FileManifest,
    },
    FileChunk {
        transfer_id: TransferId,
        index: u32,
        data: Vec<u8>,
    },
    CancelTransfer {
        transfer_id: TransferId,
    },
}

/// Send collaboration messages to authenticated sessions.
#[async_trait]
pub trait CollaborationTransport: Send + Sync {
    fn active_sessions(&self) -> Vec<DeviceId>;
    async fn send(&self, to: DeviceId, message: OutboundMessage) -> Result<()>;
}

/// In-memory fan-out used by tests. Each device has an inbox.
#[derive(Clone, Default)]
pub struct MemoryTransport {
    inner: Arc<Mutex<MemoryTransportState>>,
}

#[derive(Default)]
struct MemoryTransportState {
    sessions: Vec<DeviceId>,
    inboxes: HashMap<DeviceId, mpsc::UnboundedSender<OutboundMessage>>,
    sent: Vec<(DeviceId, OutboundMessage)>,
}

impl MemoryTransport {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn connect(&self, device: DeviceId) -> mpsc::UnboundedReceiver<OutboundMessage> {
        let (tx, rx) = mpsc::unbounded_channel();
        if let Ok(mut state) = self.inner.lock() {
            if !state.sessions.contains(&device) {
                state.sessions.push(device);
            }
            state.inboxes.insert(device, tx);
        }
        rx
    }

    pub fn disconnect(&self, device: DeviceId) {
        if let Ok(mut state) = self.inner.lock() {
            state.sessions.retain(|id| *id != device);
            state.inboxes.remove(&device);
        }
    }

    pub fn sent(&self) -> Vec<(DeviceId, OutboundMessage)> {
        self.inner
            .lock()
            .map(|state| state.sent.clone())
            .unwrap_or_default()
    }
}

#[async_trait]
impl CollaborationTransport for MemoryTransport {
    fn active_sessions(&self) -> Vec<DeviceId> {
        self.inner
            .lock()
            .map(|state| state.sessions.clone())
            .unwrap_or_default()
    }

    async fn send(&self, to: DeviceId, message: OutboundMessage) -> Result<()> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| crate::error::CollaborationError::failed("transport lock poisoned"))?;
        state.sent.push((to, message.clone()));
        if let Some(tx) = state.inboxes.get(&to) {
            let _ = tx.send(message);
        }
        Ok(())
    }
}
