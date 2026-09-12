//! Actor-owned control-session recovery. Dial tasks never mutate peer state.
use super::*;
use std::time::Duration;
use tokio::time::Instant;

#[derive(Default)]
pub(super) struct Recovery {
    retries: BTreeMap<DeviceId, Retry>,
    attempts: BTreeMap<DeviceId, Attempt>,
    generation: u64,
    suspended: BTreeSet<DeviceId>,
    discovered: BTreeSet<DeviceId>,
    connected_nonces: BTreeMap<DeviceId, Vec<u8>>,
    incoming_instances: BTreeMap<DeviceId, (Vec<u8>, Endpoint)>,
    return_endpoints: BTreeMap<DeviceId, Endpoint>,
}
struct Retry {
    failures: u32,
    due: Instant,
    endpoints: Vec<Endpoint>,
}
struct Attempt {
    generation: u64,
    installed: Arc<std::sync::atomic::AtomicU64>,
    routes: Vec<RouteCandidate>,
    task: task::JoinHandle<()>,
}
pub(super) struct DialResult {
    peer: DeviceId,
    generation: u64,
    failed: Vec<RouteId>,
    resolved: Option<Endpoint>,
    result: Result<(RouteCandidate, TransportConnection), RuntimeError>,
}
fn retry_delay(failures: u32) -> Duration {
    Duration::from_secs((1u64 << failures.min(5)).min(30))
}
fn validate_connection(
    peer: DeviceId,
    key: [u8; 32],
    route: &RouteCandidate,
    connection: &TransportConnection,
) -> Result<(), RuntimeError> {
    if connection.peer_id != peer || connection.route_id != route.id {
        return Err(RuntimeError::Protocol(
            "transport returned mismatched peer or route".into(),
        ));
    }
    if let Some(hello) = &connection.remote_hello {
        verify_hello(hello).map_err(|e| RuntimeError::Protocol(e.to_string()))?;
        if core_device_id_from_bytes(&hello.device_id) != peer || hello.public_key.as_slice() != key
        {
            return Err(RuntimeError::Protocol(
                "transport hello identity mismatch".into(),
            ));
        }
    }
    Ok(())
}
impl Actor {
    pub(super) fn observe_peer_discovery(&mut self, peer: DeviceId) {
        if !self.may_reconnect(peer) || !self.recovery.discovered.insert(peer) {
            return;
        }
        // A real disappearance/reappearance (including the same IP) starts a
        // fresh round. Duplicate Bonjour refreshes do not reset the backoff.
        self.recovery.retries.remove(&peer);
        if let Some(state) = self.peers.get_mut(&peer) {
            for route in state.routes.values_mut() {
                if route.health == RouteHealth::Unavailable {
                    route.health = RouteHealth::Unknown;
                }
            }
        }
    }

    pub(super) fn peer_discovery_lost(&mut self, peer: DeviceId) {
        self.recovery.discovered.remove(&peer);
    }

