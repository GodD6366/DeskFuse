//! Recipient approval and a durable, owner-signed roster. This actor alone changes membership.
use super::*;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::oneshot;
use unidesk_core::RequestId;
use unidesk_identity::{
    NetworkDecision, NetworkMember, NetworkOffer, NetworkOperation, NetworkRoster,
    SignedNetworkOperation, network_offer_hash,
};
use unidesk_transport::{NetworkTransportRequest, PeerHello};
mod member_invitation;
use member_invitation::RelayedInvitation;
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
fn invalid(error: impl ToString) -> RuntimeError {
    RuntimeError::Protocol(error.to_string())
}
struct Incoming {
    offer: NetworkOffer,
    remote: PeerHello,
    reply: oneshot::Sender<Vec<u8>>,
}
enum PairingOutcome {
    Accepted,
    Rejected(Option<String>),
    Redirect(NetworkMember),
}
struct Outgoing {
    offer: NetworkOffer,
    peer: DeviceId,
    endpoint: Endpoint,
}
pub(super) enum NetworkResult {
    ManualProbe {
        attempt: RequestId,
        endpoint: Endpoint,
        result: Result<Hello, RuntimeError>,
    },
    MemberInvitation {
        attempt: RequestId,
        owner: NetworkMember,
        target: DeviceId,
        result: Result<(PeerHello, Vec<u8>), RuntimeError>,
    },
    Offer {
        attempt: RequestId,
        result: Result<(PeerHello, Vec<u8>), RuntimeError>,
    },
    Exchange {
        peer: DeviceId,
        result: Result<(PeerHello, Vec<u8>), RuntimeError>,
    },
}
#[derive(Default)]
pub(super) struct DeviceNetworkState {
    pub(super) route_preferences: BTreeMap<DeviceId, unidesk_core::RoutePreference>,
    roster: Option<SignedNetworkRoster>,
    origins: NetworkTrustOrigins,
    origins_loaded: bool,
    origins_error: Option<String>,
    integrity_error: Option<String>,
    incoming: BTreeMap<RequestId, Incoming>,
    outgoing: BTreeMap<RequestId, Outgoing>,
    relayed: BTreeMap<RequestId, RelayedInvitation>,
    pending: BTreeMap<RequestId, NetworkPairingPending>,
    seen: BTreeMap<RequestId, u64>,
    incoming_rate: BTreeMap<DeviceId, u64>,
    pairing_recovery: BTreeMap<DeviceId, tokio::time::Instant>,
    inflight: BTreeSet<DeviceId>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    last_sync: u64,
    manual_probe: Option<RequestId>,
}
fn is_solo_network(roster: &SignedNetworkRoster, device: DeviceId) -> bool {
    roster.roster.owner.device_id == device
        && roster.roster.members.len() == 1
        && roster.roster.members[0].device_id == device
}

impl DeviceNetworkState {
    fn can_receive_repair_invitation(&self, peer: DeviceId) -> bool {
        self.pairing_recovery
            .get(&peer)
            .is_some_and(|deadline| *deadline > tokio::time::Instant::now())
    }
    pub fn view(&self, blocked: &BTreeSet<DeviceId>) -> DeviceNetworkSnapshot {
        DeviceNetworkSnapshot {
            route_preferences: self.route_preferences.clone(),
            integrity_error: self.integrity_error.clone(),
            network_id: self
                .roster
                .as_ref()
                .map(|r| r.roster.network_id.to_string()),
            owner_id: self.roster.as_ref().map(|r| r.roster.owner.device_id),
            epoch: self.roster.as_ref().map_or(0, |r| r.roster.epoch),
            member_ids: self
                .roster
                .as_ref()
                .map(|r| {
                    r.roster
                        .members
                        .iter()
                        .map(|m| m.device_id)
                        .filter(|id| !blocked.contains(id))
                        .collect()
                })
                .unwrap_or_default(),
            pending: self.pending.values().cloned().collect(),
            blocked_peer_ids: blocked.iter().copied().collect(),
            pairing_recovery_peer_ids: self
                .pairing_recovery
                .iter()
                .filter(|(_, deadline)| **deadline > tokio::time::Instant::now())
                .map(|(peer, _)| *peer)
                .collect(),
        }
    }
    pub fn stop(&mut self) {
        self.manual_probe = None;
        for task in self.tasks.drain(..) {
            task.abort();
        }
        self.incoming.clear();
        self.outgoing.clear();
        self.relayed.clear();
        self.pairing_recovery.clear();
    }
}
impl Actor {
    fn can_receive_repair_invitation(&self, peer: DeviceId) -> bool {
        self.device_network.can_receive_repair_invitation(peer)
    }

    pub(super) async fn allow_network_pairing(
        &mut self,
        peer: DeviceId,
    ) -> Result<(), RuntimeError> {
        if !self.transport.supports_network_pairing() || !self.blocked.contains(&peer) {
            return Err(invalid("该设备不需要解除配对阻止"));
        }
        if self.device_network.integrity_error.is_some() {
            return Err(invalid("设备网络记录无法验证，请先修复网络记录"));
        }
        // Keep the durable tombstone and actor block. Only the signed Offer
        // path may use this local, bounded permission. No application identity
        // may remain registered while waiting for the user's second approval.
        self.transport.revoke_peer(peer).await?;
        self.registered_identities.remove(&peer);
        self.transport.restore_peer(peer).await?;
        self.device_network.incoming_rate.remove(&peer);
        self.device_network.pairing_recovery.insert(
            peer,
            tokio::time::Instant::now() + std::time::Duration::from_secs(120),
        );
        self.publish_view();
        Ok(())
    }

