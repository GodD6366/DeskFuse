//! Authenticated control transport adapters for DeskFuse.
//!
//! The public API intentionally contains no Quinn or rustls session types.  A
//! [`Connection`] is an opaque control session that can be backed by the
//! deterministic [`InMemoryTransport`] in tests or by [`QuinnTransport`] in a
//! real process.  The protocol crate remains the only owner of wire framing;
//! every RPC is one length-prefixed protobuf frame on a fresh bidirectional
//! QUIC stream.

#![deny(unsafe_code)]
#![allow(clippy::ignored_unit_patterns)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_fields_in_debug)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::unused_async)]

mod control_executor;
mod input;
pub mod input_executor;
pub mod microphone;
pub mod motion_diag;
mod network;
pub mod rdp_tunnel;
pub mod remote_application;
mod session_auth;
pub use network::{NetworkTransportRequest, NetworkTransportStream};
pub mod lan;
pub mod routes;
pub mod socket_diag;
pub use input::{
    InputMotionDiagnostics, InputTransportEvent, InputTransportStream, UpdateReplyStream,
    UpdateTransportRequest, UpdateTransportStream,
};

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    net::SocketAddr,
    pin::Pin,
    sync::{
        Arc, Mutex, OnceLock, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use deskfuse_core::{Capabilities, DeviceId, Endpoint};
use deskfuse_identity::{IdentityError, LocalIdentity};
use deskfuse_protocol::{self as protocol, Envelope, Message, PeerInfo, Ping, Pong, ProtocolError};
use futures::stream::{self, Stream};
use thiserror::Error;
use tokio::sync::{Notify, broadcast};
use tokio::{task::JoinHandle, time};

const HELLO_REQUEST_ID: [u8; protocol::REQUEST_ID_SIZE] = [0; protocol::REQUEST_ID_SIZE];
const MAX_RPC_FRAME: usize = protocol::MAX_FRAME_SIZE;
const QUIC_CLOSE_CODE: quinn::VarInt = quinn::VarInt::from_u32(0x100);
/// 被本机主动断开的设备拨入时使用的关闭原因，拨号方据此识别"对方已断开"。
const PEER_SUSPENDED_REASON: &[u8] = b"peer suspended";

/// A stream of authenticated application messages received on control
/// sessions. The `DeviceId` is the authenticated peer that sent the message.
pub type TransportMessageStream = Pin<Box<dyn Stream<Item = (DeviceId, Message)> + Send + 'static>>;

/// An incoming control session that completed fresh identity proof and admission.
/// This is a recovery hint, not authority to trust or pair a discovered peer.
#[derive(Clone, Debug)]
pub struct ControlPeerConnected {
    pub hello: PeerHello,
    pub endpoint: Endpoint,
}
pub type ControlPeerStream = Pin<Box<dyn Stream<Item = ControlPeerConnected> + Send + 'static>>;

/// 控制会话准入状态。挂起集合与入站会话共用一把锁，保证"挂起并关闭入站"
/// 与"登记新的入站"不会交错，避免挂起后仍漏进一条入站会话。
#[derive(Default)]
struct ControlAdmission {
    /// 本机用户主动断开的设备：拒绝其入站控制会话，两端状态保持一致。
    suspended: BTreeSet<DeviceId>,
    /// 对端拨入本机的控制会话，用于判断对端是否也确认了与本机的连接。
    inbound: BTreeMap<DeviceId, Vec<quinn::Connection>>,
}

impl ControlAdmission {
    /// 登记一条入站会话；挂起的设备返回 false，由调用方关闭连接。
    fn admit(&mut self, peer: DeviceId, connection: &quinn::Connection) -> bool {
        if self.suspended.contains(&peer) {
            return false;
        }
        self.inbound.retain(|_, list| {
            list.retain(|session| session.close_reason().is_none());
            !list.is_empty()
        });
        self.inbound
            .entry(peer)
            .or_default()
            .push(connection.clone());
        true
    }

    fn inbound_alive(&self, peer: DeviceId) -> bool {
        self.inbound
            .get(&peer)
            .is_some_and(|list| list.iter().any(|session| session.close_reason().is_none()))
    }
}

/// Defaults for control-session deadlines.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransportConfig {
    handshake_timeout: Duration,
    idle_timeout: Duration,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            handshake_timeout: Duration::from_secs(5),
            idle_timeout: Duration::from_secs(30),
        }
    }
}

impl TransportConfig {
    /// Set the maximum duration allowed for a connection handshake.
    #[must_use]
    pub const fn with_handshake_timeout(mut self, timeout: Duration) -> Self {
        self.handshake_timeout = timeout;
        self
    }

    /// Set the maximum period without successful RPC activity.  Zero disables
    /// the application-level idle check (QUIC still has its configured limit).
    #[must_use]
    pub const fn with_idle_timeout(mut self, timeout: Duration) -> Self {
        self.idle_timeout = timeout;
        self
    }

    /// Return the handshake deadline.
    #[must_use]
    pub const fn handshake_timeout(self) -> Duration {
        self.handshake_timeout
    }

    /// Return the application-level idle deadline.
    #[must_use]
    pub const fn idle_timeout(self) -> Duration {
        self.idle_timeout
    }

    /// Construct a config with explicit deadlines.
    #[must_use]
    pub const fn new(handshake_timeout: Duration, idle_timeout: Duration) -> Self {
        Self {
            handshake_timeout,
            idle_timeout,
        }
    }
}

/// A safe, transport-independent description of a signed peer hello.
#[derive(Clone, Eq, PartialEq)]
pub struct PeerHello {
    /// SHA-256 digest of [`public_key`](Self::public_key).
    pub device_id: DeviceId,
    /// Human-readable device name.
    pub display_name: String,
    /// Raw Ed25519 verifying key bytes.
    pub public_key: [u8; protocol::ED25519_PUBLIC_KEY_SIZE],
    /// Fresh challenge nonce.
    pub nonce: [u8; protocol::HELLO_NONCE_SIZE],
    /// Detached Ed25519 signature over the canonical protocol hello.
    pub signature: [u8; protocol::ED25519_SIGNATURE_SIZE],
    /// Capability bitset negotiated by the control protocol.
    pub capabilities: Capabilities,
}

impl fmt::Debug for PeerHello {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PeerHello")
            .field("device_id", &self.device_id)
            .field("display_name", &self.display_name)
            .field("public_key", &"[redacted]")
            .field("nonce", &"[redacted]")
            .field("signature", &"[redacted]")
            .field("capabilities", &self.capabilities)
            .finish()
    }
}

impl PeerHello {
    /// Build and sign a hello using the application's identity boundary.
    pub fn signed(
        identity: &LocalIdentity,
        display_name: impl Into<String>,
        capabilities: impl Into<Capabilities>,
        nonce: [u8; protocol::HELLO_NONCE_SIZE],
    ) -> Result<Self, TransportError> {
        let mut wire = protocol::Hello {
            device_id: identity.device_id().as_bytes().to_vec(),
            display_name: display_name.into(),
            public_key: identity.verifying_key_bytes().to_vec(),
            nonce: nonce.to_vec(),
            signature: Vec::new(),
            capabilities: capabilities.into().bits(),
        };
        wire.signature = identity
            .sign(&protocol::canonical_hello_payload(&wire))
            .as_ref()
            .to_vec();
        Self::from_wire(wire)
    }

    /// Alias for [`PeerHello::signed`], useful at composition roots.
    pub fn from_identity(
        identity: &LocalIdentity,
        display_name: impl Into<String>,
        capabilities: impl Into<Capabilities>,
        nonce: [u8; protocol::HELLO_NONCE_SIZE],
    ) -> Result<Self, TransportError> {
        Self::signed(identity, display_name, capabilities, nonce)
    }

    /// Return the validated device id.
    #[must_use]
    pub const fn device_id(&self) -> DeviceId {
        self.device_id
    }

    /// Convert a wire hello after verifying its identity binding and signature.
    pub fn from_wire(wire: protocol::Hello) -> Result<Self, TransportError> {
        protocol::verify_hello(&wire).map_err(TransportError::Protocol)?;
        let device_id = bytes_to_device_id(&wire.device_id)?;
        let public_key = wire
            .public_key
            .as_slice()
            .try_into()
            .map_err(|_| TransportError::MalformedPeer("public key length"))?;
        let nonce = wire
            .nonce
            .as_slice()
            .try_into()
            .map_err(|_| TransportError::MalformedPeer("nonce length"))?;
        let signature = wire
            .signature
            .as_slice()
            .try_into()
            .map_err(|_| TransportError::MalformedPeer("signature length"))?;
        Ok(Self {
            device_id,
            display_name: wire.display_name,
            public_key,
            nonce,
            signature,
            capabilities: Capabilities::from_bits(wire.capabilities),
        })
    }

    /// Return the validated protocol representation.
    #[must_use]
    pub fn to_wire(&self) -> protocol::Hello {
        protocol::Hello {
            device_id: self.device_id.as_bytes().to_vec(),
            display_name: self.display_name.clone(),
            public_key: self.public_key.to_vec(),
            nonce: self.nonce.to_vec(),
            signature: self.signature.to_vec(),
            capabilities: self.capabilities.bits(),
        }
    }

    /// Verify this hello again at a trust boundary.
    pub fn verify(&self) -> Result<(), TransportError> {
        protocol::verify_hello(&self.to_wire()).map_err(TransportError::Protocol)
    }

    /// Return this hello as public peer metadata.
    #[must_use]
    pub fn peer_info(&self) -> PeerInfo {
        PeerInfo {
            device_id: self.device_id.as_bytes().to_vec(),
            display_name: self.display_name.clone(),
            public_key: self.public_key.to_vec(),
            capabilities: self.capabilities.bits(),
        }
    }
}

/// A compact operation counter snapshot useful for deterministic tests and
/// diagnostics.  Counters include failed calls.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CallCounts {
    /// Number of connection attempts.
    pub connect: u64,
    /// Number of hello RPCs.
    pub hello: u64,
    /// Number of ping RPCs.
    pub ping: u64,
    /// Number of close operations that transitioned a session to closed.
    pub close: u64,
}

impl CallCounts {
    /// Alias for [`CallCounts::connect`].
    #[must_use]
    pub const fn connects(self) -> u64 {
        self.connect
    }
    /// Alias for [`CallCounts::hello`].
    #[must_use]
    pub const fn hellos(self) -> u64 {
        self.hello
    }
    /// Alias for [`CallCounts::ping`].
    #[must_use]
    pub const fn pings(self) -> u64 {
        self.ping
    }
    /// Alias for [`CallCounts::close`].
    #[must_use]
    pub const fn closes(self) -> u64 {
        self.close
    }
    /// Explicit count-name alias for [`CallCounts::connect`].
    #[must_use]
    pub const fn connect_count(self) -> u64 {
        self.connect
    }
    /// Explicit count-name alias for [`CallCounts::hello`].
    #[must_use]
    pub const fn hello_count(self) -> u64 {
        self.hello
    }
    /// Explicit count-name alias for [`CallCounts::ping`].
    #[must_use]
    pub const fn ping_count(self) -> u64 {
        self.ping
    }
    /// Explicit count-name alias for [`CallCounts::close`].
    #[must_use]
    pub const fn close_count(self) -> u64 {
        self.close
    }
}

#[derive(Debug, Default)]
struct Counters {
    connect: std::sync::atomic::AtomicU64,
    hello: std::sync::atomic::AtomicU64,
    ping: std::sync::atomic::AtomicU64,
    close: std::sync::atomic::AtomicU64,
}

impl Counters {
    fn snapshot(&self) -> CallCounts {
        use std::sync::atomic::Ordering::Relaxed;
        CallCounts {
            connect: self.connect.load(Relaxed),
            hello: self.hello.load(Relaxed),
            ping: self.ping.load(Relaxed),
            close: self.close.load(Relaxed),
        }
    }
}

/// Errors exposed by every control transport implementation.
#[derive(Debug, Error)]
pub enum TransportError {
    /// A peer was not registered or did not match the requested identity.
    #[error("unknown peer {0}")]
    UnknownPeer(DeviceId),
    /// A peer was explicitly revoked at the transport boundary.
    #[error("peer {0} is revoked")]
    RevokedPeer(DeviceId),
    /// A signed hello had a different identity than the caller requested.
    #[error("peer identity mismatch: expected {expected}, got {actual}")]
    IdentityMismatch {
        expected: DeviceId,
        actual: DeviceId,
    },
    /// A session operation happened after it had been closed.
    #[error("connection is closed")]
    Closed,
    /// A peer has exceeded an inactivity or handshake deadline.
    #[error("transport operation timed out: {operation}")]
    Timeout { operation: &'static str },
    /// A real QUIC adapter has not been provided the certificate material it
    /// needs.  This is preferable to silently falling back to insecure TLS.
    #[error("QUIC transport configuration required: {0}")]
    ConfigurationRequired(&'static str),
    /// Invalid peer data was rejected before a session could be established.
    #[error("malformed peer data: {0}")]
    MalformedPeer(&'static str),
    /// Wire protocol validation or framing failed.
    #[error("protocol error: {0}")]
    Protocol(#[from] ProtocolError),
    /// Identity provider/signature operation failed.
    #[error("identity error: {0}")]
    Identity(#[from] IdentityError),
    /// Socket or stream I/O failed.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// Quinn/rustls setup or session operation failed without exposing its type.
    #[error("QUIC error: {0}")]
    Quic(String),
    /// 对端用户主动断开了与本机的连接，对端拒绝本机的控制会话。
    #[error("peer {0} has paused connections with this device")]
    PeerSuspended(DeviceId),
    /// 独立输入链路的全部候选路径都失败；`failure` 是归纳后的原因。
    #[error("input link unavailable ({failure:?}): {detail}")]
    InputLinkUnavailable {
        failure: deskfuse_core::InputLinkFailure,
        detail: String,
    },
}

/// 单条输入候选路径的失败类别。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum InputPathFailure {
    /// 已通过物理链路检查，但对端身份探测失败。
    Probe,
    /// 身份探测成功，正式连接失败。
    Connect,
    /// 路由经过 VPN/代理隧道等覆盖网络。
    Overlay,
    /// 路由不经过监听所在的物理接口。
    Route,
    Other,
}

/// 汇总输入链路全部候选路径的失败：每类都计数，每类只保留最近一条明细，
/// 避免路径较多时关键原因（例如探测超时）被后来的路由拒绝挤掉。
#[derive(Default)]
struct InputLinkFailures {
    latest: BTreeMap<InputPathFailure, String>,
    counts: BTreeMap<InputPathFailure, usize>,
}

impl InputLinkFailures {
    /// 单条明细上限：三类明细加前缀仍落在运行时 1024 字节的日志字段内。
    const DETAIL_BYTES: usize = 200;
    const DETAIL_KINDS: usize = 3;

    fn record(&mut self, kind: InputPathFailure, mut detail: String) {
        if detail.len() > Self::DETAIL_BYTES {
            let mut end = Self::DETAIL_BYTES;
            while !detail.is_char_boundary(end) {
                end -= 1;
            }
            detail.truncate(end);
        }
        *self.counts.entry(kind).or_default() += 1;
        self.latest.insert(kind, detail);
    }

    fn count(&self, kind: InputPathFailure) -> usize {
        self.counts.get(&kind).copied().unwrap_or(0)
    }

    fn classify(
        &self,
        listeners: usize,
        input_endpoints: usize,
        candidates: usize,
    ) -> deskfuse_core::InputLinkFailure {
        use deskfuse_core::InputLinkFailure as Failure;
        if listeners == 0 {
            Failure::NoLocalLan
        } else if input_endpoints == 0 {
            Failure::NoPeerEndpoints
        } else if candidates > 0
            && self.count(InputPathFailure::Probe) + self.count(InputPathFailure::Connect) > 0
        {
            Failure::PeerUnresponsive
        } else if self.count(InputPathFailure::Overlay) > 0
            && self.count(InputPathFailure::Route) == 0
            && self.count(InputPathFailure::Other) == 0
        {
            Failure::OverlayOnly
        } else if self.count(InputPathFailure::Overlay) + self.count(InputPathFailure::Route) > 0 {
            Failure::NoLanRoute
        } else {
            Failure::Other
        }
    }

    fn summary(&self) -> String {
        let counts = self
            .counts
            .iter()
            .map(|(kind, count)| format!("{kind:?}={count}"))
            .collect::<Vec<_>>()
            .join(",");
        // BTreeMap 按类别顺序排列：探测/连接失败优先于路由拒绝。
        let details = self
            .latest
            .values()
            .take(Self::DETAIL_KINDS)
            .collect::<Vec<_>>();
        format!("failures=[{counts}] details={details:?}")
    }
}

/// An opaque control session.  No Quinn or rustls types appear in this API.
#[derive(Clone)]
pub struct Connection {
    generation: u64,
    peer: DeviceId,
    kind: ConnectionKind,
}

impl fmt::Debug for Connection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Connection")
            .field("peer", &self.peer)
            .field("closed", &self.is_closed())
            .finish_non_exhaustive()
    }
}

enum ConnectionKind {
    Memory(Arc<MemoryConnection>),
    Quinn(Arc<QuinnConnection>),
}

impl Clone for ConnectionKind {
    fn clone(&self) -> Self {
        match self {
            Self::Memory(connection) => Self::Memory(Arc::clone(connection)),
            Self::Quinn(connection) => Self::Quinn(Arc::clone(connection)),
        }
    }
}

impl Connection {
    /// Process-local identity, stable across clones and distinct for every socket.
    pub fn generation(&self) -> u64 {
        self.generation
    }
    fn next_generation() -> u64 {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }
    fn memory(peer: DeviceId, connection: MemoryConnection) -> Self {
        Self {
            peer,
            kind: ConnectionKind::Memory(Arc::new(connection)),
            generation: Self::next_generation(),
        }
    }

    fn quinn(peer: DeviceId, connection: QuinnConnection) -> Self {
        Self {
            peer,
            kind: ConnectionKind::Quinn(Arc::new(connection)),
            generation: Self::next_generation(),
        }
    }

    /// Return the authenticated remote device id.
    #[must_use]
    pub const fn peer_id(&self) -> DeviceId {
        self.peer
    }

    /// Alias for [`Connection::peer_id`].
    #[must_use]
    pub const fn device_id(&self) -> DeviceId {
        self.peer
    }

    /// Return whether this session has been explicitly or remotely closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        match &self.kind {
            ConnectionKind::Memory(connection) => {
                connection.closed.load(std::sync::atomic::Ordering::Acquire)
            }
            ConnectionKind::Quinn(connection) => connection.connection.close_reason().is_some(),
        }
    }

    /// Return the authenticated signed hello received during handshake.
    pub async fn hello(&self) -> Result<PeerHello, TransportError> {
        match &self.kind {
            ConnectionKind::Memory(connection) => connection.hello().await,
            ConnectionKind::Quinn(connection) => connection.hello().await,
        }
    }

    /// Send a ping with an empty challenge and return the peer's pong.
    pub async fn ping(&self) -> Result<Pong, TransportError> {
        self.ping_with_nonce(Vec::new()).await
    }

