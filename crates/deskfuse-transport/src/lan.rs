//! Host-owned LAN route and listener boundary.
//!
//! Transport never executes platform commands. A host supplies the typed LAN
//! port; absent or failed evidence is deliberately fail-closed.
use deskfuse_core::{ConnectionContext, InterfaceKind, LinkScope, TransportSessionId};
use deskfuse_platform::ports::LanWebPort;
use std::net::{SocketAddr, UdpSocket};

fn unknown(
    session_id: TransportSessionId,
    local_addr: SocketAddr,
    remote_addr: SocketAddr,
) -> ConnectionContext {
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
    inspect_with_error(port, session_id, local_addr, remote_addr).0
}

pub(crate) fn inspect_with_error(
    port: Option<&dyn LanWebPort>,
    session_id: TransportSessionId,
    local_addr: SocketAddr,
    remote_addr: SocketAddr,
) -> (ConnectionContext, Option<String>) {
    match port.map(|port| port.transport_context(session_id, local_addr, remote_addr)) {
        Some(Ok(context)) => (context, None),
        result => (
            unknown(session_id, local_addr, remote_addr),
            Some(match result {
                Some(Err(error)) => error.to_string().chars().take(320).collect(),
                _ => "LAN inspection port unavailable".into(),
            }),
        ),
    }
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
