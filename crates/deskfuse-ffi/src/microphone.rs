use super::{Engine, EngineError, failure};
use deskfuse_engine::microphone::{NativeState, service};
use std::sync::Arc;

#[derive(uniffi::Record)]
pub struct MicrophoneNativeState {
    pub driver: bool,
    pub readers: bool,
    pub streaming: bool,
    pub locked: bool,
    pub permission: String,
    pub level: f32,
    pub error: String,
}

#[derive(uniffi::Object)]
pub struct MicrophoneChannel {
    id: u64,
    session: Arc<deskfuse_transport::microphone::MicrophoneSession>,
}
#[uniffi::export]
impl MicrophoneChannel {
    pub fn session_id(&self) -> u64 {
        self.id
    }
    pub fn send(&self, bytes: Vec<u8>) -> Result<(), EngineError> {
        self.session.send(bytes).map_err(failure)
    }
    pub fn receive(&self) -> Result<Option<Vec<u8>>, EngineError> {
        self.session.receive().map_err(failure)
    }
    pub fn close(&self) {
        self.session.close();
    }
}
#[uniffi::export]
impl Engine {
    pub fn microphone_snapshot(&self) -> Result<String, EngineError> {
        self.active()?;
        serde_json::to_string(&service().map_err(failure)?.snapshot())
            .map_err(|e| failure(e.to_string()))
    }
    pub fn configure_microphone(&self, settings_json: String) -> Result<(), EngineError> {
        self.active()?;
        service()
            .map_err(failure)?
            .configure(&settings_json)
            .map_err(failure)
    }
    pub fn report_microphone_native(
        &self,
        state: MicrophoneNativeState,
    ) -> Result<(), EngineError> {
        self.active()?;
        service().map_err(failure)?.report(NativeState {
            driver: state.driver,
            readers: state.readers,
            streaming: state.streaming,
            locked: state.locked,
            permission: state.permission,
            level: state.level,
            error: state.error,
        });
        Ok(())
    }
    pub fn microphone_channel(&self) -> Result<Option<Arc<MicrophoneChannel>>, EngineError> {
        self.active()?;
        Ok(service()
            .map_err(failure)?
            .channel()
            .map(|(id, session)| Arc::new(MicrophoneChannel { id, session })))
    }
    pub fn fail_microphone_session(&self, id: u64, detail: String) {
        if let Ok(service) = service() {
            service.fail(id, detail);
        }
    }
    pub fn stop_microphone(&self) {
        if let Ok(service) = service() {
            service.stop();
        }
    }
}
