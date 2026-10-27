//! Live physical IPv4 networks and LAN route attestation.
#![allow(unsafe_code)]

use deskfuse_core::{ConnectionContext, InterfaceKind, LinkScope, TransportSessionId};
use deskfuse_platform::{
    LanInterface, LanWebPort, PlatformError, PlatformResult,
};
use std::{
    collections::BTreeSet,
    net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket},
    path::Path,
};
use windows_sys::Win32::{
    Foundation::NO_ERROR,
    NetworkManagement::IpHelper::{
        FreeMibTable, GetBestInterfaceEx, GetIfTable2, MIB_IF_TABLE2,
    },
    Networking::WinSock::{SOCKADDR_IN, AF_INET},
};

struct InterfaceTable(*mut MIB_IF_TABLE2);
impl Drop for InterfaceTable {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: GetIfTable2 allocated this table and it is freed exactly once.
            unsafe { FreeMibTable(self.0.cast()) };
        }
    }
}

fn physical_interface_indexes() -> Result<BTreeSet<u32>, String> {
    let mut raw = std::ptr::null_mut();
    // SAFETY: the out pointer is valid and ownership is immediately guarded.
    let status = unsafe { GetIfTable2(&mut raw) };
    if status != NO_ERROR {
        return Err(format!("GetIfTable2 failed: {status}"));
    }
    let table = InterfaceTable(raw);
    if raw.is_null() {
        return Ok(BTreeSet::new());
    }
    // SAFETY: the API returns NumEntries contiguous rows in the owned table.
    let rows = unsafe {
        std::slice::from_raw_parts((*table.0).Table.as_ptr(), (*table.0).NumEntries as usize)
    };
    Ok(rows
        .iter()
        .filter(|row| row.InterfaceAndOperStatusFlags._bitfield & 1 != 0)
        .map(|row| row.InterfaceIndex)
        .collect())
}

pub fn lan_ipv4_networks() -> Result<Vec<(Ipv4Addr, u8)>, String> {
    let physical = physical_interface_indexes()?;
    let mut networks = if_addrs::get_if_addrs()
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter_map(|interface| {
            if interface.is_loopback()
                || interface.is_link_local()
                || interface.is_p2p()
                || !interface.is_oper_up()
                || !interface
                    .index
                    .is_some_and(|index| physical.contains(&index))
            {
                return None;
            }
            let if_addrs::IfAddr::V4(address) = interface.addr else {
                return None;
            };
            let ip = address.ip;
            if ip.is_unspecified()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_multicast()
                || ip.is_broadcast()
                || address.prefixlen == 0
                || address.prefixlen > 30
            {
                return None;
            }
            Some((ip, address.prefixlen))
        })
        .collect::<Vec<_>>();
    networks.sort_unstable();
    networks.dedup();
    Ok(networks)
}

/// One adapter row: display name, interface type, and whether the physical
/// hardware bit is set. Used as route evidence, never guessed from names.
struct AdapterRow {
    index: u32,
    name: String,
    if_type: u32,
    physical: bool,
}

fn adapters() -> Result<Vec<AdapterRow>, String> {
    let mut raw = std::ptr::null_mut();
    // SAFETY: the out pointer is valid and ownership is immediately guarded.
    let status = unsafe { GetIfTable2(&mut raw) };
    if status != NO_ERROR {
        return Err(format!("GetIfTable2 failed: {status}"));
    }
    let table = InterfaceTable(raw);
    if raw.is_null() {
        return Ok(Vec::new());
    }
    // SAFETY: the API returns NumEntries contiguous rows in the owned table.
    let rows = unsafe {
        std::slice::from_raw_parts((*table.0).Table.as_ptr(), (*table.0).NumEntries as usize)
    };
    Ok(rows
        .iter()
        .map(|row| AdapterRow {
            index: row.InterfaceIndex,
            name: String::from_utf16_lossy(&row.Alias)
                .trim_end_matches('\0')
                .to_owned(),
            if_type: row.Type,
            physical: row.InterfaceAndOperStatusFlags._bitfield & 1 != 0,
        })
        .collect())
}

/// `IF_TYPE_*` values that identify a physical LAN port. Tunnel, PPP and
/// virtual adapters are deliberately absent: they are not on-link evidence.
const IF_TYPE_ETHERNET_CSMACD: u32 = 6;
const IF_TYPE_IEEE80211: u32 = 71;

fn interface_kind(if_type: u32) -> InterfaceKind {
    match if_type {
        IF_TYPE_IEEE80211 => InterfaceKind::WiFi,
        IF_TYPE_ETHERNET_CSMACD => InterfaceKind::Ethernet,
        _ => InterfaceKind::Other,
    }
}

fn best_interface_index(remote: Ipv4Addr) -> Result<u32, String> {
    let mut destination = SOCKADDR_IN::default();
    destination.sin_family = AF_INET;
    destination.sin_addr.S_un.S_addr = u32::from_ne_bytes(remote.octets());
    let mut index = 0_u32;
    // SAFETY: destination is an initialized IPv4 sockaddr and index is writable.
    let status = unsafe {
        GetBestInterfaceEx(
            std::ptr::from_ref(&destination).cast(),
            std::ptr::from_mut(&mut index),
        )
    };
    if status != 0 {
        return Err(format!("GetBestInterfaceEx failed: {status}"));
    }
    Ok(index)
}

