//! RDP is opaque TLS/NLA ciphertext. Only the single-use launch ticket is
//! interpreted here; there is no listener or generic remote TCP destination.
use super::*;
use std::{collections::BTreeMap, sync::mpsc as sync, time::Instant};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::watch,
};

pub const RDP_CHUNK_BYTES: usize = 64 * 1024;
const TICKET_TTL: Duration = Duration::from_secs(30);
const MAX_TICKETS: usize = 128;
const MAX_TUNNELS: usize = 16;

#[derive(Default)]
struct Tickets(BTreeMap<Vec<u8>, (DeviceId, Instant)>);
impl Tickets {
    fn issue(&mut self, peer: DeviceId, now: Instant) -> Result<Vec<u8>, String> {
        self.0.retain(|_, (_, expires)| *expires > now);
        if self.0.len() >= MAX_TICKETS {
            return Err("远程应用启动请求过多，请稍后重试".into());
        }
        let ticket = [
            uuid::Uuid::new_v4().as_bytes().as_slice(),
            uuid::Uuid::new_v4().as_bytes().as_slice(),
        ]
        .concat();
        self.0.insert(ticket.clone(), (peer, now + TICKET_TTL));
        Ok(ticket)
    }
    fn consume(&mut self, ticket: &[u8], peer: DeviceId, now: Instant) -> bool {
        self.0.retain(|_, (_, expires)| *expires > now);
        if ticket.len() != 32 || !self.0.get(ticket).is_some_and(|(owner, _)| *owner == peer) {
            return false;
        }
        self.0.remove(ticket).is_some()
    }
}

#[derive(Default)]
struct HubState {
    enabled: bool,
    tickets: Tickets,
    active: Vec<watch::Sender<bool>>,
}
#[derive(Clone)]
pub(super) struct RdpHub {
    state: Arc<Mutex<HubState>>,
    tx: sync::SyncSender<RdpTunnel>,
    rx: Arc<Mutex<sync::Receiver<RdpTunnel>>>,
}
impl RdpHub {
    pub fn new() -> Self {
        let (tx, rx) = sync::sync_channel(MAX_TUNNELS);
        Self {
            state: Arc::default(),
            tx,
            rx: Arc::new(Mutex::new(rx)),
        }
    }
    pub fn set_enabled(&self, enabled: bool) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.enabled = enabled;
        if !enabled {
            state.tickets.0.clear();
            for cancel in state.active.drain(..) {
                let _ = cancel.send(true);
            }
            while self.take().is_some() {}
        }
    }
    fn issue(&self, peer: DeviceId) -> Result<Vec<u8>, String> {
        let mut state = self.state.lock().map_err(|_| "RDP 隧道状态不可用")?;
        if !state.enabled {
            return Err("远程应用已关闭".into());
        }
        state.tickets.issue(peer, Instant::now())
    }
    fn track(state: &mut HubState) -> Result<watch::Receiver<bool>, String> {
        state.active.retain(|sender| sender.receiver_count() > 0);
        if !state.enabled || state.active.len() >= MAX_TUNNELS {
            return Err("远程应用已关闭或连接数已达上限".into());
        }
        let (tx, rx) = watch::channel(false);
        state.active.push(tx);
        Ok(rx)
    }
    pub fn take(&self) -> Option<RdpTunnel> {
        self.rx.lock().ok()?.try_recv().ok()
    }
    pub async fn accept(
        &self,
        peer: DeviceId,
        ticket: Vec<u8>,
        mut send: quinn::SendStream,
        mut recv: quinn::RecvStream,
    ) {
        let cancel = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if !state.enabled || !state.tickets.consume(&ticket, peer, Instant::now()) {
                None
            } else {
                Self::track(&mut state).ok()
            }
        };
        let Some(cancel) = cancel else {
            let _ = send.reset(1u32.into());
            let _ = recv.stop(1u32.into());
            return;
        };
        let ack = Envelope::new(
            [0; 16],
            Message::RemoteRdpTunnel(protocol::RemoteRdpTunnel {
                version: 1,
                ticket: vec![],
            }),
        );
        if time::timeout(Duration::from_secs(5), write_frame(&mut send, &ack))
            .await
            .is_ok_and(|r| r.is_ok())
        {
            // Dropping a rejected pending tunnel resets its QUIC streams.
            let _ = self.tx.try_send(RdpTunnel {
                peer,
                send,
                recv,
                cancel,
            });
        }
    }
}

