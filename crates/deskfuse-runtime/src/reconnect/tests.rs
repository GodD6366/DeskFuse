use super::*;

#[derive(Clone, Default)]
struct ProbeTransport {
    inner: MemoryTransport,
    alive: Arc<Mutex<BTreeSet<DeviceId>>>,
    hanging: Arc<Mutex<BTreeSet<DeviceId>>>,
    started: Arc<AtomicUsize>,
}
impl TransportProvider for ProbeTransport {
    fn control_generation(&self, peer: DeviceId) -> Option<u64> {
        self.inner.control_generation(peer)
    }

    fn control_session_alive(&self, peer: DeviceId) -> Option<bool> {
        Some(self.alive.lock().unwrap().contains(&peer))
    }
    fn connect(
        &self,
        hello: Hello,
        peer: DeviceId,
        route: RouteCandidate,
    ) -> ProviderFuture<TransportConnection> {
        let this = self.clone();
        Box::pin(async move {
            this.started.fetch_add(1, Ordering::Relaxed);
            let hanging = this.hanging.lock().unwrap().contains(&peer);
            if hanging {
                futures::future::pending::<()>().await;
            }
            let result = this.inner.connect(hello, peer, route).await?;
            this.alive.lock().unwrap().insert(peer);
            Ok(result)
        })
    }
    fn disconnect(&self, peer: DeviceId) -> ProviderFuture<()> {
        self.alive.lock().unwrap().remove(&peer);
        self.inner.disconnect(peer)
    }
}
fn actor(transport: ProbeTransport) -> Actor {
    let config = RuntimeConfig::default();
    let identity = Arc::new(LocalIdentity::from_bytes(&[80; 32]).unwrap());
    let hello = make_local_hello(&identity, &config).unwrap();
    let snapshot = NetworkSnapshot::new(0, identity.device_id(), vec![]).unwrap();
    let (snapshot_tx, _) = watch::channel(snapshot.clone());
    let (view_tx, _) = watch::channel(SnapshotView {
        snapshot,
        discovered_peers: vec![],
        device_network: Default::default(),
    });
    let (_, command_rx) = mpsc::channel(16);
    let (provider_tx, provider_rx) = mpsc::channel(16);
    let (event_tx, _) = broadcast::channel(16);
    Actor::new(
        config,
        identity,
        hello,
        Arc::new(MemoryDiscovery::new()),
        Arc::new(transport),
        Arc::new(MemoryTrustStore::new()),
        Arc::new(MemoryRuntimeStore::new()),
        command_rx,
        provider_rx,
        provider_tx,
        event_tx,
        snapshot_tx,
        view_tx,
    )
}
fn add_peer(actor: &mut Actor, transport: &ProbeTransport, seed: u8) -> (DeviceId, RouteCandidate) {
    let identity = LocalIdentity::from_bytes(&[seed; 32]).unwrap();
    let peer = identity.device_id();
    let hello = make_local_hello(&identity, &RuntimeConfig::default()).unwrap();
    transport.inner.set_remote_hello(peer, hello.clone());
    let route = RouteCandidate::new(
        Endpoint::new("127.0.0.1", 4100 + u16::from(seed)).unwrap(),
        RouteSource::Manual,
        RouteHealth::Healthy,
        Capabilities::NETWORK,
    );
    actor.trusted.insert(peer);
    actor.peers.insert(
        peer,
        PeerState {
            id: peer,
            public_key: hello.public_key.try_into().unwrap(),
            display_name: "test".into(),
            routes: BTreeMap::from([(route.id, route.clone())]),
            liveness: LivenessState::Online,
            trust: TrustState::Trusted,
            connection: ConnectionState::Disconnected,
            preferred_route: Some(route.id),
            active_route: None,
            active_capabilities: None,
            pending_pairing: None,
        },
    );
    (peer, route)
}
async fn complete(actor: &mut Actor) {
    let result = tokio::time::timeout(Duration::from_secs(3), actor.dial_result_rx.recv())
        .await
        .unwrap()
        .unwrap();
    if let Err(error) = &result.result {
        eprintln!("dial result: {error}");
    }
    actor.handle_dial_result(result).await;
}
async fn connect(actor: &mut Actor, peer: DeviceId) {
    actor.connect_peer(peer).await;
    complete(actor).await;
    assert_eq!(actor.peers[&peer].connection, ConnectionState::Connected);
}
fn expire(actor: &mut Actor, peer: DeviceId) {
    actor.recovery.retries.get_mut(&peer).unwrap().due = Instant::now();
}

