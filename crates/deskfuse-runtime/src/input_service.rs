//! Process boundary for native input. Remote-app input has a bounded, separate
//! typed lane; normal cross-screen motion remains on the helper network.
//! Control, replaceable status, and urgent/lifetime traffic use different sockets.
use super::*;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use std::{
    io::{self, Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::atomic::AtomicU64,
};
mod child;
pub use child::{ChildConnection, InputOnlyRuntime};
const VERSION: u32 = 1;
const MAX_FRAME: usize = 2 * 1024 * 1024;

/// 主进程和输入进程共用的构建号。安装包从 `release.json` 读取，
/// 不编译进 Rust，因此只改构建号不会重编 FFI 和 Swift。
pub fn build_identity() -> &'static str {
    static CACHED: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    CACHED.get_or_init(resolve_build_identity).as_str()
}

fn resolve_build_identity() -> String {
    if let Some(identity) = std::env::var("DESKFUSE_INPUT_BUILD_ID")
        .ok()
        .and_then(|value| accept_identity(&value))
    {
        return identity;
    }
    if let Ok(executable) = std::env::current_exe()
        && let Some(directory) = executable.parent()
    {
        let resources = directory.join("../Resources");
        if let Some(identity) = read_release_identity(&resources.join("release.json")) {
            return identity;
        }
        if let Some(identity) = read_sidecar(&directory.join("build-id")) {
            return identity;
        }
        if let Some(identity) = read_sidecar(&resources.join("build-id")) {
            return identity;
        }
    }
    env!("CARGO_PKG_VERSION").to_string()
}

fn accept_identity(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'+' | b'-'))
    {
        return None;
    }
    Some(value.to_string())
}

fn read_sidecar(path: &std::path::Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(256).read_to_end(&mut bytes).ok()?;
    accept_identity(&String::from_utf8(bytes).ok()?)
}

fn read_release_identity(path: &std::path::Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(65_536).read_to_end(&mut bytes).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let version = value.get("version")?.as_str()?;
    let build = value.get("build_number")?.as_u64()?;
    accept_identity(&format!("{version}-{build}"))
}