    pub(super) async fn handle_control_peer(
        &mut self,
        incoming: unidesk_transport::ControlPeerConnected,
    ) {
        let peer = incoming.hello.device_id;
        if !self.may_reconnect(peer) {
            return;
        }
        if incoming.hello.verify().is_err()
            || self.peers[&peer].public_key != incoming.hello.public_key
        {
            self.emit_error(RuntimeError::Protocol(
                "incoming control identity does not match trusted peer".into(),
            ));
            return;
        }
        let nonce = incoming.hello.nonce.to_vec();
        let instance = (nonce.clone(), incoming.endpoint.clone());
        if self.recovery.incoming_instances.get(&peer) == Some(&instance) {
            return;
        }
        self.recovery.incoming_instances.insert(peer, instance);
        let restarted = self
            .recovery
            .connected_nonces
            .get(&peer)
            .is_some_and(|previous| previous != &nonce);
        if !restarted
            && self.peers[&peer].connection == ConnectionState::Connected
            && self.transport.control_session_alive(peer) != Some(false)
        {
            return;
        }
        // Discovery has no process nonce. A freshly authenticated incoming
        // Hello does: invalidate only the old outgoing generation, not the new
        // incoming socket or the durable trust/identity of either device.
        if restarted
            || (self.peers[&peer].connection == ConnectionState::Connected
                && self.transport.control_session_alive(peer) == Some(false))
        {
            self.cancel_reconnect(peer).await;
            if let Some(generation) = self.transport.control_generation(peer)
                && let Err(error) = self.transport.disconnect_control(peer, generation).await
            {
                self.emit_error(error);
            }
            let state = self.peers.get_mut(&peer).expect("trusted peer");
            state.connection = ConnectionState::Disconnected;
            state.liveness = LivenessState::Offline;
            state.active_route = None;
            state.active_capabilities = None;
            self.pending_app_messages.remove(&peer);
        }
        self.recovery
            .return_endpoints
            .insert(peer, incoming.endpoint.clone());
        let state = self.peers.get_mut(&peer).expect("trusted peer");
        if !state
            .routes
            .values()
            .any(|r| r.endpoint == incoming.endpoint)
        {
            // A source address is not permission to enable public networking.
            // Public return paths still have to pass the existing route policy.
            let source = match incoming.endpoint.host.parse::<std::net::Ipv4Addr>() {
                Ok(ip) if ip.is_private() || ip.is_link_local() || ip.is_loopback() => {
                    RouteSource::CachedDirect
                }
                _ => RouteSource::PublicDirect,
            };
            let route = RouteCandidate::new(
                incoming.endpoint,
                source,
                RouteHealth::Unknown,
                incoming.hello.capabilities,
            );
            state.routes.insert(route.id, route);
        }
        self.collapse_peer_routes(peer);
        self.recovery.retries.remove(&peer);
        self.recovery_event(
            peer,
            if restarted {
                "peer_restarted"
            } else {
                "peer_available"
            },
            None,
            None,
        );
        self.start_reconnect(peer).await;
    }

    pub(super) fn may_reconnect(&self, peer: DeviceId) -> bool {
        !self.recovery.suspended.contains(&peer)
            && self.trusted.contains(&peer)
            && !self.blocked.contains(&peer)
            && self
                .peers
                .get(&peer)
                .is_some_and(|p| p.trust == TrustState::Trusted)
    }

    pub(super) fn resume_reconnect(&mut self, peer: DeviceId) {
        self.recovery.suspended.remove(&peer);
    }
    pub(super) async fn suspend_reconnect(&mut self, peer: DeviceId) {
        self.cancel_reconnect(peer).await;
        self.recovery.suspended.insert(peer);
    }
    fn recovery_event(
        &self,
        peer: DeviceId,
        phase: &'static str,
        route: Option<RouteId>,
        reason: Option<String>,
    ) {
        let retry = self.recovery.retries.get(&peer);
        let _ = self.event_tx.send(RuntimeEvent::ConnectionRecovery {
            peer_id: peer,
            phase,
            route_id: route,
            attempt: retry
                .map_or(0, |r| r.failures)
                .saturating_add(u32::from(matches!(phase, "dialing" | "connected"))),
            retry_after_secs: retry.map(|r| {
                r.due
                    .saturating_duration_since(Instant::now())
                    .as_millis()
                    .div_ceil(1000) as u64
            }),
            reason,
        });
    }

    fn schedule_retry(&mut self, peer: DeviceId, failed: bool) {
        if !self.may_reconnect(peer) {
            return;
        }
        let retry = self.recovery.retries.entry(peer).or_insert_with(|| Retry {
            failures: 0,
            due: Instant::now() + retry_delay(0),
            endpoints: vec![],
        });
        retry.due = Instant::now() + retry_delay(retry.failures);
        if failed {
            retry.failures = retry.failures.saturating_add(1);
        }
        self.recovery_event(peer, "retry_scheduled", None, None);
    }

    /// Abort and join before any replacement can dial. A provider may have
    /// installed a session just before cancellation; close it only here, while
    /// this generation still exclusively owns this peer's outgoing slot.
    pub(super) async fn cancel_reconnect(&mut self, peer: DeviceId) {
        self.recovery.retries.remove(&peer);
        self.recovery.return_endpoints.remove(&peer);
        if let Some(attempt) = self.recovery.attempts.remove(&peer) {
            attempt.task.abort();
            let _ = attempt.task.await;
            let installed = attempt.installed.load(Ordering::Acquire);
            if installed != 0 {
                if let Err(error) = self.transport.disconnect_control(peer, installed).await {
                    self.emit_error(error);
                }
            }
            if let Some(state) = self.peers.get_mut(&peer) {
                state.connection = ConnectionState::Disconnected;
                state.active_route = None;
                state.active_capabilities = None;
            }
            self.pending_app_messages.remove(&peer);
        }
    }