    /// Send a ping with an application challenge and return the echoed pong.
    pub async fn ping_with_nonce(&self, nonce: Vec<u8>) -> Result<Pong, TransportError> {
        match &self.kind {
            ConnectionKind::Memory(connection) => connection.ping(nonce).await,
            ConnectionKind::Quinn(connection) => connection.ping(nonce).await,
        }
    }

    /// Send one authenticated application message over this session.
    pub async fn send_message(&self, message: Message) -> Result<(), TransportError> {
        if matches!(message, Message::InputSharing(_)) {
            return Err(TransportError::MalformedPeer(
                "input requires pinned ordered stream",
            ));
        }
        match &self.kind {
            ConnectionKind::Memory(connection) => connection.send_message(self.peer, message).await,
            ConnectionKind::Quinn(connection) => connection.send_message(message).await,
        }
    }

    /// Close only this session without waiting for a peer response. Used by
    /// cancellation/recovery paths that must not stall the runtime actor.
    pub fn close_immediately(&self) {
        match &self.kind {
            ConnectionKind::Memory(connection) => {
                if !connection
                    .closed
                    .swap(true, std::sync::atomic::Ordering::AcqRel)
                {
                    connection
                        .counters
                        .close
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
            ConnectionKind::Quinn(connection) => {
                if connection.connection.close_reason().is_none() {
                    connection
                        .connection
                        .close(QUIC_CLOSE_CODE, b"control recovery");
                    connection
                        .counters
                        .close
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }
    }

    /// Gracefully close this control session.  Closing is idempotent.
    pub async fn close(&self) -> Result<(), TransportError> {
        match &self.kind {
            ConnectionKind::Memory(connection) => connection.close().await,
            ConnectionKind::Quinn(connection) => connection.close().await,
        }
    }
}

/// Transport-independent control session factory.
#[async_trait]
pub trait ControlTransport: Send + Sync {
    fn independent_input(&self) -> bool {
        false
    }
    fn control_peers(&self) -> ControlPeerStream {
        Box::pin(stream::pending())
    }

    fn configure_identity(&self, _identity: Arc<LocalIdentity>) -> Result<(), TransportError> {
        Ok(())
    }

    fn supports_network_pairing(&self) -> bool {
        false
    }
    fn network_port(&self) -> Option<u16> {
        None
    }
    async fn network_endpoint_for(&self, _remote: Endpoint) -> Result<Endpoint, TransportError> {
        Err(TransportError::Closed)
    }
    fn network_requests(&self) -> NetworkTransportStream {
        Box::pin(stream::pending())
    }
    async fn network_exchange(
        &self,
        _peer: DeviceId,
        _endpoint: Endpoint,
        _payload: Vec<u8>,
    ) -> Result<(PeerHello, Vec<u8>), TransportError> {
        Err(TransportError::Closed)
    }

    /// Register the public key expected for a peer before connecting.
    ///
    /// Implementations must reject a key whose digest does not equal
    /// `peer`. The default implementation keeps older adapters source
    /// compatible while still enforcing that binding at the API boundary.
    async fn register_peer_identity(
        &self,
        peer: DeviceId,
        public_key: [u8; protocol::ED25519_PUBLIC_KEY_SIZE],
    ) -> Result<(), TransportError> {
        let actual = DeviceId::from_public_key(public_key)
            .map_err(|error| TransportError::Quic(error.to_string()))?;
        if actual != peer {
            return Err(TransportError::IdentityMismatch {
                expected: peer,
                actual,
            });
        }
        Ok(())
    }

    /// Connect to the explicitly requested authenticated peer using the
    /// transport's configured route. This is retained for compatibility.
    async fn connect(&self, peer: DeviceId) -> Result<Connection, TransportError>;

    /// Connect using the route selected by the runtime. Older implementations
    /// can keep implementing [`Self::connect`]; new transports should use the
    /// supplied endpoint instead of a process-global remote address.
    async fn connect_with_route(
        &self,
        peer: DeviceId,
        _endpoint: Endpoint,
    ) -> Result<Connection, TransportError> {
        self.connect(peer).await
    }

    /// Prove the identity at a user-entered endpoint without trusting it or
    /// registering an application session. Pairing consent remains separate.
    async fn probe_endpoint(&self, _endpoint: Endpoint) -> Result<PeerHello, TransportError> {
        Err(TransportError::ConfigurationRequired(
            "endpoint pairing is unsupported",
        ))
    }

    /// Bootstrap an authenticated hello using a discovery hint and endpoint.
    ///
    /// Unlike [`Self::connect`], this method deliberately does not require a
    /// pre-registered identity. It is a one-shot handshake: implementations
    /// must validate the signed hello against `peer`, close the temporary
    /// session, and return only the verified hello.
    async fn bootstrap_connect(
        &self,
        peer: DeviceId,
        endpoint: Endpoint,
    ) -> Result<PeerHello, TransportError> {
        let connection = self.connect_with_route(peer, endpoint).await?;
        let hello = connection.hello().await;
        let close = connection.close().await;
        match (hello, close) {
            (Ok(hello), Ok(())) => Ok(hello),
            (Err(error), _) | (Ok(_), Err(error)) => Err(error),
        }
    }

    /// Revoke a peer at the transport boundary. Implementations must remove
    /// any registered identity and hard-close every session for that peer.
    async fn revoke_peer(&self, _peer: DeviceId) -> Result<(), TransportError> {
        Ok(())
    }

    /// 挂起或恢复某个设备的入站控制会话。挂起时实现必须立即关闭该设备已有的
    /// 入站控制会话，并拒绝之后的入站，使对端不会误以为连接仍然可用。
    fn set_control_suspended(&self, _peer: DeviceId, _suspended: bool) {}

    /// 对端拨入本机的控制会话是否存活。`None` 表示实现无法区分入站会话。
    fn inbound_control_alive(&self, _peer: DeviceId) -> Option<bool> {
        None
    }

    /// 本机物理网卡与地址集合的指纹。插拔雷电线、换 Wi-Fi 时才会变化，
    /// 上层据此触发线路复查，而不是周期性地探测所有候选地址。
    /// `None` 表示实现无法观察本地网络，调用方回退到定时复查。
    fn local_network_fingerprint(&self) -> Option<u64> {
        None
    }

    /// Lift a previous [`Self::revoke_peer`] so the peer can be re-admitted.
    ///
    /// Revocation is a trust decision, and lifting one is too: only the
    /// runtime may call this after the user explicitly asks to re-pair a
    /// revoked peer. Implementations must only clear the revocation marker;
    /// the peer still has to verify and register its identity before it can
    /// connect again.
    async fn restore_peer(&self, _peer: DeviceId) -> Result<(), TransportError> {
        Ok(())
    }

    /// Shut down transport-owned listeners and background tasks.
    ///
    /// The default is a no-op for transports which do not own resources.
    async fn shutdown(&self) -> Result<(), TransportError> {
        Ok(())
    }

    /// Retire other input routes after the actor reaches a local-cursor boundary.
    fn select_input_route(&self, _peer: DeviceId, _session: deskfuse_core::TransportSessionId) {}

    /// 给活跃输入会话采一次质量样本，默认什么都不做。
    fn sample_input_quality(&self) {}
    /// Establish input or prepare a better physical route without closing the current one.
    async fn ensure_input_link(
        &self,
        _peer: DeviceId,
        _endpoints: Vec<Endpoint>,
    ) -> Result<(), TransportError> {
        Err(TransportError::Closed)
    }
    fn update_messages(&self) -> UpdateTransportStream {
        Box::pin(stream::pending())
    }
    async fn exchange_update(
        &self,
        _peer: DeviceId,
        _session: deskfuse_core::TransportSessionId,
        _message: Message,
    ) -> Result<Message, TransportError> {
        Err(TransportError::Closed)
    }
    async fn exchange_update_stream(
        &self,
        _peer: DeviceId,
        _session: deskfuse_core::TransportSessionId,
        _message: Message,
    ) -> Result<UpdateReplyStream, TransportError> {
        Err(TransportError::Closed)
    }
    fn input_contexts(&self, _peer: DeviceId) -> Vec<deskfuse_core::ConnectionContext> {
        Vec::new()
    }
    fn input_capabilities(
        &self,
        _peer: DeviceId,
        _session: deskfuse_core::TransportSessionId,
    ) -> Capabilities {
        Capabilities::default()
    }
    fn input_messages(&self) -> InputTransportStream {
        Box::pin(stream::pending())
    }
    fn send_input(
        &self,
        _peer: DeviceId,
        _session: deskfuse_core::TransportSessionId,
        _message: deskfuse_core::InputShareMessage,
    ) -> Result<(), TransportError> {
        Err(TransportError::Closed)
    }

    /// True only after a capability-gated application-level bidirectional probe.
    fn input_motion_available(
        &self,
        _peer: DeviceId,
        _session: deskfuse_core::TransportSessionId,
    ) -> bool {
        false
    }
    fn send_input_motion(
        &self,
        _peer: DeviceId,
        _session: deskfuse_core::TransportSessionId,
        _motion: deskfuse_core::MotionDatagram,
    ) -> Result<(), TransportError> {
        Err(TransportError::Closed)
    }
    fn send_input_relative(
        &self,
        _peer: DeviceId,
        _session: deskfuse_core::TransportSessionId,
        _motion: deskfuse_core::RelativeMotionDatagram,
    ) -> Result<(), TransportError> {
        Err(TransportError::Closed)
    }
    fn cancel_input_motion(
        &self,
        _peer: DeviceId,
        _session: deskfuse_core::TransportSessionId,
        _control: deskfuse_core::InputSessionId,
    ) {
    }
    fn input_motion_diagnostics(
        &self,
        _peer: DeviceId,
        _session: deskfuse_core::TransportSessionId,
    ) -> InputMotionDiagnostics {
        InputMotionDiagnostics::default()
    }

    async fn send_message(&self, _peer: DeviceId, _message: Message) -> Result<(), TransportError> {
        Err(TransportError::Protocol(ProtocolError::UnknownPayload))
    }

    /// Subscribe to authenticated application messages received from peers.
    ///
    /// The default stream never yields, which keeps existing adapters source
    /// compatible while making the missing capability visible.
    fn messages(&self) -> TransportMessageStream {
        Box::pin(stream::pending())
    }

    /// Route-candidate spelling retained for callers that already model the
    /// selected route as a full [`RouteCandidate`].
    async fn connect_route(
        &self,
        peer: DeviceId,
        route: deskfuse_core::RouteCandidate,
    ) -> Result<Connection, TransportError> {
        self.connect_with_route(peer, route.endpoint).await
    }
}

/// Optional bulk-transfer extension.  It deliberately remains separate from
/// control RPCs so high-volume streams can evolve without changing this trait.
#[async_trait]
pub trait BulkTransport: ControlTransport {
    /// Open a bulk-capable session.  The initial adapter uses the authenticated
    /// control connection as the capability boundary until bulk framing lands.
    async fn open_bulk(&self, peer: DeviceId) -> Result<Connection, TransportError> {
        self.connect(peer).await
    }
}

impl<T: ControlTransport + ?Sized> BulkTransport for T {}

#[derive(Debug)]
struct MemoryConnection {
    hello: PeerHello,
    config: TransportConfig,
    operation_delay: Duration,
    counters: Arc<Counters>,
    closed: std::sync::atomic::AtomicBool,
    last_activity: Mutex<time::Instant>,
    sent: Arc<Mutex<Vec<(DeviceId, Message)>>>,
}

impl MemoryConnection {
    async fn wait(&self, operation: &'static str) -> Result<(), TransportError> {
        let delay = self.operation_delay;
        if delay.is_zero() {
            return Ok(());
        }
        let deadline = if operation == "hello" {
            self.config.handshake_timeout
        } else {
            self.config.idle_timeout_or_handshake()
        };
        time::timeout(deadline, time::sleep(delay))
            .await
            .map_err(|_| TransportError::Timeout { operation })?;
        Ok(())
    }

    fn check_open(&self) -> Result<(), TransportError> {
        if self.closed.load(std::sync::atomic::Ordering::Acquire) {
            return Err(TransportError::Closed);
        }
        if self.config.idle_timeout != Duration::ZERO {
            let idle = self
                .last_activity
                .lock()
                .map_err(|_| TransportError::Closed)?
                .elapsed();
            if idle > self.config.idle_timeout {
                self.closed
                    .store(true, std::sync::atomic::Ordering::Release);
                return Err(TransportError::Timeout { operation: "idle" });
            }
        }
        Ok(())
    }

    fn touch(&self) {
        if let Ok(mut last) = self.last_activity.lock() {
            *last = time::Instant::now();
        }
    }

    async fn hello(&self) -> Result<PeerHello, TransportError> {
        use std::sync::atomic::Ordering::Relaxed;
        self.counters.hello.fetch_add(1, Relaxed);
        self.check_open()?;
        self.wait("hello").await?;
        self.check_open()?;
        self.hello.verify()?;
        self.touch();
        Ok(self.hello.clone())
    }

    async fn ping(&self, nonce: Vec<u8>) -> Result<Pong, TransportError> {
        use std::sync::atomic::Ordering::Relaxed;
        self.counters.ping.fetch_add(1, Relaxed);
        self.check_open()?;
        self.wait("ping").await?;
        self.check_open()?;
        if !nonce.is_empty() && nonce.len() != protocol::HELLO_NONCE_SIZE {
            return Err(TransportError::Protocol(ProtocolError::InvalidNonce {
                expected: protocol::HELLO_NONCE_SIZE,
                actual: nonce.len(),
            }));
        }
        self.touch();
        Ok(Pong { nonce })
    }

    async fn send_message(&self, peer: DeviceId, message: Message) -> Result<(), TransportError> {
        self.check_open()?;
        self.sent
            .lock()
            .map_err(|_| TransportError::Closed)?
            .push((peer, message));
        self.touch();
        Ok(())
    }

    async fn close(&self) -> Result<(), TransportError> {
        use std::sync::atomic::Ordering::{AcqRel, Acquire};
        if self.closed.swap(true, AcqRel) {
            return Ok(());
        }
        self.counters.close.fetch_add(1, Acquire);
        Ok(())
    }
}

/// Deterministic, signed-hello-validating backend for tests and local runtime
/// development.  It never binds a socket and has no wall-clock network state.
#[derive(Clone)]
pub struct InMemoryTransport {
    local_hello: PeerHello,
    peers: Arc<RwLock<BTreeMap<DeviceId, PeerHello>>>,
    registered_identities: Arc<RwLock<BTreeMap<DeviceId, [u8; protocol::ED25519_PUBLIC_KEY_SIZE]>>>,
    revoked: Arc<RwLock<BTreeSet<DeviceId>>>,
    config: TransportConfig,
    operation_delay: Duration,
    counters: Arc<Counters>,
    notify: Arc<Notify>,
    inbound: broadcast::Sender<(DeviceId, Message)>,
    sent: Arc<Mutex<Vec<(DeviceId, Message)>>>,
}

impl fmt::Debug for InMemoryTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InMemoryTransport")
            .field("local_device_id", &self.local_hello.device_id)
            .field(
                "registered_peers",
                &self
                    .peers
                    .read()
                    .map(|peers| peers.len())
                    .unwrap_or_default(),
            )
            .field("config", &self.config)
            .finish()
    }
}

impl InMemoryTransport {
    /// Create an empty peer registry for a local identity.
    #[must_use]
    pub fn new(local_hello: PeerHello) -> Self {
        let (inbound, _) = broadcast::channel(256);
        Self {
            local_hello,
            peers: Arc::new(RwLock::new(BTreeMap::new())),
            registered_identities: Arc::new(RwLock::new(BTreeMap::new())),
            revoked: Arc::new(RwLock::new(BTreeSet::new())),
            config: TransportConfig::default(),
            operation_delay: Duration::ZERO,
            counters: Arc::new(Counters::default()),
            notify: Arc::new(Notify::new()),
            inbound,
            sent: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Inject an authenticated inbound application message, as a remote
    /// session would after forwarding a received frame.
    pub fn push_inbound(&self, peer: DeviceId, message: Message) {
        let _ = self.inbound.send((peer, message));
    }

    /// Return every application message this transport attempted to send.
    #[must_use]
    pub fn sent_messages(&self) -> Vec<(DeviceId, Message)> {
        self.sent
            .lock()
            .map(|sent| sent.clone())
            .unwrap_or_default()
    }

    /// Clear the outbound recording buffer.
    pub fn clear_sent(&self) {
        if let Ok(mut sent) = self.sent.lock() {
            sent.clear();
        }
    }

    /// Convenience constructor for a signed local hello.
    pub fn from_identity(
        identity: &LocalIdentity,
        display_name: impl Into<String>,
        capabilities: impl Into<Capabilities>,
        nonce: [u8; protocol::HELLO_NONCE_SIZE],
    ) -> Result<Self, TransportError> {
        Ok(Self::new(PeerHello::signed(
            identity,
            display_name,
            capabilities,
            nonce,
        )?))
    }

    /// Return this endpoint's authenticated hello.
    #[must_use]
    pub fn local_hello(&self) -> &PeerHello {
        &self.local_hello
    }

    /// Register a validated remote peer.
    pub fn register_peer(&self, peer: PeerHello) -> Result<(), TransportError> {
        if self.is_revoked(peer.device_id)? {
            return Err(TransportError::RevokedPeer(peer.device_id));
        }
        peer.verify()?;
        self.register_peer_identity_sync(peer.device_id, peer.public_key)?;
        self.peers
            .write()
            .map_err(|_| TransportError::Closed)?
            .insert(peer.device_id, peer);
        self.notify.notify_waiters();
        Ok(())
    }

    /// Alias for [`InMemoryTransport::register_peer_at`].
    pub fn register_peer_with_id(
        &self,
        expected: DeviceId,
        peer: PeerHello,
    ) -> Result<(), TransportError> {
        self.register_peer_at(expected, peer)
    }

    /// Register a wire hello, validating its signature before insertion.
    pub fn register_wire_peer(&self, peer: protocol::Hello) -> Result<(), TransportError> {
        self.register_peer(PeerHello::from_wire(peer)?)
    }

    /// Register a hello under an expected id.  This is useful for testing an
    /// identity mismatch between discovery metadata and a signed hello.
    pub fn register_peer_at(
        &self,
        expected: DeviceId,
        peer: PeerHello,
    ) -> Result<(), TransportError> {
        peer.verify()?;
        self.peers
            .write()
            .map_err(|_| TransportError::Closed)?
            .insert(expected, peer);
        self.notify.notify_waiters();
        Ok(())
    }

    fn is_revoked(&self, peer: DeviceId) -> Result<bool, TransportError> {
        Ok(self
            .revoked
            .read()
            .map_err(|_| TransportError::Closed)?
            .contains(&peer))
    }

    fn register_peer_identity_sync(
        &self,
        peer: DeviceId,
        public_key: [u8; protocol::ED25519_PUBLIC_KEY_SIZE],
    ) -> Result<(), TransportError> {
        if self.is_revoked(peer)? {
            return Err(TransportError::RevokedPeer(peer));
        }
        let actual = DeviceId::from_public_key(public_key)
            .map_err(|error| TransportError::Quic(error.to_string()))?;
        if actual != peer {
            return Err(TransportError::IdentityMismatch {
                expected: peer,
                actual,
            });
        }
        let mut identities = self
            .registered_identities
            .write()
            .map_err(|_| TransportError::Closed)?;
        if let Some(existing) = identities.get(&peer)
            && existing != &public_key
        {
            return Err(TransportError::IdentityMismatch {
                expected: peer,
                actual,
            });
        }
        identities.insert(peer, public_key);
        Ok(())
    }

    /// Remove a registered peer.
    pub fn unregister_peer(&self, peer: DeviceId) -> Result<bool, TransportError> {
        Ok(self
            .peers
            .write()
            .map_err(|_| TransportError::Closed)?
            .remove(&peer)
            .is_some())
    }

    /// Return whether a peer is registered.
    #[must_use]
    pub fn has_peer(&self, peer: DeviceId) -> bool {
        self.peers
            .read()
            .is_ok_and(|peers| peers.contains_key(&peer))
    }

    /// Set deterministic operation latency used by timeout tests.
    #[must_use]
    pub const fn with_operation_delay(mut self, delay: Duration) -> Self {
        self.operation_delay = delay;
        self
    }

    /// Replace operation deadlines.
    #[must_use]
    pub const fn with_config(mut self, config: TransportConfig) -> Self {
        self.config = config;
        self
    }

    /// Replace the handshake timeout.
    #[must_use]
    pub const fn with_handshake_timeout(mut self, timeout: Duration) -> Self {
        self.config = self.config.with_handshake_timeout(timeout);
        self
    }

    /// Replace the idle timeout.
    #[must_use]
    pub const fn with_idle_timeout(mut self, timeout: Duration) -> Self {
        self.config = self.config.with_idle_timeout(timeout);
        self
    }

    /// Return operation counters.
    #[must_use]
    pub fn call_counts(&self) -> CallCounts {
        self.counters.snapshot()
    }

    /// Alias for [`InMemoryTransport::call_counts`].
    #[must_use]
    pub fn counters(&self) -> CallCounts {
        self.call_counts()
    }

    /// Number of connection attempts.
    #[must_use]
    pub fn connect_count(&self) -> u64 {
        self.counters().connect
    }

    /// Number of hello RPCs.
    #[must_use]
    pub fn hello_count(&self) -> u64 {
        self.counters().hello
    }

    /// Number of ping RPCs.
    #[must_use]
    pub fn ping_count(&self) -> u64 {
        self.counters().ping
    }

    /// Number of close transitions.
    #[must_use]
    pub fn close_count(&self) -> u64 {
        self.counters().close
    }

    /// Return a future that resolves after the registry changes.  Runtime code
    /// can use this instead of polling discovery in deterministic tests.
    pub async fn wait_for_peer(&self, peer: DeviceId) -> Result<(), TransportError> {
        loop {
            let notified = self.notify.notified();
            if self.has_peer(peer) {
                return Ok(());
            }
            notified.await;
        }
    }
}

#[async_trait]
impl ControlTransport for InMemoryTransport {
    async fn register_peer_identity(
        &self,
        peer: DeviceId,
        public_key: [u8; protocol::ED25519_PUBLIC_KEY_SIZE],
    ) -> Result<(), TransportError> {
        self.register_peer_identity_sync(peer, public_key)
    }

    async fn connect(&self, peer: DeviceId) -> Result<Connection, TransportError> {
        use std::sync::atomic::Ordering::Relaxed;
        self.counters.connect.fetch_add(1, Relaxed);
        if self.is_revoked(peer)? {
            return Err(TransportError::RevokedPeer(peer));
        }
        let registered = self
            .peers
            .read()
            .map_err(|_| TransportError::Closed)?
            .get(&peer)
            .cloned();
        let remote = registered.ok_or(TransportError::UnknownPeer(peer))?;
        let actual = remote.device_id;
        if actual != peer {
            return Err(TransportError::IdentityMismatch {
                expected: peer,
                actual,
            });
        }
        remote.verify()?;
        if self.is_revoked(peer)? {
            return Err(TransportError::RevokedPeer(peer));
        }
        if self.operation_delay > self.config.handshake_timeout {
            time::sleep(self.config.handshake_timeout).await;
            return Err(TransportError::Timeout {
                operation: "handshake",
            });
        }
        if !self.operation_delay.is_zero() {
            time::sleep(self.operation_delay).await;
        }
        Ok(Connection::memory(
            peer,
            MemoryConnection {
                hello: remote,
                config: self.config,
                operation_delay: self.operation_delay,
                counters: Arc::clone(&self.counters),
                closed: std::sync::atomic::AtomicBool::new(false),
                last_activity: Mutex::new(time::Instant::now()),
                sent: Arc::clone(&self.sent),
            },
        ))
    }

    async fn bootstrap_connect(
        &self,
        peer: DeviceId,
        _endpoint: Endpoint,
    ) -> Result<PeerHello, TransportError> {
        if self.is_revoked(peer)? {
            return Err(TransportError::RevokedPeer(peer));
        }
        let remote = self
            .peers
            .read()
            .map_err(|_| TransportError::Closed)?
            .get(&peer)
            .cloned()
            .ok_or(TransportError::UnknownPeer(peer))?;
        remote.verify()?;
        if remote.device_id != peer {
            return Err(TransportError::IdentityMismatch {
                expected: peer,
                actual: remote.device_id,
            });
        }
        if self.is_revoked(peer)? {
            return Err(TransportError::RevokedPeer(peer));
        }
        if self.operation_delay > self.config.handshake_timeout {
            time::sleep(self.config.handshake_timeout).await;
            return Err(TransportError::Timeout {
                operation: "handshake",
            });
        }
        if !self.operation_delay.is_zero() {
            time::sleep(self.operation_delay).await;
        }
        if self.is_revoked(peer)? {
            return Err(TransportError::RevokedPeer(peer));
        }
        Ok(remote)
    }

    async fn revoke_peer(&self, peer: DeviceId) -> Result<(), TransportError> {
        self.revoked
            .write()
            .map_err(|_| TransportError::Closed)?
            .insert(peer);
        self.registered_identities
            .write()
            .map_err(|_| TransportError::Closed)?
            .remove(&peer);
        self.peers
            .write()
            .map_err(|_| TransportError::Closed)?
            .remove(&peer);
        self.notify.notify_waiters();
        Ok(())
    }

    async fn restore_peer(&self, peer: DeviceId) -> Result<(), TransportError> {
        self.revoked
            .write()
            .map_err(|_| TransportError::Closed)?
            .remove(&peer);
        self.notify.notify_waiters();
        Ok(())
    }

    async fn send_message(&self, peer: DeviceId, message: Message) -> Result<(), TransportError> {
        if self.is_revoked(peer)? {
            return Err(TransportError::RevokedPeer(peer));
        }
        if !self.has_peer(peer) {
            return Err(TransportError::UnknownPeer(peer));
        }
        self.sent
            .lock()
            .map_err(|_| TransportError::Closed)?
            .push((peer, message));
        Ok(())
    }

    fn messages(&self) -> TransportMessageStream {
        let receiver = self.inbound.subscribe();
        Box::pin(stream::unfold(receiver, |mut receiver| async move {
            loop {
                match receiver.recv().await {
                    Ok(item) => return Some((item, receiver)),
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        }))
    }
}

/// Configuration for the real QUIC adapter.
///
/// Certificate material is supplied as DER to keep key storage outside this
/// crate.  The trusted certificate is added to a rustls root store (certificate
/// pinning), and the signed protocol hello additionally checks the expected
/// [`DeviceId`].  Empty certificate/key material intentionally yields
/// [`TransportError::ConfigurationRequired`] rather than insecure TLS.
#[derive(Clone)]
pub struct QuinnTransportConfig {
    input_service: bool,
    bind_addr: SocketAddr,
    remote_addr: SocketAddr,
    server_name: String,
    certificate_der: Vec<u8>,
    private_key_der: Vec<u8>,
    trusted_certificate_der: Vec<u8>,
    transport: TransportConfig,
    lan_port: Option<Arc<dyn deskfuse_platform::ports::LanWebPort>>,
}

impl QuinnTransportConfig {
    /// Construct a TLS-pinned QUIC config.  `private_key_der` must be PKCS#8.
    #[must_use]
    pub fn new(
        bind_addr: SocketAddr,
        remote_addr: SocketAddr,
        server_name: impl Into<String>,
        certificate_der: Vec<u8>,
        private_key_der: Vec<u8>,
        trusted_certificate_der: Vec<u8>,
    ) -> Self {
        Self {
            input_service: false,
            bind_addr,
            remote_addr,
            server_name: server_name.into(),
            certificate_der,
            private_key_der,
            trusted_certificate_der,
            transport: TransportConfig::default(),
            lan_port: None,
        }
    }

    /// Inject host-owned route inspection and physical listener APIs.
    #[must_use]
    pub fn with_lan_port(
        mut self,
        lan_port: Arc<dyn deskfuse_platform::ports::LanWebPort>,
    ) -> Self {
        self.lan_port = Some(lan_port);
        self
    }

    /// Dedicated helper listener; it never accepts application RPC or updates.
    pub fn for_input_service(mut self) -> Self {
        self.input_service = true;
        self
    }

    /// Bind an ephemeral loopback endpoint and connect to `remote_addr`.
    #[must_use]
    pub fn loopback(
        remote_addr: SocketAddr,
        server_name: impl Into<String>,
        certificate_der: Vec<u8>,
        private_key_der: Vec<u8>,
        trusted_certificate_der: Vec<u8>,
    ) -> Self {
        Self::new(
            SocketAddr::from(([127, 0, 0, 1], 0)),
            remote_addr,
            server_name,
            certificate_der,
            private_key_der,
            trusted_certificate_der,
        )
    }

    /// Replace handshake and idle limits.
    #[must_use]
    pub const fn with_transport_config(mut self, transport: TransportConfig) -> Self {
        self.transport = transport;
        self
    }

    /// Set the handshake timeout.
    #[must_use]
    pub const fn with_handshake_timeout(mut self, timeout: Duration) -> Self {
        self.transport = self.transport.with_handshake_timeout(timeout);
        self
    }

    /// Set the idle timeout.
    #[must_use]
    pub const fn with_idle_timeout(mut self, timeout: Duration) -> Self {
        self.transport = self.transport.with_idle_timeout(timeout);
        self
    }

    /// Return the configured local bind address.
    #[must_use]
    pub const fn bind_addr(&self) -> SocketAddr {
        self.bind_addr
    }

    /// Return the configured remote address.
    #[must_use]
    pub const fn remote_addr(&self) -> SocketAddr {
        self.remote_addr
    }
}

type LanEndpoints = BTreeMap<SocketAddr, (String, Arc<quinn::Endpoint>)>;

/// 与顺序无关的 (网卡名, IP) 集合指纹；端口随重绑变化，不参与计算。
fn lan_fingerprint<'a>(entries: impl Iterator<Item = (&'a str, std::net::IpAddr)>) -> u64 {
    use std::hash::{Hash, Hasher};
    let entries = entries.collect::<BTreeSet<_>>();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    entries.hash(&mut hasher);
    hasher.finish()
}

/// Client-only endpoints for outbound input dials, keyed by local interface
/// address.
///
/// A dial used to borrow a LAN listener endpoint, which pinned its whole
/// handshake - crypto, timers, retransmits, up to the 3 s timeout per candidate
/// address - to the same two workers that drive a *live* session's datagram
/// reader. Discovery therefore moves to its own endpoint on the probe executor,
/// so probing an unreachable address can no longer delay live input. Cached for
/// the process because a dialed connection must outlive the call that made it.
static INPUT_DIALERS: OnceLock<Mutex<BTreeMap<std::net::IpAddr, CachedDialer>>> = OnceLock::new();

struct CachedDialer {
    endpoint: Arc<quinn::Endpoint>,
    created: std::time::Instant,
    uses: u64,
}

impl CachedDialer {
    /// Age and reuse count, so a probe failure shows whether it went out on a
    /// socket created before the last network change.
    fn usage(&self) -> String {
        format!(
            "dialer_age_secs={} dialer_uses={}",
            self.created.elapsed().as_secs(),
            self.uses
        )
    }
}

/// A cached dialer plus its usage at the time it was handed out.
struct InputDialer {
    endpoint: Arc<quinn::Endpoint>,
    usage: String,
}

fn input_dialer(config: &QuinnTransportConfig, local: SocketAddr) -> Option<InputDialer> {
    let dialers = INPUT_DIALERS.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut dialers = dialers.lock().ok()?;
    if let Some(cached) = dialers.get_mut(&local.ip()) {
        cached.uses += 1;
        return Some(InputDialer {
            endpoint: cached.endpoint.clone(),
            usage: cached.usage(),
        });
    }
    let (_, client) = build_input_quic_configs(config).ok()?;
    let socket = std::net::UdpSocket::bind(SocketAddr::new(local.ip(), 0)).ok()?;
    socket.set_nonblocking(true).ok()?;
    let mut endpoint = quinn::Endpoint::new(
        quinn::EndpointConfig::default(),
        None,
        socket,
        Arc::new(input_executor::InputQuicRuntime::dial().ok()?),
    )
    .ok()?;
    endpoint.set_default_client_config(client);
    let endpoint = Arc::new(endpoint);
    let bound = endpoint.local_addr().unwrap_or(local);
    socket_diag::record("dialer_created", bound, String::new());
    let cached = CachedDialer {
        endpoint: endpoint.clone(),
        created: std::time::Instant::now(),
        uses: 1,
    };
    let usage = cached.usage();
    dialers.insert(local.ip(), cached);
    Some(InputDialer { endpoint, usage })
}

/// Drop a cached dialer after a failed attempt: the interface address it was
/// bound to may be gone, and the next candidate deserves a fresh socket.
fn forget_input_dialer(local: SocketAddr, endpoint: &Arc<quinn::Endpoint>) {
    let Some(dialers) = INPUT_DIALERS.get() else {
        return;
    };
    if let Ok(mut dialers) = dialers.lock()
        && dialers
            .get(&local.ip())
            .is_some_and(|cached| Arc::ptr_eq(&cached.endpoint, endpoint))
        && let Some(cached) = dialers.remove(&local.ip())
    {
        socket_diag::record(
            "dialer_forgotten",
            cached.endpoint.local_addr().unwrap_or(local),
            cached.usage(),
        );
    }
}

#[derive(Clone)]
struct QuinnShared {
    /// All QUIC listener work runs on the process-wide I/O executor. The
    /// application bridge creates the transport on its own single-thread
    /// runtime, but that runtime must not own the control accept loop: a
    /// synchronous bridge operation would otherwise make 42424 stop
    /// answering while the standalone input service on 42425 stays alive.
    io_executor: tokio::runtime::Handle,
    identity: Arc<RwLock<Option<Arc<LocalIdentity>>>>,
    network: network::NetworkHub,
    remote_app: remote_application::RemoteHub,
    rdp: rdp_tunnel::RdpHub,
    microphone: microphone::MicrophoneHub,
    stopping: Arc<AtomicBool>,
    lan_tasks: Arc<Mutex<Vec<JoinHandle<()>>>>,
    lan_endpoints: Arc<Mutex<LanEndpoints>>,
    bound_interface: Option<String>,
    input: input::InputHub,
    bound_addr: SocketAddr,
    local_hello: PeerHello,
    peers: Arc<RwLock<BTreeMap<DeviceId, PeerHello>>>,
    registered_identities: Arc<RwLock<BTreeMap<DeviceId, [u8; protocol::ED25519_PUBLIC_KEY_SIZE]>>>,
    revoked: Arc<RwLock<BTreeSet<DeviceId>>>,
    sessions: Arc<Mutex<BTreeMap<DeviceId, Vec<quinn::Connection>>>>,
    counters: Arc<Counters>,
    config: TransportConfig,
    lan_port: Option<Arc<dyn deskfuse_platform::ports::LanWebPort>>,
    shutdown: Arc<Notify>,
    inbound: broadcast::Sender<(DeviceId, Message)>,
    control_peers: broadcast::Sender<ControlPeerConnected>,
    admission: Arc<Mutex<ControlAdmission>>,
}

impl QuinnShared {
    fn is_revoked(&self, peer: DeviceId) -> Result<bool, TransportError> {
        Ok(self
            .revoked
            .read()
            .map_err(|_| TransportError::Closed)?
            .contains(&peer))
    }

    fn register_session(
        &self,
        peer: DeviceId,
        connection: &quinn::Connection,
    ) -> Result<(), TransportError> {
        let mut sessions = self.sessions.lock().map_err(|_| TransportError::Closed)?;
        if self.is_revoked(peer)? {
            return Err(TransportError::RevokedPeer(peer));
        }
        // Recovery can create many generations over the life of the app.
        sessions.retain(|_, list| {
            list.retain(|session| session.close_reason().is_none());
            !list.is_empty()
        });
        sessions.entry(peer).or_default().push(connection.clone());
        if self.is_revoked(peer)? {
            if let Some(list) = sessions.get_mut(&peer) {
                list.pop();
                if list.is_empty() {
                    sessions.remove(&peer);
                }
            }
            return Err(TransportError::RevokedPeer(peer));
        }
        Ok(())
    }
}

/// Real QUIC control transport with TLS certificate pinning and signed hello
/// identity checks.
#[derive(Clone)]
pub struct QuinnTransport {
    io_executor: Option<tokio::runtime::Handle>,
    identity: Arc<RwLock<Option<Arc<LocalIdentity>>>>,
    network: network::NetworkHub,
    remote_app: remote_application::RemoteHub,
    rdp: rdp_tunnel::RdpHub,
    microphone: microphone::MicrophoneHub,
    lan_tasks: Arc<Mutex<Vec<JoinHandle<()>>>>,
    lan_endpoints: Arc<Mutex<LanEndpoints>>,
    bound_interface: Option<String>,
    input: input::InputHub,
    bound_addr: SocketAddr,
    endpoint: Option<Arc<quinn::Endpoint>>,
    local_hello: PeerHello,
    peers: Arc<RwLock<BTreeMap<DeviceId, PeerHello>>>,
    registered_identities: Arc<RwLock<BTreeMap<DeviceId, [u8; protocol::ED25519_PUBLIC_KEY_SIZE]>>>,
    revoked: Arc<RwLock<BTreeSet<DeviceId>>>,
    sessions: Arc<Mutex<BTreeMap<DeviceId, Vec<quinn::Connection>>>>,
    config: QuinnTransportConfig,
    counters: Arc<Counters>,
    shutdown: Arc<Notify>,
    accept_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    shutdown_started: Arc<AtomicBool>,
    shutdown_finished: Arc<AtomicBool>,
    shutdown_complete: Arc<Notify>,
    inbound: broadcast::Sender<(DeviceId, Message)>,
    control_peers: broadcast::Sender<ControlPeerConnected>,
    admission: Arc<Mutex<ControlAdmission>>,
}

impl fmt::Debug for QuinnTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("QuinnTransport")
            .field("local_device_id", &self.local_hello.device_id)
            .field(
                "local_addr",
                &self
                    .endpoint
                    .as_ref()
                    .and_then(|endpoint| endpoint.local_addr().ok()),
            )
            .field("remote_addr", &self.config.remote_addr)
            .finish_non_exhaustive()
    }
}

/// 对端以"本机已被挂起"为由关闭了这条连接。只在握手已失败时调用：
/// 流读写可能先于关闭帧报错，最多再等 200ms 让关闭原因到达。
async fn suspended_by_peer(connection: &quinn::Connection) -> bool {
    if connection.close_reason().is_none() {
        let _ = time::timeout(Duration::from_millis(200), connection.closed()).await;
    }
    matches!(
        connection.close_reason(),
        Some(quinn::ConnectionError::ApplicationClosed(close))
            if close.reason.as_ref() == PEER_SUSPENDED_REASON
    )
}

struct PendingControlConnection(Option<quinn::Connection>);
impl Drop for PendingControlConnection {
    fn drop(&mut self) {
        if let Some(connection) = self.0.take() {
            connection.close(QUIC_CLOSE_CODE, b"control attempt cancelled");
        }
    }
}

impl QuinnTransport {
    fn shared_state(&self) -> QuinnShared {
        QuinnShared {
            io_executor: self
                .io_executor
                .as_ref()
                .expect("bound transport has a QUIC I/O executor")
                .clone(),
            identity: self.identity.clone(),
            network: self.network.clone(),
            remote_app: self.remote_app.clone(),
            rdp: self.rdp.clone(),
            microphone: self.microphone.clone(),
            lan_tasks: self.lan_tasks.clone(),
            stopping: self.shutdown_started.clone(),
            lan_endpoints: self.lan_endpoints.clone(),
            bound_interface: self.bound_interface.clone(),
            input: self.input.clone(),
            bound_addr: self.bound_addr,
            local_hello: self.local_hello.clone(),
            peers: self.peers.clone(),
            registered_identities: self.registered_identities.clone(),
            revoked: self.revoked.clone(),
            sessions: self.sessions.clone(),
            counters: self.counters.clone(),
            config: self.config.transport,
            lan_port: self.config.lan_port.clone(),
            shutdown: self.shutdown.clone(),
            inbound: self.inbound.clone(),
            control_peers: self.control_peers.clone(),
            admission: self.admission.clone(),
        }
    }
    /// Build the explicit safe skeleton.  Calling [`ControlTransport::connect`]
    /// on it returns [`TransportError::ConfigurationRequired`].
    pub fn new(local_hello: PeerHello) -> Self {
        // There is no insecure placeholder endpoint.  The adapter is created
        // through `bind` once certificate material is available.
        let (inbound, _) = broadcast::channel(256);
        let (control_peers, _) = broadcast::channel(64);
        Self {
            io_executor: None,
            identity: Arc::default(),
            network: network::NetworkHub::new(),
            remote_app: remote_application::RemoteHub::new(),
            rdp: rdp_tunnel::RdpHub::new(),
            microphone: microphone::MicrophoneHub::new(),
            lan_tasks: Arc::default(),
            lan_endpoints: Arc::default(),
            bound_interface: None,
            input: input::InputHub::new(),
            bound_addr: SocketAddr::from(([0, 0, 0, 0], 0)),
            endpoint: None,
            local_hello,
            peers: Arc::new(RwLock::new(BTreeMap::new())),
            registered_identities: Arc::new(RwLock::new(BTreeMap::new())),
            revoked: Arc::new(RwLock::new(BTreeSet::new())),
            sessions: Arc::new(Mutex::new(BTreeMap::new())),
            config: QuinnTransportConfig::new(
                SocketAddr::from(([127, 0, 0, 1], 0)),
                SocketAddr::from(([127, 0, 0, 1], 0)),
                "localhost",
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ),
            counters: Arc::new(Counters::default()),
            shutdown: Arc::new(Notify::new()),
            accept_task: Arc::new(Mutex::new(None)),
            shutdown_started: Arc::new(AtomicBool::new(false)),
            shutdown_finished: Arc::new(AtomicBool::new(false)),
            shutdown_complete: Arc::new(Notify::new()),
            inbound,
            control_peers,
            admission: Arc::default(),
        }
    }

    async fn ensure_service_input_link(
        &self,
        peer: DeviceId,
        endpoints: Vec<Endpoint>,
    ) -> Result<(), TransportError> {
        let current_rank = self
            .input
            .contexts(peer)
            .iter()
            .filter(|c| c.input_eligible())
            .map(|c| routes::input_path_rank(c.interface_kind))
            .min();
        let expected = self
            .registered_identities
            .read()
            .ok()
            .and_then(|m| m.get(&peer).copied())
            .ok_or(TransportError::Closed)?;
        let listeners = self
            .lan_endpoints
            .lock()
            .map_err(|_| TransportError::Closed)?
            .clone();
        // Keep the reason each physical path was rejected instead of returning
        // only "closed": every path is counted, details stay bounded.
        let mut failures = InputLinkFailures::default();
        let mut candidates = Vec::new();
        let endpoint_count = endpoints.len();
        let mut input_endpoint_count = 0;
        for remote in endpoints.into_iter().filter_map(|e| {
            e.host
                .parse::<std::net::IpAddr>()
                .ok()
                .map(|ip| SocketAddr::new(ip, e.port))
        }) {
            if remote.port() != 42425 {
                continue;
            }
            input_endpoint_count += 1;
            for (local, (interface, endpoint)) in &listeners {
                if local.is_ipv4() != remote.is_ipv4() {
                    continue;
                }
                let local = *local;
                let lan_port = self.config.lan_port.clone();
                let context = input_executor::inspect_blocking(move || {
                    lan::inspect_with_error(
                        lan_port.as_deref(),
                        deskfuse_core::TransportSessionId::new(),
                        local,
                        remote,
                    )
                })
                .await;
                let Ok((context, inspection_error)) = context else {
                    failures.record(
                        InputPathFailure::Other,
                        format!("local={local} remote={remote} inspection_worker_unavailable"),
                    );
                    continue;
                };
                if !context.input_eligible() || context.interface_name.as_ref() != Some(interface) {
                    let kind = if context.scope == deskfuse_core::LinkScope::Overlay
                        || context.interface_kind == deskfuse_core::InterfaceKind::Vpn
                    {
                        InputPathFailure::Overlay
                    } else {
                        InputPathFailure::Route
                    };
                    failures.record(kind, format!(
                        "local={local} remote={remote} bound_interface={interface} route_interface={:?} kind={:?} scope={:?} inspection_error={inspection_error:?}",
                        context.interface_name, context.interface_kind, context.scope,
                    ));
                    continue;
                }
                let rank = routes::input_path_rank(context.interface_kind);
                if current_rank.is_some_and(|current| rank >= current) {
                    continue;
                }
                if !candidates
                    .iter()
                    .any(|(_, l, r, _, _)| *l == local && *r == remote)
                {
                    candidates.push((rank, local, remote, interface.clone(), endpoint.clone()));
                }
            }
        }
        candidates.sort_by_key(|(rank, local, remote, _, _)| (*rank, *local, *remote));
        let candidate_count = candidates.len();
        for (rank, local, remote, interface, endpoint) in candidates {
            // An inbound dial may have won while inspection was in flight.
            if self
                .input
                .contexts(peer)
                .iter()
                .any(|c| c.input_eligible() && routes::input_path_rank(c.interface_kind) <= rank)
            {
                return Ok(());
            }
            // A disposable signed identity probe is never promoted into a live connection.
            let Some(InputDialer {
                endpoint: probe_endpoint,
                usage: dialer_usage,
            }) = input_dialer(&self.config, local)
            else {
                failures.record(
                    InputPathFailure::Other,
                    format!("local={local} remote={remote} stage=create_probe failed"),
                );
                continue;
            };
            let mut probe = self.clone();
            probe.endpoint = Some(probe_endpoint.clone());
            probe.bound_addr = local;
            probe.bound_interface = Some(interface.clone());
            match time::timeout(
                Duration::from_secs(1),
                probe.connect_and_hello_mode(peer, remote, Some(expected), true),
            )
            .await
            {
                Ok(Ok((connection, _))) => {
                    connection.close(QUIC_CLOSE_CODE, b"input probe complete")
                }
                result => {
                    let error = match result {
                        Ok(Err(error)) => error.to_string(),
                        Err(_) => "timeout (1s)".into(),
                        _ => unreachable!(),
                    };
                    // 缓存的拨号 socket 可能已随网卡失效，下次换新的。
                    forget_input_dialer(local, &probe_endpoint);
                    failures.record(InputPathFailure::Probe, format!(
                        "local={local} remote={remote} interface={interface} stage=identity_probe error={error} {dialer_usage}"
                    ));
                    continue;
                }
            }
            let mut live = self.clone();
            live.endpoint = Some(endpoint.clone());
            live.bound_addr = local;
            live.bound_interface = Some(interface.clone());
            match time::timeout(Duration::from_secs(3), live.connect_at(peer, remote)).await {
                Ok(Ok(_)) => return Ok(()),
                result => {
                    let error = match result {
                        Ok(Err(error)) => error.to_string(),
                        Err(_) => "timeout (3s)".into(),
                        _ => unreachable!(),
                    };
                    failures.record(InputPathFailure::Connect, format!(
                        "local={local} remote={remote} interface={interface} stage=connect error={error}"
                    ));
                }
            }
        }
        // An unreachable upgrade must never tear down a working fallback.
        if self.input.contexts(peer).iter().any(|c| c.input_eligible()) {
            Ok(())
        } else {
            Err(TransportError::InputLinkUnavailable {
                failure: failures.classify(listeners.len(), input_endpoint_count, candidate_count),
                detail: format!(
                    "endpoints={endpoint_count} listeners={} eligible_candidates={candidate_count} {}",
                    listeners.len(),
                    failures.summary(),
                ),
            })
        }
    }

    /// Bind a secure QUIC endpoint and start its authenticated server loop.
    pub async fn bind(
        local_hello: PeerHello,
        config: QuinnTransportConfig,
    ) -> Result<Self, TransportError> {
        local_hello.verify()?;
        let (server_config, client_config) = build_quic_configs(&config)?;
        let io_executor = if config.input_service {
            input_executor::handle()
        } else {
            control_executor::handle()
        }
        .map_err(|error| TransportError::Quic(format!("QUIC I/O runtime: {error}")))?;
        let bind_addr = if config.input_service {
            SocketAddr::from(([127, 0, 0, 1], 0))
        } else {
            config.bind_addr
        };
        let socket = std::net::UdpSocket::bind(bind_addr)
            .map_err(|error| TransportError::Quic(error.to_string()))?;
        socket
            .set_nonblocking(true)
            .map_err(|error| TransportError::Quic(error.to_string()))?;
        let mut endpoint = quinn::Endpoint::new(
            quinn::EndpointConfig::default(),
            Some(server_config),
            socket,
            Arc::new(control_executor::ControlQuicRuntime(io_executor.clone())),
        )
        .map_err(|error| TransportError::Quic(error.to_string()))?;
        endpoint.set_default_client_config(client_config);
        let endpoint = Arc::new(endpoint);
        let peers = Arc::new(RwLock::new(BTreeMap::new()));
        let registered_identities = Arc::new(RwLock::new(BTreeMap::new()));
        let revoked = Arc::new(RwLock::new(BTreeSet::new()));
        let sessions = Arc::new(Mutex::new(BTreeMap::new()));
        let counters = Arc::new(Counters::default());
        let shutdown = Arc::new(Notify::new());
        let (inbound, _) = broadcast::channel(256);
        let (control_peers, _) = broadcast::channel(64);
        let admission = Arc::new(Mutex::new(ControlAdmission::default()));
        let mut input = input::InputHub::with_lan_port(config.lan_port.clone());
        if config.input_service {
            input.disable_updates();
        }
        let network = network::NetworkHub::new();
        let remote_app = remote_application::RemoteHub::new();
        let rdp = rdp_tunnel::RdpHub::new();
        let microphone = microphone::MicrophoneHub::new();
        let identity = Arc::default();
        let bound_addr = endpoint
            .local_addr()
            .map_err(|e| TransportError::Quic(e.to_string()))?;
        let lan_endpoints = Arc::new(Mutex::new(BTreeMap::new()));
        if local_hello
            .capabilities
            .contains(Capabilities::INPUT_SHARING)
        {
            let port = if config.input_service { 42425 } else { 0 };
            let lan_port = config.lan_port.clone();
            let sockets = tokio::task::spawn_blocking(move || {
                lan::listener_sockets_on(lan_port.as_deref(), port)
            })
            .await
            .unwrap_or_default();
            for (interface, socket) in sockets {
                let (lan_server, lan_client) = build_input_quic_configs(&config)?;
                if let Ok(mut endpoint) = quinn::Endpoint::new(
                    quinn::EndpointConfig::default(),
                    Some(lan_server),
                    socket,
                    Arc::new(input_executor::InputQuicRuntime::new()?),
                ) {
                    endpoint.set_default_client_config(lan_client);
                    if let Ok(address) = endpoint.local_addr() {
                        socket_diag::record(
                            "listener_bound",
                            address,
                            format!("interface={interface}"),
                        );
                        lan_endpoints
                            .lock()
                            .map_err(|_| TransportError::Closed)?
                            .insert(address, (interface, Arc::new(endpoint)));
                    }
                }
            }
        }
        let lan_tasks = Arc::new(Mutex::new(Vec::new()));
        let stopping = Arc::new(AtomicBool::new(false));
        let shared = QuinnShared {
            io_executor: io_executor.clone(),
            identity: Arc::clone(&identity),
            network: network.clone(),
            remote_app: remote_app.clone(),
            rdp: rdp.clone(),
            microphone: microphone.clone(),
            lan_tasks: lan_tasks.clone(),
            stopping: stopping.clone(),
            lan_endpoints: lan_endpoints.clone(),
            bound_interface: None,
            input: input.clone(),
            bound_addr,
            local_hello: local_hello.clone(),
            peers: Arc::clone(&peers),
            registered_identities: Arc::clone(&registered_identities),
            revoked: Arc::clone(&revoked),
            sessions: Arc::clone(&sessions),
            counters: Arc::clone(&counters),
            config: config.transport,
            lan_port: config.lan_port.clone(),
            shutdown: Arc::clone(&shutdown),
            inbound: inbound.clone(),
            control_peers: control_peers.clone(),
            admission: Arc::clone(&admission),
        };
        for (address, (interface, endpoint)) in lan_endpoints
            .lock()
            .map_err(|_| TransportError::Closed)?
            .iter()
        {
            let mut state = shared.clone();
            state.io_executor = input_executor::handle()
                .map_err(|error| TransportError::Quic(error.to_string()))?;
            state.bound_addr = *address;
            state.bound_interface = Some(interface.clone());
            let endpoint = endpoint.clone();
            let task = state.io_executor.clone().spawn(async move {
                accept_loop(endpoint, state).await;
            });
            lan_tasks
                .lock()
                .map_err(|_| TransportError::Closed)?
                .push(task);
        }
        if local_hello
            .capabilities
            .contains(Capabilities::INPUT_SHARING)
        {
            let mut state = shared.clone();
            state.io_executor = input_executor::handle()
                .map_err(|error| TransportError::Quic(error.to_string()))?;
            let refresh_config = config.clone();
            let task = state.io_executor.clone().spawn(async move {
                refresh_lan_listeners(state, refresh_config).await;
            });
            lan_tasks
                .lock()
                .map_err(|_| TransportError::Closed)?
                .push(task);
        }
        let accept_endpoint = Arc::clone(&endpoint);
        let accept_task = if config.input_service {
            None
        } else {
            Some(io_executor.spawn(async move {
                accept_loop(accept_endpoint, shared).await;
            }))
        };
        Ok(Self {
            io_executor: Some(io_executor),
            identity,
            network,
            remote_app,
            rdp,
            microphone,
            lan_tasks,
            lan_endpoints,
            bound_interface: None,
            input,
            bound_addr,
            endpoint: Some(endpoint),
            local_hello,
            peers,
            registered_identities,
            revoked,
            sessions,
            config,
            counters,
            shutdown,
            accept_task: Arc::new(Mutex::new(accept_task)),
            shutdown_started: stopping,
            shutdown_finished: Arc::new(AtomicBool::new(false)),
            shutdown_complete: Arc::new(Notify::new()),
            inbound,
            control_peers,
            admission,
        })
    }

    /// Alias for [`QuinnTransport::bind`].
    pub async fn from_config(
        local_hello: PeerHello,
        config: QuinnTransportConfig,
    ) -> Result<Self, TransportError> {
        Self::bind(local_hello, config).await
    }

    /// Return this endpoint's local UDP address.
    pub fn local_addr(&self) -> Result<SocketAddr, TransportError> {
        self.endpoint
            .as_ref()
            .ok_or(TransportError::ConfigurationRequired(
                "call QuinnTransport::bind with pinned certificate material",
            ))?
            .local_addr()
            .map_err(|error| TransportError::Quic(error.to_string()))
    }

    /// Return the local signed hello.
    #[must_use]
    pub fn local_hello(&self) -> &PeerHello {
        &self.local_hello
    }

    /// Register an expected peer for incoming and outgoing identity checks.
    pub fn register_peer(&self, peer: PeerHello) -> Result<(), TransportError> {
        if self.is_revoked(peer.device_id)? {
            return Err(TransportError::RevokedPeer(peer.device_id));
        }
        peer.verify()?;
        self.register_peer_identity_sync(peer.device_id, peer.public_key)?;
        self.peers
            .write()
            .map_err(|_| TransportError::Closed)?
            .insert(peer.device_id, peer);
        Ok(())
    }

    fn is_revoked(&self, peer: DeviceId) -> Result<bool, TransportError> {
        Ok(self
            .revoked
            .read()
            .map_err(|_| TransportError::Closed)?
            .contains(&peer))
    }

    fn register_session(
        &self,
        peer: DeviceId,
        connection: &quinn::Connection,
    ) -> Result<(), TransportError> {
        let mut sessions = self.sessions.lock().map_err(|_| TransportError::Closed)?;
        if self.is_revoked(peer)? {
            return Err(TransportError::RevokedPeer(peer));
        }
        sessions.retain(|_, list| {
            list.retain(|session| session.close_reason().is_none());
            !list.is_empty()
        });
        sessions.entry(peer).or_default().push(connection.clone());
        if self.is_revoked(peer)? {
            if let Some(list) = sessions.get_mut(&peer) {
                list.pop();
                if list.is_empty() {
                    sessions.remove(&peer);
                }
            }
            return Err(TransportError::RevokedPeer(peer));
        }
        Ok(())
    }

    fn register_peer_identity_sync(
        &self,
        peer: DeviceId,
        public_key: [u8; protocol::ED25519_PUBLIC_KEY_SIZE],
    ) -> Result<(), TransportError> {
        if self.is_revoked(peer)? {
            return Err(TransportError::RevokedPeer(peer));
        }
        let actual = DeviceId::from_public_key(public_key)
            .map_err(|error| TransportError::Quic(error.to_string()))?;
        if actual != peer {
            return Err(TransportError::IdentityMismatch {
                expected: peer,
                actual,
            });
        }
        let mut identities = self
            .registered_identities
            .write()
            .map_err(|_| TransportError::Closed)?;
        if let Some(existing) = identities.get(&peer)
            && existing != &public_key
        {
            return Err(TransportError::IdentityMismatch {
                expected: peer,
                actual,
            });
        }
        identities.insert(peer, public_key);
        Ok(())
    }

    /// Return operation counters.
    #[must_use]
    pub fn call_counts(&self) -> CallCounts {
        self.counters.snapshot()
    }

    /// Close the listener and await the accept loop.
    pub async fn shutdown(&self) -> Result<(), TransportError> {
        if self.shutdown_started.swap(true, Ordering::AcqRel) {
            while !self.shutdown_finished.load(Ordering::Acquire) {
                let notified = self.shutdown_complete.notified();
                if self.shutdown_finished.load(Ordering::Acquire) {
                    break;
                }
                notified.await;
            }
            return Ok(());
        }

        let lan_endpoints = self
            .lan_endpoints
            .lock()
            .map(|mut m| std::mem::take(&mut *m))
            .unwrap_or_default();
        for (_, (_, endpoint)) in lan_endpoints {
            endpoint.close(QUIC_CLOSE_CODE, b"transport shutdown");
        }
        let tasks = self
            .lan_tasks
            .lock()
            .map(|mut tasks| std::mem::take(&mut *tasks))
            .unwrap_or_default();
        for task in tasks {
            task.abort();
            let _ = task.await;
        }
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| TransportError::Closed)
            .map(|mut sessions| std::mem::take(&mut *sessions))?;
        for connection in sessions.into_values().flatten() {
            connection.close(QUIC_CLOSE_CODE, b"transport shutdown");
        }
        if let Some(endpoint) = &self.endpoint {
            endpoint.close(QUIC_CLOSE_CODE, b"transport shutdown");
        }
        self.shutdown.notify_waiters();

        let accept_task = self
            .accept_task
            .lock()
            .map_err(|_| TransportError::Closed)?
            .take();
        let result = if let Some(task) = accept_task {
            task.await
                .map_err(|error| TransportError::Quic(error.to_string()))
        } else {
            Ok(())
        };
        self.shutdown_finished.store(true, Ordering::Release);
        self.shutdown_complete.notify_waiters();
        result
    }
}

#[async_trait]
impl ControlTransport for QuinnTransport {
    fn independent_input(&self) -> bool {
        self.config.input_service
    }
    fn control_peers(&self) -> ControlPeerStream {
        let receiver = self.control_peers.subscribe();
        Box::pin(stream::unfold(receiver, |mut receiver| async move {
            loop {
                match receiver.recv().await {
                    Ok(item) => return Some((item, receiver)),
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        }))
    }

    fn set_control_suspended(&self, peer: DeviceId, suspended: bool) {
        let closing = match self.admission.lock() {
            Ok(mut admission) if suspended => {
                admission.suspended.insert(peer);
                admission.inbound.remove(&peer).unwrap_or_default()
            }
            Ok(mut admission) => {
                admission.suspended.remove(&peer);
                Vec::new()
            }
            Err(_) => return,
        };
        // 关闭对端拨入的会话：对端的心跳随即失败并重拨，重拨被拒后显示"对方已断开"。
        for connection in closing {
            connection.close(QUIC_CLOSE_CODE, PEER_SUSPENDED_REASON);
        }
    }

    fn inbound_control_alive(&self, peer: DeviceId) -> Option<bool> {
        if self.bound_interface.is_some() || self.config.input_service {
            return None;
        }
        self.admission
            .lock()
            .ok()
            .map(|admission| admission.inbound_alive(peer))
    }

    fn local_network_fingerprint(&self) -> Option<u64> {
        // 物理网卡监听由 refresh_lan_listeners 每 2 秒按网卡变化增删，
        // 这里只读取其结果，不额外枚举网卡或发包。
        if self.endpoint.is_none()
            || !self
                .local_hello
                .capabilities
                .contains(Capabilities::INPUT_SHARING)
        {
            return None;
        }
        let listeners = self.lan_endpoints.lock().ok()?;
        Some(lan_fingerprint(listeners.iter().map(
            |(address, (interface, _))| (interface.as_str(), address.ip()),
        )))
    }

    fn configure_identity(&self, identity: Arc<LocalIdentity>) -> Result<(), TransportError> {
        if identity.device_id() != self.local_hello.device_id {
            return Err(TransportError::IdentityMismatch {
                expected: self.local_hello.device_id,
                actual: identity.device_id(),
            });
        }
        *self.identity.write().map_err(|_| TransportError::Closed)? = Some(identity);
        Ok(())
    }

    fn supports_network_pairing(&self) -> bool {
        true
    }
    fn network_port(&self) -> Option<u16> {
        self.local_addr().ok().map(|a| a.port())
    }
    async fn network_endpoint_for(&self, remote: Endpoint) -> Result<Endpoint, TransportError> {
        let local = self.local_addr()?;
        let ip = if local.ip().is_unspecified() {
            let remote = resolve_endpoint(&remote).await?;
            let socket = std::net::UdpSocket::bind(if remote.is_ipv4() {
                "0.0.0.0:0"
            } else {
                "[::]:0"
            })
            .map_err(|e| TransportError::Quic(e.to_string()))?;
            socket
                .connect(remote)
                .map_err(|e| TransportError::Quic(e.to_string()))?;
            socket
                .local_addr()
                .map_err(|e| TransportError::Quic(e.to_string()))?
                .ip()
        } else {
            local.ip()
        };
        Endpoint::new(ip.to_string(), local.port()).map_err(|_| TransportError::Closed)
    }
    fn network_requests(&self) -> NetworkTransportStream {
        self.network.stream()
    }
    async fn network_exchange(
        &self,
        peer: DeviceId,
        endpoint: Endpoint,
        payload: Vec<u8>,
    ) -> Result<(PeerHello, Vec<u8>), TransportError> {
        self.network_exchange_at(peer, endpoint, payload).await
    }

    fn select_input_route(&self, peer: DeviceId, session: deskfuse_core::TransportSessionId) {
        if self.config.input_service {
            self.input.select_route(peer, session);
        }
    }
    fn sample_input_quality(&self) {
        if self.config.input_service {
            self.input.sample_quality();
        }
    }
    async fn ensure_input_link(
        &self,
        peer: DeviceId,
        endpoints: Vec<Endpoint>,
    ) -> Result<(), TransportError> {
        if self.config.input_service {
            return self.ensure_service_input_link(peer, endpoints).await;
        }
        if self
            .input
            .contexts(peer)
            .iter()
            .any(deskfuse_core::ConnectionContext::input_eligible)
        {
            return Ok(());
        }
        let session = self
            .sessions
            .lock()
            .map_err(|_| TransportError::Closed)?
            .get(&peer)
            // A superseded inbound socket can linger until QUIC times out.
            // Rebuild input through the most recently authenticated control
            // connection, not the oldest still-open socket after reconnect.
            .and_then(|v| v.iter().rev().find(|c| c.close_reason().is_none()))
            .cloned()
            .ok_or(TransportError::Closed)?;
        let (mut send, mut recv) = time::timeout(Duration::from_secs(2), session.open_bi())
            .await
            .map_err(|_| TransportError::Closed)?
            .map_err(|_| TransportError::Closed)?;
        write_frame(
            &mut send,
            &Envelope::new(
                [0; 16],
                Message::InputEndpoints(protocol::InputEndpoints { addresses: vec![] }),
            ),
        )
        .await?;
        send.finish().map_err(|_| TransportError::Closed)?;
        let response = read_frame(&mut recv, Duration::from_secs(2)).await?;
        let Some(Message::InputEndpoints(advertised)) = response.message else {
            return Err(TransportError::Closed);
        };
        let listeners = self
            .lan_endpoints
            .lock()
            .map_err(|_| TransportError::Closed)?
            .clone();
        // Keep input, clipboard bulk and updates on the selected control path
        // where possible; other physical interfaces remain fallbacks.
        let mut addresses = advertised.addresses;
        addresses.sort_by_key(|address| {
            address
                .parse::<SocketAddr>()
                .ok()
                .is_none_or(|remote| remote.ip() != session.remote_address().ip())
        });
        for address in addresses {
            let Ok(remote) = address.parse::<SocketAddr>() else {
                continue;
            };
            for (local, (interface, endpoint)) in &listeners {
                if local.is_ipv4() != remote.is_ipv4() {
                    continue;
                }
                let local = *local;
                let lan_port = self.config.lan_port.clone();
                let Ok(context) = input_executor::inspect_blocking(move || {
                    lan::inspect(
                        lan_port.as_deref(),
                        deskfuse_core::TransportSessionId::new(),
                        local,
                        remote,
                    )
                })
                .await
                else {
                    continue;
                };
                if !context.input_eligible() || context.interface_name.as_deref() != Some(interface)
                {
                    continue;
                }
                let dialer = input_dialer(&self.config, local).map(|dialer| dialer.endpoint);
                let mut dedicated = self.clone();
                dedicated.endpoint = Some(dialer.clone().unwrap_or_else(|| endpoint.clone()));
                dedicated.bound_addr = local;
                dedicated.bound_interface = Some(interface.clone());
                let attempt =
                    time::timeout(Duration::from_secs(3), dedicated.connect_at(peer, remote)).await;
                if !matches!(attempt, Ok(Ok(_)))
                    && let Some(dialer) = &dialer
                {
                    forget_input_dialer(local, dialer);
                }
                if let Ok(Ok(connected)) = attempt {
                    if session.close_reason().is_some() {
                        connected.close_immediately();
                        return Err(TransportError::Closed);
                    }
                    return Ok(());
                }
            }
        }
        Err(TransportError::Closed)
    }
    fn update_messages(&self) -> UpdateTransportStream {
        self.input.update_stream()
    }
    async fn exchange_update(
        &self,
        peer: DeviceId,
        session: deskfuse_core::TransportSessionId,
        message: Message,
    ) -> Result<Message, TransportError> {
        if self.is_revoked(peer)? {
            return Err(TransportError::RevokedPeer(peer));
        }
        self.input.exchange_update(peer, session, message).await
    }
    async fn exchange_update_stream(
        &self,
        peer: DeviceId,
        session: deskfuse_core::TransportSessionId,
        message: Message,
    ) -> Result<UpdateReplyStream, TransportError> {
        if self.is_revoked(peer)? {
            return Err(TransportError::RevokedPeer(peer));
        }
        self.input
            .exchange_update_stream(peer, session, message)
            .await
    }
    fn input_contexts(&self, peer: DeviceId) -> Vec<deskfuse_core::ConnectionContext> {
        self.input.contexts(peer)
    }
    fn input_capabilities(
        &self,
        peer: DeviceId,
        session: deskfuse_core::TransportSessionId,
    ) -> Capabilities {
        self.input.capabilities(peer, session)
    }
    fn input_messages(&self) -> InputTransportStream {
        self.input.stream()
    }
    fn send_input(
        &self,
        peer: DeviceId,
        session: deskfuse_core::TransportSessionId,
        message: deskfuse_core::InputShareMessage,
    ) -> Result<(), TransportError> {
        if self.is_revoked(peer)? {
            return Err(TransportError::RevokedPeer(peer));
        }
        self.input.send(peer, session, message)
    }
    fn input_motion_available(
        &self,
        peer: DeviceId,
        session: deskfuse_core::TransportSessionId,
    ) -> bool {
        !self.is_revoked(peer).unwrap_or(true) && self.input.motion_available(peer, session)
    }
    fn send_input_motion(
        &self,
        peer: DeviceId,
        session: deskfuse_core::TransportSessionId,
        motion: deskfuse_core::MotionDatagram,
    ) -> Result<(), TransportError> {
        if self.is_revoked(peer)? {
            return Err(TransportError::RevokedPeer(peer));
        }
        self.input.send_motion(peer, session, motion)
    }
    fn send_input_relative(
        &self,
        peer: DeviceId,
        session: deskfuse_core::TransportSessionId,
        motion: deskfuse_core::RelativeMotionDatagram,
    ) -> Result<(), TransportError> {
        if self.is_revoked(peer)? {
            return Err(TransportError::RevokedPeer(peer));
        }
        self.input.send_relative(peer, session, motion)
    }
    fn cancel_input_motion(
        &self,
        peer: DeviceId,
        session: deskfuse_core::TransportSessionId,
        control: deskfuse_core::InputSessionId,
    ) {
        self.input.cancel_motion(peer, session, control);
    }
    fn input_motion_diagnostics(
        &self,
        peer: DeviceId,
        session: deskfuse_core::TransportSessionId,
    ) -> InputMotionDiagnostics {
        self.input.motion_diagnostics(peer, session)
    }
    async fn register_peer_identity(
        &self,
        peer: DeviceId,
        public_key: [u8; protocol::ED25519_PUBLIC_KEY_SIZE],
    ) -> Result<(), TransportError> {
        self.register_peer_identity_sync(peer, public_key)
    }

    async fn connect(&self, peer: DeviceId) -> Result<Connection, TransportError> {
        self.connect_at(peer, self.config.remote_addr).await
    }

    async fn connect_with_route(
        &self,
        peer: DeviceId,
        endpoint: Endpoint,
    ) -> Result<Connection, TransportError> {
        let address = resolve_endpoint(&endpoint).await?;
        self.connect_at(peer, address).await
    }

    async fn probe_endpoint(&self, endpoint: Endpoint) -> Result<PeerHello, TransportError> {
        let address = resolve_endpoint(&endpoint).await?;
        let (connection, remote) = self
            .connect_and_hello_expected(None, address, None, true)
            .await?;
        connection.close(QUIC_CLOSE_CODE, b"endpoint identity probe complete");
        Ok(remote)
    }

    async fn bootstrap_connect(
        &self,
        peer: DeviceId,
        endpoint: Endpoint,
    ) -> Result<PeerHello, TransportError> {
        let address = resolve_endpoint(&endpoint).await?;
        self.bootstrap_at(peer, address).await
    }

    async fn revoke_peer(&self, peer: DeviceId) -> Result<(), TransportError> {
        QuinnTransport::revoke_peer(self, peer).await
    }

    async fn restore_peer(&self, peer: DeviceId) -> Result<(), TransportError> {
        QuinnTransport::restore_peer(self, peer).await
    }

    async fn shutdown(&self) -> Result<(), TransportError> {
        QuinnTransport::shutdown(self).await
    }

    async fn send_message(&self, peer: DeviceId, message: Message) -> Result<(), TransportError> {
        if matches!(message, Message::InputSharing(_)) {
            return Err(TransportError::MalformedPeer(
                "input requires pinned ordered stream",
            ));
        }
        if self.is_revoked(peer)? {
            return Err(TransportError::RevokedPeer(peer));
        }
        let session = self
            .sessions
            .lock()
            .map_err(|_| TransportError::Closed)?
            .get(&peer)
            .and_then(|sessions| sessions.iter().rev().find(|c| c.close_reason().is_none()))
            .cloned()
            .ok_or(TransportError::UnknownPeer(peer))?;
        if self.is_revoked(peer)? {
            return Err(TransportError::RevokedPeer(peer));
        }
        send_application_frame(
            &session,
            message,
            self.config.transport.idle_timeout_or_handshake(),
        )
        .await
    }

    fn messages(&self) -> TransportMessageStream {
        let receiver = self.inbound.subscribe();
        Box::pin(stream::unfold(receiver, |mut receiver| async move {
            loop {
                match receiver.recv().await {
                    Ok(item) => return Some((item, receiver)),
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        }))
    }
}

impl QuinnTransport {
    async fn connect_at(
        &self,
        peer: DeviceId,
        remote_addr: SocketAddr,
    ) -> Result<Connection, TransportError> {
        let expected_public_key = self
            .registered_identities
            .read()
            .map_err(|_| TransportError::Closed)?
            .get(&peer)
            .copied()
            .or_else(|| {
                self.peers
                    .read()
                    .ok()
                    .and_then(|peers| peers.get(&peer).map(|expected| expected.public_key))
            })
            .ok_or(TransportError::UnknownPeer(peer))?;
        if let Some(expected_peer) = self
            .peers
            .read()
            .map_err(|_| TransportError::Closed)?
            .get(&peer)
            .cloned()
        {
            expected_peer.verify()?;
        }
        let (connection, remote) = self
            .connect_and_hello(peer, remote_addr, Some(expected_public_key))
            .await?;
        // Once registered, shared session maps retain clones. Cancellation
        // must close this exact connection, not a newer peer-level session.
        let mut pending = PendingControlConnection(Some(connection.clone()));
        if self.bound_interface.is_none() {
            // Signed Hello proves identity, while Pong proves the receiver admitted this as an application session.
            let ready = async {
                let (mut send, mut recv) = connection
                    .open_bi()
                    .await
                    .map_err(|_| TransportError::Closed)?;
                write_frame(
                    &mut send,
                    &Envelope::new(HELLO_REQUEST_ID, Message::Ping(Ping { nonce: vec![7; 32] })),
                )
                .await?;
                send.finish().map_err(|_| TransportError::Closed)?;
                match read_frame(&mut recv, self.config.transport.handshake_timeout)
                    .await?
                    .message
                {
                    Some(Message::Pong(p)) if p.nonce == [7; 32] => Ok(()),
                    _ => Err(TransportError::Closed),
                }
            }
            .await;
            if let Err(error) = ready {
                if suspended_by_peer(&connection).await {
                    return Err(TransportError::PeerSuspended(peer));
                }
                connection.close(
                    QUIC_CLOSE_CODE,
                    b"receiver has not admitted application session",
                );
                return Err(error);
            }
            self.register_session(peer, &connection)?;
        }
        let negotiated = self.local_hello.capabilities & remote.capabilities;
        if let Some(interface) = self.bound_interface.as_ref() {
            if !self
                .input
                .register(
                    peer,
                    connection.clone(),
                    self.bound_addr,
                    interface.clone(),
                    negotiated,
                )
                .await
            {
                connection.close(QUIC_CLOSE_CODE, b"input eligibility rejected");
                return Err(TransportError::MalformedPeer(
                    "input requires signed capability and bound physical LAN",
                ));
            }
        } else {
            let shared = self.shared_state();
            shared.io_executor.clone().spawn(serve_connection(
                connection.clone(),
                peer,
                shared,
                negotiated,
            ));
        }
        pending.0 = None;
        Ok(Connection::quinn(
            peer,
            QuinnConnection {
                connection,
                remote,
                config: self.config.transport,
                counters: Arc::clone(&self.counters),
                revoked: Arc::clone(&self.revoked),
            },
        ))
    }

    async fn bootstrap_at(
        &self,
        peer: DeviceId,
        remote_addr: SocketAddr,
    ) -> Result<PeerHello, TransportError> {
        let (connection, remote) = self.connect_and_hello(peer, remote_addr, None).await?;
        // Bootstrap is intentionally a one-shot proof. Do not register this
        // raw session and never expose it to the control-RPC server.
        connection.close(QUIC_CLOSE_CODE, b"bootstrap complete");
        Ok(remote)
    }

    async fn connect_and_hello(
        &self,
        peer: DeviceId,
        remote_addr: SocketAddr,
        expected_public_key: Option<[u8; 32]>,
    ) -> Result<(quinn::Connection, PeerHello), TransportError> {
        self.connect_and_hello_mode(peer, remote_addr, expected_public_key, false)
            .await
    }
    async fn connect_and_hello_mode(
        &self,
        peer: DeviceId,
        remote_addr: SocketAddr,
        expected_public_key: Option<[u8; protocol::ED25519_PUBLIC_KEY_SIZE]>,
        network_only: bool,
    ) -> Result<(quinn::Connection, PeerHello), TransportError> {
        self.connect_and_hello_expected(Some(peer), remote_addr, expected_public_key, network_only)
            .await
    }
    // None is used solely by the one-shot manual network identity probe. It
    // does not restore revoked identities or expose an application session.
    async fn connect_and_hello_expected(
        &self,
        peer: Option<DeviceId>,
        remote_addr: SocketAddr,
        expected_public_key: Option<[u8; protocol::ED25519_PUBLIC_KEY_SIZE]>,
        network_only: bool,
    ) -> Result<(quinn::Connection, PeerHello), TransportError> {
        use std::sync::atomic::Ordering::Relaxed;
        self.counters.connect.fetch_add(1, Relaxed);
        if let Some(peer) = peer
            && self.is_revoked(peer)?
        {
            return Err(TransportError::RevokedPeer(peer));
        }
        let endpoint = self
            .endpoint
            .as_ref()
            .ok_or(TransportError::ConfigurationRequired(
                "call QuinnTransport::bind with pinned certificate material",
            ))?;
        let connecting = endpoint
            .connect(remote_addr, &self.config.server_name)
            .map_err(|error| TransportError::Quic(error.to_string()))?;
        let connection = time::timeout(self.config.transport.handshake_timeout, connecting)
            .await
            .map_err(|_| TransportError::Timeout {
                operation: "handshake",
            })?
            .map_err(|error| TransportError::Quic(error.to_string()))?;
        if let Some(peer) = peer
            && self.is_revoked(peer)?
        {
            connection.close(QUIC_CLOSE_CODE, b"peer revoked");
            return Err(TransportError::RevokedPeer(peer));
        }
        let mut pending = PendingControlConnection(Some(connection.clone()));
        let (mut send, mut recv) = time::timeout(
            self.config.transport.handshake_timeout,
            connection.open_bi(),
        )
        .await
        .map_err(|_| TransportError::Timeout { operation: "hello" })?
        .map_err(|error| TransportError::Quic(error.to_string()))?;
        let request = Envelope::new(
            if network_only {
                network::NETWORK_HELLO_ID
            } else {
                HELLO_REQUEST_ID
            },
            Message::Hello(self.local_hello.to_wire()),
        );
        write_frame(&mut send, &request).await?;
        send.finish()
            .map_err(|error| TransportError::Quic(error.to_string()))?;
        let response = read_frame(&mut recv, self.config.transport.handshake_timeout).await?;
        let remote = match response.message {
            Some(Message::Hello(hello)) => PeerHello::from_wire(hello)?,
            Some(Message::HelloAck(_)) => {
                return Err(TransportError::Protocol(ProtocolError::InvalidPayload(
                    "hello ack does not carry a verifiable public key",
                )));
            }
            _ => return Err(TransportError::Protocol(ProtocolError::UnknownPayload)),
        };
        if let Some(peer) = peer
            && remote.device_id != peer
        {
            connection.close(QUIC_CLOSE_CODE, b"identity mismatch");
            return Err(TransportError::IdentityMismatch {
                expected: peer,
                actual: remote.device_id,
            });
        }
        if expected_public_key.is_some_and(|key| key != remote.public_key) {
            connection.close(QUIC_CLOSE_CODE, b"identity mismatch");
            return Err(TransportError::IdentityMismatch {
                expected: peer.unwrap_or(remote.device_id),
                actual: remote.device_id,
            });
        }
        if let Some(peer) = peer
            && self.is_revoked(peer)?
        {
            connection.close(QUIC_CLOSE_CODE, b"peer revoked");
            return Err(TransportError::RevokedPeer(peer));
        }
        if let Err(error) = session_auth::client(
            &connection,
            &self.local_hello,
            &remote,
            &self.identity,
            network_only,
            self.config.transport.handshake_timeout,
        )
        .await
        {
            // 对方认证后立即以"已挂起"关闭时，本端的认证收尾可能先失败；以关闭原因为准。
            if suspended_by_peer(&connection).await {
                return Err(TransportError::PeerSuspended(remote.device_id));
            }
            return Err(error);
        }
        pending.0 = None;
        Ok((connection, remote))
    }

    async fn revoke_peer(&self, peer: DeviceId) -> Result<(), TransportError> {
        self.input.invalidate_peer(peer);
        // Serialize the revocation marker and session drain with session
        // registration. This closes every raw incoming and outgoing session,
        // including sessions established concurrently with the revoke call.
        let sessions = {
            let mut sessions = self.sessions.lock().map_err(|_| TransportError::Closed)?;
            self.revoked
                .write()
                .map_err(|_| TransportError::Closed)?
                .insert(peer);
            sessions.remove(&peer).unwrap_or_default()
        };
        self.peers
            .write()
            .map_err(|_| TransportError::Closed)?
            .remove(&peer);
        self.registered_identities
            .write()
            .map_err(|_| TransportError::Closed)?
            .remove(&peer);
        for connection in sessions {
            connection.close(QUIC_CLOSE_CODE, b"peer revoked");
        }
        Ok(())
    }

    /// Clears only the revocation marker. The peer still has to verify and
    /// register its identity before any session is accepted again.
    pub async fn restore_peer(&self, peer: DeviceId) -> Result<(), TransportError> {
        self.revoked
            .write()
            .map_err(|_| TransportError::Closed)?
            .remove(&peer);
        Ok(())
    }
}

async fn resolve_endpoint(endpoint: &Endpoint) -> Result<SocketAddr, TransportError> {
    if let Ok(address) = endpoint.host.parse::<std::net::IpAddr>() {
        return Ok(SocketAddr::new(address, endpoint.port));
    }
    tokio::net::lookup_host((endpoint.host.as_str(), endpoint.port))
        .await?
        .next()
        .ok_or(TransportError::Quic(format!(
            "endpoint {} did not resolve to an address",
            endpoint
        )))
}

#[derive(Debug)]
struct QuinnConnection {
    connection: quinn::Connection,
    remote: PeerHello,
    config: TransportConfig,
    counters: Arc<Counters>,
    revoked: Arc<RwLock<BTreeSet<DeviceId>>>,
}

impl QuinnConnection {
    fn check_allowed(&self) -> Result<(), TransportError> {
        if self
            .revoked
            .read()
            .map_err(|_| TransportError::Closed)?
            .contains(&self.remote.device_id)
        {
            return Err(TransportError::RevokedPeer(self.remote.device_id));
        }
        if self.connection.close_reason().is_some() {
            return Err(TransportError::Closed);
        }
        Ok(())
    }

    async fn send_message(&self, message: Message) -> Result<(), TransportError> {
        self.check_allowed()?;
        send_application_frame(
            &self.connection,
            message,
            self.config.idle_timeout_or_handshake(),
        )
        .await
    }

    async fn hello(&self) -> Result<PeerHello, TransportError> {
        use std::sync::atomic::Ordering::Relaxed;
        self.counters.hello.fetch_add(1, Relaxed);
        self.check_allowed()?;
        Ok(self.remote.clone())
    }

    async fn ping(&self, nonce: Vec<u8>) -> Result<Pong, TransportError> {
        use std::sync::atomic::Ordering::Relaxed;
        self.counters.ping.fetch_add(1, Relaxed);
        self.check_allowed()?;
        let (mut send, mut recv) = time::timeout(
            self.config.idle_timeout_or_handshake(),
            self.connection.open_bi(),
        )
        .await
        .map_err(|_| TransportError::Timeout { operation: "ping" })?
        .map_err(|error| TransportError::Quic(error.to_string()))?;
        self.check_allowed()?;
        send.set_priority(i32::MAX)
            .map_err(|error| TransportError::Quic(error.to_string()))?;
        let envelope = Envelope::new(
            [1; protocol::REQUEST_ID_SIZE],
            Message::Ping(Ping { nonce }),
        );
        write_frame(&mut send, &envelope).await?;
        send.finish()
            .map_err(|error| TransportError::Quic(error.to_string()))?;
        let response = read_frame(&mut recv, self.config.idle_timeout_or_handshake()).await?;
        match response.message {
            Some(Message::Pong(pong)) => Ok(pong),
            Some(Message::Error(error)) => Err(TransportError::Quic(error.message)),
            _ => Err(TransportError::Protocol(ProtocolError::UnknownPayload)),
        }
    }

    async fn close(&self) -> Result<(), TransportError> {
        if self.connection.close_reason().is_some() {
            return Ok(());
        }
        if let Ok(Ok((mut send, mut recv))) =
            time::timeout(self.config.handshake_timeout, self.connection.open_bi()).await
        {
            let envelope = Envelope::new(
                [2; protocol::REQUEST_ID_SIZE],
                Message::Goodbye(protocol::Goodbye {
                    reason: "closed".into(),
                }),
            );
            if write_frame(&mut send, &envelope).await.is_ok() {
                let _ = send.finish();
                let _ = time::timeout(
                    self.config.handshake_timeout,
                    read_frame(&mut recv, self.config.handshake_timeout),
                )
                .await;
            }
        }
        self.connection.close(QUIC_CLOSE_CODE, b"closed");
        self.counters
            .close
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }
}

impl TransportConfig {
    const fn idle_timeout_or_handshake(self) -> Duration {
        if self.idle_timeout.is_zero() {
            self.handshake_timeout
        } else {
            self.idle_timeout
        }
    }
}

/// Reconcile physical-IP listeners after cable hotplug or DHCP changes.
/// Existing sockets and ordinary clipboard connections are retained.
async fn refresh_lan_listeners(shared: QuinnShared, config: QuinnTransportConfig) {
    let input_executor = input_executor::handle().ok();
    let mut interval = time::interval(Duration::from_secs(2));
    interval.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
    interval.tick().await;
    loop {
        if shared.stopping.load(Ordering::Acquire) {
            break;
        }
        tokio::select! {_=shared.shutdown.notified()=>break,_=interval.tick()=>{}}
        let lan_port = shared.lan_port.clone();
        let sockets = tokio::select! {
            _=shared.shutdown.notified()=>break,
            sockets=tokio::task::spawn_blocking(move || lan::listener_sockets(lan_port.as_deref()))=>sockets.unwrap_or_default()
        };
        if shared.stopping.load(Ordering::Acquire) {
            break;
        }
        let active = sockets
            .iter()
            .filter_map(|(name, socket)| socket.local_addr().ok().map(|a| (name.clone(), a.ip())))
            .collect::<BTreeSet<_>>();
        if let Ok(mut listeners) = shared.lan_endpoints.lock() {
            listeners.retain(|address, (name, endpoint)| {
                let keep = active.contains(&(name.clone(), address.ip()));
                if !keep {
                    socket_diag::record(
                        "listener_closed",
                        *address,
                        format!("interface={name} reason=interface_changed"),
                    );
                    endpoint.close(QUIC_CLOSE_CODE, b"physical interface changed");
                }
                keep
            });
        }
        let sockets = if config.input_service {
            let lan_port = shared.lan_port.clone();
            tokio::task::spawn_blocking(move || {
                lan::listener_sockets_on(lan_port.as_deref(), 42425)
            })
            .await
            .unwrap_or_default()
        } else {
            sockets
        };
        for (interface, socket) in sockets {
            let Ok(address) = socket.local_addr() else {
                continue;
            };
            if shared
                .lan_endpoints
                .lock()
                .map(|m| {
                    m.iter()
                        .any(|(a, (name, _))| a.ip() == address.ip() && name == &interface)
                })
                .unwrap_or(true)
            {
                continue;
            }
            let Ok((server, client)) = build_input_quic_configs(&config) else {
                continue;
            };
            let Ok(input_runtime) = input_executor::InputQuicRuntime::new() else {
                continue;
            };
            let Ok(mut endpoint) = quinn::Endpoint::new(
                quinn::EndpointConfig::default(),
                Some(server),
                socket,
                Arc::new(input_runtime),
            ) else {
                continue;
            };
            endpoint.set_default_client_config(client);
            let endpoint = Arc::new(endpoint);
            if let Ok(mut listeners) = shared.lan_endpoints.lock() {
                socket_diag::record("listener_bound", address, format!("interface={interface}"));
                listeners.insert(address, (interface.clone(), endpoint.clone()));
            }
            let mut state = shared.clone();
            if let Some(executor) = input_executor.clone() {
                state.io_executor = executor;
            }
            state.bound_addr = address;
            state.bound_interface = Some(interface);
            let task = state.io_executor.clone().spawn(async move {
                accept_loop(endpoint, state).await;
            });
            if let Ok(mut tasks) = shared.lan_tasks.lock() {
                tasks.retain(|task| !task.is_finished());
                tasks.push(task);
            }
        }
    }
}

async fn accept_loop(endpoint: Arc<quinn::Endpoint>, shared: QuinnShared) {
    loop {
        if shared.stopping.load(Ordering::Acquire) {
            break;
        }
        tokio::select! {
            () = shared.shutdown.notified() => break,
            incoming = endpoint.accept() => {
                let Some(incoming) = incoming else { break; };
                let state = shared.clone();
                shared.io_executor.spawn(async move {
                    if let Ok(Ok(connection)) = time::timeout(
                        state.config.handshake_timeout,
                        incoming,
                    )
                    .await
                    {
                        let _ = handle_incoming(connection, state).await;
                    }
                });
            }
        }
    }
    // Quinn 的 endpoint driver 遇到接收错误（睡眠或断网时绑定网卡的 socket 常见）
    // 会永久退出，accept 随即返回 None。网卡名和 IP 恢复后 refresh 仍会认为监听
    // 健在，对端探测 42425 永远超时；这里摘掉死监听，让下一轮 refresh 重新绑定。
    if shared.bound_interface.is_some() && !shared.stopping.load(Ordering::Acquire) {
        let removed = shared.lan_endpoints.lock().is_ok_and(|mut listeners| {
            listeners
                .get(&shared.bound_addr)
                .is_some_and(|(_, current)| Arc::ptr_eq(current, &endpoint))
                && listeners.remove(&shared.bound_addr).is_some()
        });
        socket_diag::record(
            "listener_accept_ended",
            shared.bound_addr,
            format!(
                "interface={} removed={removed}",
                shared.bound_interface.as_deref().unwrap_or_default()
            ),
        );
    }
}

async fn handle_incoming(
    connection: quinn::Connection,
    shared: QuinnShared,
) -> Result<(), TransportError> {
    shared
        .counters
        .hello
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let (mut send, mut recv) =
        time::timeout(shared.config.handshake_timeout, connection.accept_bi())
            .await
            .map_err(|_| TransportError::Timeout { operation: "hello" })?
            .map_err(|error| TransportError::Quic(error.to_string()))?;
    let request = read_frame(&mut recv, shared.config.handshake_timeout).await?;
    let network_only = request.request_id == network::NETWORK_HELLO_ID;
    let remote = match request.message {
        Some(Message::Hello(hello)) => PeerHello::from_wire(hello)?,
        _ => return Err(TransportError::Protocol(ProtocolError::UnknownPayload)),
    };
    if shared.is_revoked(remote.device_id)? {
        connection.close(QUIC_CLOSE_CODE, b"peer revoked");
        return Err(TransportError::RevokedPeer(remote.device_id));
    }
    let expected_public_key = shared
        .registered_identities
        .read()
        .map_err(|_| TransportError::Closed)?
        .get(&remote.device_id)
        .copied()
        .or_else(|| {
            shared
                .peers
                .read()
                .ok()
                .and_then(|peers| peers.get(&remote.device_id).map(|peer| peer.public_key))
        });
    if let Some(expected_public_key) = expected_public_key
        && expected_public_key != remote.public_key
    {
        connection.close(QUIC_CLOSE_CODE, b"identity mismatch");
        return Err(TransportError::IdentityMismatch {
            expected: remote.device_id,
            actual: remote.device_id,
        });
    }
    let response = Envelope::new(
        HELLO_REQUEST_ID,
        Message::Hello(shared.local_hello.to_wire()),
    );
    write_frame(&mut send, &response).await?;
    send.finish()
        .map_err(|error| TransportError::Quic(error.to_string()))?;

    session_auth::server(
        &connection,
        &shared.local_hello,
        &remote,
        &shared.identity,
        network_only,
        shared.config.handshake_timeout,
    )
    .await?;
    if network_only && shared.bound_interface.is_some() {
        let _ = connection.closed().await;
        return Ok(());
    }
    if network_only {
        if !(shared.local_hello.capabilities & remote.capabilities)
            .contains(Capabilities::DEVICE_NETWORK)
        {
            return Err(TransportError::MalformedPeer(
                "device-network upgrade required",
            ));
        }
        if shared.bound_interface.is_some() {
            return Err(TransportError::Closed);
        }
        let result = shared.network.serve(connection.clone(), remote).await;
        connection.close(QUIC_CLOSE_CODE, b"network-only connection complete");
        return result;
    }

    // Discovery-only peers are allowed to prove ownership, but must not get a
    // control-RPC server until the runtime registers their identity. This is
    // the incoming half of the bootstrap seam.
    //
    // The connection must stay alive until the client reads the final Hello
    // frame: returning here would drop this task's `quinn::Connection` and
    // race the client's read. The client closes the one-shot bootstrap
    // session after reading; the idle timeout reaps a client that disappears
    // without closing.
    if expected_public_key.is_none() {
        let _ = connection.closed().await;
        return Ok(());
    }
    if shared.is_revoked(remote.device_id)? {
        connection.close(QUIC_CLOSE_CODE, b"peer revoked");
        return Err(TransportError::RevokedPeer(remote.device_id));
    }
    if shared.bound_interface.is_none() {
        // 本机主动断开的设备：身份认证后明确拒绝，而不是让它单方面连上。
        // 拨号方据关闭原因显示"对方已断开"，两端都不会把这条连接当作在线。
        let admitted = shared
            .admission
            .lock()
            .map_err(|_| TransportError::Closed)?
            .admit(remote.device_id, &connection);
        if !admitted {
            connection.close(QUIC_CLOSE_CODE, PEER_SUSPENDED_REASON);
            return Err(TransportError::PeerSuspended(remote.device_id));
        }
        shared.register_session(remote.device_id, &connection)?;
    }
    let negotiated = shared.local_hello.capabilities & remote.capabilities;
    if let Some(interface) = shared.bound_interface.as_ref()
        && !shared
            .input
            .register(
                remote.device_id,
                connection.clone(),
                shared.bound_addr,
                interface.clone(),
                negotiated,
            )
            .await
    {
        connection.close(QUIC_CLOSE_CODE, b"input eligibility rejected");
        return Err(TransportError::MalformedPeer(
            "input requires signed capability and bound physical LAN",
        ));
    }
    if shared.is_revoked(remote.device_id)? {
        connection.close(QUIC_CLOSE_CODE, b"peer revoked");
        return Err(TransportError::RevokedPeer(remote.device_id));
    }
    if shared.bound_interface.is_none() {
        // The control endpoint both listens and dials, unlike the ephemeral
        // physical input endpoints. Its authenticated source is a return route.
        let address = connection.remote_address();
        let _ = shared.control_peers.send(ControlPeerConnected {
            hello: remote.clone(),
            endpoint: Endpoint {
                host: address.ip().to_string(),
                port: address.port(),
            },
        });
        tokio::spawn(serve_connection(
            connection,
            remote.device_id,
            shared,
            negotiated,
        ));
    }
    Ok(())
}

async fn serve_connection(
    connection: quinn::Connection,
    peer: DeviceId,
    shared: QuinnShared,
    negotiated: Capabilities,
) {
    shared.microphone.register(&connection, negotiated);
    // Read independently so a large clipboard body cannot hold up Ping/Pong.
    // Application frames are still published in stream creation order.
    let mut readers = tokio::task::JoinSet::new();
    let mut pending = BTreeMap::new();
    let mut next = 0u64;
    let mut accepted = 0u64;
    loop {
        if shared.is_revoked(peer).unwrap_or(true) {
            connection.close(QUIC_CLOSE_CODE, b"peer revoked");
            break;
        }
        tokio::select! {
            _ = connection.closed() => break,
            _ = shared.shutdown.notified() => break,
            incoming = connection.accept_bi(), if readers.len() + pending.len() < 8 => {
                let Ok((send, mut recv)) = incoming else { break; };
                let ordinal = accepted;
                accepted += 1;
                let timeout = shared.config.idle_timeout_or_handshake();
                readers.spawn(async move { let request = read_control_frame(&mut recv, timeout).await; (ordinal, send, recv, request) });
            }
            done = readers.join_next(), if !readers.is_empty() => {
                let Some(Ok((ordinal, mut send, recv, request))) = done else { connection.close(QUIC_CLOSE_CODE,b"control reader failed"); break; };
                if let Ok(request) = request {
                    let request_id = request_id_or_zero(&request);
                    match request.message {
                        Some(Message::MicrophoneOpen(open)) => {
                            pending.insert(ordinal, None);
                            if shared.is_revoked(peer).unwrap_or(true) {
                                let _ = send.reset(1u32.into());
                            } else if open.version == 2 && open.status == 0 && negotiated.contains(Capabilities::MICROPHONE) {
                                let enabled = shared.microphone.source_enabled.load(Ordering::Acquire);
                                tokio::spawn(async move {
                                    let reply = Envelope::new(request_id, Message::MicrophoneOpen(protocol::MicrophoneOpen {
                                        version: 2, status: if enabled { 1 } else { 2 },
                                    }));
                                    if time::timeout(Duration::from_millis(500), write_frame(&mut send, &reply)).await.is_ok_and(|r| r.is_ok()) {
                                        let _ = send.finish();
                                    }
                                });
                            } else if open.version == 1 && open.status == 0 && negotiated.contains(Capabilities::MICROPHONE) {
                                shared.microphone.accept(peer, send, recv);
                            } else {
                                // Never silently drop an audio request: an empty
                                // FIN appears to users as "0 bytes read".
                                tokio::spawn(async move {
                                    let reply = Envelope::new(request_id, Message::Error(protocol::Error {
                                        code: 426,
                                        message: "microphone capability not negotiated".into(),
                                        retryable: false,
                                    }));
                                    if time::timeout(Duration::from_millis(500), write_frame(&mut send, &reply)).await.is_ok_and(|r| r.is_ok()) {
                                        let _ = send.finish();
                                    }
                                });
                            }
                        }
                        Some(Message::RemoteRdpTunnel(open)) => {
                            pending.insert(ordinal, None);
                            // REMOTE_APPLICATION_RDP 表示“本机是 RDP 主机”，macOS 查看端不会声明它。
                            // 不能要求双方协商出该位，否则查看端发起的隧道会被静默丢弃。
                            if shared.local_hello.capabilities.contains(Capabilities::REMOTE_APPLICATION_RDP)
                                && negotiated.contains(Capabilities::REMOTE_APPLICATION)
                                && !shared.is_revoked(peer).unwrap_or(true) {
                                let hub = shared.rdp.clone();
                                tokio::spawn(async move { hub.accept(peer, open.ticket, send, recv).await; });
                            }
                        }
                        Some(Message::RemoteApplicationOpen(_)) => {
                            pending.insert(ordinal, None);
                            if negotiated.contains(Capabilities::REMOTE_APPLICATION) && !shared.is_revoked(peer).unwrap_or(true) {
                                let hub = shared.remote_app.clone();
                                let app_connection = connection.clone();
                                tokio::spawn(async move {
                                    hub.accept(peer, app_connection, send, recv).await;
                                });
                            }
                        }
                        Some(message @ (Message::ClipboardUpdate(_) | Message::InputEventBatch(_) | Message::DisplayInfo(_))) => {
                            pending.insert(ordinal, Some(message));
                        }
                        Some(message) => {
                            pending.insert(ordinal, None);
                            let state = shared.clone();
                            tokio::spawn(async move {
                    let response = match message {
                        Message::InputEndpoints(query)
                            if negotiated.contains(Capabilities::INPUT_SHARING)
                                && query.addresses.is_empty() =>
                        {
                            let addresses = state
                                .lan_endpoints
                                .lock()
                                .map(|e| e.keys().take(32).map(ToString::to_string).collect())
                                .unwrap_or_default();
                            Envelope::new(
                                request_id,
                                Message::InputEndpoints(protocol::InputEndpoints { addresses }),
                            )
                        }
                        Message::Ping(ping) => {
                            state
                                .counters
                                .ping
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            Envelope::new(request_id, Message::Pong(Pong { nonce: ping.nonce }))
                        }
                        Message::Goodbye(goodbye) => {
                            state
                                .counters
                                .close
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            Envelope::new(request_id, Message::Goodbye(goodbye))
                        }
                        Message::Hello(_) => Envelope::new(
                            request_id,
                            Message::Error(protocol::Error {
                                code: 400,
                                message: "hello already completed".into(),
                                retryable: false,
                            }),
                        ),
                        _ => Envelope::new(
                            request_id,
                            Message::Error(protocol::Error {
                                code: 400,
                                message: "unsupported control message".into(),
                                retryable: false,
                            }),
                        ),
                    };
                                if state.is_revoked(peer).unwrap_or(true) { return; }
                                if send.set_priority(i32::MAX).is_err() { return; }
                                if time::timeout(Duration::from_secs(2),write_frame(&mut send,&response)).await.is_ok_and(|r|r.is_ok()) { let _=send.finish(); }
                            });
                        }
                        None => { pending.insert(ordinal,None); }
                    }
                } else { pending.insert(ordinal,None); }
                while let Some(message) = pending.remove(&next) {
                    if shared.is_revoked(peer).unwrap_or(true) { break; }
                    if let Some(message) = message { let _=shared.inbound.send((peer,message)); }
                    next += 1;
                }
            }
        }
    }
    shared.microphone.unregister(&connection);
    readers.abort_all();
    while readers.join_next().await.is_some() {}
}

fn request_id_or_zero(envelope: &Envelope) -> [u8; protocol::REQUEST_ID_SIZE] {
    envelope
        .request_id
        .as_slice()
        .try_into()
        .unwrap_or([0; protocol::REQUEST_ID_SIZE])
}

/// A completed write only means Quinn buffered the bytes. Wait for delivery
/// before the per-peer writer opens its next stream, bounding queued file data
/// and preventing it from filling every control reader ahead of a heartbeat.
/// File success still requires the receiver's separate integrity/pasteboard ACK.
async fn send_application_frame(
    connection: &quinn::Connection,
    message: Message,
    timeout: Duration,
) -> Result<(), TransportError> {
    time::timeout(timeout, async {
        let (mut send, _recv) = connection
            .open_bi()
            .await
            .map_err(|error| TransportError::Quic(error.to_string()))?;
        let envelope = Envelope::new([0_u8; protocol::REQUEST_ID_SIZE], message);
        write_frame(&mut send, &envelope).await?;
        send.finish()
            .map_err(|error| TransportError::Quic(error.to_string()))?;
        match send.stopped().await {
            Ok(None) => Ok(()),
            Ok(Some(_)) => Err(TransportError::Closed),
            Err(error) => Err(TransportError::Quic(error.to_string())),
        }
    })
    .await
    .map_err(|_| TransportError::Timeout {
        operation: "deliver application frame",
    })?
}

/// Each control stream carries exactly one frame. Consume FIN before dropping
/// the reader so an otherwise complete send is not reset by RecvStream::drop.
async fn read_control_frame(
    recv: &mut quinn::RecvStream,
    timeout: Duration,
) -> Result<Envelope, TransportError> {
    let envelope = read_frame(recv, timeout).await?;
    if matches!(
        envelope.message,
        Some(
            Message::RemoteApplicationOpen(_)
                | Message::MicrophoneOpen(_)
                | Message::RemoteRdpTunnel(_)
        )
    ) {
        return Ok(envelope);
    }
    let mut trailing = [0_u8; 1];
    match time::timeout(timeout, recv.read(&mut trailing))
        .await
        .map_err(|_| TransportError::Timeout {
            operation: "finish control frame",
        })?
        .map_err(|error| TransportError::Quic(error.to_string()))?
    {
        None => Ok(envelope),
        Some(_) => Err(TransportError::MalformedPeer("trailing control frame data")),
    }
}

async fn write_frame(
    send: &mut quinn::SendStream,
    envelope: &Envelope,
) -> Result<(), TransportError> {
    let frame = protocol::encode_frame(envelope).map_err(TransportError::Protocol)?;
    send.write_all(&frame).await.map_err(|error| {
        TransportError::Io(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            error.to_string(),
        ))
    })
}

async fn read_frame(
    recv: &mut quinn::RecvStream,
    timeout: Duration,
) -> Result<Envelope, TransportError> {
    read_frame_limited(recv, timeout, MAX_RPC_FRAME).await
}
async fn read_input_frame(
    recv: &mut quinn::RecvStream,
    timeout: Duration,
) -> Result<Envelope, TransportError> {
    // InputSharing permits a 96 KiB serialized wallpaper payload because a
    // 64 KiB JPEG is base64-encoded inside JSON. Keep the stream bound in
    // sync with that protocol allowance; `to_core` still rejects oversized
    // ordinary input messages after the envelope is decoded.
    read_frame_limited(
        recv,
        timeout,
        deskfuse_protocol::wire::MAX_INPUT_WALLPAPER_PAYLOAD,
    )
    .await
}
async fn read_frame_limited(
    recv: &mut quinn::RecvStream,
    timeout: Duration,
    max_frame: usize,
) -> Result<Envelope, TransportError> {
    read_frame_or_eof(recv, timeout, max_frame)
        .await?
        .ok_or_else(|| {
            TransportError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "stream finished before a frame",
            ))
        })
}

/// 与 `read_frame_limited` 相同，但把"对端在帧边界正常结束流"返回为 `None`，
/// 供一次请求对应多帧应答的流式读取判断终点。
async fn read_frame_or_eof(
    recv: &mut quinn::RecvStream,
    timeout: Duration,
    max_frame: usize,
) -> Result<Option<Envelope>, TransportError> {
    let mut prefix = [0_u8; 4];
    let first = time::timeout(timeout, recv.read(&mut prefix[..1]))
        .await
        .map_err(|_| TransportError::Timeout {
            operation: "read frame",
        })?
        .map_err(|error| {
            TransportError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                error.to_string(),
            ))
        })?;
    if first.is_none() {
        return Ok(None);
    }
    time::timeout(timeout, recv.read_exact(&mut prefix[1..]))
        .await
        .map_err(|_| TransportError::Timeout {
            operation: "read frame",
        })?
        .map_err(|error| {
            TransportError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                error.to_string(),
            ))
        })?;
    let length = u32::from_be_bytes(prefix) as usize;
    if length > max_frame {
        return Err(TransportError::Protocol(ProtocolError::FrameTooLarge {
            size: length,
            max: max_frame,
        }));
    }
    let mut payload = vec![0_u8; length];
    time::timeout(timeout, recv.read_exact(&mut payload))
        .await
        .map_err(|_| TransportError::Timeout {
            operation: "read frame",
        })?
        .map_err(|error| {
            TransportError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                error.to_string(),
            ))
        })?;
    let mut frame = Vec::with_capacity(4 + length);
    frame.extend_from_slice(&prefix);
    frame.extend_from_slice(&payload);
    protocol::decode_frame(&frame)
        .map(Some)
        .map_err(TransportError::Protocol)
}

