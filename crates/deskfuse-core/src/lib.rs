//! Transport-independent domain types for DeskFuse networking.
//!
//! This crate intentionally contains no runtime, I/O, persistence, or transport
//! dependencies.  Values crossing a process or network boundary are validated at
//! construction time (and again when deserialized).

#![allow(clippy::ignored_unit_patterns)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::return_self_not_must_use)]
#![allow(clippy::too_many_arguments)]

pub mod debug_logging;

mod file_transfer;
pub use file_transfer::*;

mod sharing;
pub use sharing::*;

use std::{
    cmp::Ordering,
    collections::BTreeMap,
    fmt,
    ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign, Not},
    str::FromStr,
};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// A result returned by this crate.
pub type Result<T> = std::result::Result<T, NetworkError>;

/// Stable error categories for invalid external data and policy failures.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkErrorCode {
    /// A device identifier or public key did not have the required shape.
    InvalidDeviceId,
    /// A request identifier was not a valid UUID.
    InvalidRequestId,
    /// A protocol version was malformed.
    InvalidProtocolVersion,
    /// An endpoint host or port was malformed.
    InvalidEndpoint,
    /// A route identifier was not a valid UUID.
    InvalidRouteId,
    /// A capability bitset was malformed.
    InvalidCapabilities,
    /// A display name or pairing request was malformed.
    InvalidPairingRequest,
    /// A peer or network snapshot violated its invariants.
    InvalidSnapshot,
    /// A clipboard payload or remote-input batch violated its invariants.
    InvalidPayload,
    /// No candidate remained after policy filtering.
    NoEligibleRoute,
}

/// An error with a stable machine-readable code and a safe human-readable message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkError {
    code: NetworkErrorCode,
    message: String,
}

impl NetworkError {
    /// Construct an error without embedding potentially sensitive input data.
    pub fn new(code: NetworkErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// Return the stable error code.
    pub const fn code(&self) -> NetworkErrorCode {
        self.code
    }

    /// Return the safe explanatory message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for NetworkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for NetworkError {}

impl fmt::Display for NetworkErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidDeviceId => "invalid_device_id",
            Self::InvalidRequestId => "invalid_request_id",
            Self::InvalidProtocolVersion => "invalid_protocol_version",
            Self::InvalidEndpoint => "invalid_endpoint",
            Self::InvalidRouteId => "invalid_route_id",
            Self::InvalidCapabilities => "invalid_capabilities",
            Self::InvalidPairingRequest => "invalid_pairing_request",
            Self::InvalidSnapshot => "invalid_snapshot",
            Self::InvalidPayload => "invalid_payload",
            Self::NoEligibleRoute => "no_eligible_route",
        })
    }
}

/// The identity of a device, computed as SHA-256 over its 32-byte public key.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DeviceId([u8; 32]);

impl DeviceId {
    /// Compute a device ID from exactly one 32-byte public key.
    pub fn from_public_key<P: AsRef<[u8]>>(public_key: P) -> Result<Self> {
        let public_key = public_key.as_ref();
        if public_key.len() != 32 {
            return Err(NetworkError::new(
                NetworkErrorCode::InvalidDeviceId,
                "public key must contain exactly 32 bytes",
            ));
        }
        let digest = Sha256::digest(public_key);
        let mut bytes = [0_u8; 32];
        bytes.copy_from_slice(&digest);
        Ok(Self(bytes))
    }

    /// Construct an ID from its already validated 32-byte digest.
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Parse a canonical hexadecimal device ID.
    pub fn from_hex(value: &str) -> Result<Self> {
        value.parse()
    }

    /// Return the digest bytes.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Copy the digest bytes out of this ID.
    pub const fn into_bytes(self) -> [u8; 32] {
        self.0
    }

    /// Render the canonical lower-case hexadecimal representation.
    pub fn to_hex(self) -> String {
        encode_hex(&self.0)
    }
}

impl AsRef<[u8]> for DeviceId {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&encode_hex(&self.0))
    }
}

impl From<[u8; 32]> for DeviceId {
    fn from(bytes: [u8; 32]) -> Self {
        Self::from_bytes(bytes)
    }
}

impl FromStr for DeviceId {
    type Err = NetworkError;

    fn from_str(value: &str) -> Result<Self> {
        let bytes = decode_hex_32(value).map_err(|_| {
            NetworkError::new(
                NetworkErrorCode::InvalidDeviceId,
                "device ID must be exactly 64 hexadecimal characters",
            )
        })?;
        Ok(Self(bytes))
    }
}

impl Serialize for DeviceId {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for DeviceId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(de::Error::custom)
    }
}

/// An ID for one pairing or request operation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RequestId(Uuid);

impl RequestId {
    /// Generate a new random request ID.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Construct from a UUID.
    pub const fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// Return the wrapped UUID.
    pub const fn as_uuid(&self) -> &Uuid {
        &self.0
    }

    /// Copy the wrapped UUID.
    pub const fn into_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for RequestId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl From<Uuid> for RequestId {
    fn from(uuid: Uuid) -> Self {
        Self::from_uuid(uuid)
    }
}

impl FromStr for RequestId {
    type Err = NetworkError;

    fn from_str(value: &str) -> Result<Self> {
        Uuid::parse_str(value).map(Self).map_err(|_| {
            NetworkError::new(
                NetworkErrorCode::InvalidRequestId,
                "request ID must be a valid UUID",
            )
        })
    }
}

impl Serialize for RequestId {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for RequestId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(de::Error::custom)
    }
}

/// The wire protocol major/minor version.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProtocolVersion {
    pub major: u16,
    pub minor: u16,
}

impl ProtocolVersion {
    /// Current first protocol version.
    pub const V2: Self = Self { major: 2, minor: 0 };

    /// Construct a version from numeric components.
    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }

    /// Return the major component.
    pub const fn major(self) -> u16 {
        self.major
    }

    /// Return the minor component.
    pub const fn minor(self) -> u16 {
        self.minor
    }
}

impl Serialize for ProtocolVersion {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl fmt::Display for ProtocolVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}", self.major, self.minor)
    }
}

impl FromStr for ProtocolVersion {
    type Err = NetworkError;

    fn from_str(value: &str) -> Result<Self> {
        let (major, minor) = value.split_once('.').ok_or_else(|| {
            NetworkError::new(
                NetworkErrorCode::InvalidProtocolVersion,
                "protocol version must be MAJOR.MINOR",
            )
        })?;
        if major.is_empty() || minor.is_empty() || minor.contains('.') {
            return Err(NetworkError::new(
                NetworkErrorCode::InvalidProtocolVersion,
                "protocol version must be MAJOR.MINOR",
            ));
        }
        let major = major.parse::<u16>().map_err(|_| {
            NetworkError::new(
                NetworkErrorCode::InvalidProtocolVersion,
                "protocol version major is out of range",
            )
        })?;
        let minor = minor.parse::<u16>().map_err(|_| {
            NetworkError::new(
                NetworkErrorCode::InvalidProtocolVersion,
                "protocol version minor is out of range",
            )
        })?;
        Ok(Self::new(major, minor))
    }
}