    pub(super) async fn cancel_all_reconnects(&mut self) {
        for peer in self.recovery.attempts.keys().copied().collect::<Vec<_>>() {
            self.cancel_reconnect(peer).await;
        }
        self.recovery.retries.clear();
    }

    pub(super) async fn start_reconnect(&mut self, peer: DeviceId) {
        if !self.may_reconnect(peer)
            || self.recovery.attempts.contains_key(&peer)
            || self
                .peers
                .get(&peer)
                .is_some_and(|p| p.connection == ConnectionState::Connected)
        {
            return;
        }
        // Only a scheduled round or explicit request unsuppresses failed routes.
        // Repeated Bonjour observations cannot turn failure into a busy loop.
        if let Some(state) = self.peers.get_mut(&peer) {
            for route in state.routes.values_mut() {
                if route.health == RouteHealth::Unavailable {
                    route.health = RouteHealth::Unknown;
                }
            }
        }
        // The portable mDNS publisher owns this synthetic hostname. macOS
        // system Bonjour advertises the actual system hostname instead; its
        // resolved addresses arrive through discovery, so do not spend a DNS
        // timeout on an unpublished synthetic name before every macOS dial.
        #[cfg(not(target_os = "macos"))]
        if let Some(state) = self.peers.get_mut(&peer) {
            let host = unidesk_core::device_hostname(peer)
                .trim_end_matches('.')
                .to_owned();
            if !state.routes.values().any(|r| r.endpoint.host == host) {
                if let Some(existing) = state
                    .routes
                    .values()
                    .find(|r| !r.source.is_public())
                    .cloned()
                {
                    let route = RouteCandidate::new(
                        Endpoint {
                            host,
                            port: existing.endpoint.port,
                        },
                        RouteSource::CachedDirect,
                        RouteHealth::Unknown,
                        existing.capabilities,
                    );
                    state.routes.insert(route.id, route);
                }
            }
        }
        self.collapse_peer_routes(peer);
        self.reselect(peer);
        let routes = self.eligible_routes(peer);
        if routes.is_empty() {
            if let Some(state) = self.peers.get_mut(&peer) {
                state.connection = ConnectionState::Failed;
                state.active_route = None;
                state.active_capabilities = None;
                state.liveness = LivenessState::Offline;
            }
            self.schedule_retry(peer, true);
            self.publish();
            return;
        }
        let key = self.peers[&peer].public_key;
        if let Err(error) = self.register_peer_identity(peer, key).await {
            self.emit_error(error);
            self.schedule_retry(peer, true);
            return;
        }
        let retry = self.recovery.retries.entry(peer).or_insert_with(|| Retry {
            failures: 0,
            due: Instant::now(),
            endpoints: vec![],
        });
        retry.endpoints = routes.iter().map(|r| r.endpoint.clone()).collect();
        self.recovery.generation += 1;
        let generation = self.recovery.generation;
        let transport = self.transport.clone();
        let local = self.local_hello.clone();
        let tx = self.dial_result_tx.clone();
        let return_endpoint = self.recovery.return_endpoints.remove(&peer);
        let preference = self
            .device_network
            .route_preferences
            .get(&peer)
            .copied()
            .unwrap_or_default();
        let classified = unidesk_transport::routes::classify_endpoints(
            routes.iter().map(|r| r.endpoint.clone()).collect(),
        )
        .await;
        let rtt = measure_route_rtts(
            self.transport.clone(),
            peer,
            classified
                .iter()
                .map(|item| item.endpoint.clone())
                .collect(),
        )
        .await;
        if let Some(state) = self.peers.get_mut(&peer) {
            for item in &classified {
                let Some(route) = state.routes.get_mut(&routes[item.index].id) else {
                    continue;
                };
                route.path_kind = Some(item.kind);
                if let Some(delay) = rtt.get(&(item.endpoint.host.clone(), item.endpoint.port)) {
                    route.rtt_ms = Some(u32::try_from(delay.as_millis()).unwrap_or(u32::MAX));
                    if route.health != RouteHealth::Healthy {
                        route.health = RouteHealth::Unknown;
                    }
                }
            }
        }
        let mut candidates =
            unidesk_transport::routes::sort_classified(classified, preference, &rtt);
        // A return address is only a tie-break when Auto has no measured RTT.
        // Explicit 雷电/有线/Wi-Fi must not be overridden by the last inbound IP.
        if preference == RoutePreference::Automatic && rtt.is_empty() {
            candidates.sort_by_key(|(_, endpoint)| return_endpoint.as_ref() != Some(endpoint));
        }
        let installed = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let installed_task = installed.clone();
        let dial_routes = routes.clone();
        let task = tokio::spawn(async move {
            let mut failed = vec![];
            let mut result = Err(RuntimeError::Transport("no available control route".into()));
            let mut resolved = None;
            for (index, endpoint) in candidates {
                let route = dial_routes[index].clone();
                let mut target = route.clone();
                target.endpoint = endpoint.clone();
                // Bound even a custom provider, not just Quinn's handshake.
                let dial = tokio::time::timeout(
                    Duration::from_secs(3),
                    transport.connect(local.clone(), peer, target),
                )
                .await
                .unwrap_or_else(|_| {
                    Err(RuntimeError::Transport("control connection timeout".into()))
                });
                let socket_generation = if dial.is_ok() {
                    transport.control_generation(peer)
                } else {
                    None
                };
                installed_task.store(socket_generation.unwrap_or(0), Ordering::Release);
                match dial.and_then(|connection| {
                    validate_connection(peer, key, &route, &connection)?;
                    Ok(connection)
                }) {
                    Ok(connection) => {
                        resolved = Some(endpoint);
                        result = Ok((route, connection));
                        break;
                    }
                    Err(error) => {
                        failed.push(route.id);
                        result = Err(error);
                        // This task is the only dialler for this peer. Clean up
                        // malformed/partially installed sessions before fallback.
                        if let Some(generation) = socket_generation {
                            let _ = transport.disconnect_control(peer, generation).await;
                        }
                        installed_task.store(0, Ordering::Release);
                    }
                }
            }
            let _ = tx
                .send(DialResult {
                    peer,
                    generation,
                    failed,
                    resolved,
                    result,
                })
                .await;
        });
        self.recovery_event(peer, "dialing", routes.first().map(|r| r.id), None);
        self.recovery.attempts.insert(
            peer,
            Attempt {
                generation,
                installed,
                routes,
                task,
            },
        );
        if let Some(state) = self.peers.get_mut(&peer) {
            state.connection = ConnectionState::Connecting;
            state.active_route = None;
            state.active_capabilities = None;
        }
        self.publish();
    }