#[tokio::test]
async fn incoming_same_process_and_address_expedites_each_new_recovery() {
    let transport = ProbeTransport::default();
    let mut actor = actor(transport.clone());
    let (peer, route) = add_peer(&mut actor, &transport, 81);
    let identity = LocalIdentity::from_bytes(&[81; 32]).unwrap();
    let hello =
        deskfuse_transport::PeerHello::signed(&identity, "remote", Capabilities::NETWORK, [1; 32])
            .unwrap();
    transport.inner.set_remote_hello(peer, hello.to_wire());
    for _ in 0..2 {
        actor
            .handle_control_peer(deskfuse_transport::ControlPeerConnected {
                hello: hello.clone(),
                endpoint: route.endpoint.clone(),
            })
            .await;
        complete(&mut actor).await;
        assert_eq!(actor.peers[&peer].connection, ConnectionState::Connected);
        // Duplicate live notifications leave the healthy session untouched.
        let generation = transport.control_generation(peer);
        actor
            .handle_control_peer(deskfuse_transport::ControlPeerConnected {
                hello: hello.clone(),
                endpoint: route.endpoint.clone(),
            })
            .await;
        assert_eq!(transport.control_generation(peer), generation);
        assert!(!actor.recovery.attempts.contains_key(&peer));
        transport.alive.lock().unwrap().remove(&peer);
        actor.recovery_tick().await;
        actor.recovery.retries.get_mut(&peer).unwrap().due =
            Instant::now() + Duration::from_secs(30);
    }
}

#[tokio::test]
async fn explicit_connect_recovers_dead_session_before_periodic_tick() {
    let transport = ProbeTransport::default();
    let mut actor = actor(transport.clone());
    let (peer, _) = add_peer(&mut actor, &transport, 82);
    connect(&mut actor, peer).await;
    transport.alive.lock().unwrap().remove(&peer);
    actor
        .handle_command(RuntimeCommand::Connect { peer_id: peer })
        .await;
    complete(&mut actor).await;
    assert_eq!(actor.peers[&peer].connection, ConnectionState::Connected);
    assert_eq!(transport.started.load(Ordering::Relaxed), 2);
}

#[tokio::test]
async fn repeated_restart_notifications_do_not_cancel_replacement_dial() {
    let transport = ProbeTransport::default();
    let mut actor = actor(transport.clone());
    let (peer, route) = add_peer(&mut actor, &transport, 83);
    connect(&mut actor, peer).await;
    let identity = LocalIdentity::from_bytes(&[83; 32]).unwrap();
    let hello = deskfuse_transport::PeerHello::signed(
        &identity,
        "restarted",
        Capabilities::NETWORK,
        [2; 32],
    )
    .unwrap();
    transport.inner.set_remote_hello(peer, hello.to_wire());
    actor
        .handle_control_peer(deskfuse_transport::ControlPeerConnected {
            hello: hello.clone(),
            endpoint: route.endpoint.clone(),
        })
        .await;
    let generation = actor.recovery.attempts[&peer].generation;
    let mut other_interface = route.endpoint;
    other_interface.host = "127.0.0.2".into();
    actor
        .handle_control_peer(deskfuse_transport::ControlPeerConnected {
            hello,
            endpoint: other_interface,
        })
        .await;
    assert_eq!(actor.recovery.attempts[&peer].generation, generation);
    complete(&mut actor).await;
    assert_eq!(actor.peers[&peer].connection, ConnectionState::Connected);
}

#[test]
fn backoff_is_bounded() {
    assert_eq!(
        (0..8).map(|n| retry_delay(n).as_secs()).collect::<Vec<_>>(),
        vec![1, 2, 4, 8, 16, 30, 30, 30]
    );
    assert_eq!(retry_delay(u32::MAX), Duration::from_secs(30));
}

