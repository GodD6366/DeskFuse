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
    return_endpoints: BTreeMap<DeviceId, Endpoint>,
    evaluations: BTreeMap<DeviceId, Evaluation>,
    next_evaluation: BTreeMap<DeviceId, Instant>,
    /// 对方用户主动断开了与本机的连接（拨号被以挂起为由拒绝）。
    remote_suspended: BTreeSet<DeviceId>,
    /// 本机出站已连上、但对方尚未拨回本机的设备。双方都不把它当作在线，
    /// 避免出现"A 显示已连接、B 显示离线"的单边状态。
    awaiting_return: BTreeSet<DeviceId>,
}
struct Retry {
    failures: u32,
    due: Instant,
    endpoints: Vec<Endpoint>,
}
impl Recovery {
    /// Drop a peer's retry ladder once something else proved it is reachable.
    /// Sibling modules own those proofs (the session heartbeat), so they need
    /// this door instead of the raw map.
    pub(super) fn clear_retry(&mut self, peer: DeviceId) {
        self.retries.remove(&peer);
    }
    /// 是否有一轮拨号正在进行；进行中的连接状态由拨号结果决定。
    pub(super) fn dial_in_flight(&self, peer: DeviceId) -> bool {
        self.attempts.contains_key(&peer)
    }
    /// 已发布快照中是否把该设备标记为"等待对方确认"。
    pub(super) fn return_pending(&self, peer: DeviceId) -> bool {
        self.awaiting_return.contains(&peer)
    }
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
    measurements: Vec<RouteMeasurement>,
    result: Result<(RouteCandidate, TransportConnection), RuntimeError>,
}
const ROUTE_RECHECK_INTERVAL: Duration = Duration::from_secs(30);
const ROUTE_PROBE_TIMEOUT: Duration = Duration::from_secs(3);
/// A route whose last probe failed is known dead: re-probing it must not cost
/// the full handshake deadline again on every retry round.
const ROUTE_PROBE_RETRY_TIMEOUT: Duration = Duration::from_secs(1);
/// Interval cap for a peer that never comes back. Retries continue forever,
/// just far apart enough to stop crowding live input sessions.
const RETRY_MAX_DELAY: Duration = Duration::from_secs(300);

struct RouteMeasurement {
    route: RouteId,
    endpoint: Endpoint,
    kind: Option<InterfaceKind>,
    completed_at: u64,
    result: Result<Duration, String>,
}
struct ProbeBatch {
    classified: Vec<deskfuse_transport::routes::ClassifiedEndpoint>,
    rtt: BTreeMap<(String, u16), Duration>,
    measurements: Vec<RouteMeasurement>,
}
struct Evaluation {
    current: Endpoint,
    control_generation: Option<u64>,
    key: [u8; 32],
    task: task::JoinHandle<ProbeBatch>,
}