    pub(super) async fn handle_dial_result(&mut self, result: DialResult) {
        let peer = result.peer;
        if !self
            .recovery
            .attempts
            .get(&peer)
            .is_some_and(|a| a.generation == result.generation)
        {
            // Never disconnect by peer ID for an obsolete completion: a newer
            // task may already own a healthy session.
            return;
        }
        if !self.may_reconnect(peer) {
            self.cancel_reconnect(peer).await;
            return;
        }
        if let Ok((route, _)) = &result.result {
            let still_allowed = self.eligible_routes(peer).iter().any(|current| {
                current.id == route.id
                    && current.endpoint == route.endpoint
                    && current.source == route.source
            });
            if !still_allowed {
                self.cancel_reconnect(peer).await;
                self.schedule_retry(peer, false);
                self.publish();
                return;
            }
        }
        self.recovery.attempts.remove(&peer);
        for route in result.failed {
            self.mark_route_failed(peer, route);
        }
        match result.result {
            Ok((route, connection)) => {
                if let Some(hello) = &connection.remote_hello {
                    self.recovery
                        .connected_nonces
                        .insert(peer, hello.nonce.clone());
                }
                self.recovery_event(peer, "connected", Some(route.id), None);
                self.recovery.retries.remove(&peer);
                let state = self.peers.get_mut(&peer).expect("validated peer");
                if let Some(candidate) = state.routes.get_mut(&route.id) {
                    if let Some(endpoint) = result.resolved {
                        candidate.endpoint = endpoint;
                    }
                    candidate.health = RouteHealth::Healthy;
                    candidate.last_success_unix_ms = Some(now_unix_ms());
                }
                state.connection = ConnectionState::Connected;
                state.liveness = LivenessState::Online;
                state.active_route = Some(route.id);
                state.preferred_route = Some(route.id);
                state.active_capabilities = connection
                    .remote_hello
                    .as_ref()
                    .map(|h| Capabilities::from_bits(h.capabilities));
                self.drain_pending_app_messages(peer);
            }
            Err(error) => {
                let state = self.peers.get_mut(&peer).expect("validated peer");
                state.connection = ConnectionState::Failed;
                state.liveness = LivenessState::Offline;
                state.active_route = None;
                state.active_capabilities = None;
                state.preferred_route = None;
                self.pending_app_messages.remove(&peer);
                self.schedule_retry(peer, true);
                self.recovery_event(peer, "failed", None, Some(error.to_string()));
            }
        }
        self.publish();
        self.persist_peer(peer).await;
    }