#[tokio::test]
async fn closed_session_clears_capabilities_and_recovers_without_discovery() {
    let transport = ProbeTransport::default();
    let mut actor = actor(transport.clone());
    let (peer, _) = add_peer(&mut actor, &transport, 81);
    connect(&mut actor, peer).await;
    assert!(actor.peers[&peer].active_capabilities.is_some());
    transport.alive.lock().unwrap().remove(&peer);
    actor.recovery_tick().await;
    assert_eq!(actor.peers[&peer].connection, ConnectionState::Disconnected);
    assert!(actor.peers[&peer].active_route.is_none());
    assert!(actor.peers[&peer].active_capabilities.is_none());
    assert_eq!(transport.started.load(Ordering::Relaxed), 1);
    expire(&mut actor, peer);
    actor.recovery_tick().await;
    complete(&mut actor).await;
    assert_eq!(actor.peers[&peer].connection, ConnectionState::Connected);
    assert!(!actor.recovery.retries.contains_key(&peer));
}

#[tokio::test]
async fn failed_routes_retry_with_backoff_and_reset_after_success() {
    let transport = ProbeTransport::default();
    let mut actor = actor(transport.clone());
    let (peer, route) = add_peer(&mut actor, &transport, 82);
    transport.inner.fail_route(route.id);
    actor.connect_peer(peer).await;
    complete(&mut actor).await;
    for failure in 1..=3 {
        assert_eq!(actor.peers[&peer].connection, ConnectionState::Failed);
        assert_eq!(actor.recovery.retries[&peer].failures, failure);
        assert_eq!(actor.peers[&peer].routes[&route.id].failure_count, failure);
        actor.recovery_tick().await;
        assert!(!actor.recovery.attempts.contains_key(&peer));
        // Identical discovery cannot bypass the retry deadline.
        actor.reconnect_after_discovery(peer).await;
        assert!(!actor.recovery.attempts.contains_key(&peer));
        expire(&mut actor, peer);
        if failure == 3 {
            transport.inner.failures.lock().unwrap().clear();
        }
        actor.recovery_tick().await;
        complete(&mut actor).await;
    }
    assert_eq!(actor.peers[&peer].connection, ConnectionState::Connected);
    assert_eq!(actor.peers[&peer].routes[&route.id].failure_count, 3);
    assert!(!actor.recovery.retries.contains_key(&peer));
    transport.alive.lock().unwrap().clear();
    actor.recovery_tick().await;
    assert_eq!(actor.recovery.retries[&peer].failures, 0);
}

#[tokio::test]
async fn connected_discovery_is_idempotent() {
    let transport = ProbeTransport::default();
    let mut actor = actor(transport.clone());
    let (peer, _) = add_peer(&mut actor, &transport, 83);
    connect(&mut actor, peer).await;
    actor.reconnect_after_discovery(peer).await;
    actor.system_woke().await;
    assert_eq!(transport.started.load(Ordering::Relaxed), 1);
    assert_eq!(actor.peers[&peer].connection, ConnectionState::Connected);
}

#[tokio::test]
async fn manual_disconnect_holds_until_explicit_connect() {
    let transport = ProbeTransport::default();
    let mut actor = actor(transport.clone());
    let (peer, _) = add_peer(&mut actor, &transport, 83);
    connect(&mut actor, peer).await;
    actor
        .handle_command(RuntimeCommand::Disconnect { peer_id: peer })
        .await;
    assert_eq!(actor.peers[&peer].connection, ConnectionState::Disconnected);
    actor.reconnect_after_discovery(peer).await;
    actor.system_woke().await;
    actor.auto_connect_trusted_peers().await;
    actor.recovery_tick().await;
    assert_eq!(actor.peers[&peer].connection, ConnectionState::Disconnected);
    assert!(actor.recovery.attempts.is_empty());
    actor
        .handle_command(RuntimeCommand::Connect { peer_id: peer })
        .await;
    complete(&mut actor).await;
    assert_eq!(actor.peers[&peer].connection, ConnectionState::Connected);
}

