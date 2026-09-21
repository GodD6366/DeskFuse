//! Headless two-Mac verification probe for DeskFuse M1 discovery and pairing.
//!
//! This binary runs the real production runtime wiring (Bonjour/mDNS discovery
//! plus an authenticated QUIC control listener) without the GPUI shell. It is
//! intentionally small and deterministic enough to drive two real Macs through
//! the first-time mDNS-to-QUIC bootstrap and pairing approval flow.

use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use deskfuse_core::{
    Capabilities, ClipboardPayload, ConnectionState, DeviceId, DisplayInfo, Endpoint,
    InputEventBatch, NetworkConfig, RemoteInputEvent, RouteCandidate, RouteHealth, RouteSource,
    TrustState,
};
use deskfuse_discovery::{PlatformDiscovery, ServiceRecord};
use deskfuse_identity::{MemoryIdentityProvider, MemoryTrustStore};
use deskfuse_runtime::{
    DiscoveryEvent, DiscoveryProvider, DiscoveryStream, ExternalDiscoveryAdapter, MemoryDiscovery,
    MemoryRuntimeStore, ProviderFuture, RuntimeBuilder, RuntimeCommand, RuntimeConfig,
    RuntimeEvent, RuntimeHandle,
};
use deskfuse_transport::{PeerHello, QuinnTransport, QuinnTransportConfig};
use futures::StreamExt;
use prost::Message as _;

const CONTROL_PORT: u16 = 42424;
const SERVER_NAME: &str = "deskfuse.local";
const CERTIFICATE_DER: &[u8] = include_bytes!("../../../resources/deskfuse-transport-cert.der");
const PRIVATE_KEY_DER: &[u8] = include_bytes!("../../../resources/deskfuse-transport-key.der");

#[derive(Debug)]
struct Args {
    name: String,
    port: u16,
    duration_secs: u64,
    connect_on_discovery: bool,
    auto_approve: bool,
    secret: Option<[u8; 32]>,
    manual_peer: Option<(deskfuse_core::DeviceId, deskfuse_core::Endpoint)>,
    push_clipboard: Option<String>,
    push_input: bool,
    show_display: bool,
    peer_hello: Option<Vec<u8>>,
    print_hello: bool,
    no_dial: bool,
    approve_delay: Duration,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            name: hostname_label(),
            port: CONTROL_PORT,
            duration_secs: 90,
            connect_on_discovery: false,
            auto_approve: false,
            secret: None,
            manual_peer: None,
            push_clipboard: None,
            push_input: false,
            show_display: false,
            peer_hello: None,
            print_hello: false,
            no_dial: false,
            approve_delay: Duration::ZERO,
        }
    }
}

/// Combines the real mDNS provider with a one-shot manual peer hint so the
/// probe can still exercise pairing when multicast discovery is blocked by the
/// network segment between the two Macs.
struct HybridDiscovery {
    mdns: ExternalDiscoveryAdapter<PlatformDiscovery>,
    manual: MemoryDiscovery,
}

impl DiscoveryProvider for HybridDiscovery {
    fn refresh(&self) -> ProviderFuture<Vec<DiscoveryEvent>> {
        self.manual.refresh()
    }

    fn events(&self) -> DiscoveryStream {
        Box::pin(futures::stream::select(
            self.mdns.events(),
            self.manual.events(),
        ))
    }

    fn shutdown(&self) -> ProviderFuture<()> {
        let mdns_shutdown = self.mdns.shutdown();
        let manual_shutdown = self.manual.shutdown();
        Box::pin(async move {
            mdns_shutdown.await?;
            manual_shutdown.await
        })
    }
}

fn hostname_label() -> String {
    std::process::Command::new("/bin/hostname")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|name| {
            let trimmed = name.trim();
            trimmed.strip_suffix(".local").unwrap_or(trimmed).to_owned()
        })
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "deskfuse-probe".to_owned())
}