impl<'de> Deserialize<'de> for ProtocolVersion {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(de::Error::custom)
    }
}

/// Negotiated feature flags.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Capabilities(u64);

impl Capabilities {
    /// Network discovery and route exchange.
    pub const NETWORK: Self = Self(1 << 0);
    /// File transfer.
    pub const TRANSFER: Self = Self(1 << 1);
    /// Metadata-only file clipboard offers requested by the receiving peer.
    /// Older TRANSFER peers cannot decode deferred clipboard request frames.
    pub const DEFERRED_FILE_CLIPBOARD: Self = Self(1 << 7);
    /// Remote input/control.
    pub const INPUT_SHARING: Self = Self(1 << 4);
    /// A selected controller may temporarily manage a peer's input role.
    pub const INPUT_ACTIVATION: Self = Self(1 << 6);
    /// Recipient-approved membership in a signed, single-owner device network.
    pub const DEVICE_NETWORK: Self = Self(1 << 5);

    pub const INPUT: Self = Self(1 << 2);
    /// Clipboard synchronization.
    pub const CLIPBOARD: Self = Self(1 << 3);
    /// Epoch-bound clipboard delivery with application acknowledgement.
    pub const CLIPBOARD_SYNC_V2: Self = Self(1 << 11);

    /// Signed, permanently automatic release distribution over authenticated physical LAN.
    /// Advertise only once a verified installer and runtime are connected.
    pub const LAN_UPDATES: Self = Self(1 << 8);
    /// SSID controller arbitration on trusted physical input sessions.
    pub const INPUT_ENVIRONMENT: Self = Self(1 << 10);

    /// Epoch-barriered mouse positions carried by bounded QUIC datagrams.
    pub const INPUT_MOTION_DATAGRAM_V1: Self = Self(1 << 9);

    /// Synchronized workflow definitions and sender-side rules.
    pub const WORKFLOW_SYNC: Self = Self(1 << 14);

    /// Native remote applications over an admitted, paired control connection.
    pub const REMOTE_APPLICATION: Self = Self(1 << 12);
    /// JSON catalog plus native RDP/NLA, not the macOS media protocol.
    pub const REMOTE_APPLICATION_RDP: Self = Self(1 << 13);

    /// No capabilities.
    pub const NONE: Self = Self(0);

    /// Construct from raw bits. Unknown bits are preserved for forward compatibility.
    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    /// Return the raw bitset.
    pub const fn bits(self) -> u64 {
        self.0
    }

    /// Test whether every bit in `other` is present.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Test whether any bit in `other` is present.
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    /// Insert all flags in `other`.
    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    /// Remove all flags in `other`.
    pub fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }
}

impl BitOr for Capabilities {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for Capabilities {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl BitAnd for Capabilities {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self::Output {
        Self(self.0 & rhs.0)
    }
}

impl BitAndAssign for Capabilities {
    fn bitand_assign(&mut self, rhs: Self) {
        self.0 &= rhs.0;
    }
}

impl Not for Capabilities {
    type Output = Self;

    fn not(self) -> Self::Output {
        Self(!self.0)
    }
}

impl<'de> Deserialize<'de> for Capabilities {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        Ok(Self(u64::deserialize(deserializer)?))
    }
}

/// A validated host/port pair.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct Endpoint {
    /// DNS name, IPv4 address, or unbracketed IPv6 address.
    pub host: String,
    /// Non-zero TCP/UDP port.
    pub port: u16,
}

impl Endpoint {
    /// Validate and construct an endpoint.
    pub fn new(host: impl Into<String>, port: u16) -> Result<Self> {
        let host = normalize_host(host.into())?;
        if port == 0 {
            return Err(NetworkError::new(
                NetworkErrorCode::InvalidEndpoint,
                "endpoint port must be non-zero",
            ));
        }
        Ok(Self { host, port })
    }
}

impl fmt::Display for Endpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.host.contains(':') {
            write!(formatter, "[{}]:{}", self.host, self.port)
        } else {
            write!(formatter, "{}:{}", self.host, self.port)
        }
    }
}

impl FromStr for Endpoint {
    type Err = NetworkError;

    fn from_str(value: &str) -> Result<Self> {
        let (host, port) = if let Some(rest) = value.strip_prefix('[') {
            let (host, port) = rest.split_once("]:").ok_or_else(|| {
                NetworkError::new(
                    NetworkErrorCode::InvalidEndpoint,
                    "bracketed endpoint must be [host]:port",
                )
            })?;
            (host.to_owned(), port)
        } else {
            let (host, port) = value.rsplit_once(':').ok_or_else(|| {
                NetworkError::new(
                    NetworkErrorCode::InvalidEndpoint,
                    "endpoint must be host:port",
                )
            })?;
            (host.to_owned(), port)
        };
        let port = port.parse::<u16>().map_err(|_| {
            NetworkError::new(
                NetworkErrorCode::InvalidEndpoint,
                "endpoint port must be a number from 1 to 65535",
            )
        })?;
        Self::new(host, port)
    }
}

impl<'de> Deserialize<'de> for Endpoint {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct EndpointFields {
            host: String,
            port: u16,
        }
        let fields = EndpointFields::deserialize(deserializer)?;
        Self::new(fields.host, fields.port).map_err(de::Error::custom)
    }
}

/// Stable identity of a route candidate.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RouteId(Uuid);

impl RouteId {
    /// Generate a new route identity.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Construct from a UUID.
    pub const fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// Return the UUID.
    pub const fn as_uuid(&self) -> &Uuid {
        &self.0
    }

    /// Copy the UUID.
    pub const fn into_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for RouteId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for RouteId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl From<Uuid> for RouteId {
    fn from(uuid: Uuid) -> Self {
        Self::from_uuid(uuid)
    }
}

impl FromStr for RouteId {
    type Err = NetworkError;

    fn from_str(value: &str) -> Result<Self> {
        Uuid::parse_str(value).map(Self).map_err(|_| {
            NetworkError::new(
                NetworkErrorCode::InvalidRouteId,
                "route ID must be a valid UUID",
            )
        })
    }
}

impl Serialize for RouteId {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for RouteId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(de::Error::custom)
    }
}

/// Preferred physical interface; unavailable paths always fall back.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutePreference {
    #[default]
    Automatic,
    Thunderbolt,
    Ethernet,
    WiFi,
}
impl RoutePreference {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Automatic => "自动",
            Self::Thunderbolt => "雷电优先",
            Self::Ethernet => "有线优先",
            Self::WiFi => "Wi-Fi 优先",
        }
    }
}

pub fn device_hostname(device: DeviceId) -> String {
    format!("deskfuse-{}.local.", &device.to_hex()[..16])
}

/// Where a route was learned.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteSource {
    Mdns,
    Manual,
    CachedDirect,
    PublicDirect,
    PublicRelay,
}

impl RouteSource {
    /// Compatibility alias for the shorter cached source name.
    #[allow(non_upper_case_globals)]
    pub const Cached: Self = Self::CachedDirect;

