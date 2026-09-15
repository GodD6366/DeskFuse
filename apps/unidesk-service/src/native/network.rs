//! Real service-owned transport/discovery; no GUI-owned identity/runtime fallback.
use std::{io, net::SocketAddr};
use unidesk_core::{Capabilities, NetworkConfig, RequestId};
use unidesk_discovery::{PlatformDiscovery, ServiceRecord};
use unidesk_identity::{TrustStore, WindowsServiceDataDirectory, WindowsServiceIdentityProvider};
use unidesk_runtime::{RuntimeBuilder, RuntimeConfig, RuntimeHandle};
use unidesk_store::{PeerStore, SQLiteStore, SettingsStore};
use unidesk_transport::{PeerHello, QuinnTransport, QuinnTransportConfig};

pub struct Network {
    pub runtime: RuntimeHandle,
    pub control_port: u16,
    // Keep the parent directories and exclusive runtime lease pinned for SQLite.
    _directory: WindowsServiceDataDirectory,
}
impl Network {
    pub async fn start() -> io::Result<Self> {
        let directory = WindowsServiceDataDirectory::open().map_err(|_| stage(20))?;
        let provider = WindowsServiceIdentityProvider::new();
        let identity = provider.load_or_create().map_err(|_| stage(21))?;
        let store = SQLiteStore::open(directory.database_path()).map_err(|_| stage(22))?;
        // Validate persistent authority before opening a network listener. The
        // actor normally reports load errors asynchronously; the service must
        // not start with a silently ignored malformed trust/state database.
        store.list_peers().map_err(|_| stage(22))?;
        store.get_settings().map_err(|_| stage(22))?;
        TrustStore::list(&store).map_err(|_| stage(22))?;
        // Desktop clipboard uses acknowledged v2 epochs; service drops all user
        // content while no authenticated, unlocked desktop bridge is active.
        // File/input/update adapters remain unadvertised.
        let config = RuntimeConfig {
            network: NetworkConfig::new()
                .with_mdns_enabled(true)
                .with_public_network_enabled(false),
            display_name: "uniDesk Windows".into(),
            capabilities: Capabilities::NETWORK
                | Capabilities::DEVICE_NETWORK
                | Capabilities::CLIPBOARD
                | Capabilities::CLIPBOARD_SYNC_V2,
            ..RuntimeConfig::default()
        };
        let mut nonce = [0_u8; 32];
        nonce[..16].copy_from_slice(RequestId::new().into_uuid().as_bytes());
        nonce[16..].copy_from_slice(RequestId::new().into_uuid().as_bytes());
        let hello = PeerHello::signed(
            &identity,
            config.display_name.clone(),
            config.capabilities,
            nonce,
        )
        .map_err(|_| stage(23))?;
        // Existing project transport certificate, not the machine Ed25519 key.
        // Remote identity still requires signed hello and persisted pairing trust.
        let transport = QuinnTransport::bind(
            hello,
            QuinnTransportConfig::new(
                SocketAddr::from(([0, 0, 0, 0], 42424)),
                SocketAddr::from(([127, 0, 0, 1], 0)),
                "unidesk.local",
                include_bytes!("../../../../resources/unidesk-transport-cert.der").to_vec(),
                include_bytes!("../../../../resources/unidesk-transport-key.der").to_vec(),
                include_bytes!("../../../../resources/unidesk-transport-cert.der").to_vec(),
            ),
        )
        .await
        .map_err(|_| stage(24))?;
        let control_port = transport.local_addr().map_err(|_| stage(24))?.port();
        let mut discovery = PlatformDiscovery::new(identity.device_id()).map_err(|_| stage(25))?;
        discovery
            .set_advertisement(
                ServiceRecord::v1(
                    identity.device_id(),
                    config.display_name.clone(),
                    config.capabilities,
                    control_port,
                )
                .map_err(|_| stage(25))?,
            )
            .map_err(|_| stage(25))?;
        let runtime = RuntimeBuilder::new()
            .config(config)
            .with_identity_provider(provider)
            .with_store(store.clone())
            .with_trust_store(store)
            .with_external_discovery(discovery)
            .with_control_transport(transport)
            .build()
            .map_err(|_| stage(26))?;
        Ok(Self {
            runtime,
            control_port,
            _directory: directory,
        })
    }
}

fn stage(code: i32) -> io::Error {
    io::Error::from_raw_os_error(code)
}
