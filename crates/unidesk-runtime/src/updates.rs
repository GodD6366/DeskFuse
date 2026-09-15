//! Permanent LAN update discovery and exact-byte distribution. Installation is
//! coordinated by the application at its native-work admission boundary.
use crate::RuntimeHandle;
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
use unidesk_core::{Capabilities, ConnectionContext, ConnectionState, DeviceId, TrustState};
use unidesk_protocol::Message;
use unidesk_protocol::update::{
    MAX_UPDATE_CHUNK, ReleaseInventory, UpdateChunk, UpdateReceipt, UpdateRequest,
};
use unidesk_update::{
    Installation, ReleaseCache, ReleaseVersion, SignedReleaseManifest, Target, UPDATE_CACHE_LIMIT,
    UpdatePhase, UpdateStatus, UpdateTransaction, VerifiedRelease, select_release, verify_artifact,
};

/// Manual requests bypass this interval; automatic peer discovery stays infrequent.
const AUTOMATIC_SCAN_INTERVAL: Duration = Duration::from_secs(12 * 60 * 60);

/// An install handoff that stopped short leaves the running version in place.
const ABANDONED_INSTALL: &str = "更新未安装，当前版本保持可用；退出并重新启动 uniDesk 后可重试";

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
    pub seed_dirs: Vec<PathBuf>,
    pub key: ed25519_dalek::VerifyingKey,
    pub receipt: Option<UpdateReceipt>,
    pub previous_failure: Option<String>,
}
struct Shared {
    /// Last complete, locally verified advertisement.
    inventory: Option<ReleaseInventory>,
    inventory_refresh: Arc<tokio::sync::Mutex<()>>,
    inventory_refreshed_at: Option<Instant>,
    view: LanUpdateView,
    ready: Option<(VerifiedRelease, PathBuf)>,
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
    artifact: unidesk_update::ArtifactDescriptor,
    file: File,
    next: u64,
    touched: Instant,
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct ServedReceipt {
    peer: DeviceId,
    transaction_id: uuid::Uuid,
    release: ReleaseVersion,
    artifact: unidesk_update::ArtifactDescriptor,
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
        let cache = Arc::new(Mutex::new(
            ReleaseCache::open(&config.root.join("cache"), config.key, UPDATE_CACHE_LIMIT)
                .map_err(|e| e.to_string())?,
        ));
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
                message: "可手动检查更新；后台每 12 小时检查一次".into(),
                ..Default::default()
            },
            ready: None,
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
            .name("unidesk-updates".into())
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
                            cache.clone(),
                            state.clone(),
                        ));
                        // Prepare the local advertisement once, without scanning peers.
                        // A manual request during this work retains its Notify permit.
                        if let Err(error) = refresh_inventory(&config, &cache, &state).await {
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
                            if let Err(error) =
                                cycle(&runtime, &config, &cache, &state, &signal).await
                            {
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
        let mut state = self.shared.lock().unwrap();
        if state.install_abandoned {
            state.view.message = ABANDONED_INSTALL.into();
            return;
        }
        state.view.manual_requested = true;
        state.view.error = None;
        if state.ready.is_some() {
            state.view.message = "正在准备主动安装，完成后将重新启动".into();
        } else if !state.view.checking {
            state.view.checking = true;
            state.view.message = "正在查找并下载设备网络中的新版".into();
            self.wake.notify_one();
        }
    }
    /// A handoff the running process never completed must stop claiming an
    /// install. Without this the client keeps showing a pending restart, rejects
    /// every command behind the closed admission gate, and can only be recovered
    /// by quitting by hand.
    pub fn abandon_install(&self, reason: &str) {
        let mut state = self.shared.lock().unwrap();
        state.install_abandoned = true;
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
    pub fn installation_waiting(&self, message: &str) {
        let mut state = self.shared.lock().unwrap();
        if state.view.manual_requested {
            state.view.message = message.into();
        }
    }
    pub fn snapshot(&self) -> LanUpdateView {
        self.shared.lock().unwrap().view.clone()
    }
    pub fn ready(&self) -> Option<(VerifiedRelease, PathBuf)> {
        self.shared.lock().unwrap().ready.clone()
    }
    pub fn installing(&self) {
        let mut state = self.shared.lock().unwrap();
        if let Some(status) = &mut state.view.status {
            status.phase = UpdatePhase::Installing;
        }
        state.view.message = "正在安装更新，即将重新启动".into();
    }
    pub fn quarantine(&self, reason: String) {
        let mut state = self.shared.lock().unwrap();
        if let Some((release, _)) = state.ready.take() {
            state
                .quarantined
                .insert(release.manifest().release.release_id);
        }
        if let Some(status) = &mut state.view.status {
            status.phase = UpdatePhase::Quarantined;
        }
        state.view.manual_requested = false;
        state.view.error = Some(reason);
        if let Err(error) = unidesk_update::write_json_atomic(
            &self.root.join("quarantined.json"),
            &state.quarantined,
        ) {
            state.view.error = Some(format!("无法保存更新隔离记录：{error}"));
        }
    }
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
}
fn target_id(target: Target) -> u32 {
    match target {
        Target::MacosArm64 => 1,
        Target::WindowsX64 => 2,
    }
}
fn inventory(config: &LanUpdateConfig, cache: &ReleaseCache) -> Result<ReleaseInventory, String> {
    let mut entries = cache.inventory().map_err(|e| e.to_string())?;
    entries.sort_by(|a, b| b.artifact.release().precedence(a.artifact.release()));
    let mut seen = BTreeSet::new();
    let mut manifests = Vec::new();
    for entry in entries {
        if seen.insert(entry.release.manifest().release.release_id) {
            manifests.push(serde_json::to_vec(entry.release.signed()).map_err(|e| e.to_string())?);
            if manifests.len() == 16 {
                break;
            }
        }
    }
    Ok(ReleaseInventory {
        installed_version: config.installed.version.to_string(),
        installed_build: config.installed.build_number,
        installed_release_id: config.installed.release_id.as_bytes().to_vec(),
        target: target_id(config.target),
        signed_manifests: manifests,
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
    cache: Arc<Mutex<ReleaseCache>>,
    state: Arc<Mutex<Shared>>,
) {
    let mut inbox = runtime.update_transport.update_messages();
    while let Some(request) = inbox.next().await {
        if !runtime.update_peer_allowed(request.peer_id, &request.context) {
            continue;
        }
        // Startup may still be importing the bundled payload. Prepare an empty
        // inventory on demand; established inventories stay cheap to query so
        // repeated peer polls do not block chunk delivery on whole-cache hashes.
        let needs_inventory = matches!(&request.message, Message::ReleaseInventory(_))
            && state
                .lock()
                .unwrap()
                .inventory
                .as_ref()
                .is_none_or(|inventory| inventory.signed_manifests.is_empty());
        if needs_inventory && let Err(error) = refresh_inventory(&config, &cache, &state).await {
            state.lock().unwrap().view.error =
                Some(format!("无法准备供其他设备下载的更新包：{error}"));
            continue;
        }
        let cache = cache.clone();
        let config = config.clone();
        let state = state.clone();
        let peer = request.peer_id;
        let response = tokio::task::spawn_blocking(move || {
            respond(&config, &cache, &state, peer, request.message)
        })
        .await;
        if runtime.update_peer_allowed(peer, &request.context)
            && let Ok(Ok(response)) = response
        {
            let _ = request.reply.send(response);
        }
    }
}
fn respond(
    config: &LanUpdateConfig,
    cache: &Mutex<ReleaseCache>,
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
                    });
            Ok(Message::ReleaseInventory(inventory))
        }
        Message::UpdateRequest(query) => {
            let id = uuid::Uuid::from_slice(&query.transaction_id).map_err(|e| e.to_string())?;
            // Validate original bytes once per transfer, then retain the open
            // file and pin its release. Rehashing every 64 KiB stalls input I/O.
            if query.offset == 0 {
                let entries = cache
                    .lock()
                    .unwrap()
                    .inventory()
                    .map_err(|e| e.to_string())?;
                let item = entries
                    .into_iter()
                    .find(|item| {
                        item.artifact.release().release_id.as_bytes() == query.release_id.as_slice()
                            && target_id(item.artifact.descriptor().target) == query.target
                            && hex::decode(&item.artifact.descriptor().sha256)
                                .is_ok_and(|h| h == query.sha256)
                    })
                    .ok_or("更新包不可用")?;
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
                        release: item.artifact.release().clone(),
                        artifact: item.artifact.descriptor().clone(),
                        file: File::open(item.artifact.path()).map_err(|e| e.to_string())?,
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
                unidesk_update::write_json_atomic(
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
            // A cached download may survive a client restart, or a host may
            // install a local signed seed. Its new transaction was not served
            // by us. Reconcile that receipt with the fresh installed inventory
            // AND verified local release bytes, never just a version string.
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
                if !reported
                    || !cache
                        .lock()
                        .unwrap()
                        .inventory()
                        .map_err(|e| e.to_string())?
                        .iter()
                        .any(|entry| {
                            entry.artifact.release().release_id.as_bytes()
                                == receipt.release_id.as_slice()
                                && entry.artifact.release().version.to_string()
                                    == receipt.installed_version
                                && entry.artifact.release().build_number == receipt.installed_build
                                && target_id(entry.artifact.descriptor().target) == receipt.target
                                && hex::decode(&entry.artifact.descriptor().sha256)
                                    .is_ok_and(|hash| hash == receipt.sha256)
                        })
                {
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

/// Import complete seeds before advertising, including an empty peer inventory.
async fn refresh_inventory(
    config: &LanUpdateConfig,
    cache: &Arc<Mutex<ReleaseCache>>,
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
    // Keep the last verified snapshot until its replacement is complete.
    let seed_config = config.clone();
    let seed_cache = cache.clone();
    let seed_state = state.clone();
    tokio::task::spawn_blocking(move || {
        for root in &seed_config.seed_dirs {
            let manifest = root.join("manifest.json");
            if !manifest.is_file() {
                continue;
            }
            let Ok(file) = File::open(&manifest) else {
                continue;
            };
            let mut bytes = Vec::new();
            if file.take(65537).read_to_end(&mut bytes).is_err() {
                continue;
            }
            let Ok(signed) = SignedReleaseManifest::from_slice(&bytes) else {
                continue;
            };
            let Ok(release) = signed.verify(&seed_config.key) else {
                continue;
            };
            for descriptor in &release.manifest().artifacts {
                let path = root.join(&descriptor.file_name);
                if path.is_file() {
                    let cache = seed_cache.lock().unwrap();
                    let entries = cache.inventory().map_err(|e| e.to_string())?;
                    if entries.iter().any(|entry| {
                        entry.release.signed() == release.signed()
                            && entry.artifact.descriptor().target == descriptor.target
                    }) {
                        continue;
                    }
                    let shared = seed_state.lock().unwrap();
                    let mut protected: BTreeSet<_> = shared
                        .serving
                        .values()
                        .map(|entry| entry.release.release_id)
                        .collect();
                    protected.insert(seed_config.installed.release_id);
                    if let Some((ready, _)) = &shared.ready {
                        protected.insert(ready.manifest().release.release_id);
                    }
                    drop(shared);
                    cache
                        .reserve(descriptor.size.saturating_add(65536), &protected)
                        .map_err(|e| e.to_string())?;
                    cache
                        .import(&bytes, descriptor.target, &path)
                        .map_err(|e| e.to_string())?;
                }
            }
        }
        Ok::<_, String>(())
    })
    .await
    .map_err(|e| e.to_string())??;
    let local_inventory = {
        let cache = cache.clone();
        let config = config.clone();
        tokio::task::spawn_blocking(move || inventory(&config, &cache.lock().unwrap()))
            .await
            .map_err(|e| e.to_string())??
    };
    let mut shared = state.lock().unwrap();
    shared.inventory = Some(local_inventory.clone());
    shared.inventory_refreshed_at = Some(Instant::now());
    Ok(local_inventory)
}

async fn cycle(
    runtime: &RuntimeHandle,
    config: &LanUpdateConfig,
    cache: &Arc<Mutex<ReleaseCache>>,
    state: &Arc<Mutex<Shared>>,
    stop: &AtomicBool,
) -> Result<(), String> {
    let local_inventory = refresh_inventory(config, cache, state).await?;
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
    let mut offers = Vec::new();
    let mut sources: BTreeMap<uuid::Uuid, Vec<(DeviceId, ConnectionContext)>> = BTreeMap::new();
    // Local seeds can upgrade this host as well as its neighbours.
    for bytes in &local_inventory.signed_manifests {
        if let Ok(release) =
            SignedReleaseManifest::from_slice(bytes).and_then(|s| s.verify(&config.key))
        {
            offers.push(release);
        }
    }
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
            for bytes in remote.signed_manifests {
                if let Ok(release) =
                    SignedReleaseManifest::from_slice(&bytes).and_then(|s| s.verify(&config.key))
                {
                    sources
                        .entry(release.manifest().release.release_id)
                        .or_default()
                        .push((peer, context.clone()));
                    offers.push(release);
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
    let quarantined = state.lock().unwrap().quarantined.clone();
    let installation = Installation {
        release: &config.installed,
        target: config.target,
        os: &config.os,
        protocol_major: 1,
        update_protocol: 1,
        quarantined: &quarantined,
    };
    let selected = select_release(&installation, &offers).map_err(|e| e.to_string())?;
    let Some((release, artifact)) = selected else {
        let mut state = state.lock().unwrap();
        state.view.manual_requested = false;
        // A quarantined failure is about the artifact; a previous failure without
        // one is an attempt that stopped short. Neither hides the current version,
        // and neither may leave the update page looking like an error page.
        let quarantined_failure = state
            .view
            .status
            .as_ref()
            .is_some_and(|s| s.phase == UpdatePhase::Quarantined);
        // A receipt acknowledges the running release, not this discovery/install
        // request. Never reuse it as proof that a newly requested update completed.
        state.view.message = if quarantined_failure {
            "更新失败版本已隔离，当前版本保持可用；继续等待新版".to_owned()
        } else if config.previous_failure.is_some() {
            "上次更新未完成，当前版本保持可用；将继续寻找设备网络中的新版".to_owned()
        } else if let Some((version, build)) = newer_peer {
            format!(
                "设备网络中有 {version} · build {build}，但未找到可安装的兼容已签名更新包；本次未安装，当前仍为 {} · build {}。请让新版设备提供完整的更新包",
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
    let release = release.clone();
    let cached = {
        let cache = cache.clone();
        let expected = release.clone();
        let target = config.target;
        tokio::task::spawn_blocking(move || {
            cache.lock().unwrap().inventory().map(|entries| {
                entries.into_iter().find(|entry| {
                    entry.release.signed() == expected.signed()
                        && entry.artifact.descriptor().target == target
                })
            })
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?
    };
    let mut transaction =
        UpdateTransaction::new(&release, config.target).map_err(|e| e.to_string())?;
    {
        let mut state = state.lock().unwrap();
        state.view.status = Some(transaction.status().clone());
        state.view.error = None;
    }
    let payload = if let Some(cached) = cached {
        transaction
            .received(0, artifact.size)
            .map_err(|e| e.to_string())?;
        cached.artifact
    } else {
        let candidates = sources
            .get(&release.manifest().release.release_id)
            .ok_or("更新源已离线")?;
        let index = {
            let mut state = state.lock().unwrap();
            let attempt = state
                .attempts
                .entry(release.manifest().release.release_id)
                .or_default();
            let index = *attempt % candidates.len();
            *attempt = attempt.wrapping_add(1);
            index
        };
        let (peer, context) = &candidates[index];
        let mut protected: BTreeSet<_> = state
            .lock()
            .unwrap()
            .serving
            .values()
            .map(|entry| entry.release.release_id)
            .collect();
        protected.insert(config.installed.release_id);
        protected.insert(release.manifest().release.release_id);
        cache
            .lock()
            .unwrap()
            .reserve(
                artifact.size.saturating_mul(2).saturating_add(65536),
                &protected,
            )
            .map_err(|e| e.to_string())?;
        let path = config.root.join(format!(
            "download-{}.part",
            transaction.status().transaction_id
        ));
        let result = download(
            runtime,
            (*peer, context),
            &release,
            &mut transaction,
            &path,
            state,
            stop,
        )
        .await;
        let imported = match result {
            Ok(()) => {
                let cache = cache.clone();
                let release = release.clone();
                let path = path.clone();
                let target = config.target;
                tokio::task::spawn_blocking(move || {
                    cache
                        .lock()
                        .unwrap()
                        .import(
                            &serde_json::to_vec(release.signed()).map_err(|e| e.to_string())?,
                            target,
                            &path,
                        )
                        .map_err(|e| e.to_string())
                })
                .await
                .map_err(|e| e.to_string())?
            }
            Err(error) => Err(error),
        };
        let _ = fs::remove_file(&path);
        imported?.artifact
    };
    transaction.verified(&payload).map_err(|e| e.to_string())?;
    let refreshed = {
        let cache = cache.clone();
        let config = config.clone();
        tokio::task::spawn_blocking(move || inventory(&config, &cache.lock().unwrap()))
            .await
            .map_err(|e| e.to_string())??
    };
    state.lock().unwrap().inventory = Some(refreshed);
    let mut state = state.lock().unwrap();
    state.ready = Some((release, payload.path().to_path_buf()));
    state.view.status = Some(transaction.status().clone());
    state.view.error = None;
    state.view.message = "新版已下载，等待传输、粘贴和远端控制结束后自动安装".into();
    Ok(())
}
async fn download(
    runtime: &RuntimeHandle,
    source: (DeviceId, &ConnectionContext),
    release: &VerifiedRelease,
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
            release_id: release.manifest().release.release_id.as_bytes().to_vec(),
            target: target_id(artifact.target),
            sha256: hex::decode(&artifact.sha256).map_err(|e| e.to_string())?,
            offset,
        };
        let Message::UpdateChunk(chunk) = runtime
            .update_exchange(peer, context, Message::UpdateRequest(request))
            .await?
        else {
            return Err("更新源返回了错误的数据类型".into());
        };
        if chunk.transaction_id != transaction.status().transaction_id.as_bytes()
            || chunk.offset != offset
            || chunk.data.len() > MAX_UPDATE_CHUNK
        {
            return Err("更新块与当前任务不匹配".into());
        }
        transaction
            .received(chunk.offset, chunk.data.len() as u64)
            .map_err(|e| e.to_string())?;
        file.write_all(&chunk.data).map_err(|e| e.to_string())?;
        let mut state = state.lock().unwrap();
        state.view.status = Some(transaction.status().clone());
        state.view.message = "正在从可信局域网设备下载更新".into();
        state.view.error = None;
    }
    file.sync_all().map_err(|e| e.to_string())?;
    verify_artifact(release, artifact.target, path).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    #[test]
    fn transfer_serves_original_chunks_and_only_acknowledges_the_exact_completed_task() {
        let dir = tempfile::tempdir().unwrap();
        let key = ed25519_dalek::SigningKey::from_bytes(&[42; 32]);
        let payload = vec![37; MAX_UPDATE_CHUNK + 17];
        let path = dir.path().join("source.zip");
        fs::write(&path, &payload).unwrap();
        let release = ReleaseVersion {
            version: semver::Version::new(0, 1, 0),
            build_number: 2,
            release_id: uuid::Uuid::new_v4(),
        };
        let descriptor = unidesk_update::ArtifactDescriptor {
            target: Target::MacosArm64,
            kind: unidesk_update::PackageKind::MacosAppZip,
            file_name: "uniDesk-update.zip".into(),
            size: payload.len() as u64,
            sha256: hex::encode(Sha256::digest(&payload)),
            minimum_os: semver::Version::new(14, 0, 0),
        };
        let manifest = SignedReleaseManifest::sign(
            unidesk_update::ReleaseManifest {
                schema_version: 1,
                application_id: "com.unidesk.app".into(),
                release: release.clone(),
                protocol_major: 1,
                minimum_update_protocol: 1,
                artifacts: vec![descriptor.clone()],
            },
            &key,
        )
        .unwrap();
        let cache = ReleaseCache::open(
            &dir.path().join("cache"),
            key.verifying_key(),
            UPDATE_CACHE_LIMIT,
        )
        .unwrap();
        cache
            .import(
                &serde_json::to_vec(&manifest).unwrap(),
                Target::MacosArm64,
                &path,
            )
            .unwrap();
        let cache = Mutex::new(cache);
        let config = LanUpdateConfig {
            installed: release.clone(),
            target: Target::MacosArm64,
            os: semver::Version::new(15, 0, 0),
            root: dir.path().to_path_buf(),
            seed_dirs: vec![],
            key: key.verifying_key(),
            receipt: None,
            previous_failure: None,
        };
        let state = Arc::new(Mutex::new(Shared {
            inventory: None,
            inventory_refresh: Arc::new(tokio::sync::Mutex::new(())),
            inventory_refreshed_at: None,
            view: Default::default(),
            ready: None,
            quarantined: BTreeSet::new(),
            serving: BTreeMap::new(),
            receipts: vec![],
            attempts: BTreeMap::new(),
            reconnect_confirmed: false,
            install_abandoned: false,
        }));
        let peer = DeviceId::from_bytes([1; 32]);
        let other = DeviceId::from_bytes([2; 32]);
        let id = uuid::Uuid::new_v4();
        let mut query = UpdateRequest {
            transaction_id: id.as_bytes().to_vec(),
            release_id: release.release_id.as_bytes().to_vec(),
            target: 1,
            sha256: hex::decode(&descriptor.sha256).unwrap(),
            offset: 0,
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
                &cache,
                &state,
                peer,
                Message::UpdateReceipt(receipt.clone())
            )
            .is_err()
        );
        let Message::UpdateChunk(first) = respond(
            &config,
            &cache,
            &state,
            peer,
            Message::UpdateRequest(query.clone()),
        )
        .unwrap() else {
            panic!("chunk")
        };
        assert_eq!(first.data, payload[..MAX_UPDATE_CHUNK]);
        query.offset = MAX_UPDATE_CHUNK as u64;
        assert!(
            respond(
                &config,
                &cache,
                &state,
                other,
                Message::UpdateRequest(query.clone())
            )
            .is_err()
        );
        let mut skipped = query.clone();
        skipped.offset += 1;
        assert!(
            respond(
                &config,
                &cache,
                &state,
                peer,
                Message::UpdateRequest(skipped)
            )
            .is_err()
        );
        let Message::UpdateChunk(last) =
            respond(&config, &cache, &state, peer, Message::UpdateRequest(query)).unwrap()
        else {
            panic!("chunk")
        };
        assert_eq!(last.data, payload[MAX_UPDATE_CHUNK..]);
        let mut wrong = receipt.clone();
        wrong.installed_build += 1;
        assert!(respond(&config, &cache, &state, peer, Message::UpdateReceipt(wrong)).is_err());
        assert!(
            respond(
                &config,
                &cache,
                &state,
                other,
                Message::UpdateReceipt(receipt.clone())
            )
            .is_err()
        );
        assert_eq!(
            respond(
                &config,
                &cache,
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
                &cache,
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
        };
        respond(
            &config,
            &cache,
            &state,
            peer,
            Message::ReleaseInventory(installed),
        )
        .unwrap();
        assert!(
            respond(
                &config,
                &cache,
                &state,
                peer,
                Message::UpdateReceipt(restarted.clone())
            )
            .is_ok()
        );
        restarted.sha256[0] ^= 1;
        assert!(
            respond(
                &config,
                &cache,
                &state,
                peer,
                Message::UpdateReceipt(restarted)
            )
            .is_err()
        );
    }
}
