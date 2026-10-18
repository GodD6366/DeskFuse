//! 局域网单播探测：mDNS 组播被 AP 过滤时的发现回退。
//!
//! 企业 Wi-Fi 常对 `224.0.0.251:5353` 做组播抑制，而同网段单播不受影响。
//! 此时 Bonjour 看不到对端，但 QUIC 握手可以直接到达。启动和显式刷新时，
//! 对本机物理接口所在网段（最多一个 /22）的候选地址并发做一次身份握手；
//! 命中结果与 mDNS 一样只作为发现提示：未配对设备进入发现列表，
//! 已信任设备补充一条可拨号路由。握手不授予任何信任。
use super::*;
use device_network::NetworkResult;
use std::net::Ipv4Addr;
use std::time::Duration;

/// 标准控制端口，与手动输入 IP 配对使用的端口一致。
const LAN_PROBE_PORT: u16 = 42424;
/// 单个候选的握手上限。无监听的主机会在此超时后放弃。
const LAN_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
/// 同时进行的握手数。/22 网段约 16 轮、最坏约 32 秒完成。
const LAN_PROBE_CONCURRENCY: usize = 64;
/// 最大探测范围。更大的网段（如 /16）只探测本机所在的 /22，
/// 避免在大型企业网里做无界扫描。
const LAN_PROBE_MIN_PREFIX: u8 = 22;

/// 查询本机物理局域网接口的 IPv4 地址与前缀长度。每次扫描时实时调用。
pub type LanNetworks = Arc<dyn Fn() -> Vec<(Ipv4Addr, u8)> + Send + Sync>;

/// 由本机网段推导单播探测候选地址：排除网络号、广播地址、本机地址和
/// `exclude` 中的地址；回环、链路本地（169.254/16）和 /31、/32 网段不探测。
pub(crate) fn lan_candidates(
    networks: &[(Ipv4Addr, u8)],
    exclude: &BTreeSet<Ipv4Addr>,
) -> Vec<Ipv4Addr> {
    let locals = networks
        .iter()
        .map(|(address, _)| *address)
        .collect::<BTreeSet<_>>();
    let mut candidates = BTreeSet::new();
    for &(address, prefix) in networks {
        if prefix == 0
            || prefix > 30
            || address.is_unspecified()
            || address.is_loopback()
            || address.is_link_local()
            || address.is_multicast()
            || address.is_broadcast()
        {
            continue;
        }
        let prefix = prefix.max(LAN_PROBE_MIN_PREFIX);
        let mask = u32::MAX << (32 - u32::from(prefix));
        let network = u32::from(address) & mask;
        let broadcast = network | !mask;
        for host in (network + 1)..broadcast {
            let host = Ipv4Addr::from(host);
            if !locals.contains(&host) && !exclude.contains(&host) {
                candidates.insert(host);
            }
        }
    }
    candidates.into_iter().collect()
}