/// An accepted ticket authorizes only a connection to this host's RDP listener.
/// A bounded buffer in each direction propagates QUIC/TCP flow control to the
/// producer. EOF in either direction drops both halves, including on idle RDP.
pub struct RdpTunnel {
    pub peer: DeviceId,
    send: quinn::SendStream,
    recv: quinn::RecvStream,
    cancel: watch::Receiver<bool>,
}
impl RdpTunnel {
    pub async fn forward<S>(self, socket: S) -> Result<(u64, u64), String>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let RdpTunnel {
            peer: _,
            mut send,
            mut recv,
            mut cancel,
        } = self;
        if *cancel.borrow() {
            return Err("RDP 隧道授权已撤销".into());
        }
        let result = pump(socket, &mut send, &mut recv, &mut cancel).await;
        let _ = send.reset(1u32.into());
        let _ = recv.stop(1u32.into());
        result
    }
}

/// Copies both directions with a bounded per-read buffer. Awaiting a full QUIC
/// send stream stops reads from the socket, so TCP/QUIC flow control reaches
/// the RDP client instead of growing memory; EOF or cancellation stops both.
async fn pump<S, W, R>(
    socket: S,
    send: &mut W,
    recv: &mut R,
    cancel: &mut watch::Receiver<bool>,
) -> Result<(u64, u64), String>
where
    S: AsyncRead + AsyncWrite + Unpin,
    W: AsyncWrite + Unpin,
    R: AsyncRead + Unpin,
{
    let (mut read, mut write) = tokio::io::split(socket);
    let outbound = async {
        let mut buffer = vec![0; RDP_CHUNK_BYTES];
        let mut bytes = 0;
        loop {
            let count = read.read(&mut buffer).await.map_err(|e| e.to_string())?;
            if count == 0 {
                return Ok::<u64, String>(bytes);
            }
            send.write_all(&buffer[..count])
                .await
                .map_err(|e| e.to_string())?;
            bytes += count as u64;
        }
    };
    let inbound = async {
        let mut buffer = vec![0; RDP_CHUNK_BYTES];
        let mut bytes = 0;
        loop {
            let count = recv.read(&mut buffer).await.map_err(|e| e.to_string())?;
            if count == 0 {
                return Ok::<u64, String>(bytes);
            }
            write
                .write_all(&buffer[..count])
                .await
                .map_err(|e| e.to_string())?;
            bytes += count as u64;
        }
    };
    tokio::select! {
        _ = cancel.changed() => Err("RDP 隧道授权已撤销".into()),
        result = outbound => result.map(|n| (n, 0)),
        result = inbound => result.map(|n| (0, n)),
    }
}
impl QuinnTransport {
    pub fn issue_remote_rdp_ticket(&self, peer: DeviceId) -> Result<Vec<u8>, String> {
        if self.is_revoked(peer).unwrap_or(true) {
            return Err("设备信任已撤销".into());
        }
        self.rdp.issue(peer)
    }
    pub fn take_remote_rdp_tunnel(&self) -> Option<RdpTunnel> {
        self.rdp.take()
    }
    pub async fn open_remote_rdp_tunnel(
        &self,
        peer: DeviceId,
        ticket: Vec<u8>,
    ) -> Result<RdpTunnel, String> {
        if ticket.len() != 32 || self.is_revoked(peer).unwrap_or(true) {
            return Err("无效的 RDP 启动票据或设备信任已撤销".into());
        }
        let cancel = {
            let mut state = self.rdp.state.lock().map_err(|_| "RDP 隧道状态不可用")?;
            RdpHub::track(&mut state)?
        };
        let connection = self
            .sessions
            .lock()
            .map_err(|_| "连接状态不可用")?
            .get(&peer)
            .and_then(|s| s.iter().rev().find(|s| s.close_reason().is_none()))
            .cloned()
            .ok_or("请先连接已配对设备")?;
        time::timeout(Duration::from_secs(5), async move {
            let (mut send, mut recv) = connection.open_bi().await.map_err(|e| e.to_string())?;
            let open = Envelope::new([0; 16], Message::RemoteRdpTunnel(protocol::RemoteRdpTunnel { version: 1, ticket }));
            write_frame(&mut send, &open).await.map_err(|e| e.to_string())?;
            let ack = read_frame(&mut recv, Duration::from_secs(5)).await.map_err(|e| e.to_string())?;
            if !matches!(ack.message, Some(Message::RemoteRdpTunnel(protocol::RemoteRdpTunnel { version: 1, ref ticket })) if ticket.is_empty()) {
                return Err("RDP 启动票据被拒绝，请刷新目录并重试".into());
            }
            Ok(RdpTunnel { peer, send, recv, cancel })
        }).await.map_err(|_| "RDP 隧道协商超时".to_string())?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pump_propagates_backpressure_and_remote_eof() {
        let (mut app, socket) = tokio::io::duplex(1024);
        let (_outbound_reader, mut send) = tokio::io::duplex(1024);
        let (inbound_writer, mut recv) = tokio::io::duplex(1024);
        let (_cancel, mut cancel) = watch::channel(false);
        let mut pump = Box::pin(pump(socket, &mut send, &mut recv, &mut cancel));
        let mut flood = Box::pin(app.write_all(&[7u8; 1 << 20]));
        let outcome = tokio::select! {
            _ = &mut pump => "pump ended",
            _ = &mut flood => "flood completed",
            _ = time::sleep(Duration::from_millis(100)) => "blocked",
        };
        assert_eq!(
            outcome, "blocked",
            "an unread tunnel must stop the socket producer"
        );
        drop(flood);
        drop(inbound_writer);
        let result = time::timeout(Duration::from_secs(1), &mut pump)
            .await
            .expect("remote EOF ends the pump")
            .expect("clean EOF");
        assert_eq!(result, (0, 0));
    }

    #[tokio::test]
    async fn pump_stops_on_cancellation() {
        let (_app, socket) = tokio::io::duplex(1024);
        let (_outbound_reader, mut send) = tokio::io::duplex(1024);
        let (_inbound_writer, mut recv) = tokio::io::duplex(1024);
        let (cancel, mut cancelled) = watch::channel(false);
        let mut pump = Box::pin(pump(socket, &mut send, &mut recv, &mut cancelled));
        cancel.send(true).expect("receiver alive");
        let result = time::timeout(Duration::from_secs(1), &mut pump)
            .await
            .expect("revocation is prompt");
        assert_eq!(result.unwrap_err(), "RDP 隧道授权已撤销");
    }

    #[test]
    fn tickets_are_peer_bound_expire_and_are_single_use() {
        let a = LocalIdentity::from_bytes(&[21; 32]).unwrap().device_id();
        let b = LocalIdentity::from_bytes(&[22; 32]).unwrap().device_id();
        let now = Instant::now();
        let mut tickets = Tickets::default();
        let ticket = tickets.issue(a, now).unwrap();
        assert!(!tickets.consume(&ticket, b, now));
        assert!(tickets.consume(&ticket, a, now));
        assert!(!tickets.consume(&ticket, a, now));
        let ticket = tickets.issue(a, now).unwrap();
        assert!(!tickets.consume(&ticket, a, now + TICKET_TTL));
        for _ in 0..MAX_TICKETS {
            tickets.issue(a, now).unwrap();
        }
        assert!(tickets.issue(a, now).is_err());
        assert!(tickets.issue(a, now + TICKET_TTL).is_ok());
    }
}
