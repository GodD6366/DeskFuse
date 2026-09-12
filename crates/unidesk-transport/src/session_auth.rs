//! Fresh identity proof tied to the exact QUIC TLS exporter and both signed Hellos.
use super::*;
const LABEL: &[u8] = b"uniDesk session identity v1";

fn payload(
    connection: &quinn::Connection,
    client: &PeerHello,
    server: &PeerHello,
    network_only: bool,
    server_proof: bool,
) -> Result<Vec<u8>, TransportError> {
    let mut transcript = LABEL.to_vec();
    transcript.push(u8::from(network_only));
    transcript.extend_from_slice(&protocol::canonical_hello_payload(&client.to_wire()));
    transcript.extend_from_slice(&protocol::canonical_hello_payload(&server.to_wire()));
    let mut exporter = [0u8; 32];
    connection
        .export_keying_material(&mut exporter, LABEL, &transcript)
        .map_err(|_| TransportError::Closed)?;
    transcript.extend_from_slice(&exporter);
    transcript.push(u8::from(server_proof));
    Ok(transcript)
}
fn signer(
    identity: &Arc<RwLock<Option<Arc<LocalIdentity>>>>,
    hello: &PeerHello,
) -> Result<Arc<LocalIdentity>, TransportError> {
    identity
        .read()
        .ok()
        .and_then(|s| s.clone())
        .filter(|s| s.device_id() == hello.device_id)
        .ok_or(TransportError::ConfigurationRequired(
            "fresh identity signer required for input and device network",
        ))
}
fn verify(proof: Envelope, key: &[u8; 32], payload: &[u8]) -> Result<(), TransportError> {
    let Some(Message::SessionProof(proof)) = proof.message else {
        return Err(TransportError::MalformedPeer(
            "fresh session proof required; upgrade the other device",
        ));
    };
    LocalIdentity::verify_with_key(key, payload, &proof.signature).map_err(TransportError::Identity)
}
pub(crate) async fn client(
    connection: &quinn::Connection,
    local: &PeerHello,
    remote: &PeerHello,
    identity: &Arc<RwLock<Option<Arc<LocalIdentity>>>>,
    network_only: bool,
    timeout: Duration,
) -> Result<(), TransportError> {
    let identity = signer(identity, local)?;
    let (mut send, mut recv) = time::timeout(timeout, connection.open_bi())
        .await
        .map_err(|_| TransportError::Closed)?
        .map_err(|_| TransportError::Closed)?;
    let signature = identity
        .sign(&payload(connection, local, remote, network_only, false)?)
        .as_ref()
        .to_vec();
    write_frame(
        &mut send,
        &Envelope::new(
            HELLO_REQUEST_ID,
            Message::SessionProof(protocol::SessionProof { signature }),
        ),
    )
    .await?;
    send.finish().map_err(|_| TransportError::Closed)?;
    verify(
        read_frame(&mut recv, timeout).await.map_err(|_| {
            TransportError::MalformedPeer(
                "fresh session proof unavailable; upgrade the other device and reconnect",
            )
        })?,
        &remote.public_key,
        &payload(connection, local, remote, network_only, true)?,
    )
}
pub(crate) async fn server(
    connection: &quinn::Connection,
    local: &PeerHello,
    remote: &PeerHello,
    identity: &Arc<RwLock<Option<Arc<LocalIdentity>>>>,
    network_only: bool,
    timeout: Duration,
) -> Result<(), TransportError> {
    let identity = signer(identity, local)?;
    let (mut send, mut recv) = time::timeout(timeout, connection.accept_bi())
        .await
        .map_err(|_| TransportError::Closed)?
        .map_err(|_| TransportError::Closed)?;
    verify(
        read_frame(&mut recv, timeout).await?,
        &remote.public_key,
        &payload(connection, remote, local, network_only, false)?,
    )?;
    let signature = identity
        .sign(&payload(connection, remote, local, network_only, true)?)
        .as_ref()
        .to_vec();
    write_frame(
        &mut send,
        &Envelope::new(
            HELLO_REQUEST_ID,
            Message::SessionProof(protocol::SessionProof { signature }),
        ),
    )
    .await?;
    send.finish().map_err(|_| TransportError::Closed)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    async fn raw_hello(
        client: &QuinnTransport,
        server: &QuinnTransport,
        captured: &PeerHello,
    ) -> (quinn::Connection, PeerHello) {
        let connection = client
            .endpoint
            .as_ref()
            .unwrap()
            .connect(server.local_addr().unwrap(), "localhost")
            .unwrap()
            .await
            .unwrap();
        let (mut send, mut recv) = connection.open_bi().await.unwrap();
        write_frame(
            &mut send,
            &Envelope::new(HELLO_REQUEST_ID, Message::Hello(captured.to_wire())),
        )
        .await
        .unwrap();
        send.finish().unwrap();
        let Some(Message::Hello(remote)) = read_frame(&mut recv, Duration::from_secs(2))
            .await
            .unwrap()
            .message
        else {
            panic!("hello")
        };
        (connection, PeerHello::from_wire(remote).unwrap())
    }
    #[tokio::test]
    async fn clipboard_only_hello_and_previous_exporter_proof_cannot_be_replayed() {
        let a_identity = Arc::new(LocalIdentity::from_bytes(&[101; 32]).unwrap());
        let b_identity = Arc::new(LocalIdentity::from_bytes(&[102; 32]).unwrap());
        // Captured upgrade-era Hello omits both DEVICE_NETWORK and INPUT_SHARING. Proof is still mandatory.
        let a_hello = PeerHello::signed(
            &a_identity,
            "old A",
            Capabilities::NETWORK | Capabilities::CLIPBOARD,
            [1; 32],
        )
        .unwrap();
        let b_hello =
            PeerHello::signed(&b_identity, "B", Capabilities::DEVICE_NETWORK, [2; 32]).unwrap();
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let der = cert.cert.der().to_vec();
        let key = cert.key_pair.serialize_der();
        let addr = "127.0.0.1:0".parse().unwrap();
        let config = QuinnTransportConfig::new(addr, addr, "localhost", der.clone(), key, der);
        let a = QuinnTransport::bind(a_hello.clone(), config.clone())
            .await
            .unwrap();
        let b = QuinnTransport::bind(b_hello, config).await.unwrap();
        b.configure_identity(b_identity).unwrap();
        b.register_peer(a_hello.clone()).unwrap();
        let (first, remote) = raw_hello(&a, &b, &a_hello).await;
        let proof = a_identity
            .sign(&payload(&first, &a_hello, &remote, false, false).unwrap())
            .as_ref()
            .to_vec();
        let (mut send, mut recv) = first.open_bi().await.unwrap();
        write_frame(
            &mut send,
            &Envelope::new(
                HELLO_REQUEST_ID,
                Message::SessionProof(protocol::SessionProof {
                    signature: proof.clone(),
                }),
            ),
        )
        .await
        .unwrap();
        send.finish().unwrap();
        assert!(matches!(
            read_frame(&mut recv, Duration::from_secs(2))
                .await
                .unwrap()
                .message,
            Some(Message::SessionProof(_))
        ));
        first.close(QUIC_CLOSE_CODE, b"done");
        let (second, _) = raw_hello(&a, &b, &a_hello).await;
        let (mut send, mut recv) = second.open_bi().await.unwrap();
        write_frame(
            &mut send,
            &Envelope::new(
                HELLO_REQUEST_ID,
                Message::SessionProof(protocol::SessionProof { signature: proof }),
            ),
        )
        .await
        .unwrap();
        send.finish().unwrap();
        assert!(read_frame(&mut recv, Duration::from_secs(2)).await.is_err());
        assert_eq!(
            b.sessions
                .lock()
                .unwrap()
                .values()
                .flatten()
                .filter(|c| c.close_reason().is_none())
                .count(),
            0
        );
        // A replaying client with no private signer also cannot use the regular API.
        assert!(
            a.bootstrap_connect(
                remote.device_id,
                Endpoint::new("127.0.0.1", b.local_addr().unwrap().port()).unwrap()
            )
            .await
            .is_err()
        );
        a.shutdown().await.unwrap();
        b.shutdown().await.unwrap();
    }
}
