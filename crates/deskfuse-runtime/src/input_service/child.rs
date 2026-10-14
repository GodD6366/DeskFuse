use super::*;
use std::os::fd::AsFd;
use std::sync::atomic::AtomicU8;
/// Inherited descriptors are the only IPC entry points. No public local socket.
pub struct ChildConnection {
    pub remote: mpsc::Receiver<crate::input_service_types::RemoteApplicationInput>,
    pub bootstrap: Bootstrap,
    pub stopping: Arc<AtomicBool>,
    pub recording: Arc<AtomicBool>,
    pub emergency: Arc<AtomicBool>,
    pub pause: Arc<AtomicU8>,
    pub authority: watch::Receiver<Authority>,
    pub commands: mpsc::Receiver<(u64, RuntimeCommand)>,
    pub status: watch::Sender<ServiceSnapshot>,
    store: Arc<RemoteStore>,
}
fn inherited(fd: i32) -> io::Result<UnixStream> {
    let owned = match fd {
        0 => io::stdin().as_fd().try_clone_to_owned()?,
        1 => io::stdout().as_fd().try_clone_to_owned()?,
        2 => io::stderr().as_fd().try_clone_to_owned()?,
        _ => return Err(io::Error::other("unexpected input descriptor")),
    };
    Ok(UnixStream::from(owned))
}
impl ChildConnection {
    pub fn open() -> Result<Self, String> {
        let mut command = inherited(0).map_err(|e| e.to_string())?;
        let mut output = inherited(1).map_err(|e| e.to_string())?;
        let mut urgent = inherited(2).map_err(|e| e.to_string())?;
        let HostMessage::Bootstrap(bootstrap) =
            read_frame(&mut command).map_err(|e| e.to_string())?
        else {
            return Err("input service bootstrap required".into());
        };
        // 两边都按同一安装包解析构建号，已编译的主程序和输入进程可以一起复用。
        if bootstrap.version != VERSION || bootstrap.build != build_identity() {
            return Err("输入进程与主应用版本不匹配".into());
        }
        let stopping = Arc::new(AtomicBool::new(false));
        let recording = Arc::new(AtomicBool::new(false));
        let emergency = Arc::new(AtomicBool::new(false));
        let pause = Arc::new(AtomicU8::new(0));
        let (persist_tx, persist_rx) = std::sync::mpsc::sync_channel(128);
        let mut persist_writer = command.try_clone().map_err(|e| e.to_string())?;
        let store = Arc::new(RemoteStore {
            cache: Mutex::new(bootstrap.stored.clone()),
            writer: persist_tx,
            next: AtomicU64::new(0),
            pending: Mutex::new(BTreeSet::new()),
            error: Mutex::new(None),
        });
        let persistence = store.clone();
        std::thread::Builder::new()
            .name("deskfuse-input-store-writer".into())
            .spawn(move || {
                while let Ok(request) = persist_rx.recv() {
                    if let Err(error) = write_frame(&mut persist_writer, &request) {
                        *persistence.error.lock().unwrap_or_else(|e| e.into_inner()) =
                            Some(error.to_string());
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        let (authority_tx, authority) = watch::channel(bootstrap.authority.clone());
        let (command_tx, commands) = mpsc::channel(128);
        let (remote_tx, remote) = mpsc::channel(128);
        let signal = stopping.clone();
        let persistence = store.clone();
        std::thread::Builder::new()
            .name("deskfuse-input-control".into())
            .spawn(move || {
                let mut revision = 0;
                while let Ok(message) = read_frame(&mut command) {
                    match message {
                        HostMessage::RemoteApplication(packet) => {
                            if remote_tx.try_send(packet).is_err() {
                                break;
                            }
                        }
                        HostMessage::Authority {
                            revision: incoming,
                            value,
                        } if incoming > revision => {
                            revision = incoming;
                            authority_tx.send_replace(value);
                        }
                        HostMessage::Command { id, value }
                            if id > revision && host_command_allowed(&value) =>
                        {
                            revision = id;
                            if command_tx.try_send((id, value)).is_err() {
                                break;
                            }
                        }
                        HostMessage::Persisted { id, error } => {
                            persistence
                                .pending
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .remove(&id);
                            if error.is_some() {
                                *persistence.error.lock().unwrap_or_else(|e| e.into_inner()) =
                                    error;
                            }
                        }
                        _ => break,
                    }
                }
                signal.store(true, Ordering::Release);
            })
            .map_err(|e| e.to_string())?;
        let signal = stopping.clone();
        let rec = recording.clone();
        let reclaim = emergency.clone();
        let update = pause.clone();
        std::thread::Builder::new()
            .name("deskfuse-input-lifetime".into())
            .spawn(move || {
                let mut byte = [0];
                while urgent.read_exact(&mut byte).is_ok() {
                    match byte[0] {
                        b'Q' => break,
                        b'E' => reclaim.store(true, Ordering::Release),
                        b'R' => rec.store(true, Ordering::Release),
                        b'r' => rec.store(false, Ordering::Release),
                        b'V' => {
                            let _ =
                                update.compare_exchange(0, 6, Ordering::AcqRel, Ordering::Acquire);
                        }
                        b'v' => {
                            // The coordinator may acknowledge 6 -> 7 concurrently.
                            // One atomic update releases either viewer state.
                            let _ =
                                update.try_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                                    matches!(value, 6 | 7).then_some(0)
                                });
                        }
                        b'P' => {
                            let _ =
                                update.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire);
                        }
                        b'M' => {
                            let _ = update.try_update(Ordering::AcqRel, Ordering::Acquire, |v| {
                                matches!(v, 0 | 1).then_some(3)
                            });
                        }
                        b'p' => crate::input_service_types::resume_update_pause(&update),
                        b'D' => deskfuse_core::debug_logging::set_enabled(true),
                        b'd' => deskfuse_core::debug_logging::set_enabled(false),
                        _ => break,
                    }
                }
                reclaim.store(true, Ordering::Release);
                signal.store(true, Ordering::Release);
                // EOF is parent death, not a missed heartbeat. Give native
                // cleanup and the 1.5s leases a grace period, then guarantee
                // that a stuck child supervisor cannot remain orphaned.
                std::thread::sleep(Duration::from_secs(2));
                std::process::exit(0);
            })
            .map_err(|e| e.to_string())?;
        let (status, mut updates) = watch::channel(ServiceSnapshot {
            generation: bootstrap.generation,
            native: NativeStatus {
                running: true,
                pid: std::process::id(),
                ..Default::default()
            },
            input: Default::default(),
            diagnostics: String::new(),
            quality: Default::default(),
            last_command: 0,
        });
        let signal = stopping.clone();
        std::thread::Builder::new()
            .name("deskfuse-input-status-writer".into())
            .spawn(move || {
                // A blocked main application only blocks this thread. Producers replace
                // the watch value; they never wait for the GUI or IPC socket to drain.
                let mut previous: Option<ServiceSnapshot> = None;
                while !signal.load(Ordering::Acquire) {
                    let value = updates.borrow_and_update().clone();
                    if previous.as_ref() != Some(&value) {
                        if write_frame(&mut output, &value).is_err() {
                            signal.store(true, Ordering::Release);
                            break;
                        }
                        previous = Some(value);
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            bootstrap,
            remote,
            stopping,
            recording,
            emergency,
            pause,
            authority,
            commands,
            status,
            store,
        })
    }
    pub fn persistence_status(&self) -> (usize, Option<String>) {
        (
            self.store.pending.lock().map(|p| p.len()).unwrap_or(128),
            self.store.error.lock().ok().and_then(|e| e.clone()),
        )
    }
    pub fn start_runtime(
        &self,
        transport: Arc<dyn TransportProvider>,
        view: SnapshotView,
    ) -> Result<InputOnlyRuntime, RuntimeError> {
        InputOnlyRuntime::start(transport, self.store.clone(), view)
    }
}
/// Input actor only: no clipboard, business discovery actor, updater or database.
pub struct InputOnlyRuntime {
    pub handle: RuntimeHandle,
    view: watch::Sender<SnapshotView>,
}
impl InputOnlyRuntime {
    fn start(
        transport: Arc<dyn TransportProvider>,
        store: Arc<dyn RuntimeStore>,
        initial: SnapshotView,
    ) -> Result<Self, RuntimeError> {
        let (input_tx, input_rx) = mpsc::channel(256);
        let overflow = Arc::new(AtomicBool::new(false));
        let delivery = Arc::new(input_delivery::InputDelivery::new(overflow.clone()));
        let (snapshot_tx, snapshot_rx) = watch::channel(InputSharingSnapshot::default());
        let (view, network) = watch::channel(initial.clone());
        let (_, general) = watch::channel(initial.snapshot);
        let (events, _) = broadcast::channel(128);
        let worker_transport = transport.clone();
        let worker_delivery = delivery.clone();
        let worker_overflow = overflow.clone();
        let worker_events = events.clone();
        let worker_network = network.clone();
        let join = thread::Builder::new()
            .name("deskfuse-input-runtime".into())
            .spawn(move || {
                deskfuse_transport::input_executor::prioritize_current_thread();
                if let Ok(executor) = TokioRuntimeBuilder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    executor.block_on(input_sharing::run(
                        input_rx,
                        snapshot_tx,
                        worker_network,
                        worker_transport,
                        store,
                        worker_events,
                        worker_overflow,
                        worker_delivery,
                    ));
                }
            })
            .map_err(|e| RuntimeError::ThreadStart(e.to_string()))?;
        let handle = RuntimeHandle {
            input_service: None,
            clipboard_results: application_delivery::Receipts::default(),
            update_transport: transport,
            input_tx: input_tx.clone(),
            input_snapshot_rx: snapshot_rx,
            input_overflow: overflow,
            input_delivery: delivery,
            command_tx: input_tx,
            event_tx: events,
            snapshot_rx: general,
            view_rx: network,
            join: Arc::new(Mutex::new(Some(join))),
            shutdown_requested: Arc::new(AtomicBool::new(false)),
        };
        Ok(Self { handle, view })
    }
    pub fn update_view(&self, view: SnapshotView) {
        self.view.send_replace(view);
    }
}
struct RemoteStore {
    cache: Mutex<StoredInput>,
    writer: std::sync::mpsc::SyncSender<ChildRequest>,
    next: AtomicU64,
    pending: Mutex<BTreeSet<u64>>,
    error: Mutex<Option<String>>,
}
impl RemoteStore {
    fn persist(&self, value: Persist) -> Result<(), RuntimeError> {
        // Durable writes are off the actor and native hot path. Parent suspension
        // can delay ACKs but must never hold a mouse/lease turn hostage.
        let mut cache = self
            .cache
            .lock()
            .map_err(|e| RuntimeError::Store(e.to_string()))?;
        if let Some(error) = self
            .error
            .lock()
            .map_err(|e| RuntimeError::Store(e.to_string()))?
            .as_ref()
        {
            return Err(RuntimeError::Store(error.clone()));
        }
        let mut pending = self
            .pending
            .lock()
            .map_err(|e| RuntimeError::Store(e.to_string()))?;
        if pending.len() >= 128 {
            return Err(RuntimeError::Store(
                "input settings persistence queue full".into(),
            ));
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        self.writer
            .try_send(ChildRequest::Persist {
                id,
                value: value.clone(),
            })
            .map_err(|e| RuntimeError::Store(e.to_string()))?;
        pending.insert(id);
        match value {
            Persist::Settings(v) => cache.settings = v,
            Persist::Scenes(v) => cache.scenes = v,
            Persist::Policy(v) => cache.policy = Some(v),
            Persist::SettingsPolicy(s, p) => {
                cache.settings = s;
                cache.policy = Some(p);
            }
        }
        Ok(())
    }
    fn read(&self) -> Result<StoredInput, RuntimeError> {
        self.cache
            .lock()
            .map(|v| v.clone())
            .map_err(|e| RuntimeError::Store(e.to_string()))
    }
}
impl RuntimeStore for RemoteStore {
    fn get_input_sharing_settings(&self) -> Result<InputSharingSettings, RuntimeError> {
        Ok(self.read()?.settings)
    }
    fn get_input_sharing_scenes(&self) -> Result<Vec<InputSharingScene>, RuntimeError> {
        Ok(self.read()?.scenes)
    }
    fn get_input_activation_policy(&self) -> Result<Option<InputActivationPolicy>, RuntimeError> {
        Ok(self.read()?.policy)
    }
    fn set_input_sharing_settings(&self, v: InputSharingSettings) -> Result<(), RuntimeError> {
        self.persist(Persist::Settings(v))
    }
    fn set_input_sharing_scenes(&self, v: Vec<InputSharingScene>) -> Result<(), RuntimeError> {
        self.persist(Persist::Scenes(v))
    }
    fn set_input_activation_policy(&self, v: InputActivationPolicy) -> Result<(), RuntimeError> {
        self.persist(Persist::Policy(v))
    }
    fn set_input_sharing_settings_and_policy(
        &self,
        s: InputSharingSettings,
        p: InputActivationPolicy,
    ) -> Result<(), RuntimeError> {
        self.persist(Persist::SettingsPolicy(s, p))
    }
    fn list_peers(&self) -> Result<Vec<PeerRecord>, RuntimeError> {
        Ok(vec![])
    }
    fn upsert_peer(&self, _: PeerRecord) -> Result<(), RuntimeError> {
        Err(RuntimeError::InvalidCommand(
            "input service cannot persist peers".into(),
        ))
    }
    fn delete_peer(&self, _: DeviceId) -> Result<bool, RuntimeError> {
        Err(RuntimeError::InvalidCommand(
            "input service cannot delete peers".into(),
        ))
    }
    fn list_manual_endpoints(&self, _: DeviceId) -> Result<Vec<ManualEndpoint>, RuntimeError> {
        Ok(vec![])
    }
    fn get_settings(&self) -> Result<Settings, RuntimeError> {
        Ok(Settings::default())
    }
}
