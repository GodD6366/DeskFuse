//! Updates use bounded request/reply streams on the authenticated physical LAN
//! connection. They never enter the input queue or clipboard control stream.
use super::*;
use futures::StreamExt;
use tokio::sync::oneshot;

pub struct UpdateTransportRequest {
    pub peer_id: DeviceId,
    pub context: ConnectionContext,
    pub message: Message,
    pub reply: oneshot::Sender<Message>,
}
pub type UpdateTransportStream =
    std::pin::Pin<Box<dyn futures::Stream<Item = UpdateTransportRequest> + Send>>;

pub fn is_update_message(message: &Message) -> bool {
    matches!(
        message,
        Message::ReleaseInventory(_)
            | Message::UpdateRequest(_)
            | Message::UpdateChunk(_)
            | Message::UpdateStatus(_)
            | Message::UpdateCancel(_)
            | Message::UpdateReceipt(_)
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

    async fn update_session_current(&self, session: &Session) -> bool {
        if !session.live() || !session.negotiated.contains(Capabilities::LAN_UPDATES) {
            return false;
        }
        let context = session.context.clone();
        let expected = context.clone();
        let lan_port = self.lan_port.clone();
        crate::input_executor::inspect_blocking(move || {
            crate::lan::inspect(
                lan_port.as_deref(),
                context.session_id,
                context.local_addr,
                context.remote_addr,
            )
        })
        .await
        .is_ok_and(|current| current == expected && current.input_eligible())
    }

    pub async fn exchange_update(
        &self,
        peer: DeviceId,
        id: TransportSessionId,
        message: Message,
    ) -> Result<Message, TransportError> {
        if !is_update_message(&message) {
            return Err(TransportError::MalformedPeer("not an update message"));
        }
        let session = self
            .sessions
            .lock()
            .map_err(|_| TransportError::Closed)?
            .get(&id)
            .filter(|s| s.peer == peer)
            .cloned()
            .ok_or(TransportError::Closed)?;
        if !self.update_session_current(&session).await {
            return Err(TransportError::Closed);
        }
        let operation = async {
            let (mut send, mut recv) = session
                .connection
                .open_bi()
                .await
                .map_err(|_| TransportError::Closed)?;
            let request_id = *unidesk_core::RequestId::new().into_uuid().as_bytes();
            write_frame(&mut send, &Envelope::new(request_id, message)).await?;
            send.finish().map_err(|_| TransportError::Closed)?;
            let reply =
                crate::read_frame_limited(&mut recv, Duration::from_secs(10), 1100 * 1024).await?;
            if !self.update_session_current(&session).await
                || crate::request_id_or_zero(&reply) != request_id
            {
                return Err(TransportError::Closed);
            }
            reply
                .message
                .filter(is_update_message)
                .ok_or(TransportError::MalformedPeer("invalid update reply"))
        };
        tokio::time::timeout(Duration::from_secs(12), operation)
            .await
            .map_err(|_| TransportError::Closed)?
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
        background.spawn(async move {
            loop {
                let incoming = tokio::select! {
                    incoming = session.connection.accept_bi() => incoming,
                    _ = session.cancel.notified() => break,
                };
                let Ok((mut send, mut recv)) = incoming else {
                    break;
                };
                // One bounded operation at a time per session; slow readers cannot
                // create unbounded worker tasks or stall the separate input streams.
                let operation = async {
                    let request =
                        crate::read_frame_limited(&mut recv, Duration::from_secs(3), 1100 * 1024)
                            .await
                            .ok()?;
                    if !hub.update_session_current(&session).await {
                        return None;
                    }
                    let request_id = crate::request_id_or_zero(&request);
                    let message = request.message.filter(is_update_message)?;
                    let (reply, result) = oneshot::channel();
                    hub.update_tx
                        .try_send(UpdateTransportRequest {
                            peer_id: session.peer,
                            context: session.context.clone(),
                            message,
                            reply,
                        })
                        .ok()?;
                    let response = result.await.ok()?;
                    if !is_update_message(&response)
                        || !hub.update_session_current(&session).await
                    {
                        return None;
                    }
                    write_frame(&mut send, &Envelope::new(request_id, response))
                        .await
                        .ok()?;
                    send.finish().ok()?;
                    Some(())
                };
                tokio::select! {
                    _ = tokio::time::timeout(Duration::from_secs(10), operation) => {},
                    _ = session.cancel.notified() => break,
                }
            }
        });
    }
}
