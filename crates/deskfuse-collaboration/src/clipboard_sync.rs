//! Shared clipboard policy. macOS and Windows hosts inject ports; they do not reimplement this.
use crate::{
    api::{ClipboardStatus, CollaborationEvent, OperationId},
    content::ContentHandle,
    error::Result,
    transport::{CollaborationTransport, OutboundMessage, RemoteContent},
};
use deskfuse_core::DeviceId;
use deskfuse_platform::ports::{
    ApplyResult, CaptureSource, CapturedContent, ClipboardApplyPort, ClipboardCapturePort,
    ContentToApply, NativeRevision,
};
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};
use tokio::sync::{RwLock, mpsc};

struct ActiveOperation {
    operation_id: OperationId,
    #[allow(dead_code)]
    generation: u64,
}

struct PendingApply {
    operation_id: OperationId,
    #[allow(dead_code)]
    from: DeviceId,
    #[allow(dead_code)]
    generation: u64,
    content: RemoteContent,
    expected_revision: NativeRevision,
}

#[derive(Default)]
struct SyncState {
    local_generation: u64,
    last_local_capture_ms: u64,
    active_operation: Option<ActiveOperation>,
    pending_applies: VecDeque<PendingApply>,
    received_operations: HashMap<DeviceId, u64>,
    applying: bool,
    last_error: Option<String>,
}

/// Clipboard synchronization shared by every host.
pub struct ClipboardSyncService {
    capture_port: Arc<dyn ClipboardCapturePort>,
    apply_port: Arc<dyn ClipboardApplyPort>,
    transport: Arc<dyn CollaborationTransport>,
    state: Arc<RwLock<SyncState>>,
    events: mpsc::UnboundedSender<CollaborationEvent>,
}

impl ClipboardSyncService {
    pub fn new(
        capture_port: Arc<dyn ClipboardCapturePort>,
        apply_port: Arc<dyn ClipboardApplyPort>,
        transport: Arc<dyn CollaborationTransport>,
        events: mpsc::UnboundedSender<CollaborationEvent>,
    ) -> Self {
        Self {
            capture_port,
            apply_port,
            transport,
            state: Arc::new(RwLock::new(SyncState::default())),
            events,
        }
    }

    pub async fn status(&self) -> ClipboardStatus {
        let state = self.state.read().await;
        ClipboardStatus {
            local_generation: state.local_generation,
            last_operation: state.active_operation.as_ref().map(|op| op.operation_id),
            last_error: state.last_error.clone(),
        }
    }

    /// Local user copy. Remote writes are ignored so applied content does not loop.
    pub async fn on_local_capture(&self, content: CapturedContent) -> Result<()> {
        if content.source == CaptureSource::Remote {
            return Ok(());
        }
        if content.data.bytes.is_empty() && content.data.file_urls.is_empty() {
            return Ok(());
        }
        self.cancel_active_operation().await;
        let generation = self.next_generation().await;
        let operation_id = OperationId::new();
        {
            let mut state = self.state.write().await;
            state.active_operation = Some(ActiveOperation {
                operation_id,
                generation,
            });
            state.last_local_capture_ms = state.last_local_capture_ms.max(content.captured_at_ms);
            state.last_error = None;
        }
        let remote = RemoteContent {
            generation,
            operation_id,
            data: content.data,
            captured_at_ms: content.captured_at_ms,
        };
        for session in self.transport.active_sessions() {
            self.transport
                .send(session, OutboundMessage::Clipboard(remote.clone()))
                .await?;
        }
        Ok(())
    }

    pub async fn on_remote_clipboard(&self, from: DeviceId, content: RemoteContent) -> Result<()> {
        if self.is_duplicate(from, content.generation).await {
            return Ok(());
        }
        {
            let mut state = self.state.write().await;
            if content.captured_at_ms < state.last_local_capture_ms {
                let _ = self.events.send(CollaborationEvent::ClipboardSuperseded {
                    operation_id: content.operation_id,
                });
                return Ok(());
            }
            state.received_operations.insert(from, content.generation);
        }
        let captured = self.capture_port.capture().await?;
        let handle = ContentHandle::from_data(&content.data);
        let _ = self.events.send(CollaborationEvent::ClipboardReceived {
            content: handle,
            from,
        });
        {
            let mut state = self.state.write().await;
            state.pending_applies.push_back(PendingApply {
                operation_id: content.operation_id,
                from,
                generation: content.generation,
                content,
                expected_revision: captured.revision,
            });
        }
        self.process_apply_queue().await
    }

    pub async fn cancel_clipboard(&self) -> Result<()> {
        self.cancel_active_operation().await;
        Ok(())
    }

    async fn process_apply_queue(&self) -> Result<()> {
        loop {
            let pending = {
                let mut state = self.state.write().await;
                if state.applying {
                    return Ok(());
                }
                match state.pending_applies.pop_front() {
                    Some(pending) => {
                        state.applying = true;
                        pending
                    }
                    None => return Ok(()),
                }
            };
            let result = self
                .apply_port
                .apply(
                    ContentToApply {
                        data: pending.content.data.clone(),
                        remote_origin_marker: Some(pending.operation_id.to_string()),
                    },
                    pending.expected_revision,
                )
                .await;
            {
                let mut state = self.state.write().await;
                state.applying = false;
            }
            match result {
                Ok(ApplyResult::Applied { .. }) => {
                    let _ = self.events.send(CollaborationEvent::ClipboardApplied {
                        operation_id: pending.operation_id,
                    });
                }
                Ok(ApplyResult::Superseded { .. }) => {
                    let _ = self.events.send(CollaborationEvent::ClipboardSuperseded {
                        operation_id: pending.operation_id,
                    });
                }
                Ok(ApplyResult::Rejected { reason }) => {
                    self.record_error(reason.clone()).await;
                    let _ = self.events.send(CollaborationEvent::ClipboardFailed {
                        operation_id: pending.operation_id,
                        error: reason,
                    });
                }
                Err(error) => {
                    let message = error.to_string();
                    self.record_error(message.clone()).await;
                    let _ = self.events.send(CollaborationEvent::ClipboardFailed {
                        operation_id: pending.operation_id,
                        error: message,
                    });
                }
            }
        }
    }

    async fn is_duplicate(&self, from: DeviceId, generation: u64) -> bool {
        let state = self.state.read().await;
        state
            .received_operations
            .get(&from)
            .is_some_and(|seen| *seen >= generation)
    }

    async fn cancel_active_operation(&self) {
        let active = {
            let mut state = self.state.write().await;
            state.pending_applies.clear();
            state.active_operation.take()
        };
        if let Some(active) = active {
            for session in self.transport.active_sessions() {
                let _ = self
                    .transport
                    .send(
                        session,
                        OutboundMessage::CancelClipboard {
                            operation_id: active.operation_id,
                        },
                    )
                    .await;
            }
        }
    }

    async fn next_generation(&self) -> u64 {
        let mut state = self.state.write().await;
        state.local_generation += 1;
        state.local_generation
    }

    async fn record_error(&self, error: String) {
        self.state.write().await.last_error = Some(error);
    }
}