#[tokio::test]
async fn hanging_peer_does_not_block_others_or_shutdown_and_duplicate_dials() {
    let transport = ProbeTransport::default();
    let mut actor = actor(transport.clone());
    let (slow, _) = add_peer(&mut actor, &transport, 84);
    let (fast, _) = add_peer(&mut actor, &transport, 85);
    transport.hanging.lock().unwrap().insert(slow);
    actor.connect_peer(slow).await;
    for _ in 0..10 {
        actor.connect_peer(slow).await;
        actor.reconnect_after_discovery(slow).await;
    }
    connect(&mut actor, fast).await;
    assert_eq!(transport.started.load(Ordering::Relaxed), 2);
    tokio::time::timeout(Duration::from_millis(500), actor.cancel_all_reconnects())
        .await
        .unwrap();
    assert!(actor.recovery.attempts.is_empty());
    assert_eq!(actor.peers[&fast].connection, ConnectionState::Connected);
}

#[tokio::test]
async fn obsolete_completion_cannot_replace_or_close_new_session() {
    let transport = ProbeTransport::default();
    let mut actor = actor(transport.clone());
    let (peer, _) = add_peer(&mut actor, &transport, 86);
    actor.connect_peer(peer).await;
    let obsolete = actor.dial_result_rx.recv().await.unwrap();
    actor.cancel_reconnect(peer).await;
    connect(&mut actor, peer).await;
    let calls = transport.inner.disconnect_count();
    actor.handle_dial_result(obsolete).await;
    assert_eq!(transport.inner.disconnect_count(), calls);
    assert_eq!(transport.control_session_alive(peer), Some(true));
    assert_eq!(actor.peers[&peer].connection, ConnectionState::Connected);
}

#[tokio::test]
async fn revoke_and_forget_cancel_completed_but_unpublished_connections() {
    for forget in [false, true] {
        let transport = ProbeTransport::default();
        let mut actor = actor(transport.clone());
        let (peer, _) = add_peer(&mut actor, &transport, 87);
        actor.connect_peer(peer).await;
        let obsolete = actor.dial_result_rx.recv().await.unwrap();
        if forget {
            actor.forget_peer(peer).await;
        } else {
            actor.revoke_peer(peer).await;
        }
        actor.handle_dial_result(obsolete).await;
        actor.system_woke().await;
        assert!(!actor.recovery.attempts.contains_key(&peer));
        assert!(!actor.recovery.retries.contains_key(&peer));
        assert_eq!(transport.control_session_alive(peer), Some(false));
        assert_eq!(transport.started.load(Ordering::Relaxed), 1);
    }
}

#[tokio::test]
async fn wake_refreshes_addresses_and_expedites_offline_peer() {
    let transport = ProbeTransport::default();
    let mut actor = actor(transport.clone());
    let (peer, route) = add_peer(&mut actor, &transport, 88);
    transport.inner.fail_route(route.id);
    actor.connect_peer(peer).await;
    complete(&mut actor).await;
    let discovery = MemoryDiscovery::new();
    let fresh = RouteCandidate::new(
        Endpoint::new("127.0.0.1", 4999).unwrap(),
        RouteSource::Mdns,
        RouteHealth::Healthy,
        Capabilities::NETWORK,
    );
    discovery.push(DiscoveryEvent::RouteAdded {
        peer_id: peer,
        route: fresh.clone(),
    });
    actor.discovery = Arc::new(discovery);
    actor.system_woke().await;
    complete(&mut actor).await;
    assert_eq!(actor.peers[&peer].active_route, Some(fresh.id));
    assert_eq!(actor.peers[&peer].connection, ConnectionState::Connected);
}

#[tokio::test]
async fn wake_starts_a_lan_probe_for_the_network_it_woke_into() {
    let transport = ProbeTransport::default();
    let mut actor = actor(transport);
    actor.lan_networks = Arc::new(|| vec![(std::net::Ipv4Addr::new(10, 0, 0, 1), 24)]);
    actor.system_woke().await;
    assert!(actor.lan_probe.is_some());
}

#[tokio::test]
async fn new_address_expedites_retry_but_same_address_does_not() {
    let transport = ProbeTransport::default();
    let mut actor = actor(transport.clone());
    let (peer, route) = add_peer(&mut actor, &transport, 89);
    transport.inner.fail_route(route.id);
    actor.connect_peer(peer).await;
    complete(&mut actor).await;
    actor
        .handle_discovery_event(DiscoveryEvent::RouteAdded {
            peer_id: peer,
            route: route.clone(),
        })
        .await;
    assert!(!actor.recovery.attempts.contains_key(&peer));
    let fresh = RouteCandidate::new(
        Endpoint::new("127.0.0.1", 4998).unwrap(),
        RouteSource::Manual,
        RouteHealth::Healthy,
        Capabilities::NETWORK,
    );
    actor
        .handle_discovery_event(DiscoveryEvent::RouteAdded {
            peer_id: peer,
            route: fresh.clone(),
        })
        .await;
    complete(&mut actor).await;
    assert_eq!(actor.peers[&peer].active_route, Some(fresh.id));
}

