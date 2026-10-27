//! The standalone input process. No application actor, SQLite, clipboard or file worker.
use crate::{
    input_coordinator::InputCoordinator,
    tokio_bridge::{UiEvent, production_transport::*},
};
use deskfuse_core::*;
use deskfuse_discovery::{DiscoveryEvent, DiscoveryProvider, PlatformDiscovery, ServiceRecord};
use deskfuse_platform::ports::EnginePlatformPort;
use deskfuse_runtime::{
    ControlTransportAdapter, SnapshotView,
    input_service::{Authority, ChildConnection},
};
use deskfuse_transport::{ControlTransport, PeerHello, QuinnTransport, QuinnTransportConfig};
use futures::StreamExt;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};

pub fn main_with_platform(
    platform: Arc<dyn EnginePlatformPort>,
) -> std::result::Result<(), String> {
    if platform.runtime().arguments().get(1).map(String::as_str) != Some("--input-service") {
        return Err("input service requires inherited parent channels".into());
    }
    let connection = ChildConnection::open()?;
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    executor.block_on(run(connection, platform))
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
        // The input transport is physically separate, but it is not an
        // independent device session. Keep the host control state so a failed
        // device connection cannot leave mouse sharing active.
        peer.selected_route = None;
        peer.routes = routes.get(&peer.id).cloned().unwrap_or_default();
    }
    SnapshotView {
        device_network: authority.network.clone(),
        snapshot: NetworkSnapshot {
            sequence,
            local_device_id: authority.local,
            peers,
        },
        discovered_peers: vec![],
    }
}
fn quality_lease(input: &InputSharingSnapshot) -> (String, Option<String>) {
    let link = input
        .links
        .iter()
        .find(|link| link.control_session.is_some());
    (
        link.and_then(|link| link.motion_mode.clone())
            .unwrap_or_default(),
        link.and_then(|link| link.motion_fallback_reason.clone()),
    )
}

