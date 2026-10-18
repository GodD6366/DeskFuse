//! Real loopback QUIC: exercise the signed control connection and dedicated streams.
use super::*;
use microphone::MicrophoneSession;
use std::net::{Ipv4Addr, SocketAddr};

struct Pair {
    a: QuinnTransport,
    b: QuinnTransport,
    a_id: DeviceId,
    b_id: DeviceId,
}
impl Pair {
    async fn new(caps: Capabilities) -> Self {
        let identity_a = LocalIdentity::from_bytes(&[81; 32]).unwrap();
        let identity_b = LocalIdentity::from_bytes(&[82; 32]).unwrap();
        let hello_a = PeerHello::signed(&identity_a, "reader", caps, [81; 32]).unwrap();
        let hello_b = PeerHello::signed(&identity_b, "source", caps, [82; 32]).unwrap();
        let certificate = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let cert = certificate.cert.der().to_vec();
        let key = certificate.key_pair.serialize_der();
        let placeholder = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
        let config = || {
            QuinnTransportConfig::new(
                placeholder,
                placeholder,
                "localhost",
                cert.clone(),
                key.clone(),
                cert.clone(),
            )
        };
        let a = QuinnTransport::bind(hello_a.clone(), config())
            .await
            .unwrap();
        let b = QuinnTransport::bind(hello_b.clone(), config())
            .await
            .unwrap();
        a.configure_identity(Arc::new(identity_a)).unwrap();
        b.configure_identity(Arc::new(identity_b)).unwrap();
        a.register_peer(hello_b.clone()).unwrap();
        b.register_peer(hello_a.clone()).unwrap();
        let addr = b.local_addr().unwrap();
        let connection = a
            .connect_with_route(
                hello_b.device_id,
                Endpoint::new(addr.ip().to_string(), addr.port()).unwrap(),
            )
            .await
            .unwrap();
        connection.hello().await.unwrap();
        // Await capability registration on the serve task rather than sleep.
        time::timeout(Duration::from_secs(2), async {
            loop {
                let registered = a
                    .sessions
                    .lock()
                    .unwrap()
                    .get(&hello_b.device_id)
                    .is_some_and(|sessions| {
                        sessions
                            .iter()
                            .any(|connection| a.microphone.supports(connection, caps) == Some(true))
                    });
                if registered {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        Self {
            a,
            b,
            a_id: hello_a.device_id,
            b_id: hello_b.device_id,
        }
    }

    async fn pending(&self) -> microphone::PendingMicrophone {
        time::timeout(Duration::from_secs(2), async {
            loop {
                if let Some(pending) = self.b.take_microphone() {
                    return pending;
                }
                time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .unwrap()
    }

    async fn audio(&self) -> (Arc<MicrophoneSession>, Arc<MicrophoneSession>) {
        let open = self.a.open_microphone(self.b_id);
        let accept = async { self.pending().await.respond(1).await.unwrap() };
        let (reader, source) = tokio::join!(open, accept);
        (reader.unwrap(), source)
    }

    async fn close(self) {
        self.a.shutdown().await.unwrap();
        self.b.shutdown().await.unwrap();
    }
}
async fn received(channel: &MicrophoneSession) -> Vec<u8> {
    time::timeout(Duration::from_secs(1), async {
        loop {
            if let Some(data) = channel.receive().unwrap() {
                return data;
            }
            time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap()
}
async fn closed(channel: &MicrophoneSession) {
    time::timeout(Duration::from_secs(2), async {
        while !channel.is_closed() {
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(channel.receive().is_err());
    assert!(channel.send(vec![1, 42]).is_err());
}

#[tokio::test]
async fn microphone_admission_delay_is_not_audio_age_and_close_reaches_both_ends() {
    let pair = Pair::new(Capabilities::NETWORK | Capabilities::MICROPHONE).await;
    let open = pair.a.open_microphone(pair.b_id);
    let accept = async {
        let pending = pair.pending().await;
        assert_eq!(pending.peer, pair.a_id);
        time::sleep(Duration::from_millis(350)).await;
        pending.respond(1).await.unwrap()
    };
    let (reader, source) = tokio::join!(open, accept);
    let reader = reader.unwrap();
    source.send(vec![1, 12, 34]).unwrap();
    assert_eq!(received(&reader).await, vec![1, 12, 34]);
    reader.send(vec![3, 56]).unwrap();
    assert_eq!(received(&source).await, vec![3, 56]);
    source.close();
    closed(&reader).await;
    pair.close().await;
}

#[tokio::test]
async fn microphone_refused_admission_cannot_create_audio_session() {
    let pair = Pair::new(Capabilities::NETWORK | Capabilities::MICROPHONE).await;
    for (status, message) in [(2, "未开启"), (3, "尚未允许"), (4, "另一台"), (5, "未就绪")]
    {
        let open = pair.a.open_microphone(pair.b_id);
        let reject = async {
            assert!(pair.pending().await.respond(status).await.is_none());
        };
        let (result, ()) = tokio::join!(open, reject);
        assert!(result.err().unwrap().contains(message));
    }
    pair.close().await;
}

#[tokio::test]
async fn microphone_revoking_peer_terminates_live_audio_and_prevents_reopen() {
    let pair = Pair::new(Capabilities::NETWORK | Capabilities::MICROPHONE).await;
    let (reader, source) = pair.audio().await;
    pair.a.revoke_peer(pair.b_id).await.unwrap();
    closed(&reader).await;
    closed(&source).await;
    assert!(
        pair.a
            .open_microphone(pair.b_id)
            .await
            .err()
            .unwrap()
            .contains("撤销")
    );
    pair.close().await;
}

#[tokio::test]
async fn microphone_slow_consumer_discards_stale_speech_instead_of_replaying() {
    let pair = Pair::new(Capabilities::NETWORK | Capabilities::MICROPHONE).await;
    let (reader, source) = pair.audio().await;
    source.send(vec![1, 42]).unwrap();
    time::sleep(Duration::from_millis(400)).await;
    assert!(reader.receive().is_err());
    closed(&source).await;
    pair.close().await;
}

#[tokio::test]
async fn microphone_queue_overflow_closes_session_instead_of_buffering_speech() {
    let pair = Pair::new(Capabilities::NETWORK | Capabilities::MICROPHONE).await;
    let (reader, source) = pair.audio().await;
    // No await: bounded writer queue must reject before the producer can enqueue unbounded audio.
    for _ in 0..microphone::AUDIO_QUEUE_PACKETS {
        source.send(vec![1, 42]).unwrap();
    }
    assert!(source.send(vec![1, 42]).is_err());
    closed(&source).await;
    closed(&reader).await;
    pair.close().await;
}

#[tokio::test]
async fn microphone_capability_is_required_for_audio() {
    let pair = Pair::new(Capabilities::NETWORK).await;
    assert!(pair.a.open_microphone(pair.b_id).await.is_err());
    assert!(pair.b.take_microphone().is_none());
    pair.close().await;
}

#[tokio::test]
async fn microphone_silent_peer_expires_without_native_polling() {
    let pair = Pair::new(Capabilities::NETWORK | Capabilities::MICROPHONE).await;
    let connection = pair
        .a
        .sessions
        .lock()
        .unwrap()
        .get(&pair.b_id)
        .unwrap()
        .last()
        .unwrap()
        .clone();
    let raw = async {
        let (mut send, mut recv) = connection.open_bi().await.unwrap();
        write_frame(
            &mut send,
            &Envelope::new(
                [0; 16],
                Message::MicrophoneOpen(protocol::MicrophoneOpen {
                    version: 1,
                    status: 0,
                }),
            ),
        )
        .await
        .unwrap();
        let ack = read_frame(&mut recv, Duration::from_secs(2)).await.unwrap();
        assert!(matches!(ack.message, Some(Message::MicrophoneOpen(open)) if open.status == 1));
        (send, recv)
    };
    let accept = async { pair.pending().await.respond(1).await.unwrap() };
    let ((_send, _recv), source) = tokio::join!(raw, accept);
    // A peer that neither sends audio nor keepalives must lose the channel,
    // even when the UI never calls receive().
    closed(&source).await;
    pair.close().await;
}

#[tokio::test]
async fn microphone_unknown_protocol_version_is_rejected_before_admission() {
    let pair = Pair::new(Capabilities::NETWORK | Capabilities::MICROPHONE).await;
    let connection = pair
        .a
        .sessions
        .lock()
        .unwrap()
        .get(&pair.b_id)
        .unwrap()
        .last()
        .unwrap()
        .clone();
    let (mut send, mut recv) = connection.open_bi().await.unwrap();
    // Bypass the local validating encoder to simulate a future/malformed peer.
    let invalid = Envelope::new(
        [0; 16],
        Message::MicrophoneOpen(protocol::MicrophoneOpen {
            version: 99,
            status: 0,
        }),
    );
    let payload = prost::Message::encode_to_vec(&invalid);
    send.write_all(&(payload.len() as u32).to_be_bytes())
        .await
        .unwrap();
    send.write_all(&payload).await.unwrap();
    assert!(read_frame(&mut recv, Duration::from_secs(2)).await.is_err());
    assert!(pair.b.take_microphone().is_none());
    pair.close().await;
}

#[tokio::test]
async fn microphone_capture_burst_survives_without_consumer_interleaving() {
    let pair = Pair::new(Capabilities::NETWORK | Capabilities::MICROPHONE).await;
    let (reader, source) = pair.audio().await;
    // One 100 ms tap: 20 audio + 8 FEC packets, plus control/meter headroom.
    for n in 0..32u8 {
        source.send(vec![1, n]).unwrap();
    }
    time::sleep(Duration::from_millis(30)).await;
    for n in 0..32u8 {
        assert_eq!(received(&reader).await, vec![1, n]);
    }
    assert!(!source.is_closed());
    pair.close().await;
}