#[tokio::test]
async fn removed_route_result_is_rejected_and_fallback_remains_retryable() {
    let transport = ProbeTransport::default();
    let mut actor = actor(transport.clone());
    let (peer, route) = add_peer(&mut actor, &transport, 90);
    actor.connect_peer(peer).await;
    let obsolete = actor.dial_result_rx.recv().await.unwrap();
    actor.peers.get_mut(&peer).unwrap().routes.remove(&route.id);
    actor.handle_dial_result(obsolete).await;
    assert_eq!(transport.control_session_alive(peer), Some(false));
    assert_ne!(actor.peers[&peer].connection, ConnectionState::Connected);
    assert!(actor.recovery.retries.contains_key(&peer));
}

#[tokio::test]
async fn network_removal_suspends_even_legacy_trust_until_explicit_connect() {
    let transport = ProbeTransport::default();
    let mut actor = actor(transport.clone());
    let (peer, _) = add_peer(&mut actor, &transport, 92);
    actor.suspend_reconnect(peer).await;
    actor.auto_connect_trusted_peers().await;
    actor.system_woke().await;
    assert!(actor.recovery.attempts.is_empty());
    assert!(actor.recovery.retries.is_empty());
    actor
        .handle_command(RuntimeCommand::Connect { peer_id: peer })
        .await;
    complete(&mut actor).await;
    assert_eq!(actor.peers[&peer].connection, ConnectionState::Connected);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_quic_peer_restart_recovers_same_route_and_delivers_one_clipboard() {
    use deskfuse_protocol::Message;
    use deskfuse_transport::{ControlTransport, PeerHello, QuinnTransport, QuinnTransportConfig};
    let cert = rcgen::generate_simple_self_signed(vec!["deskfuse.local".into()]).unwrap();
    let der = cert.cert.der().to_vec();
    let key = cert.key_pair.serialize_der();
    let config = |addr| {
        QuinnTransportConfig::new(
            addr,
            addr,
            "deskfuse.local",
            der.clone(),
            key.clone(),
            der.clone(),
        )
    };
    let local = LocalIdentity::from_bytes(&[80; 32]).unwrap();
    let remote = LocalIdentity::from_bytes(&[94; 32]).unwrap();
    let caps = Capabilities::NETWORK | Capabilities::CLIPBOARD;
    let local_hello = PeerHello::signed(&local, "local", caps, [80; 32]).unwrap();
    let remote_hello = PeerHello::signed(&remote, "remote", caps, [94; 32]).unwrap();
    let local_transport =
        QuinnTransport::bind(local_hello.clone(), config("127.0.0.1:0".parse().unwrap()))
            .await
            .unwrap();
    let remote_transport =
        QuinnTransport::bind(remote_hello.clone(), config("127.0.0.1:0".parse().unwrap()))
            .await
            .unwrap();
    remote_transport
        .configure_identity(Arc::new(LocalIdentity::from_bytes(&[94; 32]).unwrap()))
        .unwrap();
    remote_transport
        .register_peer_identity(local.device_id(), local_hello.public_key)
        .await
        .unwrap();
    let remote_addr = remote_transport.local_addr().unwrap();
    let adapter = ControlTransportAdapter::new(local_transport);
    adapter
        .configure_identity(Arc::new(LocalIdentity::from_bytes(&[80; 32]).unwrap()))
        .unwrap();
    let fake = ProbeTransport::default();
    let mut actor = actor(fake.clone());
    let (peer, route) = add_peer(&mut actor, &fake, 94);
    actor
        .peers
        .get_mut(&peer)
        .unwrap()
        .routes
        .get_mut(&route.id)
        .unwrap()
        .endpoint = Endpoint::new("127.0.0.1", remote_addr.port()).unwrap();
    actor.transport = Arc::new(adapter.clone());
    connect(&mut actor, peer).await;
    assert_eq!(adapter.control_session_alive(peer), Some(true));
    let generation = adapter.control_generation(peer).unwrap();
    for _ in 0..3 {
        tokio::time::timeout(
            Duration::from_secs(2),
            adapter.probe_control(peer, generation),
        )
        .await
        .expect("real QUIC heartbeat deadline")
        .expect("real QUIC heartbeat");
    }
    // Stop and rebind a real QUIC endpoint with the same durable peer identity.
    remote_transport.shutdown().await.unwrap();
    drop(remote_transport);
    tokio::time::timeout(Duration::from_secs(3), async {
        while adapter.control_session_alive(peer) != Some(false) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    actor.recovery_tick().await;
    assert_eq!(actor.peers[&peer].connection, ConnectionState::Disconnected);
    // Quinn drains a closed UDP endpoint before releasing its socket.
    let restarted = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match QuinnTransport::bind(remote_hello.clone(), config(remote_addr)).await {
                Ok(transport) => break transport,
                Err(error) if error.to_string().contains("Address already in use") => {
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
                Err(error) => panic!("restart bind failed: {error}"),
            }
        }
    })
    .await
    .unwrap();
    restarted
        .configure_identity(Arc::new(LocalIdentity::from_bytes(&[94; 32]).unwrap()))
        .unwrap();
    restarted
        .register_peer_identity(local.device_id(), local_hello.public_key)
        .await
        .unwrap();
    let mut inbox = restarted.messages();
    expire(&mut actor, peer);
    actor.recovery_tick().await;
    complete(&mut actor).await;
    assert_eq!(actor.peers[&peer].connection, ConnectionState::Connected);
    let payload = ClipboardPayload::new("text/plain", b"after reconnect".to_vec(), 42).unwrap();
    adapter
        .send_message(peer, PeerMessage::Clipboard(payload))
        .await
        .unwrap();
    let (sender, message) = tokio::time::timeout(Duration::from_secs(3), inbox.next())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sender, local.device_id());
    assert!(matches!(message, Message::ClipboardUpdate(update) if update.sequence == 42));
    assert!(
        tokio::time::timeout(Duration::from_millis(100), inbox.next())
            .await
            .is_err()
    );
    adapter.shutdown().await.unwrap();
    restarted.shutdown().await.unwrap();
}