async fn run(
    mut child: ChildConnection,
    platform: Arc<dyn EnginePlatformPort>,
) -> std::result::Result<(), String> {
    let identity = Arc::new(
        deskfuse_identity::FileIdentityProvider::new(&child.bootstrap.identity_path)
            .load_existing()
            .map_err(|e| e.to_string())?,
    );
    if identity.device_id() != child.bootstrap.authority.local {
        return Err("input identity mismatch".into());
    }
    let caps = Capabilities::INPUT_TRANSPORT;
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
    .for_input_service()
    .with_lan_port(platform.lan_web());
    let transport = QuinnTransport::bind(hello, config)
        .await
        .map_err(|e| e.to_string())?;
    transport
        .configure_identity(identity)
        .map_err(|e| e.to_string())?;
    // 发现服务 shutdown 后不可再 start：每次重新打开键鼠共享都要新建实例。
    let local = child.bootstrap.authority.local;
    let display_name = child.bootstrap.display_name.clone();
    let new_discovery = move || -> std::result::Result<PlatformDiscovery, String> {
        let mut discovery =
            PlatformDiscovery::for_input_service(local).map_err(|e| e.to_string())?;
        discovery
            .set_advertisement(
                ServiceRecord::v2(local, display_name.clone(), caps, 42425)
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        Ok(discovery)
    };
    let mut discovery = new_discovery()?;
    let mut sharing_enabled = child.bootstrap.stored.settings.feature_enabled;
    let mut discovery_events = if sharing_enabled {
        Some(discovery.start().map_err(|e| e.to_string())?)
    } else {
        None
    };
    let mut authority = child.authority.borrow().clone();
    // Bonjour 发现结果单独保存，路由表每次由两路来源合并重建。
    let mut mdns_routes = BTreeMap::new();
    let mut routes = BTreeMap::new();
    rebuild_routes(&authority, &mdns_routes, &mut routes);
    let mut authorized = BTreeSet::new();
    let mut last_participants = child
        .bootstrap
        .stored
        .settings
        .participants
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let mut sequence = 0;
    let initial = view(&authority, &routes, sequence);
    for p in initial.snapshot.peers.iter().filter(|_| sharing_enabled) {
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
        platform.clone(),
        ui,
        child.recording.clone(),
        child.emergency.clone(),
        child.pause.clone(),
        child.stopping.clone(),
    );
    let diagnostics = Arc::new(std::sync::Mutex::new(String::new()));
    let input_records = crate::diagnostics::Diagnostics::new(512);
    let exported_records = input_records.clone();
    let diagnostic_handle = runtime.handle.clone();
    let diagnostic_output = diagnostics.clone();
    let runtime_port = platform.runtime();
    let stop = child.stopping.clone();
    std::thread::Builder::new().name("deskfuse-input-diagnostics".into()).spawn(move || {
        while !stop.load(Ordering::Acquire) {
            if deskfuse_core::debug_logging::enabled() {
                let mut lines = serde_json::json!({"subsystem":"input_service", "pid":runtime_port.process_id(), "build":deskfuse_runtime::input_service::build_identity()}).to_string(); lines.push('\n');
                let input = diagnostic_handle.input_snapshot();
                let activation_snapshot = serde_json::json!({
                    "subsystem":"input_activation_snapshot", "pid":runtime_port.process_id(),
                    "timestamp_ms":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis(),
                    "enabled":input.settings.enabled, "role":input.settings.role,
                    "local_suspended":input.environment.local_suspended,
                    "environment":input.environment,
                    "placements":input.settings.placements,
                    "participants":input.settings.participants,
                    "local_topology":input.local_topology,
                    "remote_topologies":input.remote_topologies,
                    "scenes":input.scenes.iter().map(|scene|serde_json::json!({
                        "id":scene.id, "ssid":scene.ssid,
                        "placements":scene.settings.placements,
                        "layout_profile_count":scene.settings.layout_profiles.len(),
                    })).collect::<Vec<_>>(),
                    "activations":input.activations,
                    "managed_control":input.managed_control,
                    "links":input.links.iter().map(|link|serde_json::json!({
                        "peer":link.peer_id, "ready":link.ready, "reason":link.reason,
                        "context":link.context, "has_lease":link.control_session.is_some(),
                    })).collect::<Vec<_>>()
                });
                lines.push_str(&activation_snapshot.to_string()); lines.push('\n');
                lines.push_str(&exported_records.to_json_lines()); lines.push('\n');
                for record in deskfuse_platform::timing::snapshot().records.into_iter().rev().take(256) {
                    if let Ok(line) = serde_json::to_string(&record) { lines.push_str(&line); lines.push('\n'); }
                }
                let (motion,dropped) = deskfuse_transport::motion_diag::snapshot();
                for r in motion.into_iter().rev().take(256) {
                    let line = serde_json::json!({"subsystem":"input_motion_transport", "timestamp_ms":r.timestamp_ms, "stage":r.event, "peer":r.peer, "transport_session":r.transport_session, "local_addr":r.local_addr, "remote_addr":r.remote_addr, "interface_name":r.interface_name, "interface_kind":r.interface_kind, "ready":r.ready, "sent":r.sent, "received":r.received, "send_replaced":r.send_replaced, "receive_replaced":r.receive_replaced, "dropped":r.dropped, "pending_send":r.pending_send, "rtt_micros":r.rtt_micros, "sent_packets":r.sent_packets, "lost_packets":r.lost_packets, "congestion_events":r.congestion_events});
                    lines.push_str(&line.to_string()); lines.push('\n');
                }
                lines.push_str(&format!("{{\"subsystem\":\"input_service\",\"motion_dropped\":{dropped},\"export_limit\":256}}\n"));
                let (sockets, sockets_dropped) = deskfuse_transport::socket_diag::snapshot();
                for r in sockets {
                    let line = serde_json::json!({"subsystem":"input_socket", "timestamp_ms":r.timestamp_ms, "pid":runtime_port.process_id(), "stage":r.event, "local_addr":r.local_addr, "detail":r.detail});
                    lines.push_str(&line.to_string()); lines.push('\n');
                }
                lines.push_str(&format!("{{\"subsystem\":\"input_socket\",\"dropped\":{sockets_dropped}}}\n"));
                *diagnostic_output.lock().unwrap_or_else(|e|e.into_inner()) = lines;
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    }).map_err(|e|e.to_string())?;
    // Wi-Fi observations arrive from the authorized UI process over input IPC.
    let mut remote_input =
        crate::remote_application_input::Owner::new(child.pause.clone(), platform.clone());
    let mut runtime_events = runtime.handle.event_stream();
    let mut status = child.status.borrow().clone();
    let mut tick = tokio::time::interval(Duration::from_millis(50));
    let mut quality_line = String::new();
    let mut quality_at: Option<Instant> = None;
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
                let input_snapshot = runtime.handle.input_snapshot();
                let next_enabled = input_snapshot.settings.feature_enabled;
                let participants = input_snapshot
                    .settings
                    .participants
                    .iter()
                    .copied()
                    .collect::<BTreeSet<_>>();
                let mut changed = next_enabled != sharing_enabled;
                if next_enabled != sharing_enabled {
                    sharing_enabled = next_enabled;
                    if sharing_enabled {
                        discovery = new_discovery()?;
                        discovery_events = Some(discovery.start().map_err(|e| e.to_string())?);
                    } else {
                        discovery.shutdown().map_err(|e| e.to_string())?;
                        discovery_events = None;
                    }
                }
                if sharing_enabled && !participants.is_subset(&last_participants) {
                    // 新加入的屏幕可能就在刚切换到的网络上；主控发现结果不能
                    // 代替独立输入服务的 42425 Bonjour 路由，立即重扫一次。
                    crate::diagnostics::record!(
                        input_records,
                        crate::diagnostics::DiagnosticLevel::Info,
                        "input_discovery",
                        "refresh_after_participant_added",
                        Some(format!(
                            "participants={} previous={}",
                            participants.len(),
                            last_participants.len()
                        )),
                        None,
                    );
                    let _ = discovery.refresh();
                }
                last_participants = participants;
                // Bound discovery processing so a discovery burst cannot starve lifetime checks.
                for _ in 0..32 {
                    let Some(discovery_events) = discovery_events.as_ref() else { break; };
                    match discovery_events.try_recv() {
                        Ok(Some(DiscoveryEvent::Found(service) | DiscoveryEvent::Updated(service))) => {
                            let id = service.device_id();
                            let previous = routes.get(&id);
                            let next = service.endpoints.into_iter().filter(|e|e.port == 42425).map(|endpoint| {
                                previous.and_then(|p| p.iter().find(|r|r.endpoint==endpoint)).cloned().unwrap_or_else(||RouteCandidate::new(endpoint,RouteSource::Mdns,RouteHealth::Unknown,caps))
                            }).collect();
                            mdns_routes.insert(id, next);
                            rebuild_routes(&authority, &mdns_routes, &mut routes);
                            changed = true;
                        }
                        // Discovery disappearance is not revocation and must not tear down
                        // an otherwise healthy signed input connection.
                        Ok(Some(DiscoveryEvent::Removed(_))) => {}
                        Ok(Some(DiscoveryEvent::Error(_))) => {},
                        _ => break,
                    }
                }
                if child.authority.has_changed().unwrap_or(false) {
                    authority = child.authority.borrow_and_update().clone();
                    let before = input_route_endpoints(&routes);
                    rebuild_routes(&authority, &mdns_routes, &mut routes);
                    let after = input_route_endpoints(&routes);
                    if sharing_enabled && after != before {
                        // 主连接在新网络发现了对端地址。独立输入服务使用
                        // _deskfuse-input 命名空间，必须重新浏览，否则仍会
                        // 拿旧网络地址去连 42425。
                        crate::diagnostics::record!(
                            input_records,
                            crate::diagnostics::DiagnosticLevel::Info,
                            "input_discovery",
                            "refresh_after_authority_routes_changed",
                            Some(format!(
                                "routes_before={} routes_after={}",
                                before.len(),
                                after.len()
                            )),
                            None,
                        );
                        let _ = discovery.refresh();
                    }
                    changed = true;
                }
                if changed {
                    sequence += 1;
                    let next = view(&authority,&routes,sequence);
                    let allowed: BTreeSet<_> = next.snapshot.peers.iter().filter(|_| sharing_enabled).map(|p|p.id).collect();
                    for id in authorized.difference(&allowed) { transport.revoke_peer(*id).await.map_err(|e|e.to_string())?; }
                    for p in next.snapshot.peers.iter().filter(|_| sharing_enabled) {
                        if !authorized.contains(&p.id) { transport.restore_peer(p.id).await.map_err(|e|e.to_string())?; transport.register_peer_identity(p.id,p.public_key).await.map_err(|e|e.to_string())?; }
                    }
                    authorized = allowed; runtime.update_view(next);
                }
                status.native.remote_application_viewer_paused = child.pause.load(Ordering::Acquire) == 7;
                status.native.remote_application_lease = remote_input.lease();
                status.native.remote_application_quiesced = remote_input.quiesced.clone();
                status.native.remote_application_request = remote_input.request.clone();
                status.native.remote_application_error = remote_input.error.clone();
                status.input = runtime.handle.input_snapshot();
                let (pending, error) = child.persistence_status();
                status.native.persistence_pending = pending;
                if let Some(error) = error { status.native.error = Some(format!("输入设置尚未保存：{error}")); }
                status.native.update_paused = child.pause.load(Ordering::Acquire) == 2;
                // Always send chart metadata, even when debug logging is disabled
                // and no pointer events or input-state changes are occurring.
                if quality_at.is_none_or(|at| at.elapsed() >= Duration::from_secs(1)) {
                    deskfuse_core::fold_pending_stages();
                    let (mode, fallback) = quality_lease(&status.input);
                    status.quality.records = deskfuse_transport::motion_diag::quality_snapshot();
                    status.quality.report = deskfuse_input::quality::build_report(
                        &deskfuse_core::quality_ticks(), &mode, fallback.as_deref(),
                    );
                    quality_line = deskfuse_input::quality::diagnostics_record(
                        "input_child", &mode, fallback.as_deref(),
                    );
                    quality_at = Some(Instant::now());
                }
                if let Ok(lines) = diagnostics.try_lock() {
                    status.diagnostics = lines.clone();
                    if !quality_line.is_empty() {
                        if !status.diagnostics.ends_with('\n') {
                            status.diagnostics.push('\n');
                        }
                        status.diagnostics.push_str(&quality_line);
                        status.diagnostics.push('\n');
                    }
                }
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
            Some(event) = runtime_events.next() => match event {
                deskfuse_runtime::RuntimeEvent::InputDiagnostic {stage, detail} => {
                    crate::diagnostics::record!(input_records,
                        crate::diagnostics::DiagnosticLevel::Info, "input_activation", stage,
                        Some(format!("pid={} {detail}", platform.runtime().process_id())), None,
                    );
                }
                deskfuse_runtime::RuntimeEvent::Error(error) => {
                    crate::diagnostics::record!(input_records,
                        crate::diagnostics::DiagnosticLevel::Error, "input_service", "runtime_error",
                        Some(format!("pid={} error={error}", platform.runtime().process_id())), None,
                    );
                    status.native.error = Some(error.to_string());
                }
                _ => {},
            },
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

/// 独立输入服务的候选路由 = 自身 Bonjour 发现的 42425 地址 ∪ 主连接当前的局域网地址提示。
/// 每次整体重建：Bonjour 更新不能抹掉主连接提示，主连接撤回的旧地址也不会残留；
/// 已有候选沿用原对象，保留其健康状态。
fn rebuild_routes(
    authority: &Authority,
    mdns: &BTreeMap<DeviceId, Vec<RouteCandidate>>,
    routes: &mut BTreeMap<DeviceId, Vec<RouteCandidate>>,
) {
    let mut next = BTreeMap::new();
    for peer in authority
        .peers
        .iter()
        .filter(|p| p.trust == TrustState::Trusted)
    {
        let previous = routes.get(&peer.id);
        let mut known = mdns.get(&peer.id).cloned().unwrap_or_default();
        for hint in &peer.routes {
            if hint.source.is_public() || hint.endpoint.host.parse::<std::net::IpAddr>().is_err() {
                continue;
            }
            let mut endpoint = hint.endpoint.clone();
            endpoint.port = 42425;
            if !known.iter().any(|r| r.endpoint == endpoint) && known.len() < 32 {
                let candidate = previous
                    .and_then(|p| p.iter().find(|r| r.endpoint == endpoint))
                    .cloned()
                    .unwrap_or_else(|| {
                        RouteCandidate::new(
                            endpoint,
                            RouteSource::CachedDirect,
                            RouteHealth::Unknown,
                            Capabilities::INPUT_SHARING,
                        )
                    });
                known.push(candidate);
            }
        }
        next.insert(peer.id, known);
    }
    *routes = next;
}

/// 独立输入服务当前可用于连接的对端地址集合，用来判断主连接是否刚刚
/// 发现了新网络上的地址，需要重新浏览自己的 Bonjour 命名空间。
fn input_route_endpoints(
    routes: &BTreeMap<DeviceId, Vec<RouteCandidate>>,
) -> BTreeSet<(DeviceId, Endpoint)> {
    routes
        .iter()
        .flat_map(|(peer, entries)| {
            entries
                .iter()
                .map(move |route| (*peer, route.endpoint.clone()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use deskfuse_runtime::DeviceNetworkSnapshot;

    fn candidate(host: &str, port: u16, source: RouteSource) -> RouteCandidate {
        RouteCandidate::new(
            Endpoint::new(host, port).unwrap(),
            source,
            RouteHealth::Unknown,
            Capabilities::INPUT_SHARING,
        )
    }

    fn authority(peer: DeviceId, hints: Vec<RouteCandidate>) -> Authority {
        Authority {
            local: DeviceId::from([1; 32]),
            network: DeviceNetworkSnapshot::default(),
            peers: vec![PeerSnapshot {
                id: peer,
                display_name: "peer".into(),
                public_key: [3; 32],
                liveness: LivenessState::Online,
                trust: TrustState::Trusted,
                connection: ConnectionState::Connected,
                routes: hints,
                selected_route: None,
            }],
        }
    }

    fn hosts(routes: &BTreeMap<DeviceId, Vec<RouteCandidate>>, peer: DeviceId) -> Vec<String> {
        routes[&peer]
            .iter()
            .map(|r| r.endpoint.host.clone())
            .collect()
    }

    #[test]
    fn bonjour_update_keeps_control_route_hints() {
        let peer = DeviceId::from([2; 32]);
        let authority = authority(
            peer,
            vec![candidate("10.10.10.2", 42424, RouteSource::Mdns)],
        );
        let mut mdns = BTreeMap::new();
        let mut routes = BTreeMap::new();
        rebuild_routes(&authority, &mdns, &mut routes);
        assert_eq!(hosts(&routes, peer), ["10.10.10.2"]);

        // Bonjour 只报告另一块网卡的地址时，主连接提示仍须保留且端口改为 42425。
        mdns.insert(peer, vec![candidate("10.0.0.9", 42425, RouteSource::Mdns)]);
        rebuild_routes(&authority, &mdns, &mut routes);
        assert_eq!(hosts(&routes, peer), ["10.0.0.9", "10.10.10.2"]);
        assert!(routes[&peer].iter().all(|r| r.endpoint.port == 42425));
    }

    #[test]
    fn withdrawn_hints_and_untrusted_peers_are_dropped() {
        let peer = DeviceId::from([2; 32]);
        let mut routes = BTreeMap::new();
        let mdns = BTreeMap::new();
        rebuild_routes(
            &authority(peer, vec![candidate("10.0.0.9", 42424, RouteSource::Mdns)]),
            &mdns,
            &mut routes,
        );
        rebuild_routes(
            &authority(
                peer,
                vec![candidate("10.10.10.2", 42424, RouteSource::Mdns)],
            ),
            &mdns,
            &mut routes,
        );
        assert_eq!(hosts(&routes, peer), ["10.10.10.2"]);

        let mut untrusted = authority(peer, vec![]);
        untrusted.peers[0].trust = TrustState::Blocked;
        rebuild_routes(&untrusted, &mdns, &mut routes);
        assert!(routes.is_empty());
    }
}
