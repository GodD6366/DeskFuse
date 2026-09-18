use super::{Engine, EngineError, device, failure};
use std::sync::Arc;
use unidesk_engine::remote_application::service;
#[derive(uniffi::Object)]
pub struct RemoteApplicationChannel {
    session: Arc<unidesk_transport::remote_application::RemoteSession>,
}
#[uniffi::export]
impl RemoteApplicationChannel {
    pub fn peer_id(&self) -> String {
        self.session.peer.to_string()
    }
    /// Compressed packets only. Never a pixel buffer and never a UI snapshot.
    pub fn send(&self, bytes: Vec<u8>) -> Result<(), EngineError> {
        self.session.send_blocking(bytes).map_err(failure)
    }
    pub fn receive(&self, wait_ms: u64) -> Result<Option<Vec<u8>>, EngineError> {
        self.session.receive(wait_ms).map_err(failure)
    }
    pub fn close(&self) {
        self.session.close();
    }
}
#[uniffi::export]
impl Engine {
    pub fn remote_application_diagnostic(&self, event: String, detail: String) {
        if event.len() <= 64 && detail.len() <= 512 {
            unidesk_platform::timing::record(
                "remote_app",
                std::time::Duration::ZERO,
                format!("event={event} {detail}"),
            );
        }
    }
    pub fn set_remote_application_hosting(&self, enabled: bool) -> Result<(), EngineError> {
        self.active()?;
        service().map_err(failure)?.enable(enabled);
        Ok(())
    }
    pub fn open_remote_application(
        &self,
        peer_id: String,
    ) -> Result<Arc<RemoteApplicationChannel>, EngineError> {
        self.active()?;
        Ok(Arc::new(RemoteApplicationChannel {
            session: service()
                .map_err(failure)?
                .open(device(&peer_id)?)
                .map_err(failure)?,
        }))
    }
    pub fn take_remote_application(
        &self,
    ) -> Result<Option<Arc<RemoteApplicationChannel>>, EngineError> {
        self.active()?;
        Ok(service()
            .map_err(failure)?
            .incoming()
            .map(|session| Arc::new(RemoteApplicationChannel { session })))
    }
    pub fn remote_application_viewer_control(&self, active: bool) -> Result<(), EngineError> {
        self.active()?;
        service()
            .map_err(failure)?
            .viewer_control(active)
            .map_err(failure)
    }
    pub fn remote_application_viewer_ready(&self) -> Result<bool, EngineError> {
        self.active()?;
        Ok(service().map_err(failure)?.viewer_ready())
    }
    pub fn remote_application_control_ready(
        &self,
        peer_id: String,
        session: String,
        generation: u64,
    ) -> Result<bool, EngineError> {
        self.active()?;
        Ok(service()
            .map_err(failure)?
            .control_ready(device(&peer_id)?, &session, generation))
    }
    pub fn remote_application_input(
        &self,
        peer_id: String,
        packet_json: String,
    ) -> Result<(), EngineError> {
        self.active()?;
        service()
            .map_err(failure)?
            .input(device(&peer_id)?, &packet_json)
            .map_err(failure)
    }
    pub fn remote_application_control_problem(
        &self,
        peer_id: String,
        session: String,
        generation: u64,
    ) -> Result<String, EngineError> {
        self.active()?;
        Ok(service()
            .map_err(failure)?
            .control_problem(device(&peer_id)?, &session, generation))
    }
}
