//! Native client boundary. Callbacks are serialized on a dedicated dispatcher;
//! clients enqueue them and must not synchronously wait on subscribe/unsubscribe from a callback.
// UniFFI exports require owned arguments across the native language boundary.
#![allow(clippy::needless_pass_by_value)]
mod clipboard_paste;
mod microphone;
mod mouse_settings;
#[cfg(target_os = "macos")]
mod platform;
mod remote_application;
mod workflow;
pub use clipboard_paste::*;
mod clipboard_library;
#[cfg(target_os = "macos")]
mod lan_web;
mod types;
use deskfuse_core::{DeviceId, RequestId};
use deskfuse_engine::{
    diagnostics::Diagnostics,
    state::EngineState,
    tokio_bridge::{TokioBridge, TokioCommand, UiEvent},
};
use deskfuse_runtime::RuntimeCommand;
use deskfuse_transfer::TransferId;
use futures::{
    StreamExt,
    channel::mpsc::{UnboundedSender, unbounded},
};
use std::{
    collections::BTreeSet,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
};
pub use types::*;
pub use workflow::*;
uniffi::setup_scaffolding!();

/// Run the standalone input helper with the same native ports as the macOS app.
///
/// # Errors
/// Returns an error if inherited channels or the input runtime cannot start.
#[cfg(target_os = "macos")]
pub fn run_input_service() -> Result<(), String> {
    deskfuse_engine::input_service::main_with_platform(crate::platform::platform())
}

static RUNNING: AtomicBool = AtomicBool::new(false);
static STARTUP: OnceLock<Mutex<Option<deskfuse_update::UpdateStatus>>> = OnceLock::new();
fn failure(message: impl ToString) -> EngineError {
    EngineError::Failed {
        message: message.to_string(),
    }
}
fn device(id: &str) -> Result<DeviceId, EngineError> {
    id.parse().map_err(|_| failure("设备 ID 无效"))
}
fn transfer(id: &str) -> Result<TransferId, EngineError> {
    Ok(TransferId::from_uuid(
        uuid::Uuid::parse_str(id).map_err(failure)?,
    ))
}
fn request(id: &str) -> Result<RequestId, EngineError> {
    id.parse().map_err(|_| failure("配对请求 ID 无效"))
}

