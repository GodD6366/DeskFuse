//! Compressed remote-app traffic on the existing authenticated QUIC connection.
//! No new listener, certificate, pairing secret, or UI snapshot dispatch.
use super::*;
use std::sync::{atomic::Ordering, mpsc as sync};
use tokio::sync::{Notify, mpsc, watch};

pub const MAX_PACKET: usize = 17 * 1024 * 1024;
// A compressed key frame can be several megabytes. The old 2/3 second
// deadlines treated normal QUIC flow control as a dead peer, then reset the
// stream; the other side surfaced that reset as `sending stopped by peer`.
// Match the control connection's 30-second idle budget and let QUIC report a
// real reset immediately when the peer actually stops the stream.
const MEDIA_IO_TIMEOUT: Duration = Duration::from_secs(30);
pub struct RemoteSession {
    pub peer: DeviceId,
    connection: quinn::Connection,
    outgoing: mpsc::Sender<Vec<u8>>,
    incoming: Mutex<sync::Receiver<Vec<u8>>>,
    cancel: watch::Sender<bool>,
    closed: Arc<AtomicBool>,
    incoming_space: Arc<Notify>,
    failure: Arc<Mutex<Option<String>>>,
}
impl RemoteSession {
    fn new(
        peer: DeviceId,
        connection: quinn::Connection,
        mut send: quinn::SendStream,
        mut recv: quinn::RecvStream,
    ) -> Arc<Self> {
        // Backpressure reaches the media producer before multiple transport
        // queues can accumulate obsolete video. Never discard input edges here.
        let (outgoing, mut output) = mpsc::channel::<Vec<u8>>(1);
        let (input, incoming) = sync::sync_channel(1);
        let (cancel, mut writer_cancel) = watch::channel(false);
        let mut reader_cancel = cancel.subscribe();
        let closed = Arc::new(AtomicBool::new(false));
        let incoming_space = Arc::new(Notify::new());
        let reader_space = incoming_space.clone();
        let failure = Arc::new(Mutex::new(None));
        let writer_failure = failure.clone();
        let reader_failure = failure.clone();
        let writer_closed = closed.clone();
        let writer_stop = cancel.clone();
        tokio::spawn(async move {
            loop {
                let packet = tokio::select! {
                    _ = writer_cancel.changed() => break,
                    packet = output.recv() => match packet { Some(packet) => packet, None => break },
                };
                let write = async {
                    send.write_all(&(packet.len() as u32).to_be_bytes()).await?;
                    send.write_all(&packet).await
                };
                let result = tokio::select! {
                    _ = writer_cancel.changed() => break,
                    result = time::timeout(MEDIA_IO_TIMEOUT, write) => result,
                };
                let problem = match result {
                    Ok(Ok(())) => None,
                    Ok(Err(error)) => Some(format!("视频通道写入失败：{error}")),
                    Err(_) => Some("视频通道写入持续阻塞超过 30 秒".into()),
                };
                if let Some(problem) = problem {
                    *writer_failure.lock().unwrap_or_else(|e| e.into_inner()) = Some(problem);
                    break;
                }
            }
            let _ = send.reset(1u32.into());
            writer_closed.store(true, Ordering::Release);
            let _ = writer_stop.send(true);
        });
        let reader_closed = closed.clone();
        let reader_stop = cancel.clone();
        tokio::spawn(async move {
            'reading: loop {
                let read = async {
                    let mut length = [0u8; 4];
                    recv.read_exact(&mut length)
                        .await
                        .map_err(|e| format!("视频通道读取失败：{e}"))?;
                    let length = u32::from_be_bytes(length) as usize;
                    if length == 0 || length > MAX_PACKET {
                        return Err("视频通道消息长度无效".to_string());
                    }
                    let mut data = vec![0; length];
                    recv.read_exact(&mut data)
                        .await
                        .map_err(|e| format!("视频通道消息不完整：{e}"))?;
                    Ok(data)
                };
                let mut packet = tokio::select! {
                    _ = reader_cancel.changed() => break,
                    result = time::timeout(MEDIA_IO_TIMEOUT, read) => match result {
                        Ok(Ok(data)) => data,
                        result => {
                            let reason = match result {
                                Ok(Err(reason)) => reason,
                                _ => "视频通道连续 30 秒未收到完整消息".into(),
                            };
                            *reader_failure.lock().unwrap_or_else(|e| e.into_inner()) = Some(reason);
                            break;
                        }
                    },
                };
                // Apply backpressure at the FFI boundary. A scheduling burst is
                // not a broken connection; retain all reference frames and edges.
                let deadline = time::Instant::now() + MEDIA_IO_TIMEOUT;
                loop {
                    match input.try_send(packet) {
                        Ok(()) => break,
                        Err(sync::TrySendError::Full(value)) => packet = value,
                        Err(sync::TrySendError::Disconnected(_)) => break 'reading,
                    }
                    tokio::select! {
                        _ = reader_cancel.changed() => break 'reading,
                        _ = reader_space.notified() => {},
                        _ = time::sleep_until(deadline) => {
                            *reader_failure.lock().unwrap_or_else(|e| e.into_inner()) = Some("视频接收处理持续阻塞超过 30 秒".into());
                            break 'reading;
                        }
                    }
                }
            }
            let _ = recv.stop(1u32.into());
            reader_closed.store(true, Ordering::Release);
            let _ = reader_stop.send(true);
        });
        Arc::new(Self {
            peer,
            connection,
            outgoing,
            incoming: Mutex::new(incoming),
            cancel,
            closed,
            incoming_space,
            failure,
        })
    }
    pub fn try_send(&self, data: Vec<u8>) -> Result<(), String> {
        if data.is_empty() || data.len() > MAX_PACKET || self.is_closed() {
            return Err("远程应用通道已关闭或数据超限".into());
        }
        self.outgoing
            .try_send(data)
            .map_err(|_| "远程应用发送队列已满".into())
    }
    /// Only call from the dedicated Swift/FFI writer thread, never a Tokio task
    /// or the UI thread. The network writer has its own bounded timeout/cancel.
    pub fn send_blocking(&self, data: Vec<u8>) -> Result<(), String> {
        if data.is_empty() || data.len() > MAX_PACKET || self.is_closed() {
            return Err("远程应用通道已关闭或数据超限".into());
        }
        self.outgoing
            .blocking_send(data)
            .map_err(|_| self.closed_reason())
    }
    fn closed_reason(&self) -> String {
        self.failure
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .unwrap_or_else(|| "远程应用连接已断开".into())
    }
    pub fn receive(&self, wait_ms: u64) -> Result<Option<Vec<u8>>, String> {
        let receiver = self.incoming.lock().map_err(|_| "远程通道不可用")?;
        match receiver.recv_timeout(Duration::from_millis(wait_ms.min(250))) {
            Ok(data) => {
                self.incoming_space.notify_one();
                Ok(Some(data))
            }
            Err(sync::RecvTimeoutError::Timeout) if !self.is_closed() => Ok(None),
            _ => Err(self.closed_reason()),
        }
    }
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }
    pub fn network_stats(&self) -> (u64, u64, f64) {
        let path = self.connection.stats().path;
        (
            path.sent_packets,
            path.lost_packets,
            path.rtt.as_secs_f64() * 1000.0,
        )
    }
    pub fn remote_address(&self) -> String {
        self.connection.remote_address().to_string()
    }
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        let _ = self.cancel.send(true);
    }
}
impl Drop for RemoteSession {
    fn drop(&mut self) {
        self.close();
    }
}

