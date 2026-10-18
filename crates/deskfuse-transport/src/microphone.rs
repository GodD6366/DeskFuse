//! Small, deadline-bounded audio streams on the existing paired QUIC connection.
//! Queue congestion invalidates the entire stream: stale speech is never replayed.
use super::*;
use std::sync::{atomic::Ordering, mpsc as sync};
use tokio::sync::{mpsc, watch};

const MAX_PACKET: usize = 4097;
// macOS taps can deliver ~100 ms at once: 20 audio packets plus RS8M
// repair/control packets. Keep room for a callback burst; age is bounded
// independently by AUDIO_DEADLINE on both producer and consumer paths.
pub(super) const AUDIO_QUEUE_PACKETS: usize = 64;
const AUDIO_DEADLINE: Duration = Duration::from_millis(150);
pub struct MicrophoneSession {
    pub peer: DeviceId,
    outgoing: mpsc::Sender<(std::time::Instant, Vec<u8>)>,
    incoming: Mutex<sync::Receiver<(std::time::Instant, Vec<u8>)>>,
    cancel: watch::Sender<bool>,
    closed: Arc<AtomicBool>,
}
impl MicrophoneSession {
    fn new(
        peer: DeviceId,
        mut send: quinn::SendStream,
        mut recv: quinn::RecvStream,
        origin: std::time::Instant,
    ) -> Arc<Self> {
        let (outgoing, mut output) =
            mpsc::channel::<(std::time::Instant, Vec<u8>)>(AUDIO_QUEUE_PACKETS);
        let (input, incoming) = sync::sync_channel(AUDIO_QUEUE_PACKETS);
        let (cancel, mut writer_cancel) = watch::channel(false);
        let mut reader_cancel = cancel.subscribe();
        let closed = Arc::new(AtomicBool::new(false));
        let writer_closed = closed.clone();
        let writer_stop = cancel.clone();
        // All streams use the same default priority; the small bounded audio
        // stream neither enters nor holds the keyboard/mouse dispatch queue.
        tokio::spawn(async move {
            loop {
                let (created, data) = tokio::select! {
                    _ = writer_cancel.changed() => break,
                    value = output.recv() => match value {
                        Some((created, value)) if created.elapsed() < AUDIO_DEADLINE => (created, value),
                        _ => break,
                    },
                    _ = time::sleep(Duration::from_millis(250)) => (std::time::Instant::now(), Vec::new()),
                };
                let write = async {
                    send.write_all(&(data.len() as u16).to_be_bytes()).await?;
                    send.write_all(
                        &(created.duration_since(origin).as_millis() as u64).to_be_bytes(),
                    )
                    .await?;
                    send.write_all(&data).await
                };
                let ok = tokio::select! {
                    _ = writer_cancel.changed() => false,
                    result = time::timeout(AUDIO_DEADLINE, write) => result.is_ok_and(|r| r.is_ok()),
                };
                if !ok {
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
            loop {
                let read = async {
                    let mut length = [0u8; 2];
                    recv.read_exact(&mut length).await.map_err(|_| ())?;
                    let length = u16::from_be_bytes(length) as usize;
                    if length > MAX_PACKET {
                        return Err(());
                    }
                    let mut stamp = [0u8; 8];
                    recv.read_exact(&mut stamp).await.map_err(|_| ())?;
                    // Both clocks start at acceptance, excluding time spent
                    // waiting for admission. Reject reliable-stream backlog
                    // even when write_all previously fit Quinn's buffer.
                    let sent_ms = u64::from_be_bytes(stamp);
                    if origin.elapsed().as_millis() > u128::from(sent_ms) + 250 {
                        return Err(());
                    }
                    let mut data = vec![0; length];
                    time::timeout(AUDIO_DEADLINE, recv.read_exact(&mut data))
                        .await
                        .map_err(|_| ())?
                        .map_err(|_| ())?;
                    if origin.elapsed().as_millis() > u128::from(sent_ms) + 250 {
                        return Err(());
                    }
                    Ok(data)
                };
                let data = tokio::select! {
                    _ = reader_cancel.changed() => break,
                    result = time::timeout(Duration::from_millis(1200), read) => match result {
                        Ok(Ok(data)) => data,
                        _ => break,
                    },
                };
                if data.is_empty() {
                    continue;
                }
                if !(1..=4).contains(&data[0])
                    || input.try_send((std::time::Instant::now(), data)).is_err()
                {
                    break;
                }
            }
            let _ = recv.stop(1u32.into());
            reader_closed.store(true, Ordering::Release);
            let _ = reader_stop.send(true);
        });
        Arc::new(Self {
            peer,
            outgoing,
            incoming: Mutex::new(incoming),
            cancel,
            closed,
        })
    }
    pub fn send(&self, data: Vec<u8>) -> Result<(), String> {
        if self.is_closed()
            || data.len() < 2
            || data.len() > MAX_PACKET
            || !(1..=4).contains(&data[0])
        {
            return Err("麦克风通道已关闭或音频包无效".into());
        }
        self.outgoing
            .try_send((std::time::Instant::now(), data))
            .map_err(|_| {
                self.close();
                "音频通道积压，已丢弃本次会话".into()
            })
    }
    pub fn receive(&self) -> Result<Option<Vec<u8>>, String> {
        if self.is_closed() {
            return Err("麦克风连接已断开".into());
        }
        let rx = self.incoming.lock().map_err(|_| "音频队列不可用")?;
        match rx.try_recv() {
            Ok((created, data)) if created.elapsed() < AUDIO_DEADLINE => Ok(Some(data)),
            Err(sync::TryRecvError::Empty) => Ok(None),
            _ => {
                self.close();
                Err("音频会话已过期".into())
            }
        }
    }
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        let _ = self.cancel.send(true);
    }
}
impl Drop for MicrophoneSession {
    fn drop(&mut self) {
        self.close();
    }
}

pub struct PendingMicrophone {
    pub peer: DeviceId,
    created: std::time::Instant,
    send: quinn::SendStream,
    recv: quinn::RecvStream,
}
impl PendingMicrophone {
    pub async fn respond(mut self, mut status: u32) -> Option<Arc<MicrophoneSession>> {
        if self.created.elapsed() > Duration::from_secs(2) {
            // A queued request must receive a refusal, never an empty FIN.
            status = 5;
        }
        let ack = Envelope::new(
            [0; 16],
            Message::MicrophoneOpen(protocol::MicrophoneOpen { version: 1, status }),
        );
        if !time::timeout(
            Duration::from_millis(500),
            write_frame(&mut self.send, &ack),
        )
        .await
        .is_ok_and(|r| r.is_ok())
        {
            return None;
        }
        if status != 1 {
            let _ = self.send.finish();
            return None;
        }
        Some(MicrophoneSession::new(
            self.peer,
            self.send,
            self.recv,
            std::time::Instant::now(),
        ))
    }
}
#[derive(Clone)]
pub(super) struct MicrophoneHub {
    tx: sync::SyncSender<PendingMicrophone>,
    rx: Arc<Mutex<sync::Receiver<PendingMicrophone>>>,
    // Signed Hello capabilities for the exact live connection, not discovery
    // cache: a peer may still have sessions from an older running version.
    capabilities: Arc<Mutex<BTreeMap<usize, Capabilities>>>,
}
impl MicrophoneHub {
    pub fn new() -> Self {
        let (tx, rx) = sync::sync_channel(4);
        Self {
            tx,
            rx: Arc::new(Mutex::new(rx)),
            capabilities: Arc::default(),
        }
    }
    pub fn register(&self, connection: &quinn::Connection, negotiated: Capabilities) {
        self.capabilities
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(connection.stable_id(), negotiated);
    }
    pub fn unregister(&self, connection: &quinn::Connection) {
        self.capabilities
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&connection.stable_id());
    }
    pub(super) fn supports(
        &self,
        connection: &quinn::Connection,
        capability: Capabilities,
    ) -> Option<bool> {
        self.capabilities
            .lock()
            .ok()?
            .get(&connection.stable_id())
            .map(|caps| caps.contains(capability))
    }
    pub fn accept(&self, peer: DeviceId, send: quinn::SendStream, recv: quinn::RecvStream) {
        let pending = PendingMicrophone {
            peer,
            send,
            recv,
            created: std::time::Instant::now(),
        };
        if let Err(sync::TrySendError::Full(pending) | sync::TrySendError::Disconnected(pending)) =
            self.tx.try_send(pending)
        {
            tokio::spawn(async move {
                pending.respond(4).await;
            });
        }
    }
}
impl QuinnTransport {
    pub fn take_microphone(&self) -> Option<PendingMicrophone> {
        self.microphone.rx.lock().ok()?.try_recv().ok()
    }
    pub async fn open_microphone(&self, peer: DeviceId) -> Result<Arc<MicrophoneSession>, String> {
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
            .ok_or("等待来源设备上线")?;
        match self
            .microphone
            .supports(&connection, Capabilities::MICROPHONE)
        {
            Some(true) => {}
            Some(false) => {
                return Err(
                    "来源端运行的 DeskFuse 不支持麦克风共享，请更新并重新打开两端应用".into(),
                );
            }
            None => return Err("等待来源端完成连接能力协商，请稍候".into()),
        }
        let future = async {
            let (mut send, mut recv) = connection.open_bi().await.map_err(|e| e.to_string())?;
            let request = Envelope::new(
                [0; 16],
                Message::MicrophoneOpen(protocol::MicrophoneOpen {
                    version: 1,
                    status: 0,
                }),
            );
            write_frame(&mut send, &request)
                .await
                .map_err(|e| e.to_string())?;
            let ack = read_frame(&mut recv, Duration::from_secs(2))
                .await
                .map_err(|e| format!("来源端未完成麦克风协商，请确认两端已更新并开启共享：{e}"))?;
            match ack.message {
                Some(Message::MicrophoneOpen(open)) if open.version == 1 && open.status == 1 => {
                    // Start after admission, like the source. Queueing and
                    // authorization time are not audio latency and must not
                    // make every packet fail the 250ms freshness check.
                    Ok(MicrophoneSession::new(
                        peer,
                        send,
                        recv,
                        std::time::Instant::now(),
                    ))
                }
                Some(Message::MicrophoneOpen(open)) => Err(match open.status {
                    2 => "来源设备未开启本机麦克风共享",
                    3 => "来源设备尚未允许此设备使用麦克风",
                    4 => "麦克风正被另一台设备使用",
                    _ => "来源设备已锁屏或音频服务未就绪",
                }
                .into()),
                Some(Message::Error(error)) if error.code == 426 => {
                    Err("当前连接未协商麦克风能力，请更新并重新打开两端 DeskFuse".into())
                }
                _ => Err("请更新两端 DeskFuse 以使用麦克风共享".into()),
            }
        };
        time::timeout(Duration::from_secs(3), future)
            .await
            .map_err(|_| "麦克风协商超时，请确认两端版本".to_string())?
    }
}