#[uniffi::export(callback_interface)]
pub trait EngineListener: Send + Sync {
    fn on_event(&self, event: EngineEvent);
}
enum Control {
    FlushSnapshot,
    Subscribe(Box<dyn EngineListener>, mpsc::Sender<()>),
    Unsubscribe(mpsc::Sender<()>),
    Stop(mpsc::Sender<()>),
    Workflow(deskfuse_platform_macos::WorkflowHotkeyEvent),
}
#[derive(uniffi::Object)]
pub struct Engine {
    #[cfg(target_os = "macos")]
    lan_web: deskfuse_engine::lan_web::LanWebService,
    /// Collaboration host. Clipboard I/O still runs in TokioBridge until cutover.
    #[allow(dead_code)]
    host: deskfuse_engine::host::EngineHost,
    bridge: Arc<TokioBridge>,
    workflows: Arc<deskfuse_engine::workflow::WorkflowService>,
    workflow_hotkeys: Arc<Mutex<deskfuse_platform_macos::WorkflowHotkeys>>,
    workflow_dispatch_stop: Arc<AtomicBool>,
    workflow_dispatch: Mutex<Option<thread::JoinHandle<()>>>,
    state: Arc<Mutex<EngineState>>,
    control: UnboundedSender<Control>,
    next_id: AtomicU64,
    stopped: AtomicBool,
    pending: Arc<Mutex<BTreeSet<u64>>>,
    worker: Mutex<Option<thread::JoinHandle<()>>>,
    shutdown_lock: Mutex<()>,
}
#[uniffi::export]
impl Engine {
    #[uniffi::constructor]
    #[allow(clippy::too_many_lines)] // Explicit native engine setup/command dispatch.
    /// # Errors
    /// Returns an error if an engine is already running, startup recovery fails, or a worker cannot start.
    pub fn new() -> Result<Arc<Self>, EngineError> {
        if STARTUP
            .get_or_init(|| Mutex::new(None))
            .lock()
            .map_err(failure)?
            .is_some()
        {
            return Err(failure("启动自检期间不能启动业务引擎"));
        }
        if RUNNING.swap(true, Ordering::AcqRel) {
            return Err(failure("本进程已有 DeskFuse 引擎"));
        }
        deskfuse_engine::diagnostics::install_panic_recorder();
        #[cfg(target_os = "macos")]
        let native_platform = crate::platform::platform();
        #[cfg(not(target_os = "macos"))]
        let native_platform = deskfuse_engine::platform::memory_platform();
        let (bridge, mut events) =
            match TokioBridge::start_with_platform(Diagnostics::new(1024), native_platform.clone())
            {
                Ok(value) => value,
                Err(e) => {
                    RUNNING.store(false, Ordering::Release);
                    return Err(failure(e));
                }
            };
        #[cfg(target_os = "macos")]
        let lan_web = deskfuse_engine::lan_web::LanWebService::new(&bridge).map_err(|e| {
            bridge.shutdown();
            RUNNING.store(false, Ordering::Release);
            failure(e)
        })?;
        bridge.record_startup_stage("engine_host");
        let host = match deskfuse_engine::host::EngineHost::assemble_with(
            deskfuse_store::SQLiteStore::default_path()
                .map_err(failure)?
                .parent()
                .ok_or_else(|| failure("state directory is missing"))?
                .to_path_buf(),
            #[cfg(target_os = "macos")]
            crate::platform::host_ports(
                deskfuse_store::SQLiteStore::default_path()
                    .map_err(failure)?
                    .parent()
                    .ok_or_else(|| failure("state directory is missing"))?,
            )
            .map_err(failure)?,
            #[cfg(not(target_os = "macos"))]
            deskfuse_platform::ports::HostPorts::memory(
                deskfuse_store::SQLiteStore::default_path()
                    .map_err(failure)?
                    .parent()
                    .ok_or_else(|| failure("state directory is missing"))?,
            ),
            native_platform.clone(),
        ) {
            Ok(host) => host,
            Err(e) => {
                bridge.shutdown();
                RUNNING.store(false, Ordering::Release);
                return Err(failure(e));
            }
        };
        bridge.record_startup_stage("engine_workflows");
        let workflows = deskfuse_engine::workflow::WorkflowService::new(
            bridge.clone(),
            host.workflow_execution(),
        )
        .map_err(|e| {
            bridge.shutdown();
            RUNNING.store(false, Ordering::Release);
            failure(e)
        })?;
        deskfuse_engine::workflow_sharing::attach(&workflows);
        let workflow_hotkeys = Arc::new(Mutex::new(
            deskfuse_platform_macos::WorkflowHotkeys::default(),
        ));
        if let Ok(mut keys) = workflow_hotkeys.lock() {
            keys.bindings = crate::workflow::hotkey_bindings(
                &workflows.list().map_err(failure)?,
                &deskfuse_core::InputShortcutBindings::default(),
            );
        }
        let state = Arc::new(Mutex::new(EngineState::default()));
        let event_state = state.clone();
        let pending = Arc::new(Mutex::new(BTreeSet::new()));
        let event_pending = pending.clone();
        let (control, mut controls) = unbounded();
        let (workflow_tx, workflow_rx) = mpsc::channel();
        let workflow_dispatch_stop = Arc::new(AtomicBool::new(false));
        let dispatcher_stop = workflow_dispatch_stop.clone();
        let dispatcher_control = control.clone();
        let workflow_dispatch = thread::Builder::new()
            .name("deskfuse-workflow-events".into())
            .spawn(move || {
                let mut last_flush = std::time::Instant::now();
                while !dispatcher_stop.load(Ordering::Acquire) {
                    if last_flush.elapsed() >= std::time::Duration::from_millis(100) {
                        let _ = dispatcher_control.unbounded_send(Control::FlushSnapshot);
                        last_flush = std::time::Instant::now();
                    }
                    match workflow_rx.recv_timeout(std::time::Duration::from_millis(100)) {
                        Ok(event) => {
                            if dispatcher_control
                                .unbounded_send(Control::Workflow(event))
                                .is_err()
                            {
                                break;
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(_) => break,
                    }
                }
            })
            .map_err(failure)?;
        let monitor_workflows = workflow_hotkeys.clone();
        let event_workflows = workflows.clone();
        let interception = bridge.paste_interception_state();
        let file_paste_capture = bridge.file_paste_capture();
        let event_bridge = bridge.clone();
        let worker=thread::Builder::new().name("deskfuse-native-events".into()).spawn(move||{
   let mut listener:Option<Box<dyn EngineListener>>=None;
   let mut paste_monitor=None;
   let mut snapshot_dirty=false;
   let mut quality_refresh:Option<std::time::Instant>=None;
   futures::executor::block_on(async move{
    loop {
     futures::select!{
      control=controls.next()=>match control {
       Some(Control::FlushSnapshot)=>{
        if quality_refresh.is_none_or(|at|at.elapsed()>=std::time::Duration::from_secs(1)) {
         let now=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis();
         event_state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).sample_idle_input_quality(now);
         quality_refresh=Some(std::time::Instant::now());snapshot_dirty=true;
        }
        if snapshot_dirty {if let Some(l)=&listener{let snapshot=EngineSnapshot::from(&*event_state.lock().unwrap_or_else(std::sync::PoisonError::into_inner));l.on_event(EngineEvent::Snapshot{snapshot});}snapshot_dirty=false;}
       },
       Some(Control::Subscribe(next,ack))=>{listener=Some(next);let snapshot=EngineSnapshot::from(&*event_state.lock().unwrap_or_else(std::sync::PoisonError::into_inner));if let Some(l)=&listener{l.on_event(EngineEvent::Snapshot{snapshot});}let _=ack.send(());},
       Some(Control::Unsubscribe(ack))=>{listener=None;let _=ack.send(());},
       Some(Control::Workflow(event))=>{if let Err(message)=event_workflows.start(event.id,deskfuse_engine::workflow::WorkflowInput::Clipboard,deskfuse_engine::workflow::WorkflowOutputMode::Configured,event.target_pid)&& let Some(l)=&listener{l.on_event(EngineEvent::CommandFailed{request_id:0,message});}},
       Some(Control::Stop(ack))=>{if let Some(l)=&listener{for id in std::mem::take(&mut *event_pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner)){l.on_event(EngineEvent::CommandFailed{request_id:id,message:"引擎已停止".into()});}}drop(paste_monitor.take());let _=ack.send(());break;},
       None=>break,
      },
      event=events.next()=>{
       let Some(event)=event else{break;};
       if let UiEvent::PermissionSnapshot(p)=&event {
        if p.accessibility().is_granted() && paste_monitor.is_none(){
         paste_monitor=deskfuse_platform_macos::MacOSPasteShortcutMonitor::start_with_file_capture(interception.0.clone(),interception.1.clone(),interception.2.clone(),monitor_workflows.clone(),workflow_tx.clone(),Some(file_paste_capture.clone())).ok();
        }else if !p.accessibility().is_granted(){paste_monitor=None;}
       }
       let notification=match &event{
        UiEvent::ClipboardLibraryChanged(revision)=>Some(EngineEvent::ClipboardLibraryChanged{revision:*revision}),
        UiEvent::CommandResult{request_id,error}=>{event_pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(request_id);Some(match error{Some(message)=>EngineEvent::CommandFailed{request_id:*request_id,message:message.clone()},None=>EngineEvent::CommandApplied{request_id:*request_id}})},
        UiEvent::RuntimeEvent(deskfuse_runtime::RuntimeEvent::PairingDecisionFailed{request_id,reason})=>Some(EngineEvent::PairingFailed{request_id:request_id.to_string(),message:reason.clone()}),
        UiEvent::PasteReady{target_pid,expected_change_count}=>Some(EngineEvent::PasteReady{target_pid:*target_pid,change_count:*expected_change_count}),
        UiEvent::WorkflowCatalogChanged=>Some(EngineEvent::WorkflowCatalogChanged),
        UiEvent::WorkflowRun(run)=>Some(EngineEvent::WorkflowRun{run:run.clone().into()}),
        UiEvent::StageFileShelf(paths)=>Some(EngineEvent::StageFiles{paths:paths.clone()}),
        UiEvent::Quit=>Some(EngineEvent::Quit),_=>None,
       };
       let refresh_keys=matches!(&event,UiEvent::WorkflowCatalogChanged|UiEvent::InputSharingSnapshot(_));
       let terminal=match &event{UiEvent::TransferStatus{transfer_id,state,..} if state.is_terminal()=>Some(*transfer_id),_=>None};
       let urgent=notification.is_some() || terminal.is_some();
       let snapshot={let mut state=event_state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);let changed=state.apply(event);
        if changed && refresh_keys && let (Ok(list),Ok(mut keys))=(event_workflows.list(),monitor_workflows.lock()) { keys.bindings=crate::workflow::hotkey_bindings(&list,&state.model.input_sharing.settings.shortcuts); }
        if urgent { snapshot_dirty=false; Some(EngineSnapshot::from(&*state)) } else { if changed { snapshot_dirty=true; } None }};
       if let Some(l)=&listener{if let Some(snapshot)=snapshot{let revision=snapshot.revision;l.on_event(EngineEvent::Snapshot{snapshot});if let Some(id)=terminal{event_bridge.record_transfer_presentation(id,revision);}}
        if let Some(event)=notification{l.on_event(event);}}
      }
     }
    }
   });
  }).map_err(|e|{bridge.shutdown();RUNNING.store(false,Ordering::Release);failure(e)})?;
        bridge.record_startup_stage("engine_ready");
        Ok(Arc::new(Self {
            #[cfg(target_os = "macos")]
            lan_web,
            host,
            workflows,
            workflow_hotkeys,
            workflow_dispatch_stop,
            workflow_dispatch: Mutex::new(Some(workflow_dispatch)),
            bridge,
            state,
            control,
            next_id: AtomicU64::new(1),
            stopped: AtomicBool::new(false),
            pending,
            worker: Mutex::new(Some(worker)),
            shutdown_lock: Mutex::new(()),
        }))
    }
    /// # Errors
    /// Returns an error if the engine is stopped or its state lock is poisoned.
    pub fn snapshot(&self) -> Result<EngineSnapshot, EngineError> {
        Ok(EngineSnapshot::from(&*self.state.lock().map_err(failure)?))
    }
    /// # Errors
    /// Returns an error if the engine is stopped or the callback dispatcher is unavailable.
    pub fn subscribe(&self, listener: Box<dyn EngineListener>) -> Result<(), EngineError> {
        self.active()?;
        let (tx, rx) = mpsc::channel();
        self.control
            .unbounded_send(Control::Subscribe(listener, tx))
            .map_err(failure)?;
        rx.recv_timeout(std::time::Duration::from_secs(15))
            .map_err(|error| {
                self.bridge.record_startup_stage("subscription_timeout");
                failure(format!(
                    "界面状态订阅未完成，请退出后重新打开 DeskFuse，并导出诊断日志：{error}"
                ))
            })
    }
    /// # Errors
    /// Returns an error if the callback dispatcher cannot acknowledge removal.
    pub fn unsubscribe(&self) -> Result<(), EngineError> {
        if self.stopped.load(Ordering::Acquire) {
            return Ok(());
        }
        let (tx, rx) = mpsc::channel();
        self.control
            .unbounded_send(Control::Unsubscribe(tx))
            .map_err(failure)?;
        rx.recv().map_err(failure)
    }
    #[allow(clippy::too_many_lines)] // Explicit native engine setup/command dispatch.
    /// # Errors
    /// Returns an error for invalid command data, a stopped engine or an unavailable worker queue.
    pub fn submit(&self, command: EngineCommand) -> Result<CommandReceipt, EngineError> {
        self.active()?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut transfer_id = None;
        let command = match command {
            EngineCommand::PublishDisplayWallpaper { display_id, jpeg } => {
                TokioCommand::Runtime(RuntimeCommand::PublishDisplayWallpaper {
                    display_id: deskfuse_core::DisplayId::new(display_id),
                    jpeg,
                })
            }
            EngineCommand::RefreshDevices => TokioCommand::Runtime(RuntimeCommand::RefreshPeers),
            EngineCommand::Pair { peer_id } => {
                TokioCommand::Runtime(RuntimeCommand::RequestPairing {
                    peer_id: device(&peer_id)?,
                })
            }
            EngineCommand::PairAt { ip } => {
                let ip: std::net::Ipv4Addr = ip
                    .trim()
                    .parse()
                    .map_err(|_| failure("请输入有效的 IPv4 地址"))?;
                TokioCommand::Runtime(RuntimeCommand::RequestPairingAt {
                    endpoint: deskfuse_core::Endpoint::new(ip.to_string(), 42424)
                        .map_err(failure)?,
                })
            }
            EngineCommand::Approve { request_id } => {
                TokioCommand::Runtime(RuntimeCommand::ApprovePairing {
                    request_id: request(&request_id)?,
                })
            }
            EngineCommand::Reject { request_id } => {
                TokioCommand::Runtime(RuntimeCommand::RejectPairing {
                    request_id: request(&request_id)?,
                    reason: None,
                })
            }
            EngineCommand::Forget { peer_id } => {
                TokioCommand::Runtime(RuntimeCommand::ForgetPeer {
                    peer_id: device(&peer_id)?,
                })
            }
            EngineCommand::Block { peer_id } => TokioCommand::Runtime(RuntimeCommand::RevokePeer {
                peer_id: device(&peer_id)?,
            }),
            EngineCommand::AllowPairing { peer_id } => {
                TokioCommand::Runtime(RuntimeCommand::AllowPairing {
                    peer_id: device(&peer_id)?,
                })
            }
            EngineCommand::Disconnect { peer_id } => {
                TokioCommand::Runtime(RuntimeCommand::Disconnect {
                    peer_id: device(&peer_id)?,
                })
            }
            EngineCommand::Connect { peer_id } => TokioCommand::Runtime(RuntimeCommand::Connect {
                peer_id: device(&peer_id)?,
            }),
            EngineCommand::LeaveNetwork => {
                TokioCommand::Runtime(RuntimeCommand::LeaveDeviceNetwork)
            }
            EngineCommand::SetDebugLogging { enabled } => TokioCommand::SetDebugLogging(enabled),
            EngineCommand::SetClipboard { enabled } => TokioCommand::SetClipboardSync(enabled),
            EngineCommand::PushClipboard => TokioCommand::PushClipboard,
            EngineCommand::RestoreHistory { id } => TokioCommand::RestoreClipboardHistory(id),
            EngineCommand::ClearHistory => TokioCommand::ClearClipboardHistory,
            EngineCommand::SendFiles { peer_id, paths } => {
                if paths.is_empty() {
                    return Err(failure("请先选择文件"));
                }
                let transfer = TransferId::new();
                transfer_id = Some(transfer.to_string());
                TokioCommand::SendFiles {
                    peer_id: device(&peer_id)?,
                    paths,
                    transfer_id: Some(transfer),
                }
            }
            EngineCommand::CancelTransfer { id } => {
                TokioCommand::CancelClipboardTransfer(transfer(&id)?)
            }
            EngineCommand::RetryTransfer { id } => TokioCommand::RetryFileTransfer(transfer(&id)?),
            EngineCommand::SetFiles { settings } => {
                let settings: deskfuse_core::FileTransferSettings = settings.into();
                settings.validate().map_err(failure)?;
                TokioCommand::SetFileTransferSettings(settings)
            }
            EngineCommand::ClearCache => TokioCommand::ClearTransferCache,
            EngineCommand::RefreshCache => TokioCommand::RefreshTransferCache,
            EngineCommand::MoveInputDevice {
                device_id,
                display_id,
                x,
                y,
            } => {
                if !x.is_finite() || !y.is_finite() {
                    return Err(failure("无效的屏幕位置"));
                }
                TokioCommand::Runtime(RuntimeCommand::MoveInputDevice {
                    device_id: device_id.parse().map_err(failure)?,
                    display_id: deskfuse_core::DisplayId::new(display_id),
                    x,
                    y,
                })
            }
            EngineCommand::SetInput { settings } => {
                for shortcut in &settings.shortcuts {
                    if crate::workflow::clipboard_shortcut_conflict(
                        shortcut.key_code,
                        shortcut.modifiers,
                    )? {
                        return Err(failure("该键鼠快捷键已用于唤起剪贴板"));
                    }
                }
                for workflow in self
                    .workflows
                    .list()
                    .map_err(failure)?
                    .iter()
                    .filter(|w| w.enabled)
                {
                    if let Some(chord) = &workflow.shortcut
                        && settings
                            .shortcuts
                            .iter()
                            .any(|s| s.key_code == chord.key_code && s.modifiers == chord.modifiers)
                    {
                        return Err(failure("该键鼠快捷键已被工作流占用"));
                    }
                }
                TokioCommand::Runtime(RuntimeCommand::SetInputSharingSettings(
                    settings.into_core(
                        &self
                            .state
                            .lock()
                            .map_err(failure)?
                            .model
                            .input_sharing
                            .settings,
                    )?,
                ))
            }
            EngineCommand::RestartInputActivation { peer_id } => {
                TokioCommand::Runtime(RuntimeCommand::RestartInputActivation {
                    peer_id: device(&peer_id)?,
                })
            }
            EngineCommand::SaveScene { name } => {
                TokioCommand::Runtime(RuntimeCommand::SaveInputSharingScene { name })
            }
            EngineCommand::SaveEnvironmentLayout { name } => {
                TokioCommand::Runtime(RuntimeCommand::SaveEnvironmentLayout { name })
            }
            EngineCommand::ApplyScene { id } => {
                TokioCommand::Runtime(RuntimeCommand::ApplyInputSharingScene { scene_id: id })
            }
            EngineCommand::RenameScene { id, name } => {
                TokioCommand::Runtime(RuntimeCommand::RenameInputSharingScene {
                    scene_id: id,
                    name,
                })
            }
            EngineCommand::DeleteScene { id } => {
                TokioCommand::Runtime(RuntimeCommand::DeleteInputSharingScene { scene_id: id })
            }
            EngineCommand::ChooseController { id, remember } => {
                TokioCommand::Runtime(RuntimeCommand::ChooseEnvironmentController {
                    controller: device(&id)?,
                    remember,
                })
            }
            EngineCommand::ForgetEnvironment { ssid } => {
                TokioCommand::Runtime(RuntimeCommand::ForgetControllerEnvironment { ssid })
            }
            EngineCommand::CheckUpdates => TokioCommand::CheckForUpdates,
            EngineCommand::DownloadAndInstallUpdate => TokioCommand::DownloadAndInstallUpdate,
            EngineCommand::TriggerPeerUpdate { peer_id } => TokioCommand::TriggerPeerUpdate {
                peer_id: device(&peer_id)?,
            },
            EngineCommand::TriggerPeerUpdates { peer_ids } => TokioCommand::TriggerPeerUpdates {
                peer_ids: peer_ids
                    .into_iter()
                    .map(|peer_id| device(&peer_id))
                    .collect::<Result<Vec<_>, _>>()?,
            },
            EngineCommand::RefreshPermissions => TokioCommand::RefreshPermissions,
        };
        self.pending.lock().map_err(failure)?.insert(id);
        if let Err(e) = self.bridge.submit_tracked(id, command) {
            self.pending.lock().map_err(failure)?.remove(&id);
            return Err(failure(e));
        }
        Ok(CommandReceipt {
            request_id: id,
            transfer_id,
        })
    }
    /// # Errors
    /// Returns an error if the engine is stopped or its state lock is poisoned.
    pub fn search_history(&self, query: String, filter: u8) -> Result<Vec<u64>, EngineError> {
        self.active()?;
        let state = self.state.lock().map_err(failure)?;
        Ok(state
            .model
            .clipboard_history
            .iter()
            .filter(|item| {
                deskfuse_engine::history_search::clipboard_matches(
                    item,
                    &query,
                    usize::from(filter),
                )
            })
            .map(|item| item.id)
            .collect())
    }
    /// # Errors
    /// Returns an error if the engine is stopped, the state lock is poisoned or the item no longer exists.
    pub fn content(&self, id: u64) -> Result<ClipboardContent, EngineError> {
        self.active()?;
        let item = deskfuse_engine::clipboard_library::global()
            .map_err(failure)?
            .content(id)
            .map_err(failure)?;
        let can_request_files = if item.mime == deskfuse_transfer::CLIPBOARD_FILE_MIME {
            deskfuse_engine::clipboard_library::global()
                .map_err(failure)?
                .snapshot(id)
                .ok()
                .and_then(|snapshot| {
                    deskfuse_engine::library_files::reference(&snapshot)
                        .ok()
                        .flatten()
                })
                .is_some()
        } else {
            false
        };
        Ok(ClipboardContent {
            can_request_files,
            id,
            text: item.content.clone(),
            truncated: item.content_truncated,
            image_png: item.image_png.as_deref().cloned().unwrap_or_default(),
            file_paths: item.file_paths.clone(),
        })
    }
    /// # Errors
    /// Returns an error for empty, unavailable or unretainable file paths, or a stopped engine.
    pub fn acquire_export(&self, paths: Vec<String>) -> Result<ExportLease, EngineError> {
        self.active()?;
        if paths.is_empty() {
            return Err(failure("没有可导出的文件"));
        }
        let id = self
            .bridge
            .retain_transfer_cache_paths(paths.clone())
            .map_err(failure)?;
        for path in &paths {
            if !std::path::Path::new(path).exists() {
                self.bridge.release_transfer_cache_paths(id);
                return Err(failure("文件已不可用，请重新获取"));
            }
        }
        Ok(ExportLease {
            id: id.to_string(),
            paths,
        })
    }
    /// # Errors
    /// Returns an error if the export lease identifier is invalid.
    pub fn release_export(&self, id: String, outcome: ExportOutcome) -> Result<(), EngineError> {
        let _ = outcome;
        self.bridge.release_transfer_cache_paths(transfer(&id)?);
        Ok(())
    }
    pub fn emergency_reclaim(&self) {
        self.bridge.pause_current_managed_activation();
    }
    pub fn record_startup_stage(&self, stage: String) {
        self.bridge.record_startup_stage(&stage);
    }

    /// 工作流入口由界面开关控制；关闭时全局工作流快捷键一并停用。
    pub fn set_workflow_hotkeys_enabled(&self, enabled: bool) {
        if let Ok(mut keys) = self.workflow_hotkeys.lock() {
            keys.disabled = !enabled;
        }
    }

    pub fn set_recording(&self, enabled: bool) {
        if let Ok(mut keys) = self.workflow_hotkeys.lock() {
            keys.recording = enabled;
        }
        self.bridge.set_input_recording(enabled);
    }
    /// # Errors
    /// Returns an error if the diagnostic export cannot be written to the requested path.
    pub fn export_diagnostics(&self, path: String) -> Result<(), EngineError> {
        self.bridge
            .export_diagnostics(std::path::Path::new(&path))
            .map_err(failure)
    }
    /// # Errors
    /// Returns an error if shutdown coordination is poisoned or the worker cannot terminate.
    pub fn shutdown(&self) -> Result<(), EngineError> {
        let _shutdown = self.shutdown_lock.lock().map_err(failure)?;
        if self.stopped.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        self.workflow_dispatch_stop.store(true, Ordering::Release);
        if let Ok(mut handle) = self.workflow_dispatch.lock()
            && let Some(handle) = handle.take()
        {
            let _ = handle.join();
        }
        #[cfg(target_os = "macos")]
        self.lan_web.shutdown();
        self.workflows.shutdown();
        self.bridge.pause_current_managed_activation();
        let result = self.bridge.shutdown_and_wait().map_err(failure);
        let (tx, rx) = mpsc::channel();
        if self.control.unbounded_send(Control::Stop(tx)).is_ok() {
            let _ = rx.recv();
        }
        if let Some(worker) = self.worker.lock().map_err(failure)?.take() {
            let _ = worker.join();
        }
        RUNNING.store(false, Ordering::Release);
        result
    }
}
impl Engine {
    fn active(&self) -> Result<(), EngineError> {
        if self.stopped.load(Ordering::Acquire) {
            Err(failure("引擎已停止"))
        } else {
            Ok(())
        }
    }
}
impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
#[uniffi::export]
/// # Errors
/// Returns an error if update recovery fails or the startup status lock is poisoned.
pub fn prepare_startup(transaction: Option<String>) -> Result<bool, EngineError> {
    let status = deskfuse_engine::app_update::prepare_startup(transaction).map_err(failure)?;
    let self_test = status.is_some();
    *STARTUP
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(failure)? = status;
    Ok(self_test)
}
#[uniffi::export]
/// # Errors
/// Returns an error if no update awaits acknowledgement, the status lock is poisoned or acknowledgement fails.
pub fn acknowledge_startup() -> Result<(), EngineError> {
    let status = STARTUP
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(failure)?;
    let status = status
        .as_ref()
        .ok_or_else(|| failure("没有待确认的启动事务"))?;
    deskfuse_engine::app_update::acknowledge_startup(status).map_err(failure)
}
#[uniffi::export]
/// # Errors
/// Returns the native permission request error.
pub fn request_permission(kind: PermissionKind) -> Result<(), EngineError> {
    deskfuse_platform_macos::request_permission(kind.into()).map_err(failure)
}
#[uniffi::export]
pub fn request_wifi_access() {
    deskfuse_platform_macos::wifi_environment::request_ssid_access();
}
#[uniffi::export]
/// # Errors
/// Returns an error when reading the pasteboard generation or posting the native paste shortcut fails.
pub fn replay_paste(
    target_pid: i32,
    current_pid: i32,
    change_count: i64,
) -> Result<bool, EngineError> {
    use deskfuse_platform::Pasteboard;
    if target_pid <= 0 || target_pid != current_pid {
        return Ok(false);
    }
    let current = deskfuse_platform_macos::MacOSPasteboard::new()
        .change_count()
        .map_err(failure)?;
    if current != change_count {
        return Ok(false);
    }
    deskfuse_platform_macos::post_paste_shortcut().map_err(failure)?;
    Ok(true)
}