struct RemoteHubState {
    enabled: bool,
    sessions: Vec<std::sync::Weak<RemoteSession>>,
}
#[derive(Clone)]
pub(super) struct RemoteHub {
    state: Arc<Mutex<RemoteHubState>>,
    tx: sync::SyncSender<Arc<RemoteSession>>,
    rx: Arc<Mutex<sync::Receiver<Arc<RemoteSession>>>>,
}
impl RemoteHub {
    pub fn new() -> Self {
        // A launcher plus several application windows can negotiate together.
        // This is still bounded; the native host enforces the same session cap.
        let (tx, rx) = sync::sync_channel(16);
        Self {
            state: Arc::new(Mutex::new(RemoteHubState {
                enabled: false,
                sessions: Vec::new(),
            })),
            tx,
            rx: Arc::new(Mutex::new(rx)),
        }
    }
    fn set_enabled(&self, enabled: bool) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.enabled = enabled;
        if !enabled {
            for session in state.sessions.drain(..).filter_map(|s| s.upgrade()) {
                session.close();
            }
            while let Some(session) = self.take() {
                session.close();
            }
        }
    }
    pub fn take(&self) -> Option<Arc<RemoteSession>> {
        self.rx.lock().ok()?.try_recv().ok()
    }
    pub async fn accept(
        &self,
        peer: DeviceId,
        connection: quinn::Connection,
        mut send: quinn::SendStream,
        mut recv: quinn::RecvStream,
    ) {
        if !self.state.lock().unwrap_or_else(|e| e.into_inner()).enabled {
            let _ = send.reset(1u32.into());
            let _ = recv.stop(1u32.into());
            return;
        }
        let ack = Envelope::new(
            [0; 16],
            Message::RemoteApplicationOpen(protocol::RemoteApplicationOpen { version: 1 }),
        );
        if !time::timeout(Duration::from_secs(2), write_frame(&mut send, &ack))
            .await
            .is_ok_and(|r| r.is_ok())
        {
            return;
        }
        let session = RemoteSession::new(peer, connection, send, recv);
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !state.enabled {
            session.close();
            return;
        }
        state.sessions.retain(|s| s.strong_count() > 0);
        state.sessions.push(Arc::downgrade(&session));
        if let Err(sync::TrySendError::Full(session) | sync::TrySendError::Disconnected(session)) =
            self.tx.try_send(session)
        {
            session.close();
        }
    }
}
impl QuinnTransport {
    pub fn set_remote_application_enabled(&self, enabled: bool) {
        self.remote_app.set_enabled(enabled);
    }
    pub fn take_remote_application(&self) -> Option<Arc<RemoteSession>> {
        self.remote_app.take()
    }
    pub async fn open_remote_application(
        &self,
        peer: DeviceId,
    ) -> Result<Arc<RemoteSession>, String> {
        if self.is_revoked(peer).unwrap_or(true) {
            return Err("设备信任已撤销".into());
        }
        let connection = self
            .sessions
            .lock()
            .map_err(|_| "连接状态不可用")?
            .get(&peer)
            .and_then(|sessions| sessions.iter().rev().find(|s| s.close_reason().is_none()))
            .cloned()
            .ok_or("请先连接已配对设备")?;
        let future = async {
            let (mut send, mut recv) = connection.open_bi().await.map_err(|e| e.to_string())?;
            let request = Envelope::new(
                [0; 16],
                Message::RemoteApplicationOpen(protocol::RemoteApplicationOpen { version: 1 }),
            );
            write_frame(&mut send, &request)
                .await
                .map_err(|e| e.to_string())?;
            let ack = read_frame(&mut recv, Duration::from_secs(5))
                .await
                .map_err(|e| e.to_string())?;
            if !matches!(ack.message, Some(Message::RemoteApplicationOpen(_))) {
                return Err("对端未支持远程应用，请更新两端 DeskFuse".into());
            }
            Ok(RemoteSession::new(peer, connection.clone(), send, recv))
        };
        time::timeout(Duration::from_secs(4), future)
            .await
            .map_err(|_| "远程应用协商超时，请更新两端 DeskFuse".to_string())?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn paired_media_stream_coexists_with_control_and_dies_on_revocation() {
        let identity_a = Arc::new(LocalIdentity::from_bytes(&[81; 32]).unwrap());
        let identity_b = Arc::new(LocalIdentity::from_bytes(&[82; 32]).unwrap());
        let caps = Capabilities::NETWORK | Capabilities::REMOTE_APPLICATION;
        let hello_a = PeerHello::signed(&identity_a, "a", caps, [81; 32]).unwrap();
        let hello_b = PeerHello::signed(&identity_b, "b", caps, [82; 32]).unwrap();
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let addr = SocketAddr::from(([127, 0, 0, 1], 0));
        let config = QuinnTransportConfig::new(
            addr,
            addr,
            "localhost",
            cert.cert.der().to_vec(),
            cert.key_pair.serialize_der(),
            cert.cert.der().to_vec(),
        );
        let a = QuinnTransport::bind(hello_a.clone(), config.clone())
            .await
            .unwrap();
        let b = QuinnTransport::bind(hello_b.clone(), config).await.unwrap();
        a.configure_identity(identity_a).unwrap();
        b.configure_identity(identity_b).unwrap();
        a.register_peer(hello_b.clone()).unwrap();
        b.register_peer(hello_a.clone()).unwrap();
        let address = b.local_addr().unwrap();
        let control = a
            .connect_with_route(
                hello_b.device_id,
                Endpoint::new("127.0.0.1", address.port()).unwrap(),
            )
            .await
            .unwrap();
        a.set_remote_application_enabled(true);
        b.set_remote_application_enabled(true);
        let outbound = a.open_remote_application(hello_b.device_id).await.unwrap();
        let inbound = time::timeout(Duration::from_secs(2), async {
            loop {
                if let Some(session) = b.take_remote_application() {
                    break session;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        outbound.try_send(vec![42; 65536]).unwrap();
        assert_eq!(inbound.receive(250).unwrap().unwrap(), vec![42; 65536]);
        inbound.try_send(b"input".to_vec()).unwrap();
        assert_eq!(outbound.receive(250).unwrap().unwrap(), b"input");
        assert!(
            control.ping().await.is_ok(),
            "media stream must not occupy the control dispatcher"
        );
        assert!(outbound.try_send(vec![0; MAX_PACKET + 1]).is_err());
        b.set_remote_application_enabled(false);
        assert!(
            inbound.is_closed(),
            "disabling must close existing media sessions"
        );
        assert!(
            control.ping().await.is_ok(),
            "disabling media must preserve the device connection"
        );
        assert!(
            a.open_remote_application(hello_b.device_id).await.is_err(),
            "disabled hosts must reject new media streams"
        );
        b.set_remote_application_enabled(true);
        let outbound = a.open_remote_application(hello_b.device_id).await.unwrap();
        b.revoke_peer(hello_a.device_id).await.unwrap();
        time::sleep(Duration::from_millis(50)).await;
        assert!(outbound.receive(250).is_err());
        a.shutdown().await.unwrap();
        b.shutdown().await.unwrap();
    }
}