/// 两个真实 QUIC 端点：`local` 由 actor 驱动，`remote` 直接操作传输层。
struct QuicPair {
    actor: Actor,
    adapter: ControlTransportAdapter<deskfuse_transport::QuinnTransport>,
    local_endpoint: Endpoint,
    local_id: DeviceId,
    remote: deskfuse_transport::QuinnTransport,
    peer: DeviceId,
    route: RouteId,
    // watch 没有接收者时 send 不会更新值，测试必须持有订阅。
    snapshot_rx: watch::Receiver<NetworkSnapshot>,
    view_rx: watch::Receiver<SnapshotView>,
}

async fn quic_pair(remote_seed: u8) -> QuicPair {
    use deskfuse_transport::{ControlTransport, PeerHello, QuinnTransport, QuinnTransportConfig};
    let cert = rcgen::generate_simple_self_signed(vec!["deskfuse.local".into()]).unwrap();
    let der = cert.cert.der().to_vec();
    let key = cert.key_pair.serialize_der();
    let config = |addr| {
        QuinnTransportConfig::new(
            addr,
            addr,
            "deskfuse.local",
            der.clone(),
            key.clone(),
            der.clone(),
        )
    };
    let local = LocalIdentity::from_bytes(&[80; 32]).unwrap();
    let remote = LocalIdentity::from_bytes(&[remote_seed; 32]).unwrap();
    let caps = Capabilities::NETWORK | Capabilities::CLIPBOARD;
    let local_hello = PeerHello::signed(&local, "local", caps, [80; 32]).unwrap();
    let remote_hello = PeerHello::signed(&remote, "remote", caps, [remote_seed; 32]).unwrap();
    let local_transport =
        QuinnTransport::bind(local_hello.clone(), config("127.0.0.1:0".parse().unwrap()))
            .await
            .unwrap();
    let remote_transport =
        QuinnTransport::bind(remote_hello.clone(), config("127.0.0.1:0".parse().unwrap()))
            .await
            .unwrap();
    remote_transport
        .configure_identity(Arc::new(
            LocalIdentity::from_bytes(&[remote_seed; 32]).unwrap(),
        ))
        .unwrap();
    remote_transport
        .register_peer_identity(local.device_id(), local_hello.public_key)
        .await
        .unwrap();
    let remote_addr = remote_transport.local_addr().unwrap();
    let local_addr = local_transport.local_addr().unwrap();
    let adapter = ControlTransportAdapter::new(local_transport);
    adapter
        .configure_identity(Arc::new(LocalIdentity::from_bytes(&[80; 32]).unwrap()))
        .unwrap();
    let fake = ProbeTransport::default();
    let mut actor = actor(fake.clone());
    let (peer, route) = add_peer(&mut actor, &fake, remote_seed);
    actor
        .peers
        .get_mut(&peer)
        .unwrap()
        .routes
        .get_mut(&route.id)
        .unwrap()
        .endpoint = Endpoint::new("127.0.0.1", remote_addr.port()).unwrap();
    actor.transport = Arc::new(adapter.clone());
    let snapshot_rx = actor.snapshot_tx.subscribe();
    let view_rx = actor.view_tx.subscribe();
    QuicPair {
        snapshot_rx,
        view_rx,
        actor,
        adapter,
        local_endpoint: Endpoint::new("127.0.0.1", local_addr.port()).unwrap(),
        local_id: local.device_id(),
        remote: remote_transport,
        peer,
        route: route.id,
    }
}

