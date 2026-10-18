use super::{Engine, EngineError, failure};

#[uniffi::export]
impl Engine {
    pub fn mouse_settings_json(&self) -> Result<String, EngineError> {
        self.active()?;
        deskfuse_engine::mouse_settings::global()
            .map_err(failure)?
            .settings_json()
            .map_err(failure)
    }
    pub fn save_mouse_settings_json(&self, json: String) -> Result<(), EngineError> {
        self.active()?;
        let author = self.state.lock().map_err(failure)?.model.local.id.clone();
        deskfuse_engine::mouse_settings::global()
            .map_err(failure)?
            .save(&json, &author)
            .map_err(failure)
    }
}
