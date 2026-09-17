//! Host-assembled collaboration core. Product extensions subscribe; they are not dependencies.
use crate::{
    api::{
        CollaborationCommand, CollaborationEvent, CollaborationSnapshot, DeviceInfo,
        DisconnectReason,
    },
    clipboard_sync::ClipboardSyncService,
    error::{CollaborationError, Result},
    file_transfer::FileTransferService,
    input_coordinator::InputCoordinator,
    session::SessionManager,
    transport::CollaborationTransport,
};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::{RwLock, mpsc};
use unidesk_platform::ports::HostPorts;

/// Which collaboration workers a host process should start.
#[derive(Clone, Debug)]
pub struct CoreConfig {
    pub data_dir: PathBuf,
    pub enable_clipboard: bool,
    pub enable_file: bool,
    pub enable_input: bool,
}

impl CoreConfig {
    #[must_use]
    pub fn desktop(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            enable_clipboard: true,
            enable_file: true,
            enable_input: true,
        }
    }

    #[must_use]
    pub fn input_only(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            enable_clipboard: false,
            enable_file: false,
            enable_input: true,
        }
    }
}

/// Shared K3/K5/K6/K7 core. Hosts inject ports and transport; extensions subscribe to events.
pub struct CollaborationCore {
    config: CoreConfig,
    ports: HostPorts,
    sessions: Arc<RwLock<SessionManager>>,
    clipboard: ClipboardSyncService,
    files: FileTransferService,
    input: InputCoordinator,
    events: mpsc::UnboundedSender<CollaborationEvent>,
    event_rx: RwLock<Option<mpsc::UnboundedReceiver<CollaborationEvent>>>,
}

impl CollaborationCore {
    pub fn new(
        ports: HostPorts,
        transport: Arc<dyn CollaborationTransport>,
        config: CoreConfig,
    ) -> Self {
        let (events, event_rx) = mpsc::unbounded_channel();
        let clipboard = ClipboardSyncService::new(
            Arc::clone(&ports.clipboard_capture),
            Arc::clone(&ports.clipboard_apply),
            Arc::clone(&transport),
            events.clone(),
        );
        let files = FileTransferService::new(
            Arc::clone(&ports.file_authorization),
            transport,
            events.clone(),
        );
        let input = InputCoordinator::new(Arc::clone(&ports.input_execution), events.clone());
        Self {
            config,
            ports,
            sessions: Arc::new(RwLock::new(SessionManager::new())),
            clipboard,
            files,
            input,
            events,
            event_rx: RwLock::new(Some(event_rx)),
        }
    }

    pub fn new_desktop(
        ports: HostPorts,
        transport: Arc<dyn CollaborationTransport>,
        data_dir: impl Into<PathBuf>,
    ) -> Self {
        Self::new(ports, transport, CoreConfig::desktop(data_dir))
    }

    pub fn new_input_only(
        ports: HostPorts,
        transport: Arc<dyn CollaborationTransport>,
        data_dir: impl Into<PathBuf>,
    ) -> Self {
        Self::new(ports, transport, CoreConfig::input_only(data_dir))
    }

    pub async fn start(&self) -> Result<()> {
        if self.config.enable_clipboard {
            self.ports.clipboard_capture.start_monitoring().await?;
        }
        Ok(())
    }

    pub async fn stop(&self) -> Result<()> {
        if self.config.enable_clipboard {
            self.ports.clipboard_capture.stop_monitoring().await?;
        }
        if self.input.has_active_lease().await {
            self.input.release_lease("core stopped").await?;
        }
        Ok(())
    }

    pub async fn take_events(&self) -> Result<mpsc::UnboundedReceiver<CollaborationEvent>> {
        self.event_rx
            .write()
            .await
            .take()
            .ok_or_else(|| CollaborationError::failed("event subscriber already taken"))
    }

    pub fn subscribe(&self) -> mpsc::UnboundedSender<CollaborationEvent> {
        self.events.clone()
    }

    pub async fn snapshot(&self) -> CollaborationSnapshot {
        CollaborationSnapshot {
            devices: self.sessions.read().await.device_states(),
            active_connections: self.sessions.read().await.connections(),
            clipboard_status: self.clipboard.status().await,
            pending_transfers: self.files.snapshot().await,
            input_status: self.input.status().await,
        }
    }

    pub async fn submit(&self, command: CollaborationCommand) -> Result<()> {
        match command {
            CollaborationCommand::StartPairing { timeout } => {
                let _ = timeout;
                Ok(())
            }
            CollaborationCommand::AcceptDevice { device_id } => {
                let accepted = self.sessions.write().await.accept(device_id);
                if accepted.is_some() {
                    if let Some(session) = self.sessions.write().await.connect(device_id) {
                        let _ = self
                            .events
                            .send(CollaborationEvent::Connected { device_id, session });
                    }
                    Ok(())
                } else {
                    Err(CollaborationError::SessionNotFound(device_id))
                }
            }
            CollaborationCommand::RejectDevice { device_id } => {
                self.sessions.write().await.reject(device_id);
                Ok(())
            }
            CollaborationCommand::RemoveTrust { device_id } => {
                self.sessions.write().await.remove_trust(device_id);
                let _ = self.events.send(CollaborationEvent::Disconnected {
                    device_id,
                    reason: DisconnectReason::User,
                });
                Ok(())
            }
            CollaborationCommand::PublishClipboard { content } => {
                if !self.config.enable_clipboard {
                    return Err(CollaborationError::failed("clipboard is disabled"));
                }
                let captured = self.ports.clipboard_capture.capture().await?;
                let _ = content;
                self.clipboard.on_local_capture(captured).await
            }
            CollaborationCommand::CancelClipboard => self.clipboard.cancel_clipboard().await,
            CollaborationCommand::SendFiles { files, to } => {
                if !self.config.enable_file {
                    return Err(CollaborationError::failed("file transfer is disabled"));
                }
                self.files.send_files(files, to).await.map(|_| ())
            }
            CollaborationCommand::CancelTransfer { transfer_id } => {
                self.files.cancel(transfer_id).await
            }
            CollaborationCommand::RequestInputControl { device_id } => {
                if !self.config.enable_input {
                    return Err(CollaborationError::failed("input is disabled"));
                }
                self.input
                    .grant_lease(device_id, Duration::from_secs(10))
                    .await
            }
            CollaborationCommand::ReleaseInputControl => {
                self.input.release_lease("released by user").await
            }
        }
    }

    pub fn clipboard(&self) -> &ClipboardSyncService {
        &self.clipboard
    }

    pub fn files(&self) -> &FileTransferService {
        &self.files
    }

    pub fn input(&self) -> &InputCoordinator {
        &self.input
    }

    pub async fn discover(&self, info: DeviceInfo) {
        self.sessions.write().await.discover(info.clone());
        let _ = self
            .events
            .send(CollaborationEvent::DeviceDiscovered { device: info });
    }

    pub fn config(&self) -> &CoreConfig {
        &self.config
    }
}