impl Args {
    fn parse(raw: Vec<String>) -> Result<Self, String> {
        let mut args = Self::default();
        let mut index = 0;
        while index < raw.len() {
            match raw[index].as_str() {
                "--name" => {
                    index += 1;
                    args.name = raw
                        .get(index)
                        .ok_or_else(|| "--name needs a value".to_owned())?
                        .clone();
                }
                "--port" => {
                    index += 1;
                    let value = raw
                        .get(index)
                        .ok_or_else(|| "--port needs a value".to_owned())?;
                    args.port = value.parse::<u16>().map_err(|error| error.to_string())?;
                    if args.port == 0 {
                        return Err("port must be non-zero".to_owned());
                    }
                }
                "--duration" => {
                    index += 1;
                    let value = raw
                        .get(index)
                        .ok_or_else(|| "--duration needs a value".to_owned())?;
                    args.duration_secs = value.parse::<u64>().map_err(|error| error.to_string())?;
                }
                "--connect-on-discovery" => args.connect_on_discovery = true,
                "--auto-approve" => args.auto_approve = true,
                "--secret" => {
                    index += 1;
                    let value = raw
                        .get(index)
                        .ok_or_else(|| "--secret needs a value".to_owned())?;
                    args.secret = Some(parse_secret(value)?);
                }
                "--manual-peer" => {
                    index += 1;
                    let value = raw
                        .get(index)
                        .ok_or_else(|| "--manual-peer needs a value".to_owned())?;
                    args.manual_peer = Some(parse_manual_peer(value)?);
                }
                "--push-clipboard" => {
                    index += 1;
                    args.push_clipboard = Some(
                        raw.get(index)
                            .ok_or_else(|| "--push-clipboard needs a value".to_owned())?
                            .clone(),
                    );
                }
                "--push-input" => args.push_input = true,
                "--show-display" => args.show_display = true,
                "--peer-hello" => {
                    index += 1;
                    let value = raw
                        .get(index)
                        .ok_or_else(|| "--peer-hello needs a value".to_owned())?;
                    args.peer_hello = Some(
                        hex_decode(value).ok_or_else(|| "--peer-hello must be hex".to_owned())?,
                    );
                }
                "--print-hello" => args.print_hello = true,
                "--no-dial" => args.no_dial = true,
                "--approve-delay" => {
                    index += 1;
                    let value = raw
                        .get(index)
                        .ok_or_else(|| "--approve-delay needs seconds".to_owned())?;
                    args.approve_delay = Duration::from_secs(
                        value.parse::<u64>().map_err(|error| error.to_string())?,
                    );
                }
                "--help" => {
                    return Err(
                        "usage: deskfuse-probe [--name NAME] [--port PORT] [--duration SECONDS] \
                         [--connect-on-discovery] [--auto-approve] [--secret HEX32] \
                         [--manual-peer DEVICE_ID@HOST:PORT] [--push-clipboard TEXT] \
                         [--push-input] [--show-display] [--peer-hello HEX] [--print-hello] \
                         [--no-dial] [--approve-delay SECONDS]"
                            .to_owned(),
                    );
                }
                other => return Err(format!("unknown argument: {other}")),
            }
            index += 1;
        }
        Ok(args)
    }
}

/// Parse a 32-byte fixed identity secret from 64 hex characters.
fn parse_secret(value: &str) -> Result<[u8; 32], String> {
    let bytes = hex_bytes(value).ok_or_else(|| "secret must be 64 hex characters".to_owned())?;
    let mut secret = [0_u8; 32];
    secret.copy_from_slice(&bytes);
    Ok(secret)
}

/// Parse `DEVICE_ID@HOST:PORT` into a device id and dialable endpoint.
fn parse_manual_peer(value: &str) -> Result<(DeviceId, Endpoint), String> {
    let (id, endpoint) = value
        .split_once('@')
        .ok_or_else(|| "manual peer must be DEVICE_ID@HOST:PORT".to_owned())?;
    let device_id = DeviceId::from_hex(id).map_err(|error| error.to_string())?;
    let endpoint = endpoint
        .parse::<Endpoint>()
        .map_err(|error| error.to_string())?;
    Ok((device_id, endpoint))
}