    pub const fn rank(self) -> u8 {
        match self {
            Self::Mdns => 0,
            Self::Manual => 1,
            Self::CachedDirect => 2,
            Self::PublicDirect => 3,
            Self::PublicRelay => 4,
        }
    }

    /// Whether this source traverses the public-network feature boundary.
    pub const fn is_public(self) -> bool {
        matches!(self, Self::PublicDirect | Self::PublicRelay)
    }
}

/// Health of a route candidate.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteHealth {
    Healthy,
    Unknown,
    Backoff,
    Unavailable,
}

impl RouteHealth {
    pub const fn rank(self) -> u8 {
        match self {
            Self::Healthy => 0,
            Self::Unknown => 1,
            Self::Backoff => 2,
            Self::Unavailable => 3,
        }
    }
}

/// One possible endpoint for a peer.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RouteCandidate {
    pub id: RouteId,
    pub endpoint: Endpoint,
    pub source: RouteSource,
    pub health: RouteHealth,
    pub last_success_unix_ms: Option<u64>,
    pub failure_count: u32,
    pub capabilities: Capabilities,
    /// Last observed local interface for this address. Not persisted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_kind: Option<InterfaceKind>,
    /// Last measured handshake delay in milliseconds. Not persisted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtt_ms: Option<u32>,
    /// Most recent completed probe; session-only diagnostics, not persisted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_probe_unix_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe_error: Option<String>,
}

impl RouteCandidate {
    /// Construct a route with a fresh stable identity.
    pub fn new(
        endpoint: Endpoint,
        source: RouteSource,
        health: RouteHealth,
        capabilities: Capabilities,
    ) -> Self {
        Self {
            id: RouteId::new(),
            endpoint,
            source,
            health,
            last_success_unix_ms: None,
            failure_count: 0,
            capabilities,
            path_kind: None,
            rtt_ms: None,
            last_probe_unix_ms: None,
            probe_error: None,
        }
    }

    /// Construct a route with an explicit identity, useful for cached records and tests.
    pub const fn with_id(
        id: RouteId,
        endpoint: Endpoint,
        source: RouteSource,
        health: RouteHealth,
        last_success_unix_ms: Option<u64>,
        failure_count: u32,
        capabilities: Capabilities,
    ) -> Self {
        Self {
            id,
            endpoint,
            source,
            health,
            last_success_unix_ms,
            failure_count,
            capabilities,
            path_kind: None,
            rtt_ms: None,
            last_probe_unix_ms: None,
            probe_error: None,
        }
    }

    fn merge_from(&mut self, other: &Self) {
        if other.source.rank() < self.source.rank() {
            self.source = other.source;
        }
        if other.health.rank() < self.health.rank() {
            self.health = other.health;
        }
        match (self.last_success_unix_ms, other.last_success_unix_ms) {
            (None, Some(ts)) => self.last_success_unix_ms = Some(ts),
            (Some(left), Some(right)) if right > left => self.last_success_unix_ms = Some(right),
            _ => {}
        }
        self.failure_count = self.failure_count.max(other.failure_count);
        self.capabilities.insert(other.capabilities);
        if self.path_kind.is_none() {
            self.path_kind = other.path_kind;
        }
        if other.last_probe_unix_ms > self.last_probe_unix_ms {
            self.last_probe_unix_ms = other.last_probe_unix_ms;
            self.probe_error = other.probe_error.clone();
            self.rtt_ms = other.rtt_ms;
        } else if self.last_probe_unix_ms.is_none() && other.last_probe_unix_ms.is_none() {
            self.rtt_ms = match (self.rtt_ms, other.rtt_ms) {
                (Some(left), Some(right)) => Some(left.min(right)),
                (None, value) | (value, None) => value,
            };
        }
    }

    /// Return the deterministic rank used by [`RoutePolicy`].
    pub const fn rank(&self) -> (u8, u8) {
        (self.health.rank(), self.source.rank())
    }
}

/// Keep one candidate per `host:port`. `prefer` wins when it shares an endpoint.
pub fn collapse_routes_by_endpoint(
    routes: &mut BTreeMap<RouteId, RouteCandidate>,
    prefer: Option<RouteId>,
) -> BTreeMap<RouteId, RouteId> {
    let mut groups = BTreeMap::<(String, u16), Vec<RouteId>>::new();
    for route in routes.values() {
        groups
            .entry((route.endpoint.host.clone(), route.endpoint.port))
            .or_default()
            .push(route.id);
    }
    let mut mapping = BTreeMap::new();
    let mut remove = Vec::new();
    for ids in groups.into_values() {
        let keep = ids
            .iter()
            .copied()
            .min_by_key(|id| {
                let route = &routes[id];
                (
                    u8::from(prefer != Some(*id)),
                    route.source.rank(),
                    route.health.rank(),
                    *id,
                )
            })
            .unwrap_or(ids[0]);
        for id in ids {
            mapping.insert(id, keep);
            if id == keep {
                continue;
            }
            if let Some(other) = routes.get(&id).cloned()
                && let Some(kept) = routes.get_mut(&keep)
            {
                kept.merge_from(&other);
            }
            remove.push(id);
        }
    }
    for id in remove {
        routes.remove(&id);
    }
    mapping
}

/// Independent reachability/liveness state.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LivenessState {
    #[default]
    Unknown,
    Online,
    Offline,
    Stale,
}

impl LivenessState {
    /// Alias useful to callers that call an online peer alive.
    #[allow(non_upper_case_globals)]
    pub const Alive: Self = Self::Online;
}

/// Independent user trust/pairing state.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustState {
    #[default]
    Unknown,
    Pairing,
    Pending,
    Trusted,
    Blocked,
}

/// Independent transport connection state.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    #[default]
    Disconnected,
    Connecting,
    Connected,
    Backoff,
    Failed,
}

/// A public view of a peer and its currently known routes.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "PeerSnapshotWire")]
pub struct PeerSnapshot {
    pub id: DeviceId,
    pub display_name: String,
    pub public_key: [u8; 32],
    pub liveness: LivenessState,
    pub trust: TrustState,
    pub connection: ConnectionState,
    pub routes: Vec<RouteCandidate>,
    pub selected_route: Option<RouteId>,
}

#[derive(Deserialize)]
struct PeerSnapshotWire {
    id: DeviceId,
    display_name: String,
    public_key: [u8; 32],
    liveness: LivenessState,
    trust: TrustState,
    connection: ConnectionState,
    routes: Vec<RouteCandidate>,
    selected_route: Option<RouteId>,
}

impl TryFrom<PeerSnapshotWire> for PeerSnapshot {
    type Error = NetworkError;

    fn try_from(wire: PeerSnapshotWire) -> Result<Self> {
        Self::new(
            wire.id,
            wire.display_name,
            wire.public_key,
            wire.liveness,
            wire.trust,
            wire.connection,
            wire.routes,
            wire.selected_route,
        )
    }
}