impl QuicPair {
    fn published(&self) -> (ConnectionState, Option<PeerLinkHoldKind>) {
        let connection = self
            .snapshot_rx
            .borrow()
            .peers
            .iter()
            .find(|peer| peer.id == self.peer)
            .unwrap()
            .connection;
        let hold = self
            .view_rx
            .borrow()
            .device_network
            .link_holds
            .iter()
            .find(|hold| hold.peer_id == self.peer)
            .map(|hold| hold.kind);
        (connection, hold)
    }

    async fn wait_inbound(&self, alive: bool) {
        tokio::time::timeout(Duration::from_secs(3), async {
            while self.adapter.inbound_control_alive(self.peer) != Some(alive) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("inbound control session state");
    }

    async fn shutdown(self) {
        self.adapter.shutdown().await.unwrap();
        self.remote.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn one_sided_session_is_published_as_connecting_until_peer_dials_back() {
    use deskfuse_transport::ControlTransport;
    let mut pair = quic_pair(95).await;
    let peer = pair.peer;
    connect(&mut pair.actor, peer).await;
    // 只有本机→对方一条会话：对方那边此时还不认为已连接，本机也不能宣布在线。
    pair.actor.publish();
    assert_eq!(
        pair.published(),
        (
            ConnectionState::Connecting,
            Some(PeerLinkHoldKind::AwaitingPeer)
        )
    );
    // 对方回拨后两条会话都在，双方一致显示已连接。
    pair.remote
        .connect_with_route(pair.local_id, pair.local_endpoint.clone())
        .await
        .unwrap();
    pair.wait_inbound(true).await;
    pair.actor.recovery_tick().await;
    assert_eq!(pair.published(), (ConnectionState::Connected, None));
    pair.shutdown().await;
}

#[tokio::test]
async fn remote_suspension_is_rejected_and_reported_without_failing_the_route() {
    use deskfuse_transport::ControlTransport;
    let mut pair = quic_pair(96).await;
    let peer = pair.peer;
    connect(&mut pair.actor, peer).await;
    assert_eq!(pair.adapter.control_session_alive(peer), Some(true));
    // 对方点"断开"：关闭本机拨入的会话，而不是让本机继续单方面显示已连接。
    pair.remote.set_control_suspended(pair.local_id, true);
    tokio::time::timeout(Duration::from_secs(3), async {
        while pair.adapter.control_session_alive(peer) != Some(false) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    pair.actor.recovery_tick().await;
    expire(&mut pair.actor, peer);
    pair.actor.recovery_tick().await;
    complete(&mut pair.actor).await;
    let state = &pair.actor.peers[&peer];
    assert_eq!(state.connection, ConnectionState::Failed);
    assert_eq!(state.routes[&pair.route].failure_count, 0);
    assert_ne!(state.routes[&pair.route].health, RouteHealth::Unavailable);
    assert_eq!(
        pair.published(),
        (
            ConnectionState::Failed,
            Some(PeerLinkHoldKind::RemoteSuspended)
        )
    );
    assert!(
        pair.actor.recovery.retries[&peer].due > Instant::now() + Duration::from_secs(200),
        "对方挂起时不应频繁重试"
    );
    // 对方恢复并拨入本机：立即回拨，双方恢复一致的已连接状态。
    pair.remote.set_control_suspended(pair.local_id, false);
    let mut incoming = pair.adapter.control_peers();
    pair.remote
        .connect_with_route(pair.local_id, pair.local_endpoint.clone())
        .await
        .unwrap();
    let hint = tokio::time::timeout(Duration::from_secs(3), incoming.next())
        .await
        .unwrap()
        .unwrap();
    pair.actor.handle_control_peer(hint).await;
    complete(&mut pair.actor).await;
    pair.actor.publish();
    assert_eq!(pair.published(), (ConnectionState::Connected, None));
    pair.shutdown().await;
}

#[tokio::test]
async fn local_disconnect_rejects_peer_dial_in_so_both_sides_agree() {
    use deskfuse_transport::{ControlTransport, TransportError};
    let mut pair = quic_pair(97).await;
    let peer = pair.peer;
    connect(&mut pair.actor, peer).await;
    pair.remote
        .connect_with_route(pair.local_id, pair.local_endpoint.clone())
        .await
        .unwrap();
    pair.wait_inbound(true).await;
    pair.actor
        .handle_command(RuntimeCommand::Disconnect { peer_id: peer })
        .await;
    // 本机断开同时关闭对方拨入的会话，对方随后的重拨被明确拒绝。
    pair.wait_inbound(false).await;
    let rejected = pair
        .remote
        .connect_with_route(pair.local_id, pair.local_endpoint.clone())
        .await;
    assert!(matches!(rejected, Err(TransportError::PeerSuspended(id)) if id == pair.local_id));
    assert_eq!(
        pair.published(),
        (
            ConnectionState::Disconnected,
            Some(PeerLinkHoldKind::LocalSuspended)
        )
    );
    // 显式连接解除挂起，对方又可以拨入。
    pair.actor
        .handle_command(RuntimeCommand::Connect { peer_id: peer })
        .await;
    complete(&mut pair.actor).await;
    pair.remote
        .connect_with_route(pair.local_id, pair.local_endpoint.clone())
        .await
        .unwrap();
    pair.wait_inbound(true).await;
    pair.actor.recovery_tick().await;
    assert_eq!(pair.published(), (ConnectionState::Connected, None));
    pair.shutdown().await;
}

#[tokio::test]
async fn explicit_refresh_retries_failed_peer_immediately() {
    let transport = ProbeTransport::default();
    let mut actor = actor(transport.clone());
    let (peer, route) = add_peer(&mut actor, &transport, 98);
    transport.inner.fail_route(route.id);
    actor.connect_peer(peer).await;
    complete(&mut actor).await;
    assert_eq!(actor.peers[&peer].connection, ConnectionState::Failed);
    // 失败会清空 preferred_route；刷新仍必须绕过退避立即重试。
    assert!(actor.peers[&peer].preferred_route.is_none());
    assert!(actor.recovery.retries[&peer].due > Instant::now());
    transport.inner.failures.lock().unwrap().clear();
    actor.handle_command(RuntimeCommand::RefreshPeers).await;
    assert!(actor.recovery.attempts.contains_key(&peer));
    complete(&mut actor).await;
    assert_eq!(actor.peers[&peer].connection, ConnectionState::Connected);
}

#[tokio::test]
async fn only_advertised_peers_stay_on_the_short_retry_ladder() {
    let transport = ProbeTransport::default();
    let mut actor = actor(transport.clone());
    let (peer, _) = add_peer(&mut actor, &transport, 99);
    // 持久化地址不再算"在场"，否则长期离线的设备永远每 30 秒全量探测。
    assert!(!actor.peer_present(peer));
    assert_eq!(retry_backoff_for(8, false), RETRY_MAX_DELAY);
    actor.observe_peer_discovery(peer);
    assert!(actor.peer_present(peer));
    assert_eq!(retry_backoff_for(8, true), Duration::from_secs(30));
}
