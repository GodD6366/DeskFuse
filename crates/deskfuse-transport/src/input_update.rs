//! Updates use bounded request streams on the authenticated physical LAN
//! connection. They never enter the input queue or clipboard control stream.
//! 一次请求对应一条双向流：请求只有一帧，应答可以是多帧（更新包按块连续写出），
//! 以对端正常结束流作为终点，避免每 64 KiB 一次完整往返。
use super::*;
use futures::StreamExt;
use tokio::sync::Semaphore;

/// 同一会话同时处理的更新流上限；多出的流直接丢弃，防止慢读端制造无界任务。
const MAX_CONCURRENT_UPDATE_STREAMS: usize = 4;
/// 流式应答单帧的读写空闲上限。
const UPDATE_FRAME_IDLE: Duration = Duration::from_secs(10);
const MAX_UPDATE_FRAME: usize = 1100 * 1024;

pub struct UpdateTransportRequest {
    pub peer_id: DeviceId,
    pub context: ConnectionContext,
    pub message: Message,
    /// 应答通道：每发送一条消息写出一帧，发送端关闭即结束流。
    pub reply: mpsc::Sender<Message>,
}
pub type UpdateTransportStream =
    std::pin::Pin<Box<dyn futures::Stream<Item = UpdateTransportRequest> + Send>>;
/// 一次请求的连续应答；流结束表示对端正常结束，错误表示传输失败。
pub type UpdateReplyStream =
    std::pin::Pin<Box<dyn futures::Stream<Item = Result<Message, TransportError>> + Send>>;

pub fn is_update_message(message: &Message) -> bool {
    matches!(
        message,
        Message::ReleaseInventory(_)
            | Message::UpdateRequest(_)
            | Message::UpdateChunk(_)
            | Message::UpdateStatus(_)
            | Message::UpdateCancel(_)
            | Message::UpdateReceipt(_)
            | Message::UpdateCommand(_)
            | Message::UpdateCommandAck(_)
    )
}

impl InputHub {
    pub fn update_stream(&self) -> UpdateTransportStream {
        match self.update_rx.lock().ok().and_then(|mut rx| rx.take()) {
            Some(rx) => futures::stream::unfold(rx, |mut rx| async {
                rx.recv().await.map(|message| (message, rx))
            })
            .boxed(),
            None => futures::stream::pending().boxed(),
        }
    }

    /// 会话看门狗每 250 ms 重新校验物理路由，路由变化会立即使会话失效，
    /// 因此逐帧只需读取会话状态，不再每块启动 route/ifconfig/networksetup 子进程。
    fn update_session_current(session: &Session) -> bool {
        session.live()
            && session.negotiated.contains(Capabilities::LAN_UPDATES)
            && session.context.input_eligible()
    }

    fn update_session(
        &self,
        peer: DeviceId,
        id: TransportSessionId,
    ) -> Result<Session, TransportError> {
        self.sessions
            .lock()
            .map_err(|_| TransportError::Closed)?
            .get(&id)
            .filter(|s| s.peer == peer)
            .cloned()
            .filter(Self::update_session_current)
            .ok_or(TransportError::Closed)
    }

    pub async fn exchange_update(
        &self,
        peer: DeviceId,
        id: TransportSessionId,
        message: Message,
    ) -> Result<Message, TransportError> {
        let operation = async {
            let mut replies = self.exchange_update_stream(peer, id, message).await?;
            replies
                .next()
                .await
                .unwrap_or(Err(TransportError::MalformedPeer("invalid update reply")))
        };
        tokio::time::timeout(Duration::from_secs(12), operation)
            .await
            .map_err(|_| TransportError::Closed)?
    }

    /// 发送一条更新请求并返回连续应答流。调用方按需消费；丢弃流即放弃剩余数据。
    pub async fn exchange_update_stream(
        &self,
        peer: DeviceId,
        id: TransportSessionId,
        message: Message,
    ) -> Result<UpdateReplyStream, TransportError> {
        if !is_update_message(&message) {
            return Err(TransportError::MalformedPeer("not an update message"));
        }
        let session = self.update_session(peer, id)?;
        let (mut send, recv) =
            tokio::time::timeout(UPDATE_FRAME_IDLE, session.connection.open_bi())
                .await
                .map_err(|_| TransportError::Closed)?
                .map_err(|_| TransportError::Closed)?;
        let request_id = *deskfuse_core::RequestId::new().into_uuid().as_bytes();
        write_frame(&mut send, &Envelope::new(request_id, message)).await?;
        send.finish().map_err(|_| TransportError::Closed)?;
        Ok(
            futures::stream::unfold(Some((recv, session)), move |state| async move {
                let (mut recv, session) = state?;
                let frame =
                    crate::read_frame_or_eof(&mut recv, UPDATE_FRAME_IDLE, MAX_UPDATE_FRAME).await;
                let item = match frame {
                    Ok(None) => return None,
                    Ok(Some(reply))
                        if Self::update_session_current(&session)
                            && crate::request_id_or_zero(&reply) == request_id =>
                    {
                        reply
                            .message
                            .filter(is_update_message)
                            .ok_or(TransportError::MalformedPeer("invalid update reply"))
                    }
                    Ok(Some(_)) => Err(TransportError::Closed),
                    Err(error) => Err(error),
                };
                // 出错后不再继续读取，下一次轮询结束流。
                let next = item.is_ok().then_some((recv, session));
                Some((item, next))
            })
            .boxed(),
        )
    }