impl PeerSnapshot {
    /// Construct a validated peer snapshot.
    pub fn new(
        id: DeviceId,
        display_name: impl Into<String>,
        public_key: [u8; 32],
        liveness: LivenessState,
        trust: TrustState,
        connection: ConnectionState,
        routes: Vec<RouteCandidate>,
        selected_route: Option<RouteId>,
    ) -> Result<Self> {
        let snapshot = Self {
            id,
            display_name: display_name.into(),
            public_key,
            liveness,
            trust,
            connection,
            routes,
            selected_route,
        };
        snapshot.validate()?;
        Ok(snapshot)
    }

    /// Check all public snapshot invariants.
    pub fn validate(&self) -> Result<()> {
        validate_display_name(&self.display_name)
            .map_err(|message| NetworkError::new(NetworkErrorCode::InvalidSnapshot, message))?;
        let expected_id = DeviceId::from_public_key(self.public_key).map_err(|_| {
            NetworkError::new(
                NetworkErrorCode::InvalidSnapshot,
                "peer public key must contain 32 bytes",
            )
        })?;
        if self.id != expected_id {
            return Err(NetworkError::new(
                NetworkErrorCode::InvalidSnapshot,
                "peer ID does not match the SHA-256 public-key digest",
            ));
        }
        let mut route_ids = self.routes.iter().map(|route| route.id).collect::<Vec<_>>();
        route_ids.sort_unstable();
        if route_ids.windows(2).any(|window| window[0] == window[1]) {
            return Err(NetworkError::new(
                NetworkErrorCode::InvalidSnapshot,
                "peer routes must have unique IDs",
            ));
        }
        if let Some(selected) = self.selected_route
            && !self.routes.iter().any(|route| route.id == selected)
        {
            return Err(NetworkError::new(
                NetworkErrorCode::InvalidSnapshot,
                "selected route must refer to one of the peer routes",
            ));
        }
        Ok(())
    }
}

/// Inputs controlling discovery and optional public networking.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NetworkConfig {
    pub mdns_enabled: bool,
    pub public_network_enabled: bool,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            mdns_enabled: true,
            public_network_enabled: false,
        }
    }
}

impl NetworkConfig {
    /// Construct defaults: mDNS on, public networking off.
    pub const fn new() -> Self {
        Self {
            mdns_enabled: true,
            public_network_enabled: false,
        }
    }

    /// Enable or disable mDNS discovery.
    pub const fn with_mdns_enabled(mut self, enabled: bool) -> Self {
        self.mdns_enabled = enabled;
        self
    }

    /// Enable or disable the additive public-network transport.
    pub const fn with_public_network_enabled(mut self, enabled: bool) -> Self {
        self.public_network_enabled = enabled;
        self
    }
}

/// A validated request to begin pairing with a peer.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "PairingRequestWire")]
pub struct PairingRequest {
    request_id: RequestId,
    device_id: DeviceId,
    display_name: String,
    public_key: [u8; 32],
    protocol_version: ProtocolVersion,
    capabilities: Capabilities,
}

#[derive(Deserialize)]
struct PairingRequestWire {
    request_id: RequestId,
    device_id: DeviceId,
    display_name: String,
    public_key: [u8; 32],
    protocol_version: ProtocolVersion,
    capabilities: Capabilities,
}

impl TryFrom<PairingRequestWire> for PairingRequest {
    type Error = NetworkError;

    fn try_from(wire: PairingRequestWire) -> Result<Self> {
        let request = Self {
            request_id: wire.request_id,
            device_id: wire.device_id,
            display_name: wire.display_name,
            public_key: wire.public_key,
            protocol_version: wire.protocol_version,
            capabilities: wire.capabilities,
        };
        request.validate()?;
        Ok(request)
    }
}

impl PairingRequest {
    /// Construct a pairing request and derive its device identity from the public key.
    pub fn new(
        display_name: impl Into<String>,
        public_key: [u8; 32],
        protocol_version: ProtocolVersion,
        capabilities: Capabilities,
    ) -> Result<Self> {
        let display_name = display_name.into();
        validate_display_name(&display_name).map_err(|message| {
            NetworkError::new(NetworkErrorCode::InvalidPairingRequest, message)
        })?;
        let device_id = DeviceId::from_public_key(public_key).map_err(|_| {
            NetworkError::new(
                NetworkErrorCode::InvalidPairingRequest,
                "public key must contain 32 bytes",
            )
        })?;
        Ok(Self {
            request_id: RequestId::new(),
            device_id,
            display_name,
            public_key,
            protocol_version,
            capabilities,
        })
    }

    /// Construct with a caller-supplied request ID (for replay-safe retries).
    pub fn with_request_id(
        request_id: RequestId,
        display_name: impl Into<String>,
        public_key: [u8; 32],
        protocol_version: ProtocolVersion,
        capabilities: Capabilities,
    ) -> Result<Self> {
        let mut request = Self::new(display_name, public_key, protocol_version, capabilities)?;
        request.request_id = request_id;
        Ok(request)
    }

    pub const fn request_id(&self) -> RequestId {
        self.request_id
    }

    pub const fn device_id(&self) -> DeviceId {
        self.device_id
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub const fn public_key(&self) -> &[u8; 32] {
        &self.public_key
    }

    pub const fn protocol_version(&self) -> ProtocolVersion {
        self.protocol_version
    }

    pub const fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    /// Re-check the request invariants after deserialization.
    pub fn validate(&self) -> Result<()> {
        validate_display_name(&self.display_name).map_err(|message| {
            NetworkError::new(NetworkErrorCode::InvalidPairingRequest, message)
        })?;
        let expected = DeviceId::from_public_key(self.public_key).map_err(|_| {
            NetworkError::new(
                NetworkErrorCode::InvalidPairingRequest,
                "public key must contain 32 bytes",
            )
        })?;
        if self.device_id != expected {
            return Err(NetworkError::new(
                NetworkErrorCode::InvalidPairingRequest,
                "pairing device ID does not match its public key",
            ));
        }
        Ok(())
    }
}

/// A read-only view of network state suitable for UI or API exposure.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "NetworkSnapshotWire")]
pub struct NetworkSnapshot {
    pub sequence: u64,
    pub local_device_id: DeviceId,
    pub peers: Vec<PeerSnapshot>,
    pub public_network_enabled: bool,
}

#[derive(Deserialize)]
struct NetworkSnapshotWire {
    sequence: u64,
    local_device_id: DeviceId,
    peers: Vec<PeerSnapshot>,
    public_network_enabled: bool,
}

impl TryFrom<NetworkSnapshotWire> for NetworkSnapshot {
    type Error = NetworkError;

    fn try_from(wire: NetworkSnapshotWire) -> Result<Self> {
        Self::new(
            wire.sequence,
            wire.local_device_id,
            wire.peers,
            wire.public_network_enabled,
        )
    }
}

impl NetworkSnapshot {
    /// Construct a validated snapshot.
    pub fn new(
        sequence: u64,
        local_device_id: DeviceId,
        peers: Vec<PeerSnapshot>,
        public_network_enabled: bool,
    ) -> Result<Self> {
        let snapshot = Self {
            sequence,
            local_device_id,
            peers,
            public_network_enabled,
        };
        snapshot.validate()?;
        Ok(snapshot)
    }