    fn sign_network(&self, operation: NetworkOperation) -> Result<Vec<u8>, RuntimeError> {
        SignedNetworkOperation::signed(operation, &self.identity)
            .and_then(|o| o.encode())
            .map_err(invalid)
    }
    async fn network_self(&self, remote: Endpoint) -> Result<NetworkMember, RuntimeError> {
        Ok(NetworkMember {
            device_id: self.identity.device_id(),
            public_key: self.identity.verifying_key_bytes(),
            display_name: self.config.display_name.clone(),
            capabilities: self.config.capabilities,
            endpoint: self.transport.network_endpoint_for(remote).await?,
        })
    }
    async fn persist_network(&mut self, roster: SignedNetworkRoster) -> Result<(), RuntimeError> {
        roster.verify().map_err(invalid)?;
        if !self.device_network.origins_loaded {
            self.persist_network_origins(self.device_network.origins.clone())
                .await?;
        }
        let store = self.persistence.clone();
        let saved = roster.clone();
        task::spawn_blocking(move || store.set_device_network(Some(saved)))
            .await
            .map_err(invalid)??;
        self.device_network.roster = Some(roster);
        self.publish_view();
        Ok(())
    }
    async fn persist_network_origins(
        &mut self,
        origins: NetworkTrustOrigins,
    ) -> Result<(), RuntimeError> {
        let store = self.persistence.clone();
        let saved = origins.clone();
        task::spawn_blocking(move || store.set_network_trust_origins(saved))
            .await
            .map_err(invalid)??;
        self.device_network.origins = origins;
        self.device_network.origins_loaded = true;
        Ok(())
    }
    pub(super) async fn load_device_network(&mut self) {
        let store = self.persistence.clone();
        match task::spawn_blocking(move || store.get_route_preferences()).await {
            Ok(Ok(preferences)) => self.device_network.route_preferences = preferences,
            Ok(Err(error)) => self.emit_error(error),
            Err(error) => self.emit_error(RuntimeError::Store(error.to_string())),
        }
        let store = self.persistence.clone();
        match task::spawn_blocking(move || store.reconnect_suspended()).await {
            Ok(Ok(suspended)) => self.restore_reconnect_suspended(suspended),
            Ok(Err(error)) => self.emit_error(error),
            Err(error) => self.emit_error(RuntimeError::Store(error.to_string())),
        }

        let store = self.persistence.clone();
        match task::spawn_blocking(move || {
            (
                store.get_device_network(),
                store.get_network_trust_origins(),
            )
        })
        .await
        {
            Ok((roster, origins)) => {
                match roster {
                    Ok(roster) => self.device_network.roster = roster,
                    Err(e) => {
                        self.device_network.integrity_error = Some(e.to_string());
                        self.emit_error(e);
                    }
                }
                match origins {
                    Ok(Some(origins)) => {
                        self.device_network.origins = origins;
                        self.device_network.origins_loaded = true;
                    }
                    Ok(None) => {
                        if self.device_network.roster.is_some() {
                            // Missing development-era provenance must preserve all pre-existing trust records.
                            if let Err(e) = self
                                .persist_network_origins(NetworkTrustOrigins::default())
                                .await
                            {
                                self.device_network.integrity_error = Some(e.to_string());
                                self.emit_error(e);
                            }
                        }
                    }
                    Err(e) => {
                        self.device_network.origins_error = Some(e.to_string());
                        self.device_network.integrity_error = Some(e.to_string());
                        self.emit_error(e);
                    }
                }
                for peer in &self.device_network.origins.network_revoked {
                    self.blocked.remove(peer);
                    if let Some(state) = self.peers.get_mut(peer) {
                        state.trust = TrustState::Unknown;
                    }
                }
            }
            Err(e) => {
                self.device_network.integrity_error = Some(e.to_string());
                self.emit_error(invalid(e));
            }
        }
    }
    fn finish_network_pairing(&mut self, attempt: RequestId, peer: DeviceId, outcome: &str) {
        if let Some(relayed) = self.device_network.relayed.remove(&attempt) {
            self.reply_to_member(relayed, outcome);
        }
        self.device_network.pairing_recovery.remove(&peer);
        self.device_network.pending.remove(&attempt);
        self.device_network.incoming.remove(&attempt);
        self.device_network.outgoing.remove(&attempt);
        self.device_network
            .seen
            .insert(attempt, now_ms().saturating_add(300_000));
        let _ = self.event_tx.send(RuntimeEvent::PairingFinished {
            peer_id: peer,
            request_id: attempt,
            outcome: outcome.into(),
        });
        self.publish_view();
    }
    pub(super) fn request_pairing_at(&mut self, endpoint: Endpoint) {
        let status = |message: String| {
            let _ = self.event_tx.send(RuntimeEvent::PairingStatus { message });
        };
        let valid = endpoint.host.parse::<std::net::Ipv4Addr>().is_ok_and(|ip| {
            !ip.is_unspecified() && !ip.is_loopback() && !ip.is_multicast() && !ip.is_broadcast()
        }) && endpoint.port != 0;
        if !valid {
            status("请输入另一台设备的有效 IPv4 地址".into());
            return;
        }
        if self.device_network.manual_probe.is_some() {
            status("正在连接输入的 IP，请稍候".into());
            return;
        }
        let attempt = RequestId::new();
        self.device_network.manual_probe = Some(attempt);
        status("正在连接并验证对方身份…".into());
        let transport = self.transport.clone();
        let tx = self.network_result_tx.clone();
        self.device_network.tasks.push(task::spawn(async move {
            let result = tokio::time::timeout(
                Duration::from_secs(8),
                transport.probe_endpoint(endpoint.clone()),
            )
            .await
            .unwrap_or_else(|_| Err(invalid("连接超时，请检查 IP、对方应用及局域网连接")));
            let _ = tx
                .send(NetworkResult::ManualProbe {
                    attempt,
                    endpoint,
                    result,
                })
                .await;
        }));
    }

    async fn complete_pairing_at(
        &mut self,
        endpoint: Endpoint,
        remote: Hello,
    ) -> Result<(), RuntimeError> {
        verify_hello(&remote).map_err(invalid)?;
        let peer = core_device_id_from_bytes(&remote.device_id);
        if peer == self.identity.device_id() {
            return Err(invalid("该 IP 指向本机，请输入另一台设备的地址"));
        }
        let capabilities = Capabilities::from_bits(remote.capabilities);
        if !capabilities.contains(Capabilities::DEVICE_NETWORK) {
            return Err(invalid("对方版本不支持设备网络，请先升级"));
        }
        let route = RouteCandidate::new(
            endpoint.clone(),
            RouteSource::Manual,
            RouteHealth::Unknown,
            capabilities,
        );
        if let Some(existing) = self.peers.get_mut(&peer) {
            // Explicit IP input also supplies a fresh route for an offline
            // existing member; it does not change their trust or public key.
            if !existing
                .routes
                .values()
                .any(|previous| previous.endpoint == endpoint)
            {
                existing.routes.insert(route.id, route.clone());
            }
        }
        self.collapse_peer_routes(peer);
        self.hints.insert(
            peer,
            PeerHintState {
                display_name: remote.display_name.clone(),
                capabilities,
                routes: BTreeMap::from([(route.id, route)]),
            },
        );
        self.publish_view();
        self.start_network_pairing_at(peer, Some(endpoint)).await
    }