impl Actor {
    /// 在后台启动一轮局域网单播探测。已有一轮在进行时直接返回，
    /// 不会叠加扫描。
    pub(super) fn start_lan_probe(&mut self) {
        if self
            .lan_probe
            .as_ref()
            .is_some_and(|task| !task.is_finished())
        {
            return;
        }
        // 已连接设备的当前地址无需再探测。
        let exclude = self
            .peers
            .values()
            .filter(|peer| peer.connection == ConnectionState::Connected)
            .filter_map(|peer| peer.active_route.and_then(|id| peer.routes.get(&id)))
            .filter_map(|route| route.endpoint.host.parse::<Ipv4Addr>().ok())
            .collect::<BTreeSet<_>>();
        let candidates = lan_candidates(&(self.lan_networks)(), &exclude);
        if candidates.is_empty() {
            return;
        }
        let transport = self.transport.clone();
        let tx = self.network_result_tx.clone();
        let events = self.event_tx.clone();
        let count = candidates.len();
        let _ = events.send(RuntimeEvent::BackgroundDiagnostic {
            stage: "lan_probe_started",
            detail: format!("candidates={count} excluded_connected={} concurrency={LAN_PROBE_CONCURRENCY} timeout_ms={} reason=startup_or_explicit_refresh", exclude.len(), LAN_PROBE_TIMEOUT.as_millis()),
        });
        self.lan_probe = Some(task::spawn(async move {
            let started = std::time::Instant::now();
            let (hits, failures, timeouts) = stream::iter(candidates)
                .map(|address| {
                    let transport = transport.clone();
                    let tx = tx.clone();
                    async move {
                        let Ok(endpoint) = Endpoint::new(address.to_string(), LAN_PROBE_PORT)
                        else {
                            return (0usize, 1usize, 0usize);
                        };
                        let probe = transport.probe_endpoint(endpoint.clone());
                        // Empty addresses are expected: record one summary, not one log per timeout.
                        match tokio::time::timeout(LAN_PROBE_TIMEOUT, probe).await {
                            Ok(Ok(hello)) => {
                                let _ = tx.send(NetworkResult::LanProbe { endpoint, hello }).await;
                                (1, 0, 0)
                            }
                            Ok(Err(_)) => (0, 1, 0),
                            Err(_) => (0, 0, 1),
                        }
                    }
                })
                .buffer_unordered(LAN_PROBE_CONCURRENCY)
                .fold(
                    (0, 0, 0),
                    |(h, f, t), (hit, failed, timed_out)| async move {
                        (h + hit, f + failed, t + timed_out)
                    },
                )
                .await;
            let _ = events.send(RuntimeEvent::BackgroundDiagnostic {
                stage: "lan_probe_completed",
                detail: format!("candidates={count} hits={hits} failures={failures} timeouts={timeouts} elapsed_ms={}", started.elapsed().as_millis()),
            });
        }));
    }

    pub(super) fn stop_lan_probe(&mut self) {
        if let Some(task) = self.lan_probe.take() {
            if !task.is_finished() {
                let _ = self.event_tx.send(RuntimeEvent::BackgroundDiagnostic {
                    stage: "lan_probe_cancelled",
                    detail: "reason=runtime_stopped".to_owned(),
                });
            }
            task.abort();
        }
    }

