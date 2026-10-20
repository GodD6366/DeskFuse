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

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub enabled: bool,
    /// Empty means this Mac supplies its physical microphone.
    pub source: String,
    /// Legacy per-device allowlist. Kept for settings compatibility; trusted
    /// paired devices may use this Mac without an explicit entry.
    pub allowed: Vec<String>,
    /// Empty means the current physical system default input.
    pub input_uid: String,
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
}
impl State {
    fn ready(&self) -> bool {
        self.settings.enabled
            && !self.native.locked
            && self.native_at.elapsed() < Duration::from_secs(2)
            && self.native.error.is_empty()
    }

    fn source_status(&self, authorized: bool) -> u32 {
        if !self.settings.enabled || !self.settings.source.is_empty() {
            2
        } else if !authorized {
            3
        } else if !self.ready() {
            5
        } else if self.active.is_some() {
            4
        } else {
            1
        }
    }

    fn can_receive(&self, peer: DeviceId, authorized: bool) -> bool {
        self.ready()
            && self.native.driver
            && self.native.readers
            && self.settings.source == peer.to_string()
            && self.active.is_none()
            && authorized
    }
}

pub struct Service {
    transport: QuinnTransport,
    runtime: RuntimeHandle,
    executor: tokio::runtime::Handle,
    diagnostics: Diagnostics,
    state: Mutex<State>,
    enabled: tokio::sync::watch::Sender<bool>,
}
pub fn install(transport: QuinnTransport, runtime: RuntimeHandle, diagnostics: Diagnostics) {
    let path = SQLiteStore::default_path()
        .unwrap_or_else(|_| PathBuf::from("/nonexistent/DeskFuse"))
        .with_file_name("microphone.json");
    let settings = std::fs::read(&path)
        .ok()
        .filter(|b| b.len() <= 16384)
        .and_then(|b| serde_json::from_slice::<Settings>(&b).ok())
        .filter(|s| validate_settings(s).is_ok())
        .unwrap_or_default();
    transport.set_microphone_enabled(settings.enabled);
    transport.set_microphone_source_enabled(settings.enabled && settings.source.is_empty());
    let (enabled, mut enabled_rx) = tokio::sync::watch::channel(settings.enabled);
    let service = Arc::new(Service {
        enabled,
        transport,
        runtime,
        executor: tokio::runtime::Handle::current(),
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
            let is_enabled = *enabled_rx.borrow_and_update();
            tokio::select! {
                changed = enabled_rx.changed() => {
                    if changed.is_err() { break; }
                    continue;
                }
                _ = timer.tick(), if is_enabled => {}
            }
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
    Ok(())
}
impl Service {
    pub fn source_enabled(&self, peer: DeviceId) -> Result<bool, String> {
        if !self.authorized(peer) {
            return Ok(false);
        }
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let transport = self.transport.clone();
        self.executor.spawn(async move {
            let _ = tx.send(transport.microphone_source_enabled(peer).await);
        });
        let enabled = rx
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "麦克风状态查询超时".to_string())??;
        Ok(enabled && self.authorized(peer))
    }

    fn activate(&self, state: &mut State, sender: bool, channel: Arc<MicrophoneSession>) {
        state.generation += 1;
        let active = Active {
            id: state.generation,
            sender,
            channel,
        };
        self.diagnostics.record(
            DiagnosticLevel::Info,
            "microphone",
            "session_active",
            Some(format!(
                "session_id={} role={} peer={}",
                active.id,
                if active.sender { "sender" } else { "receiver" },
                active.channel.peer
            )),
            None,
        );
        state.active = Some(active);
        state.detail.clear();
    }

    fn deactivate(&self, state: &mut State, reason: &'static str) {
        if let Some(active) = state.active.take() {
            active.channel.close();
            self.diagnostics.record(
                DiagnosticLevel::Info,
                "microphone",
                "session_ended",
                Some(format!(
                    "session_id={} role={} peer={} reason={reason}",
                    active.id,
                    if active.sender { "sender" } else { "receiver" },
                    active.channel.peer
                )),
                None,
            );
        }
    }

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
        self.deactivate(
            &mut state,
            if settings.enabled {
                "settings_changed"
            } else {
                "disabled"
            },
        );
        self.transport.set_microphone_enabled(settings.enabled);
        self.transport
            .set_microphone_source_enabled(settings.enabled && settings.source.is_empty());
        self.enabled.send_replace(settings.enabled);
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
            ("not_installed", "请先安装 DeskFuse 麦克风驱动".into())
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
        self.deactivate(&mut s, "stopped");
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
            self.deactivate(&mut s, "native_session_failed");
            s.detail = detail.chars().take(256).collect();
            s.retry_at = Instant::now() + Duration::from_secs(2);
        }
    }
    async fn tick(self: &Arc<Self>) {
        // A native heartbeat is required even while permission UI is shown.
        let opening = {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let ready = s.ready();
            let ended = s.active.as_ref().and_then(|active| {
                if !s.settings.enabled {
                    Some("disabled")
                } else if s.native.locked {
                    Some("locked")
                } else if s.native_at.elapsed() >= Duration::from_secs(2) {
                    Some("native_heartbeat_expired")
                } else if !s.native.error.is_empty() {
                    Some("native_error")
                } else if !self.authorized(active.channel.peer) {
                    Some("authorization_revoked")
                } else if active.channel.is_closed() {
                    Some("transport_closed_or_timed_out")
                } else if active.sender {
                    if !s.settings.source.is_empty() {
                        Some("source_changed")
                    } else {
                        None
                    }
                } else if !s.native.driver {
                    Some("driver_unavailable")
                } else if !s.native.readers {
                    Some("readers_stopped")
                } else if s.settings.source != active.channel.peer.to_string() {
                    Some("source_changed")
                } else {
                    None
                }
            });
            if let Some(reason) = ended {
                self.deactivate(&mut s, reason);
                s.generation += 1;
                s.retry_at = Instant::now() + Duration::from_millis(500);
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
                    Ok(c) if s.can_receive(peer, service.authorized(peer)) => {
                        service.activate(&mut s, false, c);
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
                (s.source_status(self.authorized(pending.peer)), s.generation)
            };
            if let Some(c) = pending.respond(status).await {
                let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
                if s.generation == generation && s.source_status(self.authorized(c.peer)) == 1 {
                    self.activate(&mut s, true, c);
                } else {
                    c.close();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_state(peer: DeviceId, receiving: bool) -> State {
        State {
            settings: Settings {
                enabled: true,
                source: if receiving {
                    peer.to_string()
                } else {
                    String::new()
                },
                allowed: vec![peer.to_string()],
                ..Default::default()
            },
            native: NativeState {
                driver: true,
                readers: true,
                ..Default::default()
            },
            native_at: Instant::now(),
            active: None,
            opening: true,
            generation: 1,
            retry_at: Instant::now(),
            detail: String::new(),
            path: PathBuf::new(),
        }
    }

    #[test]
    fn microphone_source_rechecks_authorization_and_native_liveness_after_admission_wait() {
        let peer = DeviceId::from_bytes([9; 32]);
        let mut state = ready_state(peer, false);
        // Permission may still be prompting; admission authorizes a channel,
        // while native capture remains responsible for acquiring permission.
        assert_eq!(state.source_status(true), 1);
        state.native.locked = true;
        assert_eq!(state.source_status(true), 5);
        state.native.locked = false;
        state.native_at = Instant::now() - Duration::from_secs(3);
        assert_eq!(state.source_status(true), 5);
        state.native_at = Instant::now();
        state.native.error = "capture stopped".into();
        assert_eq!(state.source_status(true), 5);
        state.native.error.clear();
        assert_eq!(state.source_status(false), 3);
        state.settings.allowed.clear();
        // Trusted paired devices no longer need a legacy allowlist entry.
        assert_eq!(state.source_status(true), 1);
        state.settings.enabled = false;
        assert_eq!(state.source_status(true), 2);
    }

    #[test]
    fn microphone_receiver_rejects_late_handshake_after_reader_driver_or_heartbeat_loss() {
        let peer = DeviceId::from_bytes([9; 32]);
        let mut state = ready_state(peer, true);
        assert!(state.can_receive(peer, true));
        state.native.readers = false;
        assert!(!state.can_receive(peer, true));
        state.native.readers = true;
        state.native.driver = false;
        assert!(!state.can_receive(peer, true));
        state.native.driver = true;
        state.native_at = Instant::now() - Duration::from_secs(3);
        assert!(!state.can_receive(peer, true));
        state.native_at = Instant::now();
        state.native.error = "driver disconnected".into();
        assert!(!state.can_receive(peer, true));
        state.native.error.clear();
        assert!(!state.can_receive(peer, false));
        state.settings.source = DeviceId::from_bytes([8; 32]).to_string();
        assert!(!state.can_receive(peer, true));
    }
    #[test]
    fn legacy_shortcut_settings_are_ignored_without_losing_audio_configuration() {
        let settings: Settings = serde_json::from_str(r#"{"enabled":true,"input_uid":"physical-mic","shortcut_key":54,"shortcut_modifiers":0}"#).unwrap();
        assert!(settings.enabled);
        assert_eq!(settings.input_uid, "physical-mic");
        validate_settings(&settings).unwrap();
        let saved = serde_json::to_value(&settings).unwrap();
        assert!(saved.get("shortcut_key").is_none());
        assert!(saved.get("shortcut_modifiers").is_none());
    }
}
