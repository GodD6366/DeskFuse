//! Permanent LAN update discovery and exact-byte distribution. Installation is
//! coordinated by the application at its native-work admission boundary.
//!
//! 每台设备把正在运行的应用打包，经已配对的可信局域网连接发给同一发行通道的
//! 设备。不校验发布密钥；接收端核对 SHA-256，安装器再核对代码签名要求与通道。
use crate::RuntimeHandle;
use deskfuse_core::{Capabilities, ConnectionContext, ConnectionState, DeviceId, TrustState};
use deskfuse_protocol::Message;
use deskfuse_protocol::update::{
    MAX_UPDATE_CHUNK, ReleaseInventory, UpdateChunk, UpdateCommand, UpdateCommandAck,
    UpdateReceipt, UpdateRequest,
};
use deskfuse_update::{
    PeerAppArtifact, ReleaseVersion, Target, UpdatePhase, UpdateStatus, UpdateTransaction,
    verify_payload,
};
use futures::StreamExt;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

/// Manual requests bypass this interval; automatic peer discovery stays infrequent.
const AUTOMATIC_SCAN_INTERVAL: Duration = Duration::from_secs(12 * 60 * 60);
const UPDATE_COMMAND_DOWNLOAD_AND_INSTALL: u32 = 1;
const UPDATE_COMMAND_ACCEPTED: u32 = 1;
const UPDATE_COMMAND_UNSUPPORTED: u32 = 2;
const UPDATE_COMMAND_BUSY: u32 = 3;
/// 单个更新流的发送速率上限。更新包与鼠标位置共用同一条 QUIC 连接，
/// 不限速时大包会占满 Wi-Fi 队列，造成光标抖动；16 MiB/s 下 170 MB 约 11 秒。
const UPDATE_STREAM_RATE_LIMIT: f64 = 16.0 * 1024.0 * 1024.0;
/// 流式传输中每隔多少个块重新确认一次对端授权。
const PEER_RECHECK_CHUNKS: u32 = 64;

/// 本机自打包的目录与从对端下载的包目录。
const SELF_PACKAGE_DIR: &str = "self-package";
const PEER_PACKAGE_DIR: &str = "peer-packages";
/// 旧版签名分发留下的缓存、种子和按通道命名的包目录；启动时整体删除。
const RETIRED_DIRS: [&str; 4] = ["cache", "inbox", "github-self", "github-peer"];