    /// 把一次握手命中合并进发现状态。与 mDNS 提示走相同的信任边界：
    /// 已知设备必须公钥一致，未知设备只成为待配对提示。
    pub(super) async fn handle_lan_probe(&mut self, endpoint: Endpoint, hello: Hello) {
        if verify_hello(&hello).is_err() {
            return;
        }
        let peer_id = core_device_id_from_bytes(&hello.device_id);
        if peer_id == core_device_id(self.identity.device_id()) {
            return;
        }
        let _ = self.event_tx.send(RuntimeEvent::BackgroundDiagnostic {
            stage: "lan_probe_peer_found",
            detail: format!(
                "peer={peer_id} endpoint={}:{} known={}",
                endpoint.host,
                endpoint.port,
                self.peers.contains_key(&peer_id)
            ),
        });
        let capabilities = Capabilities::from_bits(hello.capabilities);
        let route = RouteCandidate::new(
            endpoint.clone(),
            RouteSource::Manual,
            RouteHealth::Unknown,
            capabilities,
        );
        self.observe_peer_discovery(peer_id);
        if let Some(peer) = self.peers.get_mut(&peer_id) {
            if peer.public_key.as_slice() != hello.public_key.as_slice() {
                self.emit_error(RuntimeError::Protocol(
                    "局域网探测到的设备公钥与已知设备不一致".to_owned(),
                ));
                return;
            }
            if !peer.routes.values().any(|known| known.endpoint == endpoint) {
                peer.routes.insert(route.id, route);
            }
            self.collapse_peer_routes(peer_id);
            self.reselect(peer_id);
            self.publish();
            self.persist_peer(peer_id).await;
            let should_connect = self.peers.get(&peer_id).is_some_and(|peer| {
                peer.trust == TrustState::Trusted
                    && matches!(
                        peer.connection,
                        ConnectionState::Disconnected | ConnectionState::Failed
                    )
                    && peer.preferred_route.is_some()
            });
            if should_connect {
                self.reconnect_after_discovery(peer_id).await;
            }
            return;
        }
        let hint = self.hints.entry(peer_id).or_insert_with(|| PeerHintState {
            display_name: hello.display_name.clone(),
            capabilities,
            routes: BTreeMap::new(),
        });
        hint.display_name = hello.display_name.clone();
        hint.capabilities = capabilities;
        if !hint.routes.values().any(|known| known.endpoint == endpoint) {
            hint.routes.insert(route.id, route);
        }
        self.collapse_hint_routes(peer_id);
        let public_hint = DiscoveredPeerHint {
            peer_id,
            display_name: hello.display_name,
            capabilities,
            routes: self
                .hints
                .get(&peer_id)
                .map(|hint| hint.routes.values().cloned().collect())
                .unwrap_or_default(),
        };
        self.publish_view();
        let _ = self
            .event_tx
            .send(RuntimeEvent::DiscoveredPeerHint(public_hint));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(value: &str) -> Ipv4Addr {
        value.parse().unwrap()
    }

    #[test]
    fn slash_22_covers_peer_in_neighbouring_third_octet() {
        // 实际故障现场：两台设备都在 172.18.172.0/22，第三段不同。
        let candidates = lan_candidates(&[(ip("172.18.172.190"), 22)], &BTreeSet::new());
        assert!(candidates.contains(&ip("172.18.175.65")));
        assert!(candidates.contains(&ip("172.18.172.1")));
        assert!(!candidates.contains(&ip("172.18.172.190")), "不探测本机");
        assert!(!candidates.contains(&ip("172.18.172.0")), "不探测网络号");
        assert!(
            !candidates.contains(&ip("172.18.175.255")),
            "不探测广播地址"
        );
        assert_eq!(candidates.len(), 1021);
    }

    #[test]
    fn wide_networks_are_clamped_to_the_local_slash_22() {
        let candidates = lan_candidates(&[(ip("10.20.30.40"), 16)], &BTreeSet::new());
        assert_eq!(candidates.first(), Some(&ip("10.20.28.1")));
        assert_eq!(candidates.last(), Some(&ip("10.20.31.254")));
        assert!(!candidates.contains(&ip("10.20.0.1")));
    }

    #[test]
    fn unusable_networks_and_excluded_hosts_are_skipped() {
        let networks = [
            (ip("127.0.0.1"), 8),
            (ip("169.254.10.10"), 16),
            (ip("192.168.1.10"), 31),
            (ip("192.168.2.10"), 32),
            (ip("192.168.3.10"), 0),
        ];
        assert!(lan_candidates(&networks, &BTreeSet::new()).is_empty());

        let exclude = BTreeSet::from([ip("192.168.5.20")]);
        let candidates = lan_candidates(&[(ip("192.168.5.10"), 24)], &exclude);
        // /24 小于上限，按实际网段探测；扩到 /22 会越出本机广播域。
        // 254 个主机地址，去掉本机和排除项。
        assert_eq!(candidates.len(), 252);
        assert!(!candidates.contains(&ip("192.168.5.20")));
        assert!(!candidates.contains(&ip("192.168.4.1")));
    }

    #[test]
    fn multiple_interfaces_share_one_deduplicated_list() {
        let networks = [(ip("192.168.5.10"), 24), (ip("192.168.5.11"), 24)];
        let candidates = lan_candidates(&networks, &BTreeSet::new());
        assert_eq!(candidates.len(), 252);
        assert!(!candidates.contains(&ip("192.168.5.11")));
    }
}
