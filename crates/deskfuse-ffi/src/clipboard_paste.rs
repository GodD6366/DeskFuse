//! Native panel reuses the workflow's guarded paste primitive. No clipboard data
//! crosses this API; the caller provides the applied restore's change count.
use crate::{Engine, EngineError, failure};

#[uniffi::export]
pub fn clipboard_paste_if_current(
    target_pid: i32,
    expected_change_count: i64,
) -> Result<(), EngineError> {
    deskfuse_platform_macos::workflow_paste_if_current(target_pid, expected_change_count)
        .map_err(failure)
}

#[uniffi::export]
impl Engine {
    /// Restore via the serialized engine worker, returning the clipboard version
    /// for a subsequent focus- and version-guarded native paste event.
    pub fn prepare_clipboard_paste(&self, id: u64, session: String) -> Result<i64, EngineError> {
        self.active()?;
        self.bridge
            .prepare_clipboard_paste(id, session)
            .map_err(failure)
    }
    pub fn cancel_clipboard_preparation(&self, session: String) -> Result<(), EngineError> {
        self.active()?;
        self.bridge
            .cancel_clipboard_preparation(session)
            .map_err(failure)
    }
}
