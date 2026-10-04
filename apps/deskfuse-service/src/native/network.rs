//! Service-owned control and input transports; no GUI identity/runtime fallback.
use super::{file_transfer, remote_app};
use deskfuse_core::{
    Capabilities, ConnectionState, NetworkConfig, RequestId, RouteHealth, TrustState,
};
use deskfuse_discovery::{DiscoveryHandle, DiscoveryProvider, PlatformDiscovery, ServiceRecord};
use deskfuse_identity::{TrustStore, WindowsServiceDataDirectory, WindowsServiceIdentityProvider};
use deskfuse_runtime::{
    ControlTransportAdapter, InputOnlyRuntime, RuntimeBuilder, RuntimeConfig, RuntimeHandle,
    SnapshotView,
};
use deskfuse_store::{PeerStore, SQLiteStore, SettingsStore};
use deskfuse_transport::{ControlTransport, PeerHello, QuinnTransport, QuinnTransportConfig};
use std::{
    collections::BTreeSet,
    io,
    net::SocketAddr,
    sync::{Arc, Mutex},
};

const CONTROL_PORT: u16 = 42424;
const INPUT_PORT: u16 = 42425;
const INPUT_CAPABILITIES: Capabilities = Capabilities::from_bits(
    Capabilities::INPUT_SHARING.bits()
        | Capabilities::DEVICE_NETWORK.bits()
        | Capabilities::INPUT_ACTIVATION.bits()
        | Capabilities::INPUT_MOTION_DATAGRAM_V1.bits(),
);

