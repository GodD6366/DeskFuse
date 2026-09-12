use super::*;
use std::time::Duration;
use unidesk_transport::{ControlTransport, PeerHello, QuinnTransport, QuinnTransportConfig};
struct Node {
    id: DeviceId,
    runtime: RuntimeHandle,
    discovery: MemoryDiscovery,
    transport: QuinnTransport,
    events: broadcast::Receiver<RuntimeEvent>,
    trust: unidesk_identity::MemoryTrustStore,
    store: MemoryRuntimeStore,
}
impl Node {
    async fn new(seed: u8, name: &str, cert: &[u8], key: &[u8]) -> Self {
        Self::with_trust(
            seed,
            name,
            cert,
            key,
            unidesk_identity::MemoryTrustStore::new(),
        )
        .await
    }
    async fn with_trust(
        seed: u8,
        name: &str,
        cert: &[u8],
        key: &[u8],
        trust: unidesk_identity::MemoryTrustStore,
    ) -> Self {
        Self::with_state(
            seed,
            name,
            cert,
            key,
            trust,
            MemoryRuntimeStore::new(),
            "127.0.0.1:0".parse().unwrap(),
        )
        .await
    }
    async fn with_state(
        seed: u8,
        name: &str,
        cert: &[u8],
        key: &[u8],
        trust: unidesk_identity::MemoryTrustStore,
        store: MemoryRuntimeStore,
        addr: std::net::SocketAddr,
    ) -> Self {
        let identity = LocalIdentity::from_bytes(&[seed; 32]).unwrap();
        let caps = Capabilities::NETWORK | Capabilities::CLIPBOARD | Capabilities::DEVICE_NETWORK;
        let hello = PeerHello::signed(&identity, name, caps, [seed; 32]).unwrap();
        let transport = QuinnTransport::bind(
            hello,
            QuinnTransportConfig::new(
                addr,
                addr,
                "unidesk.local",
                cert.to_vec(),
                key.to_vec(),
                cert.to_vec(),
            ),
        )
        .await
        .unwrap();
        let discovery = MemoryDiscovery::new();
        let runtime = RuntimeBuilder::new()
            .config(RuntimeConfig {
                display_name: name.into(),
                capabilities: caps,
                ..RuntimeConfig::default()
            })
            .with_identity_provider(MemoryIdentityProvider::from_secret_bytes(&[seed; 32]).unwrap())
            .with_trust_store(trust.clone())
            .with_store(store.clone())
            .with_discovery(discovery.clone())
            .with_control_transport(transport.clone())
            .build()
            .unwrap();
        let events = runtime.events();
        Self {
            id: identity.device_id(),
            runtime,
            discovery,
            transport,
            events,
            trust,
            store,
        }
    }
    fn hint(&self, other: &Self) {
        self.discovery.push(DiscoveryEvent::PeerHint {
            peer_id: other.id,
            display_name: "discovery name is not identity".into(),
            capabilities: Capabilities::NETWORK | Capabilities::DEVICE_NETWORK,
            routes: vec![RouteCandidate::new(
                Endpoint::new("127.0.0.1", other.transport.local_addr().unwrap().port()).unwrap(),
                RouteSource::Manual,
                RouteHealth::Healthy,
                Capabilities::NETWORK | Capabilities::DEVICE_NETWORK,
            )],
        });
    }
    async fn next_pairing(&mut self) -> CorePairingRequest {
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                match self.events.recv().await.unwrap() {
                    RuntimeEvent::PairingRequest(r) => return r,
                    RuntimeEvent::Error(e) => eprintln!("{} error: {e}", self.id),
                    _ => {}
                }
            }
        })
        .await
        .expect("recipient invitation")
    }
}
async fn wait(mut predicate: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(20), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("network state convergence");
}
fn connected(node: &Node, count: usize) -> bool {
    node.runtime
        .snapshot()
        .peers
        .iter()
        .filter(|p| p.trust == TrustState::Trusted && p.connection == ConnectionState::Connected)
        .count()
        == count
}
async fn invite(owner: &Node, recipient: &mut Node) {
    owner.hint(recipient);
    wait(|| {
        owner
            .runtime
            .snapshot_view()
            .discovered_peers
            .iter()
            .any(|p| p.peer_id == recipient.id)
    })
    .await;
    owner
        .runtime
        .send(RuntimeCommand::RequestPairing {
            peer_id: recipient.id,
        })
        .await
        .unwrap();
    let request = recipient.next_pairing().await;
    assert_eq!(request.device_id(), owner.id);
    assert!(
        recipient
            .runtime
            .snapshot()
            .peers
            .iter()
            .all(|p| p.trust != TrustState::Trusted),
        "invitation must not create ordinary application trust"
    );
    assert!(
        owner
            .runtime
            .snapshot_view()
            .device_network
            .pending
            .iter()
            .all(|p| !p.incoming)
    );
    recipient
        .runtime
        .send(RuntimeCommand::ApprovePairing {
            request_id: request.request_id(),
        })
        .await
        .unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_quic_three_nodes_recipient_approval_mesh_clipboard_and_revoke() {
    let cert = rcgen::generate_simple_self_signed(vec!["unidesk.local".into()]).unwrap();
    let der = cert.cert.der().to_vec();
    let key = cert.key_pair.serialize_der();
    let mut a = Node::new(81, "owner A", &der, &key).await;
    let mut b = Node::new(82, "member B", &der, &key).await;
    let mut c = Node::new(83, "member C", &der, &key).await;
    invite(&a, &mut b).await;
    for _ in 0..100 {
        if connected(&a, 1) && connected(&b, 1) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    if !connected(&a, 1) || !connected(&b, 1) {
        eprintln!(
            "A state {:?} B state {:?}",
            a.runtime.snapshot_view(),
            b.runtime.snapshot_view()
        );
        while let Ok(e) = a.events.try_recv() {
            eprintln!("A event {e:?}");
        }
        while let Ok(e) = b.events.try_recv() {
            eprintln!("B event {e:?}");
        }
        panic!("AB did not connect");
    }
    invite(&a, &mut c).await;
    wait(|| connected(&a, 2) && connected(&b, 2) && connected(&c, 2)).await;
    assert_eq!(b.runtime.snapshot_view().device_network.member_ids.len(), 3);
    assert_eq!(
        c.runtime.snapshot_view().device_network.owner_id,
        Some(a.id)
    );
    // A's discovery was the only hint source. B/C mutually connected from the signed roster.
    assert!(b.runtime.snapshot_view().discovered_peers.is_empty());
    assert!(c.runtime.snapshot_view().discovered_peers.is_empty());
    for (sender, targets) in [(&a, [b.id, c.id]), (&b, [a.id, c.id]), (&c, [a.id, b.id])] {
        for target in targets {
            sender
                .runtime
                .send(RuntimeCommand::SendClipboard {
                    peer_id: target,
                    payload: ClipboardPayload::new(
                        "text/plain",
                        format!("{}->{target}", sender.id).into_bytes(),
                        1,
                    )
                    .unwrap(),
                })
                .await
                .unwrap();
        }
    }
    for node in [&mut a, &mut b, &mut c] {
        let mut from = BTreeSet::new();
        tokio::time::timeout(Duration::from_secs(10), async {
            while from.len() < 2 {
                match node.events.recv().await.unwrap() {
                    RuntimeEvent::ClipboardReceived { peer_id, .. } => {
                        from.insert(peer_id);
                    }
                    RuntimeEvent::Error(e) => panic!("clipboard error {e}"),
                    _ => {}
                }
            }
        })
        .await
        .expect("all clipboard directions arrive");
    }
    let epoch = b.runtime.snapshot_view().device_network.epoch;
    a.runtime
        .send(RuntimeCommand::RevokePeer { peer_id: c.id })
        .await
        .unwrap();
    wait(|| {
        b.runtime.snapshot_view().device_network.epoch > epoch
            && b.runtime
                .snapshot()
                .peers
                .iter()
                .any(|p| p.id == c.id && p.trust != TrustState::Trusted)
    })
    .await;
    assert!(
        !b.runtime
            .snapshot_view()
            .device_network
            .member_ids
            .contains(&c.id)
    );
    assert!(
        b.runtime
            .send(RuntimeCommand::SendClipboard {
                peer_id: c.id,
                payload: ClipboardPayload::new("text/plain", b"blocked".to_vec(), 2).unwrap()
            })
            .await
            .is_ok()
    );
    wait(|| {
        c.runtime
            .snapshot_view()
            .device_network
            .network_id
            .is_none()
    })
    .await;
    // Network revocation on B is distinct from a local permanent block. A fresh approved epoch restores B/C automatically.
    invite(&a, &mut c).await;
    wait(|| connected(&a, 2) && connected(&b, 2) && connected(&c, 2)).await;
    a.runtime
        .send(RuntimeCommand::LeaveDeviceNetwork)
        .await
        .unwrap();
    wait(|| {
        [&a, &b, &c].iter().all(|node| {
            node.runtime
                .snapshot_view()
                .device_network
                .network_id
                .is_none()
        })
    })
    .await;
    assert!([&a, &b, &c].iter().all(|node| {
        node.runtime
            .snapshot()
            .peers
            .iter()
            .all(|p| p.trust != TrustState::Trusted)
    }));
    for node in [&a, &b, &c] {
        node.runtime.shutdown_and_wait().await.unwrap();
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_quic_rejection_does_not_trust_or_create_application_session() {
    let cert = rcgen::generate_simple_self_signed(vec!["unidesk.local".into()]).unwrap();
    let der = cert.cert.der().to_vec();
    let key = cert.key_pair.serialize_der();
    let a = Node::new(84, "A", &der, &key).await;
    let mut b = Node::new(85, "B", &der, &key).await;
    a.hint(&b);
    wait(|| !a.runtime.snapshot_view().discovered_peers.is_empty()).await;
    a.runtime
        .send(RuntimeCommand::RequestPairing { peer_id: b.id })
        .await
        .unwrap();
    let request = b.next_pairing().await;
    b.runtime
        .send(RuntimeCommand::RejectPairing {
            request_id: request.request_id(),
            reason: Some("not now".into()),
        })
        .await
        .unwrap();
    wait(|| {
        a.runtime.snapshot_view().device_network.pending.is_empty()
            && b.runtime.snapshot_view().device_network.pending.is_empty()
    })
    .await;
    assert!(a.runtime.snapshot().peers.is_empty());
    assert!(b.runtime.snapshot().peers.is_empty());
    assert!(
        a.transport
            .send_message(
                b.id,
                unidesk_protocol::Message::Ping(unidesk_protocol::Ping { nonce: vec![1] })
            )
            .await
            .is_err()
    );
    assert!(b.runtime.snapshot_view().device_network.owner_id.is_none());
    a.runtime.shutdown_and_wait().await.unwrap();
    b.runtime.shutdown_and_wait().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_quic_expired_invitation_releases_prompt_without_trust() {
    use unidesk_identity::{
        NetworkMember, NetworkOffer, NetworkOperation, NetworkRoster, SignedNetworkOperation,
        SignedNetworkRoster,
    };
    let cert = rcgen::generate_simple_self_signed(vec!["unidesk.local".into()]).unwrap();
    let der = cert.cert.der().to_vec();
    let key = cert.key_pair.serialize_der();
    let a = Node::new(86, "A", &der, &key).await;
    let mut b = Node::new(87, "B", &der, &key).await;
    let identity = LocalIdentity::from_bytes(&[86; 32]).unwrap();
    let member = NetworkMember {
        device_id: a.id,
        public_key: identity.verifying_key_bytes(),
        display_name: "A".into(),
        capabilities: Capabilities::DEVICE_NETWORK,
        endpoint: Endpoint::new("127.0.0.1", a.transport.local_addr().unwrap().port()).unwrap(),
    };
    let roster = SignedNetworkRoster::signed(
        NetworkRoster {
            merge_permits: Vec::new(),
            network_id: unidesk_core::RequestId::new(),
            owner: member.clone(),
            epoch: 1,
            members: vec![member],
        },
        &identity,
    )
    .unwrap();
    let offer = NetworkOffer::signed(roster, b.id, now_unix_ms() + 700, &identity).unwrap();
    let payload = SignedNetworkOperation::signed(NetworkOperation::Offer(offer), &identity)
        .unwrap()
        .encode()
        .unwrap();
    let transport = a.transport.clone();
    let peer = b.id;
    let endpoint = Endpoint::new("127.0.0.1", b.transport.local_addr().unwrap().port()).unwrap();
    let exchange =
        tokio::spawn(async move { transport.network_exchange(peer, endpoint, payload).await });
    let request = b.next_pairing().await;
    wait(|| b.runtime.snapshot_view().device_network.pending.is_empty()).await;
    assert!(exchange.await.unwrap().is_err());
    b.runtime
        .send(RuntimeCommand::ApprovePairing {
            request_id: request.request_id(),
        })
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(b.runtime.snapshot().peers.is_empty());
    a.runtime.shutdown_and_wait().await.unwrap();
    b.runtime.shutdown_and_wait().await.unwrap();
}
#[derive(Clone)]
struct BrokenRosterStore {
    inner: MemoryRuntimeStore,
    broken: Arc<AtomicBool>,
}
impl RuntimeStore for BrokenRosterStore {
    fn get_device_network(&self) -> Result<Option<SignedNetworkRoster>, RuntimeError> {
        if self.broken.load(Ordering::SeqCst) {
            Err(RuntimeError::Store("bad owner signature".into()))
        } else {
            Ok(None)
        }
    }
    fn set_device_network(&self, _roster: Option<SignedNetworkRoster>) -> Result<(), RuntimeError> {
        self.broken.store(false, Ordering::SeqCst);
        Ok(())
    }
    fn get_network_trust_origins(&self) -> Result<Option<NetworkTrustOrigins>, RuntimeError> {
        self.inner.get_network_trust_origins()
    }
    fn set_network_trust_origins(&self, origins: NetworkTrustOrigins) -> Result<(), RuntimeError> {
        self.inner.set_network_trust_origins(origins)
    }
    fn list_peers(&self) -> Result<Vec<PeerRecord>, RuntimeError> {
        self.inner.list_peers()
    }
    fn upsert_peer(&self, peer: PeerRecord) -> Result<(), RuntimeError> {
        self.inner.upsert_peer(peer)
    }
    fn delete_peer(&self, peer: DeviceId) -> Result<bool, RuntimeError> {
        self.inner.delete_peer(peer)
    }
    fn list_manual_endpoints(&self, peer: DeviceId) -> Result<Vec<ManualEndpoint>, RuntimeError> {
        self.inner.list_manual_endpoints(peer)
    }

    fn get_settings(&self) -> Result<Settings, RuntimeError> {
        self.inner.get_settings()
    }
    fn set_public_network_enabled(&self, enabled: bool) -> Result<(), RuntimeError> {
        self.inner.set_public_network_enabled(enabled)
    }
}
#[tokio::test]
async fn corrupt_roster_leave_uses_independently_loaded_origins_and_preserves_legacy_trust() {
    let introduced = LocalIdentity::from_bytes(&[112; 32]).unwrap();
    let legacy = LocalIdentity::from_bytes(&[113; 32]).unwrap();
    let store = BrokenRosterStore {
        inner: MemoryRuntimeStore::default(),
        broken: Arc::new(AtomicBool::new(true)),
    };
    store
        .inner
        .set_network_trust_origins(NetworkTrustOrigins {
            introduced: BTreeSet::from([introduced.device_id()]),
            network_revoked: BTreeSet::new(),
        })
        .unwrap();
    let trust = MemoryTrustStore::new();
    for id in [introduced.device_id(), legacy.device_id()] {
        trust
            .upsert(TrustRecord::new(id, Fingerprint::from(id)))
            .unwrap();
    }
    let discovery = MemoryDiscovery::new();
    for (identity, port) in [(&introduced, 4411), (&legacy, 4412)] {
        let hello = PeerHello::signed(identity, "peer", Capabilities::NETWORK, [1; 32]).unwrap();
        discovery.push(DiscoveryEvent::PeerDiscovered {
            hello: hello.to_wire(),
            routes: vec![RouteCandidate::new(
                Endpoint::new("127.0.0.1", port).unwrap(),
                RouteSource::Manual,
                RouteHealth::Healthy,
                Capabilities::NETWORK,
            )],
        });
    }
    let runtime = RuntimeBuilder::new()
        .with_store(store.clone())
        .with_trust_store(trust.clone())
        .with_discovery(discovery)
        .build()
        .unwrap();
    wait(|| {
        runtime
            .snapshot_view()
            .device_network
            .integrity_error
            .is_some()
            && runtime.snapshot().peers.len() == 2
    })
    .await;
    assert!(
        !runtime
            .snapshot_view()
            .device_network
            .allows(runtime.snapshot().local_device_id, legacy.device_id())
    );
    runtime
        .send(RuntimeCommand::LeaveDeviceNetwork)
        .await
        .unwrap();
    wait(|| {
        runtime
            .snapshot_view()
            .device_network
            .integrity_error
            .is_none()
    })
    .await;
    assert!(!trust.is_trusted(&introduced.device_id()).unwrap());
    assert!(trust.is_trusted(&legacy.device_id()).unwrap());
    assert!(
        runtime
            .snapshot_view()
            .device_network
            .allows(runtime.snapshot().local_device_id, legacy.device_id())
    );
    runtime.shutdown_and_wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn invalid_invitation_deadlines_return_signed_reasons_without_trust() {
    use unidesk_identity::{
        NetworkMember, NetworkOffer, NetworkOperation, NetworkRoster, SignedNetworkOperation,
        SignedNetworkRoster,
    };
    let cert = rcgen::generate_simple_self_signed(vec!["unidesk.local".into()]).unwrap();
    let der = cert.cert.der().to_vec();
    let key = cert.key_pair.serialize_der();
    let a = Node::new(91, "owner", &der, &key).await;
    let mut b = Node::new(92, "recipient", &der, &key).await;
    let identity = LocalIdentity::from_bytes(&[91; 32]).unwrap();
    let member = NetworkMember {
        device_id: a.id,
        public_key: identity.verifying_key_bytes(),
        display_name: "owner".into(),
        capabilities: Capabilities::DEVICE_NETWORK,
        endpoint: Endpoint::new("127.0.0.1", a.transport.local_addr().unwrap().port()).unwrap(),
    };
    let roster = SignedNetworkRoster::signed(
        NetworkRoster {
            merge_permits: Vec::new(),
            network_id: unidesk_core::RequestId::new(),
            owner: member.clone(),
            epoch: 1,
            members: vec![member],
        },
        &identity,
    )
    .unwrap();
    for (expires, expected) in [
        (now_unix_ms() - 1000, "邀请已过期"),
        (now_unix_ms() + 180_000, "系统时间"),
    ] {
        let offer = NetworkOffer::signed(roster.clone(), b.id, expires, &identity).unwrap();
        let payload =
            SignedNetworkOperation::signed(NetworkOperation::Offer(offer.clone()), &identity)
                .unwrap()
                .encode()
                .unwrap();
        let (hello, bytes) = a
            .transport
            .network_exchange(
                b.id,
                Endpoint::new("127.0.0.1", b.transport.local_addr().unwrap().port()).unwrap(),
                payload,
            )
            .await
            .expect(
                "validly signed but expired/skewed offer must return a reason, not connection lost",
            );
        let signed = SignedNetworkOperation::decode_verify(&bytes, &hello.public_key).unwrap();
        let NetworkOperation::Decision(decision) = signed.operation else {
            panic!("expected decision")
        };
        assert!(!decision.accepted);
        assert_eq!(decision.attempt, offer.attempt);
        assert_eq!(decision.target, a.id);
        assert!(decision.reason.unwrap().contains(expected));
        assert!(b.runtime.snapshot_view().device_network.pending.is_empty());
        assert!(b.runtime.snapshot().peers.is_empty());
        assert!(
            b.runtime
                .snapshot_view()
                .device_network
                .network_id
                .is_none()
        );
    }
    // A retry while the original prompt is pending must preserve that prompt
    // and return a signed explanation; a replay after rejection stays denied.
    let offer = NetworkOffer::signed(roster, b.id, now_unix_ms() + 120_000, &identity).unwrap();
    let payload = SignedNetworkOperation::signed(NetworkOperation::Offer(offer.clone()), &identity)
        .unwrap()
        .encode()
        .unwrap();
    let endpoint = Endpoint::new("127.0.0.1", b.transport.local_addr().unwrap().port()).unwrap();
    let transport = a.transport.clone();
    let target = b.id;
    let first_endpoint = endpoint.clone();
    let first_payload = payload.clone();
    let original = tokio::spawn(async move {
        transport
            .network_exchange(target, first_endpoint, first_payload)
            .await
    });
    let pending = b.next_pairing().await;
    for expected in ["待处理的邀请", "该邀请已处理"] {
        let (hello, bytes) = a
            .transport
            .network_exchange(b.id, endpoint.clone(), payload.clone())
            .await
            .expect("retry receives a signed rejection");
        let NetworkOperation::Decision(decision) =
            SignedNetworkOperation::decode_verify(&bytes, &hello.public_key)
                .unwrap()
                .operation
        else {
            panic!("expected decision")
        };
        assert!(!decision.accepted);
        assert_eq!(decision.attempt, offer.attempt);
        assert!(decision.reason.unwrap().contains(expected));
        if expected == "待处理的邀请" {
            assert_eq!(b.runtime.snapshot_view().device_network.pending.len(), 1);
            b.runtime
                .send(RuntimeCommand::reject_pairing(pending.request_id()))
                .await
                .unwrap();
            wait(|| b.runtime.snapshot_view().device_network.pending.is_empty()).await;
        }
        assert!(b.runtime.snapshot().peers.is_empty());
    }
    assert!(original.await.unwrap().is_ok());
    a.runtime.shutdown_and_wait().await.unwrap();
    b.runtime.shutdown_and_wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn recipient_rejection_reason_reaches_inviter_without_trust() {
    let cert = rcgen::generate_simple_self_signed(vec!["unidesk.local".into()]).unwrap();
    let der = cert.cert.der().to_vec();
    let key = cert.key_pair.serialize_der();
    let mut a = Node::new(93, "owner", &der, &key).await;
    let mut b = Node::new(94, "recipient", &der, &key).await;
    a.hint(&b);
    wait(|| !a.runtime.snapshot_view().discovered_peers.is_empty()).await;
    a.runtime
        .send(RuntimeCommand::RequestPairing { peer_id: b.id })
        .await
        .unwrap();
    let request = b.next_pairing().await;
    b.runtime
        .send(RuntimeCommand::reject_pairing_with_reason(
            request.request_id(),
            "请先处理原邀请，稍后重试",
        ))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let RuntimeEvent::Error(error) = a.events.recv().await.unwrap() {
                assert!(
                    error.to_string().contains("请先处理原邀请，稍后重试"),
                    "{error}"
                );
                break;
            }
        }
    })
    .await
    .expect("inviter receives signed rejection reason");
    assert!(a.runtime.snapshot_view().device_network.pending.is_empty());
    assert!(b.runtime.snapshot_view().device_network.pending.is_empty());
    assert!(a.runtime.snapshot().peers.is_empty());
    assert!(b.runtime.snapshot().peers.is_empty());
    a.runtime.shutdown_and_wait().await.unwrap();
    b.runtime.shutdown_and_wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn revoked_recipient_requires_local_repair_permission_then_fresh_approval() {
    use unidesk_identity::MemoryTrustStore;
    let cert = rcgen::generate_simple_self_signed(vec!["unidesk.local".into()]).unwrap();
    let der = cert.cert.der().to_vec();
    let key = cert.key_pair.serialize_der();
    let a = Node::new(95, "owner", &der, &key).await;
    let trust = MemoryTrustStore::new();
    trust
        .upsert(TrustRecord::revoked(a.id, Fingerprint::from(a.id)))
        .unwrap();
    let mut b = Node::with_trust(96, "recipient", &der, &key, trust).await;
    wait(|| {
        b.runtime
            .snapshot_view()
            .device_network
            .blocked_peer_ids
            .contains(&a.id)
    })
    .await;
    a.hint(&b);
    b.hint(&a);
    wait(|| !a.runtime.snapshot_view().discovered_peers.is_empty()).await;
    a.runtime
        .send(RuntimeCommand::RequestPairing { peer_id: b.id })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let RuntimeEvent::Error(e) = b.events.recv().await.unwrap() {
                assert!(e.to_string().contains("允许重新配对"), "{e}");
                break;
            }
        }
    })
    .await
    .unwrap();
    wait(|| a.runtime.snapshot_view().device_network.pending.is_empty()).await;
    assert!(b.runtime.snapshot_view().device_network.pending.is_empty());
    assert!(b.trust.get(&a.id).unwrap().unwrap().revoked);
    b.runtime
        .send(RuntimeCommand::AllowPairing { peer_id: a.id })
        .await
        .unwrap();
    wait(|| {
        b.runtime
            .snapshot_view()
            .device_network
            .pairing_recovery_peer_ids
            .contains(&a.id)
    })
    .await;
    assert!(
        b.runtime
            .snapshot_view()
            .device_network
            .network_id
            .is_none(),
        "allowing an invitation must not create a competing network"
    );
    assert!(
        b.runtime
            .snapshot_view()
            .device_network
            .blocked_peer_ids
            .contains(&a.id)
    );
    assert!(b.trust.get(&a.id).unwrap().unwrap().revoked);
    a.runtime
        .send(RuntimeCommand::RequestPairing { peer_id: b.id })
        .await
        .unwrap();
    let request = b.next_pairing().await;
    assert!(
        b.trust.get(&a.id).unwrap().unwrap().revoked,
        "fresh prompt is not approval"
    );
    assert!(!connected(&b, 1));
    b.runtime
        .send(RuntimeCommand::reject_pairing(request.request_id()))
        .await
        .unwrap();
    wait(|| a.runtime.snapshot_view().device_network.pending.is_empty()).await;
    assert!(b.trust.get(&a.id).unwrap().unwrap().revoked);
    assert!(
        b.runtime
            .snapshot_view()
            .device_network
            .pairing_recovery_peer_ids
            .is_empty()
    );
    b.runtime
        .send(RuntimeCommand::AllowPairing { peer_id: a.id })
        .await
        .unwrap();
    wait(|| {
        b.runtime
            .snapshot_view()
            .device_network
            .pairing_recovery_peer_ids
            .contains(&a.id)
    })
    .await;
    a.runtime
        .send(RuntimeCommand::RequestPairing { peer_id: b.id })
        .await
        .unwrap();
    let request = b.next_pairing().await;
    b.runtime
        .send(RuntimeCommand::approve_pairing(request.request_id()))
        .await
        .unwrap();
    wait(|| connected(&a, 1) && connected(&b, 1)).await;
    assert!(!b.trust.get(&a.id).unwrap().unwrap().revoked);
    assert!(
        !b.runtime
            .snapshot_view()
            .device_network
            .blocked_peer_ids
            .contains(&a.id)
    );
    assert!(
        b.runtime
            .snapshot_view()
            .device_network
            .pairing_recovery_peer_ids
            .is_empty()
    );
    a.runtime.shutdown_and_wait().await.unwrap();
    b.runtime.shutdown_and_wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn repair_permission_does_not_survive_restart_or_revoke() {
    use unidesk_identity::MemoryTrustStore;
    let cert = rcgen::generate_simple_self_signed(vec!["unidesk.local".into()]).unwrap();
    let der = cert.cert.der().to_vec();
    let key = cert.key_pair.serialize_der();
    let remote = LocalIdentity::from_bytes(&[97; 32]).unwrap().device_id();
    let trust = MemoryTrustStore::new();
    trust
        .upsert(TrustRecord::revoked(remote, Fingerprint::from(remote)))
        .unwrap();
    let b = Node::with_trust(98, "recipient", &der, &key, trust.clone()).await;
    wait(|| {
        b.runtime
            .snapshot_view()
            .device_network
            .blocked_peer_ids
            .contains(&remote)
    })
    .await;
    b.runtime
        .send(RuntimeCommand::AllowPairing { peer_id: remote })
        .await
        .unwrap();
    wait(|| {
        b.runtime
            .snapshot_view()
            .device_network
            .pairing_recovery_peer_ids
            .contains(&remote)
    })
    .await;
    b.runtime
        .send(RuntimeCommand::RevokePeer { peer_id: remote })
        .await
        .unwrap();
    wait(|| {
        b.runtime
            .snapshot_view()
            .device_network
            .pairing_recovery_peer_ids
            .is_empty()
    })
    .await;
    assert!(trust.get(&remote).unwrap().unwrap().revoked);
    b.runtime
        .send(RuntimeCommand::AllowPairing { peer_id: remote })
        .await
        .unwrap();
    wait(|| {
        b.runtime
            .snapshot_view()
            .device_network
            .pairing_recovery_peer_ids
            .contains(&remote)
    })
    .await;
    b.runtime.shutdown_and_wait().await.unwrap();
    let b = Node::with_trust(98, "recipient", &der, &key, trust).await;
    wait(|| {
        b.runtime
            .snapshot_view()
            .device_network
            .blocked_peer_ids
            .contains(&remote)
    })
    .await;
    assert!(
        b.runtime
            .snapshot_view()
            .device_network
            .pairing_recovery_peer_ids
            .is_empty()
    );
    assert!(b.trust.get(&remote).unwrap().unwrap().revoked);
    b.runtime.shutdown_and_wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn third_device_can_request_existing_network_through_owner_or_member() {
    let cert = rcgen::generate_simple_self_signed(vec!["unidesk.local".into()]).unwrap();
    let der = cert.cert.der().to_vec();
    let key = cert.key_pair.serialize_der();
    let mut a = Node::new(101, "owner A", &der, &key).await;
    let mut b = Node::new(102, "member B", &der, &key).await;
    let c = Node::new(103, "joining C", &der, &key).await;
    invite(&a, &mut b).await;
    wait(|| connected(&a, 1) && connected(&b, 1)).await;
    let network_id = a.runtime.snapshot_view().device_network.network_id.clone();

    // First request the owner directly, then decline: existing membership and
    // application trust must remain unchanged on all three nodes.
    c.hint(&a);
    wait(|| !c.runtime.snapshot_view().discovered_peers.is_empty()).await;
    c.runtime
        .send(RuntimeCommand::RequestPairing { peer_id: a.id })
        .await
        .unwrap();
    let request = a.next_pairing().await;
    assert_eq!(request.device_id(), c.id);
    assert!(
        c.runtime
            .snapshot()
            .peers
            .iter()
            .all(|p| p.trust != TrustState::Trusted)
    );
    a.runtime
        .send(RuntimeCommand::reject_pairing(request.request_id()))
        .await
        .unwrap();
    wait(|| c.runtime.snapshot_view().device_network.pending.is_empty()).await;
    assert_eq!(
        a.runtime.snapshot_view().device_network.network_id,
        network_id
    );
    assert_eq!(a.runtime.snapshot_view().device_network.member_ids.len(), 2);
    assert!(
        c.runtime
            .snapshot()
            .peers
            .iter()
            .all(|p| p.trust != TrustState::Trusted)
    );

    // Retry through an ordinary member, with the solo roster left by rejection.
    // Only the signed owner may approve; B must not admit C or replace its network.
    tokio::time::sleep(Duration::from_millis(5100)).await;
    c.hint(&b);
    wait(|| {
        c.runtime
            .snapshot_view()
            .discovered_peers
            .iter()
            .any(|p| p.peer_id == b.id)
    })
    .await;
    c.runtime
        .send(RuntimeCommand::RequestPairing { peer_id: b.id })
        .await
        .unwrap();
    let request = a.next_pairing().await;
    assert_eq!(request.device_id(), c.id);
    assert!(b.runtime.snapshot_view().device_network.pending.is_empty());
    assert_eq!(a.runtime.snapshot_view().device_network.member_ids.len(), 2);
    a.runtime
        .send(RuntimeCommand::ApprovePairing {
            request_id: request.request_id(),
        })
        .await
        .unwrap();
    wait(|| connected(&a, 2) && connected(&b, 2) && connected(&c, 2)).await;
    for node in [&a, &b, &c] {
        let view = node.runtime.snapshot_view();
        assert_eq!(view.device_network.network_id, network_id);
        assert_eq!(view.device_network.owner_id, Some(a.id));
        assert_eq!(view.device_network.member_ids.len(), 3);
        assert!(view.device_network.pending.is_empty());
    }
    let d = Node::new(107, "joining D", &der, &key).await;
    d.hint(&a);
    wait(|| !d.runtime.snapshot_view().discovered_peers.is_empty()).await;
    d.runtime
        .send(RuntimeCommand::RequestPairing { peer_id: a.id })
        .await
        .unwrap();
    let request = a.next_pairing().await;
    assert_eq!(request.device_id(), d.id);
    a.runtime
        .send(RuntimeCommand::ApprovePairing {
            request_id: request.request_id(),
        })
        .await
        .unwrap();
    wait(|| connected(&a, 3) && connected(&b, 3) && connected(&c, 3) && connected(&d, 3)).await;
    assert_eq!(
        d.runtime.snapshot_view().device_network.network_id,
        network_id
    );
    d.runtime.shutdown_and_wait().await.unwrap();
    for node in [&a, &b, &c] {
        node.runtime.shutdown_and_wait().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn failed_solo_invitation_does_not_prevent_owner_inviting_third_device() {
    let cert = rcgen::generate_simple_self_signed(vec!["unidesk.local".into()]).unwrap();
    let der = cert.cert.der().to_vec();
    let key = cert.key_pair.serialize_der();
    let mut a = Node::new(104, "owner A", &der, &key).await;
    let mut b = Node::new(105, "member B", &der, &key).await;
    let mut c = Node::new(106, "joining C", &der, &key).await;
    invite(&a, &mut b).await;
    wait(|| connected(&a, 1) && connected(&b, 1)).await;
    c.hint(&a);
    wait(|| !c.runtime.snapshot_view().discovered_peers.is_empty()).await;
    c.runtime
        .send(RuntimeCommand::RequestPairing { peer_id: a.id })
        .await
        .unwrap();
    let request = a.next_pairing().await;
    a.runtime
        .send(RuntimeCommand::reject_pairing(request.request_id()))
        .await
        .unwrap();
    wait(|| c.runtime.snapshot_view().device_network.pending.is_empty()).await;
    invite(&a, &mut c).await;
    wait(|| connected(&a, 2) && connected(&b, 2) && connected(&c, 2)).await;
    assert_eq!(
        c.runtime.snapshot_view().device_network.network_id,
        a.runtime.snapshot_view().device_network.network_id
    );
    for node in [&a, &b, &c] {
        node.runtime.shutdown_and_wait().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn member_invites_new_device_and_receives_final_result() {
    let cert = rcgen::generate_simple_self_signed(vec!["unidesk.local".into()]).unwrap();
    let der = cert.cert.der().to_vec();
    let key = cert.key_pair.serialize_der();
    let a = Node::new(141, "coordinator A", &der, &key).await;
    let mut b = Node::new(142, "initiator B", &der, &key).await;
    let mut c = Node::new(143, "recipient C", &der, &key).await;
    invite(&a, &mut b).await;
    wait(|| connected(&a, 1) && connected(&b, 1)).await;
    b.hint(&c);
    wait(|| !b.runtime.snapshot_view().discovered_peers.is_empty()).await;
    b.runtime
        .send(RuntimeCommand::RequestPairing { peer_id: c.id })
        .await
        .unwrap();
    let request = c.next_pairing().await;
    assert_eq!(request.device_id(), a.id);
    assert!(
        c.runtime
            .snapshot_view()
            .device_network
            .member_ids
            .is_empty()
    );
    let pending = b.runtime.snapshot_view().device_network.pending[0].clone();
    assert_eq!(pending.peer_id, c.id);
    assert!(!pending.incoming);
    c.runtime
        .send(RuntimeCommand::ApprovePairing {
            request_id: request.request_id(),
        })
        .await
        .unwrap();
    wait(|| {
        connected(&a, 2)
            && connected(&b, 2)
            && connected(&c, 2)
            && b.runtime.snapshot_view().device_network.pending.is_empty()
    })
    .await;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let RuntimeEvent::PairingFinished {
                request_id,
                outcome,
                ..
            } = b.events.recv().await.unwrap()
                && request_id == pending.request_id
            {
                assert_eq!(outcome, "accepted");
                break;
            }
        }
    })
    .await
    .unwrap();
    for node in [&a, &b, &c] {
        node.runtime.shutdown_and_wait().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn outsider_cannot_delegate_an_invitation_to_network_owner() {
    use unidesk_identity::{NetworkMember, NetworkOperation, SignedNetworkOperation};
    let cert = rcgen::generate_simple_self_signed(vec!["unidesk.local".into()]).unwrap();
    let der = cert.cert.der().to_vec();
    let key = cert.key_pair.serialize_der();
    let a = Node::new(144, "owner", &der, &key).await;
    let mut b = Node::new(145, "member", &der, &key).await;
    let outsider = Node::new(146, "outsider", &der, &key).await;
    let target = Node::new(147, "target", &der, &key).await;
    invite(&a, &mut b).await;
    wait(|| connected(&a, 1) && connected(&b, 1)).await;
    let roster = a.store.get_device_network().unwrap().unwrap();
    let identity = LocalIdentity::from_bytes(&[146; 32]).unwrap();
    let payload = SignedNetworkOperation::signed(
        NetworkOperation::InviteViaOwner {
            attempt: unidesk_core::RequestId::new(),
            network_id: roster.roster.network_id,
            target: NetworkMember {
                device_id: target.id,
                public_key: LocalIdentity::from_bytes(&[147; 32])
                    .unwrap()
                    .verifying_key_bytes(),
                display_name: "target".into(),
                capabilities: Capabilities::NETWORK | Capabilities::DEVICE_NETWORK,
                endpoint: Endpoint::new("127.0.0.1", target.transport.local_addr().unwrap().port())
                    .unwrap(),
            },
            expires_at_unix_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64
                + 120_000,
        },
        &identity,
    )
    .unwrap()
    .encode()
    .unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        outsider.transport.network_exchange(
            a.id,
            Endpoint::new("127.0.0.1", a.transport.local_addr().unwrap().port()).unwrap(),
            payload,
        ),
    )
    .await
    .unwrap();
    assert!(result.is_err());
    assert!(a.runtime.snapshot_view().device_network.pending.is_empty());
    assert!(
        target
            .runtime
            .snapshot_view()
            .device_network
            .pending
            .is_empty()
    );
    assert_eq!(a.runtime.snapshot_view().device_network.member_ids.len(), 2);
    for node in [&a, &b, &outsider, &target] {
        node.runtime.shutdown_and_wait().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn removed_device_remains_searchable_without_another_advertisement() {
    let cert = rcgen::generate_simple_self_signed(vec!["unidesk.local".into()]).unwrap();
    let der = cert.cert.der().to_vec();
    let key = cert.key_pair.serialize_der();
    let a = Node::new(148, "owner", &der, &key).await;
    let mut b = Node::new(149, "member", &der, &key).await;
    let mut c = Node::new(150, "nearby", &der, &key).await;
    invite(&a, &mut b).await;
    invite(&a, &mut c).await;
    wait(|| connected(&a, 2) && connected(&b, 2) && connected(&c, 2)).await;
    b.hint(&c);
    wait(|| {
        b.runtime
            .snapshot_view()
            .discovered_peers
            .iter()
            .any(|h| h.peer_id == c.id)
    })
    .await;
    b.runtime
        .send(RuntimeCommand::ForgetPeer { peer_id: c.id })
        .await
        .unwrap();
    wait(|| !b.runtime.snapshot().peers.iter().any(|p| p.id == c.id)).await;
    b.runtime.send(RuntimeCommand::RefreshPeers).await.unwrap();
    wait(|| {
        b.runtime
            .snapshot_view()
            .discovered_peers
            .iter()
            .any(|h| h.peer_id == c.id)
    })
    .await;
    assert!(b.trust.get(&c.id).unwrap().unwrap().revoked);
    assert!(
        b.runtime
            .snapshot_view()
            .device_network
            .blocked_peer_ids
            .contains(&c.id)
    );
    // No c advertisement is injected here. The retained hint starts the real
    // signed coordination flow and restores this member's local trust.
    b.runtime
        .send(RuntimeCommand::RequestPairing { peer_id: c.id })
        .await
        .unwrap();
    wait(|| connected(&b, 2) && b.runtime.snapshot_view().device_network.pending.is_empty()).await;
    assert!(!b.trust.get(&c.id).unwrap().unwrap().revoked);

    // Owner removal changes the actual roster, so joining again must wait
    // for a new recipient approval as well as rediscovery on the owner.
    a.runtime
        .send(RuntimeCommand::ForgetPeer { peer_id: c.id })
        .await
        .unwrap();
    wait(|| {
        c.runtime
            .snapshot_view()
            .device_network
            .network_id
            .is_none()
    })
    .await;
    a.runtime.send(RuntimeCommand::RefreshPeers).await.unwrap();
    wait(|| {
        a.runtime
            .snapshot_view()
            .discovered_peers
            .iter()
            .any(|h| h.peer_id == c.id)
    })
    .await;
    a.runtime
        .send(RuntimeCommand::RequestPairing { peer_id: c.id })
        .await
        .unwrap();
    let request = c.next_pairing().await;
    assert!(
        !a.runtime
            .snapshot_view()
            .device_network
            .member_ids
            .contains(&c.id)
    );
    c.runtime
        .send(RuntimeCommand::ApprovePairing {
            request_id: request.request_id(),
        })
        .await
        .unwrap();
    wait(|| connected(&a, 2) && connected(&b, 2) && connected(&c, 2)).await;
    for node in [&a, &b, &c] {
        node.runtime.shutdown_and_wait().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn member_initiates_network_merge_through_both_owners() {
    let cert = rcgen::generate_simple_self_signed(vec!["unidesk.local".into()]).unwrap();
    let der = cert.cert.der().to_vec();
    let key = cert.key_pair.serialize_der();
    let mut a = Node::new(108, "owner A", &der, &key).await;
    let mut b = Node::new(109, "member B", &der, &key).await;
    let c = Node::new(110, "owner C", &der, &key).await;
    let mut d = Node::new(111, "member D", &der, &key).await;
    invite(&a, &mut b).await;
    invite(&c, &mut d).await;
    wait(|| connected(&a, 1) && connected(&b, 1) && connected(&c, 1) && connected(&d, 1)).await;
    let network_id = a.runtime.snapshot_view().device_network.network_id.clone();
    d.hint(&b);
    wait(|| {
        d.runtime
            .snapshot_view()
            .discovered_peers
            .iter()
            .any(|p| p.peer_id == b.id)
    })
    .await;
    d.runtime
        .send(RuntimeCommand::RequestPairing { peer_id: b.id })
        .await
        .unwrap();
    let request = a.next_pairing().await;
    assert_eq!(request.device_id(), c.id);
    assert_eq!(
        a.runtime.snapshot_view().device_network.pending[0]
            .member_names
            .len(),
        4
    );
    assert_eq!(a.runtime.snapshot_view().device_network.member_ids.len(), 2);
    assert_eq!(c.runtime.snapshot_view().device_network.member_ids.len(), 2);
    let original_c = c.runtime.snapshot_view().device_network.network_id.clone();
    a.runtime
        .send(RuntimeCommand::reject_pairing(request.request_id()))
        .await
        .unwrap();
    wait(|| {
        c.runtime.snapshot_view().device_network.pending.is_empty()
            && d.runtime.snapshot_view().device_network.pending.is_empty()
    })
    .await;
    assert_eq!(
        c.runtime.snapshot_view().device_network.network_id,
        original_c
    );
    assert_eq!(
        a.runtime.snapshot_view().device_network.network_id,
        network_id
    );
    assert!(connected(&a, 1) && connected(&c, 1));
    tokio::time::sleep(Duration::from_millis(5100)).await;
    d.runtime
        .send(RuntimeCommand::RequestPairing { peer_id: b.id })
        .await
        .unwrap();
    let request = a.next_pairing().await;
    a.runtime
        .send(RuntimeCommand::ApprovePairing {
            request_id: request.request_id(),
        })
        .await
        .unwrap();
    wait(|| connected(&a, 3) && connected(&b, 3) && connected(&c, 3) && connected(&d, 3)).await;
    for node in [&a, &b, &c, &d] {
        assert_eq!(
            node.runtime.snapshot_view().device_network.network_id,
            network_id
        );
        assert_eq!(
            node.runtime.snapshot_view().device_network.owner_id,
            Some(a.id)
        );
        assert_eq!(
            node.runtime.snapshot_view().device_network.member_ids.len(),
            4
        );
        assert!(
            node.runtime
                .snapshot_view()
                .device_network
                .pending
                .is_empty()
        );
    }
    // Traffic between the formerly separate non-owner members proves the mesh
    // actually migrated, rather than merely displaying a combined member count.
    b.runtime
        .send(RuntimeCommand::SendClipboard {
            peer_id: d.id,
            payload: ClipboardPayload::new("text/plain", b"merged network".to_vec(), 1).unwrap(),
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let RuntimeEvent::ClipboardReceived { peer_id, .. } = d.events.recv().await.unwrap()
                && peer_id == b.id
            {
                break;
            }
        }
    })
    .await
    .unwrap();
    for node in [&a, &b, &c, &d] {
        node.runtime.shutdown_and_wait().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn offline_member_migrates_after_merge_and_owner_restart() {
    let cert = rcgen::generate_simple_self_signed(vec!["unidesk.local".into()]).unwrap();
    let der = cert.cert.der().to_vec();
    let key = cert.key_pair.serialize_der();
    let mut a = Node::new(112, "owner A", &der, &key).await;
    let mut b = Node::new(113, "member B", &der, &key).await;
    let c = Node::new(114, "owner C", &der, &key).await;
    let mut d = Node::new(115, "offline D", &der, &key).await;
    invite(&a, &mut b).await;
    invite(&c, &mut d).await;
    wait(|| connected(&a, 1) && connected(&b, 1) && connected(&c, 1) && connected(&d, 1)).await;
    let d_state = (
        d.trust.clone(),
        d.store.clone(),
        d.transport.local_addr().unwrap(),
    );
    d.runtime.shutdown_and_wait().await.unwrap();
    drop(d);
    c.hint(&a);
    wait(|| {
        c.runtime
            .snapshot_view()
            .discovered_peers
            .iter()
            .any(|p| p.peer_id == a.id)
    })
    .await;
    c.runtime
        .send(RuntimeCommand::RequestPairing { peer_id: a.id })
        .await
        .unwrap();
    let request = a.next_pairing().await;
    a.runtime
        .send(RuntimeCommand::ApprovePairing {
            request_id: request.request_id(),
        })
        .await
        .unwrap();
    wait(|| connected(&a, 2) && connected(&b, 2) && connected(&c, 2)).await;
    let network_id = a.runtime.snapshot_view().device_network.network_id.clone();
    let a_state = (
        a.trust.clone(),
        a.store.clone(),
        a.transport.local_addr().unwrap(),
    );
    a.runtime.shutdown_and_wait().await.unwrap();
    drop(a);
    wait(|| std::net::UdpSocket::bind(a_state.2).is_ok()).await;
    wait(|| std::net::UdpSocket::bind(d_state.2).is_ok()).await;
    let a = Node::with_state(112, "owner A", &der, &key, a_state.0, a_state.1, a_state.2).await;
    let d = Node::with_state(
        115,
        "offline D",
        &der,
        &key,
        d_state.0,
        d_state.1,
        d_state.2,
    )
    .await;
    wait(|| connected(&a, 3) && connected(&b, 3) && connected(&c, 3) && connected(&d, 3)).await;
    assert_eq!(
        d.runtime.snapshot_view().device_network.network_id,
        network_id
    );
    assert_eq!(
        d.runtime.snapshot_view().device_network.owner_id,
        Some(a.id)
    );
    for node in [&a, &b, &c, &d] {
        node.runtime.shutdown_and_wait().await.unwrap();
    }
}