    pub(super) fn start_updates(&self, session: Session) {
        if !session.negotiated.contains(Capabilities::LAN_UPDATES) {
            return;
        }
        let hub = self.clone();
        let background = self
            .background
            .clone()
            .unwrap_or_else(tokio::runtime::Handle::current);
        let permits = Arc::new(Semaphore::new(MAX_CONCURRENT_UPDATE_STREAMS));
        let spawner = background.clone();
        background.spawn(async move {
            loop {
                let incoming = tokio::select! {
                    incoming = session.connection.accept_bi() => incoming,
                    _ = session.cancel.notified() => break,
                };
                let Ok((send, recv)) = incoming else {
                    break;
                };
                let Ok(permit) = permits.clone().try_acquire_owned() else {
                    continue;
                };
                let hub = hub.clone();
                let session = session.clone();
                spawner.spawn(async move {
                    tokio::select! {
                        _ = hub.serve_update_stream(&session, send, recv) => {},
                        _ = session.cancel.notified() => {},
                    }
                    drop(permit);
                });
            }
        });
    }

    /// 读取一条请求并把运行时产生的全部应答逐帧写回；每一帧都有独立空闲超时，
    /// 整包传输不受总时长限制，但读端停止消费会在 10 秒后释放任务。
    async fn serve_update_stream(
        &self,
        session: &Session,
        mut send: quinn::SendStream,
        mut recv: quinn::RecvStream,
    ) -> Option<()> {
        let request =
            crate::read_frame_limited(&mut recv, Duration::from_secs(3), MAX_UPDATE_FRAME)
                .await
                .ok()?;
        if !Self::update_session_current(session) {
            return None;
        }
        let request_id = crate::request_id_or_zero(&request);
        let message = request.message.filter(is_update_message)?;
        // 批量更新数据让位于输入报文，避免占满链路影响鼠标位置更新。
        let _ = send.set_priority(-1);
        let (reply, mut replies) = mpsc::channel(8);
        self.update_tx
            .try_send(UpdateTransportRequest {
                peer_id: session.peer,
                context: session.context.clone(),
                message,
                reply,
            })
            .ok()?;
        loop {
            let response = match tokio::time::timeout(UPDATE_FRAME_IDLE, replies.recv()).await {
                Ok(Some(response)) => response,
                Ok(None) => break,
                Err(_) => return None,
            };
            if !is_update_message(&response) || !Self::update_session_current(session) {
                return None;
            }
            tokio::time::timeout(
                UPDATE_FRAME_IDLE,
                write_frame(&mut send, &Envelope::new(request_id, response)),
            )
            .await
            .ok()?
            .ok()?;
        }
        send.finish().ok()?;
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deskfuse_protocol::update::UpdateChunk;

    fn chunk(offset: u64) -> Message {
        Message::UpdateChunk(UpdateChunk {
            transaction_id: vec![1; 16],
            offset,
            data: vec![7; 1024],
        })
    }

    /// 一次请求的多帧应答：按顺序读到每一帧，对端正常结束流后得到 `None`。
    #[tokio::test]
    async fn multi_frame_reply_ends_with_clean_eof() {
        let (_server, _client, outgoing, incoming) = crate::input::tests::connections().await;
        let writer = tokio::spawn(async move {
            let (mut send, mut recv) = incoming.accept_bi().await.unwrap();
            let _ = crate::read_frame_limited(&mut recv, Duration::from_secs(2), MAX_UPDATE_FRAME)
                .await
                .unwrap();
            for index in 0..3 {
                write_frame(&mut send, &Envelope::new([5; 16], chunk(index * 1024)))
                    .await
                    .unwrap();
            }
            send.finish().unwrap();
            // 保持连接直到读端结束，避免关闭连接打断未读完的数据。
            let _ = send.stopped().await;
            incoming
        });
        let (mut send, mut recv) = outgoing.open_bi().await.unwrap();
        write_frame(&mut send, &Envelope::new([5; 16], chunk(0)))
            .await
            .unwrap();
        send.finish().unwrap();
        for index in 0..3 {
            let frame =
                crate::read_frame_or_eof(&mut recv, Duration::from_secs(2), MAX_UPDATE_FRAME)
                    .await
                    .unwrap()
                    .expect("frame");
            let Some(Message::UpdateChunk(received)) = frame.message else {
                panic!("wrong frame")
            };
            assert_eq!(received.offset, index * 1024);
        }
        assert!(
            crate::read_frame_or_eof(&mut recv, Duration::from_secs(2), MAX_UPDATE_FRAME)
                .await
                .unwrap()
                .is_none()
        );
        drop(writer.await.unwrap());
    }

    /// 流在帧中间被重置必须是错误，不能被当成正常结束。
    #[tokio::test]
    async fn reset_mid_frame_is_an_error_not_eof() {
        let (_server, _client, outgoing, incoming) = crate::input::tests::connections().await;
        let writer = tokio::spawn(async move {
            let (mut send, _recv) = incoming.accept_bi().await.unwrap();
            send.write_all(&[0, 0, 4]).await.unwrap();
            send.reset(1u32.into()).unwrap();
            incoming
        });
        let (mut send, mut recv) = outgoing.open_bi().await.unwrap();
        send.write_all(b"x").await.unwrap();
        send.finish().unwrap();
        let result =
            crate::read_frame_or_eof(&mut recv, Duration::from_secs(2), MAX_UPDATE_FRAME).await;
        assert!(result.is_err());
        drop(writer.await.unwrap());
    }
}