pub struct Network {
    pub runtime: RuntimeHandle,
    pub input_runtime: InputOnlyRuntime,
    /// Shared settings store for desktop-owned catalogs. The service remains
    /// the only writer; the WPF process accesses it through authenticated IPC.
    pub store: SQLiteStore,
    pub workflow_catalog: super::workflow::Catalog,
    pub control_port: u16,
    input_transport: QuinnTransport,
    remote_app_transport: QuinnTransport,
    input_authorized: Mutex<BTreeSet<deskfuse_core::DeviceId>>,
    _input_discovery: Mutex<PlatformDiscovery>,
    input_discovery_events: Mutex<DiscoveryHandle>,
    // Keep the parent directories and exclusive runtime lease pinned for SQLite.
    _directory: WindowsServiceDataDirectory,
}
impl Network {
    pub async fn start() -> io::Result<Self> {
        let directory = WindowsServiceDataDirectory::open().map_err(|_| stage(20))?;
        let provider = WindowsServiceIdentityProvider::new();
        let identity = std::sync::Arc::new(provider.load_or_create().map_err(|_| stage(21))?);
        let store = SQLiteStore::open(directory.database_path()).map_err(|_| stage(22))?;
        store.list_peers().map_err(|_| stage(22))?;
        store.get_settings().map_err(|_| stage(22))?;
        TrustStore::list(&store).map_err(|_| stage(22))?;

        let cache = directory.transfer_cache_path();
        file_transfer::open_transfer_cache(&cache)?;
        file_transfer::set_cache_root(cache);
        let control_capabilities = Capabilities::NETWORK
            | Capabilities::DEVICE_NETWORK
            | Capabilities::CLIPBOARD
            | Capabilities::CLIPBOARD_SYNC_V2
            | Capabilities::DEFERRED_FILE_CLIPBOARD
            | Capabilities::REMOTE_APPLICATION
            | Capabilities::REMOTE_APPLICATION_RDP
            | Capabilities::WORKFLOW_SYNC;
        let config = RuntimeConfig {
            network: NetworkConfig::new()
                .with_mdns_enabled(true)
                .with_public_network_enabled(false),
            display_name: "DeskFuse Windows".into(),
            capabilities: control_capabilities,
            ..RuntimeConfig::default()
        };
        let control_transport =
            bind_transport(identity.as_ref(), control_capabilities, CONTROL_PORT, false)
                .await
                .map_err(|_| stage(24))?;
        let control_port = control_transport
            .local_addr()
            .map_err(|_| stage(24))?
            .port();
        // Remote-app sessions arrive on the control transport; keep a shared
        // handle so the service can serve them without the runtime's mediation.
        let remote_app_transport = control_transport.clone();
        let mut discovery = PlatformDiscovery::new(identity.device_id()).map_err(|_| stage(25))?;
        discovery
            .set_advertisement(
                ServiceRecord::v2(
                    identity.device_id(),
                    config.display_name.clone(),
                    control_capabilities,
                    control_port,
                )
                .map_err(|_| stage(25))?,
            )
            .map_err(|_| stage(25))?;
        let runtime = RuntimeBuilder::new()
            .config(config)
            .with_identity_provider(provider)
            .with_store(store.clone())
            .with_trust_store(store.clone())
            .with_external_discovery(discovery)
            .with_control_transport(control_transport)
            .build()
            .map_err(|_| stage(26))?;

        file_transfer::start_delivery(runtime.clone());
        let input_transport =
            bind_transport(identity.as_ref(), INPUT_CAPABILITIES, INPUT_PORT, true)
                .await
                .map_err(|_| stage(24))?;
        input_transport
            .configure_identity(identity.clone())
            .map_err(|_| stage(23))?;
        let input_port = input_transport.local_addr().map_err(|_| stage(24))?.port();
        let mut input_discovery =
            PlatformDiscovery::for_input_service(identity.device_id()).map_err(|_| stage(25))?;
        input_discovery
            .set_advertisement(
                ServiceRecord::v2(
                    identity.device_id(),
                    "DeskFuse Windows",
                    INPUT_CAPABILITIES,
                    input_port,
                )
                .map_err(|_| stage(25))?,
            )
            .map_err(|_| stage(25))?;
        let input_events = input_discovery.start().map_err(|_| stage(25))?;
        let initial = input_view(runtime.snapshot_view());
        let input_runtime = InputOnlyRuntime::start(
            std::sync::Arc::new(ControlTransportAdapter::new(input_transport.clone())),
            std::sync::Arc::new(store.clone()),
            initial,
        )
        .map_err(|_| stage(26))?;
        let workflow_catalog =
            super::workflow::Catalog::open(store.clone(), identity.device_id().to_string())
                .map_err(|_| stage(22))?;
        let network = Self {
            runtime,
            input_runtime,
            store,
            workflow_catalog,
            control_port,
            input_transport,
            remote_app_transport,
            input_authorized: Mutex::new(BTreeSet::new()),
            _input_discovery: Mutex::new(input_discovery),
            input_discovery_events: Mutex::new(input_events),
            _directory: directory,
        };
        network.sync_input_authority().await?;
        Ok(network)
    }

    pub async fn sync_input_authority(&self) -> io::Result<()> {
        if let Ok(events) = self.input_discovery_events.lock() {
            for _ in 0..64 {
                if events.try_recv().ok().flatten().is_none() {
                    break;
                }
            }
        }
        let view = input_view(self.runtime.snapshot_view());
        let allowed = view
            .snapshot
            .peers
            .iter()
            .map(|peer| peer.id)
            .collect::<BTreeSet<_>>();
        let previous = self
            .input_authorized
            .lock()
            .map_err(|_| io::Error::other("input authority lock poisoned"))?
            .clone();
        for id in previous.difference(&allowed) {
            self.input_transport
                .revoke_peer(*id)
                .await
                .map_err(io::Error::other)?;
        }
        for peer in &view.snapshot.peers {
            if !previous.contains(&peer.id) {
                self.input_transport
                    .restore_peer(peer.id)
                    .await
                    .map_err(io::Error::other)?;
                self.input_transport
                    .register_peer_identity(peer.id, peer.public_key)
                    .await
                    .map_err(io::Error::other)?;
            }
        }
        *self
            .input_authorized
            .lock()
            .map_err(|_| io::Error::other("input authority lock poisoned"))? = allowed;
        self.input_runtime.update_view(view);
        Ok(())
    }