    /// Construct from a network configuration, preserving its public feature boundary.
    pub fn from_config(
        sequence: u64,
        local_device_id: DeviceId,
        peers: Vec<PeerSnapshot>,
        config: &NetworkConfig,
    ) -> Result<Self> {
        Self::new(
            sequence,
            local_device_id,
            peers,
            config.public_network_enabled,
        )
    }

    /// Validate peers, duplicate identities, and the local-device exclusion invariant.
    pub fn validate(&self) -> Result<()> {
        let mut ids = Vec::with_capacity(self.peers.len());
        for peer in &self.peers {
            peer.validate()?;
            if peer.id == self.local_device_id {
                return Err(NetworkError::new(
                    NetworkErrorCode::InvalidSnapshot,
                    "network snapshot cannot list the local device as a peer",
                ));
            }
            ids.push(peer.id);
        }
        ids.sort_unstable();
        if ids.windows(2).any(|window| window[0] == window[1]) {
            return Err(NetworkError::new(
                NetworkErrorCode::InvalidSnapshot,
                "network peers must have unique IDs",
            ));
        }
        Ok(())
    }
}

/// Why a route was selected.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum RouteSelectionReason {
    /// The candidate was the best remaining rank by health, source, and stable ID.
    BestRanked {
        health: RouteHealth,
        source: RouteSource,
    },
}

impl fmt::Display for RouteSelectionReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BestRanked { health, source } => write!(
                formatter,
                "best ranked route ({health:?}, {source:?}, stable route ID tie-break)"
            ),
        }
    }
}

/// A selected route and an explanation suitable for logs or UI.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RouteSelection {
    pub route: RouteCandidate,
    pub reason: RouteSelectionReason,
}

impl RouteSelection {
    /// Alias for callers that use candidate terminology.
    pub fn candidate(&self) -> &RouteCandidate {
        &self.route
    }

    /// Return the selected route ID.
    pub const fn route_id(&self) -> RouteId {
        self.route.id
    }
}

/// Reason a policy did not return a route.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum RouteDecisionReason {
    /// No route candidates were provided or all were filtered.
    NoEligibleRoutes,
    /// Public direct/relay routes were omitted while the feature was disabled.
    PublicRoutesDisabled,
    /// mDNS routes were omitted while mDNS was disabled.
    MdnsDisabled,
}

/// Complete policy result, including an explanation when no route is selected.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RouteDecision {
    pub selection: Option<RouteSelection>,
    pub reason: Option<RouteDecisionReason>,
}

impl RouteDecision {
    /// Whether a route was selected.
    pub const fn is_selected(&self) -> bool {
        self.selection.is_some()
    }

    /// Borrow the selected route when available.
    pub fn selected(&self) -> Option<&RouteSelection> {
        self.selection.as_ref()
    }
}

/// Deterministic and additive route selection policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RoutePolicy {
    mdns_enabled: bool,
    public_network_enabled: bool,
}

impl RoutePolicy {
    /// Build a route policy from network configuration.
    pub const fn new(config: &NetworkConfig) -> Self {
        Self {
            mdns_enabled: config.mdns_enabled,
            public_network_enabled: config.public_network_enabled,
        }
    }

    /// Build a policy from the default configuration.
    pub const fn default_policy() -> Self {
        Self {
            mdns_enabled: true,
            public_network_enabled: false,
        }
    }

    /// Whether public direct and relay candidates may be considered.
    pub const fn public_network_enabled(self) -> bool {
        self.public_network_enabled
    }

    /// Whether mDNS candidates may be considered.
    pub const fn mdns_enabled(self) -> bool {
        self.mdns_enabled
    }

    /// Return eligible candidates sorted by health, source, then stable route ID.
    pub fn eligible_routes(&self, routes: &[RouteCandidate]) -> Vec<RouteCandidate> {
        let mut eligible = routes
            .iter()
            .filter(|route| route.health != RouteHealth::Unavailable)
            .filter(|route| self.mdns_enabled || route.source != RouteSource::Mdns)
            .filter(|route| self.public_network_enabled || !route.source.is_public())
            .cloned()
            .collect::<Vec<_>>();
        eligible.sort_by(compare_routes);
        eligible
    }

    /// Select the deterministic best route, or `None` if policy filtered all candidates.
    pub fn select(&self, routes: &[RouteCandidate]) -> Option<RouteSelection> {
        self.select_with_reason(routes).selection
    }

    /// Alias for [`Self::select`].
    pub fn choose(&self, routes: &[RouteCandidate]) -> Option<RouteSelection> {
        self.select(routes)
    }

    /// Select a route while retaining the reason for an empty result.
    pub fn select_with_reason(&self, routes: &[RouteCandidate]) -> RouteDecision {
        let eligible = self.eligible_routes(routes);
        if let Some(route) = eligible.into_iter().next() {
            return RouteDecision {
                reason: None,
                selection: Some(RouteSelection {
                    reason: RouteSelectionReason::BestRanked {
                        health: route.health,
                        source: route.source,
                    },
                    route,
                }),
            };
        }

        let filtered_public =
            !self.public_network_enabled && routes.iter().any(|route| route.source.is_public());
        let filtered_mdns =
            !self.mdns_enabled && routes.iter().any(|route| route.source == RouteSource::Mdns);
        let reason = if filtered_public {
            RouteDecisionReason::PublicRoutesDisabled
        } else if filtered_mdns {
            RouteDecisionReason::MdnsDisabled
        } else {
            RouteDecisionReason::NoEligibleRoutes
        };
        RouteDecision {
            selection: None,
            reason: Some(reason),
        }
    }
}

impl Default for RoutePolicy {
    fn default() -> Self {
        Self::default_policy()
    }
}

fn compare_routes(left: &RouteCandidate, right: &RouteCandidate) -> Ordering {
    left.rank()
        .cmp(&right.rank())
        .then_with(|| left.id.cmp(&right.id))
}

fn validate_display_name(value: &str) -> std::result::Result<(), &'static str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("display name must not be empty");
    }
    if trimmed.len() > 255 {
        return Err("display name must not exceed 255 bytes");
    }
    if value.chars().any(char::is_control) {
        return Err("display name must not contain control characters");
    }
    Ok(())
}

fn normalize_host(host: String) -> Result<String> {
    let host = if host.starts_with('[') && host.ends_with(']') {
        host[1..host.len() - 1].to_owned()
    } else {
        host
    };
    if host.is_empty()
        || host.len() > 253
        || host
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
    {
        return Err(NetworkError::new(
            NetworkErrorCode::InvalidEndpoint,
            "endpoint host must be non-empty and contain no whitespace",
        ));
    }
    if host.contains('/') || host.contains('[') || host.contains(']') {
        return Err(NetworkError::new(
            NetworkErrorCode::InvalidEndpoint,
            "endpoint host contains invalid bracket or path characters",
        ));
    }
    Ok(host)
}

