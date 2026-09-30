use super::{Engine, EngineError, failure};
#[uniffi::export]
impl Engine {
    pub fn clipboard_thumbnail(
        &self,
        id: u64,
        max_pixel_size: u32,
    ) -> Result<Vec<u8>, EngineError> {
        self.active()?;
        deskfuse_engine::clipboard_library::global()
            .map_err(failure)?
            .thumbnail(id, max_pixel_size)
            .map_err(failure)
    }

    pub fn library_query(&self, request: String) -> Result<String, EngineError> {
        self.active()?;
        let response = deskfuse_engine::clipboard_library::global()
            .map_err(failure)?
            .query(&request)
            .map_err(failure)?;
        if serde_json::from_str::<serde_json::Value>(&request).map_err(failure)?["op"] == "state" {
            let mut value: serde_json::Value = serde_json::from_str(&response).map_err(failure)?;
            let state = self.state.lock().map_err(failure)?;
            let devices = value["devices"]
                .as_array_mut()
                .ok_or_else(|| failure("设备列表格式无效"))?;
            for device in std::iter::once(&state.model.local).chain(state.model.peers.iter()) {
                if let Some(found) = devices
                    .iter_mut()
                    .find(|v| v["id"].as_str() == Some(device.id.as_str()))
                {
                    found["name"] = serde_json::json!(device.name);
                } else {
                    devices.push(serde_json::json!({"id":device.id,"name":device.name}));
                }
            }
            return serde_json::to_string(&value).map_err(failure);
        }
        Ok(response)
    }
    pub fn library_mutate(&self, request: String) -> Result<String, EngineError> {
        self.active()?;
        let value: serde_json::Value = serde_json::from_str(&request).map_err(failure)?;
        if let Some(shortcut) = value.get("settings").and_then(|s| s.get("shortcut")) {
            let key: deskfuse_engine::clipboard_library::Shortcut =
                serde_json::from_value(shortcut.clone()).map_err(failure)?;
            if key.enabled {
                if key.key_code > 127
                    || key.modifiers & 0x1c0000 == 0
                    || key.modifiers & !0x1e0000 != 0
                {
                    return Err(failure("唤起快捷键需包含 Command、Option 或 Control"));
                }
                if (key.key_code == 2 && key.modifiers == 0x120000)
                    || (key.key_code == 53 && key.modifiers == 0x1c0000)
                {
                    return Err(failure("快捷键与 DeskFuse 固定操作冲突"));
                }
                let state = self.state.lock().map_err(failure)?;
                let shortcuts = &state.model.input_sharing.settings.shortcuts;
                if [
                    &shortcuts.left,
                    &shortcuts.right,
                    &shortcuts.up,
                    &shortcuts.down,
                    &shortcuts.lock,
                ]
                .iter()
                .any(|s| s.key_code == key.key_code && s.modifiers == key.modifiers)
                {
                    return Err(failure("快捷键与键鼠控制冲突"));
                }
                drop(state);
                if self
                    .workflows
                    .list()
                    .map_err(failure)?
                    .iter()
                    .filter(|w| w.enabled)
                    .any(|w| {
                        w.shortcut.as_ref().is_some_and(|s| {
                            s.key_code == key.key_code && s.modifiers == key.modifiers
                        })
                    })
                {
                    return Err(failure("快捷键与工作流冲突"));
                }
            }
        }
        deskfuse_engine::clipboard_library::global()
            .map_err(failure)?
            .mutate(&request)
            .map_err(failure)
    }
}