    /// Drain and serve pending remote-app sessions from trusted peers. Incoming
    /// sessions are polled by the service loop; each is served on a blocking
    /// thread so its bounded `receive` waits never stall the Tokio worker.
    pub fn serve_remote_applications(&self) {
        while let Some(session) = self.remote_app_transport.take_remote_application() {
            let peer = session.peer;
            if remote_app::availability() == remote_app::Availability::Available
                && self.remote_app_authorized(peer)
            {
                let runtime = self.runtime.clone();
                tokio::task::spawn_blocking(move || {
                    remote_app::serve(session, || {
                        remote_app_authorized(&runtime.snapshot_view(), peer)
                    });
                });
            } else {
                session.close();
            }
        }
    }

    fn remote_app_authorized(&self, peer: deskfuse_core::DeviceId) -> bool {
        remote_app_authorized(&self.runtime.snapshot_view(), peer)
    }
}

fn remote_app_authorized(view: &SnapshotView, peer: deskfuse_core::DeviceId) -> bool {
    view.device_network
        .allows(view.snapshot.local_device_id, peer)
        && !view.device_network.blocked_peer_ids.contains(&peer)
        && view
            .snapshot
            .peers
            .iter()
            .any(|entry| entry.id == peer && entry.trust == TrustState::Trusted)
}

async fn bind_transport(
    identity: &deskfuse_identity::LocalIdentity,
    capabilities: Capabilities,
    port: u16,
    input: bool,
) -> Result<QuinnTransport, deskfuse_transport::TransportError> {
    let mut nonce = [0_u8; 32];
    nonce[..16].copy_from_slice(RequestId::new().into_uuid().as_bytes());
    nonce[16..].copy_from_slice(RequestId::new().into_uuid().as_bytes());
    let hello = PeerHello::signed(identity, "DeskFuse Windows", capabilities, nonce)?;
    let config = QuinnTransportConfig::new(
        SocketAddr::from(([0, 0, 0, 0], port)),
        SocketAddr::from(([127, 0, 0, 1], 0)),
        "deskfuse.local",
        include_bytes!("../../../../resources/deskfuse-transport-cert.der").to_vec(),
        include_bytes!("../../../../resources/deskfuse-transport-key.der").to_vec(),
        include_bytes!("../../../../resources/deskfuse-transport-cert.der").to_vec(),
    )
    .with_lan_port(Arc::new(deskfuse_platform::ports::FailClosedLanPort));
    QuinnTransport::bind(
        hello,
        if input {
            config.for_input_service()
        } else {
            config
        },
    )
    .await
}

fn input_view(mut view: SnapshotView) -> SnapshotView {
    view.device_network.pending.clear();
    let local = view.snapshot.local_device_id;
    view.snapshot.peers.retain(|peer| {
        peer.trust == TrustState::Trusted
            && view.device_network.allows(local, peer.id)
            && !view.device_network.blocked_peer_ids.contains(&peer.id)
    });
    for peer in &mut view.snapshot.peers {
        peer.connection = ConnectionState::Disconnected;
        peer.selected_route = None;
        peer.routes.retain(|route| !route.source.is_public());
        for route in &mut peer.routes {
            route.endpoint.port = INPUT_PORT;
            route.health = RouteHealth::Unknown;
            route.last_success_unix_ms = None;
            route.failure_count = 0;
            route.capabilities = INPUT_CAPABILITIES;
            route.rtt_ms = None;
            route.last_probe_unix_ms = None;
            route.probe_error = None;
        }
    }
    view.discovered_peers.clear();
    view
}

fn stage(code: i32) -> io::Error {
    io::Error::from_raw_os_error(code)
}