fn encode_hex(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(64);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn decode_hex_32(value: &str) -> std::result::Result<[u8; 32], ()> {
    if value.len() != 64 || !value.is_ascii() {
        return Err(());
    }
    let bytes = value.as_bytes();
    let mut output = [0_u8; 32];
    for (index, pair) in bytes.as_chunks::<2>().0.iter().enumerate() {
        let high = hex_value(pair[0]).ok_or(())?;
        let low = hex_value(pair[1]).ok_or(())?;
        output[index] = (high << 4) | low;
    }
    Ok(output)
}

const fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

/// Maximum clipboard payload size shared with the wire validator.
pub const MAX_CLIPBOARD_PAYLOAD_SIZE: usize = 16 * 1024 * 1024 - 256;
/// Maximum input events allowed in one batch.
pub const MAX_INPUT_EVENTS_PER_BATCH: usize = 512;

/// A clipboard payload pushed between paired devices.
///
/// The body is intentionally opaque here: diagnostics and log exports must
/// never retain `data` or the originating text bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipboardPayload {
    /// MIME type, for example `text/plain` or `image/png`.
    pub mime: String,
    /// Clipboard body bytes.
    pub data: Vec<u8>,
    /// Monotonic sender-local sequence number used for echo suppression.
    pub sequence: u64,
}

impl ClipboardPayload {
    /// Construct a validated payload.
    pub fn new(mime: impl Into<String>, data: Vec<u8>, sequence: u64) -> Result<Self> {
        let payload = Self {
            mime: mime.into(),
            data,
            sequence,
        };
        payload.validate()?;
        Ok(payload)
    }

    /// Validate the invariant shared with the wire boundary.
    pub fn validate(&self) -> Result<()> {
        if self.mime.is_empty() || self.mime.len() > 256 {
            return Err(NetworkError::new(
                NetworkErrorCode::InvalidPayload,
                "clipboard mime type must be 1..=256 bytes",
            ));
        }
        if self.data.len() > MAX_CLIPBOARD_PAYLOAD_SIZE {
            return Err(NetworkError::new(
                NetworkErrorCode::InvalidPayload,
                "clipboard payload is too large",
            ));
        }
        Ok(())
    }
}

/// One normalized remote-input event.
#[derive(Clone, Debug, PartialEq)]
pub enum RemoteInputEvent {
    /// Cursor position normalized to the sender's main display.
    MouseMove { x: f64, y: f64 },
    /// Mouse button state change (`0` left, `1` right, `2` middle).
    MouseButton { button: u8, pressed: bool },
    /// macOS virtual key code state change.
    Key { code: u16, pressed: bool },
    /// Scroll-wheel deltas in lines.
    Scroll { delta_x: f64, delta_y: f64 },
}

impl RemoteInputEvent {
    /// Validate the invariant shared with the wire boundary.
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::MouseMove { x, y } => {
                if !x.is_finite()
                    || !y.is_finite()
                    || !(0.0..=1.0).contains(x)
                    || !(0.0..=1.0).contains(y)
                {
                    return Err(NetworkError::new(
                        NetworkErrorCode::InvalidPayload,
                        "mouse coordinates must be finite and normalized to 0.0..=1.0",
                    ));
                }
                Ok(())
            }
            Self::MouseButton { button, .. } => {
                if *button > 31 {
                    return Err(NetworkError::new(
                        NetworkErrorCode::InvalidPayload,
                        "mouse button number is too large",
                    ));
                }
                Ok(())
            }
            Self::Key { .. } => Ok(()),
            Self::Scroll { delta_x, delta_y } => {
                if !delta_x.is_finite() || !delta_y.is_finite() {
                    return Err(NetworkError::new(
                        NetworkErrorCode::InvalidPayload,
                        "scroll deltas must be finite",
                    ));
                }
                Ok(())
            }
        }
    }
}

/// A bounded batch of normalized remote-input events.
#[derive(Clone, Debug, PartialEq)]
pub struct InputEventBatch {
    pub sequence: u64,
    pub events: Vec<RemoteInputEvent>,
}

impl InputEventBatch {
    /// Construct a validated batch.
    pub fn new(sequence: u64, events: Vec<RemoteInputEvent>) -> Result<Self> {
        let batch = Self { sequence, events };
        batch.validate()?;
        Ok(batch)
    }

    /// Validate the invariant shared with the wire boundary.
    pub fn validate(&self) -> Result<()> {
        if self.events.is_empty() || self.events.len() > MAX_INPUT_EVENTS_PER_BATCH {
            return Err(NetworkError::new(
                NetworkErrorCode::InvalidPayload,
                "input batch must contain 1..=512 events",
            ));
        }
        for event in &self.events {
            event.validate()?;
        }
        Ok(())
    }
}

/// Main-display geometry used to map a controller cursor onto a controlled
/// device. Dimensions are physical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayInfo {
    pub width: u32,
    pub height: u32,
}

impl DisplayInfo {
    /// Construct a validated display snapshot.
    pub fn new(width: u32, height: u32) -> Result<Self> {
        if width == 0 || height == 0 || width > 32_768 || height > 32_768 {
            return Err(NetworkError::new(
                NetworkErrorCode::InvalidPayload,
                "display dimensions must be within 1..=32768",
            ));
        }
        Ok(Self { width, height })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, from_str, to_value};

    fn key(byte: u8) -> [u8; 32] {
        [byte; 32]
    }

    fn endpoint(host: &str, port: u16) -> Endpoint {
        Endpoint::new(host, port).expect("test endpoint is valid")
    }

    fn route(
        id_byte: u8,
        endpoint: Endpoint,
        source: RouteSource,
        health: RouteHealth,
    ) -> RouteCandidate {
        RouteCandidate::with_id(
            RouteId::from_uuid(Uuid::from_bytes([id_byte; 16])),
            endpoint,
            source,
            health,
            None,
            0,
            Capabilities::NETWORK,
        )
    }

    #[test]
    fn device_id_is_stable_sha256_and_round_trips_hex() {
        let public_key = key(7);
        let first = DeviceId::from_public_key(public_key).expect("valid key");
        let second = DeviceId::from_public_key(public_key).expect("valid key");
        assert_eq!(first, second);
        assert_eq!(first.to_string().len(), 64);
        assert_eq!(first, first.to_string().parse().expect("valid hex"));
        assert!(DeviceId::from_public_key([0_u8; 31]).is_err());
        assert!("not-a-device-id".parse::<DeviceId>().is_err());
    }

    #[test]
    fn device_id_serde_is_hex_not_raw_secret_material() {
        let id = DeviceId::from_public_key(key(3)).expect("valid key");
        let json = to_value(id).expect("serialize");
        assert!(json.as_str().is_some_and(|value| value.len() == 64));
        let decoded: DeviceId =
            from_str(&serde_json::to_string(&json).expect("string")).expect("deserialize");
        assert_eq!(decoded, id);
    }

