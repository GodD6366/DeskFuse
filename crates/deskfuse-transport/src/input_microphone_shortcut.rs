//! Input-only bidirectional holds. Reliable keyboard transitions remain on their
//! existing unidirectional streams; application RPC and updates stay disabled.
use super::*;

impl InputHub {
    pub(crate) fn enable_microphone_shortcuts(
        &mut self,
        shortcuts: crate::microphone_shortcut::ShortcutHub,
    ) {
        self.microphone_shortcuts = Some(shortcuts);
    }

    pub(crate) fn microphone_shortcut_connection(
        &self,
        peer: DeviceId,
    ) -> Option<quinn::Connection> {
        self.sessions
            .lock()
            .ok()?
            .values()
            .filter(|session| {
                session.peer == peer
                    && session.live()
                    && session.context.input_eligible()
                    && session
                        .negotiated
                        .contains(Capabilities::MICROPHONE_SHORTCUT)
            })
            .max_by_key(|session| session.motion.is_ready())
            .map(|session| session.connection.clone())
    }

    pub(super) fn start_microphone_shortcuts(&self, session: Session) {
        let Some(shortcuts) = self.microphone_shortcuts.clone() else {
            return;
        };
        if !session
            .negotiated
            .contains(Capabilities::MICROPHONE_SHORTCUT)
        {
            return;
        }
        let Some(executor) = self.background.as_ref() else {
            return;
        };
        executor.spawn(async move {
            loop {
                let incoming = tokio::select! {
                    incoming = session.connection.accept_bi() => incoming,
                    _ = session.cancel.notified() => break,
                };
                let Ok((send, mut recv)) = incoming else {
                    break;
                };
                // The connection was already physically admitted by register().
                // Bound parsing prevents an unfinished request from monopolizing
                // this lane; audio/update/application messages are never accepted.
                let operation = async {
                    let envelope =
                        crate::read_frame_limited(&mut recv, Duration::from_millis(500), 4096)
                            .await
                            .ok()?;
                    let Some(Message::MicrophoneShortcutOpen(request)) = envelope.message else {
                        return None;
                    };
                    if !session.live() {
                        return None;
                    }
                    shortcuts.accept(session.peer, request, send, recv).await;
                    Some(())
                };
                tokio::select! {
                    _ = operation => {},
                    _ = session.cancel.notified() => break,
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deskfuse_core::{InterfaceKind, LinkScope};

    #[tokio::test]
    async fn microphone_shortcut_input_lane_rejects_application_rpc_and_closes_on_invalidation() {
        // Exercise the lane after physical admission with loopback QUIC. This
        // intentionally does not claim real hardware route eligibility.
        let (_server, client, outgoing, incoming) = super::super::tests::connections().await;
        let mut hub = InputHub::new();
        hub.disable_updates();
        let shortcuts = crate::microphone_shortcut::ShortcutHub::new();
        hub.enable_microphone_shortcuts(shortcuts.clone());
        let peer = DeviceId::from_bytes([31; 32]);
        let session = Session {
            negotiated: Capabilities::INPUT_SHARING | Capabilities::MICROPHONE_SHORTCUT,
            peer,
            connection: incoming.clone(),
            context: ConnectionContext {
                session_id: TransportSessionId::new(),
                local_addr: incoming
                    .local_ip()
                    .map(|ip| SocketAddr::new(ip, 0))
                    .unwrap_or(client.local_addr().unwrap()),
                remote_addr: incoming.remote_address(),
                interface_name: None,
                interface_kind: InterfaceKind::Loopback,
                scope: LinkScope::Unknown,
            },
            valid: Arc::new(AtomicBool::new(true)),
            cancel: Arc::default(),
            motion: Arc::default(),
        };
        assert!(hub.admit_session(session.clone()));
        // Loopback fixtures may exercise framing, but must never become a
        // production shortcut route merely because the capability is present.
        assert!(hub.microphone_shortcut_connection(peer).is_none());
        hub.start_microphone_shortcuts(session.clone());
        let (mut send, mut recv) = outgoing.open_bi().await.unwrap();
        write_frame(
            &mut send,
            &Envelope::new(
                [0; 16],
                Message::Ping(crate::protocol::Ping { nonce: vec![1; 32] }),
            ),
        )
        .await
        .unwrap();
        assert!(
            crate::read_frame(&mut recv, Duration::from_secs(1))
                .await
                .is_err()
        );
        assert!(shortcuts.take().is_none());

        let (mut send, mut recv) = outgoing.open_bi().await.unwrap();
        let request = crate::protocol::MicrophoneShortcutOpen {
            version: 1,
            key_code: 63,
            modifiers: 0,
            routed: true,
        };
        write_frame(
            &mut send,
            &Envelope::new([0; 16], Message::MicrophoneShortcutOpen(request.clone())),
        )
        .await
        .unwrap();
        let ack = crate::read_frame(&mut recv, Duration::from_secs(1))
            .await
            .unwrap();
        assert!(
            matches!(ack.message, Some(Message::MicrophoneShortcutOpen(accepted)) if accepted == request)
        );
        let mut hold = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if let Some(hold) = shortcuts.take() {
                    return hold;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        send.write_all(&[1]).await.unwrap();
        assert!(hold.receive_held().await.unwrap());
        hub.invalidate(&session, "test route removed");
        assert!(hold.receive_held().await.is_err());
        assert!(hub.microphone_shortcut_connection(peer).is_none());
    }
}