    pub(super) async fn recovery_tick(&mut self) {
        let peers = self.peers.keys().copied().collect::<Vec<_>>();
        for peer in peers {
            if !self.may_reconnect(peer) {
                self.cancel_reconnect(peer).await;
                continue;
            }
            // Changes to discovery/policy invalidate in-flight route snapshots.
            if self.recovery.attempts.get(&peer).is_some_and(|attempt| {
                let current = self.eligible_routes(peer);
                attempt.routes.iter().any(|old| {
                    !current.iter().any(|r| {
                        r.id == old.id && r.endpoint == old.endpoint && r.source == old.source
                    })
                })
            }) {
                self.cancel_reconnect(peer).await;
                self.schedule_retry(peer, false);
                self.publish();
            }
            if self.recovery.attempts.contains_key(&peer) {
                continue;
            }
            if self.peers[&peer].connection == ConnectionState::Connected {
                if self.transport.control_session_alive(peer) != Some(false) {
                    continue;
                }
                if let Err(error) = self.transport.disconnect(peer).await {
                    self.emit_error(error);
                }
                let state = self.peers.get_mut(&peer).expect("known peer");
                state.connection = ConnectionState::Disconnected;
                state.liveness = LivenessState::Offline;
                state.active_route = None;
                state.active_capabilities = None;
                self.pending_app_messages.remove(&peer);
                self.recovery_event(peer, "disconnected", None, None);
                self.publish();
                self.persist_peer(peer).await;
            }
            if !self.recovery.retries.contains_key(&peer) {
                self.schedule_retry(peer, false);
            }
            if self
                .recovery
                .retries
                .get(&peer)
                .is_some_and(|r| r.due <= Instant::now())
            {
                self.start_reconnect(peer).await;
            }
        }
    }

    pub(super) async fn reconnect_after_discovery(&mut self, peer: DeviceId) {
        if !self.may_reconnect(peer) {
            return;
        }
        let fresh = self.recovery.retries.get(&peer).is_none_or(|retry| {
            self.eligible_routes(peer)
                .iter()
                .any(|r| !retry.endpoints.contains(&r.endpoint))
        });
        if fresh {
            self.start_reconnect(peer).await;
        }
    }

