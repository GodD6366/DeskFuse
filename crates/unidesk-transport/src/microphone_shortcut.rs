//! A separate authenticated input-service stream for one shortcut hold.
//! A 1.5-second read deadline makes a lost release fail open at the target.
use super::*;
use std::sync::mpsc as sync;

pub struct ShortcutSession {
    pub peer: DeviceId,
    pub request: protocol::MicrophoneShortcutOpen,
    send: quinn::SendStream,
    recv: quinn::RecvStream,
}
impl ShortcutSession {
    pub async fn send_held(&mut self, held: bool) -> Result<(), String> {
        time::timeout(
            Duration::from_millis(500),
            self.send.write_all(&[u8::from(held)]),
        )
        .await
        .map_err(|_| "快捷键通道写入超时".to_string())?
        .map_err(|e| e.to_string())?;
        if !held {
            self.send.finish().map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    pub async fn receive_held(&mut self) -> Result<bool, String> {
        let mut state = [0];
        time::timeout(
            Duration::from_millis(1500),
            self.recv.read_exact(&mut state),
        )
        .await
        .map_err(|_| "快捷键控制超时，已释放".to_string())?
        .map_err(|e| e.to_string())?;
        match state[0] {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err("快捷键状态无效".into()),
        }
    }
}
#[derive(Clone)]
pub(crate) struct ShortcutHub {
    tx: sync::SyncSender<ShortcutSession>,
    rx: Arc<Mutex<sync::Receiver<ShortcutSession>>>,
}
impl ShortcutHub {
    pub(crate) fn new() -> Self {
        let (tx, rx) = sync::sync_channel(8);
        Self {
            tx,
            rx: Arc::new(Mutex::new(rx)),
        }
    }
    pub(crate) async fn accept(
        &self,
        peer: DeviceId,
        request: protocol::MicrophoneShortcutOpen,
        mut send: quinn::SendStream,
        recv: quinn::RecvStream,
    ) {
        let ack = Envelope::new([0; 16], Message::MicrophoneShortcutOpen(request.clone()));
        if time::timeout(Duration::from_millis(500), write_frame(&mut send, &ack))
            .await
            .is_ok_and(|r| r.is_ok())
        {
            let _ = self.tx.try_send(ShortcutSession {
                peer,
                request,
                send,
                recv,
            });
        }
    }
}
impl QuinnTransport {
    pub fn take_microphone_shortcut(&self) -> Option<ShortcutSession> {
        self.microphone_shortcut.rx.lock().ok()?.try_recv().ok()
    }
    pub async fn open_microphone_shortcut(
        &self,
        peer: DeviceId,
        key_code: u16,
        modifiers: u64,
        routed: bool,
    ) -> Result<ShortcutSession, String> {
        if self.is_revoked(peer).unwrap_or(true) {
            return Err("设备信任已撤销".into());
        }
        let connection = self
            .sessions
            .lock()
            .map_err(|_| "输入连接不可用")?
            .get(&peer)
            .and_then(|sessions| sessions.iter().rev().find(|s| s.close_reason().is_none()))
            .cloned()
            .ok_or("键鼠控制设备尚未连接")?;
        let request = protocol::MicrophoneShortcutOpen {
            version: 1,
            key_code: u32::from(key_code),
            modifiers,
            routed,
        };
        time::timeout(Duration::from_millis(1000), async {
            let (mut send, mut recv) = connection.open_bi().await.map_err(|e| e.to_string())?;
            write_frame(
                &mut send,
                &Envelope::new([0; 16], Message::MicrophoneShortcutOpen(request.clone())),
            )
            .await
            .map_err(|e| e.to_string())?;
            let ack = read_frame(&mut recv, Duration::from_millis(500))
                .await
                .map_err(|e| e.to_string())?;
            if !matches!(ack.message, Some(Message::MicrophoneShortcutOpen(_))) {
                return Err("对端不支持麦克风快捷键".into());
            }
            Ok(ShortcutSession {
                peer,
                request,
                send,
                recv,
            })
        })
        .await
        .map_err(|_| "麦克风快捷键连接超时".to_string())?
    }
}
