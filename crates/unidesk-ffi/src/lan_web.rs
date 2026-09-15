use super::{Engine, EngineError, failure};
#[uniffi::export]
impl Engine {
    pub fn lan_web_state(&self) -> Result<String, EngineError> {
        self.active()?;
        self.lan_web.state().map_err(failure)
    }
    pub fn configure_lan_web(
        &self,
        enabled: bool,
        interface_name: Option<String>,
    ) -> Result<String, EngineError> {
        self.active()?;
        self.lan_web
            .configure(enabled, interface_name)
            .map_err(failure)
    }
}
