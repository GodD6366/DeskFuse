//! Resolve every advertised address and rank using this machine's actual route.
use std::{
    collections::BTreeMap,
    net::{SocketAddr, UdpSocket},
    time::Duration,
};
use unidesk_core::{Endpoint, InterfaceKind, RoutePreference, TransportSessionId};

#[derive(Clone, Debug)]
pub struct ClassifiedEndpoint {
    pub index: usize,
    pub endpoint: Endpoint,
    pub kind: InterfaceKind,
    pub input_eligible: bool,
}

/// Input favours a stable physical path over a one-off handshake measurement.
/// Explicit preferences still win; automatic input uses Thunderbolt first.
pub fn input_path_rank(preference: RoutePreference, kind: InterfaceKind) -> (u8, u8) {
    let (class, _, kind) = path_sort_key(preference, kind, true, None);
    (class, kind)
}

pub fn path_sort_key(
    preference: RoutePreference,
    kind: InterfaceKind,
    input_eligible: bool,
    rtt: Option<Duration>,
) -> (u8, u64, u8) {
    let kind_order = match kind {
        InterfaceKind::Thunderbolt => 0,
        InterfaceKind::Ethernet => 1,
        InterfaceKind::WiFi => 2,
        _ => 10,
    };
    let preferred = matches!(
        (preference, kind),
        (RoutePreference::Thunderbolt, InterfaceKind::Thunderbolt)
            | (RoutePreference::Ethernet, InterfaceKind::Ethernet)
            | (RoutePreference::WiFi, InterfaceKind::WiFi)
    );
    let reachable = input_eligible || rtt.is_some();
    let class = if reachable {
        if preference == RoutePreference::Automatic || preferred {
            0
        } else {
            1
        }
    } else {
        2
    };
    let rtt_us = rtt
        .map(|delay| u64::try_from(delay.as_micros()).unwrap_or(u64::MAX))
        .unwrap_or(u64::MAX);
    (class, rtt_us, kind_order)
}

pub async fn classify_endpoints(endpoints: Vec<Endpoint>) -> Vec<ClassifiedEndpoint> {
    let mut addresses = Vec::new();
    for (index, endpoint) in endpoints.into_iter().enumerate() {
        if let Ok(Ok(resolved)) = tokio::time::timeout(
            Duration::from_secs(2),
            tokio::net::lookup_host((endpoint.host.as_str(), endpoint.port)),
        )
        .await
        {
            for address in resolved.filter(SocketAddr::is_ipv4).take(16) {
                if !addresses.iter().any(|(_, existing)| *existing == address) {
                    addresses.push((index, address));
                }
            }
        }
    }
    super::input_executor::inspect_blocking(move || {
        addresses
            .into_iter()
            .map(|(index, address)| {
                let context = UdpSocket::bind("0.0.0.0:0").ok().and_then(|socket| {
                    socket.connect(address).ok()?;
                    Some(super::lan::inspect(
                        None,
                        TransportSessionId::new(),
                        socket.local_addr().ok()?,
                        address,
                    ))
                });
                ClassifiedEndpoint {
                    index,
                    endpoint: Endpoint {
                        host: address.ip().to_string(),
                        port: address.port(),
                    },
                    kind: context
                        .as_ref()
                        .map_or(InterfaceKind::Other, |c| c.interface_kind),
                    input_eligible: context
                        .as_ref()
                        .is_some_and(unidesk_core::ConnectionContext::input_eligible),
                }
            })
            .collect()
    })
    .await
    .unwrap_or_default()
}

pub fn sort_classified(
    classified: Vec<ClassifiedEndpoint>,
    preference: RoutePreference,
    rtt: &BTreeMap<(String, u16), Duration>,
) -> Vec<(usize, Endpoint)> {
    let mut ranked = classified
        .into_iter()
        .map(|item| {
            let key = path_sort_key(
                preference,
                item.kind,
                item.input_eligible,
                rtt.get(&(item.endpoint.host.clone(), item.endpoint.port))
                    .copied(),
            );
            (key, item.index, item.endpoint)
        })
        .collect::<Vec<_>>();
    ranked.sort_by_key(|(key, _, _)| *key);
    ranked
        .into_iter()
        .map(|(_, index, endpoint)| (index, endpoint))
        .collect()
}

pub async fn ranked_endpoints(
    endpoints: Vec<Endpoint>,
    preference: RoutePreference,
) -> Vec<(usize, Endpoint)> {
    let classified = classify_endpoints(endpoints).await;
    sort_classified(classified, preference, &BTreeMap::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_prefers_lowest_rtt() {
        let wifi = path_sort_key(
            RoutePreference::Automatic,
            InterfaceKind::WiFi,
            true,
            Some(Duration::from_millis(2)),
        );
        let thunderbolt = path_sort_key(
            RoutePreference::Automatic,
            InterfaceKind::Thunderbolt,
            true,
            Some(Duration::from_millis(8)),
        );
        assert!(wifi < thunderbolt);
    }

    #[test]
    fn automatic_without_rtt_keeps_interface_order() {
        let thunderbolt = path_sort_key(
            RoutePreference::Automatic,
            InterfaceKind::Thunderbolt,
            true,
            None,
        );
        let ethernet = path_sort_key(
            RoutePreference::Automatic,
            InterfaceKind::Ethernet,
            true,
            None,
        );
        let wifi = path_sort_key(RoutePreference::Automatic, InterfaceKind::WiFi, true, None);
        assert!(thunderbolt < ethernet);
        assert!(ethernet < wifi);
    }

    #[test]
    fn thunderbolt_preference_beats_faster_wifi() {
        let wifi = path_sort_key(
            RoutePreference::Thunderbolt,
            InterfaceKind::WiFi,
            true,
            Some(Duration::from_millis(1)),
        );
        let thunderbolt = path_sort_key(
            RoutePreference::Thunderbolt,
            InterfaceKind::Thunderbolt,
            true,
            Some(Duration::from_millis(20)),
        );
        assert!(thunderbolt < wifi);
    }
}