    #[test]
    fn endpoint_validates_and_formats_ipv4_dns_and_ipv6() {
        assert_eq!(
            Endpoint::new("example.local", 443)
                .expect("valid")
                .to_string(),
            "example.local:443"
        );
        assert_eq!(
            Endpoint::new("::1", 443).expect("valid").to_string(),
            "[::1]:443"
        );
        assert_eq!(
            "[::1]:443".parse::<Endpoint>().expect("valid"),
            Endpoint::new("::1", 443).expect("valid")
        );
        assert!(Endpoint::new("", 443).is_err());
        assert!(Endpoint::new("host name", 443).is_err());
        assert!(Endpoint::new("example.local", 0).is_err());
        assert!("example.local".parse::<Endpoint>().is_err());
    }

    #[test]
    fn request_and_route_ids_are_uuid_strings() {
        let uuid = Uuid::from_bytes([9; 16]);
        let request = RequestId::from_uuid(uuid);
        let route = RouteId::from_uuid(uuid);
        assert_eq!(request.to_string(), uuid.to_string());
        assert_eq!(route.to_string(), uuid.to_string());
        assert_eq!(request, request.to_string().parse().expect("request UUID"));
        assert_eq!(route, route.to_string().parse().expect("route UUID"));
    }

    #[test]
    fn deferred_file_clipboard_requires_explicit_capability() {
        let legacy = Capabilities::TRANSFER | Capabilities::CLIPBOARD;
        assert!(!legacy.contains(Capabilities::DEFERRED_FILE_CLIPBOARD));
        let current = legacy | Capabilities::DEFERRED_FILE_CLIPBOARD;
        assert!(current.contains(Capabilities::DEFERRED_FILE_CLIPBOARD));
        assert_eq!(Capabilities::from_bits(current.bits()), current);
        assert_eq!(legacy & current, legacy);
    }

    #[test]
    fn capabilities_support_independent_bits() {
        let mut capabilities = Capabilities::NETWORK | Capabilities::CLIPBOARD;
        assert!(capabilities.contains(Capabilities::NETWORK));
        assert!(!capabilities.contains(Capabilities::TRANSFER));
        capabilities.insert(Capabilities::TRANSFER);
        assert!(capabilities.intersects(Capabilities::TRANSFER | Capabilities::INPUT));
        capabilities.remove(Capabilities::NETWORK);
        assert!(!capabilities.contains(Capabilities::NETWORK));
    }

    #[test]
    fn negotiated_features_have_unique_single_bits() {
        let features = [
            Capabilities::NETWORK,
            Capabilities::TRANSFER,
            Capabilities::INPUT,
            Capabilities::CLIPBOARD,
            Capabilities::INPUT_SHARING,
            Capabilities::DEVICE_NETWORK,
            Capabilities::INPUT_ACTIVATION,
            Capabilities::DEFERRED_FILE_CLIPBOARD,
            Capabilities::LAN_UPDATES,
            Capabilities::INPUT_MOTION_DATAGRAM_V1,
            Capabilities::INPUT_ENVIRONMENT,
        ];
        let mut seen = Capabilities::NONE;
        for feature in features {
            assert_eq!(feature.bits().count_ones(), 1);
            assert!(
                !seen.intersects(feature),
                "duplicate capability: {feature:?}"
            );
            seen.insert(feature);
        }
    }

    #[test]
    fn motion_peer_does_not_negotiate_controller_environment() {
        let peer = Capabilities::INPUT_SHARING | Capabilities::INPUT_MOTION_DATAGRAM_V1;
        let local = peer | Capabilities::INPUT_ENVIRONMENT;
        let negotiated = Capabilities::from_bits(local.bits() & peer.bits());
        assert!(negotiated.contains(Capabilities::INPUT_MOTION_DATAGRAM_V1));
        assert!(!negotiated.contains(Capabilities::INPUT_ENVIRONMENT));
    }

    #[test]
    fn protocol_version_round_trips() {
        let version: ProtocolVersion = "12.34".parse().expect("valid version");
        assert_eq!(version.major(), 12);
        assert_eq!(version.minor(), 34);
        assert_eq!(version.to_string(), "12.34");
        let json = serde_json::to_string(&version).expect("serialize version");
        assert_eq!(json, "\"12.34\"");
        assert_eq!(
            serde_json::from_str::<ProtocolVersion>(&json).expect("deserialize version"),
            version
        );
        assert!("1".parse::<ProtocolVersion>().is_err());
        assert!("1.2.3".parse::<ProtocolVersion>().is_err());
    }

    #[test]
    fn orthogonal_states_can_be_combined_without_loss() {
        let peer = PeerSnapshot {
            id: DeviceId::from_public_key(key(4)).expect("valid key"),
            display_name: "Peer".to_owned(),
            public_key: key(4),
            liveness: LivenessState::Offline,
            trust: TrustState::Trusted,
            connection: ConnectionState::Connected,
            routes: Vec::new(),
            selected_route: None,
        };
        assert_eq!(peer.liveness, LivenessState::Offline);
        assert_eq!(peer.trust, TrustState::Trusted);
        assert_eq!(peer.connection, ConnectionState::Connected);
    }

    #[test]
    fn collapse_routes_keeps_one_address_and_merges_learned_state() {
        let endpoint = endpoint("10.10.10.1", 42424);
        let mut manual = route(
            1,
            endpoint.clone(),
            RouteSource::Manual,
            RouteHealth::Unknown,
        );
        manual.failure_count = 12;
        let mut mdns = route(
            2,
            endpoint.clone(),
            RouteSource::Mdns,
            RouteHealth::Unavailable,
        );
        mdns.failure_count = 3;
        mdns.last_success_unix_ms = Some(9);
        let mut routes = BTreeMap::from([(manual.id, manual.clone()), (mdns.id, mdns.clone())]);
        let mapping = collapse_routes_by_endpoint(&mut routes, None);
        assert_eq!(routes.len(), 1);
        assert_eq!(mapping[&manual.id], mapping[&mdns.id]);
        let kept = routes.values().next().expect("collapsed");
        assert_eq!(kept.endpoint, endpoint);
        assert_eq!(kept.source, RouteSource::Mdns);
        assert_eq!(kept.health, RouteHealth::Unknown);
        assert_eq!(kept.failure_count, 12);
        assert_eq!(kept.last_success_unix_ms, Some(9));
    }

    #[test]
    fn collapse_routes_prefers_the_live_identity() {
        let endpoint = endpoint("10.0.0.8", 42424);
        let cached = route(
            3,
            endpoint.clone(),
            RouteSource::CachedDirect,
            RouteHealth::Unknown,
        );
        let mdns = route(4, endpoint, RouteSource::Mdns, RouteHealth::Healthy);
        let mut routes = BTreeMap::from([(cached.id, cached.clone()), (mdns.id, mdns.clone())]);
        collapse_routes_by_endpoint(&mut routes, Some(cached.id));
        assert_eq!(routes.len(), 1);
        let kept = &routes[&cached.id];
        assert_eq!(kept.source, RouteSource::Mdns);
        assert_eq!(kept.health, RouteHealth::Healthy);
    }

