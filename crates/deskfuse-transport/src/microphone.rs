//! Small, deadline-bounded audio streams on the existing paired QUIC connection.
//! 过期音频按包丢弃、从不回放；Wi-Fi 常见的数百毫秒停顿只丢包不断开，
//! 只有持续卡死超过 [`AUDIO_STALL_LIMIT`] 才作废整条流并重建。
use super::*;
use std::sync::{atomic::Ordering, mpsc as sync};
use tokio::sync::{mpsc, watch};

const MAX_PACKET: usize = 4097;
// macOS taps can deliver ~100 ms at once: 20 audio packets plus RS8M
// repair/control packets. Keep room for a callback burst; age is bounded
// independently by AUDIO_DEADLINE on both producer and consumer paths.
pub(super) const AUDIO_QUEUE_PACKETS: usize = 64;
const AUDIO_DEADLINE: Duration = Duration::from_millis(150);
/// 接收端允许的到达延迟（毫秒），更晚的包只丢弃该包。
const AUDIO_LATE_MS: u128 = 250;
/// 持续卡死上限。实测 Wi-Fi 会成批停顿 350–415ms，必须挺过去；写入阻塞、
/// 读端静默或积压超过该值才说明链路失效，重建会话以重新对齐时钟。
const AUDIO_STALL_LIMIT: Duration = Duration::from_millis(1500);
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
                        Some(value) => value,
                        None => break,
                    },
                    _ = time::sleep(Duration::from_millis(250)) => (std::time::Instant::now(), Vec::new()),
                };
                // 停顿期间积压的旧语音直接跳过，不再作废整条会话。
                if !data.is_empty() && created.elapsed() >= AUDIO_DEADLINE {
                    continue;
                }
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
                    result = time::timeout(AUDIO_STALL_LIMIT, write) => result.is_ok_and(|r| r.is_ok()),
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
                    let sent_ms = u64::from_be_bytes(stamp);
                    let mut data = vec![0; length];
                    recv.read_exact(&mut data).await.map_err(|_| ())?;
                    Ok((sent_ms, data))
                };
                let (sent_ms, data) = tokio::select! {
                    _ = reader_cancel.changed() => break,
                    result = time::timeout(AUDIO_STALL_LIMIT, read) => match result {
                        Ok(Ok(value)) => value,
                        _ => break,
                    },
                };
                // Both clocks start at acceptance, excluding time spent waiting
                // for admission. 可靠流积压只丢弃迟到的包；积压大到无法追上时
                // 才重建会话。
                let late = origin
                    .elapsed()
                    .as_millis()
                    .saturating_sub(u128::from(sent_ms));
                if late > AUDIO_STALL_LIMIT.as_millis() {
                    break;
                }
                if data.is_empty() || late > AUDIO_LATE_MS {
                    continue;
                }
                if !(1..=4).contains(&data[0]) {
                    break;
                }
                // 消费端暂时落后时丢弃新包；只有消费端已经关闭才结束。
                if let Err(sync::TrySendError::Disconnected(_)) =
                    input.try_send((std::time::Instant::now(), data))
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
        match self.outgoing.try_send((std::time::Instant::now(), data)) {
            // 网络停顿时队列会暂时占满：丢弃这一包，写端恢复后自动追上。
            Ok(()) | Err(mpsc::error::TrySendError::Full(_)) => Ok(()),
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.close();
                Err("麦克风连接已断开".into())
            }
        }
    }
    pub fn receive(&self) -> Result<Option<Vec<u8>>, String> {
        if self.is_closed() {
            return Err("麦克风连接已断开".into());
        }
        let rx = self.incoming.lock().map_err(|_| "音频队列不可用")?;
        loop {
            match rx.try_recv() {
                Ok((created, data)) if created.elapsed() < AUDIO_DEADLINE => {
                    return Ok(Some(data));
                }
                // 消费端短暂落后：跳过过期语音，继续取更新的包。
                Ok(_) => {}
                Err(sync::TryRecvError::Empty) => return Ok(None),
                Err(sync::TryRecvError::Disconnected) => {
                    drop(rx);
                    self.close();
                    return Err("麦克风连接已断开".into());
                }
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
    enabled: Arc<Mutex<bool>>,
    pub(super) source_enabled: Arc<AtomicBool>,
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
            enabled: Arc::new(Mutex::new(false)),
            source_enabled: Arc::new(AtomicBool::new(false)),
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
        let enabled = self.enabled.lock().unwrap_or_else(|e| e.into_inner());
        if !*enabled {
            tokio::spawn(async move {
                pending.respond(2).await;
            });
            return;
        }
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
    pub fn set_microphone_source_enabled(&self, enabled: bool) {
        self.microphone
            .source_enabled
            .store(enabled, Ordering::Release);
    }

    /// Query the current provider setting without admitting an audio stream.
    pub async fn microphone_source_enabled(&self, peer: DeviceId) -> Result<bool, String> {
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
        if self
            .microphone
            .supports(&connection, Capabilities::MICROPHONE)
            != Some(true)
        {
            return Ok(false);
        }
        time::timeout(Duration::from_secs(1), async {
            let (mut send, mut recv) = connection.open_bi().await.map_err(|e| e.to_string())?;
            let request = Envelope::new([0; 16], Message::MicrophoneOpen(protocol::MicrophoneOpen {
                version: 2, status: 0,
            }));
            write_frame(&mut send, &request).await.map_err(|e| e.to_string())?;
            let _ = send.finish();
            let reply = read_frame(&mut recv, Duration::from_secs(1)).await.map_err(|e| e.to_string())?;
            Ok(matches!(reply.message, Some(Message::MicrophoneOpen(open)) if open.version == 2 && open.status == 1))
        }).await.map_err(|_| "麦克风状态查询超时".to_string())?
    }

    pub fn set_microphone_enabled(&self, enabled: bool) {
        let mut gate = self
            .microphone
            .enabled
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *gate = enabled;
        if !enabled {
            while self.take_microphone().is_some() {}
        }
    }
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