    pub(super) async fn reevaluate_connected_route(&mut self, peer: DeviceId) {
        if !self.may_reconnect(peer)
            || self.recovery.attempts.contains_key(&peer)
            || !self
                .peers
                .get(&peer)
                .is_some_and(|state| state.connection == ConnectionState::Connected)
        {
            return;
        }
        let Some(current) = self.peers.get(&peer).and_then(|state| {
            let id = state.active_route?;
            state.routes.get(&id).map(|route| route.endpoint.clone())
        }) else {
            return;
        };
        let preference = self
            .device_network
            .route_preferences
            .get(&peer)
            .copied()
            .unwrap_or_default();
        let routes = self.eligible_routes(peer);
        if routes.is_empty() {
            return;
        }
        let classified = unidesk_transport::routes::classify_endpoints(
            routes.iter().map(|route| route.endpoint.clone()).collect(),
        )
        .await;
        let rtt = measure_route_rtts(
            self.transport.clone(),
            peer,
            classified
                .iter()
                .map(|item| item.endpoint.clone())
                .collect(),
        )
        .await;
        if let Some(state) = self.peers.get_mut(&peer) {
            for item in &classified {
                let Some(route) = state.routes.get_mut(&routes[item.index].id) else {
                    continue;
                };
                route.path_kind = Some(item.kind);
                if let Some(delay) = rtt.get(&(item.endpoint.host.clone(), item.endpoint.port)) {
                    route.rtt_ms = Some(u32::try_from(delay.as_millis()).unwrap_or(u32::MAX));
                }
            }
        }
        let ranked = unidesk_transport::routes::sort_classified(classified, preference, &rtt);
        let Some((_, best)) = ranked.first() else {
            return;
        };
        if best.host == current.host && best.port == current.port {
            return;
        }
        let current_rtt = rtt.get(&(current.host.clone(), current.port)).copied();
        let best_rtt = rtt.get(&(best.host.clone(), best.port)).copied();
        let current_kind = self.peers.get(&peer).and_then(|state| {
            state
                .routes
                .values()
                .find(|route| route.endpoint == current)
                .and_then(|route| route.path_kind)
        });
        let best_kind = self.peers.get(&peer).and_then(|state| {
            state
                .routes
                .values()
                .find(|route| route.endpoint.host == best.host && route.endpoint.port == best.port)
                .and_then(|route| route.path_kind)
        });
        let switch = match preference {
            RoutePreference::Automatic => match (best_rtt, current_rtt) {
                (Some(best_delay), Some(current_delay)) => {
                    best_delay + Duration::from_millis(5) < current_delay
                }
                (Some(_), None) => true,
                _ => false,
            },
            RoutePreference::Thunderbolt => {
                best_kind == Some(InterfaceKind::Thunderbolt)
                    && current_kind != Some(InterfaceKind::Thunderbolt)
                    && best_rtt.is_some()
            }
            RoutePreference::Ethernet => {
                best_kind == Some(InterfaceKind::Ethernet)
                    && current_kind != Some(InterfaceKind::Ethernet)
                    && best_rtt.is_some()
            }
            RoutePreference::WiFi => {
                best_kind == Some(InterfaceKind::WiFi)
                    && current_kind != Some(InterfaceKind::WiFi)
                    && best_rtt.is_some()
            }
        };
        if switch {
            self.restart_after_active_route_change(peer, true).await;
        }
    }

    pub(super) async fn system_woke(&mut self) {
        // A healthy connection survives wake. Closed sessions are cleared first.
        self.delivery.probes_wake();
        self.delivery_tick().await;
        self.recovery_tick().await;
        self.refresh_provider(self.discovery.clone()).await;
        if let Some(provider) = self.public_provider.clone() {
            self.refresh_provider(provider).await;
        }
        for peer in self.peers.keys().copied().collect::<Vec<_>>() {
            self.start_reconnect(peer).await;
        }
    }
}

async fn measure_route_rtts(
    transport: Arc<dyn TransportProvider>,
    peer: DeviceId,
    endpoints: Vec<Endpoint>,
) -> BTreeMap<(String, u16), Duration> {
    let mut tasks = Vec::with_capacity(endpoints.len());
    for endpoint in endpoints {
        let transport = transport.clone();
        tasks.push(tokio::spawn(async move {
            let started = Instant::now();
            let result = tokio::time::timeout(
                Duration::from_millis(800),
                transport.probe_endpoint(endpoint.clone()),
            )
            .await;
            match result {
                Ok(Ok(hello)) if core_device_id_from_bytes(&hello.device_id) == peer => {
                    Some((endpoint, started.elapsed()))
                }
                _ => None,
            }
        }));
    }
    let mut measured = BTreeMap::new();
    for task in tasks {
        if let Ok(Some((endpoint, delay))) = task.await {
            measured.insert((endpoint.host, endpoint.port), delay);
        }
    }
    measured
}

#[cfg(test)]
mod tests;