    pub(super) async fn request_network_pairing(&mut self, peer: DeviceId) {
        if let Err(error) = self.start_network_pairing(peer).await {
            self.emit_error(error);
        }
    }
    async fn start_network_pairing(&mut self, peer: DeviceId) -> Result<(), RuntimeError> {
        self.start_network_pairing_at(peer, None).await
    }
    async fn start_network_pairing_at(
        &mut self,
        peer: DeviceId,
        direct_endpoint: Option<Endpoint>,
    ) -> Result<(), RuntimeError> {
        if self.device_network.integrity_error.is_some() {
            return Err(invalid("设备网络记录损坏，请先退出网络恢复"));
        }
        if !self.transport.supports_network_pairing()
            || !self
                .config
                .capabilities
                .contains(Capabilities::DEVICE_NETWORK)
        {
            return Err(invalid("请升级两端以使用接收方批准的设备网络"));
        }
        if peer == self.identity.device_id() {
            return Err(invalid("cannot pair this same device"));
        }
        if self
            .device_network
            .pending
            .values()
            .any(|p| p.peer_id == peer)
        {
            return Ok(());
        }
        if self.device_network.pending.len() >= 16 {
            return Err(invalid("too many pending network invitations"));
        }
        if let Some(roster) = &self.device_network.roster {
            if roster.roster.members.iter().any(|m| m.device_id == peer)
                && !self.blocked.contains(&peer)
            {
                self.connect_peer(peer).await;
                return Ok(());
            }
            if roster.roster.members.len() >= 32 {
                return Err(invalid("device network supports at most 32 devices"));
            }
        }
        let endpoint = if let Some(endpoint) = direct_endpoint {
            endpoint
        } else {
            let routes = self
                .peers
                .get(&peer)
                .map(|p| p.routes.values().cloned().collect::<Vec<_>>())
                .or_else(|| {
                    self.hints
                        .get(&peer)
                        .map(|h| h.routes.values().cloned().collect())
                })
                .ok_or(RuntimeError::UnknownPeer(peer))?;
            routes
                .iter()
                .find(|r| self.policy.select(&[(*r).clone()]).is_some())
                .map(|r| r.endpoint.clone())
                .ok_or(RuntimeError::UnknownPeer(peer))?
        };
        if self.blocked.remove(&peer) {
            self.transport.restore_peer(peer).await?;
            self.registered_identities.remove(&peer);
        }
        let remote = self
            .transport
            .bootstrap_connect(
                self.local_hello.clone(),
                peer,
                RouteCandidate::new(
                    endpoint.clone(),
                    RouteSource::Manual,
                    RouteHealth::Unknown,
                    Capabilities::NETWORK,
                ),
            )
            .await?
            .remote_hello
            .ok_or_else(|| invalid("missing signed peer hello"))?;
        verify_hello(&remote).map_err(invalid)?;
        if !Capabilities::from_bits(remote.capabilities).contains(Capabilities::DEVICE_NETWORK) {
            return Err(invalid("对方版本需要升级，尚不支持接收方批准的设备网络"));
        }
        if let Some(roster) = self.device_network.roster.clone()
            && roster.roster.owner.device_id != self.identity.device_id()
        {
            return self.start_member_invitation(
                roster,
                PeerHello::from_wire(remote).map_err(invalid)?,
                endpoint,
            );
        }
        let roster = match self.device_network.roster.clone() {
            Some(r) => r,
            None => {
                let owner = self.network_self(endpoint.clone()).await?;
                let roster = SignedNetworkRoster::signed(
                    NetworkRoster {
                        merge_permits: Vec::new(),
                        network_id: RequestId::new(),
                        owner: owner.clone(),
                        epoch: 1,
                        members: vec![owner],
                    },
                    &self.identity,
                )
                .map_err(invalid)?;
                self.persist_network(roster.clone()).await?;
                roster
            }
        };
        let offer = NetworkOffer::signed(
            roster,
            peer,
            now_ms().saturating_add(120_000),
            &self.identity,
        )
        .map_err(invalid)?;
        let attempt = offer.attempt;
        let payload = self.sign_network(NetworkOperation::Offer(offer.clone()))?;
        self.device_network.pending.insert(
            attempt,
            NetworkPairingPending {
                request_id: attempt,
                peer_id: peer,
                display_name: remote.display_name,
                incoming: false,
                expires_at_unix_ms: offer.expires_at_unix_ms,
                request: None,
                owner_name: offer.roster.roster.owner.display_name.clone(),
                member_names: offer
                    .roster
                    .roster
                    .members
                    .iter()
                    .map(|m| m.display_name.clone())
                    .collect(),
            },
        );
        self.device_network.outgoing.insert(
            attempt,
            Outgoing {
                offer,
                peer,
                endpoint: endpoint.clone(),
            },
        );
        let transport = self.transport.clone();
        let tx = self.network_result_tx.clone();
        self.device_network.tasks.push(task::spawn(async move {
            let result = transport.network_exchange(peer, endpoint, payload).await;
            let _ = tx.send(NetworkResult::Offer { attempt, result }).await;
        }));
        let _ = self.event_tx.send(RuntimeEvent::PairingRequestSent {
            peer_id: peer,
            request_id: attempt,
        });
        self.publish_view();
        Ok(())
    }
    pub(super) async fn handle_network_request(&mut self, request: NetworkTransportRequest) {
        if let Err(error) = self.receive_network_request(request).await {
            self.emit_error(error);
        }
    }
    async fn receive_network_request(
        &mut self,
        request: NetworkTransportRequest,
    ) -> Result<(), RuntimeError> {
        let peer = request.remote.device_id;
        if self.device_network.integrity_error.is_some() {
            return Err(invalid("device network integrity unavailable"));
        }
        if !(self.config.capabilities & request.remote.capabilities)
            .contains(Capabilities::DEVICE_NETWORK)
        {
            return Err(invalid("双方均需运行支持设备网络的 uniDesk 版本"));
        }
        let reauthorizing = self.blocked.contains(&peer);
        if reauthorizing && !self.can_receive_repair_invitation(peer) {
            return Err(invalid(format!(
                "{} 已在本机被阻止；请在设备网络中点击该设备的「允许重新配对」，再让对方发起邀请",
                request.remote.display_name
            )));
        }
        let operation =
            SignedNetworkOperation::decode_verify(&request.payload, &request.remote.public_key)
                .map_err(invalid)?
                .operation;
        if reauthorizing && !matches!(&operation, NetworkOperation::Offer(_)) {
            return Err(RuntimeError::PeerBlocked(peer));
        }
        match operation {
            NetworkOperation::InviteViaOwner {
                attempt,
                network_id,
                target,
                expires_at_unix_ms,
            } => {
                return self
                    .receive_member_invitation(
                        request,
                        attempt,
                        network_id,
                        target,
                        expires_at_unix_ms,
                    )
                    .await;
            }
            NetworkOperation::Offer(offer) => {
                let now = now_ms();
                if offer.target != self.identity.device_id()
                    || offer.roster.roster.owner.device_id != peer
                    || offer.roster.roster.owner.public_key != request.remote.public_key
                {
                    return Err(invalid("misaddressed network invitation"));
                }
                // Only a verified invitation addressed to this identity gets
                // a signed rejection. Never relax expiry or replay checks;
                // report their reason instead of dropping the reply channel.
                let rejection = if offer.expires_at_unix_ms <= now {
                    Some("邀请已过期，请检查两台 Mac 的系统时间后重新发起配对")
                } else if offer.expires_at_unix_ms > now.saturating_add(125_000) {
                    Some("两台 Mac 的系统时间可能不一致，请同步系统时间后重新发起配对")
                } else if self.device_network.seen.contains_key(&offer.attempt) {
                    Some("该邀请已处理，请重新发起配对")
                } else {
                    None
                };
                if let Some(reason) = rejection {
                    return self
                        .send_immediate_decision(request, offer, false, Some(reason.into()))
                        .await;
                }
                if let Some(current) = &self.device_network.roster {
                    if current.roster.network_id == offer.roster.roster.network_id
                        && offer.roster.roster.epoch < current.roster.epoch
                    {
                        return self
                            .send_immediate_decision(
                                request,
                                offer,
                                false,
                                Some("邀请中的设备网络信息已过期，请在发起端刷新后重试".into()),
                            )
                            .await;
                    }
                    if !reauthorizing
                        && current.roster.network_id == offer.roster.roster.network_id
                        && current
                            .roster
                            .members
                            .iter()
                            .any(|m| m.device_id == self.identity.device_id())
                    {
                        self.send_immediate_decision(request, offer, true, None)
                            .await?;
                        return Ok(());
                    }
                }
                // Preserve the receiving network and its owner when connecting networks,
                // and route requests received by a member to its signed owner.
                if let Some(current) = &self.device_network.roster
                    && !is_solo_network(current, self.identity.device_id())
                    && current.roster.network_id != offer.roster.roster.network_id
                    && current.roster.owner.device_id != self.identity.device_id()
                {
                    let payload = self.sign_network(NetworkOperation::Redirect {
                        attempt: offer.attempt,
                        offer_hash: network_offer_hash(&offer).map_err(invalid)?,
                        roster: current.clone(),
                    })?;
                    let _ = request.reply.send(payload);
                    return Ok(());
                }
                if self.device_network.pending.len() >= 16
                    || self
                        .device_network
                        .pending
                        .values()
                        .any(|p| p.peer_id == peer)
                    || self
                        .device_network
                        .incoming_rate
                        .get(&peer)
                        .is_some_and(|time| now.saturating_sub(*time) < 5000)
                {
                    return self
                        .send_immediate_decision(
                            request,
                            offer,
                            false,
                            Some(
                                "接收方有待处理的邀请或配对请求过于频繁，请先处理原邀请，稍后重试"
                                    .into(),
                            ),
                        )
                        .await;
                }
                self.device_network.incoming_rate.insert(peer, now);
                let core = CorePairingRequest::with_request_id(
                    offer.attempt,
                    request.remote.display_name.clone(),
                    request.remote.public_key,
                    ProtocolVersion::V1,
                    request.remote.capabilities,
                )
                .map_err(invalid)?;
                let displayed_roster = self
                    .device_network
                    .roster
                    .as_ref()
                    .filter(|r| !is_solo_network(r, self.identity.device_id()))
                    .unwrap_or(&offer.roster);
                let mut displayed_members = displayed_roster.roster.members.clone();
                for member in &offer.roster.roster.members {
                    if !displayed_members
                        .iter()
                        .any(|m| m.device_id == member.device_id)
                    {
                        displayed_members.push(member.clone());
                    }
                }
                self.device_network.pending.insert(
                    offer.attempt,
                    NetworkPairingPending {
                        request_id: offer.attempt,
                        peer_id: peer,
                        display_name: request.remote.display_name.clone(),
                        incoming: true,
                        expires_at_unix_ms: offer.expires_at_unix_ms,
                        request: Some(core.clone()),
                        owner_name: displayed_roster.roster.owner.display_name.clone(),
                        member_names: displayed_members
                            .iter()
                            .map(|m| m.display_name.clone())
                            .collect(),
                    },
                );
                self.device_network.incoming.insert(
                    offer.attempt,
                    Incoming {
                        offer,
                        remote: request.remote,
                        reply: request.reply,
                    },
                );
                self.publish_view();
                let _ = self.event_tx.send(RuntimeEvent::PairingRequest(core));
            }
            NetworkOperation::Roster(roster) => {
                self.accept_network_roster(peer, roster.clone()).await?;
                let payload = self.sign_network(NetworkOperation::Ack {
                    network_id: roster.roster.network_id,
                    epoch: roster.roster.epoch,
                })?;
                let _ = request.reply.send(payload);
            }
            NetworkOperation::Leave { network_id } => {
                let current = self
                    .device_network
                    .roster
                    .as_ref()
                    .filter(|r| {
                        r.roster.network_id == network_id
                            && r.roster.owner.device_id == self.identity.device_id()
                            && r.roster.members.iter().any(|m| {
                                m.device_id == peer && m.public_key == request.remote.public_key
                            })
                    })
                    .ok_or_else(|| invalid("only active members may leave this network"))?;
                let mut roster = current.roster.clone();
                roster.members.retain(|m| m.device_id != peer);
                roster.epoch = roster
                    .epoch
                    .checked_add(1)
                    .ok_or_else(|| invalid("network epoch overflow"))?;
                let roster =
                    SignedNetworkRoster::signed(roster, &self.identity).map_err(invalid)?;
                self.persist_network(roster.clone()).await?;
                let payload = self.sign_network(NetworkOperation::Ack {
                    network_id,
                    epoch: roster.roster.epoch,
                })?;
                let _ = request.reply.send(payload);
                self.remove_inherited_member(peer).await?;
                self.device_network.last_sync = 0;
            }
            NetworkOperation::Query { network_id, .. } => {
                let current = self
                    .device_network
                    .roster
                    .as_ref()
                    .filter(|r| {
                        ((r.roster.network_id == network_id
                            && r.roster.owner.device_id == self.identity.device_id())
                            || r.roster.merge_permits.iter().any(|p| {
                                p.network_id == network_id
                                    && p.owner_public_key == self.identity.verifying_key_bytes()
                            }))
                            && r.roster.members.iter().any(|m| {
                                m.device_id == peer && m.public_key == request.remote.public_key
                            })
                    })
                    .ok_or_else(|| invalid("only current members may request a roster"))?;
                let payload = self.sign_network(NetworkOperation::Roster(current.clone()))?;
                let _ = request.reply.send(payload);
            }
            _ => return Err(invalid("unsolicited network decision or acknowledgement")),
        }
        Ok(())
    }
    async fn send_immediate_decision(
        &self,
        request: NetworkTransportRequest,
        offer: NetworkOffer,
        accepted: bool,
        reason: Option<String>,
    ) -> Result<(), RuntimeError> {
        let member = self
            .network_self(offer.roster.roster.owner.endpoint.clone())
            .await?;
        let payload = self.sign_network(NetworkOperation::Decision(NetworkDecision {
            attempt: offer.attempt,
            offer_hash: network_offer_hash(&offer).map_err(invalid)?,
            target: request.remote.device_id,
            accepted,
            member,
            reason,
        }))?;
        let _ = request.reply.send(payload);
        Ok(())
    }
    pub(super) async fn decide_network_pairing(
        &mut self,
        attempt: RequestId,
        accepted: bool,
        reason: Option<String>,
    ) {
        if let Err(error) = self
            .decide_network_pairing_inner(attempt, accepted, reason)
            .await
        {
            if self.device_network.pending.contains_key(&attempt) {
                let _ = self.event_tx.send(RuntimeEvent::PairingDecisionFailed {
                    request_id: attempt,
                    reason: error.to_string(),
                });
            }
            self.emit_error(error);
        }
    }
    async fn decide_network_pairing_inner(
        &mut self,
        attempt: RequestId,
        accepted: bool,
        reason: Option<String>,
    ) -> Result<(), RuntimeError> {
        let incoming = self
            .device_network
            .incoming
            .get(&attempt)
            .ok_or(RuntimeError::UnknownPairingRequest(attempt))?;
        if incoming.offer.expires_at_unix_ms <= now_ms() || incoming.reply.is_closed() {
            let peer = incoming.remote.device_id;
            self.finish_network_pairing(attempt, peer, "expired");
            return Err(invalid("network invitation expired"));
        }
        let offer = incoming.offer.clone();
        let peer = incoming.remote.device_id;
        let reauthorizing = self.blocked.contains(&peer);
        if reauthorizing && !self.can_receive_repair_invitation(peer) {
            self.finish_network_pairing(attempt, peer, "rejected");
            return Err(RuntimeError::PeerBlocked(peer));
        }
        let member = self
            .network_self(offer.roster.roster.owner.endpoint.clone())
            .await?;
        if accepted
            && let Some(current) = self.device_network.roster.clone()
            && current.roster.network_id != offer.roster.roster.network_id
            && !is_solo_network(&current, self.identity.device_id())
        {
            if current.roster.owner.device_id != self.identity.device_id() || reauthorizing {
                return Err(invalid("network join is no longer authorized"));
            }
            let mut roster = current.roster;
            for joining in &offer.roster.roster.members {
                if self.blocked.contains(&joining.device_id) {
                    return Err(invalid("待合并网络含有本机已阻止的设备，请先处理该设备"));
                }
                if let Some(existing) = roster
                    .members
                    .iter()
                    .find(|m| m.device_id == joining.device_id)
                {
                    if existing.public_key != joining.public_key {
                        return Err(invalid("conflicting member identity"));
                    }
                } else {
                    roster.members.push(joining.clone());
                }
            }
            if roster.members.len() > 32 {
                return Err(invalid("device network supports at most 32 devices"));
            }
            for permit in offer
                .roster
                .roster
                .merge_permits
                .iter()
                .chain(std::iter::once(&offer.merge_permit))
            {
                if !roster.merge_permits.contains(permit) {
                    roster.merge_permits.push(permit.clone());
                }
            }
            roster.members.sort_by_key(|m| m.device_id);
            roster.epoch = roster
                .epoch
                .checked_add(1)
                .ok_or_else(|| invalid("network epoch overflow"))?;
            let roster = SignedNetworkRoster::signed(roster, &self.identity).map_err(invalid)?;
            let payload = self.sign_network(NetworkOperation::Joined {
                decision: NetworkDecision {
                    attempt,
                    offer_hash: network_offer_hash(&offer).map_err(invalid)?,
                    target: peer,
                    accepted: true,
                    member,
                    reason: None,
                },
                roster: roster.clone(),
            })?;
            self.persist_network(roster.clone()).await?;
            for joining in &offer.roster.roster.members {
                self.admit_network_member(joining.clone()).await?;
            }
            if let Some(incoming) = self.device_network.incoming.remove(&attempt) {
                let _ = incoming.reply.send(payload);
            }
            self.finish_network_pairing(attempt, peer, "accepted");
            self.device_network.last_sync = 0;
            return Ok(());
        }
        if accepted {
            // Persist acceptance before sending the decision. Only the owner is admitted until it publishes a roster including us.
            if self.device_network.roster.as_ref().is_some_and(|r| {
                r.roster.network_id != offer.roster.roster.network_id
                    && !is_solo_network(r, self.identity.device_id())
            }) {
                return Err(invalid("请先退出原设备网络"));
            }
            let previous = self.device_network.roster.clone();
            let was_trusted = self.trusted.contains(&peer);
            self.persist_network(offer.roster.clone()).await?;
            if reauthorizing {
                self.blocked.remove(&peer);
            }
            if let Err(error) = self
                .admit_network_member(offer.roster.roster.owner.clone())
                .await
            {
                if reauthorizing {
                    // Admission can fail after its trust write. Restore the
                    // durable tombstone and close sessions even if roster-origin
                    // cleanup fails, rather than resurrecting trust on restart.
                    self.trusted.remove(&peer);
                    self.registered_identities.remove(&peer);
                    self.pending_app_messages.remove(&peer);
                    if let Some(state) = self.peers.get_mut(&peer) {
                        state.trust = TrustState::Blocked;
                        state.connection = ConnectionState::Disconnected;
                        state.active_route = None;
                        state.active_capabilities = None;
                    }
                    let transport_result = self.transport.revoke_peer(peer).await;
                    let trust = self.trust_store.clone();
                    let trust_result = task::spawn_blocking(move || {
                        trust.upsert(TrustRecord::revoked(peer, Fingerprint::from(peer)))
                    })
                    .await;
                    if transport_result.is_err() || !matches!(trust_result, Ok(Ok(()))) {
                        self.blocked.insert(peer);
                        self.device_network.integrity_error = Some(
                            "批准失败且撤销状态未能完整恢复；设备保持阻止，请检查存储与传输诊断"
                                .into(),
                        );
                        self.publish();
                        return Err(error);
                    }
                }
                if !was_trusted && let Err(cleanup_error) = self.remove_inherited_member(peer).await
                {
                    self.device_network.integrity_error = Some(format!(
                        "批准失败且信任回滚未完成，请退出网络恢复：{cleanup_error}"
                    ));
                    if reauthorizing {
                        self.blocked.insert(peer);
                    }
                    self.publish_view();
                    return Err(error);
                }
                let store = self.persistence.clone();
                let saved = previous.clone();
                if matches!(
                    task::spawn_blocking(move || store.set_device_network(saved)).await,
                    Ok(Ok(()))
                ) {
                    self.device_network.roster = previous;
                } else {
                    self.device_network.integrity_error =
                        Some("批准失败，网络记录无法回滚，请退出网络恢复".into());
                }
                if reauthorizing {
                    self.blocked.insert(peer);
                }
                self.publish_view();
                return Err(error);
            }
        }
        let payload = self.sign_network(NetworkOperation::Decision(NetworkDecision {
            attempt,
            offer_hash: network_offer_hash(&offer).map_err(invalid)?,
            target: peer,
            accepted,
            member,
            reason,
        }))?;
        if let Some(incoming) = self.device_network.incoming.remove(&attempt) {
            let _ = incoming.reply.send(payload);
        }
        self.finish_network_pairing(
            attempt,
            peer,
            if accepted { "accepted" } else { "rejected" },
        );
        Ok(())
    }
    async fn admit_network_member(&mut self, member: NetworkMember) -> Result<(), RuntimeError> {
        if member.device_id == self.identity.device_id() || self.blocked.contains(&member.device_id)
        {
            return Ok(());
        }
        member.validate().map_err(invalid)?;
        let peer = member.device_id;
        let mut origins = self.device_network.origins.clone();
        if !self.trusted.contains(&peer) {
            origins.introduced.insert(peer);
        }
        let restore = origins.network_revoked.remove(&peer);
        if origins != self.device_network.origins {
            self.persist_network_origins(origins).await?;
        }
        if restore {
            self.transport.restore_peer(peer).await?;
            self.registered_identities.remove(&peer);
        }
        let trust = self.trust_store.clone();
        let record = TrustRecord::new(peer, Fingerprint::from(peer));
        task::spawn_blocking(move || trust.upsert(record))
            .await
            .map_err(invalid)?
            .map_err(invalid)?;
        self.resume_reconnect(peer).await;
        self.trusted.insert(peer);
        self.register_peer_identity(peer, member.public_key).await?;
        let existing_route = self
            .peers
            .get(&peer)
            .and_then(|p| p.routes.values().find(|r| r.endpoint == member.endpoint))
            .cloned();
        let route = existing_route.unwrap_or_else(|| {
            RouteCandidate::new(
                member.endpoint,
                RouteSource::Manual,
                RouteHealth::Unknown,
                member.capabilities,
            )
        });
        let state = self.peers.entry(peer).or_insert_with(|| PeerState {
            id: peer,
            display_name: member.display_name.clone(),
            public_key: member.public_key,
            routes: BTreeMap::new(),
            liveness: LivenessState::Online,
            trust: TrustState::Trusted,
            connection: ConnectionState::Disconnected,
            preferred_route: None,
            active_route: None,
            active_capabilities: None,
            pending_pairing: None,
        });
        state.display_name = member.display_name;
        state.public_key = member.public_key;
        state.trust = TrustState::Trusted;
        state.routes.insert(route.id, route);
        state.pending_pairing = None;
        if state.connection == ConnectionState::Failed {
            state.connection = ConnectionState::Disconnected;
        }
        self.collapse_peer_routes(peer);
        self.reselect(peer);
        self.publish();
        self.persist_peer(peer).await;
        Ok(())
    }
    async fn accept_network_roster(
        &mut self,
        sender: DeviceId,
        roster: SignedNetworkRoster,
    ) -> Result<(), RuntimeError> {
        roster.verify().map_err(invalid)?;
        let current = self
            .device_network
            .roster
            .as_ref()
            .ok_or_else(|| invalid("network roster requires explicit prior acceptance"))?;
        let migrating = current.roster.network_id != roster.roster.network_id
            && roster.authorizes_migration(&current.roster);
        if self.blocked.contains(&sender)
            || (roster.roster.owner.device_id != sender
                && !(migrating && current.roster.owner.device_id == sender))
            || (!migrating
                && (current.roster.owner.device_id != sender
                    || current.roster.owner.public_key != roster.roster.owner.public_key
                    || current.roster.network_id != roster.roster.network_id
                    || roster.roster.epoch < current.roster.epoch))
        {
            return Err(invalid("unauthorized or stale network roster"));
        }
        if !migrating && roster.roster.epoch == current.roster.epoch && roster != *current {
            return Err(invalid("conflicting roster at the same epoch"));
        }
        if roster == *current
            && roster.roster.members.iter().all(|m| {
                m.device_id == self.identity.device_id()
                    || self.blocked.contains(&m.device_id)
                    || (self.trusted.contains(&m.device_id)
                        && self.registered_identities.get(&m.device_id) == Some(&m.public_key))
            })
        {
            return Ok(());
        }
        let old = current
            .roster
            .members
            .iter()
            .map(|m| m.device_id)
            .collect::<BTreeSet<_>>();
        let active = roster
            .roster
            .members
            .iter()
            .map(|m| m.device_id)
            .collect::<BTreeSet<_>>();
        self.persist_network(roster.clone()).await?;
        if !active.contains(&self.identity.device_id()) {
            // Removal of this device completes local network departure. Only explicit local blocks remain permanent.
            for peer in old {
                if peer != self.identity.device_id() {
                    self.remove_inherited_member(peer).await?;
                }
            }
            let store = self.persistence.clone();
            task::spawn_blocking(move || store.set_device_network(None))
                .await
                .map_err(invalid)??;
            self.device_network.roster = None;
            self.device_network.incoming_rate.remove(&sender);
            self.device_network.last_sync = 0;
            self.publish_view();
            return Ok(());
        }
        for peer in old.difference(&active) {
            if *peer != self.identity.device_id() {
                self.remove_inherited_member(*peer).await?;
            }
        }
        for member in roster.roster.members {
            self.admit_network_member(member).await?;
        }
        Ok(())
    }
    pub(super) async fn handle_network_result(&mut self, result: NetworkResult) {
        match result {
            NetworkResult::ManualProbe {
                attempt,
                endpoint,
                result,
            } => {
                if self.device_network.manual_probe != Some(attempt) {
                    return;
                }
                self.device_network.manual_probe = None;
                let result = match result {
                    Ok(remote) => self.complete_pairing_at(endpoint, remote).await,
                    Err(error) => Err(error),
                };
                let message = match result {
                    Ok(()) => "已发起连接；如需加入设备网络，请在对方设备确认邀请".to_owned(),
                    Err(error) => format!("无法通过 IP 配对：{error}"),
                };
                let _ = self.event_tx.send(RuntimeEvent::PairingStatus { message });
            }
            NetworkResult::MemberInvitation {
                attempt,
                owner,
                target,
                result,
            } => {
                self.complete_member_invitation(attempt, owner, target, result)
                    .await;
            }
            NetworkResult::Offer { attempt, result } => {
                let Some(outgoing) = self.device_network.outgoing.remove(&attempt) else {
                    return;
                };
                if self
                    .device_network
                    .relayed
                    .get(&attempt)
                    .is_some_and(|r| r.expires_at_unix_ms <= now_ms())
                {
                    self.finish_network_pairing(attempt, outgoing.peer, "expired");
                    return;
                }
                let outcome = self.complete_outgoing(&outgoing, result).await;
                match outcome {
                    Ok(PairingOutcome::Accepted) => {
                        self.finish_network_pairing(attempt, outgoing.peer, "accepted")
                    }
                    Ok(PairingOutcome::Redirect(owner)) => {
                        let relayed = self.device_network.relayed.remove(&attempt);
                        self.finish_network_pairing(attempt, outgoing.peer, "redirected");
                        let route = RouteCandidate::new(
                            owner.endpoint,
                            RouteSource::Manual,
                            RouteHealth::Unknown,
                            owner.capabilities,
                        );
                        self.hints.insert(
                            owner.device_id,
                            PeerHintState {
                                display_name: owner.display_name,
                                capabilities: owner.capabilities,
                                routes: BTreeMap::from([(route.id, route)]),
                            },
                        );
                        let result = self.start_network_pairing(owner.device_id).await;
                        if let Some(relayed) = relayed {
                            self.attach_relay(owner.device_id, relayed, result);
                        } else if let Err(error) = result {
                            self.emit_error(error);
                        }
                    }
                    Ok(PairingOutcome::Rejected(reason)) => {
                        self.finish_network_pairing(attempt, outgoing.peer, "rejected");
                        if let Some(reason) = reason {
                            self.emit_error(invalid(format!("配对未完成：{reason}")));
                        }
                    }
                    Err(e) => {
                        self.finish_network_pairing(attempt, outgoing.peer, "failed");
                        self.emit_error(e);
                    }
                }
                self.device_network.last_sync = 0;
            }
            NetworkResult::Exchange { peer, result } => {
                self.device_network.inflight.remove(&peer);
                if let Ok((hello, bytes)) = result {
                    if hello.device_id != peer {
                        return;
                    }
                    if let Ok(signed) =
                        SignedNetworkOperation::decode_verify(&bytes, &hello.public_key)
                        && let NetworkOperation::Roster(roster) = signed.operation
                        && let Err(e) = self.accept_network_roster(peer, roster).await
                    {
                        self.emit_error(e);
                    }
                }
            }
        }
    }
    async fn complete_outgoing(
        &mut self,
        outgoing: &Outgoing,
        result: Result<(PeerHello, Vec<u8>), RuntimeError>,
    ) -> Result<PairingOutcome, RuntimeError> {
        if outgoing.offer.expires_at_unix_ms <= now_ms() {
            return Err(invalid("network invitation expired"));
        }
        let (hello, bytes) = result?;
        if hello.device_id != outgoing.peer
            || !hello.capabilities.contains(Capabilities::DEVICE_NETWORK)
        {
            return Err(invalid("network decision identity or capability mismatch"));
        }
        let signed =
            SignedNetworkOperation::decode_verify(&bytes, &hello.public_key).map_err(invalid)?;
        let (decision, joined) =
            match signed.operation {
                NetworkOperation::Decision(d) => (d, None),
                NetworkOperation::Joined { decision, roster } => (decision, Some(roster)),
                NetworkOperation::Redirect {
                    attempt,
                    offer_hash,
                    roster,
                } => {
                    if attempt != outgoing.offer.attempt
                        || offer_hash != network_offer_hash(&outgoing.offer).map_err(invalid)?
                        || !roster.roster.members.iter().any(|m| {
                            m.device_id == hello.device_id && m.public_key == hello.public_key
                        })
                        || roster.roster.owner.device_id == hello.device_id
                        || self.blocked.contains(&roster.roster.owner.device_id)
                    {
                        return Err(invalid("invalid network owner redirect"));
                    }
                    return Ok(PairingOutcome::Redirect(roster.roster.owner));
                }
                _ => return Err(invalid("expected recipient decision")),
            };
        if decision.attempt != outgoing.offer.attempt
            || decision.target != self.identity.device_id()
            || decision.offer_hash != network_offer_hash(&outgoing.offer).map_err(invalid)?
            || decision.member.device_id != outgoing.peer
            || decision.member.public_key != hello.public_key
        {
            return Err(invalid("decision does not match this exact invitation"));
        }
        if !decision.accepted {
            return Ok(PairingOutcome::Rejected(decision.reason));
        }
        let mut roster = self
            .device_network
            .roster
            .clone()
            .ok_or_else(|| invalid("network owner state lost"))?
            .roster;
        if self.blocked.contains(&outgoing.peer) {
            return Err(invalid("network invitation was revoked"));
        }
        if let Some(joined) = joined {
            let offered = &outgoing.offer.roster.roster;
            if joined.roster.owner.device_id != hello.device_id
                || joined.roster.owner.public_key != hello.public_key
                || !joined
                    .roster
                    .merge_permits
                    .contains(&outgoing.offer.merge_permit)
                || !joined.authorizes_migration(offered)
                || !offered.members.iter().all(|member| {
                    joined.roster.members.iter().any(|m| {
                        m.device_id == member.device_id && m.public_key == member.public_key
                    })
                })
            {
                return Err(invalid("network join conflicts with invitation"));
            }
            // The periodic signed roster may arrive before this response.
            if roster.network_id == joined.roster.network_id && roster.owner == joined.roster.owner
            {
                if roster.epoch < joined.roster.epoch {
                    self.accept_network_roster(hello.device_id, joined).await?;
                }
                return Ok(PairingOutcome::Accepted);
            }
            if roster != *offered || self.device_network.pending.len() != 1 {
                return Err(invalid("network join conflicts with current membership"));
            }
            self.persist_network(joined.clone()).await?;
            for member in joined.roster.members {
                self.admit_network_member(member).await?;
            }
            return Ok(PairingOutcome::Accepted);
        }
        if roster.owner.device_id != self.identity.device_id()
            || roster.network_id != outgoing.offer.roster.roster.network_id
        {
            return Err(invalid("network invitation was revoked"));
        }
        if roster.members.len() >= 32
            && !roster.members.iter().any(|m| m.device_id == outgoing.peer)
        {
            return Err(invalid("network member limit"));
        }
        roster.members.retain(|m| m.device_id != outgoing.peer);
        roster.members.push(decision.member.clone());
        roster.members.sort_by_key(|m| m.device_id);
        roster.epoch = roster
            .epoch
            .checked_add(1)
            .ok_or_else(|| invalid("network epoch overflow"))?;
        let roster = SignedNetworkRoster::signed(roster, &self.identity).map_err(invalid)?;
        self.persist_network(roster.clone()).await?;
        self.admit_network_member(decision.member).await?;
        self.spawn_network_exchange(
            outgoing.peer,
            outgoing.endpoint.clone(),
            NetworkOperation::Roster(roster),
        );
        self.connect_peer(outgoing.peer).await;
        Ok(PairingOutcome::Accepted)
    }
    fn spawn_network_exchange(
        &mut self,
        peer: DeviceId,
        endpoint: Endpoint,
        operation: NetworkOperation,
    ) {
        if self.device_network.inflight.len() >= 32 || !self.device_network.inflight.insert(peer) {
            return;
        }
        let payload = match self.sign_network(operation) {
            Ok(p) => p,
            Err(e) => {
                self.device_network.inflight.remove(&peer);
                self.emit_error(e);
                return;
            }
        };
        let transport = self.transport.clone();
        let tx = self.network_result_tx.clone();
        self.device_network.tasks.push(task::spawn(async move {
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                transport.network_exchange(peer, endpoint, payload),
            )
            .await
            .unwrap_or_else(|_| Err(invalid("network roster exchange timeout")));
            let _ = tx.send(NetworkResult::Exchange { peer, result }).await;
        }));
    }
    pub(super) async fn network_tick(&mut self) {
        if !self.transport.supports_network_pairing()
            || self.device_network.integrity_error.is_some()
        {
            return;
        }
        let now = now_ms();
        let recovery_count = self.device_network.pairing_recovery.len();
        self.device_network
            .pairing_recovery
            .retain(|_, deadline| *deadline > tokio::time::Instant::now());
        if recovery_count != self.device_network.pairing_recovery.len() {
            self.publish_view();
        }
        let expired = self
            .device_network
            .pending
            .values()
            .filter(|p| {
                p.expires_at_unix_ms <= now
                    || self
                        .device_network
                        .incoming
                        .get(&p.request_id)
                        .is_some_and(|i| i.reply.is_closed())
            })
            .map(|p| (p.request_id, p.peer_id))
            .collect::<Vec<_>>();
        for (attempt, peer) in expired {
            self.finish_network_pairing(attempt, peer, "expired");
        }
        self.device_network.seen.retain(|_, expiry| *expiry > now);
        self.device_network
            .incoming_rate
            .retain(|_, time| now.saturating_sub(*time) < 300_000);
        self.device_network.tasks.retain(|t| !t.is_finished());
        if now.saturating_sub(self.device_network.last_sync) < 5000 {
            return;
        }
        self.device_network.last_sync = now;
        let Some(roster) = self.device_network.roster.clone() else {
            return;
        };
        if roster.roster.owner.device_id == self.identity.device_id() {
            for member in &roster.roster.members {
                if member.device_id != self.identity.device_id()
                    && !self.blocked.contains(&member.device_id)
                {
                    self.spawn_network_exchange(
                        member.device_id,
                        member.endpoint.clone(),
                        NetworkOperation::Roster(roster.clone()),
                    );
                }
            }
        } else if !self.blocked.contains(&roster.roster.owner.device_id) {
            self.spawn_network_exchange(
                roster.roster.owner.device_id,
                roster.roster.owner.endpoint.clone(),
                NetworkOperation::Query {
                    network_id: roster.roster.network_id,
                    epoch: roster.roster.epoch,
                },
            );
        }
    }
    pub(super) async fn network_revoke_member(&mut self, peer: DeviceId) {
        self.device_network.pairing_recovery.remove(&peer);
        let ids = self
            .device_network
            .pending
            .values()
            .filter(|p| {
                p.peer_id == peer
                    || self
                        .device_network
                        .relayed
                        .get(&p.request_id)
                        .is_some_and(|r| r.member == peer)
            })
            .map(|p| p.request_id)
            .collect::<Vec<_>>();
        for id in ids {
            self.finish_network_pairing(id, peer, "rejected");
        }
        let mut origins = self.device_network.origins.clone();
        origins.network_revoked.remove(&peer);
        if let Err(e) = self.persist_network_origins(origins).await {
            self.emit_error(e);
        }
        let Some(mut roster) = self.device_network.roster.clone() else {
            return;
        };
        if roster.roster.owner.device_id != self.identity.device_id()
            || peer == self.identity.device_id()
        {
            return;
        }
        if !roster.roster.members.iter().any(|m| m.device_id == peer) {
            return;
        }
        let recipients = roster.roster.members.clone();
        roster.roster.members.retain(|m| m.device_id != peer);
        let Some(epoch) = roster.roster.epoch.checked_add(1) else {
            return;
        };
        roster.roster.epoch = epoch;
        match SignedNetworkRoster::signed(roster.roster, &self.identity) {
            Ok(roster) => {
                if let Err(e) = self.persist_network(roster.clone()).await {
                    self.emit_error(e);
                    return;
                }
                if let Ok(payload) = self.sign_network(NetworkOperation::Roster(roster)) {
                    let exchanges = recipients
                        .into_iter()
                        .filter(|m| {
                            m.device_id != self.identity.device_id()
                                && !self.blocked.contains(&m.device_id)
                        })
                        .map(|m| {
                            self.transport.network_exchange(
                                m.device_id,
                                m.endpoint,
                                payload.clone(),
                            )
                        })
                        .collect::<Vec<_>>();
                    let _ = tokio::time::timeout(
                        std::time::Duration::from_secs(2),
                        futures::future::join_all(exchanges),
                    )
                    .await;
                }
                self.device_network.last_sync = 0;
            }
            Err(e) => self.emit_error(invalid(e)),
        }
    }
    async fn remove_inherited_member(&mut self, peer: DeviceId) -> Result<(), RuntimeError> {
        self.suspend_reconnect(peer).await;
        if peer == self.identity.device_id() || self.blocked.contains(&peer) {
            return Ok(());
        }
        let introduced = self.device_network.origins.introduced.contains(&peer);
        if introduced {
            let mut origins = self.device_network.origins.clone();
            origins.network_revoked.insert(peer);
            self.persist_network_origins(origins).await?;
            self.trusted.remove(&peer);
            if let Some(state) = self.peers.get_mut(&peer) {
                state.trust = TrustState::Unknown;
            }
        }
        if let Some(state) = self.peers.get_mut(&peer) {
            state.connection = ConnectionState::Disconnected;
            state.active_route = None;
            state.active_capabilities = None;
        }
        self.pending_app_messages.remove(&peer);
        self.registered_identities.remove(&peer);
        self.publish();
        self.transport.revoke_peer(peer).await?;
        // A network removal may receive a fresh signed invitation later; no application identity remains registered.
        self.transport.restore_peer(peer).await?;
        if introduced {
            let trust = self.trust_store.clone();
            task::spawn_blocking(move || {
                trust.upsert(TrustRecord::revoked(peer, Fingerprint::from(peer)))
            })
            .await
            .map_err(invalid)?
            .map_err(invalid)?;
        } else if let Some(key) = self.peers.get(&peer).map(|p| p.public_key) {
            self.register_peer_identity(peer, key).await?;
        }
        self.persist_peer(peer).await;
        Ok(())
    }
    pub(super) async fn leave_device_network(&mut self) {
        if let Some(error) = &self.device_network.origins_error {
            self.emit_error(RuntimeError::Store(format!(
                "无法安全退出：信任来源记录不可读：{error}"
            )));
            return;
        }
        let roster = self.device_network.roster.clone();
        self.cancel_all_reconnects().await;
        self.device_network.integrity_error = Some("正在退出设备网络".into());
        self.publish_view();
        let pending = self
            .device_network
            .pending
            .values()
            .map(|p| (p.request_id, p.peer_id))
            .collect::<Vec<_>>();
        for (id, peer) in pending {
            self.finish_network_pairing(id, peer, "rejected");
        }
        self.device_network.stop();
        if let Some(current) = &roster {
            let mut exchanges = Vec::new();
            if current.roster.owner.device_id == self.identity.device_id() {
                let mut dissolved = current.roster.clone();
                dissolved.members = vec![dissolved.owner.clone()];
                dissolved.epoch = dissolved.epoch.saturating_add(1);
                if let Ok(signed) = SignedNetworkRoster::signed(dissolved, &self.identity)
                    && let Ok(payload) = self.sign_network(NetworkOperation::Roster(signed))
                {
                    for member in &current.roster.members {
                        if member.device_id != self.identity.device_id()
                            && !self.blocked.contains(&member.device_id)
                        {
                            exchanges.push(self.transport.network_exchange(
                                member.device_id,
                                member.endpoint.clone(),
                                payload.clone(),
                            ));
                        }
                    }
                }
            } else if !self.blocked.contains(&current.roster.owner.device_id)
                && let Ok(payload) = self.sign_network(NetworkOperation::Leave {
                    network_id: current.roster.network_id,
                })
            {
                exchanges.push(self.transport.network_exchange(
                    current.roster.owner.device_id,
                    current.roster.owner.endpoint.clone(),
                    payload,
                ));
            }
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                futures::future::join_all(exchanges),
            )
            .await;
        }
        let members = roster
            .as_ref()
            .map(|r| {
                r.roster
                    .members
                    .iter()
                    .map(|m| m.device_id)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| {
                self.device_network
                    .origins
                    .introduced
                    .iter()
                    .copied()
                    .collect()
            });
        let mut cleanup_failed = false;
        for peer in members {
            if let Err(e) = self.remove_inherited_member(peer).await {
                self.device_network.integrity_error = Some(e.to_string());
                self.emit_error(e);
                cleanup_failed = true;
            }
        }
        if cleanup_failed {
            self.publish_view();
            return;
        }
        let store = self.persistence.clone();
        match task::spawn_blocking(move || store.set_device_network(None)).await {
            Ok(Ok(())) => {
                self.device_network.roster = None;
                self.device_network.integrity_error = None;
                self.device_network.last_sync = 0;
                self.publish();
            }
            Ok(Err(e)) => {
                self.device_network.integrity_error = Some(e.to_string());
                self.emit_error(e);
                self.publish_view();
            }
            Err(e) => {
                self.device_network.integrity_error = Some(e.to_string());
                self.emit_error(invalid(e));
                self.publish_view();
            }
        }
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;

    #[test]
    fn repair_permission_expires_without_waiting_for_cleanup_tick() {
        let peer = LocalIdentity::from_bytes(&[99; 32]).unwrap().device_id();
        let other = LocalIdentity::from_bytes(&[100; 32]).unwrap().device_id();
        let mut state = DeviceNetworkState::default();
        let blocked = BTreeSet::from([peer]);
        assert!(!state.can_receive_repair_invitation(peer));
        state.pairing_recovery.insert(
            peer,
            tokio::time::Instant::now() + std::time::Duration::from_secs(120),
        );
        assert!(state.can_receive_repair_invitation(peer));
        assert!(!state.can_receive_repair_invitation(other));
        assert_eq!(state.view(&blocked).pairing_recovery_peer_ids, vec![peer]);
        state
            .pairing_recovery
            .insert(peer, tokio::time::Instant::now());
        assert!(!state.can_receive_repair_invitation(peer));
        assert!(state.view(&blocked).pairing_recovery_peer_ids.is_empty());
        assert_eq!(state.view(&blocked).blocked_peer_ids, vec![peer]);
    }
}