#[cfg(test)]
mod build_identity_tests {
    use super::{accept_identity, read_release_identity, read_sidecar};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn scratch() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "deskfuse-build-id-{}-{nanos}-{n}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("scratch");
        path
    }

    #[test]
    fn retry_is_routed_to_input_service_and_allowed_across_ipc() {
        use super::*;
        let command = RuntimeCommand::RefreshNativeObserver;
        assert!(command.is_input());
        assert!(host_command_allowed(&command));
        let encoded = serde_json::to_vec(&HostMessage::Command {
            id: 1,
            value: command,
        })
        .unwrap();
        let decoded: HostMessage = serde_json::from_slice(&encoded).unwrap();
        assert!(matches!(
            decoded,
            HostMessage::Command {
                value: RuntimeCommand::RefreshNativeObserver,
                ..
            }
        ));
    }

    #[test]
    fn environment_layout_save_is_forwarded_to_input_service() {
        use super::*;
        let command = RuntimeCommand::SaveEnvironmentLayout { name: None };
        assert!(command.is_input());
        assert!(host_command_allowed(&command));
    }

    #[test]
    fn wallpaper_crosses_both_input_service_ipc_directions() {
        use super::*;
        let peer = DeviceId::from_public_key([7; 32]).unwrap();
        let display_id = DisplayId::from("remote-screen");
        let jpeg = [
            vec![0xff, 0xd8, 0xff],
            vec![42; deskfuse_core::MAX_DISPLAY_WALLPAPER_BYTES - 3],
        ]
        .concat();
        let command = RuntimeCommand::PublishDisplayWallpaper {
            display_id: display_id.clone(),
            jpeg: jpeg.clone(),
        };
        assert!(command.is_input() && host_command_allowed(&command));
        let (mut host, mut child) = UnixStream::pair().unwrap();
        let mut command_writer = host.try_clone().unwrap();
        let writer = std::thread::spawn(move || {
            write_frame(
                &mut command_writer,
                &HostMessage::Command {
                    id: 1,
                    value: command,
                },
            )
            .unwrap();
        });
        match read_frame::<HostMessage>(&mut child).unwrap() {
            HostMessage::Command {
                value:
                    RuntimeCommand::PublishDisplayWallpaper {
                        display_id: received_id,
                        jpeg: received_jpeg,
                    },
                ..
            } => {
                assert_eq!(received_id, display_id);
                assert_eq!(received_jpeg, jpeg);
            }
            _ => panic!("wallpaper command lost across IPC"),
        }
        writer.join().unwrap();

        let mut snapshot = ServiceSnapshot {
            generation: 1,
            native: NativeStatus::default(),
            input: InputSharingSnapshot::default(),
            diagnostics: String::new(),
            quality: Default::default(),
            last_command: 1,
        };
        snapshot
            .input
            .remote_wallpapers
            .insert(peer, BTreeMap::from([(display_id.clone(), jpeg.clone())]));
        let mut snapshot_writer = child.try_clone().unwrap();
        let writer =
            std::thread::spawn(move || write_frame(&mut snapshot_writer, &snapshot).unwrap());
        let returned: ServiceSnapshot = read_frame(&mut host).unwrap();
        writer.join().unwrap();
        assert_eq!(returned.input.remote_wallpapers[&peer][&display_id], jpeg);
    }

    #[test]
    fn quality_crosses_status_ipc_without_debug_text() {
        use super::*;
        let record = deskfuse_transport::motion_diag::QualityRecord {
            timestamp_ms: 123_000,
            rtt_micros: 4_000,
            hitches: 2,
            slow_injections: 1,
            capture_queue_micros: 9_000,
            capture_count: 1,
            ready: false,
        };
        let quality = InputServiceQuality {
            records: vec![record],
            report: deskfuse_input::quality::build_report(&[], "reliable-only", None),
        };
        let snapshot = ServiceSnapshot {
            generation: 1,
            native: NativeStatus::default(),
            input: Default::default(),
            diagnostics: String::new(),
            quality: quality.clone(),
            last_command: 0,
        };
        let (mut host, mut child) = UnixStream::pair().unwrap();
        write_frame(&mut child, &snapshot).unwrap();
        let returned: ServiceSnapshot = read_frame(&mut host).unwrap();
        assert!(returned.diagnostics.is_empty());
        assert_eq!(returned.quality, quality);

        // Older service status frames still decode during a rolling upgrade.
        let mut old = serde_json::to_value(&snapshot).unwrap();
        old.as_object_mut().unwrap().remove("quality");
        let decoded: ServiceSnapshot = serde_json::from_value(old).unwrap();
        assert_eq!(decoded.quality, InputServiceQuality::default());
    }

    #[test]
    fn observer_exhaustion_requests_process_recovery_but_ordinary_errors_do_not() {
        use super::*;
        let control = ServiceControl::default();
        control.status.lock().unwrap().error = Some("permission denied".into());
        assert!(!process_recovery_required(&control));
        control.status.lock().unwrap().observer_recovery_required = true;
        assert!(process_recovery_required(&control));
        *control.status.lock().unwrap() = NativeStatus::default();
        assert!(!process_recovery_required(&control));
    }

    #[tokio::test]
    async fn exhausted_service_retries_can_be_restarted_without_changing_settings() {
        use super::*;
        let dir = scratch();
        let control = ServiceControl::default();
        let (commands, command_rx) = mpsc::channel(8);
        let (snapshots, _) = watch::channel(InputSharingSnapshot::default());
        let (_network, network) = watch::channel(SnapshotView {
            snapshot: NetworkSnapshot::new(0, DeviceId::from_public_key([9; 32]).unwrap(), vec![])
                .unwrap(),
            discovered_peers: vec![],
            device_network: Default::default(),
        });
        let store = Arc::new(crate::MemoryRuntimeStore::default());
        let settings = store.get_input_sharing_settings().unwrap();
        let (events, _) = broadcast::channel(16);
        // A missing executable exercises real supervisor failure/backoff without
        // launching native input, touching identity, or requesting OS permissions.
        let supervisor = tokio::spawn(run_host(
            ServiceOptions {
                executable: dir.join("missing-helper"),
                identity_path: dir.join("unused-identity"),
            },
            "test".into(),
            control.clone(),
            command_rx,
            snapshots,
            network,
            store.clone(),
            events,
        ));
        let exhausted = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if control
                    .status
                    .lock()
                    .unwrap()
                    .error
                    .as_deref()
                    .is_some_and(|e| e.contains("自动恢复未成功"))
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        if exhausted.is_err() {
            supervisor.abort();
            panic!("supervisor did not expose retry exhaustion");
        }
        let sequence = *control.sequence.lock().unwrap();
        commands
            .send(RuntimeCommand::RefreshNativeObserver)
            .await
            .unwrap();
        let retried = tokio::time::timeout(Duration::from_secs(2), async {
            while *control.sequence.lock().unwrap() <= sequence {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        commands.send(RuntimeCommand::Shutdown).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), supervisor)
            .await
            .unwrap()
            .unwrap();
        assert!(
            retried.is_ok(),
            "manual refresh must restart an exhausted helper"
        );
        assert_eq!(store.get_input_sharing_settings().unwrap(), settings);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn release_json_formats_version_and_build() {
        let dir = scratch();
        let path = dir.join("release.json");
        fs::write(
            &path,
            r#"{"version":"0.4.2","build_number":20,"release_id":"0d7ed72f-e679-4d79-a8fc-2a77020338f6"}"#,
        )
        .unwrap();
        assert_eq!(read_release_identity(&path).as_deref(), Some("0.4.2-20"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn sidecar_trims_one_line_and_rejects_empty() {
        let dir = scratch();
        let path = dir.join("build-id");
        fs::write(&path, "dev-abc123\n").unwrap();
        assert_eq!(read_sidecar(&path).as_deref(), Some("dev-abc123"));
        fs::write(&path, " \n").unwrap();
        assert_eq!(read_sidecar(&path), None);
        assert_eq!(accept_identity("0.4.2-20"), Some("0.4.2-20".to_string()));
        assert_eq!(accept_identity("has space"), None);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn authority_preserves_control_connection_for_input_gate() {
        use crate::{NetworkSnapshot, SnapshotView, input_service::Authority};
        use deskfuse_core::{
            Capabilities, ConnectionState, DeviceId, Endpoint, LivenessState, PeerSnapshot,
            RouteCandidate, RouteHealth, RouteSource, TrustState,
        };
        let local = DeviceId::from_public_key([1; 32]).unwrap();
        let peer = DeviceId::from_public_key([2; 32]).unwrap();
        let route = RouteCandidate::new(
            Endpoint::new("10.0.0.2", 42424).unwrap(),
            RouteSource::Mdns,
            RouteHealth::Healthy,
            Capabilities::INPUT_SHARING,
        );
        let snapshot = NetworkSnapshot::new(
            1,
            local,
            vec![
                PeerSnapshot::new(
                    peer,
                    "peer",
                    [2; 32],
                    LivenessState::Online,
                    TrustState::Trusted,
                    ConnectionState::Connected,
                    vec![route],
                    None,
                )
                .unwrap(),
            ],
        )
        .unwrap();
        let view = SnapshotView {
            snapshot,
            discovered_peers: vec![],
            device_network: Default::default(),
        };
        assert_eq!(
            Authority::from_view(&view).peers[0].connection,
            ConnectionState::Connected
        );
    }
}
#[derive(Clone, Debug)]
pub struct ServiceOptions {
    pub executable: PathBuf,
    pub identity_path: PathBuf,
}
pub use super::input_service_types::{InputServiceQuality, NativeStatus, ServiceControl};
// The authority cache contains no clipboard, transfer or other business snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Authority {
    pub local: DeviceId,
    pub network: DeviceNetworkSnapshot,
    pub peers: Vec<PeerSnapshot>,
}
impl Authority {
    fn from_view(view: &SnapshotView) -> Self {
        let mut network = view.device_network.clone();
        network.pending.clear();
        let peers = view
            .snapshot
            .peers
            .iter()
            .cloned()
            .map(|mut p| {
                // The input socket is separate, but mouse sharing follows the
                // host device session. Preserve the control state so the input
                // actor can release leases when the device goes offline.
                p.liveness = deskfuse_core::LivenessState::Unknown;
                p.selected_route = None;
                p.routes.retain(|r| !r.source.is_public());
                for route in &mut p.routes {
                    route.health = deskfuse_core::RouteHealth::Unknown;
                    route.last_success_unix_ms = None;
                    route.failure_count = 0;
                    route.path_kind = None;
                    route.rtt_ms = None;
                    route.last_probe_unix_ms = None;
                    route.probe_error = None;
                }
                p
            })
            .collect();
        Self {
            local: view.snapshot.local_device_id,
            network,
            peers,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredInput {
    pub settings: InputSharingSettings,
    pub scenes: Vec<InputSharingScene>,
    pub policy: Option<InputActivationPolicy>,
}
impl StoredInput {
    fn load(store: &dyn RuntimeStore) -> Result<Self, RuntimeError> {
        Ok(Self {
            settings: store.get_input_sharing_settings()?,
            scenes: store.get_input_sharing_scenes()?,
            policy: store.get_input_activation_policy()?,
        })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
enum Persist {
    Settings(InputSharingSettings),
    Scenes(Vec<InputSharingScene>),
    Policy(InputActivationPolicy),
    SettingsPolicy(InputSharingSettings, InputActivationPolicy),
}
impl Persist {
    fn apply(&self, store: &dyn RuntimeStore) -> Result<(), RuntimeError> {
        match self {
            Self::Settings(v) => store.set_input_sharing_settings(v.clone()),
            Self::Scenes(v) => store.set_input_sharing_scenes(v.clone()),
            Self::Policy(v) => store.set_input_activation_policy(v.clone()),
            Self::SettingsPolicy(s, p) => {
                store.set_input_sharing_settings_and_policy(s.clone(), p.clone())
            }
        }
    }
}
#[derive(Serialize, Deserialize)]
pub struct Bootstrap {
    pub version: u32,
    pub build: String,
    pub generation: u64,
    pub identity_path: PathBuf,
    pub display_name: String,
    pub authority: Authority,
    pub stored: StoredInput,
}
#[derive(Serialize, Deserialize)]
// The bootstrap variant is the largest, but it crosses the handle only once per
// session; boxing it would add an allocation to the hot IPC path.
#[allow(clippy::large_enum_variant)]
enum HostMessage {
    Bootstrap(Bootstrap),
    RemoteApplication(super::input_service_types::RemoteApplicationInput),
    Authority { revision: u64, value: Authority },
    Command { id: u64, value: RuntimeCommand },
    Persisted { id: u64, error: Option<String> },
}
#[derive(Serialize, Deserialize)]
enum ChildRequest {
    Persist { id: u64, value: Persist },
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct ServiceSnapshot {
    pub generation: u64,
    pub native: NativeStatus,
    pub input: InputSharingSnapshot,
    pub diagnostics: String,
    #[serde(default)]
    pub quality: InputServiceQuality,
    pub last_command: u64,
}
fn write_frame<T: Serialize>(stream: &mut UnixStream, value: &T) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    if bytes.len() > MAX_FRAME {
        return Err(io::Error::other("input IPC frame too large"));
    }
    stream.write_all(&(bytes.len() as u32).to_be_bytes())?;
    stream.write_all(&bytes)
}
fn read_frame<T: serde::de::DeserializeOwned>(stream: &mut UnixStream) -> io::Result<T> {
    let mut len = [0; 4];
    stream.read_exact(&mut len)?;
    let len = u32::from_be_bytes(len) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(io::Error::other("invalid input IPC frame size"));
    }
    let mut bytes = vec![0; len];
    stream.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}
fn host_command_allowed(command: &RuntimeCommand) -> bool {
    matches!(
        command,
        RuntimeCommand::ObserveWifiEnvironment { .. }
            | RuntimeCommand::ChooseEnvironmentController { .. }
            | RuntimeCommand::ForgetControllerEnvironment { .. }
            | RuntimeCommand::SaveInputSharingScene { .. }
            | RuntimeCommand::SaveEnvironmentLayout { .. }
            | RuntimeCommand::ApplyInputSharingScene { .. }
            | RuntimeCommand::RenameInputSharingScene { .. }
            | RuntimeCommand::DeleteInputSharingScene { .. }
            | RuntimeCommand::SetInputSharingSettings(_)
            | RuntimeCommand::RestartInputActivation { .. }
            | RuntimeCommand::ForgetInputPeer { .. }
            | RuntimeCommand::RefreshNativeObserver
            | RuntimeCommand::PublishDisplayWallpaper { .. }
            | RuntimeCommand::MoveInputDevice { .. }
            | RuntimeCommand::PauseManagedActivation { .. }
            | RuntimeCommand::PauseCurrentManagedActivation
    )
}
#[cfg(test)]
#[test]
fn wifi_observation_can_cross_host_input_ipc() {
    let command = RuntimeCommand::ObserveWifiEnvironment { ssid: None };
    assert!(command.is_input());
    assert!(host_command_allowed(&command));
}
#[cfg(test)]
#[test]
fn forgotten_peer_cleanup_can_cross_host_input_ipc() {
    let command = RuntimeCommand::ForgetInputPeer {
        peer_id: DeviceId::from_public_key([7; 32]).unwrap(),
    };
    assert!(command.is_input());
    assert!(host_command_allowed(&command));
}

fn process_recovery_required(control: &ServiceControl) -> bool {
    control
        .status
        .lock()
        .is_ok_and(|status| status.observer_recovery_required)
}

/// The input service is the only component that can capture and inject input.
/// Giving up after a few crashes leaves sharing dead until the user restarts
/// the app, so keep retrying at a capped interval instead.
fn input_service_retry_delay(attempts: u32) -> Duration {
    match attempts {
        0 | 1 => Duration::from_secs(1),
        2 => Duration::from_secs(2),
        3 => Duration::from_secs(4),
        4 => Duration::from_secs(8),
        5 => Duration::from_secs(16),
        _ => Duration::from_secs(30),
    }
}

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_host(
    options: ServiceOptions,
    display_name: String,
    control: ServiceControl,
    mut commands: mpsc::Receiver<RuntimeCommand>,
    snapshots: watch::Sender<InputSharingSnapshot>,
    mut network: watch::Receiver<SnapshotView>,
    store: Arc<dyn RuntimeStore>,
    events: broadcast::Sender<RuntimeEvent>,
) {
    let mut attempts = 0u32;
    let mut pending = std::collections::VecDeque::new();
    loop {
        let generation = {
            let mut value = control.sequence.lock().unwrap_or_else(|e| e.into_inner());
            *value += 1;
            *value
        };
        let authority = Authority::from_view(&network.borrow());
        let launched_at = std::time::Instant::now();
        let result = run_process(
            &options,
            &display_name,
            generation,
            &control,
            &mut commands,
            &snapshots,
            &mut network,
            store.clone(),
            &events,
            authority,
            &mut pending,
        )
        .await;
        *control.sequence.lock().unwrap_or_else(|e| e.into_inner()) += 1;
        *control.urgent.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *control.remote.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *control.quality.lock().unwrap_or_else(|e| e.into_inner()) = Default::default();
        let mut error = result.err();
        // Count consecutive failures, not every incident over the app's lifetime.
        if launched_at.elapsed() >= Duration::from_secs(60) {
            attempts = 0;
        }
        if error.is_some() {
            attempts += 1;
            if attempts > 3 {
                error = Some("输入服务自动恢复未成功，正在后台重试；可点击重试立即恢复".into());
            }
        }
        let status = NativeStatus {
            error: error.clone(),
            ..Default::default()
        };
        *control.status.lock().unwrap_or_else(|e| e.into_inner()) = status.clone();
        let _ = events.send(RuntimeEvent::InputServiceStatus(status));
        let mut unavailable = snapshots.borrow().clone();
        unavailable.managed_control = None;
        unavailable.remote_topologies.clear();
        for state in unavailable.activations.values_mut() {
            state.status = InputActivationStatus::Unavailable;
            state.reason = Some("独立输入进程已停止".into());
        }
        for link in &mut unavailable.links {
            link.ready = false;
            link.control_session = None;
            link.context = None;
            link.reason = Some("独立输入进程已停止".into());
        }
        snapshots.send_replace(unavailable);
        if error.is_none() {
            break;
        }
        // No hidden fallback into the application's process. Retry forever at a
        // capped interval; explicit restart commands below relaunch immediately.
        let delay = tokio::time::sleep(input_service_retry_delay(attempts));
        tokio::pin!(delay);
        loop {
            tokio::select! {
                _ = &mut delay => break,
                c = commands.recv() => match c {
                    None | Some(RuntimeCommand::Shutdown) => return,
                    Some(value @ RuntimeCommand::RefreshNativeObserver)
                    | Some(value @ RuntimeCommand::RestartInputActivation { .. })
                    | Some(value @ RuntimeCommand::SetInputSharingSettings(_)) => {
                        pending.push_back(value);
                        attempts = 0;
                        break;
                    }
                    Some(value) if host_command_allowed(&value) && pending.len() < 128 => pending.push_back(value),
                    Some(_) => { let _ = events.send(RuntimeEvent::Error(RuntimeError::InvalidCommand("输入命令未应用，请重试".into()))); },
                }
            }
        }
    }
}
#[allow(clippy::too_many_arguments)]
async fn run_process(
    options: &ServiceOptions,
    display_name: &str,
    generation: u64,
    control: &ServiceControl,
    commands: &mut mpsc::Receiver<RuntimeCommand>,
    snapshots: &watch::Sender<InputSharingSnapshot>,
    network: &mut watch::Receiver<SnapshotView>,
    store: Arc<dyn RuntimeStore>,
    events: &broadcast::Sender<RuntimeEvent>,
    mut authority: Authority,
    pending: &mut std::collections::VecDeque<RuntimeCommand>,
) -> Result<(), String> {
    let (mut command, child_command) = UnixStream::pair().map_err(|e| e.to_string())?;
    let (mut status, child_status) = UnixStream::pair().map_err(|e| e.to_string())?;
    let (urgent, child_urgent) = UnixStream::pair().map_err(|e| e.to_string())?;
    urgent
        .set_write_timeout(Some(Duration::from_millis(10)))
        .map_err(|e| e.to_string())?;
    command
        .set_write_timeout(Some(Duration::from_secs(1)))
        .map_err(|e| e.to_string())?;
    let mut process = Process(
        Command::new(&options.executable)
            .arg("--input-service")
            .stdin(Stdio::from(std::os::fd::OwnedFd::from(child_command)))
            .stdout(Stdio::from(std::os::fd::OwnedFd::from(child_status)))
            .stderr(Stdio::from(std::os::fd::OwnedFd::from(child_urgent)))
            .spawn()
            .map_err(|e| format!("无法启动独立输入进程：{e}"))?,
    );
    *control.urgent.lock().unwrap_or_else(|e| e.into_inner()) = Some(urgent);
    let bootstrap = Bootstrap {
        version: VERSION,
        build: build_identity().to_string(),
        generation,
        identity_path: options.identity_path.clone(),
        display_name: display_name.into(),
        authority: authority.clone(),
        stored: StoredInput::load(store.as_ref()).map_err(|e| e.to_string())?,
    };
    write_frame(&mut command, &HostMessage::Bootstrap(bootstrap)).map_err(|e| e.to_string())?;
    let writer = Arc::new(Mutex::new(command.try_clone().map_err(|e| e.to_string())?));
    let (remote_tx, remote_rx) = std::sync::mpsc::sync_channel(128);
    *control.remote.lock().map_err(|e| e.to_string())? = Some(remote_tx);
    let remote_writer = writer.clone();
    std::thread::Builder::new()
        .name("deskfuse-remote-input-writer".into())
        .spawn(move || {
            while let Ok(packet) = remote_rx.recv() {
                let Ok(mut writer) = remote_writer.lock() else {
                    break;
                };
                if write_frame(&mut writer, &HostMessage::RemoteApplication(packet)).is_err() {
                    break;
                }
            }
        })
        .map_err(|e| e.to_string())?;
    let rpc_writer = writer.clone();
    std::thread::Builder::new()
        .name("deskfuse-input-persistence".into())
        .spawn(move || {
            while let Ok(ChildRequest::Persist { id, value }) = read_frame(&mut command) {
                let error = value.apply(store.as_ref()).err().map(|e| e.to_string());
                let Ok(mut writer) = rpc_writer.lock() else {
                    break;
                };
                if write_frame(&mut writer, &HostMessage::Persisted { id, error }).is_err() {
                    break;
                }
            }
        })
        .map_err(|e| e.to_string())?;
    let output = snapshots.clone();
    let output_events = events.clone();
    let output_control = control.clone();
    let (closed_tx, mut closed) = mpsc::channel(1);
    std::thread::Builder::new()
        .name("deskfuse-input-status".into())
        .spawn(move || {
            while let Ok(update) = read_frame::<ServiceSnapshot>(&mut status) {
                let current = output_control
                    .sequence
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                if update.generation != generation || *current != generation {
                    break;
                }
                let _ = output_events.send(RuntimeEvent::InputServiceStatus(update.native.clone()));
                *output_control
                    .status
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = update.native;
                *output_control
                    .diagnostics
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = update.diagnostics;
                *output_control
                    .quality
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = update.quality;
                output.send_replace(update.input);
            }
            let _ = closed_tx.blocking_send(());
        })
        .map_err(|e| e.to_string())?;
    let mut revision = 0;
    while let Some(value) = pending.front() {
        revision += 1;
        write_frame(
            &mut *writer.lock().map_err(|e| e.to_string())?,
            &HostMessage::Command {
                id: revision,
                value: value.clone(),
            },
        )
        .map_err(|e| e.to_string())?;
        pending.pop_front();
    }
    let mut tick = tokio::time::interval(Duration::from_millis(50));
    loop {
        tokio::select! {
            c = commands.recv() => match c {
                Some(RuntimeCommand::Shutdown) | None => {
                    control.signal(b'Q');
                    let deadline = std::time::Instant::now() + Duration::from_secs(1);
                    while std::time::Instant::now() < deadline {
                        if process.0.try_wait().map_err(|e|e.to_string())?.is_some() { return Ok(()); }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                    return Ok(());
                }
                Some(value) if host_command_allowed(&value) => {
                    revision += 1;
                    let mut writer = writer.lock().map_err(|e|e.to_string())?;
                    write_frame(&mut writer, &HostMessage::Command { id: revision, value }).map_err(|e|e.to_string())?;
                }
                _ => {},
            },
            changed = network.changed() => {
                if changed.is_err() { control.signal(b'Q'); return Ok(()); }
                let next = Authority::from_view(&network.borrow_and_update());
                if next != authority {
                    authority = next; revision += 1;
                    let mut writer = writer.lock().map_err(|e|e.to_string())?;
                    write_frame(&mut writer, &HostMessage::Authority { revision, value: authority.clone() }).map_err(|e|e.to_string())?;
                }
            },
            _ = tick.tick() => {
                if let Some(exit) = process.0.try_wait().map_err(|e|e.to_string())? { return Err(format!("独立输入进程已退出：{exit}")); }
                if process_recovery_required(control) {
                    // The coordinator already retired native sessions. Do not send
                    // the user-emergency command: that would persist a managed pause.
                    // Bound cleanup even if an OS query never returns.
                    control.signal(b'Q');
                    let deadline = std::time::Instant::now() + Duration::from_secs(2);
                    while std::time::Instant::now() < deadline {
                        if process.0.try_wait().map_err(|e|e.to_string())?.is_some() { break; }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    return Err("原生输入检查持续无响应，正在重启输入服务".into());
                }
                control.signal(if deskfuse_core::debug_logging::enabled() { b'D' } else { b'd' });
            },
            _ = closed.recv() => return Err("独立输入进程通信已中断".into()),
        }
    }
}
