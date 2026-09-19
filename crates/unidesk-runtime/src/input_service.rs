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
const MAX_FRAME: usize = 1024 * 1024;
pub const BUILD: &str = match option_env!("UNIDESK_INPUT_BUILD_ID") {
    Some(value) => value,
    None => env!("CARGO_PKG_VERSION"),
};
#[derive(Clone, Debug)]
pub struct ServiceOptions {
    pub executable: PathBuf,
    pub identity_path: PathBuf,
}
pub use super::input_service_types::{NativeStatus, ServiceControl};
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
                // Generic application connectivity is NOT an input authorization signal.
                p.connection = ConnectionState::Disconnected;
                p.liveness = unidesk_core::LivenessState::Unknown;
                p.selected_route = None;
                p.routes.retain(|r| !r.source.is_public());
                for route in &mut p.routes {
                    route.health = unidesk_core::RouteHealth::Unknown;
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
    pub routes: BTreeMap<DeviceId, unidesk_core::RoutePreference>,
}
impl StoredInput {
    fn load(store: &dyn RuntimeStore) -> Result<Self, RuntimeError> {
        Ok(Self {
            settings: store.get_input_sharing_settings()?,
            scenes: store.get_input_sharing_scenes()?,
            policy: store.get_input_activation_policy()?,
            routes: store.get_route_preferences()?,
        })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
enum Persist {
    Settings(InputSharingSettings),
    Scenes(Vec<InputSharingScene>),
    Policy(InputActivationPolicy),
    SettingsPolicy(InputSharingSettings, InputActivationPolicy),
    Routes(BTreeMap<DeviceId, unidesk_core::RoutePreference>),
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
            Self::Routes(v) => store.set_route_preferences(v.clone()),
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
        RuntimeCommand::ChooseEnvironmentController { .. }
            | RuntimeCommand::ForgetControllerEnvironment { .. }
            | RuntimeCommand::SaveInputSharingScene { .. }
            | RuntimeCommand::ApplyInputSharingScene { .. }
            | RuntimeCommand::RenameInputSharingScene { .. }
            | RuntimeCommand::DeleteInputSharingScene { .. }
            | RuntimeCommand::SetInputSharingSettings(_)
            | RuntimeCommand::RestartInputActivation { .. }
            | RuntimeCommand::MoveInputDevice { .. }
            | RuntimeCommand::PauseManagedActivation { .. }
            | RuntimeCommand::PauseCurrentManagedActivation
    )
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
        let error = result.err();
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
        attempts += 1;
        if attempts > 3 {
            // No hidden fallback into the application's process. Explicit enable retries.
            loop {
                match commands.recv().await {
                    Some(RuntimeCommand::Shutdown) | None => return,
                    Some(value @ RuntimeCommand::RestartInputActivation { .. })
                    | Some(value @ RuntimeCommand::SetInputSharingSettings(_)) => {
                        pending.push_back(value);
                        attempts = 0;
                        break;
                    }
                    Some(_) => {
                        let _ = events.send(RuntimeEvent::Error(RuntimeError::InvalidCommand(
                            "输入进程不可用，请重新开启共享后重试".into(),
                        )));
                    }
                }
            }
        } else {
            let delay = tokio::time::sleep(Duration::from_secs(1 << (attempts - 1)));
            tokio::pin!(delay);
            loop {
                tokio::select! {
                    _ = &mut delay => break,
                    c = commands.recv() => match c {
                        None | Some(RuntimeCommand::Shutdown) => return,
                        Some(value) if host_command_allowed(&value) && pending.len() < 128 => pending.push_back(value),
                        Some(_) => { let _ = events.send(RuntimeEvent::Error(RuntimeError::InvalidCommand("输入命令未应用，请重试".into()))); },
                    }
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
        build: BUILD.into(),
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
        .name("unidesk-remote-input-writer".into())
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
        .name("unidesk-input-persistence".into())
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
        .name("unidesk-input-status".into())
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
                control.signal(if unidesk_core::debug_logging::enabled() { b'D' } else { b'd' });
            },
            _ = closed.recv() => return Err("独立输入进程通信已中断".into()),
        }
    }
}