fn bytes_to_device_id(bytes: &[u8]) -> Result<DeviceId, TransportError> {
    let array: [u8; protocol::DEVICE_ID_SIZE] = bytes
        .try_into()
        .map_err(|_| TransportError::MalformedPeer("device id length"))?;
    Ok(DeviceId::from_bytes(array))
}

fn build_input_quic_configs(
    config: &QuinnTransportConfig,
) -> Result<(quinn::ServerConfig, quinn::ClientConfig), TransportError> {
    let (mut server, mut client) = build_quic_configs(config)?;
    let mut transport = quinn::TransportConfig::default();
    transport.max_concurrent_uni_streams(64_u32.into());
    // Dedicated input links only: bound stale motion buffering in both directions.
    transport.datagram_receive_buffer_size(Some(8 * protocol::MAX_MOTION_DATAGRAM_SIZE));
    transport.datagram_send_buffer_size(2 * protocol::MAX_MOTION_DATAGRAM_SIZE);
    transport.stream_receive_window(quinn::VarInt::from_u32(128 * 1024));
    transport.receive_window(quinn::VarInt::from_u32(4 * 1024 * 1024));
    transport.max_idle_timeout(Some(
        quinn::IdleTimeout::try_from(Duration::from_secs(30))
            .map_err(|_| TransportError::Closed)?,
    ));
    let transport = Arc::new(transport);
    server.transport_config(transport.clone());
    client.transport_config(transport);
    server.migration(false);
    Ok((server, client))
}

