//! Host-owned LAN route and listener boundary.
//!
//! Transport never executes platform commands. A host supplies the typed LAN
//! port; absent or failed evidence is deliberately fail-closed.
use std::net::{SocketAddr, UdpSocket};
use unidesk_core::{ConnectionContext, InterfaceKind, LinkScope, TransportSessionId};
use unidesk_platform::ports::LanWebPort;

fn unknown(session_id: TransportSessionId, local_addr: SocketAddr, remote_addr: SocketAddr) -> ConnectionContext {
    ConnectionContext {
        session_id,
        local_addr,
        remote_addr,
        interface_name: None,
        interface_kind: if local_addr.ip().is_loopback() || remote_addr.ip().is_loopback() {
            InterfaceKind::Loopback
        } else {
            InterfaceKind::Other
        },
        scope: LinkScope::Unknown,
    }
}

pub(crate) fn inspect(
    port: Option<&dyn LanWebPort>,
    session_id: TransportSessionId,
    local_addr: SocketAddr,
    remote_addr: SocketAddr,
) -> ConnectionContext {
    port.and_then(|port| port.transport_context(session_id, local_addr, remote_addr).ok())
        .unwrap_or_else(|| unknown(session_id, local_addr, remote_addr))
}

pub(crate) fn listener_sockets_on(
    port: Option<&dyn LanWebPort>,
    port_number: u16,
) -> Vec<(String, UdpSocket)> {
    port.and_then(|port| port.transport_listener_sockets(port_number).ok())
        .unwrap_or_default()
}

pub(crate) fn listener_sockets(port: Option<&dyn LanWebPort>) -> Vec<(String, UdpSocket)> {
    listener_sockets_on(port, 0)
}