    #[test]
    fn route_policy_ranks_health_then_source_then_stable_id() {
        let routes = vec![
            route(
                5,
                endpoint("relay", 5),
                RouteSource::PublicRelay,
                RouteHealth::Healthy,
            ),
            route(
                4,
                endpoint("manual", 4),
                RouteSource::Manual,
                RouteHealth::Unknown,
            ),
            route(
                3,
                endpoint("cached", 3),
                RouteSource::CachedDirect,
                RouteHealth::Healthy,
            ),
            route(
                2,
                endpoint("mdns", 2),
                RouteSource::Mdns,
                RouteHealth::Healthy,
            ),
            route(
                1,
                endpoint("manual-low-id", 1),
                RouteSource::Manual,
                RouteHealth::Healthy,
            ),
        ];
        let policy = RoutePolicy::new(&NetworkConfig::default().with_public_network_enabled(true));
        let selection = policy.select(&routes).expect("a route");
        assert_eq!(selection.route.source, RouteSource::Mdns);
        assert_eq!(selection.route.health, RouteHealth::Healthy);
        assert_eq!(
            selection.reason,
            RouteSelectionReason::BestRanked {
                health: RouteHealth::Healthy,
                source: RouteSource::Mdns
            }
        );
    }

    #[test]
    fn route_policy_uses_route_id_as_stable_tie_breaker() {
        let routes = vec![
            route(
                0x20,
                endpoint("second", 2),
                RouteSource::Manual,
                RouteHealth::Unknown,
            ),
            route(
                0x10,
                endpoint("first", 1),
                RouteSource::Manual,
                RouteHealth::Unknown,
            ),
        ];
        let policy = RoutePolicy::default();
        let selection = policy.select(&routes).expect("a route");
        assert_eq!(
            selection.route.id,
            RouteId::from_uuid(Uuid::from_bytes([0x10; 16]))
        );
    }

    #[test]
    fn unavailable_routes_are_not_selected_when_they_are_the_only_candidates() {
        let routes = vec![route(
            1,
            endpoint("unavailable", 1),
            RouteSource::Manual,
            RouteHealth::Unavailable,
        )];
        let decision = RoutePolicy::default().select_with_reason(&routes);
        assert!(decision.selection.is_none());
        assert_eq!(decision.reason, Some(RouteDecisionReason::NoEligibleRoutes));
        assert!(RoutePolicy::default().select(&routes).is_none());
    }

    #[test]
    fn unavailable_routes_are_filtered_when_mixed_with_usable_routes() {
        let unavailable = route(
            1,
            endpoint("unavailable", 1),
            RouteSource::Mdns,
            RouteHealth::Unavailable,
        );
        let usable = route(
            2,
            endpoint("usable", 2),
            RouteSource::Manual,
            RouteHealth::Unknown,
        );
        let policy = RoutePolicy::default();
        let eligible = policy.eligible_routes(&[unavailable.clone(), usable.clone()]);
        assert_eq!(eligible, vec![usable.clone()]);
        let selection = policy
            .select(&[unavailable, usable.clone()])
            .expect("usable route");
        assert_eq!(selection.route.id, usable.id);
        assert_ne!(selection.route.health, RouteHealth::Unavailable);
    }

    #[test]
    fn public_network_is_default_off_and_public_routes_are_filtered() {
        let routes = vec![route(
            1,
            endpoint("public", 1),
            RouteSource::PublicDirect,
            RouteHealth::Healthy,
        )];
        let decision = RoutePolicy::default().select_with_reason(&routes);
        assert!(!NetworkConfig::default().public_network_enabled);
        assert!(decision.selection.is_none());
        assert_eq!(
            decision.reason,
            Some(RouteDecisionReason::PublicRoutesDisabled)
        );
        let enabled = NetworkConfig::default().with_public_network_enabled(true);
        assert!(RoutePolicy::new(&enabled).select(&routes).is_some());
    }

    #[test]
    fn mdns_toggle_is_independent_from_public_toggle() {
        let routes = vec![route(
            1,
            endpoint("mdns", 1),
            RouteSource::Mdns,
            RouteHealth::Healthy,
        )];
        let config = NetworkConfig::default().with_mdns_enabled(false);
        let decision = RoutePolicy::new(&config).select_with_reason(&routes);
        assert_eq!(decision.reason, Some(RouteDecisionReason::MdnsDisabled));
    }

    #[test]
    fn pairing_and_snapshot_validate_identity_and_do_not_include_private_fields() {
        let public_key = key(8);
        let pairing = PairingRequest::new(
            "Laptop",
            public_key,
            ProtocolVersion::V2,
            Capabilities::NETWORK,
        )
        .expect("valid pairing");
        assert_eq!(
            pairing.device_id(),
            DeviceId::from_public_key(public_key).expect("same ID")
        );
        let peer = PeerSnapshot::new(
            pairing.device_id(),
            pairing.display_name(),
            public_key,
            LivenessState::Online,
            TrustState::Trusted,
            ConnectionState::Connected,
            Vec::new(),
            None,
        )
        .expect("valid peer");
        let snapshot = NetworkSnapshot::new(
            7,
            DeviceId::from_public_key(key(9)).expect("local"),
            vec![peer],
            false,
        )
        .expect("valid snapshot");
        let json = serde_json::to_string(&snapshot).expect("serialize snapshot");
        assert!(!json.contains("private"));
        assert!(!json.contains("secret"));
        assert!(json.contains("public_key"));
        assert!(serde_json::from_str::<NetworkSnapshot>(&json).is_ok());
    }

    #[test]
    fn snapshots_reject_duplicate_peer_or_route_ids() {
        let public_key = key(1);
        let id = DeviceId::from_public_key(public_key).expect("valid key");
        let peer = PeerSnapshot::new(
            id,
            "peer",
            public_key,
            LivenessState::Online,
            TrustState::Trusted,
            ConnectionState::Connected,
            Vec::new(),
            None,
        )
        .expect("valid");
        assert!(
            NetworkSnapshot::new(
                0,
                DeviceId::from_public_key(key(2)).expect("local"),
                vec![peer.clone(), peer],
                false
            )
            .is_err()
        );
        let route_id = RouteId::from_uuid(Uuid::from_bytes([1; 16]));
        let routes = vec![
            RouteCandidate::with_id(
                route_id,
                endpoint("a", 1),
                RouteSource::Manual,
                RouteHealth::Healthy,
                None,
                0,
                Capabilities::NONE,
            ),
            RouteCandidate::with_id(
                route_id,
                endpoint("b", 2),
                RouteSource::Manual,
                RouteHealth::Healthy,
                None,
                0,
                Capabilities::NONE,
            ),
        ];
        assert!(
            PeerSnapshot::new(
                id,
                "peer",
                public_key,
                LivenessState::Online,
                TrustState::Trusted,
                ConnectionState::Connected,
                routes,
                None
            )
            .is_err()
        );
    }

    #[test]
    fn json_shapes_are_safe_and_stable() {
        let config = NetworkConfig::default();
        let json: Value = serde_json::to_value(config).expect("serialize");
        assert_eq!(json["mdns_enabled"], Value::Bool(true));
        assert_eq!(json["public_network_enabled"], Value::Bool(false));
    }
}

pub mod workflow;