fn build_quic_configs(
    config: &QuinnTransportConfig,
) -> Result<(quinn::ServerConfig, quinn::ClientConfig), TransportError> {
    if config.certificate_der.is_empty()
        || config.private_key_der.is_empty()
        || config.trusted_certificate_der.is_empty()
    {
        return Err(TransportError::ConfigurationRequired(
            "DER certificate, PKCS#8 key, and pinned trust certificate",
        ));
    }
    // Explicitly install the ring provider.  No dangerous/custom verifier is
    // enabled; the client trusts exactly the configured certificate.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let certificate = rustls::pki_types::CertificateDer::from(config.certificate_der.clone());
    let key = rustls::pki_types::PrivatePkcs8KeyDer::from(config.private_key_der.clone());
    let mut server_crypto = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![certificate], key.into())
        .map_err(|error| TransportError::Quic(error.to_string()))?;
    server_crypto.alpn_protocols = vec![protocol::ALPN.to_vec()];
    let mut server = quinn::ServerConfig::with_crypto(Arc::new(
        quinn::crypto::rustls::QuicServerConfig::try_from(server_crypto)
            .map_err(|error| TransportError::Quic(error.to_string()))?,
    ));
    let mut server_transport = quinn::TransportConfig::default();
    let idle = quinn::IdleTimeout::try_from(config.transport.idle_timeout_or_handshake())
        .map_err(|error| TransportError::Quic(error.to_string()))?;
    // Remote application frames are compressed but can still be multi-megabyte
    // key frames. Keep the control connection's stream and connection windows
    // above one frame so QUIC flow control does not stall a media write while
    // the peer is draining it. Input-only endpoints replace these values in
    // `build_input_quic_configs` with their smaller LAN-specific windows.
    server_transport.stream_receive_window(quinn::VarInt::from_u32(32 * 1024 * 1024));
    server_transport.receive_window(quinn::VarInt::from_u32(64 * 1024 * 1024));
    server_transport.max_idle_timeout(Some(idle));
    server_transport.max_concurrent_uni_streams(0_u8.into());
    server_transport.datagram_receive_buffer_size(None);
    server_transport.datagram_send_buffer_size(0);
    server.transport = Arc::new(server_transport);

    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(rustls::pki_types::CertificateDer::from(
            config.trusted_certificate_der.clone(),
        ))
        .map_err(|error| TransportError::Quic(error.to_string()))?;
    let mut client_crypto = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    client_crypto.alpn_protocols = vec![protocol::ALPN.to_vec()];
    let mut client = quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(client_crypto)
            .map_err(|error| TransportError::Quic(error.to_string()))?,
    ));
    let mut client_transport = quinn::TransportConfig::default();
    client_transport.stream_receive_window(quinn::VarInt::from_u32(32 * 1024 * 1024));
    client_transport.receive_window(quinn::VarInt::from_u32(64 * 1024 * 1024));
    client_transport.max_idle_timeout(Some(idle));
    client_transport.keep_alive_interval(Some(Duration::from_secs(10)));
    client_transport.max_concurrent_uni_streams(0_u8.into());
    client_transport.datagram_receive_buffer_size(None);
    client_transport.datagram_send_buffer_size(0);
    client.transport_config(Arc::new(client_transport));
    Ok((server, client))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgen::generate_simple_self_signed;
    use std::net::Ipv4Addr;

    #[test]
    fn input_link_failures_keep_every_path_category_and_classify_the_cause() {
        use deskfuse_core::InputLinkFailure as Failure;
        // 笔记本 → 公司电脑：一条路径路由不一致被拒，两条局域网路径探测超时。
        let mut failures = InputLinkFailures::default();
        failures.record(
            InputPathFailure::Route,
            "local=172.18.172.190 route=bridge0".into(),
        );
        failures.record(
            InputPathFailure::Probe,
            "remote=10.10.10.2 stage=identity_probe".into(),
        );
        failures.record(
            InputPathFailure::Probe,
            "remote=172.18.175.223 stage=identity_probe".into(),
        );
        assert_eq!(failures.classify(3, 2, 2), Failure::PeerUnresponsive);
        let summary = failures.summary();
        assert!(
            summary.contains("Probe=2") && summary.contains("Route=1"),
            "{summary}"
        );
        // 后来的路由拒绝不会挤掉探测失败的明细。
        assert!(
            summary.contains("172.18.175.223") && summary.contains("bridge0"),
            "{summary}"
        );

        // 笔记本 → 家里设备：所有路由都经过代理 TUN。
        let mut overlay = InputLinkFailures::default();
        overlay.record(InputPathFailure::Overlay, "route=utun8 kind=Vpn".into());
        overlay.record(InputPathFailure::Overlay, "route=utun8 kind=Vpn".into());
        assert_eq!(overlay.classify(3, 2, 0), Failure::OverlayOnly);
        overlay.record(InputPathFailure::Route, "route=en0".into());
        assert_eq!(overlay.classify(3, 2, 0), Failure::NoLanRoute);

        let empty = InputLinkFailures::default();
        assert_eq!(empty.classify(0, 2, 0), Failure::NoLocalLan);
        assert_eq!(empty.classify(2, 0, 0), Failure::NoPeerEndpoints);
        assert_eq!(empty.classify(2, 2, 0), Failure::Other);
    }

    #[test]
    fn input_link_failure_details_stay_bounded() {
        let mut failures = InputLinkFailures::default();
        for kind in [
            InputPathFailure::Probe,
            InputPathFailure::Connect,
            InputPathFailure::Overlay,
            InputPathFailure::Route,
            InputPathFailure::Other,
        ] {
            failures.record(kind, "界".repeat(400));
        }
        let summary = failures.summary();
        // 三条明细（每条截断到 200 字节以内）加计数，仍远小于 1024 字节的日志字段。
        assert!(summary.len() < 760, "{}", summary.len());
    }

    #[test]
    fn lan_fingerprint_tracks_interface_addresses_not_order_or_port() {
        let wifi = ("en0", "10.0.0.44".parse::<std::net::IpAddr>().unwrap());
        let bridge = ("bridge0", "10.10.10.1".parse::<std::net::IpAddr>().unwrap());
        let both = lan_fingerprint([wifi, bridge].into_iter());
        assert_eq!(both, lan_fingerprint([bridge, wifi].into_iter()));
        // 拔掉雷电线：bridge0 地址消失，指纹必须变化。
        assert_ne!(both, lan_fingerprint([wifi].into_iter()));
        // 同一网卡换了地址（换 Wi-Fi）也算变化。
        let moved = ("en0", "192.168.1.8".parse::<std::net::IpAddr>().unwrap());
        assert_ne!(both, lan_fingerprint([moved, bridge].into_iter()));
    }

    fn hello(seed: u8, name: &str) -> PeerHello {
        let identity = LocalIdentity::from_bytes(&[seed; 32]).expect("identity");
        PeerHello::signed(&identity, name, Capabilities::NETWORK, [seed; 32]).expect("signed hello")
    }

    #[tokio::test]
    async fn in_memory_signed_hello_ping_close_and_counts() {
        let local = hello(1, "local");
        let remote = hello(2, "remote");
        let transport = InMemoryTransport::new(local);
        transport.register_peer(remote.clone()).expect("register");
        let connection = transport.connect(remote.device_id).await.expect("connect");
        assert_eq!(connection.hello().await.expect("hello"), remote);
        assert_eq!(
            connection
                .ping_with_nonce(vec![7; 32])
                .await
                .expect("ping")
                .nonce,
            vec![7; 32]
        );
        connection.close().await.expect("close");
        connection.close().await.expect("idempotent close");
        assert_eq!(
            transport.call_counts(),
            CallCounts {
                connect: 1,
                hello: 1,
                ping: 1,
                close: 1
            }
        );
        assert!(matches!(
            connection.ping().await,
            Err(TransportError::Closed)
        ));
    }

    #[tokio::test]
    async fn in_memory_unknown_peer_and_identity_mismatch() {
        let transport = InMemoryTransport::new(hello(3, "local"));
        let remote = hello(4, "remote");
        assert!(matches!(
            transport.connect(remote.device_id).await,
            Err(TransportError::UnknownPeer(_))
        ));
        let wrong = hello(5, "wrong");
        transport
            .register_peer_at(remote.device_id, wrong.clone())
            .expect("register mismatched metadata");
        assert!(matches!(
            transport.connect(remote.device_id).await,
            Err(TransportError::IdentityMismatch { .. })
        ));
    }

    #[tokio::test]
    async fn identity_registration_rejects_mismatch_and_accepts_valid_key() {
        let local = hello(8, "local");
        let remote = hello(9, "remote");
        let transport = InMemoryTransport::new(local);
        assert!(matches!(
            transport
                .register_peer_identity(remote.device_id, [0; protocol::ED25519_PUBLIC_KEY_SIZE])
                .await,
            Err(TransportError::IdentityMismatch { .. })
        ));
        transport
            .register_peer_identity(remote.device_id, remote.public_key)
            .await
            .expect("register identity");
        transport.register_peer(remote.clone()).expect("peer");
        transport.connect(remote.device_id).await.expect("connect");
    }

    #[tokio::test]
    async fn in_memory_operation_timeout_is_deterministic() {
        let remote = hello(7, "remote");
        let transport = InMemoryTransport::new(hello(6, "local"))
            .with_operation_delay(Duration::from_millis(20))
            .with_handshake_timeout(Duration::from_millis(1));
        transport.register_peer(remote.clone()).expect("register");
        assert!(matches!(
            transport.connect(remote.device_id).await,
            Err(TransportError::Timeout {
                operation: "handshake"
            })
        ));
    }

    #[tokio::test]
    async fn quinn_bootstrap_hello_is_delivered_before_session_close() {
        // The bootstrap seam is one-shot and unregistered: the server must
        // keep the connection alive until the client has read the signed
        // hello and closed the session itself. A hard-close from the server
        // races that read and was observed as "connection lost".
        let identity_a = LocalIdentity::from_bytes(&[31; 32]).expect("identity a");
        let identity_b = LocalIdentity::from_bytes(&[32; 32]).expect("identity b");
        let local_a =
            PeerHello::signed(&identity_a, "a", Capabilities::NETWORK, [31; 32]).expect("hello a");
        let local_b =
            PeerHello::signed(&identity_b, "b", Capabilities::NETWORK, [32; 32]).expect("hello b");
        let cert = generate_simple_self_signed(vec!["deskfuse.local".into()]).expect("cert");
        let cert_der = cert.cert.der().to_vec();
        let key_der = cert.key_pair.serialize_der();
        let placeholder = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
        let b = QuinnTransport::bind(
            local_b.clone(),
            QuinnTransportConfig::new(
                placeholder,
                placeholder,
                "deskfuse.local",
                cert_der.clone(),
                key_der.clone(),
                cert_der.clone(),
            ),
        )
        .await
        .expect("bind b");
        b.configure_identity(Arc::new(identity_b))
            .expect("local signer b");
        let b_port = b.local_addr().expect("b addr").port();
        let a = QuinnTransport::bind(
            local_a.clone(),
            QuinnTransportConfig::new(
                placeholder,
                placeholder,
                "deskfuse.local",
                cert_der.clone(),
                key_der,
                cert_der,
            ),
        )
        .await
        .expect("bind a");
        a.configure_identity(Arc::new(identity_a))
            .expect("local signer a");
        let route = Endpoint::new("127.0.0.1".to_owned(), b_port).expect("route");
        let remote = a
            .bootstrap_connect(local_b.device_id, route)
            .await
            .expect("bootstrap hello");
        assert_eq!(remote.device_id, local_b.device_id);
        assert_eq!(remote.public_key, local_b.public_key);
        a.shutdown().await.expect("shutdown a");
        b.shutdown().await.expect("shutdown b");
    }

    #[tokio::test]
    async fn quinn_loopback_uses_alpn_and_pinned_certificate() {
        let identity_a = LocalIdentity::from_bytes(&[11; 32]).expect("identity a");
        let identity_b = LocalIdentity::from_bytes(&[12; 32]).expect("identity b");
        let local_a =
            PeerHello::signed(&identity_a, "a", Capabilities::NETWORK, [11; 32]).expect("hello a");
        let local_b =
            PeerHello::signed(&identity_b, "b", Capabilities::NETWORK, [12; 32]).expect("hello b");
        let cert = generate_simple_self_signed(vec!["localhost".into()]).expect("cert");
        let cert_der = cert.cert.der().to_vec();
        let key_der = cert.key_pair.serialize_der();
        let placeholder = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
        let b_config = QuinnTransportConfig::new(
            placeholder,
            placeholder,
            "localhost",
            cert_der.clone(),
            key_der.clone(),
            cert_der.clone(),
        );
        let b = QuinnTransport::bind(local_b.clone(), b_config)
            .await
            .expect("bind b");
        b.configure_identity(Arc::new(identity_b))
            .expect("local signer b");
        b.register_peer(local_a.clone()).expect("register a");
        let b_addr = b.local_addr().expect("b addr");
        // Deliberately leave the legacy configured address unusable. The
        // selected route below must be the address Quinn actually dials.
        let a_config = QuinnTransportConfig::new(
            placeholder,
            placeholder,
            "localhost",
            cert_der.clone(),
            key_der,
            cert_der,
        );
        let a = QuinnTransport::bind(local_a.clone(), a_config)
            .await
            .expect("bind a");
        a.configure_identity(Arc::new(identity_a))
            .expect("local signer a");
        a.register_peer(local_b.clone()).expect("register b");
        let route = Endpoint::new(b_addr.ip().to_string(), b_addr.port()).expect("route");
        let connection = a
            .connect_with_route(local_b.device_id, route)
            .await
            .expect("connect via selected route");
        assert_eq!(
            connection.hello().await.expect("hello").device_id,
            local_b.device_id
        );
        assert_eq!(
            connection
                .ping_with_nonce(vec![3; 32])
                .await
                .expect("ping")
                .nonce,
            vec![3; 32]
        );
        connection.close().await.expect("close");
        a.shutdown().await.expect("shutdown a");
        b.shutdown().await.expect("shutdown b");
        a.shutdown().await.expect("idempotent shutdown a");
        b.shutdown().await.expect("idempotent shutdown b");
    }

    #[tokio::test]
    async fn quinn_application_frames_do_not_overtake_a_slow_file_chunk() {
        use futures::StreamExt;

        let identity_a = LocalIdentity::from_bytes(&[21; 32]).expect("identity a");
        let identity_b = LocalIdentity::from_bytes(&[22; 32]).expect("identity b");
        let local_a =
            PeerHello::signed(&identity_a, "a", Capabilities::NETWORK, [21; 32]).expect("hello a");
        let local_b =
            PeerHello::signed(&identity_b, "b", Capabilities::NETWORK, [22; 32]).expect("hello b");
        let cert = generate_simple_self_signed(vec!["localhost".into()]).expect("cert");
        let cert_der = cert.cert.der().to_vec();
        let key_der = cert.key_pair.serialize_der();
        let placeholder = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
        let b_config = QuinnTransportConfig::new(
            placeholder,
            placeholder,
            "localhost",
            cert_der.clone(),
            key_der.clone(),
            cert_der.clone(),
        );
        let b = QuinnTransport::bind(local_b.clone(), b_config)
            .await
            .expect("bind b");
        b.configure_identity(Arc::new(identity_b))
            .expect("local signer b");
        b.register_peer(local_a.clone()).expect("register a");
        let b_addr = b.local_addr().expect("b addr");
        let a_config = QuinnTransportConfig::new(
            placeholder,
            placeholder,
            "localhost",
            cert_der.clone(),
            key_der,
            cert_der,
        );
        let a = QuinnTransport::bind(local_a.clone(), a_config)
            .await
            .expect("bind a");
        a.configure_identity(Arc::new(identity_a))
            .expect("local signer a");
        a.register_peer(local_b.clone()).expect("register b");
        let route = Endpoint::new(b_addr.ip().to_string(), b_addr.port()).expect("route");
        let connection = a
            .connect_with_route(local_b.device_id, route)
            .await
            .expect("connect via selected route");

        let mut inbox = b.messages();
        let ConnectionKind::Quinn(native) = &connection.kind else {
            panic!("real QUIC required")
        };
        let chunk = Message::ClipboardUpdate(protocol::ClipboardUpdate {
            mime: "application/x-deskfuse-file-test".into(),
            payload: vec![42; 1024 * 1024],
            sequence: 10,
        });
        let complete = Message::ClipboardUpdate(protocol::ClipboardUpdate {
            mime: "application/x-deskfuse-file-test".into(),
            payload: b"complete".to_vec(),
            sequence: 11,
        });
        let wire = protocol::encode_frame(&Envelope::new(
            [0; protocol::REQUEST_ID_SIZE],
            chunk.clone(),
        ))
        .unwrap();
        let (mut slow, _reply) = native.connection.open_bi().await.unwrap();
        slow.write_all(&wire[..8]).await.unwrap();
        connection.send_message(complete.clone()).await.unwrap();
        // QUIC accepts streams in creation order, but concurrently reading
        // their bodies used to publish the small Complete ahead of this chunk.
        assert!(
            time::timeout(Duration::from_millis(200), inbox.next())
                .await
                .is_err(),
            "later Complete overtook the unfinished file chunk"
        );
        slow.write_all(&wire[8..]).await.unwrap();
        slow.finish().unwrap();
        for expected in [chunk, complete] {
            let (peer, actual) = time::timeout(Duration::from_secs(5), inbox.next())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(peer, local_a.device_id);
            assert_eq!(actual, expected);
        }
        connection.close().await.unwrap();
        a.shutdown().await.unwrap();
        b.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn quinn_forwards_authenticated_application_messages() {
        use futures::StreamExt;

        let identity_a = LocalIdentity::from_bytes(&[21; 32]).expect("identity a");
        let identity_b = LocalIdentity::from_bytes(&[22; 32]).expect("identity b");
        let local_a =
            PeerHello::signed(&identity_a, "a", Capabilities::NETWORK, [21; 32]).expect("hello a");
        let local_b =
            PeerHello::signed(&identity_b, "b", Capabilities::NETWORK, [22; 32]).expect("hello b");
        let cert = generate_simple_self_signed(vec!["localhost".into()]).expect("cert");
        let cert_der = cert.cert.der().to_vec();
        let key_der = cert.key_pair.serialize_der();
        let placeholder = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
        let b_config = QuinnTransportConfig::new(
            placeholder,
            placeholder,
            "localhost",
            cert_der.clone(),
            key_der.clone(),
            cert_der.clone(),
        );
        let b = QuinnTransport::bind(local_b.clone(), b_config)
            .await
            .expect("bind b");
        b.configure_identity(Arc::new(identity_b))
            .expect("local signer b");
        b.register_peer(local_a.clone()).expect("register a");
        let b_addr = b.local_addr().expect("b addr");
        let a_config = QuinnTransportConfig::new(
            placeholder,
            placeholder,
            "localhost",
            cert_der.clone(),
            key_der,
            cert_der,
        );
        let a = QuinnTransport::bind(local_a.clone(), a_config)
            .await
            .expect("bind a");
        a.configure_identity(Arc::new(identity_a))
            .expect("local signer a");
        a.register_peer(local_b.clone()).expect("register b");
        let route = Endpoint::new(b_addr.ip().to_string(), b_addr.port()).expect("route");
        let connection = a
            .connect_with_route(local_b.device_id, route)
            .await
            .expect("connect via selected route");

        let update = protocol::ClipboardUpdate {
            mime: "text/plain".to_owned(),
            payload: b"cross-device".to_vec(),
            sequence: 3,
        };
        let mut inbox = b.messages();
        connection
            .send_message(Message::ClipboardUpdate(update.clone()))
            .await
            .expect("send clipboard update");

        let (peer, message) = time::timeout(Duration::from_secs(5), inbox.next())
            .await
            .expect("inbound message deadline")
            .expect("inbound stream open");
        assert_eq!(peer, local_a.device_id);
        assert_eq!(message, Message::ClipboardUpdate(update));

        connection.close().await.expect("close");
        a.shutdown().await.expect("shutdown a");
        b.shutdown().await.expect("shutdown b");
    }

    #[tokio::test]
    async fn quinn_bidirectional_application_messages_over_simultaneous_sessions() {
        use futures::StreamExt;

        let identity_a = LocalIdentity::from_bytes(&[31; 32]).expect("identity a");
        let identity_b = LocalIdentity::from_bytes(&[32; 32]).expect("identity b");
        let local_a =
            PeerHello::signed(&identity_a, "a", Capabilities::NETWORK, [31; 32]).expect("hello a");
        let local_b =
            PeerHello::signed(&identity_b, "b", Capabilities::NETWORK, [32; 32]).expect("hello b");
        let cert = generate_simple_self_signed(vec!["localhost".into()]).expect("cert");
        let cert_der = cert.cert.der().to_vec();
        let key_der = cert.key_pair.serialize_der();
        let placeholder = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));

        let b = QuinnTransport::bind(
            local_b.clone(),
            QuinnTransportConfig::new(
                placeholder,
                placeholder,
                "localhost",
                cert_der.clone(),
                key_der.clone(),
                cert_der.clone(),
            ),
        )
        .await
        .expect("bind b");
        b.configure_identity(Arc::new(identity_b))
            .expect("local signer b");
        b.register_peer(local_a.clone()).expect("register a");
        let b_addr = b.local_addr().expect("b addr");
        let a = QuinnTransport::bind(
            local_a.clone(),
            QuinnTransportConfig::new(
                placeholder,
                placeholder,
                "localhost",
                cert_der.clone(),
                key_der,
                cert_der,
            ),
        )
        .await
        .expect("bind a");
        a.configure_identity(Arc::new(identity_a))
            .expect("local signer a");
        a.register_peer(local_b.clone()).expect("register b");
        let a_addr = a.local_addr().expect("a addr");
        let route_b = Endpoint::new(b_addr.ip().to_string(), b_addr.port()).expect("route b");
        let route_a = Endpoint::new(a_addr.ip().to_string(), a_addr.port()).expect("route a");

        let (connection_a, connection_b) = tokio::join!(
            a.connect_with_route(local_b.device_id, route_b),
            b.connect_with_route(local_a.device_id, route_a),
        );
        let connection_a = connection_a.expect("a connects");
        let connection_b = connection_b.expect("b connects");
        assert_eq!(
            connection_a.hello().await.expect("hello a").device_id,
            local_b.device_id
        );
        assert_eq!(
            connection_b.hello().await.expect("hello b").device_id,
            local_a.device_id
        );

        let update_a = protocol::ClipboardUpdate {
            mime: "text/plain".to_owned(),
            payload: b"a-to-b".to_vec(),
            sequence: 1,
        };
        let mut inbox_b = b.messages();
        connection_a
            .send_message(Message::ClipboardUpdate(update_a.clone()))
            .await
            .expect("a sends");
        let (peer, message) = time::timeout(Duration::from_secs(5), inbox_b.next())
            .await
            .expect("b deadline")
            .expect("b stream");
        assert_eq!(peer, local_a.device_id);
        assert_eq!(message, Message::ClipboardUpdate(update_a));

        let update_b = protocol::ClipboardUpdate {
            mime: "text/plain".to_owned(),
            payload: b"b-to-a".to_vec(),
            sequence: 2,
        };
        let mut inbox_a = a.messages();
        connection_b
            .send_message(Message::ClipboardUpdate(update_b.clone()))
            .await
            .expect("b sends");
        let (peer, message) = time::timeout(Duration::from_secs(5), inbox_a.next())
            .await
            .expect("a deadline")
            .expect("a stream");
        assert_eq!(peer, local_b.device_id);
        assert_eq!(message, Message::ClipboardUpdate(update_b));

        connection_a.close().await.expect("close a");
        connection_b.close().await.expect("close b");
        a.shutdown().await.expect("shutdown a");
        b.shutdown().await.expect("shutdown b");
    }
}

#[cfg(test)]
mod microphone_tests;
