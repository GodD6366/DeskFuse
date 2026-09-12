//! Fail-closed physical on-link route attestation. Discovery labels and private IPs are not proof.
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    process::Command,
};
use unidesk_core::{ConnectionContext, InterfaceKind, LinkScope, TransportSessionId};

pub(crate) fn inspect(
    session_id: TransportSessionId,
    local_addr: SocketAddr,
    remote_addr: SocketAddr,
) -> ConnectionContext {
    let mut context = ConnectionContext {
        session_id,
        local_addr,
        remote_addr,
        interface_name: None,
        interface_kind: InterfaceKind::Other,
        scope: LinkScope::Unknown,
    };
    if local_addr.ip().is_loopback() || remote_addr.ip().is_loopback() {
        context.interface_kind = InterfaceKind::Loopback;
        return context;
    }
    #[cfg(target_os = "macos")]
    {
        let Ok(route) = Command::new("/sbin/route")
            .args(["-n", "get", &remote_addr.ip().to_string()])
            .output()
        else {
            return context;
        };
        let Some((interfaces, hardware)) = interface_evidence() else {
            return context;
        };
        if route.status.success() {
            let route = String::from_utf8_lossy(&route.stdout);
            let (name, kind, scope) = classify(
                &route,
                &interfaces,
                &hardware,
                local_addr.ip(),
                remote_addr.ip(),
            );
            context.interface_name = name;
            context.interface_kind = kind;
            context.scope = scope;
        }
    }
    context
}

#[cfg(target_os = "macos")]
fn interface_evidence() -> Option<(String, String)> {
    type Evidence = Option<(std::time::Instant, String, String)>;
    static CACHE: std::sync::OnceLock<std::sync::Mutex<Evidence>> = std::sync::OnceLock::new();
    let mut cache = CACHE
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .ok()?;
    if let Some((at, interfaces, hardware)) = cache.as_ref()
        && at.elapsed() < std::time::Duration::from_millis(200)
    {
        return Some((interfaces.clone(), hardware.clone()));
    }
    let interfaces = Command::new("/sbin/ifconfig").arg("-a").output().ok()?;
    let hardware = Command::new("/usr/sbin/networksetup")
        .arg("-listallhardwareports")
        .output()
        .ok()?;
    if !interfaces.status.success() || !hardware.status.success() {
        return None;
    }
    let interfaces = String::from_utf8_lossy(&interfaces.stdout).into_owned();
    let hardware = String::from_utf8_lossy(&hardware.stdout).into_owned();
    *cache = Some((
        std::time::Instant::now(),
        interfaces.clone(),
        hardware.clone(),
    ));
    Some((interfaces, hardware))
}

