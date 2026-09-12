//! Reducer for platform-independent business state. Window selection is never stored here.
use crate::{
    model::{AppModel, TransferItem},
    tokio_bridge::UiEvent,
};
use unidesk_core::{FileTransferSettings, TransferCacheSnapshot};

#[derive(Clone)]
pub struct EngineState {
    pub revision: u64,
    pub model: AppModel,
    pub file_settings: FileTransferSettings,
    pub cache: TransferCacheSnapshot,
    pub cache_message: Option<String>,
    pub recording: bool,
}
impl Default for EngineState {
    fn default() -> Self {
        Self {
            revision: 0,
            model: AppModel::starting(),
            file_settings: Default::default(),
            cache: Default::default(),
            cache_message: None,
            recording: false,
        }
    }
}
impl EngineState {
    pub fn apply(&mut self, event: UiEvent) {
        self.revision = self.revision.saturating_add(1);
        let model = &mut self.model;
        match event {
            UiEvent::ImageWorkflowRun(_) => {}
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
            } => model.apply_transfer_status(TransferItem {
                id: transfer_id.to_string(),
                name,
                detail,
                state,
                transferred_bytes,
                total_bytes,
                direction,
            }),
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
