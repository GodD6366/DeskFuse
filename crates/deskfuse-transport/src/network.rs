//! Bounded pairing/roster channel, isolated from every application session.
use super::*;
use tokio::sync::{Semaphore, mpsc, oneshot};
pub const NETWORK_HELLO_ID: [u8; 16] = *b"DeskFuseNetwrkV2";
/// A verified signed-Hello peer requesting one narrow network-management operation.
#[derive(Debug)]
pub struct NetworkTransportRequest {
    pub remote: PeerHello,
    pub remote_addr: SocketAddr,
    pub payload: Vec<u8>,
    pub reply: oneshot::Sender<Vec<u8>>,
}
pub type NetworkTransportStream = Pin<Box<dyn Stream<Item = NetworkTransportRequest> + Send>>;
#[derive(Clone)]
pub(crate) struct NetworkHub {
    sender: mpsc::Sender<NetworkTransportRequest>,
    receiver: Arc<Mutex<Option<mpsc::Receiver<NetworkTransportRequest>>>>,
    slots: Arc<Semaphore>,
}
impl NetworkHub {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::channel(16);
        Self {
            sender,
            receiver: Arc::new(Mutex::new(Some(receiver))),
            slots: Arc::new(Semaphore::new(16)),
        }
    }
    pub fn stream(&self) -> NetworkTransportStream {
        let receiver = self.receiver.lock().ok().and_then(|mut r| r.take());
        match receiver {
            Some(receiver) => Box::pin(stream::unfold(receiver, |mut receiver| async move {
                receiver.recv().await.map(|message| (message, receiver))
            })),
            None => Box::pin(stream::pending()),
        }
    }
    pub async fn serve(
        &self,
        connection: quinn::Connection,
        remote: PeerHello,
    ) -> Result<(), TransportError> {
        let _permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| TransportError::Closed)?;
        let (mut send, mut recv) = time::timeout(Duration::from_secs(5), connection.accept_bi())
            .await
            .map_err(|_| TransportError::Closed)?
            .map_err(|_| TransportError::Closed)?;
        let request = read_frame(&mut recv, Duration::from_secs(5)).await?;
        let Some(Message::NetworkControl(control)) = request.message else {
            return Err(TransportError::MalformedPeer("network-only connection"));
        };
        let (reply, response) = oneshot::channel();
        self.sender
            .try_send(NetworkTransportRequest {
                remote,
                remote_addr: connection.remote_address(),
                payload: control.payload,
                reply,
            })
            .map_err(|_| TransportError::Closed)?;
        let response = time::timeout(Duration::from_secs(125), response)
            .await
            .map_err(|_| TransportError::Closed)?
            .map_err(|_| TransportError::Closed)?;
        write_frame(
            &mut send,
            &Envelope::new(
                NETWORK_HELLO_ID,
                Message::NetworkControl(protocol::NetworkControl { payload: response }),
            ),
        )
        .await?;
        send.finish().map_err(|_| TransportError::Closed)?;
        let _ = time::timeout(Duration::from_secs(2), send.stopped()).await;
        Ok(())
    }
}
impl QuinnTransport {
    pub(crate) async fn network_exchange_at(
        &self,
        peer: DeviceId,
        endpoint: Endpoint,
        payload: Vec<u8>,
    ) -> Result<(PeerHello, Vec<u8>), TransportError> {
        let fallback = Endpoint {
            host: deskfuse_core::device_hostname(peer),
            port: endpoint.port,
        };
        let mut selected = None;
        // Try the observed/signed endpoint before resolving the mDNS fallback.
        // Resolving both up front spends the entire bounded revocation delivery
        // window on an absent hostname even when the literal IP is reachable.
        'endpoints: for endpoint in [endpoint, fallback] {
            let candidates = super::routes::ranked_endpoints(
                vec![endpoint],
                deskfuse_core::RoutePreference::Automatic,
            )
            .await;
            for (_, endpoint) in candidates {
                let address = resolve_endpoint(&endpoint).await?;
                if let Ok(Ok(session)) = time::timeout(
                    Duration::from_secs(3),
                    self.connect_and_hello_mode(peer, address, None, true),
                )
                .await
                {
                    selected = Some(session);
                    break 'endpoints;
                }
            }
        }
        let (connection, remote) = selected.ok_or(TransportError::Closed)?;
        let result = async {
            if !(self.local_hello.capabilities & remote.capabilities)
                .contains(Capabilities::DEVICE_NETWORK)
            {
                return Err(TransportError::MalformedPeer(
                    "device-network upgrade required",
                ));
            }
            let (mut send, mut recv) = connection
                .open_bi()
                .await
                .map_err(|_| TransportError::Closed)?;
            write_frame(
                &mut send,
                &Envelope::new(
                    NETWORK_HELLO_ID,
                    Message::NetworkControl(protocol::NetworkControl { payload }),
                ),
            )
            .await?;
            send.finish().map_err(|_| TransportError::Closed)?;
            let response = read_frame(&mut recv, Duration::from_secs(125)).await?;
            match response.message {
                Some(Message::NetworkControl(control)) => Ok((remote, control.payload)),
                _ => Err(TransportError::MalformedPeer("network response")),
            }
        }
        .await;
        connection.close(QUIC_CLOSE_CODE, b"network exchange complete");
        result
    }
}
