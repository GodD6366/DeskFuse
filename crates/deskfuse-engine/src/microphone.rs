//! Opt-in microphone policy and session ownership. Native code reports Core Audio
//! observations and moves bounded Roc packets; it does not own authorization.
use crate::diagnostics::{DiagnosticLevel, Diagnostics};
use deskfuse_core::{DeviceId, TrustState};
use deskfuse_runtime::RuntimeHandle;
use deskfuse_store::SQLiteStore;
use deskfuse_transport::{QuinnTransport, microphone::MicrophoneSession};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};
static SERVICE: OnceLock<Mutex<Option<Arc<Service>>>> = OnceLock::new();

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub enabled: bool,
    /// Empty means this Mac supplies its physical microphone.
    pub source: String,
    pub allowed: Vec<String>,
    /// Empty means the current physical system default input.
    pub input_uid: String,
    pub shortcut_key: u16,
    pub shortcut_modifiers: u64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            source: String::new(),
            allowed: vec![],
            input_uid: String::new(),
            shortcut_key: 63,
            shortcut_modifiers: 0,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct NativeState {
    pub driver: bool,
    pub readers: bool,
    pub streaming: bool,
    pub locked: bool,
    pub permission: String,
    pub level: f32,
    pub error: String,
}
#[derive(Serialize)]
pub struct Snapshot {
    pub settings: Settings,
    pub phase: String,
    pub detail: String,
    pub session_id: u64,
    pub role: String,
    pub peer: String,
    pub source_name: String,
    pub native: NativeState,
}
struct Active {
    id: u64,
    sender: bool,
    channel: Arc<MicrophoneSession>,
}
struct State {
    settings: Settings,
    native: NativeState,
    native_at: Instant,
    active: Option<Active>,
    opening: bool,
    generation: u64,
    retry_at: Instant,
    detail: String,
    path: PathBuf,
    shortcut_applied: Option<(bool, u16, u64)>,
    shortcut_at: Instant,
}
pub struct Service {
    transport: QuinnTransport,
    runtime: RuntimeHandle,
    diagnostics: Diagnostics,
    state: Mutex<State>,
}
pub fn install(transport: QuinnTransport, runtime: RuntimeHandle, diagnostics: Diagnostics) {
    let path = SQLiteStore::default_path()
        .unwrap_or_else(|_| PathBuf::from("/nonexistent/uniDesk"))
        .with_file_name("microphone.json");
    let settings = std::fs::read(&path)
        .ok()
        .filter(|b| b.len() <= 16384)
        .and_then(|b| serde_json::from_slice::<Settings>(&b).ok())
        .filter(|s| validate_settings(s).is_ok())
        .unwrap_or_default();
    let service = Arc::new(Service {
        transport,
        runtime,
        diagnostics,
        state: Mutex::new(State {
            settings,
            native: NativeState {
                locked: true,
                ..Default::default()
            },
            native_at: Instant::now(),
            active: None,
            opening: false,
            generation: 1,
            retry_at: Instant::now(),
            detail: String::new(),
            path,
            shortcut_applied: None,
            shortcut_at: Instant::now(),
        }),
    });
    *SERVICE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some(service.clone());
    let weak = Arc::downgrade(&service);
    tokio::spawn(async move {
        let mut timer = tokio::time::interval(Duration::from_millis(100));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            timer.tick().await;
            let Some(service) = weak.upgrade() else { break };
            service.tick().await;
        }
    });
}
pub fn service() -> Result<Arc<Service>, String> {
    SERVICE
        .get()
        .and_then(|s| s.lock().ok()?.clone())
        .ok_or_else(|| "麦克风服务尚未就绪".into())
}
fn validate_settings(s: &Settings) -> Result<(), String> {
    if s.allowed.len() > 64
        || s.input_uid.len() > 512
        || (!s.source.is_empty() && s.source.parse::<DeviceId>().is_err())
        || s.allowed.iter().any(|p| p.parse::<DeviceId>().is_err())
    {
        return Err("麦克风设置无效".into());
    }
    let modifier_key = matches!(s.shortcut_key, 54..=56 | 58..=63);
    if (s.shortcut_key == 53 && s.shortcut_modifiers & 0x1c0000 == 0x1c0000)
        || s.shortcut_key > 127
        || s.shortcut_key == 57 // Caps Lock is a toggle, not a held key.
        || s.shortcut_modifiers & !0x1e0000 != 0
        || (modifier_key && s.shortcut_modifiers != 0)
    {
        return Err("请选择单个按键或组合键；Caps Lock 和紧急收回快捷键不可用".into());
    }
    Ok(())
}
impl Service {
    fn authorized(&self, peer: DeviceId) -> bool {
        let view = self.runtime.snapshot_view();
        view.device_network
            .allows(view.snapshot.local_device_id, peer)
            && !view.device_network.blocked_peer_ids.contains(&peer)
            && view
                .snapshot
                .peers
                .iter()
                .any(|p| p.id == peer && p.trust == TrustState::Trusted)
    }
    pub fn configure(&self, json: &str) -> Result<(), String> {
        if json.len() > 16384 {
            return Err("麦克风设置超限".into());
        }
        let settings: Settings = serde_json::from_str(json).map_err(|_| "麦克风设置格式无效")?;
        validate_settings(&settings)?;
        let mut state = self.state.lock().map_err(|_| "麦克风状态不可用")?;
        let bytes = serde_json::to_vec_pretty(&settings).map_err(|e| e.to_string())?;
        let tmp = state.path.with_extension("json.tmp");
        std::fs::write(&tmp, bytes)
            .and_then(|_| std::fs::rename(&tmp, &state.path))
            .map_err(|e| format!("麦克风设置保存失败：{e}"))?;
        if let Some(active) = state.active.take() {
            active.channel.close();
        }
        state.settings = settings;
        state.generation += 1;
        state.opening = false;
        state.retry_at = Instant::now();
        state.detail.clear();
        Ok(())
    }
    pub fn report(&self, native: NativeState) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.native = NativeState {
            level: if native.level.is_finite() {
                native.level.clamp(0.0, 1.0)
            } else {
                0.0
            },
            error: native.error.chars().take(256).collect(),
            ..native
        };
        state.native_at = Instant::now();
    }
    pub fn snapshot(&self) -> Snapshot {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let settings = &state.settings;
        let source_name = if settings.source.is_empty() {
            "此 Mac".to_string()
        } else {
            self.runtime
                .snapshot_view()
                .snapshot
                .peers
                .iter()
                .find(|p| p.id.to_string() == settings.source)
                .map(|p| p.display_name.clone())
                .unwrap_or_else(|| "离线设备".into())
        };
        let (phase, detail) = if !settings.enabled {
            ("disabled", "功能已关闭".into())
        } else if state.native.locked {
            ("standby", "锁屏期间已停止共享".into())
        } else if !state.native.error.is_empty() {
            ("failed", state.native.error.clone())
        } else if !settings.source.is_empty() && !state.native.driver {
            ("not_installed", "请先安装 uniDesk 麦克风驱动".into())
        } else if settings.source.is_empty()
            && state.active.is_some()
            && state.native.permission != "authorized"
        {
            ("permission", "等待麦克风授权".into())
        } else if state.active.is_some() && state.native.streaming {
            ("streaming", "正在传输".into())
        } else if state.active.is_some() {
            ("waiting", "已连接，等待来源音频或麦克风授权".into())
        } else if !state.detail.is_empty() && state.native.readers {
            ("waiting", state.detail.clone())
        } else {
            (
                "standby",
                if settings.source.is_empty() {
                    "等待允许的设备读取麦克风".into()
                } else {
                    "待机，等待语音软件读取虚拟麦克风".into()
                },
            )
        };
        Snapshot {
            settings: settings.clone(),
            phase: phase.into(),
            detail,
            source_name,
            native: state.native.clone(),
            session_id: state.active.as_ref().map_or(0, |a| a.id),
            role: state
                .active
                .as_ref()
                .map_or("", |a| if a.sender { "sender" } else { "receiver" })
                .into(),
            peer: state
                .active
                .as_ref()
                .map_or_else(String::new, |a| a.channel.peer.to_string()),
        }
    }
    pub fn channel(&self) -> Option<(u64, Arc<MicrophoneSession>)> {
        self.state
            .lock()
            .ok()?
            .active
            .as_ref()
            .map(|a| (a.id, a.channel.clone()))
    }
    pub fn stop(&self) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(a) = s.active.take() {
            a.channel.close();
        }
        s.generation += 1;
        s.opening = false;
        s.native.locked = true;
    }
    pub fn fail(&self, id: u64, detail: String) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.active.as_ref().is_some_and(|a| a.id == id) {
            self.diagnostics.record(
                DiagnosticLevel::Warning,
                "microphone",
                "session_failed",
                Some(format!("session={id} reason={detail}")),
                Some("microphone_session_failed".into()),
            );
            if let Some(a) = s.active.take() {
                a.channel.close();
            }
            s.detail = detail.chars().take(256).collect();
            s.retry_at = Instant::now() + Duration::from_secs(2);
        }
    }
    async fn tick(self: &Arc<Self>) {
        // A native heartbeat is required even while permission UI is shown.
        let opening = {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let ready = s.settings.enabled
                && !s.native.locked
                && s.native_at.elapsed() < Duration::from_secs(2)
                && s.native.error.is_empty();
            let valid = s.active.as_ref().is_none_or(|a| {
                ready
                    && self.authorized(a.channel.peer)
                    && !a.channel.is_closed()
                    && if a.sender {
                        s.settings.source.is_empty()
                            && s.settings.allowed.contains(&a.channel.peer.to_string())
                    } else {
                        s.native.driver
                            && s.native.readers
                            && s.settings.source == a.channel.peer.to_string()
                    }
            });
            if !valid {
                if let Some(a) = s.active.take() {
                    a.channel.close();
                }
                s.generation += 1;
                s.retry_at = Instant::now() + Duration::from_millis(500);
            }
            let shortcut = (
                s.settings.enabled
                    && !s.native.locked
                    && s.native_at.elapsed() < Duration::from_secs(2),
                s.settings.shortcut_key,
                s.settings.shortcut_modifiers,
            );
            if (s.shortcut_applied != Some(shortcut)
                || s.shortcut_at.elapsed() >= Duration::from_secs(1))
                && self.runtime.input_service_control().is_some_and(|c| {
                    c.set_microphone_shortcut(shortcut.0, shortcut.1, shortcut.2)
                        .is_ok()
                })
            {
                s.shortcut_applied = Some(shortcut);
                s.shortcut_at = Instant::now();
            }
            if ready
                && s.native.driver
                && s.native.readers
                && !s.settings.source.is_empty()
                && s.active.is_none()
                && !s.opening
                && Instant::now() >= s.retry_at
            {
                match s.settings.source.parse::<DeviceId>() {
                    Ok(peer) if self.authorized(peer) => {
                        s.opening = true;
                        Some((peer, s.generation))
                    }
                    _ => {
                        s.detail = "来源设备未配对或已被阻止".into();
                        None
                    }
                }
            } else {
                None
            }
        };
        if let Some((peer, generation)) = opening {
            let weak = Arc::downgrade(self);
            let transport = self.transport.clone();
            tokio::spawn(async move {
                let result = transport.open_microphone(peer).await;
                let Some(service) = weak.upgrade() else {
                    if let Ok(c) = result {
                        c.close();
                    }
                    return;
                };
                let mut s = service.state.lock().unwrap_or_else(|e| e.into_inner());
                if s.generation != generation {
                    if let Ok(c) = result {
                        c.close();
                    }
                    return;
                }
                s.opening = false;
                s.retry_at = Instant::now() + Duration::from_secs(2);
                match result {
                    Ok(c)
                        if s.settings.enabled
                            && s.native.readers
                            && !s.native.locked
                            && service.authorized(peer) =>
                    {
                        s.generation += 1;
                        let id = s.generation;
                        s.active = Some(Active {
                            id,
                            sender: false,
                            channel: c,
                        });
                        s.detail.clear();
                    }
                    Ok(c) => c.close(),
                    Err(e) => {
                        if s.detail != e {
                            service.diagnostics.record(
                                DiagnosticLevel::Warning,
                                "microphone",
                                "open_failed",
                                Some(format!("peer={peer} reason={e}")),
                                Some("microphone_open_failed".into()),
                            );
                        }
                        s.detail = e;
                    }
                }
            });
        }
        while let Some(pending) = self.transport.take_microphone() {
            let (status, generation) = {
                let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
                (
                    if !s.settings.enabled || !s.settings.source.is_empty() {
                        2
                    } else if !self.authorized(pending.peer)
                        || !s.settings.allowed.contains(&pending.peer.to_string())
                    {
                        3
                    } else if s.native.locked
                        || s.native_at.elapsed() >= Duration::from_secs(2)
                        || !s.native.error.is_empty()
                    {
                        5
                    } else if s.active.is_some() {
                        4
                    } else {
                        1
                    },
                    s.generation,
                )
            };
            if let Some(c) = pending.respond(status).await {
                let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
                if s.generation == generation && s.active.is_none() {
                    s.generation += 1;
                    let id = s.generation;
                    s.active = Some(Active {
                        id,
                        sender: true,
                        channel: c,
                    });
                    s.detail.clear();
                } else {
                    c.close();
                }
            }
        }
    }
}