/// An install handoff that stopped short leaves the running version in place.
const ABANDONED_INSTALL: &str = "更新未安装，当前版本保持可用；退出并重新启动 DeskFuse 后可重试";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GitHubUpdateView {
    pub target_version: Option<String>,
    pub release_url: Option<String>,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LanUpdateView {
    pub github: Option<GitHubUpdateView>,
    pub checking: bool,
    pub manual_requested: bool,
    pub installed: Option<ReleaseVersion>,
    pub status: Option<UpdateStatus>,
    pub peers: BTreeMap<DeviceId, PeerUpdateView>,
    pub message: String,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PeerUpdateView {
    pub version: String,
    pub build: u64,
    pub release_id: Vec<u8>,
    pub last_receipt: Option<UpdateReceipt>,
}
#[derive(Clone)]
pub struct LanUpdateConfig {
    pub installed: ReleaseVersion,
    pub target: Target,
    pub os: semver::Version,
    pub root: PathBuf,
    pub receipt: Option<UpdateReceipt>,
    pub previous_failure: Option<String>,
    pub channel: UpdateChannel,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum UpdateChannel {
    #[default]
    Lan,
    GitHub,
}
impl UpdateChannel {
    /// 与应用包内 `update-channel` 一致的名称；只接受同名通道的对端包。
    fn name(self) -> &'static str {
        match self {
            Self::Lan => "lan",
            Self::GitHub => "github",
        }
    }
}
struct Shared {
    /// Last complete, locally verified advertisement.
    inventory: Option<ReleaseInventory>,
    inventory_refresh: Arc<tokio::sync::Mutex<()>>,
    inventory_refreshed_at: Option<Instant>,
    view: LanUpdateView,
    /// 本机可分发的包：自打包的当前版本，以及已下载、尚未安装的新版。
    packages: BTreeMap<uuid::Uuid, (PeerAppArtifact, PathBuf)>,
    ready: Option<(PeerAppArtifact, PathBuf)>,
    /// Set by the second install request to bypass the normal file-idle wait.
    force_install: bool,
    quarantined: BTreeSet<uuid::Uuid>,
    serving: BTreeMap<uuid::Uuid, Serving>,
    receipts: Vec<ServedReceipt>,
    attempts: BTreeMap<uuid::Uuid, usize>,
    reconnect_confirmed: bool,
    /// Set when an install handoff did not complete in the running process.
    install_abandoned: bool,
}
struct Serving {
    peer: DeviceId,
    release: ReleaseVersion,
    artifact: deskfuse_update::ArtifactDescriptor,
    file: File,
    next: u64,
    touched: Instant,
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct ServedReceipt {
    peer: DeviceId,
    transaction_id: uuid::Uuid,
    release: ReleaseVersion,
    artifact: deskfuse_update::ArtifactDescriptor,
}
impl ServedReceipt {
    fn matches(&self, peer: DeviceId, receipt: &UpdateReceipt) -> bool {
        self.peer == peer
            && self.transaction_id.as_bytes() == receipt.transaction_id.as_slice()
            && self.release.release_id.as_bytes() == receipt.release_id.as_slice()
            && self.release.version.to_string() == receipt.installed_version
            && self.release.build_number == receipt.installed_build
            && target_id(self.artifact.target) == receipt.target
            && hex::decode(&self.artifact.sha256).is_ok_and(|hash| hash == receipt.sha256)
    }
}
pub struct LanUpdater {
    wake: Arc<tokio::sync::Notify>,
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    root: PathBuf,
    worker: Option<thread::JoinHandle<()>>,
}
impl LanUpdater {
    pub fn start(runtime: RuntimeHandle, config: LanUpdateConfig) -> Result<Self, String> {
        Self::start_with_remote_trigger(runtime, config, None)
    }
    /// Start the updater and optionally notify another update source when a
    /// trusted peer requests an update. Public bundles use this to wake their
    /// canonical GitHub checker while retaining LAN package serving.
    pub fn start_with_remote_trigger(
        runtime: RuntimeHandle,
        config: LanUpdateConfig,
        remote_trigger: Option<Arc<dyn Fn() + Send + Sync>>,
    ) -> Result<Self, String> {
        fs::create_dir_all(&config.root).map_err(|e| e.to_string())?;
        if !config
            .root
            .symlink_metadata()
            .map_err(|e| e.to_string())?
            .is_dir()
        {
            return Err("更新根目录无效".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&config.root, fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
        }
        let quarantined = match fs::read(config.root.join("quarantined.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| "更新隔离记录损坏")?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeSet::new(),
            Err(e) => return Err(e.to_string()),
        };
        for retired in RETIRED_DIRS {
            let path = config.root.join(retired);
            if path.is_dir() {
                fs::remove_dir_all(&path).map_err(|e| e.to_string())?;
            }
        }
        for entry in fs::read_dir(&config.root).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if entry.file_type().is_ok_and(|kind| kind.is_file())
                && name
                    .strip_prefix("download-")
                    .and_then(|s| s.strip_suffix(".part"))
                    .is_some_and(|s| uuid::Uuid::parse_str(s).is_ok())
            {
                fs::remove_file(entry.path()).map_err(|e| e.to_string())?;
            }
        }
        let shared = Arc::new(Mutex::new(Shared {
            inventory: None,
            inventory_refresh: Arc::new(tokio::sync::Mutex::new(())),
            inventory_refreshed_at: None,
            view: LanUpdateView {
                installed: Some(config.installed.clone()),
                error: config.previous_failure.clone(),
                github: (config.channel == UpdateChannel::GitHub).then(GitHubUpdateView::default),
                message: if config.channel == UpdateChannel::GitHub {
                    "可手动检查更新；公开发行版也会打包本机应用传给局域网设备".into()
                } else {
                    "可手动检查更新；后台每 12 小时检查一次".into()
                },
                ..Default::default()
            },
            packages: BTreeMap::new(),
            ready: None,
            force_install: false,
            quarantined,
            serving: BTreeMap::new(),
            attempts: BTreeMap::new(),
            reconnect_confirmed: false,
            install_abandoned: false,
            receipts: fs::read(config.root.join("served.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or_default(),
        }));
        let state = shared.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let signal = stop.clone();
        let root = config.root.clone();
        let wake = Arc::new(tokio::sync::Notify::new());
        let worker_wake = wake.clone();
        let worker = thread::Builder::new()
            .name("deskfuse-updates".into())
            .spawn(move || {
                let executor = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build();
                match executor {
                    Ok(executor) => executor.block_on(async move {
                        let server = tokio::spawn(serve(
                            runtime.clone(),
                            config.clone(),
                            state.clone(),
                            worker_wake.clone(),
                            remote_trigger.clone(),
                        ));
                        // Prepare the local advertisement once, without scanning peers.
                        // A manual request during this work retains its Notify permit.
                        if let Err(error) = refresh_inventory(&config, &state).await {
                            state.lock().unwrap().view.error = Some(error);
                        }
                        while !signal.load(Ordering::Acquire) {
                            tokio::select! {
                                _ = worker_wake.notified() => {},
                                _ = tokio::time::sleep(AUTOMATIC_SCAN_INTERVAL) => {},
                            }
                            if signal.load(Ordering::Acquire) {
                                break;
                            }
                            state.lock().unwrap().view.checking = true;
                            if let Err(error) = cycle(&runtime, &config, &state, &signal).await {
                                let mut state = state.lock().unwrap();
                                state.view.error = Some(error);
                                if state.ready.is_none() {
                                    state.view.manual_requested = false;
                                    if let Some(status) = &mut state.view.status
                                        && matches!(
                                            status.phase,
                                            UpdatePhase::Downloading | UpdatePhase::Verifying
                                        )
                                    {
                                        status.phase = UpdatePhase::Cancelled;
                                    }
                                }
                            }
                            state.lock().unwrap().view.checking = false;
                            if signal.load(Ordering::Acquire) {
                                break;
                            }
                        }
                        server.abort();
                        let _ = server.await;
                    }),
                    Err(error) => state.lock().unwrap().view.error = Some(error.to_string()),
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            wake,
            shared,
            stop,
            root,
            worker: Some(worker),
        })
    }
    /// Wake the existing worker; concurrent requests share the current check.
    pub fn check_now(&self) {
        let mut state = self.shared.lock().unwrap();
        if state.install_abandoned {
            state.view.message = ABANDONED_INSTALL.into();
            return;
        }
        if state.view.checking || state.ready.is_some() {
            return;
        }
        state.view.checking = true;
        state.view.error = None;
        state.view.message = "正在检查设备网络中的新版".into();
        self.wake.notify_one();
    }
    /// Join the current download or wake discovery, then request installation.
    /// A request applies only to this attempt, never to a future release.
    pub fn download_and_install_now(&self) {
        request_download_and_install(&self.shared, &self.wake, false);
    }
    /// A handoff the running process never completed must stop claiming an
    /// install. Without this the client keeps showing a pending restart, rejects
    /// every command behind the closed admission gate, and can only be recovered
    /// by quitting by hand.
    pub fn abandon_install(&self, reason: &str) {
        let mut state = self.shared.lock().unwrap();
        state.install_abandoned = true;
        state.force_install = false;
        state.view.manual_requested = false;
        if let Some(status) = &mut state.view.status
            && matches!(
                status.phase,
                UpdatePhase::Installing | UpdatePhase::WaitingForIdle | UpdatePhase::Staged
            )
        {
            status.phase = UpdatePhase::Cancelled;
        }
        state.view.message = ABANDONED_INSTALL.into();
        state.view.error = Some(reason.into());
    }
    pub fn install_abandoned(&self) -> bool {
        self.shared.lock().unwrap().install_abandoned
    }
    pub fn force_install(&self) -> bool {
        let state = self.shared.lock().unwrap();
        state.force_install && state.ready.is_some()
    }
    pub fn installation_waiting(&self, message: &str) {
        let mut state = self.shared.lock().unwrap();
        if state.view.manual_requested {
            state.view.message = message.into();
        }
    }
    pub fn snapshot(&self) -> LanUpdateView {
        self.shared.lock().unwrap().view.clone()
    }
    pub fn ready(&self) -> Option<(PeerAppArtifact, PathBuf)> {
        self.shared.lock().unwrap().ready.clone()
    }
    pub fn installing(&self) {
        let mut state = self.shared.lock().unwrap();
        state.force_install = false;
        if let Some(status) = &mut state.view.status {
            status.phase = UpdatePhase::Installing;
        }
        state.view.message = "正在安装更新，即将重新启动".into();
    }
    pub fn quarantine(&self, reason: String) {
        let mut state = self.shared.lock().unwrap();
        state.force_install = false;
        if let Some((offer, _)) = state.ready.take() {
            state.quarantined.insert(offer.release.release_id);
        }
        if let Some(status) = &mut state.view.status {
            status.phase = UpdatePhase::Quarantined;
        }
        state.view.manual_requested = false;
        state.view.error = Some(reason);
        if let Err(error) = deskfuse_update::write_json_atomic(
            &self.root.join("quarantined.json"),
            &state.quarantined,
        ) {
            state.view.error = Some(format!("无法保存更新隔离记录：{error}"));
        }
    }
}

/// Apply a local or remote request to the same coalescing state machine. A
/// remote request only wakes the receiver's updater; it cannot choose a
/// release or bypass verification.
fn request_download_and_install(
    shared: &Arc<Mutex<Shared>>,
    wake: &Arc<tokio::sync::Notify>,
    remote: bool,
) -> u32 {
    let mut state = shared.lock().unwrap();
    if state.install_abandoned {
        state.view.message = ABANDONED_INSTALL.into();
        return UPDATE_COMMAND_BUSY;
    }
    let already_requested = state.view.manual_requested;
    // Local installation is explicitly highest priority. Preserve the
    // remote coalescing behaviour, but let a local request preempt work as
    // soon as a verified artifact is ready.
    if !already_requested {
        state.force_install = !remote;
    }
    state.view.manual_requested = true;
    state.view.error = None;
    if state.ready.is_some() {
        if already_requested {
            state.force_install = true;
            state.view.message = if remote {
                "已收到配对设备的更新请求，正在立即安装".into()
            } else {
                "正在立即安装，将跳过传输或粘贴等待".into()
            };
        } else {
            state.view.message = if remote {
                "已收到配对设备的更新请求，准备安装并重启".into()
            } else {
                "正在准备主动安装，完成后将重新启动".into()
            };
        }
    } else if !state.view.checking {
        state.view.checking = true;
        state.view.message = if remote {
            "已收到配对设备的更新请求，正在查找并下载新版".into()
        } else {
            "正在查找并下载设备网络中的新版".into()
        };
        wake.notify_one();
    } else if remote {
        state.view.message = "已收到配对设备的更新请求，正在处理当前更新任务".into();
    }
    UPDATE_COMMAND_ACCEPTED
}

impl Drop for LanUpdater {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.wake.notify_one();
        let Some(worker) = self.worker.take() else {
            return;
        };
        // A discovery or download cycle blocks on peer I/O. Application exit must
        // not wait on one forever: the thread owns no state the exit depends on,
        // and it observes `stop` on its next boundary.
        let deadline = Instant::now() + Duration::from_secs(5);
        while !worker.is_finished() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(25));
        }
        if worker.is_finished() {
            let _ = worker.join();
        }
    }
}

impl RuntimeHandle {
    pub fn update_peer_allowed(&self, peer: DeviceId, context: &ConnectionContext) -> bool {
        let view = self.snapshot_view();
        view.device_network
            .allows(view.snapshot.local_device_id, peer)
            && view.snapshot.peers.iter().any(|p| {
                p.id == peer
                    && p.trust == TrustState::Trusted
                    && p.connection == ConnectionState::Connected
            })
            && context.input_eligible()
            && self
                .update_transport
                .input_capabilities(peer, context.session_id)
                .contains(Capabilities::LAN_UPDATES)
            && self
                .update_transport
                .input_contexts(peer)
                .iter()
                .any(|live| live == context)
    }
    pub fn update_trigger_allowed(&self, peer: DeviceId, context: &ConnectionContext) -> bool {
        self.update_peer_allowed(peer, context)
            && self
                .update_transport
                .input_capabilities(peer, context.session_id)
                .contains(Capabilities::LAN_UPDATE_TRIGGER)
    }
    /// Ask one trusted, online paired device to start its own normal update
    /// flow. The receiver still discovers and verifies the compatible package.
    pub async fn trigger_peer_update(&self, peer: DeviceId) -> Result<(), String> {
        let view = self.snapshot_view();
        if !view
            .device_network
            .allows(view.snapshot.local_device_id, peer)
            || !view.snapshot.peers.iter().any(|p| {
                p.id == peer
                    && p.trust == TrustState::Trusted
                    && p.connection == ConnectionState::Connected
            })
        {
            return Err("目标设备尚未连接或不受信任".into());
        }
        let _ = self
            .update_transport
            .ensure_input_link(peer, Vec::new())
            .await;
        let context = self
            .update_transport
            .input_contexts(peer)
            .into_iter()
            .find(|context| self.update_trigger_allowed(peer, context))
            .ok_or("目标设备不支持主动更新或当前链路不符合更新要求")?;
        let reply = self
            .update_exchange(
                peer,
                &context,
                Message::UpdateCommand(UpdateCommand {
                    action: UPDATE_COMMAND_DOWNLOAD_AND_INSTALL,
                }),
            )
            .await?;
        match reply {
            Message::UpdateCommandAck(UpdateCommandAck { status })
                if status == UPDATE_COMMAND_ACCEPTED =>
            {
                Ok(())
            }
            Message::UpdateCommandAck(UpdateCommandAck { status })
                if status == UPDATE_COMMAND_BUSY =>
            {
                Err("目标设备正在安装更新，暂时无法再次触发".into())
            }
            Message::UpdateCommandAck(UpdateCommandAck { status })
                if status == UPDATE_COMMAND_UNSUPPORTED =>
            {
                Err("目标设备不支持主动更新".into())
            }
            _ => Err("目标设备未确认更新请求".into()),
        }
    }
    /// Ask several trusted, online paired devices concurrently. A partial
    /// result is reported so one unavailable peer does not hide requests that
    /// were accepted by the others.
    pub async fn trigger_peer_updates(&self, peers: Vec<DeviceId>) -> Result<(), String> {
        let mut peers = peers;
        peers.sort();
        peers.dedup();
        if peers.is_empty() {
            return Err("没有可触发更新的设备".into());
        }
        let results = futures::future::join_all(
            peers
                .iter()
                .copied()
                .map(|peer| self.trigger_peer_update(peer)),
        )
        .await;
        let mut failed = Vec::new();
        let mut accepted = 0usize;
        for result in results {
            match result {
                Ok(()) => accepted += 1,
                Err(error) => failed.push(error),
            }
        }
        if failed.is_empty() {
            Ok(())
        } else {
            let failed_count = failed.len();
            let details = failed.into_iter().take(4).collect::<Vec<_>>().join("；");
            let summary = if accepted > 0 {
                format!("已请求 {accepted} 台，{failed_count} 台失败")
            } else {
                format!("{failed_count} 台均失败")
            };
            Err(format!("批量触发更新部分失败：{summary}；原因：{details}"))
        }
    }
    async fn update_exchange(
        &self,
        peer: DeviceId,
        context: &ConnectionContext,
        message: Message,
    ) -> Result<Message, String> {
        if !self.update_peer_allowed(peer, context) {
            return Err("更新源已离线或不在可信物理局域网中".into());
        }
        let reply = self
            .update_transport
            .exchange_update(peer, context.session_id, message)
            .await
            .map_err(|e| e.to_string())?;
        if !self.update_peer_allowed(peer, context) {
            return Err("更新连接已失效".into());
        }
        Ok(reply)
    }
    async fn update_exchange_stream(
        &self,
        peer: DeviceId,
        context: &ConnectionContext,
        message: Message,
    ) -> Result<deskfuse_transport::UpdateReplyStream, String> {
        if !self.update_peer_allowed(peer, context) {
            return Err("更新源已离线或不在可信物理局域网中".into());
        }
        self.update_transport
            .exchange_update_stream(peer, context.session_id, message)
            .await
            .map_err(|e| e.to_string())
    }
}
fn target_id(target: Target) -> u32 {
    match target {
        Target::MacosArm64 => 1,
        Target::WindowsX64 => 2,
    }
}
fn inventory(config: &LanUpdateConfig, state: &Shared) -> Result<ReleaseInventory, String> {
    let mut artifacts = Vec::new();
    for (offer, _) in state.packages.values() {
        artifacts.push(serde_json::to_vec(offer).map_err(|e| e.to_string())?);
        if artifacts.len() == 16 {
            break;
        }
    }
    Ok(ReleaseInventory {
        installed_version: config.installed.version.to_string(),
        installed_build: config.installed.build_number,
        installed_release_id: config.installed.release_id.as_bytes().to_vec(),
        target: target_id(config.target),
        signed_manifests: Vec::new(),
        github_artifacts: artifacts,
    })
}
fn observe_peer(state: &Arc<Mutex<Shared>>, peer: DeviceId, inventory: &ReleaseInventory) {
    let mut state = state.lock().unwrap();
    let row = state.view.peers.entry(peer).or_default();
    if row
        .last_receipt
        .as_ref()
        .is_some_and(|receipt| receipt.release_id != inventory.installed_release_id)
    {
        row.last_receipt = None;
    }
    row.version = inventory.installed_version.clone();
    row.build = inventory.installed_build;
    row.release_id = inventory.installed_release_id.clone();
}
async fn serve(
    runtime: RuntimeHandle,
    config: LanUpdateConfig,
    state: Arc<Mutex<Shared>>,
    wake: Arc<tokio::sync::Notify>,
    remote_trigger: Option<Arc<dyn Fn() + Send + Sync>>,
) {
    let mut inbox = runtime.update_transport.update_messages();
    while let Some(request) = inbox.next().await {
        if !runtime.update_peer_allowed(request.peer_id, &request.context) {
            continue;
        }
        if let Message::UpdateCommand(command) = &request.message {
            let status = if command.action == UPDATE_COMMAND_DOWNLOAD_AND_INSTALL {
                request_download_and_install(&state, &wake, true)
            } else {
                UPDATE_COMMAND_UNSUPPORTED
            };
            if status == UPDATE_COMMAND_ACCEPTED
                && let Some(remote_trigger) = &remote_trigger
            {
                remote_trigger();
            }
            let response = Message::UpdateCommandAck(UpdateCommandAck { status });
            if runtime.update_peer_allowed(request.peer_id, &request.context) {
                let _ = request.reply.send(response).await;
            }
            continue;
        }
        // Startup may still be importing the bundled payload. Prepare an empty
        // inventory on demand; established inventories stay cheap to query so
        // repeated peer polls do not block chunk delivery on repackaging.
        let needs_inventory = matches!(&request.message, Message::ReleaseInventory(_))
            && state
                .lock()
                .unwrap()
                .inventory
                .as_ref()
                .is_none_or(|inventory| {
                    inventory.signed_manifests.is_empty() && inventory.github_artifacts.is_empty()
                });
        if needs_inventory && let Err(error) = refresh_inventory(&config, &state).await {
            state.lock().unwrap().view.error =
                Some(format!("无法准备供其他设备下载的更新包：{error}"));
            continue;
        }
        let config = config.clone();
        let state = state.clone();
        let peer = request.peer_id;
        // 更新包下发持续数秒到数分钟，放到独立阻塞任务里，不能卡住后续请求。
        if let Message::UpdateRequest(query) = request.message {
            let runtime = runtime.clone();
            let context = request.context;
            let reply = request.reply;
            drop(tokio::task::spawn_blocking(move || {
                let allowed = || runtime.update_peer_allowed(peer, &context);
                respond_stream(&allowed, &config, &state, peer, query, &reply)
            }));
            continue;
        }
        let response =
            tokio::task::spawn_blocking(move || respond(&config, &state, peer, request.message))
                .await;
        if runtime.update_peer_allowed(peer, &request.context)
            && let Ok(Ok(response)) = response
        {
            let _ = request.reply.send(response).await;
        }
    }
}
/// 在同一条流上连续写出 `[offset, offset + max_bytes)` 的更新块，直到满足请求、
/// 到达文件末尾或对端离开。每块仍经 `respond` 校验会话、偏移与哈希，
/// 只是不再为每 64 KiB 往返一次；按 `UPDATE_STREAM_RATE_LIMIT` 限速。
#[allow(clippy::too_many_arguments)]
fn respond_stream(
    allowed: &dyn Fn() -> bool,
    config: &LanUpdateConfig,
    state: &Arc<Mutex<Shared>>,
    peer: DeviceId,
    mut query: UpdateRequest,
    reply: &tokio::sync::mpsc::Sender<Message>,
) {
    let wanted = query.max_bytes.max(1);
    let started = Instant::now();
    let mut sent = 0u64;
    let mut chunks = 0u32;
    while sent < wanted {
        if chunks.is_multiple_of(PEER_RECHECK_CHUNKS) && !allowed() {
            return;
        }
        let Ok(Message::UpdateChunk(chunk)) =
            respond(config, state, peer, Message::UpdateRequest(query.clone()))
        else {
            return;
        };
        let len = chunk.data.len() as u64;
        if reply.blocking_send(Message::UpdateChunk(chunk)).is_err() {
            return;
        }
        sent += len;
        chunks += 1;
        query.offset += len;
        let due = Duration::from_secs_f64(sent as f64 / UPDATE_STREAM_RATE_LIMIT);
        if let Some(wait) = due.checked_sub(started.elapsed()) {
            thread::sleep(wait);
        }
    }
}
fn respond(
    config: &LanUpdateConfig,
    state: &Arc<Mutex<Shared>>,
    peer: DeviceId,
    message: Message,
) -> Result<Message, String> {
    match message {
        Message::ReleaseInventory(remote) => {
            observe_peer(state, peer, &remote);
            let inventory =
                state
                    .lock()
                    .unwrap()
                    .inventory
                    .clone()
                    .unwrap_or_else(|| ReleaseInventory {
                        installed_version: config.installed.version.to_string(),
                        installed_build: config.installed.build_number,
                        installed_release_id: config.installed.release_id.as_bytes().to_vec(),
                        target: target_id(config.target),
                        signed_manifests: Vec::new(),
                        github_artifacts: Vec::new(),
                    });
            Ok(Message::ReleaseInventory(inventory))
        }
        Message::UpdateRequest(query) => {
            let id = uuid::Uuid::from_slice(&query.transaction_id).map_err(|e| e.to_string())?;
            // Validate original bytes once per transfer, then retain the open
            // file and pin its release. Rehashing every 64 KiB stalls input I/O.
            if query.offset == 0 {
                let package = state
                    .lock()
                    .unwrap()
                    .packages
                    .values()
                    .find(|(offer, _)| {
                        offer.release.release_id.as_bytes() == query.release_id.as_slice()
                            && target_id(offer.artifact.target) == query.target
                            && hex::decode(&offer.artifact.sha256).is_ok_and(|h| h == query.sha256)
                    })
                    .cloned();
                let Some((
                    PeerAppArtifact {
                        release, artifact, ..
                    },
                    path,
                )) = package
                else {
                    return Err("更新包不可用".into());
                };
                let mut shared = state.lock().unwrap();
                shared
                    .serving
                    .retain(|_, session| session.touched.elapsed() < Duration::from_secs(120));
                if shared.serving.len() >= 32 {
                    return Err("更新分发正在忙".into());
                }
                shared.serving.insert(
                    id,
                    Serving {
                        peer,
                        release,
                        artifact,
                        file: File::open(path).map_err(|e| e.to_string())?,
                        next: 0,
                        touched: Instant::now(),
                    },
                );
            }
            let mut shared = state.lock().unwrap();
            let session = shared
                .serving
                .get_mut(&id)
                .ok_or("更新会话已过期，请重新下载")?;
            if session.peer != peer
                || session.next != query.offset
                || session.next >= session.artifact.size
                || session.release.release_id.as_bytes() != query.release_id.as_slice()
                || target_id(session.artifact.target) != query.target
                || hex::decode(&session.artifact.sha256).map_err(|e| e.to_string())? != query.sha256
            {
                return Err("更新任务或偏移不匹配".into());
            }
            let mut data = vec![
                0;
                (session.artifact.size - session.next).min(MAX_UPDATE_CHUNK as u64)
                    as usize
            ];
            session
                .file
                .read_exact(&mut data)
                .map_err(|e| e.to_string())?;
            session.next += data.len() as u64;
            session.touched = Instant::now();
            if session.next == session.artifact.size {
                let receipt = ServedReceipt {
                    peer,
                    transaction_id: id,
                    release: session.release.clone(),
                    artifact: session.artifact.clone(),
                };
                shared.serving.remove(&id);
                shared.receipts.push(receipt);
                if shared.receipts.len() > 256 {
                    shared.receipts.remove(0);
                }
                deskfuse_update::write_json_atomic(
                    &config.root.join("served.json"),
                    &shared.receipts,
                )
                .map_err(|e| e.to_string())?;
            }
            Ok(Message::UpdateChunk(UpdateChunk {
                transaction_id: query.transaction_id,
                offset: query.offset,
                data,
            }))
        }
        Message::UpdateReceipt(receipt) => {
            let exact_task = state
                .lock()
                .unwrap()
                .receipts
                .iter()
                .any(|expected| expected.matches(peer, &receipt));
            // A downloaded package may survive a client restart, so its new
            // transaction was not served by us. Reconcile that receipt with the
            // fresh installed inventory AND local package bytes, never just a
            // version string.
            if !exact_task {
                let reported = state
                    .lock()
                    .unwrap()
                    .view
                    .peers
                    .get(&peer)
                    .is_some_and(|row| {
                        row.version == receipt.installed_version
                            && row.build == receipt.installed_build
                            && row.release_id == receipt.release_id
                    });
                let packaged = state.lock().unwrap().packages.values().any(|(offer, _)| {
                    offer.release.release_id.as_bytes() == receipt.release_id.as_slice()
                        && offer.release.version.to_string() == receipt.installed_version
                        && offer.release.build_number == receipt.installed_build
                        && target_id(offer.artifact.target) == receipt.target
                        && hex::decode(&offer.artifact.sha256)
                            .is_ok_and(|hash| hash == receipt.sha256)
                });
                if !reported || !packaged {
                    return Err("更新回执不匹配".into());
                }
            }
            state
                .lock()
                .unwrap()
                .view
                .peers
                .entry(peer)
                .or_default()
                .last_receipt = Some(receipt.clone());
            Ok(Message::UpdateReceipt(receipt))
        }
        _ => Err("更新请求类型无效".into()),
    }
}

/// 打包本机应用，并载入已下载、仍高于当前版本的对端包。已不再需要的旧包直接删除，
/// 避免下载目录随版本无限增长。
fn load_packages(config: &LanUpdateConfig, state: &Arc<Mutex<Shared>>) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let dest = config.root.join(SELF_PACKAGE_DIR);
        match deskfuse_update::installer::package_running_app(&dest) {
            Ok(offer) => {
                let path = dest.join(&offer.artifact.file_name);
                state
                    .lock()
                    .unwrap()
                    .packages
                    .insert(offer.release.release_id, (offer, path));
            }
            Err(error) => {
                state.lock().unwrap().view.error =
                    Some(format!("无法把本机应用打包给其他设备：{error}"));
            }
        }
    }
    let root = config.root.join(PEER_PACKAGE_DIR);
    let Ok(entries) = fs::read_dir(&root) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let offer = fs::read(entry.path().join("artifact.json"))
            .ok()
            .and_then(|bytes| PeerAppArtifact::from_slice(&bytes).ok());
        let path = offer
            .as_ref()
            .map(|offer| entry.path().join(&offer.artifact.file_name));
        match (offer, path) {
            (Some(offer), Some(path))
                if offer.channel == config.channel.name()
                    && offer.release.precedence(&config.installed).is_gt()
                    && verify_payload(&offer.release, &offer.artifact, &path).is_ok() =>
            {
                state
                    .lock()
                    .unwrap()
                    .packages
                    .insert(offer.release.release_id, (offer, path));
            }
            _ => fs::remove_dir_all(entry.path()).map_err(|e| e.to_string())?,
        }
    }
    Ok(())
}

/// Package this app before advertising, including an empty peer inventory.
async fn refresh_inventory(
    config: &LanUpdateConfig,
    state: &Arc<Mutex<Shared>>,
) -> Result<ReleaseInventory, String> {
    let requested_at = Instant::now();
    let refresh = state.lock().unwrap().inventory_refresh.clone();
    let _refresh = refresh.lock().await;
    {
        let shared = state.lock().unwrap();
        // A concurrent startup/manual/peer refresh already did this work.
        if shared
            .inventory_refreshed_at
            .is_some_and(|time| time >= requested_at)
            && let Some(inventory) = &shared.inventory
        {
            return Ok(inventory.clone());
        }
    }
    // Keep the last snapshot until its replacement is complete.
    let load_config = config.clone();
    let load_state = state.clone();
    tokio::task::spawn_blocking(move || load_packages(&load_config, &load_state))
        .await
        .map_err(|e| e.to_string())??;
    let mut shared = state.lock().unwrap();
    let local_inventory = inventory(config, &shared)?;
    shared.inventory = Some(local_inventory.clone());
    shared.inventory_refreshed_at = Some(Instant::now());
    Ok(local_inventory)
}

async fn cycle(
    runtime: &RuntimeHandle,
    config: &LanUpdateConfig,
    state: &Arc<Mutex<Shared>>,
    stop: &AtomicBool,
) -> Result<(), String> {
    let local_inventory = refresh_inventory(config, state).await?;
    let view = runtime.snapshot_view();
    let peers: Vec<_> = view
        .snapshot
        .peers
        .iter()
        .filter(|p| {
            p.trust == TrustState::Trusted
                && p.connection == ConnectionState::Connected
                && view
                    .device_network
                    .allows(view.snapshot.local_device_id, p.id)
        })
        .map(|p| p.id)
        .collect();
    state
        .lock()
        .unwrap()
        .view
        .peers
        .retain(|id, _| peers.contains(id));
    let connected_peers = peers.len();
    let mut checked_peers = 0;
    let mut newer_peer: Option<(semver::Version, u64)> = None;
    let mut sources: BTreeMap<uuid::Uuid, Vec<(DeviceId, ConnectionContext)>> = BTreeMap::new();
    // 本机已有的包（含已下载未安装的新版）也能升级本机。
    let mut offers: Vec<PeerAppArtifact> = state
        .lock()
        .unwrap()
        .packages
        .values()
        .map(|(offer, _)| offer.clone())
        .collect();
    for peer in peers {
        if stop.load(Ordering::Acquire) {
            return Ok(());
        }
        let _ = runtime
            .update_transport
            .ensure_input_link(peer, Vec::new())
            .await;
        let Some(context) = runtime
            .update_transport
            .input_contexts(peer)
            .into_iter()
            .find(|c| runtime.update_peer_allowed(peer, c))
        else {
            continue;
        };
        if let Ok(Message::ReleaseInventory(remote)) = runtime
            .update_exchange(
                peer,
                &context,
                Message::ReleaseInventory(local_inventory.clone()),
            )
            .await
        {
            checked_peers += 1;
            if let Ok(version) = remote.installed_version.parse::<semver::Version>() {
                let build = remote.installed_build;
                let newer_than_installed = version
                    .cmp_precedence(&config.installed.version)
                    .then(build.cmp(&config.installed.build_number))
                    .is_gt();
                if newer_than_installed
                    && newer_peer
                        .as_ref()
                        .is_none_or(|(known_version, known_build)| {
                            version
                                .cmp_precedence(known_version)
                                .then(build.cmp(known_build))
                                .is_gt()
                        })
                {
                    newer_peer = Some((version, build));
                }
            }
            observe_peer(state, peer, &remote);
            for bytes in remote.github_artifacts {
                if let Ok(offer) = PeerAppArtifact::from_slice(&bytes) {
                    sources
                        .entry(offer.release.release_id)
                        .or_default()
                        .push((peer, context.clone()));
                    offers.push(offer);
                }
            }
            let receipt_confirmed = state.lock().unwrap().reconnect_confirmed;
            if !receipt_confirmed
                && let Some(receipt) = &config.receipt
                && runtime
                    .update_exchange(peer, &context, Message::UpdateReceipt(receipt.clone()))
                    .await
                    .is_ok_and(|reply| reply == Message::UpdateReceipt(receipt.clone()))
            {
                state.lock().unwrap().reconnect_confirmed = true;
            }
        }
    }
    if state.lock().unwrap().ready.is_some() {
        return Ok(());
    }
    finish_cycle(
        runtime,
        config,
        state,
        stop,
        Scan {
            offers,
            sources,
            connected_peers,
            checked_peers,
            newer_peer,
        },
    )
    .await
}
struct Scan {
    offers: Vec<PeerAppArtifact>,
    sources: BTreeMap<uuid::Uuid, Vec<(DeviceId, ConnectionContext)>>,
    connected_peers: usize,
    checked_peers: usize,
    newer_peer: Option<(semver::Version, u64)>,
}
async fn finish_cycle(
    runtime: &RuntimeHandle,
    config: &LanUpdateConfig,
    state: &Arc<Mutex<Shared>>,
    stop: &AtomicBool,
    scan: Scan,
) -> Result<(), String> {
    let Scan {
        mut offers,
        sources,
        connected_peers,
        checked_peers,
        newer_peer,
    } = scan;
    offers.retain(|offer| {
        offer.channel == config.channel.name()
            && offer.artifact.target == config.target
            && offer.artifact.minimum_os <= config.os
    });
    for (i, left) in offers.iter().enumerate() {
        for right in offers.iter().skip(i + 1) {
            if left.release.precedence(&right.release) == std::cmp::Ordering::Equal && left != right
            {
                return Err("设备网络中同一版本的更新包内容不一致".into());
            }
        }
    }
    let quarantined = state.lock().unwrap().quarantined.clone();
    let selected = offers
        .into_iter()
        .filter(|offer| offer.release.precedence(&config.installed).is_gt())
        .filter(|offer| !quarantined.contains(&offer.release.release_id))
        .max_by(|a, b| a.release.precedence(&b.release));
    let Some(offer) = selected else {
        let mut state = state.lock().unwrap();
        state.view.manual_requested = false;
        let quarantined_failure = state
            .view
            .status
            .as_ref()
            .is_some_and(|s| s.phase == UpdatePhase::Quarantined);
        state.view.message = if quarantined_failure {
            "更新失败版本已隔离，当前版本保持可用；继续等待新版".to_owned()
        } else if config.previous_failure.is_some() {
            "上次更新未完成，当前版本保持可用；将继续寻找设备网络中的新版".to_owned()
        } else if let Some((version, build)) = newer_peer {
            format!(
                "设备网络中有 {version} · build {build}，但对方没有提供同一发行通道的更新包；本次未安装，当前仍为 {} · build {}",
                config.installed.version, config.installed.build_number
            )
        } else if connected_peers == 0 {
            "当前没有已连接的可信局域网设备，也没有可安装的本地新版；本次未安装，请连接提供新版的设备后重试".to_owned()
        } else if checked_peers < connected_peers {
            "未能完成所有已连接设备的更新检查，也未找到可安装的新版；本次未安装，请稍后重试"
                .to_owned()
        } else {
            format!(
                "已检查 {checked_peers} 台设备：未找到高于 {} · build {} 的可安装兼容新版；本次未安装",
                config.installed.version, config.installed.build_number
            )
        };
        if !quarantined_failure {
            state.view.error = config.previous_failure.clone();
        }
        return Ok(());
    };
    let cached_path = state
        .lock()
        .unwrap()
        .packages
        .get(&offer.release.release_id)
        .map(|(_, path)| path.clone());
    if let Some(path) = cached_path {
        let mut transaction =
            UpdateTransaction::for_artifact(&offer.release, offer.artifact.clone())
                .map_err(|e| e.to_string())?;
        transaction
            .received(0, offer.artifact.size)
            .map_err(|e| e.to_string())?;
        let verified =
            verify_payload(&offer.release, &offer.artifact, &path).map_err(|e| e.to_string())?;
        transaction.verified(&verified).map_err(|e| e.to_string())?;
        let mut state = state.lock().unwrap();
        state.ready = Some((offer, path));
        state.view.status = Some(transaction.status().clone());
        state.view.error = None;
        state.view.message = "新版已就绪，等待传输、粘贴和远端控制结束后自动安装".into();
        return Ok(());
    }
    let candidates = sources
        .get(&offer.release.release_id)
        .ok_or("更新源已离线")?;
    let index = {
        let mut state = state.lock().unwrap();
        let attempt = state.attempts.entry(offer.release.release_id).or_default();
        let index = *attempt % candidates.len();
        *attempt = attempt.wrapping_add(1);
        index
    };
    let (peer, context) = &candidates[index];
    let mut transaction = UpdateTransaction::for_artifact(&offer.release, offer.artifact.clone())
        .map_err(|e| e.to_string())?;
    {
        let mut state = state.lock().unwrap();
        state.view.status = Some(transaction.status().clone());
        state.view.error = None;
    }
    let path = config.root.join(format!(
        "download-{}.part",
        transaction.status().transaction_id
    ));
    let dest_dir = config
        .root
        .join(PEER_PACKAGE_DIR)
        .join(offer.release.release_id.to_string());
    let result = async {
        download(
            runtime,
            (*peer, context),
            offer.release.release_id.as_bytes(),
            &mut transaction,
            &path,
            state,
            stop,
        )
        .await?;
        verify_payload(&offer.release, &offer.artifact, &path).map_err(|e| e.to_string())?;
        fs::create_dir_all(&dest_dir).map_err(|e| e.to_string())?;
        let final_path = dest_dir.join(&offer.artifact.file_name);
        fs::rename(&path, &final_path).map_err(|e| e.to_string())?;
        deskfuse_update::write_json_atomic(&dest_dir.join("artifact.json"), &offer)
            .map_err(|e| e.to_string())?;
        Ok::<_, String>(final_path)
    }
    .await;
    let _ = fs::remove_file(&path);
    let final_path = result?;
    let verified =
        verify_payload(&offer.release, &offer.artifact, &final_path).map_err(|e| e.to_string())?;
    transaction.verified(&verified).map_err(|e| e.to_string())?;
    let mut state = state.lock().unwrap();
    state.packages.insert(
        offer.release.release_id,
        (offer.clone(), final_path.clone()),
    );
    state.inventory = Some(inventory(config, &state)?);
    state.ready = Some((offer, final_path));
    state.view.status = Some(transaction.status().clone());
    state.view.error = None;
    state.view.message = "新版已下载，等待传输、粘贴和远端控制结束后自动安装".into();
    Ok(())
}
async fn download(
    runtime: &RuntimeHandle,
    source: (DeviceId, &ConnectionContext),
    release_id: &[u8],
    transaction: &mut UpdateTransaction,
    path: &std::path::Path,
    state: &Arc<Mutex<Shared>>,
    stop: &AtomicBool,
) -> Result<(), String> {
    let (peer, context) = source;
    let artifact = transaction.status().artifact.clone();
    if fs2::available_space(path.parent().ok_or("更新路径无效")?).map_err(|e| e.to_string())?
        < artifact
            .size
            .saturating_mul(3)
            .saturating_add(256 * 1024 * 1024)
    {
        return Err("磁盘空间不足，无法安全更新".into());
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    while transaction.status().received_bytes < artifact.size {
        if stop.load(Ordering::Acquire) {
            return Err("更新下载已中断".into());
        }
        let offset = transaction.status().received_bytes;
        let request = UpdateRequest {
            transaction_id: transaction.status().transaction_id.as_bytes().to_vec(),
            release_id: release_id.to_vec(),
            target: target_id(artifact.target),
            sha256: hex::decode(&artifact.sha256).map_err(|e| e.to_string())?,
            offset,
            max_bytes: artifact.size - offset,
        };
        let mut replies = runtime
            .update_exchange_stream(peer, context, Message::UpdateRequest(request))
            .await?;
        let mut received_chunks = 0u32;
        while let Some(reply) = replies.next().await {
            if stop.load(Ordering::Acquire) {
                return Err("更新下载已中断".into());
            }
            if received_chunks.is_multiple_of(PEER_RECHECK_CHUNKS)
                && !runtime.update_peer_allowed(peer, context)
            {
                return Err("更新连接已失效".into());
            }
            let Message::UpdateChunk(chunk) = reply.map_err(|e| e.to_string())? else {
                return Err("更新源返回了错误的数据类型".into());
            };
            if chunk.transaction_id != transaction.status().transaction_id.as_bytes()
                || chunk.offset != transaction.status().received_bytes
                || chunk.data.len() > MAX_UPDATE_CHUNK
            {
                return Err("更新块与当前任务不匹配".into());
            }
            transaction
                .received(chunk.offset, chunk.data.len() as u64)
                .map_err(|e| e.to_string())?;
            file.write_all(&chunk.data).map_err(|e| e.to_string())?;
            received_chunks += 1;
            let mut state = state.lock().unwrap();
            state.view.status = Some(transaction.status().clone());
            state.view.message = "正在从可信局域网设备下载更新".into();
            state.view.error = None;
        }
        // 对端提前结束流时按新偏移重新请求；一个块都没收到说明源已无法继续供应。
        if received_chunks == 0 {
            return Err("更新源没有返回数据".into());
        }
    }
    file.sync_all().map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn empty_shared() -> Arc<Mutex<Shared>> {
        Arc::new(Mutex::new(Shared {
            inventory: None,
            inventory_refresh: Arc::new(tokio::sync::Mutex::new(())),
            inventory_refreshed_at: None,
            view: Default::default(),
            packages: BTreeMap::new(),
            ready: None,
            force_install: false,
            quarantined: BTreeSet::new(),
            serving: BTreeMap::new(),
            receipts: Vec::new(),
            attempts: BTreeMap::new(),
            reconnect_confirmed: false,
            install_abandoned: false,
        }))
    }

    #[test]
    fn remote_update_command_coalesces_into_the_existing_install_flow() {
        let state = empty_shared();
        let wake = Arc::new(tokio::sync::Notify::new());
        assert_eq!(
            request_download_and_install(&state, &wake, true),
            UPDATE_COMMAND_ACCEPTED
        );
        {
            let state = state.lock().unwrap();
            assert!(state.view.checking);
            assert!(state.view.manual_requested);
            assert!(state.view.message.contains("已收到配对设备的更新请求"));
        }
        // A duplicate request joins the same attempt instead of creating a
        // second scan or download worker.
        assert_eq!(
            request_download_and_install(&state, &wake, true),
            UPDATE_COMMAND_ACCEPTED
        );
        assert!(state.lock().unwrap().view.manual_requested);
    }

    #[test]
    fn transfer_serves_original_chunks_and_only_acknowledges_the_exact_completed_task() {
        let dir = tempfile::tempdir().unwrap();
        let payload = vec![37; MAX_UPDATE_CHUNK + 17];
        let path = dir.path().join(PeerAppArtifact::FILE_NAME);
        fs::write(&path, &payload).unwrap();
        let release = ReleaseVersion {
            version: semver::Version::new(0, 1, 0),
            build_number: 2,
            release_id: uuid::Uuid::new_v4(),
        };
        let offer = PeerAppArtifact {
            channel: "lan".into(),
            release: release.clone(),
            artifact: deskfuse_update::ArtifactDescriptor {
                target: Target::MacosArm64,
                kind: deskfuse_update::PackageKind::MacosAppZip,
                file_name: PeerAppArtifact::FILE_NAME.into(),
                size: payload.len() as u64,
                sha256: hex::encode(Sha256::digest(&payload)),
                minimum_os: semver::Version::new(14, 0, 0),
            },
        };
        offer.validate().unwrap();
        let descriptor = offer.artifact.clone();
        let config = LanUpdateConfig {
            installed: release.clone(),
            target: Target::MacosArm64,
            os: semver::Version::new(15, 0, 0),
            root: dir.path().to_path_buf(),
            receipt: None,
            previous_failure: None,
            channel: UpdateChannel::Lan,
        };
        let state = empty_shared();
        state
            .lock()
            .unwrap()
            .packages
            .insert(release.release_id, (offer, path));
        // 自打包的包只通过包列表通告，不再附带签名清单。
        let advertised = inventory(&config, &state.lock().unwrap()).unwrap();
        assert!(advertised.signed_manifests.is_empty());
        assert_eq!(advertised.github_artifacts.len(), 1);
        let peer = DeviceId::from_bytes([1; 32]);
        let other = DeviceId::from_bytes([2; 32]);
        let id = uuid::Uuid::new_v4();
        let mut query = UpdateRequest {
            transaction_id: id.as_bytes().to_vec(),
            release_id: release.release_id.as_bytes().to_vec(),
            target: 1,
            sha256: hex::decode(&descriptor.sha256).unwrap(),
            offset: 0,
            max_bytes: 0,
        };
        let receipt = UpdateReceipt {
            transaction_id: query.transaction_id.clone(),
            release_id: query.release_id.clone(),
            installed_version: release.version.to_string(),
            installed_build: 2,
            target: 1,
            sha256: query.sha256.clone(),
        };
        assert!(
            respond(
                &config,
                &state,
                peer,
                Message::UpdateReceipt(receipt.clone())
            )
            .is_err()
        );
        let Message::UpdateChunk(first) =
            respond(&config, &state, peer, Message::UpdateRequest(query.clone())).unwrap()
        else {
            panic!("chunk")
        };
        assert_eq!(first.data, payload[..MAX_UPDATE_CHUNK]);
        query.offset = MAX_UPDATE_CHUNK as u64;
        assert!(
            respond(
                &config,
                &state,
                other,
                Message::UpdateRequest(query.clone())
            )
            .is_err()
        );
        let mut skipped = query.clone();
        skipped.offset += 1;
        assert!(respond(&config, &state, peer, Message::UpdateRequest(skipped)).is_err());
        let Message::UpdateChunk(last) =
            respond(&config, &state, peer, Message::UpdateRequest(query)).unwrap()
        else {
            panic!("chunk")
        };
        assert_eq!(last.data, payload[MAX_UPDATE_CHUNK..]);
        let mut wrong = receipt.clone();
        wrong.installed_build += 1;
        assert!(respond(&config, &state, peer, Message::UpdateReceipt(wrong)).is_err());
        assert!(
            respond(
                &config,
                &state,
                other,
                Message::UpdateReceipt(receipt.clone())
            )
            .is_err()
        );
        assert_eq!(
            respond(
                &config,
                &state,
                peer,
                Message::UpdateReceipt(receipt.clone())
            )
            .unwrap(),
            Message::UpdateReceipt(receipt.clone())
        );
        assert!(state.lock().unwrap().serving.is_empty());
        let persisted: Vec<ServedReceipt> =
            serde_json::from_slice(&fs::read(dir.path().join("served.json")).unwrap()).unwrap();
        assert!(persisted[0].matches(peer, &receipt));
        let mut restarted = receipt.clone();
        restarted.transaction_id = uuid::Uuid::new_v4().as_bytes().to_vec();
        assert!(
            respond(
                &config,
                &state,
                peer,
                Message::UpdateReceipt(restarted.clone())
            )
            .is_err()
        );
        let installed = ReleaseInventory {
            installed_version: release.version.to_string(),
            installed_build: release.build_number,
            installed_release_id: release.release_id.as_bytes().to_vec(),
            target: 1,
            signed_manifests: vec![],
            github_artifacts: vec![],
        };
        respond(&config, &state, peer, Message::ReleaseInventory(installed)).unwrap();
        assert!(
            respond(
                &config,
                &state,
                peer,
                Message::UpdateReceipt(restarted.clone())
            )
            .is_ok()
        );
        restarted.sha256[0] ^= 1;
        assert!(respond(&config, &state, peer, Message::UpdateReceipt(restarted)).is_err());

        // 流式分发：一次请求按块连续写出整个更新包，且字节与源文件一致。
        let stream_query = UpdateRequest {
            transaction_id: uuid::Uuid::new_v4().as_bytes().to_vec(),
            release_id: release.release_id.as_bytes().to_vec(),
            target: 1,
            sha256: hex::decode(&descriptor.sha256).unwrap(),
            offset: 0,
            max_bytes: payload.len() as u64,
        };
        let (reply, mut chunks) = tokio::sync::mpsc::channel(8);
        respond_stream(
            &|| true,
            &config,
            &state,
            peer,
            stream_query.clone(),
            &reply,
        );
        drop(reply);
        let mut streamed = Vec::new();
        while let Ok(Message::UpdateChunk(chunk)) = chunks.try_recv() {
            assert_eq!(chunk.offset, streamed.len() as u64);
            streamed.extend_from_slice(&chunk.data);
        }
        assert_eq!(streamed, payload);
        // 授权失效时一块都不发送。
        let (reply, mut chunks) = tokio::sync::mpsc::channel(8);
        respond_stream(&|| false, &config, &state, peer, stream_query, &reply);
        drop(reply);
        assert!(chunks.try_recv().is_err());
    }
}