pub fn classify(
    route: &str,
    interfaces: &str,
    hardware: &str,
    local: IpAddr,
    remote: IpAddr,
) -> (Option<String>, InterfaceKind, LinkScope) {
    let value = |name: &str| {
        route
            .lines()
            .find_map(|line| line.trim().strip_prefix(name).map(str::trim))
    };
    let Some(name) = value("interface:") else {
        return (None, InterfaceKind::Other, LinkScope::Unknown);
    };
    let result = |kind, scope| (Some(name.to_owned()), kind, scope);
    if local.is_loopback() || remote.is_loopback() || name.starts_with("lo") {
        return result(InterfaceKind::Loopback, LinkScope::Unknown);
    }
    if [
        "utun",
        "tun",
        "tap",
        "ppp",
        "ipsec",
        "tailscale",
        "wg",
        "awdl",
        "llw",
        "vmnet",
        "vmenet",
        "veth",
        "gif",
        "stf",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
    {
        return result(InterfaceKind::Vpn, LinkScope::Overlay);
    }
    let hardware_port = hardware.split("Hardware Port: ").find_map(|block| {
        if block
            .lines()
            .any(|line| line.trim() == format!("Device: {name}"))
        {
            block.lines().next()
        } else {
            None
        }
    });
    let kind = match hardware_port {
        Some(port) if port.contains("Wi-Fi") || port.contains("AirPort") => InterfaceKind::WiFi,
        Some(port)
            if (port.contains("Thunderbolt") || port.contains("雷雳") || port.contains("雷電")) =>
        {
            InterfaceKind::Thunderbolt
        }
        Some(port)
            if port.contains("Ethernet") || port.contains("LAN") || port.contains("以太网") =>
        {
            InterfaceKind::Ethernet
        }
        _ => InterfaceKind::Other,
    };
    if kind == InterfaceKind::Other
        || (name.starts_with("bridge") && kind != InterfaceKind::Thunderbolt)
    {
        return result(InterfaceKind::Other, LinkScope::Unknown);
    }
    let flags = value("flags:").unwrap_or("");
    if flags.contains("GATEWAY") || flags.contains("REJECT") || flags.contains("BLACKHOLE") {
        return result(kind, LinkScope::Public);
    }
    let section = interfaces
        .split('\n')
        .skip_while(|line| !line.starts_with(&format!("{name}:")))
        .take_while_with_header();
    if kind == InterfaceKind::Thunderbolt && name.starts_with("bridge") {
        let members = section
            .lines()
            .filter_map(|line| {
                line.trim()
                    .strip_prefix("member:")
                    .and_then(|v| v.split_whitespace().next())
            })
            .collect::<Vec<_>>();
        if members.is_empty()
            || members.iter().any(|member| {
                !hardware.split("Hardware Port: ").any(|block| {
                    block.lines().next().is_some_and(|port| {
                        port.contains("Thunderbolt")
                            || port.contains("雷雳")
                            || port.contains("雷電")
                    }) && block
                        .lines()
                        .any(|line| line.trim() == format!("Device: {member}"))
                })
            })
        {
            return result(InterfaceKind::Other, LinkScope::Unknown);
        }
    }
    let same_subnet = match (local, remote) {
        (IpAddr::V4(local), IpAddr::V4(remote)) => section.lines().any(|line| {
            let words = line.split_whitespace().collect::<Vec<_>>();
            if words.first() != Some(&"inet")
                || words.get(1).and_then(|s| s.parse::<Ipv4Addr>().ok()) != Some(local)
            {
                return false;
            }
            let Some(index) = words.iter().position(|v| *v == "netmask") else {
                return false;
            };
            let Some(mask) = words
                .get(index + 1)
                .and_then(|v| u32::from_str_radix(v.trim_start_matches("0x"), 16).ok())
            else {
                return false;
            };
            mask != 0 && (u32::from(local) & mask) == (u32::from(remote) & mask)
        }),
        (IpAddr::V6(local), IpAddr::V6(remote)) => section.lines().any(|line| {
            let words = line.split_whitespace().collect::<Vec<_>>();
            if words.first() != Some(&"inet6")
                || words
                    .get(1)
                    .and_then(|s| s.split('%').next())
                    .and_then(|s| s.parse::<std::net::Ipv6Addr>().ok())
                    != Some(local)
            {
                return false;
            }
            let Some(i) = words.iter().position(|v| *v == "prefixlen") else {
                return false;
            };
            let Some(prefix) = words
                .get(i + 1)
                .and_then(|v| v.parse::<u32>().ok())
                .filter(|p| *p > 0 && *p <= 128)
            else {
                return false;
            };
            let mask = u128::MAX << (128 - prefix);
            u128::from(local) & mask == u128::from(remote) & mask
        }),
        _ => false,
    };
    if same_subnet && local != remote && flags.contains("UP") {
        result(kind, LinkScope::Lan)
    } else {
        result(kind, LinkScope::Unknown)
    }
}
trait Section<'a>: Iterator<Item = &'a str> + Sized {
    fn take_while_with_header(self) -> String {
        let mut result = String::new();
        for (i, line) in self.enumerate() {
            if i > 0 && !line.starts_with(char::is_whitespace) {
                break;
            }
            result.push_str(line);
            result.push('\n');
        }
        result
    }
}
impl<'a, T: Iterator<Item = &'a str>> Section<'a> for T {}

/// Kernel-bound listener sockets are built only for actual physical hardware ports.
#[cfg(target_os = "macos")]
pub(crate) fn listener_sockets() -> Vec<(String, std::net::UdpSocket)> {
    let Ok(hardware) = Command::new("/usr/sbin/networksetup")
        .arg("-listallhardwareports")
        .output()
    else {
        return vec![];
    };
    if !hardware.status.success() {
        return vec![];
    }
    let hardware = String::from_utf8_lossy(&hardware.stdout);
    let mut sockets = Vec::new();
    for interface in if_addrs::get_if_addrs().unwrap_or_default() {
        if interface.is_loopback() || interface.is_p2p || !interface.is_oper_up() {
            continue;
        }
        let Some(index) = interface.index.and_then(std::num::NonZeroU32::new) else {
            continue;
        };
        let physical = hardware.split("Hardware Port: ").any(|block| {
            block
                .lines()
                .any(|line| line.trim() == format!("Device: {}", interface.name))
                && block.lines().next().is_some_and(|port| {
                    port.contains("Wi-Fi")
                        || port.contains("AirPort")
                        || port.contains("Ethernet")
                        || port.contains("LAN")
                        || port.contains("以太网")
                        || port.contains("Thunderbolt")
                        || port.contains("雷雳")
                        || port.contains("雷電")
                })
        });
        if !physical || (!interface.name.starts_with("en") && !interface.name.starts_with("bridge"))
        {
            continue;
        }
        // Link-local IPv6 scopes are host-local identifiers; wait for an explicit
        // scoped-address negotiation rather than advertising the wrong zone.
        if matches!(interface.ip(),IpAddr::V6(ip) if ip.is_unicast_link_local()) {
            continue;
        }
        let domain = if interface.ip().is_ipv4() {
            socket2::Domain::IPV4
        } else {
            socket2::Domain::IPV6
        };
        let Ok(socket) =
            socket2::Socket::new(domain, socket2::Type::DGRAM, Some(socket2::Protocol::UDP))
        else {
            continue;
        };
        let bound = if interface.ip().is_ipv4() {
            socket.bind_device_by_index_v4(Some(index))
        } else {
            socket.bind_device_by_index_v6(Some(index))
        };
        if bound.is_err()
            || socket
                .bind(&SocketAddr::new(interface.ip(), 0).into())
                .is_err()
            || socket.set_nonblocking(true).is_err()
        {
            continue;
        }
        sockets.push((interface.name, socket.into()));
    }
    sockets
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn listener_sockets() -> Vec<(String, std::net::UdpSocket)> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn check(
        name: &str,
        flags: &str,
        hardware: &str,
    ) -> (Option<String>, InterfaceKind, LinkScope) {
        classify(
            &format!("interface: {name}\nflags: <UP,HOST,{flags}>"),
            &format!("{name}: flags=1\n\tinet 10.0.0.1 netmask 0xffffff00\n"),
            hardware,
            "10.0.0.1".parse().unwrap(),
            "10.0.0.2".parse().unwrap(),
        )
    }
    #[test]
    fn physical_onlink_required() {
        assert_eq!(
            check("en0", "LLINFO", "Hardware Port: Wi-Fi\nDevice: en0\n").2,
            LinkScope::Lan
        );
        assert_eq!(
            check("en0", "GATEWAY", "Hardware Port: Wi-Fi\nDevice: en0\n").2,
            LinkScope::Public
        );
        assert_eq!(
            check(
                "utun8",
                "LLINFO",
                "Hardware Port: Ethernet\nDevice: utun8\n"
            )
            .2,
            LinkScope::Overlay
        );
        assert_eq!(
            check(
                "bridge8",
                "LLINFO",
                "Hardware Port: Ethernet\nDevice: bridge8\n"
            )
            .2,
            LinkScope::Unknown
        );
    }
    #[test]
    fn private_address_without_subnet_not_lan() {
        assert_ne!(
            classify(
                "interface: en0\nflags: <UP,HOST>",
                "en0: flags=1\n\tinet 10.0.0.1 netmask 0xffffff00\n",
                "Hardware Port: Wi-Fi\nDevice: en0",
                "10.0.0.1".parse().unwrap(),
                "10.0.1.2".parse().unwrap()
            )
            .2,
            LinkScope::Lan
        );
    }
}
