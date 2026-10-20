//! Native remote application boundary, using the production runtime's identity,
//! membership authorization and active QUIC connections, not an auxiliary TLS stack.
use deskfuse_core::{DeviceId, TrustState};
use deskfuse_runtime::RuntimeHandle;
use deskfuse_transport::{QuinnTransport, remote_application::RemoteSession};
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
static SERVICE: OnceLock<Mutex<Option<Arc<Service>>>> = OnceLock::new();
pub struct Service {
    transport: QuinnTransport,
    runtime: RuntimeHandle,
    executor: tokio::runtime::Handle,
    enabled: AtomicBool,
}
pub fn install(transport: QuinnTransport, runtime: RuntimeHandle) {
    *SERVICE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(Service {
        transport,
        runtime,
        executor: tokio::runtime::Handle::current(),
        enabled: AtomicBool::new(false),
    }));
}
pub fn service() -> Result<Arc<Service>, String> {
    SERVICE
        .get()
        .and_then(|s| s.lock().ok()?.clone())
        .ok_or_else(|| "远程应用引擎尚未就绪".into())
}
impl Service {
    pub fn enable(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Release);
        self.transport.set_remote_application_enabled(enabled);
    }
    pub fn authorized(&self, peer: DeviceId) -> bool {
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
    pub fn require_viewer_allowed(&self) -> Result<(), String> {
        if !self.enabled.load(Ordering::Acquire) {
            return Err("远程应用已关闭".into());
        }
        if self
            .runtime
            .input_snapshot()
            .can_operate_remote_applications()
        {
            return Ok(());
        }
        // Releasing the viewer pause is always allowed. Never leave the native
        // input service paused after a role transition rejects the viewer.
        if let Some(control) = self.runtime.input_service_control() {
            let _ = control.signal(b'v');
        }
        Err("当前键鼠共享角色不允许操作远程应用，请在主控电脑操作，或结束被控并关闭键鼠共享".into())
    }
    pub fn open(&self, peer: DeviceId) -> Result<Arc<RemoteSession>, String> {
        self.require_viewer_allowed()?;
        if !self.authorized(peer) {
            return Err("仅允许连接已配对且未被阻止的设备".into());
        }
        let (tx, rx) = mpsc::sync_channel(1);
        let transport = self.transport.clone();
        self.executor.spawn(async move {
            let result = transport.open_remote_application(peer).await;
            let _ = tx.send(result);
        });
        let session = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .map_err(|_| "远程应用连接超时".to_string())??;
        if let Err(error) = self.require_viewer_allowed() {
            session.close();
            return Err(error);
        }
        Ok(session)
    }
    pub fn incoming(&self) -> Option<Arc<RemoteSession>> {
        let session = self.transport.take_remote_application()?;
        if self.enabled.load(Ordering::Acquire) && self.authorized(session.peer) {
            Some(session)
        } else {
            session.close();
            None
        }
    }
    pub fn viewer_control(&self, active: bool) -> Result<(), String> {
        if active {
            self.require_viewer_allowed()?;
        }
        let control = self
            .runtime
            .input_service_control()
            .ok_or("输入进程不可用")?;
        if control.signal(if active { b'V' } else { b'v' }) {
            Ok(())
        } else {
            Err("输入进程不可用".into())
        }
    }
    pub fn viewer_ready(&self) -> bool {
        if self.require_viewer_allowed().is_err() {
            return false;
        }
        self.runtime.input_service_control().is_some_and(|control| {
            control
                .status
                .lock()
                .is_ok_and(|s| s.running && s.remote_application_viewer_paused)
        })
    }
    /// Runs off the UI thread. A fresh nonce rules out stale status snapshots,
    /// including an old "no active lease" observed before queued input drains.
    pub fn prepare_control(&self, peer: DeviceId) -> Result<(), String> {
        let nonce = uuid::Uuid::new_v4().to_string();
        let packet = serde_json::json!({"kind": "quiesce", "session": nonce, "generation": 0});
        self.input(peer, &packet.to_string())?;
        let acknowledgement = format!("{peer}:{nonce}");
        let control = self
            .runtime
            .input_service_control()
            .ok_or("输入进程不可用")?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while std::time::Instant::now() < deadline {
            if !self.enabled.load(Ordering::Acquire) || !self.authorized(peer) {
                return Err("远程应用访问授权已失效".into());
            }
            if control.status.lock().is_ok_and(|status| {
                status.running
                    && status.remote_application_quiesced.as_deref() == Some(&acknowledgement)
            }) {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        Err("输入服务未确认旧按键已释放，请更新两端或重新连接后重试".into())
    }
    pub fn control_ready(&self, peer: DeviceId, session: &str, generation: u64) -> bool {
        self.authorized(peer)
            && self.runtime.input_service_control().is_some_and(|control| {
                control.status.lock().is_ok_and(|s| {
                    s.running
                        && s.remote_application_lease.as_deref()
                            == Some(format!("{peer}:{session}:{generation}").as_str())
                })
            })
    }
    pub fn control_problem(&self, peer: DeviceId, session: &str, generation: u64) -> String {
        if !self.authorized(peer) {
            return "远程应用访问授权已失效".into();
        }
        let Some(control) = self.runtime.input_service_control() else {
            return "被控 Mac 的输入进程不可用，请重新打开 DeskFuse".into();
        };
        let Ok(status) = control.status.lock() else {
            return "被控 Mac 的输入进程状态不可用".into();
        };
        if !status.running {
            return "被控 Mac 的输入进程已停止，请重新打开 DeskFuse".into();
        }
        let request = format!("{peer}:{session}:{generation}");
        if status.remote_application_request.as_deref() == Some(&request) {
            if let Some(error) = &status.remote_application_error {
                return error.clone();
            }
            return "被控 Mac 的输入独占握手未完成，请点击画面重试".into();
        }
        "被控 Mac 尚未收到当前画面的输入配置，请等待画面呈现后重试".into()
    }
    pub fn input(&self, peer: DeviceId, json: &str) -> Result<(), String> {
        if !self.enabled.load(Ordering::Acquire) || !self.authorized(peer) {
            return Err("远程应用访问未授权".into());
        }
        if json.len() > 8192 {
            return Err("输入消息超限".into());
        }
        let mut packet: deskfuse_runtime::input_service_types::RemoteApplicationInput =
            serde_json::from_str(json).map_err(|e| e.to_string())?;
        packet.peer = Some(peer);
        self.runtime
            .input_service_control()
            .ok_or("独立输入进程不可用")?
            .remote_application(packet)
    }
}
