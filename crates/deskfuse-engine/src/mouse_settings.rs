//! Small, versioned macOS mouse configuration replicated over authenticated sessions.
//! A Lamport revision plus author tie-break converges without trusting wall clocks.
use deskfuse_store::SQLiteStore;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex, OnceLock},
};

pub const MIME: &str = "application/x-deskfuse-macos-mouse-settings+json";
const MAX_FRAME: usize = 16 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub enabled: bool,
    pub bindings: Vec<Binding>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub button: u8,
    pub modifiers: u64,
    pub action: Action,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Disabled,
    Mouse { button: u8 },
    Shortcut { key_code: u16, modifiers: u64 },
}
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Revision {
    counter: u64,
    author: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u8,
    platform: String,
    revision: Revision,
    settings: Settings,
}
impl Default for Document {
    fn default() -> Self {
        Self {
            version: 1,
            platform: "macos".into(),
            revision: Revision::default(),
            settings: Settings::default(),
        }
    }
}
impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        const MODIFIERS: u64 = 0x1e0000; // Shift, Control, Option, Command only.
        if self.bindings.len() > 64 {
            return Err("鼠标映射最多支持 64 项".into());
        }
        let mut keys = BTreeSet::new();
        for b in &self.bindings {
            let native = matches!(b.button, 195 | 196 | 208..=223);
            if (b.button > 31 && !native)
                || (native && b.modifiers != 0)
                || b.modifiers & !MODIFIERS != 0
                || !keys.insert((b.button, b.modifiers))
            {
                return Err("鼠标按钮或修饰键无效，或映射重复".into());
            }
            match b.action {
                Action::Mouse { button } if button > 31 || native => {
                    return Err("目标鼠标按钮无效".into());
                }
                Action::Disabled if matches!(b.button, 208..=223) => {
                    return Err("G502 实体键监听只能附加快捷键，不能停用鼠标原有功能".into());
                }
                Action::Shortcut {
                    key_code,
                    modifiers,
                } if key_code > 127 || modifiers & !MODIFIERS != 0 => {
                    return Err("快捷键无效".into());
                }
                _ => {}
            }
        }
        Ok(())
    }
}
impl Document {
    fn validate(&self) -> Result<(), String> {
        if self.version != 1
            || self.platform != "macos"
            || self.revision.counter == u64::MAX
            || !(self.revision.author.is_empty() && self.revision.counter == 0
                || self.revision.author.len() == 64
                    && self.revision.author.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err("不支持的鼠标配置版本或来源".into());
        }
        self.settings.validate()
    }
}