fn hex_decode(value: &str) -> Option<Vec<u8>> {
    if value.is_empty() || !value.len().is_multiple_of(2) {
        return None;
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    let mut chars = value.chars();
    while let (Some(high), Some(low)) = (chars.next(), chars.next()) {
        bytes.push(u8::from_str_radix(&format!("{high}{low}"), 16).ok()?);
    }
    Some(bytes)
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn hex_bytes(value: &str) -> Option<Vec<u8>> {
    if value.len() != 64 {
        return None;
    }
    let mut bytes = Vec::with_capacity(32);
    let mut chars = value.chars();
    while let (Some(high), Some(low)) = (chars.next(), chars.next()) {
        bytes.push(u8::from_str_radix(&format!("{high}{low}"), 16).ok()?);
    }
    Some(bytes)
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), String> {
    let args = Args::parse(std::env::args().skip(1).collect())?;
    println!(
        "[probe] name={} port={} duration={}s connect={} approve={}",
        args.name, args.port, args.duration_secs, args.connect_on_discovery, args.auto_approve
    );

    let identity_provider = match args.secret {
        Some(secret) => {
            MemoryIdentityProvider::from_secret_bytes(&secret).map_err(|error| error.to_string())?
        }
        None => MemoryIdentityProvider::new(),
    };
    let identity = identity_provider
        .load_or_create()
        .map_err(|error| error.to_string())?;
    let local_id = identity.device_id();
    println!("[probe] local_device_id={}", local_id.to_hex());

    let config = RuntimeConfig {
        network: NetworkConfig::new()
            .with_mdns_enabled(true)
            .with_public_network_enabled(false),
        display_name: args.name.clone(),
        capabilities: Capabilities::NETWORK,
        ..RuntimeConfig::default()
    };

    let request_id = deskfuse_core::RequestId::new().into_uuid();
    let device_bytes = identity.device_id().into_bytes();
    let mut nonce = [0_u8; 32];
    for (index, byte) in nonce.iter_mut().enumerate() {
        *byte = request_id.as_bytes()[index % 16] ^ device_bytes[index];
    }
    let local_hello = PeerHello::signed(&identity, args.name.clone(), Capabilities::NETWORK, nonce)
        .map_err(|error| error.to_string())?;
    if args.print_hello {
        let bytes = local_hello.to_wire().encode_to_vec();
        println!("LOCAL_HELLO_HEX={}", hex_encode(&bytes));
        return Ok(());
    }

    let quic_config = QuinnTransportConfig::new(
        SocketAddr::from(([0, 0, 0, 0], args.port)),
        SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        SERVER_NAME,
        CERTIFICATE_DER.to_vec(),
        PRIVATE_KEY_DER.to_vec(),
        CERTIFICATE_DER.to_vec(),
    );
    let transport = QuinnTransport::bind(local_hello, quic_config)
        .await
        .map_err(|error| error.to_string())?;
    let control_port = transport
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    println!("[probe] quic_listener=udp/{control_port}");

    let mut discovery = PlatformDiscovery::new(local_id).map_err(|error| error.to_string())?;
    let record = ServiceRecord::v2(
        local_id,
        args.name.clone(),
        Capabilities::NETWORK,
        control_port,
    )
    .map_err(|error| error.to_string())?;
    discovery
        .set_advertisement(record)
        .map_err(|error| error.to_string())?;

    let manual = MemoryDiscovery::new();
    if let Some(hello_bytes) = &args.peer_hello {
        let wire = deskfuse_protocol::Hello::decode(hello_bytes.as_slice())
            .map_err(|error| format!("peer hello decode failed: {error}"))?;
        let hello = PeerHello::from_wire(wire).map_err(|error| error.to_string())?;
        let Some((peer_id, endpoint)) = args.manual_peer.as_ref() else {
            return Err("--peer-hello requires --manual-peer DEVICE_ID@HOST:PORT".to_owned());
        };
        if hello.device_id() != *peer_id {
            return Err("peer hello device id does not match --manual-peer id".to_owned());
        }
        let route = RouteCandidate::new(
            endpoint.clone(),
            RouteSource::Manual,
            RouteHealth::Unknown,
            Capabilities::NETWORK,
        );
        manual.push(DiscoveryEvent::PeerDiscovered {
            hello: hello.to_wire(),
            routes: vec![route],
        });
        println!("[peer-hello] {}@{}", peer_id.to_hex(), endpoint);
    } else if let Some((peer_id, endpoint)) = &args.manual_peer {
        manual.push(DiscoveryEvent::PeerHint {
            peer_id: *peer_id,
            display_name: "manual-peer".to_owned(),
            capabilities: Capabilities::NETWORK,
            routes: vec![RouteCandidate::new(
                endpoint.clone(),
                RouteSource::Manual,
                RouteHealth::Unknown,
                Capabilities::NETWORK,
            )],
        });
        println!("[manual-peer] {}@{}", peer_id.to_hex(), endpoint);
    }
    let discovery = HybridDiscovery {
        mdns: ExternalDiscoveryAdapter::new(discovery),
        manual,
    };

    let runtime = RuntimeBuilder::new()
        .config(config)
        .with_identity_provider(identity_provider)
        .with_trust_store(MemoryTrustStore::new())
        .with_store(MemoryRuntimeStore::new())
        .with_discovery(discovery)
        .with_control_transport(transport)
        .build()
        .map_err(|error| error.to_string())?;

    run(args, runtime).await
}

async fn run(args: Args, runtime: RuntimeHandle) -> Result<(), String> {
    let mut events = runtime.event_stream();
    let mut application_messages_sent = false;

    if let Some((peer_id, _)) = args.manual_peer {
        if args.no_dial {
            println!("[action] skip dial (no-dial) {}", short_id(peer_id));
        }
        for _ in 0..100 {
            if runtime
                .snapshot_view()
                .discovered_peers
                .iter()
                .any(|hint| hint.peer_id == peer_id)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        if !args.no_dial {
            let _ = runtime.send(RuntimeCommand::connect(peer_id)).await;
            println!("[action] connect manual-peer {}", short_id(peer_id));
        }
    }

    let deadline = tokio::time::Instant::now() + Duration::from_secs(args.duration_secs);
    loop {
        tokio::select! {
            () = tokio::time::sleep_until(deadline) => break,
            event = events.next() => {
                let Some(event) = event else { break; };
                match event {
                    RuntimeEvent::DiscoveredPeerHint(hint) => {
                        println!(
                            "[hint] peer={} name={} routes={:?}",
                            short_id(hint.peer_id),
                            hint.display_name,
                            hint.routes
                        );
                        if args.connect_on_discovery {
                            let _ = runtime.send(RuntimeCommand::connect(hint.peer_id)).await;
                            println!("[action] connect {}", short_id(hint.peer_id));
                        }
                    }
                    RuntimeEvent::PairingRequest(request) => {
                        println!(
                            "[pairing-request] peer={} name={} request={:?}",
                            short_id(request.device_id()),
                            request.display_name(),
                            request.request_id()
                        );
                        if args.auto_approve {
                            if args.approve_delay.is_zero() {
                                let _ = runtime
                                    .send(RuntimeCommand::approve_pairing(request.request_id()))
                                    .await;
                                println!("[action] approve {:?}", request.request_id());
                            } else {
                                let runtime = runtime.clone();
                                let request_id = request.request_id();
                                let delay = args.approve_delay;
                                println!(
                                    "[action] approve {:?} in {:?}",
                                    request.request_id(),
                                    delay
                                );
                                tokio::spawn(async move {
                                    tokio::time::sleep(delay).await;
                                    let _ = runtime
                                        .send(RuntimeCommand::approve_pairing(request_id))
                                        .await;
                                    println!("[action] approve {:?}", request_id);
                                });
                            }
                        }
                    }
                    RuntimeEvent::Snapshot(snapshot) => {
                        for peer in &snapshot.peers {
                            println!(
                                "[peer] name={} id={} trust={:?} connection={:?} liveness={:?} routes={:?}",
                                peer.display_name,
                                short_id(peer.id),
                                peer.trust,
                                peer.connection,
                                peer.liveness,
                                peer.routes
                            );
                        }
                        if !application_messages_sent {
                            let target = snapshot.peers.iter().find(|peer| {
                                peer.trust == TrustState::Trusted
                                    && peer.connection == ConnectionState::Connected
                            });
                            if let Some(target) = target {
                                application_messages_sent = true;
                                push_application_messages(&args, &runtime, target.id).await?;
                            }
                        }
                    }
                    RuntimeEvent::ClipboardReceived { peer_id, payload } => {
                        println!(
                            "[clipboard-recv] peer={} mime={} len={} seq={}",
                            short_id(peer_id),
                            payload.mime,
                            payload.data.len(),
                            payload.sequence
                        );
                    }
                    RuntimeEvent::InputReceived { peer_id, batch } => {
                        println!(
                            "[input-recv] peer={} events={} seq={}",
                            short_id(peer_id),
                            batch.events.len(),
                            batch.sequence
                        );
                    }
                    RuntimeEvent::DisplayInfoReceived { peer_id, display } => {
                        println!(
                            "[display-recv] peer={} {}x{}",
                            short_id(peer_id), display.width, display.height
                        );
                    }
                    RuntimeEvent::Error(error) => println!("[error] {error}"),
                    _ => {}
                }
            }
        }
    }

    let view = runtime.snapshot_view();
    println!("--- final snapshot ---");
    for peer in &view.snapshot.peers {
        println!(
            "FINAL peer={} name={} trust={:?} connection={:?} routes={:?}",
            short_id(peer.id),
            peer.display_name,
            peer.trust,
            peer.connection,
            peer.routes
        );
    }
    runtime
        .shutdown_and_wait()
        .await
        .map_err(|error| error.to_string())
}

async fn push_application_messages(
    args: &Args,
    runtime: &RuntimeHandle,
    peer_id: DeviceId,
) -> Result<(), String> {
    let mut sequence: u64 = 0;
    if let Some(text) = &args.push_clipboard {
        sequence = sequence.wrapping_add(1);
        let payload = ClipboardPayload::new("text/plain", text.as_bytes().to_vec(), sequence)
            .map_err(|error| error.to_string())?;
        runtime
            .send(RuntimeCommand::SendClipboard { peer_id, payload })
            .await
            .map_err(|error| error.to_string())?;
        println!(
            "[action] push-clipboard peer={} len={} seq={sequence}",
            short_id(peer_id),
            text.len()
        );
    }
    if args.show_display {
        let display = DisplayInfo::new(1920, 1080).map_err(|error| error.to_string())?;
        runtime
            .send(RuntimeCommand::SendDisplayInfo { peer_id, display })
            .await
            .map_err(|error| error.to_string())?;
        println!("[action] show-display peer={} 1920x1080", short_id(peer_id));
    }
    if args.push_input {
        sequence = sequence.wrapping_add(1);
        let batch = InputEventBatch::new(
            sequence,
            vec![
                RemoteInputEvent::MouseMove { x: 0.5, y: 0.5 },
                RemoteInputEvent::MouseButton {
                    button: 0,
                    pressed: true,
                },
                RemoteInputEvent::MouseButton {
                    button: 0,
                    pressed: false,
                },
                RemoteInputEvent::Key {
                    code: 0,
                    pressed: true,
                },
                RemoteInputEvent::Key {
                    code: 0,
                    pressed: false,
                },
                RemoteInputEvent::Scroll {
                    delta_x: 0.0,
                    delta_y: 1.0,
                },
            ],
        )
        .map_err(|error| error.to_string())?;
        runtime
            .send(RuntimeCommand::SendInputBatch { peer_id, batch })
            .await
            .map_err(|error| error.to_string())?;
        println!(
            "[action] push-input peer={} 6-events seq={sequence}",
            short_id(peer_id)
        );
    }
    Ok(())
}

fn short_id(id: deskfuse_core::DeviceId) -> String {
    let hex = id.to_hex();
    format!("{}…", &hex[..12])
}
