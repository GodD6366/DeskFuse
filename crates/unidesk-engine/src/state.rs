//! Reducer for platform-independent business state. Window selection is never stored here.
use crate::{
    model::{AppModel, TransferItem},
    tokio_bridge::UiEvent,
};
use unidesk_core::{FileTransferSettings, TransferCacheSnapshot};

#[derive(Clone)]
pub struct EngineState {
    pub revision: u64,
    pub debug_logging_enabled: bool,
    pub model: AppModel,
    pub file_settings: FileTransferSettings,
    pub cache: TransferCacheSnapshot,
    pub cache_message: Option<String>,
    pub recording: bool,
    pub file_paste_acquiring: std::collections::BTreeSet<String>,
    pub file_paste_sources: std::collections::BTreeMap<String, String>,
    pub file_paste_retryable: std::collections::BTreeSet<String>,
    pub file_paste_delivering: std::collections::BTreeSet<String>,
}
impl Default for EngineState {
    fn default() -> Self {
        Self {
            revision: 0,
            debug_logging_enabled: unidesk_core::debug_logging::enabled(),
            model: AppModel::starting(),
            file_settings: Default::default(),
            cache: Default::default(),
            cache_message: None,
            recording: false,
            file_paste_acquiring: Default::default(),
            file_paste_sources: Default::default(),
            file_paste_retryable: Default::default(),
            file_paste_delivering: Default::default(),
        }
    }
}
impl EngineState {
    pub fn apply(&mut self, event: UiEvent) {
        self.revision = self.revision.saturating_add(1);
        let model = &mut self.model;
        match event {
            UiEvent::FilePasteAcquiring {
                paste_id,
                transfer_id,
            } => {
                self.file_paste_acquiring.insert(paste_id.to_string());
                self.file_paste_sources
                    .insert(paste_id.to_string(), transfer_id.to_string());
            }
            UiEvent::FilePasteLinked {
                paste_id,
                transfer_id,
                can_retry,
            } => {
                self.file_paste_acquiring.remove(&paste_id.to_string());
                self.file_paste_sources
                    .insert(paste_id.to_string(), transfer_id.to_string());
                if can_retry {
                    self.file_paste_retryable.insert(paste_id.to_string());
                } else {
                    self.file_paste_retryable.remove(&paste_id.to_string());
                }
            }
            UiEvent::DebugLoggingChanged(enabled) => self.debug_logging_enabled = enabled,
            UiEvent::WorkflowRun(_) | UiEvent::WorkflowCatalogChanged => {}
            UiEvent::RuntimeView(view) => model.apply_runtime_view(view),
            UiEvent::RuntimeEvent(event) => model.apply_runtime_event(event),
            UiEvent::BridgeError(error) => model.record_runtime_error(error),
            UiEvent::UpdateSnapshot(snapshot) => model.updates = snapshot,
            UiEvent::InputRecordingReady(ready) => self.recording = ready,
            UiEvent::InputLockStatus(locked) => model.input_locked = locked,
            UiEvent::InputSharingSnapshot(snapshot) => model.input_sharing = *snapshot,
            UiEvent::PermissionSnapshot(snapshot) => model.apply_permission_snapshot(snapshot),
            UiEvent::ClipboardDelivery { peer_id, phase } => {
                model.clipboard_delivery.insert(peer_id, phase);
            }
            UiEvent::ClipboardStatus {
                enabled,
                last_mime,
                last_len,
                last_sequence,
                error,
            } => model.apply_clipboard_status(enabled, last_mime, last_len, last_sequence, error),
            UiEvent::ClipboardHistory(items) => model.clipboard_history = items,
            UiEvent::FileTransferSettings(settings) => self.file_settings = settings,
            UiEvent::TransferCacheStatus { snapshot, message } => {
                self.cache = snapshot;
                self.cache_message = message;
            }
            UiEvent::TransferStatus {
                transfer_id,
                name,
                detail,
                state,
                transferred_bytes,
                total_bytes,
                direction,
            } => {
                if state == crate::model::TransferState::Delivering {
                    self.file_paste_delivering.insert(transfer_id.to_string());
                }
                let item = TransferItem {
                    id: transfer_id.to_string(),
                    name,
                    detail,
                    state,
                    transferred_bytes,
                    total_bytes,
                    direction,
                };
                model.apply_transfer_status(item.clone());
                // Propagate the accepted state, never a rejected late producer event.
                let item = model
                    .transfer_queue
                    .iter()
                    .find(|t| t.id == item.id)
                    .cloned()
                    .unwrap_or(item);
                // While a paste waits for its cache, show the actual underlying
                // receive state on its row. Local delivery owns all later phases.
                if matches!(direction, crate::model::TransferDirection::Receive) {
                    let pending = self
                        .file_paste_sources
                        .iter()
                        .filter(|(_, source)| **source == item.id)
                        .filter_map(|(id, _)| model.transfer_queue.iter().find(|t| &t.id == id))
                        .filter(|t| {
                            !self.file_paste_acquiring.contains(&t.id)
                                && !t.state.is_terminal()
                                && t.state != crate::model::TransferState::Delivering
                        })
                        .map(|t| t.id.clone())
                        .collect::<Vec<_>>();
                    for id in pending {
                        let mut delivery = item.clone();
                        delivery.id = id;
                        if delivery.state == crate::model::TransferState::Completed {
                            delivery.state = crate::model::TransferState::Verifying;
                            delivery.detail = "文件已校验，等待保存到原目录…".into();
                        }
                        model.apply_transfer_status(delivery);
                    }
                }
            }
            UiEvent::RemoteControlStatus {
                peer_id,
                active,
                display,
                error,
            } => model.apply_remote_control_status(
                peer_id.map(|id| id.to_string()),
                active,
                display,
                error,
            ),
            UiEvent::InputStatus { error } => model.apply_input_status(error),
            _ => {}
        }
    }
}