pub struct Catalog {
    store: SQLiteStore,
    document: Mutex<Document>,
}
static GLOBAL: OnceLock<Arc<Catalog>> = OnceLock::new();
static INITIALIZE: Mutex<()> = Mutex::new(());
pub fn global() -> Result<Arc<Catalog>, String> {
    if let Some(value) = GLOBAL.get() {
        return Ok(value.clone());
    }
    let _guard = INITIALIZE.lock().map_err(|e| e.to_string())?;
    if let Some(value) = GLOBAL.get() {
        return Ok(value.clone());
    }
    let value = Arc::new(Catalog::open(
        SQLiteStore::new().map_err(|e| e.to_string())?,
    )?);
    let _ = GLOBAL.set(value.clone());
    Ok(value)
}
impl Catalog {
    fn open(store: SQLiteStore) -> Result<Self, String> {
        let document: Document = match store.mouse_settings_json().map_err(|e| e.to_string())? {
            Some(json) => serde_json::from_str(&json).map_err(|e| e.to_string())?,
            None => Document::default(),
        };
        document.validate()?;
        Ok(Self {
            store,
            document: Mutex::new(document),
        })
    }
    pub fn settings_json(&self) -> Result<String, String> {
        serde_json::to_string(&self.document.lock().map_err(|e| e.to_string())?.settings)
            .map_err(|e| e.to_string())
    }
    pub fn save(&self, json: &str, author: &str) -> Result<(), String> {
        if json.len() > MAX_FRAME {
            return Err("鼠标配置过大".into());
        }
        let settings: Settings = serde_json::from_str(json).map_err(|e| e.to_string())?;
        settings.validate()?;
        let mut current = self.document.lock().map_err(|e| e.to_string())?;
        if current.settings == settings {
            return Ok(());
        }
        let mut next = current.clone();
        next.settings = settings;
        next.revision = Revision {
            counter: current
                .revision
                .counter
                .checked_add(1)
                .ok_or("鼠标配置版本溢出")?,
            author: author.into(),
        };
        next.validate()?;
        self.persist(&next)?;
        *current = next;
        Ok(())
    }
    fn persist(&self, document: &Document) -> Result<(), String> {
        self.store
            .set_mouse_settings_json(&serde_json::to_string(document).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())
    }
    pub fn frame(&self) -> Result<Vec<u8>, String> {
        serde_json::to_vec(&*self.document.lock().map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())
    }
    pub fn receive(&self, bytes: &[u8]) -> Result<bool, String> {
        if bytes.len() > MAX_FRAME {
            return Err("鼠标同步配置过大".into());
        }
        let mut incoming: Document = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        incoming.validate()?;
        let mut current = self.document.lock().map_err(|e| e.to_string())?;
        if incoming.revision <= current.revision {
            return Ok(false);
        }
        // Bindings sync across devices; the master switch is a local user choice.
        incoming.settings.enabled = current.settings.enabled;
        self.persist(&incoming)?;
        *current = incoming;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn catalog() -> Catalog {
        Catalog::open(SQLiteStore::open_in_memory().unwrap()).unwrap()
    }
    fn settings(button: u8) -> String {
        serde_json::to_string(&Settings {
            enabled: true,
            bindings: vec![Binding {
                button,
                modifiers: 0,
                action: Action::Shortcut {
                    key_code: 123,
                    modifiers: 0x40000,
                },
            }],
        })
        .unwrap()
    }
    #[test]
    fn concurrent_edits_converge_and_old_frames_do_not_roll_back() {
        let a = catalog();
        let b = catalog();
        a.save(&settings(3), &"a".repeat(64)).unwrap();
        b.save(&settings(4), &"b".repeat(64)).unwrap();
        let old = a.frame().unwrap();
        a.receive(&b.frame().unwrap()).unwrap();
        b.receive(&old).unwrap();
        assert_eq!(a.frame().unwrap(), b.frame().unwrap());
        a.save(&settings(5), &"a".repeat(64)).unwrap();
        b.receive(&a.frame().unwrap()).unwrap();
        assert_eq!(a.frame().unwrap(), b.frame().unwrap());
        assert!(!b.receive(&old).unwrap());
    }
    #[test]
    fn remote_bindings_never_change_the_local_master_switch() {
        for enabled in [false, true] {
            let local = catalog();
            let remote = catalog();
            let mut local_settings: Settings = serde_json::from_str(&settings(3)).unwrap();
            local_settings.enabled = enabled;
            local
                .save(
                    &serde_json::to_string(&local_settings).unwrap(),
                    &"a".repeat(64),
                )
                .unwrap();
            let mut remote_settings: Settings = serde_json::from_str(&settings(4)).unwrap();
            remote_settings.enabled = !enabled;
            remote
                .save(
                    &serde_json::to_string(&remote_settings).unwrap(),
                    &"b".repeat(64),
                )
                .unwrap();
            assert!(local.receive(&remote.frame().unwrap()).unwrap());
            let loaded: Settings = serde_json::from_str(&local.settings_json().unwrap()).unwrap();
            assert_eq!(loaded.enabled, enabled);
            assert_eq!(loaded.bindings, remote_settings.bindings);
            let reopened = Catalog::open(local.store.clone()).unwrap();
            let persisted: Settings =
                serde_json::from_str(&reopened.settings_json().unwrap()).unwrap();
            assert_eq!(persisted.enabled, enabled);
        }
    }
    #[test]
    fn reject_wrong_platform_invalid_keys_and_oversize() {
        let c = catalog();
        assert!(c.save(&settings(32), &"a".repeat(64)).is_err());
        assert!(c.receive(&vec![0; MAX_FRAME + 1]).is_err());
        let d = Document {
            platform: "windows".into(),
            ..Document::default()
        };
        assert!(c.receive(&serde_json::to_vec(&d).unwrap()).is_err());
        assert_eq!(
            c.settings_json().unwrap(),
            serde_json::to_string(&Settings::default()).unwrap()
        );
    }
    #[test]
    fn native_cids_sync_but_are_never_quartz_output_buttons() {
        let a = catalog();
        let b = catalog();
        for cid in [195, 196] {
            a.save(&settings(cid), &"a".repeat(64)).unwrap();
            b.receive(&a.frame().unwrap()).unwrap();
            let source: Settings = serde_json::from_str(&a.settings_json().unwrap()).unwrap();
            let received: Settings = serde_json::from_str(&b.settings_json().unwrap()).unwrap();
            assert_eq!(source.bindings, received.bindings);
            assert!(!received.enabled);
        }
        for cid in [32, 194, 197, 255] {
            assert!(a.save(&settings(cid), &"a".repeat(64)).is_err());
        }
        let mut value: Settings = serde_json::from_str(&settings(195)).unwrap();
        value.bindings[0].modifiers = 0x40000;
        assert!(value.validate().is_err());
        value.bindings[0].modifiers = 0;
        value.bindings[0].action = Action::Mouse { button: 2 };
        assert!(value.validate().is_err());
        value.bindings[0].button = 3;
        value.bindings[0].action = Action::Mouse { button: 195 };
        assert!(value.validate().is_err());
    }
    #[test]
    fn g502_physical_keys_sync_without_aliasing_quartz_or_mx_buttons() {
        let a = catalog();
        let b = catalog();
        for button in 208..=223 {
            a.save(&settings(button), &"a".repeat(64)).unwrap();
            b.receive(&a.frame().unwrap()).unwrap();
            let source: Settings = serde_json::from_str(&a.settings_json().unwrap()).unwrap();
            let received: Settings = serde_json::from_str(&b.settings_json().unwrap()).unwrap();
            assert_eq!(source.bindings, received.bindings);
            assert!(!received.enabled);
        }
        for button in [207, 224] {
            assert!(a.save(&settings(button), &"a".repeat(64)).is_err());
        }
        let mut value: Settings = serde_json::from_str(&settings(210)).unwrap();
        value.bindings[0].action = Action::Disabled;
        assert!(value.validate().is_err());
        value.bindings[0].action = Action::Mouse { button: 2 };
        assert!(value.validate().is_err());
        value.bindings[0].action = Action::Shortcut {
            key_code: 124,
            modifiers: 0x40000,
        };
        value.bindings[0].modifiers = 0x40000;
        assert!(value.validate().is_err());
    }
    #[test]
    fn reset_is_versioned_and_survives_reopen() {
        let c = catalog();
        c.save(&settings(3), &"a".repeat(64)).unwrap();
        c.save(
            &serde_json::to_string(&Settings::default()).unwrap(),
            &"a".repeat(64),
        )
        .unwrap();
        let reopened = Catalog::open(c.store.clone()).unwrap();
        assert_eq!(c.frame().unwrap(), reopened.frame().unwrap());
        assert_eq!(reopened.document.lock().unwrap().revision.counter, 2);
    }
}