fn retry_delay(failures: u32) -> Duration {
    Duration::from_secs((1u64 << failures.min(5)).min(30))
}
/// `retry_delay` saturates at 30 s, so a peer that has been unreachable for an
/// hour still costs a full probe sweep twice a minute forever. Keep stretching
/// past that saturation point; the ramp is only ever stretched, never dropped,
/// so a lost peer is still retried indefinitely.
fn retry_backoff(failures: u32) -> Duration {
    match failures {
        0..=5 => retry_delay(failures),
        6 => Duration::from_secs(60),
        7 => Duration::from_secs(120),
        _ => RETRY_MAX_DELAY,
    }
}
/// A peer we can still address is expected back at any moment: a sleeping Mac
/// keeps its cached address and a Bonjour-visible peer is on the network. Those
/// peers stay on the 30 s ladder, because the long tail exists to stop probing
/// peers that are genuinely gone, not to make a user wait minutes for a device
/// that is sitting right there.
fn retry_backoff_for(failures: u32, present: bool) -> Duration {
    if present {
        retry_delay(failures)
    } else {
        retry_backoff(failures)
    }
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
        incoming: deskfuse_transport::ControlPeerConnected,
    ) {
        let peer = incoming.hello.device_id;
        if !self.may_reconnect(peer) {
            // 以前这里静默返回：对方连得上本机，本机却永远不回拨，排查时毫无线索。
            let reason = if self.recovery.suspended.contains(&peer) {
                "本机已主动断开该设备，忽略其连入"
            } else {
                "设备未受信任或已被阻止，忽略其连入"
            };
            if self.peers.contains_key(&peer) {
                self.recovery_event(peer, "incoming_ignored", None, Some(reason.into()));
            }
            return;
        }
        // 对方能拨入，说明它已解除对本机的挂起。
        self.recovery.remote_suspended.remove(&peer);
        if incoming.hello.verify().is_err()
            || self.peers[&peer].public_key != incoming.hello.public_key
        {
            self.emit_error(RuntimeError::Protocol(
                "incoming control identity does not match trusted peer".into(),
            ));
            return;
        }
        let nonce = incoming.hello.nonce.to_vec();
        // A process nonce and listening address survive network interruptions.
        // Deduplicate by a live outgoing session below, never by this pair:
        // a new incoming socket must be able to expedite an offline retry.
        let restarted = self
            .recovery
            .connected_nonces
            .get(&peer)
            .is_some_and(|previous| previous != &nonce);
        // "Not closed yet" is not reachability. A socket frozen by sleep or a
        // silent network change keeps looking alive until QUIC gives up, and
        // swallowing the peer's fresh authenticated connection during that
        // window is what turns a wake-up into a minutes-long outage. Only a
        // session whose own heartbeat is still clean may suppress the inbound
        // connection.
        let outgoing_healthy = self.peers[&peer].connection == ConnectionState::Connected
            && self.transport.control_session_alive(peer) != Some(false)
            && self.delivery.control_probe_clean(peer);
        if !restarted && outgoing_healthy {
            // 双向会话刚刚凑齐：立即发布，不必等下一个 tick。
            if self.refresh_return_paths() {
                self.publish();
            }
            return;
        }
        // Discovery has no process nonce. A freshly authenticated incoming
        // Hello does: invalidate only the old outgoing generation, not the new
        // incoming socket or the durable trust/identity of either device.
        if restarted
            || (self.peers[&peer].connection == ConnectionState::Connected
                && (self.transport.control_session_alive(peer) == Some(false)
                    || !self.delivery.control_probe_clean(peer)))
        {
            // Consume the restart once. Further incoming connections while the
            // replacement dial is pending must not keep cancelling that dial.
            self.recovery.connected_nonces.remove(&peer);
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

    pub(super) fn restore_reconnect_suspended(&mut self, peers: BTreeSet<DeviceId>) {
        for peer in self.recovery.suspended.difference(&peers) {
            self.transport.set_control_suspended(*peer, false);
        }
        for peer in &peers {
            self.transport.set_control_suspended(*peer, true);
        }
        self.recovery.suspended = peers;
    }
    pub(super) async fn resume_reconnect(&mut self, peer: DeviceId) {
        self.transport.set_control_suspended(peer, false);
        if self.recovery.suspended.remove(&peer) {
            self.persist_reconnect_suspended().await;
        }
    }
    /// 主动断开必须对两端同时生效：只停本端重连而继续接受对方拨入，会让对方
    /// 显示"已连接"、本机显示"离线"。传输层随即关闭并拒绝该设备的入站会话。
    pub(super) async fn suspend_reconnect(&mut self, peer: DeviceId) {
        self.transport.set_control_suspended(peer, true);
        self.cancel_reconnect(peer).await;
        if self.recovery.suspended.insert(peer) {
            self.persist_reconnect_suspended().await;
        }
    }

    /// 本机出站已连上、但对方拨入本机的会话不存在：对方尚未确认这条连接。
    fn awaiting_peer_return(&self, peer: DeviceId) -> bool {
        self.peers
            .get(&peer)
            .is_some_and(|state| state.connection == ConnectionState::Connected)
            && self.transport.inbound_control_alive(peer) == Some(false)
    }

    /// 重新计算"等待对方确认"集合并记录变化；返回是否需要重新发布快照。
    pub(super) fn refresh_return_paths(&mut self) -> bool {
        let awaiting = self
            .peers
            .keys()
            .copied()
            .filter(|peer| self.awaiting_peer_return(*peer))
            .collect::<BTreeSet<_>>();
        if awaiting == self.recovery.awaiting_return {
            return false;
        }
        for peer in awaiting.difference(&self.recovery.awaiting_return) {
            self.recovery_event(
                *peer,
                "awaiting_peer_return",
                None,
                Some("本机已连上对方，等待对方回连确认".into()),
            );
        }
        for peer in self.recovery.awaiting_return.difference(&awaiting) {
            if self
                .peers
                .get(peer)
                .is_some_and(|state| state.connection == ConnectionState::Connected)
            {
                self.recovery_event(*peer, "peer_return_confirmed", None, None);
            }
        }
        self.recovery.awaiting_return = awaiting;
        true
    }

    /// 需要在界面上单独说明的连接状态，跟随快照一起发布。
    pub(super) fn link_holds(&self) -> Vec<PeerLinkHold> {
        self.peers
            .keys()
            .filter_map(|peer| {
                let kind = if self.recovery.suspended.contains(peer) {
                    PeerLinkHoldKind::LocalSuspended
                } else if self.recovery.remote_suspended.contains(peer) {
                    PeerLinkHoldKind::RemoteSuspended
                } else if self.recovery.awaiting_return.contains(peer) {
                    PeerLinkHoldKind::AwaitingPeer
                } else {
                    return None;
                };
                Some(PeerLinkHold {
                    peer_id: *peer,
                    kind,
                })
            })
            .collect()
    }
    async fn persist_reconnect_suspended(&mut self) {
        let suspended = self.recovery.suspended.clone();
        let store = self.persistence.clone();
        match task::spawn_blocking(move || store.set_reconnect_suspended(suspended)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => self.emit_error(error),
            Err(error) => self.emit_error(RuntimeError::Store(error.to_string())),
        }
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

    /// Whether a peer is still advertised on the network. Bonjour keeps
    /// advertising a sleeping device until its record expires, so a visible
    /// peer stays on the 30 s ladder.
    ///
    /// 以前"有任何持久化地址"也算在场，而每台配对过的设备都有地址，导致长退避
    /// 永远不生效：离开很久的设备也被每 30 秒全量探测一次，挤占输入链路。
    /// 设备回来时会主动拨入本机（`handle_control_peer` 立即加速重连），
    /// Bonjour 重新出现也会重置退避，所以长退避不会让用户久等。
    fn peer_present(&self, peer: DeviceId) -> bool {
        self.recovery.discovered.contains(&peer)
    }

    fn schedule_retry(&mut self, peer: DeviceId, failed: bool) {
        if !self.may_reconnect(peer) {
            return;
        }
        let present = self.peer_present(peer);
        let retry = self.recovery.retries.entry(peer).or_insert_with(|| Retry {
            failures: 0,
            due: Instant::now() + retry_backoff_for(0, present),
            endpoints: vec![],
        });
        retry.due = Instant::now() + retry_backoff_for(retry.failures, present);
        if failed {
            retry.failures = retry.failures.saturating_add(1);
        }
        self.recovery_event(peer, "retry_scheduled", None, None);
    }

    /// Abort and join before any replacement can dial. A provider may have
    /// installed a session just before cancellation; close it only here, while
    /// this generation still exclusively owns this peer's outgoing slot.
    pub(super) async fn cancel_reconnect(&mut self, peer: DeviceId) {
        if let Some(evaluation) = self.recovery.evaluations.remove(&peer) {
            evaluation.task.abort();
            let _ = evaluation.task.await;
        }
        self.recovery.next_evaluation.remove(&peer);
        self.recovery.retries.remove(&peer);
        self.recovery.return_endpoints.remove(&peer);
        if let Some(attempt) = self.recovery.attempts.remove(&peer) {
            attempt.task.abort();
            let _ = attempt.task.await;
            let installed = attempt.installed.load(Ordering::Acquire);
            if installed != 0
                && let Err(error) = self.transport.disconnect_control(peer, installed).await
            {
                self.emit_error(error);
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
        for (_, evaluation) in std::mem::take(&mut self.recovery.evaluations) {
            evaluation.task.abort();
            let _ = evaluation.task.await;
        }
        self.recovery.next_evaluation.clear();
        for peer in self.recovery.attempts.keys().copied().collect::<Vec<_>>() {
            self.cancel_reconnect(peer).await;
        }
        self.recovery.retries.clear();
    }

    pub(super) async fn start_reconnect(&mut self, peer: DeviceId) {
        if !self.may_reconnect(peer) || self.recovery.attempts.contains_key(&peer) {
            return;
        }
        if self.peers[&peer].connection == ConnectionState::Connected {
            if self.transport.control_session_alive(peer) != Some(false) {
                return;
            }
            // Explicit reconnect/discovery can arrive before the next recovery
            // tick. A stale Connected snapshot must not swallow that request.
            if let Err(error) = self.transport.disconnect(peer).await {
                self.emit_error(error);
            }
            let state = self.peers.get_mut(&peer).expect("trusted peer");
            state.connection = ConnectionState::Disconnected;
            state.liveness = LivenessState::Offline;
            state.active_route = None;
            state.active_capabilities = None;
            self.pending_app_messages.remove(&peer);
            self.publish();
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
            let host = deskfuse_core::device_hostname(peer)
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
        let eligible = self.eligible_routes(peer);
        if eligible.is_empty() {
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
        // Only the scheduled round is thinned out. Once the backoff outgrows the
        // periodic re-check every address is known dead and none of them changed
        // since the last sweep, so probing one address still proves reachability
        // while re-probing the rest only starves a live input session elsewhere.
        // The address rotates, so a route that recovered while its siblings
        // stayed dead is not forgotten. An explicit request, a new/changed
        // address or a cleared retry sweeps everything as before. A peer that is
        // still present never reaches this point: its ladder stops at 30 s.
        let present = self.peer_present(peer);
        let retry = self.recovery.retries.get(&peer);
        let quiet = retry.is_some_and(|retry| {
            retry.due <= Instant::now()
                && retry_backoff_for(retry.failures, present) > ROUTE_RECHECK_INTERVAL
                && !eligible
                    .iter()
                    .any(|route| !retry.endpoints.contains(&route.endpoint))
        });
        let routes = if quiet && eligible.len() > 1 {
            let next = retry.map_or(0, |retry| retry.failures as usize) % eligible.len();
            vec![eligible[next].clone()]
        } else {
            eligible.clone()
        };
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
        // Record every known address, not just this round's dial set, so a
        // quiet round cannot make the other addresses look newly discovered.
        retry.endpoints = eligible.iter().map(|r| r.endpoint.clone()).collect();
        self.recovery.generation += 1;
        let generation = self.recovery.generation;
        let transport = self.transport.clone();
        let local = self.local_hello.clone();
        let tx = self.dial_result_tx.clone();
        let return_endpoint = self.recovery.return_endpoints.remove(&peer);
        let installed = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let installed_task = installed.clone();
        let dial_routes = routes.clone();
        let task = tokio::spawn(async move {
            // Resolution and probes belong to the cancellable dial task, not the
            // actor: queued revoke/route-change/shutdown must remain responsive.
            let batch = measure_routes(transport.clone(), peer, key, &dial_routes).await;
            let measurements = batch.measurements;
            let rtt = batch.rtt;
            let mut candidates =
                deskfuse_transport::routes::sort_classified(batch.classified, &rtt);
            // A return address is only a tie-break when there is no measured RTT.
            if rtt.is_empty() {
                candidates.sort_by_key(|(_, endpoint)| return_endpoint.as_ref() != Some(endpoint));
            }

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
                    Err(error @ RuntimeError::RemoteSuspended(_)) => {
                        // 线路是通的，是对方拒绝了本机：不惩罚路由，也不必再试其他路由。
                        if let Some(generation) = socket_generation {
                            let _ = transport.disconnect_control(peer, generation).await;
                        }
                        installed_task.store(0, Ordering::Release);
                        result = Err(error);
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
                    measurements,
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
        self.apply_route_measurements(peer, result.measurements);
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
                self.recovery.remote_suspended.remove(&peer);
                // The dial already measured every route; do not immediately duplicate it.
                self.recovery
                    .next_evaluation
                    .insert(peer, Instant::now() + ROUTE_RECHECK_INTERVAL);
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
                if matches!(error, RuntimeError::RemoteSuspended(_)) {
                    // 对方恢复连接时会主动拨入本机并立即触发重连，这里无需频繁重试。
                    self.recovery.remote_suspended.insert(peer);
                    if let Some(retry) = self.recovery.retries.get_mut(&peer) {
                        retry.due = Instant::now() + RETRY_MAX_DELAY;
                    }
                    self.recovery_event(
                        peer,
                        "remote_suspended",
                        None,
                        Some("对方已断开与本机的连接，需在对方设备上重新连接".into()),
                    );
                } else {
                    self.recovery_event(peer, "failed", None, Some(error.to_string()));
                }
            }
        }
        self.publish();
        self.persist_peer(peer).await;
    }

    pub(super) async fn recovery_tick(&mut self) {
        let finished = self
            .recovery
            .evaluations
            .iter()
            .filter(|(_, evaluation)| evaluation.task.is_finished())
            .map(|(peer, _)| *peer)
            .collect::<Vec<_>>();
        for peer in finished {
            self.finish_route_evaluation(peer).await;
        }
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
                    if self
                        .recovery
                        .next_evaluation
                        .get(&peer)
                        .is_none_or(|due| *due <= Instant::now())
                    {
                        self.reevaluate_connected_route(peer).await;
                    }
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
        if self.refresh_return_paths() {
            self.publish();
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

    fn probe_candidates(&self, peer: DeviceId) -> Vec<RouteCandidate> {
        let Some(state) = self.peers.get(&peer) else {
            return Vec::new();
        };
        let candidates = state
            .routes
            .values()
            .cloned()
            .map(|mut route| {
                // Only this probe snapshot may bypass failure suppression. Retain all
                // source/network policy checks and leave actor-owned health untouched.
                if route.health == RouteHealth::Unavailable {
                    route.health = RouteHealth::Unknown;
                }
                route
            })
            .collect::<Vec<_>>();
        self.policy.eligible_routes(&candidates)
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
        if self.recovery.evaluations.contains_key(&peer) {
            return;
        }
        // Probe policy-allowed backups even if a previous dial marked them unavailable.
        // Do not resurrect them for connection selection until a probe succeeds.
        let routes = self.probe_candidates(peer);
        self.recovery
            .next_evaluation
            .insert(peer, Instant::now() + ROUTE_RECHECK_INTERVAL);
        if routes.is_empty() {
            return;
        }
        let key = self.peers[&peer].public_key;
        let transport = self.transport.clone();
        let control_generation = transport.control_generation(peer);
        let task = tokio::spawn(async move { measure_routes(transport, peer, key, &routes).await });
        self.recovery.evaluations.insert(
            peer,
            Evaluation {
                current,
                control_generation,
                key,
                task,
            },
        );
    }

    fn apply_route_measurements(&mut self, peer: DeviceId, measurements: Vec<RouteMeasurement>) {
        for measurement in measurements {
            let Some(state) = self.peers.get_mut(&peer) else {
                continue;
            };
            let active = state.active_route == Some(measurement.route);
            let Some(route) = state.routes.get_mut(&measurement.route) else {
                continue;
            };
            if route.endpoint != measurement.endpoint {
                continue;
            }
            route.path_kind = measurement.kind.or(route.path_kind);
            route.last_probe_unix_ms = Some(measurement.completed_at);
            let reason = match measurement.result {
                Ok(delay) => {
                    route.rtt_ms = Some(u32::try_from(delay.as_millis()).unwrap_or(u32::MAX));
                    route.probe_error = None;
                    route.health = RouteHealth::Healthy;
                    route.last_success_unix_ms = Some(measurement.completed_at);
                    None
                }
                Err(error) => {
                    // A new probe failing is not evidence that the live control session died.
                    route.rtt_ms = None;
                    route.probe_error = Some(error.clone());
                    // failure_count remains the historical control-dial counter;
                    // a probe plus its following dial must not count twice.
                    if !active {
                        route.health = RouteHealth::Backoff;
                    }
                    Some(error)
                }
            };
            let failed = reason.is_some();
            let detail = match reason {
                Some(error) => format!("endpoint={} error={error}", measurement.endpoint),
                None => format!(
                    "endpoint={} handshake_ms={}",
                    measurement.endpoint,
                    route.rtt_ms.unwrap_or(0)
                ),
            };
            self.recovery_event(
                peer,
                if failed {
                    "route_probe_failed"
                } else {
                    "route_probe_ok"
                },
                Some(measurement.route),
                Some(detail),
            );
        }
    }

    async fn finish_route_evaluation(&mut self, peer: DeviceId) {
        let Some(evaluation) = self.recovery.evaluations.remove(&peer) else {
            return;
        };
        let Ok(batch) = evaluation.task.await else {
            self.recovery_event(
                peer,
                "route_probe_failed",
                None,
                Some("线路探测任务异常终止".into()),
            );
            return;
        };
        let current = evaluation.current;
        // Never let a stale background result alter a new session or restored trust.
        if !self.may_reconnect(peer)
            || self.recovery.attempts.contains_key(&peer)
            || self.transport.control_generation(peer) != evaluation.control_generation
            || !self.peers.get(&peer).is_some_and(|state| {
                state.connection == ConnectionState::Connected
                    && state.public_key == evaluation.key
                    && state
                        .active_route
                        .and_then(|id| state.routes.get(&id))
                        .is_some_and(|route| route.endpoint == current)
            })
        {
            return;
        }
        let allowed = self.probe_candidates(peer);
        // Discovery/policy changes invalidate the whole ranking, not just its display.
        if batch.measurements.iter().any(|m| {
            !allowed
                .iter()
                .any(|route| route.id == m.route && route.endpoint == m.endpoint)
        }) {
            return;
        }
        self.apply_route_measurements(peer, batch.measurements);
        self.publish();
        self.persist_peer(peer).await;
        let rtt = batch.rtt;
        let reachable = batch
            .classified
            .into_iter()
            .filter(|item| rtt.contains_key(&(item.endpoint.host.clone(), item.endpoint.port)))
            .collect();
        let ranked = deskfuse_transport::routes::sort_classified(reachable, &rtt);
        let Some((_, best)) = ranked.first() else {
            return;
        };
        if best.host == current.host && best.port == current.port {
            return;
        }
        let current_rtt = rtt.get(&(current.host.clone(), current.port)).copied();
        let best_rtt = rtt.get(&(best.host.clone(), best.port)).copied();
        // A failed independent probe does not prove the live route is slower.
        // Keep the healthy session until both routes can be compared.
        let switch = match (best_rtt, current_rtt) {
            (Some(best_delay), Some(current_delay)) => {
                best_delay + Duration::from_millis(5) < current_delay
            }
            _ => false,
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
        // Sleep is not a failure. Everything learned before the machine slept
        // is stale, so every peer starts a fresh round at the first ladder step
        // instead of inheriting the pre-sleep interval. One attempt that races
        // the peer's own wake-up must not cost minutes.
        self.recovery.retries.clear();
        for peer in self.peers.keys().copied().collect::<Vec<_>>() {
            self.start_reconnect(peer).await;
        }
    }
}

async fn measure_routes(
    transport: Arc<dyn TransportProvider>,
    peer: DeviceId,
    key: [u8; 32],
    routes: &[RouteCandidate],
) -> ProbeBatch {
    let classification = transport
        .classify_endpoints(routes.iter().map(|route| route.endpoint.clone()).collect())
        .await;
    let classification_error = classification.as_ref().err().map(ToString::to_string);
    let classified = classification.unwrap_or_default();
    // Owned futures are cancelled with the dial/evaluation; they never install sessions.
    let probes = classified.iter().map(|item| {
        let transport = transport.clone();
        async move {
            let started = Instant::now();
            // A route that already failed its last probe is known dead; giving it
            // the full handshake deadline again is what starves live input. Never
            // shorten a route that is healthy or has never been probed.
            let timeout = match routes.get(item.index) {
                Some(route) if route.probe_error.is_some() => ROUTE_PROBE_RETRY_TIMEOUT,
                _ => ROUTE_PROBE_TIMEOUT,
            };
            let result = match tokio::time::timeout(
                timeout,
                transport.probe_endpoint(item.endpoint.clone()),
            )
            .await
            {
                Err(_) => Err(format!(
                    "QUIC / Hello 身份探测超时（{} 秒）",
                    timeout.as_secs()
                )),
                Ok(Err(error)) => Err(error.to_string()),
                Ok(Ok(hello)) => {
                    if core_device_id_from_bytes(&hello.device_id) != peer
                        || hello.public_key.as_slice() != key
                    {
                        Err("探测返回的设备身份与已配对设备不一致".into())
                    } else {
                        verify_hello(&hello)
                            .map(|_| started.elapsed())
                            .map_err(|error| format!("Hello 身份验证失败：{error}"))
                    }
                }
            };
            (item, result, now_unix_ms())
        }
    });
    let mut rtt = BTreeMap::new();
    let mut measurements = Vec::new();
    for (item, result, completed_at) in futures::future::join_all(probes).await {
        if let Ok(delay) = &result {
            rtt.insert((item.endpoint.host.clone(), item.endpoint.port), *delay);
        }
        if let Some(route) = routes.get(item.index) {
            measurements.push(RouteMeasurement {
                route: route.id,
                endpoint: route.endpoint.clone(),
                kind: Some(item.kind),
                completed_at,
                result,
            });
        }
    }
    for route in routes {
        if !measurements
            .iter()
            .any(|measurement| measurement.route == route.id)
        {
            measurements.push(RouteMeasurement {
                route: route.id,
                endpoint: route.endpoint.clone(),
                kind: None,
                completed_at: now_unix_ms(),
                result: Err(classification_error
                    .clone()
                    .unwrap_or_else(|| "地址解析失败或未找到可探测的 IPv4 地址".into())),
            });
        }
    }
    ProbeBatch {
        classified,
        rtt,
        measurements,
    }
}

#[cfg(test)]
mod tests;

/// Only a live connection proves that a peer has a usable replacement route.
/// Offline duration and cache size are never reasons to discard reconnect hints.
fn prune_cached_routes(
    routes: &mut BTreeMap<RouteId, RouteCandidate>,
    active: Option<RouteId>,
) -> bool {
    let Some(active) = active.filter(|id| routes.contains_key(id)) else {
        return false;
    };
    let before = routes.len();
    routes.retain(|id, route| {
        if *id == active || route.source != RouteSource::CachedDirect {
            return true;
        }
        let failed_probe = route.probe_error.is_some()
            && route.last_probe_unix_ms.is_some_and(|probe| {
                route
                    .last_success_unix_ms
                    .is_none_or(|success| probe > success)
            });
        let failed_dials = route.health == RouteHealth::Unavailable && route.failure_count >= 3;
        // Unknown includes restored cache entries. Historical failures alone
        // must not erase an address that has not been retried this session.
        let failed = matches!(
            route.health,
            RouteHealth::Backoff | RouteHealth::Unavailable
        ) && (failed_probe || failed_dials);
        !failed
    });
    routes.len() != before
}

impl Actor {
    pub(super) async fn cleanup_cached_routes(&mut self) {
        let mut changed = Vec::new();
        for (id, peer) in &mut self.peers {
            // Background tasks hold route snapshots; let their results settle
            // before removing candidates they may still be using.
            if self.recovery.attempts.contains_key(id) || self.recovery.evaluations.contains_key(id)
            {
                continue;
            }
            if peer.connection != ConnectionState::Connected
                || self.transport.control_session_alive(*id) == Some(false)
            {
                continue;
            }
            if prune_cached_routes(&mut peer.routes, peer.active_route) {
                changed.push(*id);
            }
        }
        if changed.is_empty() {
            return;
        }
        for id in changed {
            self.reselect(id);
            self.persist_peer(id).await;
        }
        self.publish();
    }
}

#[cfg(test)]
mod cache_cleanup_tests {
    use super::*;

    fn cached(host: &str) -> RouteCandidate {
        let mut route = RouteCandidate::new(
            Endpoint::new(host, 42424).unwrap(),
            RouteSource::CachedDirect,
            RouteHealth::Backoff,
            Capabilities::NETWORK,
        );
        route.last_success_unix_ms = Some(1);
        route.last_probe_unix_ms = Some(30 * 86_400_000);
        route.probe_error = Some("timeout".into());
        route.failure_count = 10;
        route
    }

    #[test]
    fn offline_peer_keeps_all_reconnect_hints_even_after_a_month() {
        let mut routes: BTreeMap<_, _> = (1..=20)
            .map(|i| {
                let route = cached(&format!("10.0.0.{i}"));
                (route.id, route)
            })
            .collect();
        let original = routes.clone();
        assert!(!prune_cached_routes(&mut routes, None));
        assert_eq!(routes, original);
        assert!(!prune_cached_routes(&mut routes, Some(RouteId::new())));
        assert_eq!(routes, original);
    }

    #[test]
    fn live_replacement_allows_only_failed_cached_routes_to_be_removed() {
        let active = cached("10.0.0.1");
        let active_id = active.id;
        let failed = cached("10.0.0.2");
        let mut manual = cached("10.0.0.3");
        manual.source = RouteSource::Manual;
        let mut mdns = cached("10.0.0.4");
        mdns.source = RouteSource::Mdns;
        let mut restored = cached("10.0.0.5");
        restored.health = RouteHealth::Unknown;
        let mut healthy = cached("10.0.0.6");
        healthy.health = RouteHealth::Healthy;
        let mut recovered = cached("10.0.0.7");
        recovered.last_success_unix_ms = recovered.last_probe_unix_ms;
        let retained = [
            active.id,
            manual.id,
            mdns.id,
            restored.id,
            healthy.id,
            recovered.id,
        ];
        let mut routes = [active, failed, manual, mdns, restored, healthy, recovered]
            .into_iter()
            .map(|r| (r.id, r))
            .collect();
        assert!(prune_cached_routes(&mut routes, Some(active_id)));
        assert_eq!(routes.len(), retained.len());
        assert!(retained.iter().all(|id| routes.contains_key(id)));
        assert!(!prune_cached_routes(&mut routes, Some(active_id)));
    }
}