/// IPv4 addresses announced by one interface, with their prefix lengths.
fn interface_ipv4(index: u32) -> Vec<(Ipv4Addr, u8)> {
    if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|interface| {
            if interface.index != Some(index) || interface.is_loopback() {
                return None;
            }
            let if_addrs::IfAddr::V4(address) = interface.addr else {
                return None;
            };
            Some((address.ip, address.prefixlen))
        })
        .collect()
}

fn on_link(network: Ipv4Addr, prefix_len: u8, remote: Ipv4Addr) -> bool {
    if prefix_len == 0 || prefix_len > 32 {
        return false;
    }
    let mask = u32::MAX.checked_shl(32 - u32::from(prefix_len)).unwrap_or(0);
    (u32::from(network) & mask) == (u32::from(remote) & mask)
}

/// Attest one input route with kernel evidence: the adapter the kernel routes
/// the peer through must own the local address and be an on-link physical port.
pub fn transport_context(
    session_id: TransportSessionId,
    local_addr: SocketAddr,
    remote_addr: SocketAddr,
) -> Result<ConnectionContext, String> {
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
        return Ok(context);
    }
    let (IpAddr::V4(local), IpAddr::V4(remote)) = (local_addr.ip(), remote_addr.ip()) else {
        return Ok(context);
    };
    let index = best_interface_index(remote)?;
    let Some(adapter) = adapters()?.into_iter().find(|adapter| adapter.index == index) else {
        return Ok(context);
    };
    if !adapter.physical {
        return Ok(context);
    }
    let addresses = interface_ipv4(index);
    if !addresses.iter().any(|(address, _)| *address == local) {
        return Ok(context);
    }
    if !addresses
        .iter()
        .any(|(address, prefix_len)| on_link(*address, *prefix_len, remote))
    {
        return Ok(context);
    }
    let kind = interface_kind(adapter.if_type);
    context.interface_name = Some(adapter.name);
    context.interface_kind = kind;
    if matches!(kind, InterfaceKind::WiFi | InterfaceKind::Ethernet) {
        context.scope = LinkScope::Lan;
    }
    Ok(context)
}

/// Bind one UDP socket per physical interface address so the input transport can
/// dial a peer from a host-verified local address on the same LAN.
pub fn transport_listener_sockets(port: u16) -> Result<Vec<(String, UdpSocket)>, String> {
    let physical = physical_interface_indexes()?;
    let mut sockets = Vec::new();
    for interface in if_addrs::get_if_addrs().map_err(|error| error.to_string())? {
        if interface.is_loopback() || interface.is_p2p() || !interface.is_oper_up() {
            continue;
        }
        // Tunnel and virtual adapters are never LAN input paths.
        if !interface
            .index
            .is_some_and(|index| physical.contains(&index))
        {
            continue;
        }
        let if_addrs::IfAddr::V4(address) = interface.addr else {
            continue;
        };
        let ip = address.ip;
        if ip.is_unspecified() || ip.is_loopback() || ip.is_link_local() || ip.is_multicast() {
            continue;
        }
        let Ok(socket) = UdpSocket::bind(SocketAddr::new(IpAddr::V4(ip), port)) else {
            continue;
        };
        if socket.set_nonblocking(true).is_err() {
            continue;
        }
        sockets.push((interface.name, socket));
    }
    Ok(sockets)
}

/// Host-owned LAN adapter for the service transports. Sockets are pinned to
/// physical interface addresses; tunnel and virtual adapters are never used.
pub struct WindowsLanWebPort;
impl LanWebPort for WindowsLanWebPort {
    fn interfaces(&self) -> PlatformResult<Vec<LanInterface>> {
        Ok(lan_ipv4_networks()
            .map_err(|_| PlatformError::Unsupported {
                operation: "LAN interfaces",
            })?
            .into_iter()
            .map(|(address, prefix_len)| LanInterface {
                name: address.to_string(),
                address: format!("{address}/{prefix_len}"),
                label: address.to_string(),
            })
            .collect())
    }
    fn ipv4_addresses(&self) -> PlatformResult<Vec<String>> {
        Ok(lan_ipv4_networks()
            .map_err(|_| PlatformError::Unsupported {
                operation: "LAN addresses",
            })?
            .into_iter()
            .map(|(address, _)| address.to_string())
            .collect())
    }
    fn image_png(&self, _path: &Path, _max_pixel_size: u32) -> PlatformResult<Vec<u8>> {
        Err(PlatformError::Unsupported {
            operation: "LAN image preview",
        })
    }
    fn data_image_png(&self, _data: &[u8], _max_pixel_size: u32) -> PlatformResult<Vec<u8>> {
        Err(PlatformError::Unsupported {
            operation: "LAN image preview",
        })
    }
    fn transport_context(
        &self,
        session_id: TransportSessionId,
        local_addr: SocketAddr,
        remote_addr: SocketAddr,
    ) -> PlatformResult<ConnectionContext> {
        transport_context(session_id, local_addr, remote_addr).map_err(|_| {
            PlatformError::Unsupported {
                operation: "LAN route attestation",
            }
        })
    }
    fn transport_listener_sockets(&self, port: u16) -> PlatformResult<Vec<(String, UdpSocket)>> {
        transport_listener_sockets(port).map_err(|_| PlatformError::Unsupported {
            operation: "LAN listener sockets",
        })
    }
}
