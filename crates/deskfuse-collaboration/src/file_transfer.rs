//! Shared file-transfer policy. Hosts authorize paths; collaboration owns progress and cancel.
use crate::{
    api::{CollaborationEvent, TransferState},
    error::{CollaborationError, Result},
    transport::{CollaborationTransport, OutboundMessage},
};
use deskfuse_core::DeviceId;
use deskfuse_platform::ports::{AuthorizedFile, FileAuthorizationPort};
use deskfuse_transfer::TransferId;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use tokio::sync::{RwLock, mpsc};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    pub size: u64,
    #[serde(skip)]
    pub path: PathBuf,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileManifest {
    pub files: Vec<FileEntry>,
}

impl FileManifest {
    pub fn create(files: &[AuthorizedFile]) -> Self {
        Self {
            files: files
                .iter()
                .map(|file| FileEntry {
                    name: file.name(),
                    size: file.size,
                    path: file.path.clone(),
                })
                .collect(),
        }
    }

    #[must_use]
    pub fn file_names(&self) -> Vec<String> {
        self.files.iter().map(|file| file.name.clone()).collect()
    }

    #[must_use]
    pub fn total_size(&self) -> u64 {
        self.files.iter().map(|file| file.size).sum()
    }
}

struct ActiveTransfer {
    state: TransferState,
    manifest: FileManifest,
}

/// File collaboration shared by every host.
pub struct FileTransferService {
    file_port: Arc<dyn FileAuthorizationPort>,
    transport: Arc<dyn CollaborationTransport>,
    active: Arc<RwLock<HashMap<TransferId, ActiveTransfer>>>,
    events: mpsc::UnboundedSender<CollaborationEvent>,
}

impl FileTransferService {
    pub fn new(
        file_port: Arc<dyn FileAuthorizationPort>,
        transport: Arc<dyn CollaborationTransport>,
        events: mpsc::UnboundedSender<CollaborationEvent>,
    ) -> Self {
        Self {
            file_port,
            transport,
            active: Arc::new(RwLock::new(HashMap::new())),
            events,
        }
    }

    pub async fn snapshot(&self) -> Vec<TransferState> {
        self.active
            .read()
            .await
            .values()
            .map(|transfer| transfer.state.clone())
            .collect()
    }

    pub async fn send_files(&self, files: Vec<PathBuf>, to: DeviceId) -> Result<TransferId> {
        let authorized = self.file_port.authorize_read(&files).await?;
        if authorized.is_empty() {
            return Err(CollaborationError::failed("no readable files"));
        }
        let manifest = FileManifest::create(&authorized);
        let transfer_id = TransferId::new();
        let name = manifest
            .file_names()
            .first()
            .cloned()
            .unwrap_or_else(|| "files".into());
        let state = TransferState {
            transfer_id,
            peer: to,
            name,
            progress: 0.0,
            inbound: false,
        };
        self.active.write().await.insert(
            transfer_id,
            ActiveTransfer {
                state: state.clone(),
                manifest: manifest.clone(),
            },
        );
        let _ = self
            .events
            .send(CollaborationEvent::TransferStarted { transfer_id });
        self.transport
            .send(
                to,
                OutboundMessage::FileManifest {
                    transfer_id,
                    manifest: manifest.clone(),
                },
            )
            .await?;
        self.transmit_chunks(transfer_id, to, manifest).await?;
        Ok(transfer_id)
    }

    pub async fn receive_file(
        &self,
        from: DeviceId,
        transfer_id: TransferId,
        manifest: FileManifest,
    ) -> Result<PathBuf> {
        let target_dir = self
            .file_port
            .authorize_write(&manifest.file_names(), manifest.total_size())
            .await?;
        let name = manifest
            .file_names()
            .first()
            .cloned()
            .unwrap_or_else(|| "files".into());
        self.active.write().await.insert(
            transfer_id,
            ActiveTransfer {
                state: TransferState {
                    transfer_id,
                    peer: from,
                    name,
                    progress: 0.0,
                    inbound: true,
                },
                manifest,
            },
        );
        let _ = self
            .events
            .send(CollaborationEvent::TransferStarted { transfer_id });
        Ok(target_dir)
    }

    pub async fn on_chunk(&self, transfer_id: TransferId, index: u32, data: Vec<u8>) -> Result<()> {
        let progress = {
            let mut active = self.active.write().await;
            let transfer = active
                .get_mut(&transfer_id)
                .ok_or(CollaborationError::TransferNotFound(transfer_id))?;
            let total = transfer.manifest.files.len().max(1) as f64;
            let progress = ((index as f64) + 1.0) / total;
            transfer.state.progress = progress;
            progress
        };
        let _ = self.events.send(CollaborationEvent::TransferProgress {
            transfer_id,
            progress,
        });
        if progress >= 1.0 {
            let _ = self
                .events
                .send(CollaborationEvent::TransferCompleted { transfer_id });
        }
        let _ = data;
        Ok(())
    }

    pub async fn cancel(&self, transfer_id: TransferId) -> Result<()> {
        let peer = {
            let mut active = self.active.write().await;
            let transfer = active
                .remove(&transfer_id)
                .ok_or(CollaborationError::TransferNotFound(transfer_id))?;
            transfer.state.peer
        };
        self.transport
            .send(peer, OutboundMessage::CancelTransfer { transfer_id })
            .await?;
        let _ = self.events.send(CollaborationEvent::TransferFailed {
            transfer_id,
            error: "cancelled".into(),
        });
        Ok(())
    }

    async fn transmit_chunks(
        &self,
        transfer_id: TransferId,
        to: DeviceId,
        manifest: FileManifest,
    ) -> Result<()> {
        let total = manifest.files.len().max(1);
        for (index, file) in manifest.files.iter().enumerate() {
            let data = std::fs::read(&file.path)
                .map_err(|error| CollaborationError::failed(error.to_string()))?;
            self.transport
                .send(
                    to,
                    OutboundMessage::FileChunk {
                        transfer_id,
                        index: index as u32,
                        data,
                    },
                )
                .await?;
            let progress = (index as f64 + 1.0) / total as f64;
            if let Some(transfer) = self.active.write().await.get_mut(&transfer_id) {
                transfer.state.progress = progress;
            }
            let _ = self.events.send(CollaborationEvent::TransferProgress {
                transfer_id,
                progress,
            });
        }
        let _ = self
            .events
            .send(CollaborationEvent::TransferCompleted { transfer_id });
        Ok(())
    }
}
