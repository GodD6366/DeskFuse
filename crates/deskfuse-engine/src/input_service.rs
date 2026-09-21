//! The standalone input process. No application actor, SQLite, clipboard or file worker.
use crate::{
    input_coordinator::InputCoordinator,
    tokio_bridge::{UiEvent, production_transport::*},
};
use deskfuse_core::*;
use deskfuse_discovery::{DiscoveryEvent, DiscoveryProvider, PlatformDiscovery, ServiceRecord};
use deskfuse_runtime::{
    ControlTransportAdapter, SnapshotView,
    input_service::{Authority, ChildConnection},
};
use deskfuse_transport::{ControlTransport, PeerHello, QuinnTransport, QuinnTransportConfig};
use futures::StreamExt;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

pub fn main() -> std::result::Result<(), String> {
    if std::env::args().nth(1).as_deref() != Some("--input-service") {
        return Err("input service requires inherited parent channels".into());
    }
    let connection = ChildConnection::open()?;
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    executor.block_on(run(connection))
}
fn view(
    authority: &Authority,
    routes: &BTreeMap<DeviceId, Vec<RouteCandidate>>,
    sequence: u64,
) -> SnapshotView {
    let mut peers = authority.peers.clone();
    peers.retain(|p| {
        p.trust == TrustState::Trusted
            && authority.network.allows(authority.local, p.id)
            && !authority.network.blocked_peer_ids.contains(&p.id)
    });
    for peer in &mut peers {
        // Ordinary control-session state is deliberately ignored. The input actor
        // verifies its own signed physical connection before granting any lease.
        peer.connection = ConnectionState::Disconnected;
        peer.selected_route = None;
        peer.routes = routes.get(&peer.id).cloned().unwrap_or_default();
    }
    SnapshotView {
        device_network: authority.network.clone(),
        snapshot: NetworkSnapshot {
            sequence,
            local_device_id: authority.local,
            peers,
            public_network_enabled: false,
        },
        discovered_peers: vec![],
    }
}
async fn run(mut child: ChildConnection) -> std::result::Result<(), String> {
    let identity = Arc::new(
        deskfuse_identity::FileIdentityProvider::new(&child.bootstrap.identity_path)
            .load_existing()
            .map_err(|e| e.to_string())?,
    );
    if identity.device_id() != child.bootstrap.authority.local {
        return Err("input identity mismatch".into());
    }
    let caps = Capabilities::INPUT_SHARING
        | Capabilities::DEVICE_NETWORK
        | Capabilities::INPUT_ACTIVATION
        | Capabilities::INPUT_ENVIRONMENT
        | Capabilities::INPUT_MOTION_DATAGRAM_V1;
    let nonce = identity.device_id().into_bytes();
    let hello = PeerHello::signed(&identity, child.bootstrap.display_name.clone(), caps, nonce)
        .map_err(|e| e.to_string())?;
    let config = QuinnTransportConfig::new(
        ([0, 0, 0, 0], 42425).into(),
        placeholder_remote(),
        SERVER_NAME,
        CERTIFICATE_DER.to_vec(),
        PRIVATE_KEY_DER.to_vec(),
        CERTIFICATE_DER.to_vec(),
    )
    .for_input_service();
    let transport = QuinnTransport::bind(hello, config)
        .await
        .map_err(|e| e.to_string())?;
    transport
        .configure_identity(identity)
        .map_err(|e| e.to_string())?;
    let mut discovery = PlatformDiscovery::for_input_service(child.bootstrap.authority.local)
        .map_err(|e| e.to_string())?;
    discovery
        .set_advertisement(
            ServiceRecord::v2(
                child.bootstrap.authority.local,
                child.bootstrap.display_name.clone(),
                caps,
                42425,
            )
            .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    let discovery_events = discovery.start().map_err(|e| e.to_string())?;
    let mut authority = child.authority.borrow().clone();
    let mut routes = BTreeMap::new();
    merge_locator_hints(&authority, &mut routes);
    let mut authorized = BTreeSet::new();
    let mut sequence = 0;
    let initial = view(&authority, &routes, sequence);
    for p in &initial.snapshot.peers {
        transport
            .register_peer_identity(p.id, p.public_key)
            .await
            .map_err(|e| e.to_string())?;
        authorized.insert(p.id);
    }
    let runtime = child
        .start_runtime(
            Arc::new(ControlTransportAdapter::new(transport.clone())),
            initial,
        )
        .map_err(|e| e.to_string())?;
    let (ui, mut ui_events) = futures::channel::mpsc::unbounded();
    let native = InputCoordinator::start_local(
        runtime.handle.clone(),
        ui,
        child.recording.clone(),
        child.emergency.clone(),
        child.pause.clone(),
        child.stopping.clone(),
    );
    let diagnostics = Arc::new(std::sync::Mutex::new(String::new()));
    let diagnostic_output = diagnostics.clone();
    let stop = child.stopping.clone();
    std::thread::Builder::new().name("deskfuse-input-diagnostics".into()).spawn(move || {
        while !stop.load(Ordering::Acquire) {
            if deskfuse_core::debug_logging::enabled() {
                let mut lines = serde_json::json!({"subsystem":"input_service", "pid":std::process::id(), "build":deskfuse_runtime::input_service::BUILD}).to_string(); lines.push('\n');
                for record in deskfuse_platform::timing::snapshot().records.into_iter().rev().take(256) {
                    if let Ok(line) = serde_json::to_string(&record) { lines.push_str(&line); lines.push('\n'); }
                }
                let (motion,dropped) = deskfuse_transport::motion_diag::snapshot();
                for r in motion.into_iter().rev().take(256) {
                    let line = serde_json::json!({"subsystem":"input_motion_transport", "timestamp_ms":r.timestamp_ms, "stage":r.event, "peer":r.peer, "transport_session":r.transport_session, "local_addr":r.local_addr, "remote_addr":r.remote_addr, "interface_name":r.interface_name, "interface_kind":r.interface_kind, "ready":r.ready, "sent":r.sent, "received":r.received, "send_replaced":r.send_replaced, "receive_replaced":r.receive_replaced, "dropped":r.dropped, "pending_send":r.pending_send, "rtt_micros":r.rtt_micros, "sent_packets":r.sent_packets, "lost_packets":r.lost_packets, "congestion_events":r.congestion_events});
                    lines.push_str(&line.to_string()); lines.push('\n');
                }
                lines.push_str(&format!("{{\"subsystem\":\"input_service\",\"motion_dropped\":{dropped},\"export_limit\":256}}\n"));
                *diagnostic_output.lock().unwrap_or_else(|e|e.into_inner()) = lines;
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    }).map_err(|e|e.to_string())?;
    let wifi_runtime = runtime.handle.clone();
    let stop = child.stopping.clone();
    std::thread::Builder::new()
        .name("deskfuse-input-wifi".into())
        .spawn(move || {
            while !stop.load(Ordering::Acquire) {
                let ssid = crate::platform::current_ssid();
                let _ = wifi_runtime
                    .try_send(deskfuse_runtime::RuntimeCommand::ObserveWifiEnvironment { ssid });
                for _ in 0..50 {
                    if stop.load(Ordering::Acquire) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        })
        .map_err(|e| e.to_string())?;
    let mut remote_input = crate::remote_application_input::Owner::new(child.pause.clone());
    let mut runtime_events = runtime.handle.event_stream();
    let mut status = child.status.borrow().clone();
    let mut tick = tokio::time::interval(Duration::from_millis(50));
    while !child.stopping.load(Ordering::Acquire) {
        tokio::select! {
            _ = tick.tick() => {
                if matches!(child.pause.load(Ordering::Acquire), 6 | 7) && child.emergency.swap(false, Ordering::AcqRel) {
                    let _ = child.pause.compare_exchange(7, 0, Ordering::AcqRel, Ordering::Acquire);
                    let _ = child.pause.compare_exchange(6, 0, Ordering::AcqRel, Ordering::Acquire);
                }
                let remote_active = matches!(child.pause.load(Ordering::Acquire), 4 | 5);
                let emergency = remote_active && child.emergency.swap(false, Ordering::AcqRel);
                remote_input.tick(&authority, emergency);
                let mut changed = false;
                // Bound discovery processing so a discovery burst cannot starve lifetime checks.
                for _ in 0..32 {
                    match discovery_events.try_recv() {
                        Ok(Some(DiscoveryEvent::Found(service) | DiscoveryEvent::Updated(service))) => {
                            let id = service.device_id();
                            let previous = routes.entry(id).or_insert_with(Vec::new);
                            let next = service.endpoints.into_iter().filter(|e|e.port == 42425).map(|endpoint| {
                                previous.iter().find(|r|r.endpoint==endpoint).cloned().unwrap_or_else(||RouteCandidate::new(endpoint,RouteSource::Mdns,RouteHealth::Unknown,caps))
                            }).collect();
                            *previous = next; changed = true;
                        }
                        // Discovery disappearance is not revocation and must not tear down
                        // an otherwise healthy signed input connection.
                        Ok(Some(DiscoveryEvent::Removed(_))) => {}
                        Ok(Some(DiscoveryEvent::Error(_))) => {},
                        _ => break,
                    }
                }
                if child.authority.has_changed().unwrap_or(false) { authority = child.authority.borrow_and_update().clone(); merge_locator_hints(&authority, &mut routes); changed = true; }
                if changed {
                    sequence += 1;
                    let next = view(&authority,&routes,sequence);
                    let allowed: BTreeSet<_> = next.snapshot.peers.iter().map(|p|p.id).collect();
                    for id in authorized.difference(&allowed) { transport.revoke_peer(*id).await.map_err(|e|e.to_string())?; }
                    for p in &next.snapshot.peers {
                        if !authorized.contains(&p.id) { transport.restore_peer(p.id).await.map_err(|e|e.to_string())?; transport.register_peer_identity(p.id,p.public_key).await.map_err(|e|e.to_string())?; }
                    }
                    authorized = allowed; runtime.update_view(next);
                }
                status.native.remote_application_viewer_paused = child.pause.load(Ordering::Acquire) == 7;
                status.native.remote_application_lease = remote_input.lease();
                status.native.remote_application_request = remote_input.request.clone();
                status.native.remote_application_error = remote_input.error.clone();
                status.input = runtime.handle.input_snapshot();
                let (pending, error) = child.persistence_status();
                status.native.persistence_pending = pending;
                if let Some(error) = error { status.native.error = Some(format!("输入设置尚未保存：{error}")); }
                status.native.update_paused = child.pause.load(Ordering::Acquire) == 2;
                if let Ok(lines) = diagnostics.try_lock() { status.diagnostics = lines.clone(); }
                child.status.send_if_modified(|current| {
                    if current == &status {
                        false
                    } else {
                        *current = status.clone();
                        true
                    }
                });
            }
            Some(packet) = child.remote.recv() => { remote_input.receive(packet, &authority); },
            Some((id,command)) = child.commands.recv() => {
                if let Err(error) = runtime.handle.try_send(command) { status.native.error = Some(error.to_string()); }
                status.last_command = id;
            }
            Some(deskfuse_runtime::RuntimeEvent::Error(error)) = runtime_events.next() => { status.native.error = Some(error.to_string()); },
            Some(event) = ui_events.next() => match event {
                UiEvent::InputStatus {error} => status.native.error = error,
                UiEvent::InputLockStatus(value) => status.native.locked = value,
                UiEvent::InputRecordingReady(value) => status.native.recording_ready = value,
                UiEvent::PermissionSnapshot(value) => {
                    status.native.accessibility = Some(permission_code(value.accessibility));
                    status.native.input_monitoring = Some(permission_code(value.input_monitoring));
                }
                _ => {},
            },
        }
    }
    drop(remote_input);
    drop(native);
    runtime.handle.shutdown().await.map_err(|e| e.to_string())?;
    Ok(())
}

fn permission_code(value: deskfuse_platform::PermissionStatus) -> u8 {
    use deskfuse_platform::PermissionStatus::*;
    match value {
        Granted => 1,
        Denied => 2,
        Unknown => 0,
        NeedsUserAction => 3,
    }
}

fn merge_locator_hints(
    authority: &Authority,
    routes: &mut BTreeMap<DeviceId, Vec<RouteCandidate>>,
) {
    for peer in &authority.peers {
        let known = routes.entry(peer.id).or_default();
        for hint in &peer.routes {
            if hint.source.is_public() || hint.endpoint.host.parse::<std::net::IpAddr>().is_err() {
                continue;
            }
            let mut endpoint = hint.endpoint.clone();
            endpoint.port = 42425;
            if !known.iter().any(|r| r.endpoint == endpoint) && known.len() < 32 {
                known.push(RouteCandidate::new(
                    endpoint,
                    RouteSource::CachedDirect,
                    RouteHealth::Unknown,
                    Capabilities::INPUT_SHARING,
                ));
            }
        }
    }
    routes.retain(|peer, _| {
        authority
            .peers
            .iter()
            .any(|p| p.id == *peer && p.trust == TrustState::Trusted)
    });
}
