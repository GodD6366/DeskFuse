//! Reducer for platform-independent business state. Window selection is never stored here.
use crate::{
    model::{AppModel, TransferItem},
    tokio_bridge::UiEvent,
};
use deskfuse_core::{FileTransferSettings, TransferCacheSnapshot};

#[derive(Clone)]
pub struct EngineState {
    pub revision: u64,
    pub debug_logging_enabled: bool,
    pub model: AppModel,
    pub file_settings: FileTransferSettings,
    pub cache: TransferCacheSnapshot,
    pub cache_message: Option<String>,
    pub recording: bool,
    pub input_quality: Option<deskfuse_runtime::input_service_types::InputServiceQuality>,
    /// One fallback observation per second; never fabricate past history.
    pub idle_quality_samples: std::collections::VecDeque<(u128, f64)>,
    pub file_paste_acquiring: std::collections::BTreeSet<String>,
    pub file_paste_sources: std::collections::BTreeMap<String, String>,
    pub file_paste_retryable: std::collections::BTreeSet<String>,
    pub file_paste_delivering: std::collections::BTreeSet<String>,
}
impl Default for EngineState {
    fn default() -> Self {
        Self {
            revision: 0,
            debug_logging_enabled: deskfuse_core::debug_logging::enabled(),
            model: AppModel::starting(),
            file_settings: Default::default(),
            cache: Default::default(),
            cache_message: None,
            recording: false,
            input_quality: None,
            idle_quality_samples: Default::default(),
            file_paste_acquiring: Default::default(),
            file_paste_sources: Default::default(),
            file_paste_retryable: Default::default(),
            file_paste_delivering: Default::default(),
        }
    }
}
impl EngineState {
    pub fn sample_idle_input_quality(&mut self, timestamp_ms: u128) {
        let rtt = self
            .model
            .network_graph_peers()
            .into_iter()
            .chain(self.model.discovered_peers.iter())
            .filter(|peer| peer.health == crate::model::Health::Ready)
            .flat_map(|peer| {
                peer.routes
                    .iter()
                    .filter(|route| Some(route.id) == peer.selected_route_id)
            })
            .filter_map(|route| route.rtt_ms)
            .max()
            .unwrap_or(0);
        self.idle_quality_samples
            .push_back((timestamp_ms, f64::from(rtt)));
        while self.idle_quality_samples.len() > 60 {
            self.idle_quality_samples.pop_front();
        }
        self.revision = self.revision.wrapping_add(1);
    }

    /// 状态未变时返回 false，调用方不得因此刷新 Swift 快照。
    pub fn apply(&mut self, event: UiEvent) -> bool {
        let model = &mut self.model;
        let changed = match event {
            UiEvent::ClipboardLibraryChanged(_) => false,
            UiEvent::FilePasteAcquiring {
                paste_id,
                transfer_id,
            } => {
                self.file_paste_acquiring.insert(paste_id.to_string());
                self.file_paste_sources
                    .insert(paste_id.to_string(), transfer_id.to_string());
                true
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
                true
            }
            UiEvent::DebugLoggingChanged(enabled) => {
                if self.debug_logging_enabled == enabled {
                    return false;
                }
                self.debug_logging_enabled = enabled;
                true
            }
            UiEvent::WorkflowRun(_) | UiEvent::WorkflowCatalogChanged => true,
            UiEvent::RuntimeView(view) => {
                model.apply_runtime_view(view);
                true
            }
            UiEvent::RuntimeEvent(event) => {
                model.apply_runtime_event(event);
                true
            }
            UiEvent::BridgeError(error) => {
                model.record_runtime_error(error);
                true
            }
            UiEvent::UpdateSnapshot(snapshot) => {
                model.updates = snapshot;
                true
            }
            UiEvent::InputRecordingReady(ready) => {
                if self.recording == ready {
                    return false;
                }
                self.recording = ready;
                true
            }
            UiEvent::InputLockStatus(locked) => {
                if model.input_locked == locked {
                    return false;
                }
                model.input_locked = locked;
                true
            }
            UiEvent::InputSharingSnapshot(snapshot) => {
                if model.input_sharing == *snapshot {
                    return false;
                }
                model.input_sharing = *snapshot;
                true
            }
            UiEvent::InputQualitySnapshot(quality) => {
                if self.input_quality.as_ref() == Some(&quality) {
                    return false;
                }
                self.input_quality = Some(quality);
                true
            }
            UiEvent::PermissionSnapshot(snapshot) => {
                if model.permissions == snapshot {
                    return false;
                }
                model.apply_permission_snapshot(snapshot);
                true
            }
            UiEvent::ClipboardDelivery { peer_id, phase } => {
                model.clipboard_delivery.insert(peer_id, phase);
                true
            }
            UiEvent::ClipboardStatus {
                enabled,
                last_mime,
                last_len,
                last_sequence,
                error,
            } => {
                model.apply_clipboard_status(enabled, last_mime, last_len, last_sequence, error);
                true
            }
            UiEvent::ClipboardHistory(items) => {
                model.clipboard_history = items;
                true
            }
            UiEvent::FileTransferSettings(settings) => {
                self.file_settings = settings;
                true
            }
            UiEvent::TransferCacheStatus { snapshot, message } => {
                self.cache = snapshot;
                self.cache_message = message;
                true
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
                true
            }
            UiEvent::RemoteControlStatus {
                peer_id,
                active,
                display,
                error,
            } => {
                model.apply_remote_control_status(
                    peer_id.map(|id| id.to_string()),
                    active,
                    display,
                    error,
                );
                true
            }
            UiEvent::InputStatus { error } => {
                if model.input_error == error {
                    return false;
                }
                model.apply_input_status(error);
                true
            }
            _ => true,
        };
        if changed {
            self.revision = self.revision.saturating_add(1);
        }
        changed
    }
}
