//! Local peer discovery for DeskFuse.
//!
//! Discovery is deliberately only a hint.  A [`ServiceRecord`] contains public
//! metadata suitable for an mDNS TXT record; it never contains a private key,
//! token, pairing secret, or any other credential.  Discovery events must not be
//! used as proof that a peer is trusted.

#![deny(unsafe_code)]
#![allow(
    clippy::boxed_local,
    clippy::cast_possible_truncation,
    clippy::doc_markdown
)]
#![allow(
    clippy::match_same_arms,
    clippy::missing_errors_doc,
    clippy::needless_lifetimes
)]
#![allow(clippy::needless_pass_by_value, clippy::unnested_or_patterns)]

use deskfuse_core::{Capabilities, DeviceId, Endpoint, NetworkError, ProtocolVersion};
use mdns_sd::{
    DaemonEvent, ResolvedService, ServiceDaemon, ServiceEvent, ServiceInfo, TxtProperties,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use thiserror::Error;

#[cfg(target_os = "macos")]
mod bonjour;
#[cfg(target_os = "macos")]
pub use bonjour::BonjourDiscovery as PlatformDiscovery;
#[cfg(not(target_os = "macos"))]
pub type PlatformDiscovery = MdnsDiscovery;

/// The DNS-SD service type used by DeskFuse.
pub const SERVICE_TYPE: &str = "_deskfuse._udp.local.";
pub const INPUT_SERVICE_TYPE: &str = "_deskfuse-input._udp.local.";
/// Compatibility alias for callers that use a more explicit name.
pub const DESKFUSE_SERVICE_TYPE: &str = SERVICE_TYPE;
/// The TXT key carrying the protocol version.
pub const TXT_VERSION: &str = "v";
/// The TXT key carrying the device id.
pub const TXT_ID: &str = "id";
/// The TXT key carrying the human-readable display name.
pub const TXT_NAME: &str = "name";
/// The TXT key carrying the transport protocol label.
pub const TXT_PROTOCOL: &str = "protocol";
/// The TXT key carrying the capability bitset.
pub const TXT_CAPABILITIES: &str = "capabilities";
/// The TXT key carrying the advertised service port.
pub const TXT_PORT: &str = "port";
/// The only protocol label currently advertised by this crate.
pub const DEFAULT_PROTOCOL: &str = "udp";

/// Errors raised by discovery and public record validation.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum DiscoveryError {
    /// A required TXT key was absent.
    #[error("mDNS TXT record is missing required field '{0}'")]
    MissingTxtField(&'static str),
    /// A TXT key contained a non-UTF-8 or otherwise malformed value.
    #[error("mDNS TXT record has an invalid field '{0}'")]
    InvalidTxtField(&'static str),
    /// The value in `id` was not an exact device-id hex value.
    #[error("mDNS TXT record has an invalid device id")]
    InvalidDeviceId,
    /// The TXT protocol version is not understood by this implementation.
    #[error("mDNS TXT record has an unsupported protocol version")]
    InvalidVersion,
    /// The TXT protocol label is not a valid token.
    #[error("mDNS TXT record has an invalid protocol label")]
    InvalidProtocol,
    /// The display name is empty or contains control characters.
    #[error("mDNS TXT record has an invalid display name")]
    InvalidName,
    /// The advertised port is zero or outside the TCP/UDP range.
    #[error("mDNS TXT record has an invalid port")]
    InvalidPort,
    /// An endpoint did not pass hostname/IP and port validation.
    #[error("invalid manual endpoint: {0}")]
    InvalidEndpoint(String),
    /// A manual endpoint was supplied for a device other than the configured
    /// expected device.
    #[error("manual endpoint does not match the expected device id")]
    UnexpectedDevice,
    /// The provider was already started or shut down.
    #[error("discovery provider is unavailable: {0}")]
    ProviderUnavailable(&'static str),
    /// The event consumer was dropped.
    #[error("discovery event channel is closed")]
    ChannelClosed,
    /// mdns-sd returned an adapter error.  Its text is retained only as a
    /// human-readable diagnostic; no TXT contents are included here.
    #[error("mDNS adapter error: {0}")]
    Mdns(String),
}

impl From<mdns_sd::Error> for DiscoveryError {
    fn from(error: mdns_sd::Error) -> Self {
        Self::Mdns(error.to_string())
    }
}

impl From<NetworkError> for DiscoveryError {
    fn from(error: NetworkError) -> Self {
        Self::InvalidEndpoint(error.to_string())
    }
}

/// Public metadata sent in an `_deskfuse._udp.local.` TXT record.
///
/// The field names intentionally mirror the TXT keys.  Device ids are the
/// SHA-256 digest used by `deskfuse-core`; no public key or private credential is
/// present in this record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceRecord {
    /// Protocol version (`v` in TXT).
    pub version: ProtocolVersion,
    /// Stable device id (`id` in TXT).
    pub id: DeviceId,
    /// Human-readable device name (`name` in TXT).
    pub name: String,
    /// Transport label (`protocol` in TXT), normally `udp`.
    pub protocol: String,
    /// Capability bitset (`capabilities` in TXT).
    pub capabilities: Capabilities,
    /// Service port (`port` in TXT and SRV).
    pub port: u16,
}

impl ServiceRecord {
    /// Construct a validated public service record.
    pub fn new(
        version: ProtocolVersion,
        id: DeviceId,
        name: impl Into<String>,
        protocol: impl Into<String>,
        capabilities: Capabilities,
        port: u16,
    ) -> Result<Self, DiscoveryError> {
        let record = Self {
            version,
            id,
            name: name.into(),
            protocol: protocol.into(),
            capabilities,
            port,
        };
        record.validate()?;
        Ok(record)
    }

    /// Construct a current v2 UDP record.
    pub fn v2(
        id: DeviceId,
        name: impl Into<String>,
        capabilities: Capabilities,
        port: u16,
    ) -> Result<Self, DiscoveryError> {
        Self::new(
            ProtocolVersion::V2,
            id,
            name,
            DEFAULT_PROTOCOL,
            capabilities,
            port,
        )
    }

    /// Compatibility constructor with the version last, useful at call sites
    /// that mirror `PairingRequest::new`.
    pub fn with_protocol(
        id: DeviceId,
        name: impl Into<String>,
        protocol: impl Into<String>,
        capabilities: Capabilities,
        port: u16,
        version: ProtocolVersion,
    ) -> Result<Self, DiscoveryError> {
        Self::new(version, id, name, protocol, capabilities, port)
    }

    /// Return the stable device id.  This is an alias for the public `id` field.
    #[must_use]
    pub const fn device_id(&self) -> DeviceId {
        self.id
    }

    /// Return the stable device id.  This is an alias for [`device_id`](Self::device_id).
    #[must_use]
    pub const fn id(&self) -> DeviceId {
        self.id
    }

    /// Validate all fields before encoding or using the record.
    pub fn validate(&self) -> Result<(), DiscoveryError> {
        if self.version.major() != ProtocolVersion::V2.major() {
            return Err(DiscoveryError::InvalidVersion);
        }
        if self.name.is_empty()
            || self.name.chars().count() > 256
            || self.name.chars().any(char::is_control)
        {
            return Err(DiscoveryError::InvalidName);
        }
        if self.protocol.is_empty()
            || !self.protocol.is_ascii()
            || !self
                .protocol
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
            || !(self.protocol.eq_ignore_ascii_case(DEFAULT_PROTOCOL)
                || self.protocol.eq_ignore_ascii_case("quic"))
        {
            return Err(DiscoveryError::InvalidProtocol);
        }
        if self.port == 0 {
            return Err(DiscoveryError::InvalidPort);
        }
        Ok(())
    }

    /// Encode the six allow-listed TXT key/value pairs in deterministic order.
    ///
    /// Unknown fields are impossible through this API, so secrets cannot be
    /// accidentally copied into a TXT record.
    #[must_use]
    pub fn encode_txt(&self) -> Vec<(String, String)> {
        // This method is intentionally infallible for ergonomic use after
        // construction.  A value made through a struct literal is handled by
        // `encode_txt_checked` instead of panicking here.
        vec![
            (TXT_VERSION.to_owned(), self.version.to_string()),
            (TXT_ID.to_owned(), self.id.to_hex()),
            (TXT_NAME.to_owned(), self.name.clone()),
            (TXT_PROTOCOL.to_owned(), self.protocol.clone()),
            (
                TXT_CAPABILITIES.to_owned(),
                self.capabilities.bits().to_string(),
            ),
            (TXT_PORT.to_owned(), self.port.to_string()),
        ]
    }

    /// Encode after re-validating a struct literal.
    pub fn encode_txt_checked(&self) -> Result<Vec<(String, String)>, DiscoveryError> {
        self.validate()?;
        Ok(self.encode_txt())
    }

    /// Alias for [`encode_txt`](Self::encode_txt).
    #[must_use]
    pub fn to_txt(&self) -> Vec<(String, String)> {
        self.encode_txt()
    }

    /// Alias for [`encode_txt`](Self::encode_txt).
    #[must_use]
    pub fn txt(&self) -> Vec<(String, String)> {
        self.encode_txt()
    }

    /// Alias for [`decode_txt`](Self::decode_txt).
    pub fn from_txt<I, E>(entries: I) -> Result<Self, DiscoveryError>
    where
        I: IntoIterator<Item = E>,
        E: TxtEntry,
    {
        Self::decode_txt(entries)
    }

    /// Return the allow-listed TXT values as a deterministic map.
    #[must_use]
    pub fn txt_map(&self) -> BTreeMap<String, String> {
        self.encode_txt().into_iter().collect()
    }

    /// Return the advertised protocol version.
    #[must_use]
    pub const fn protocol_version(&self) -> ProtocolVersion {
        self.version
    }

    /// Return the human-readable display name.
    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.name
    }

    /// Encode DNS TXT wire chunks (each item includes its one-byte length).
    pub fn encode_txt_wire(&self) -> Result<Vec<Vec<u8>>, DiscoveryError> {
        self.validate()?;
        self.encode_txt()
            .into_iter()
            .map(|(key, value)| {
                let mut item = format!("{key}={value}").into_bytes();
                if item.len() > u8::MAX as usize {
                    return Err(DiscoveryError::InvalidTxtField(TXT_NAME));
                }
                let mut wire = Vec::with_capacity(item.len() + 1);
                wire.push(item.len() as u8);
                wire.append(&mut item);
                Ok(wire)
            })
            .collect()
    }

    /// Decode a sequence of UTF-8 TXT key/value pairs.
    pub fn decode_txt<I, E>(entries: I) -> Result<Self, DiscoveryError>
    where
        I: IntoIterator<Item = E>,
        E: TxtEntry,
    {
        let mut values = BTreeMap::<String, String>::new();
        for entry in entries {
            let (key, value) = entry.into_txt_entry()?;
            let normalized = key.to_ascii_lowercase();
            // RFC 6763 says clients use the first occurrence.  Applying that
            // rule also makes duplicate/malicious TXT values deterministic.
            values.entry(normalized).or_insert(value);
        }
        Self::from_map(values)
    }

    /// Decode mdns-sd's parsed TXT properties.
    pub fn from_txt_properties(properties: &TxtProperties) -> Result<Self, DiscoveryError> {
        Self::decode_txt(properties.iter())
    }

    /// Alias for [`from_txt_properties`](Self::from_txt_properties).
    pub fn decode_properties(properties: &TxtProperties) -> Result<Self, DiscoveryError> {
        Self::from_txt_properties(properties)
    }

    /// Decode one or more length-prefixed DNS TXT chunks.
    pub fn decode_txt_wire<I, E>(chunks: I) -> Result<Self, DiscoveryError>
    where
        I: IntoIterator<Item = E>,
        E: AsRef<[u8]>,
    {
        let mut entries = Vec::<(String, String)>::new();
        for chunk in chunks {
            let chunk = chunk.as_ref();
            if chunk.is_empty() || chunk[0] as usize != chunk.len() - 1 {
                return Err(DiscoveryError::InvalidTxtField("wire"));
            }
            let text = std::str::from_utf8(&chunk[1..])
                .map_err(|_| DiscoveryError::InvalidTxtField("wire"))?;
            let (key, value) = text
                .split_once('=')
                .ok_or(DiscoveryError::InvalidTxtField("wire"))?;
            entries.push((key.to_owned(), value.to_owned()));
        }
        Self::decode_txt(entries)
    }

    fn from_map(values: BTreeMap<String, String>) -> Result<Self, DiscoveryError> {
        fn required(
            values: &BTreeMap<String, String>,
            key: &'static str,
        ) -> Result<String, DiscoveryError> {
            values
                .get(key)
                .cloned()
                .ok_or(DiscoveryError::MissingTxtField(key))
        }

        let version = required(&values, TXT_VERSION)?
            .parse::<ProtocolVersion>()
            .map_err(|_| DiscoveryError::InvalidVersion)?;
        let id = DeviceId::from_hex(&required(&values, TXT_ID)?)
            .map_err(|_| DiscoveryError::InvalidDeviceId)?;
        let name = required(&values, TXT_NAME)?;
        let protocol = required(&values, TXT_PROTOCOL)?;
        let capabilities_text = required(&values, TXT_CAPABILITIES)?;
        let capabilities = parse_u64(&capabilities_text)
            .ok_or(DiscoveryError::InvalidTxtField(TXT_CAPABILITIES))?;
        let port = required(&values, TXT_PORT)?
            .parse::<u16>()
            .map_err(|_| DiscoveryError::InvalidPort)?;
        Self::new(
            version,
            id,
            name,
            protocol,
            Capabilities::from_bits(capabilities),
            port,
        )
    }
}

/// A small conversion trait allowing TXT decoding from mdns-sd properties,
/// owned tuples, borrowed tuples, and `key=value` strings without a second parser.
pub trait TxtEntry {
    /// Convert this item to one UTF-8 key/value pair.
    fn into_txt_entry(self) -> Result<(String, String), DiscoveryError>;
}

impl<K, V> TxtEntry for (K, V)
where
    K: AsRef<str>,
    V: AsRef<str>,
{
    fn into_txt_entry(self) -> Result<(String, String), DiscoveryError> {
        Ok((self.0.as_ref().to_owned(), self.1.as_ref().to_owned()))
    }
}

impl<'a, K, V> TxtEntry for &'a (K, V)
where
    K: AsRef<str>,
    V: AsRef<str>,
{
    fn into_txt_entry(self) -> Result<(String, String), DiscoveryError> {
        Ok((self.0.as_ref().to_owned(), self.1.as_ref().to_owned()))
    }
}

impl TxtEntry for String {
    fn into_txt_entry(self) -> Result<(String, String), DiscoveryError> {
        let (key, value) = self
            .split_once('=')
            .ok_or(DiscoveryError::InvalidTxtField("entry"))?;
        Ok((key.to_owned(), value.to_owned()))
    }
}

impl<'a> TxtEntry for &'a str {
    fn into_txt_entry(self) -> Result<(String, String), DiscoveryError> {
        let (key, value) = self
            .split_once('=')
            .ok_or(DiscoveryError::InvalidTxtField("entry"))?;
        Ok((key.to_owned(), value.to_owned()))
    }
}

impl TxtEntry for mdns_sd::TxtProperty {
    fn into_txt_entry(self) -> Result<(String, String), DiscoveryError> {
        let value = self.val().unwrap_or_default();
        let value =
            std::str::from_utf8(value).map_err(|_| DiscoveryError::InvalidTxtField("value"))?;
        Ok((self.key().to_owned(), value.to_owned()))
    }
}

impl<'a> TxtEntry for &'a mdns_sd::TxtProperty {
    fn into_txt_entry(self) -> Result<(String, String), DiscoveryError> {
        let value = self.val().unwrap_or_default();
        let value =
            std::str::from_utf8(value).map_err(|_| DiscoveryError::InvalidTxtField("value"))?;
        Ok((self.key().to_owned(), value.to_owned()))
    }
}

fn parse_u64(value: &str) -> Option<u64> {
    value.parse::<u64>().ok().or_else(|| {
        value
            .strip_prefix("0x")
            .and_then(|hex| u64::from_str_radix(hex, 16).ok())
    })
}

/// A resolved record plus its deduplicated reachable endpoints.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoveredService {
    /// Public TXT metadata.
    pub record: ServiceRecord,
    /// mDNS host name, retained for display/diagnostics.
    pub hostname: String,
    /// Reachable IP endpoints, sorted and deduplicated.
    pub endpoints: Vec<Endpoint>,
}

/// Compatibility alias for callers that call a discovered service a peer.
pub type DiscoveredPeer = DiscoveredService;
/// Compatibility alias for the resolved mDNS service shape.
pub type ResolvedServiceRecord = DiscoveredService;

impl DiscoveredService {
    /// Build a service from a hostname and already validated endpoints.
    pub fn new(
        record: ServiceRecord,
        hostname: impl Into<String>,
        endpoints: impl IntoIterator<Item = Endpoint>,
    ) -> Result<Self, DiscoveryError> {
        record.validate()?;
        let hostname = validate_hostname(hostname.into())?;
        let mut unique = BTreeSet::new();
        for endpoint in endpoints {
            if endpoint.port != record.port {
                return Err(DiscoveryError::InvalidEndpoint(
                    "endpoint port differs from TXT port".to_owned(),
                ));
            }
            unique.insert(endpoint);
        }
        if unique.is_empty() {
            return Err(DiscoveryError::InvalidEndpoint(
                "service has no addresses".to_owned(),
            ));
        }
        Ok(Self {
            record,
            hostname,
            endpoints: unique.into_iter().collect(),
        })
    }

    /// Build a service from raw IP addresses, sorting and deduplicating them.
    pub fn from_ips<I>(
        record: ServiceRecord,
        hostname: impl Into<String>,
        addresses: I,
    ) -> Result<Self, DiscoveryError>
    where
        I: IntoIterator<Item = IpAddr>,
    {
        let endpoints = addresses
            .into_iter()
            .map(|address| Endpoint::new(address.to_string(), record.port))
            .collect::<Result<Vec<_>, NetworkError>>()?;
        Self::new(record, hostname, endpoints)
    }

    /// Return the service's device id.
    #[must_use]
    pub const fn device_id(&self) -> DeviceId {
        self.record.id
    }
}

/// Discovery updates sent to a [`DiscoveryHandle`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiscoveryEvent {
    /// A device was first observed.
    Found(DiscoveredService),
    /// A device's TXT metadata or addresses changed.
    Updated(DiscoveredService),
    /// A device disappeared from the local discovery source.
    Removed(DeviceId),
    /// The adapter received malformed/unusable data or encountered an
    /// operational error.  This does not terminate the provider.
    Error(DiscoveryError),
}

impl DiscoveryEvent {
    /// Return the device id carried by any non-error event.
    #[must_use]
    pub fn device_id(&self) -> Option<DeviceId> {
        match self {
            Self::Found(service) | Self::Updated(service) => Some(service.device_id()),
            Self::Removed(id) => Some(*id),
            Self::Error(_) => None,
        }
    }
}

/// A receiving endpoint for discovery events.
#[derive(Clone)]
pub struct DiscoveryHandle {
    receiver: Arc<Mutex<Receiver<DiscoveryEvent>>>,
}

impl fmt::Debug for DiscoveryHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DiscoveryHandle")
            .finish_non_exhaustive()
    }
}

impl DiscoveryHandle {
    fn new(receiver: Receiver<DiscoveryEvent>) -> Self {
        Self {
            receiver: Arc::new(Mutex::new(receiver)),
        }
    }

    /// Wait for the next event.
    pub fn recv(&self) -> Result<DiscoveryEvent, DiscoveryError> {
        self.receiver
            .lock()
            .map_err(|_| DiscoveryError::ChannelClosed)?
            .recv()
            .map_err(|_| DiscoveryError::ChannelClosed)
    }

    /// Wait up to `timeout` for the next event.
    pub fn recv_timeout(
        &self,
        timeout: Duration,
    ) -> Result<Option<DiscoveryEvent>, DiscoveryError> {
        match self
            .receiver
            .lock()
            .map_err(|_| DiscoveryError::ChannelClosed)?
            .recv_timeout(timeout)
        {
            Ok(event) => Ok(Some(event)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(DiscoveryError::ChannelClosed),
        }
    }

    /// Poll without blocking.
    pub fn try_recv(&self) -> Result<Option<DiscoveryEvent>, DiscoveryError> {
        match self
            .receiver
            .lock()
            .map_err(|_| DiscoveryError::ChannelClosed)?
            .try_recv()
        {
            Ok(event) => Ok(Some(event)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(DiscoveryError::ChannelClosed),
        }
    }

    /// Alias for [`recv`](Self::recv).
    pub fn next(&self) -> Result<DiscoveryEvent, DiscoveryError> {
        self.recv()
    }
}

/// Common lifecycle boundary for discovery implementations.
pub trait DiscoveryProvider: Send {
    /// Start the provider and return a handle for its events.
    fn start(&mut self) -> Result<DiscoveryHandle, DiscoveryError>;

    /// Issue a fresh query while keeping the existing event handle alive.
    fn refresh(&mut self) -> Result<(), DiscoveryError> {
        Ok(())
    }

    /// Stop background work and release sockets/threads.
    fn shutdown(&mut self) -> Result<(), DiscoveryError>;

    /// Advertise a public service record. Providers that do not publish may
    /// leave this default implementation in place.
    fn advertise(
        &mut self,
        _record: &ServiceRecord,
        _hostname: &str,
        _addresses: &[IpAddr],
    ) -> Result<(), DiscoveryError> {
        Err(DiscoveryError::ProviderUnavailable(
            "advertising is unsupported",
        ))
    }

    /// Compatibility alias for [`start`](Self::start).
    fn handle(&mut self) -> Result<DiscoveryHandle, DiscoveryError> {
        self.start()
    }
}

/// Deterministic discovery implementation used by tests and in-process callers.
///
/// Services are held in a `BTreeMap`; endpoints are held in a `BTreeSet`, so
/// insertion order and HashSet iteration order never affect emitted events.
#[derive(Debug)]
pub struct InMemoryDiscovery {
    local_device_id: DeviceId,
    services: BTreeMap<DeviceId, DiscoveredService>,
    sender: Sender<DiscoveryEvent>,
    handle: DiscoveryHandle,
    stopped: bool,
}

impl InMemoryDiscovery {
    /// Construct an empty deterministic provider.
    #[must_use]
    pub fn new(local_device_id: DeviceId) -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            local_device_id,
            services: BTreeMap::new(),
            sender,
            handle: DiscoveryHandle::new(receiver),
            stopped: false,
        }
    }

    /// Return the local id used to filter self-discovery.
    #[must_use]
    pub const fn local_device_id(&self) -> DeviceId {
        self.local_device_id
    }

    /// Start the provider and return its event handle. This inherent method is
    /// available without importing [`DiscoveryProvider`].
    pub fn handle(&mut self) -> Result<DiscoveryHandle, DiscoveryError> {
        <Self as DiscoveryProvider>::start(self)
    }

    /// Return a clone of the event handle without changing lifecycle state.
    #[must_use]
    pub fn event_handle(&self) -> DiscoveryHandle {
        self.handle.clone()
    }

    /// Insert or update a resolved service and emit the corresponding event.
    pub fn upsert(&mut self, service: DiscoveredService) -> Result<(), DiscoveryError> {
        self.ensure_running()?;
        if service.device_id() == self.local_device_id {
            return Ok(());
        }
        let event = match self.services.insert(service.device_id(), service.clone()) {
            Some(previous) if previous == service => return Ok(()),
            Some(_) => DiscoveryEvent::Updated(service),
            None => DiscoveryEvent::Found(service),
        };
        self.send(event)
    }

    /// Advertise/insert an in-process service. This mirrors the mDNS adapter
    /// and is useful for deterministic tests.
    pub fn advertise(
        &mut self,
        record: &ServiceRecord,
        hostname: &str,
        addresses: &[IpAddr],
    ) -> Result<(), DiscoveryError> {
        let service =
            DiscoveredService::from_ips(record.clone(), hostname, addresses.iter().copied())?;
        self.upsert(service)
    }

    /// Alias for [`upsert`](Self::upsert).
    pub fn discover(&mut self, service: DiscoveredService) -> Result<(), DiscoveryError> {
        self.upsert(service)
    }

    /// Alias for [`upsert`](Self::upsert) used by event-oriented callers.
    pub fn found(&mut self, service: DiscoveredService) -> Result<(), DiscoveryError> {
        self.upsert(service)
    }

    /// Alias for [`upsert`](Self::upsert) used by event-oriented callers.
    pub fn updated(&mut self, service: DiscoveredService) -> Result<(), DiscoveryError> {
        self.upsert(service)
    }

    /// Alias for [`remove`](Self::remove).
    pub fn removed(&mut self, id: DeviceId) -> Result<(), DiscoveryError> {
        self.remove(id)
    }

    /// Remove a service from discovery state.  This only emits a discovery
    /// event; it never changes a trust store or pairing decision.
    pub fn remove(&mut self, id: DeviceId) -> Result<(), DiscoveryError> {
        self.ensure_running()?;
        if self.services.remove(&id).is_some() {
            self.send(DiscoveryEvent::Removed(id))?;
        }
        Ok(())
    }

    /// Return an immutable deterministic snapshot of currently known peers.
    #[must_use]
    pub fn services(&self) -> Vec<DiscoveredService> {
        self.services.values().cloned().collect()
    }

    fn ensure_running(&self) -> Result<(), DiscoveryError> {
        if self.stopped {
            Err(DiscoveryError::ProviderUnavailable("provider is shut down"))
        } else {
            Ok(())
        }
    }

    fn send(&self, event: DiscoveryEvent) -> Result<(), DiscoveryError> {
        self.sender
            .send(event)
            .map_err(|_| DiscoveryError::ChannelClosed)
    }
}

impl DiscoveryProvider for InMemoryDiscovery {
    fn start(&mut self) -> Result<DiscoveryHandle, DiscoveryError> {
        if self.stopped {
            return Err(DiscoveryError::ProviderUnavailable("provider is shut down"));
        }
        Ok(self.handle.clone())
    }

    fn shutdown(&mut self) -> Result<(), DiscoveryError> {
        self.stopped = true;
        Ok(())
    }

    fn advertise(
        &mut self,
        record: &ServiceRecord,
        hostname: &str,
        addresses: &[IpAddr],
    ) -> Result<(), DiscoveryError> {
        InMemoryDiscovery::advertise(self, record, hostname, addresses)
    }
}

/// Manual endpoint registry.  It performs strict host validation and stores
/// endpoint sets per device id.  Removing a manual endpoint has no trust-store
/// side effect.
#[derive(Clone, Debug, Default)]
pub struct ManualEndpointRegistry {
    expected_device_id: Option<DeviceId>,
    endpoints: BTreeMap<DeviceId, BTreeSet<Endpoint>>,
}

impl ManualEndpointRegistry {
    /// Construct a registry, optionally restricting additions to one expected id.
    #[must_use]
    pub fn new(expected_device_id: Option<DeviceId>) -> Self {
        Self {
            expected_device_id,
            endpoints: BTreeMap::new(),
        }
    }

    /// Construct an unrestricted registry.
    #[must_use]
    pub fn unrestricted() -> Self {
        Self::new(None)
    }

    /// Construct a registry restricted to `expected_device_id`.
    #[must_use]
    pub fn for_device(expected_device_id: DeviceId) -> Self {
        Self::new(Some(expected_device_id))
    }

    /// Return the optional expected id.
    #[must_use]
    pub const fn expected_device_id(&self) -> Option<DeviceId> {
        self.expected_device_id
    }

    /// Set or clear the expected id. Existing entries are retained only when
    /// they still satisfy the new restriction.
    pub fn set_expected_device_id(&mut self, expected: Option<DeviceId>) {
        self.expected_device_id = expected;
        if let Some(expected) = expected {
            self.endpoints.retain(|id, _| *id == expected);
        }
    }

    /// Add one endpoint, returning `true` only when it was not already present.
    pub fn add(
        &mut self,
        id: DeviceId,
        host: impl Into<String>,
        port: u16,
    ) -> Result<bool, DiscoveryError> {
        self.add_endpoint(
            id,
            Endpoint::new(validate_endpoint_host(host.into())?, port)
                .map_err(DiscoveryError::from)?,
        )
    }

    /// Add an already core-validated endpoint after strict host validation.
    pub fn add_endpoint(
        &mut self,
        id: DeviceId,
        endpoint: Endpoint,
    ) -> Result<bool, DiscoveryError> {
        self.ensure_expected(id)?;
        let host = validate_endpoint_host(endpoint.host.clone())?;
        let endpoint = Endpoint::new(host, endpoint.port).map_err(DiscoveryError::from)?;
        Ok(self.endpoints.entry(id).or_default().insert(endpoint))
    }

    /// Compatibility alias for [`add`](Self::add).
    pub fn insert(
        &mut self,
        id: DeviceId,
        host: impl Into<String>,
        port: u16,
    ) -> Result<bool, DiscoveryError> {
        self.add(id, host, port)
    }

    /// Remove one endpoint. This does not revoke or alter trust.
    pub fn remove(
        &mut self,
        id: DeviceId,
        host: impl Into<String>,
        port: u16,
    ) -> Result<bool, DiscoveryError> {
        let host = validate_endpoint_host(host.into())?;
        let endpoint = Endpoint::new(host, port).map_err(DiscoveryError::from)?;
        Ok(self.remove_endpoint(id, &endpoint))
    }

    /// Remove one endpoint. This does not revoke or alter trust.
    pub fn remove_endpoint(&mut self, id: DeviceId, endpoint: &Endpoint) -> bool {
        let Some(set) = self.endpoints.get_mut(&id) else {
            return false;
        };
        let removed = set.remove(endpoint);
        if set.is_empty() {
            self.endpoints.remove(&id);
        }
        removed
    }

    /// Remove all endpoints for a device. This does not revoke or alter trust.
    pub fn remove_device(&mut self, id: DeviceId) -> bool {
        self.endpoints.remove(&id).is_some()
    }

    /// Return a sorted copy of endpoints for a device.
    #[must_use]
    pub fn get(&self, id: &DeviceId) -> Vec<Endpoint> {
        self.endpoints
            .get(id)
            .map_or_else(Vec::new, |set| set.iter().cloned().collect())
    }

    /// Alias for [`get`](Self::get).
    #[must_use]
    pub fn endpoints(&self, id: DeviceId) -> Vec<Endpoint> {
        self.get(&id)
    }

    /// Alias for [`get`](Self::get).
    #[must_use]
    pub fn all(&self, id: DeviceId) -> Vec<Endpoint> {
        self.get(&id)
    }

    /// Return whether the exact endpoint is present for a device.
    #[must_use]
    pub fn contains(&self, id: DeviceId, endpoint: &Endpoint) -> bool {
        self.endpoints
            .get(&id)
            .is_some_and(|set| set.contains(endpoint))
    }

    /// Return all device ids in deterministic order.
    #[must_use]
    pub fn device_ids(&self) -> Vec<DeviceId> {
        self.endpoints.keys().copied().collect()
    }

    /// Number of stored endpoints.
    #[must_use]
    pub fn len(&self) -> usize {
        self.endpoints.values().map(BTreeSet::len).sum()
    }

    /// Whether no manual endpoints are stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.endpoints.is_empty()
    }

    fn ensure_expected(&self, id: DeviceId) -> Result<(), DiscoveryError> {
        if self
            .expected_device_id
            .is_some_and(|expected| expected != id)
        {
            Err(DiscoveryError::UnexpectedDevice)
        } else {
            Ok(())
        }
    }
}

/// Validate and normalize a manual endpoint host. IPv4, IPv6 (including
/// bracketed input), and DNS hostnames are accepted.
pub fn validate_host(host: impl Into<String>) -> Result<String, DiscoveryError> {
    validate_endpoint_host(host.into())
}

fn validate_endpoint_host(host: String) -> Result<String, DiscoveryError> {
    let trimmed = host.trim().trim_end_matches('.').to_owned();
    let trimmed = trimmed
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(&trimmed)
        .to_owned();
    if trimmed.is_empty() {
        return Err(DiscoveryError::InvalidEndpoint("host is empty".to_owned()));
    }
    if let Ok(address) = trimmed.parse::<IpAddr>() {
        return Ok(address.to_string());
    }
    if trimmed.contains('.')
        && trimmed
            .split('.')
            .all(|label| !label.is_empty() && label.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err(DiscoveryError::InvalidEndpoint(
            "invalid IP address".to_owned(),
        ));
    }
    validate_hostname(trimmed)
}

fn validate_hostname(host: String) -> Result<String, DiscoveryError> {
    let host = host.trim().trim_end_matches('.').to_owned();
    if host.is_empty() || host.len() > 253 || !host.is_ascii() {
        return Err(DiscoveryError::InvalidEndpoint(
            "invalid hostname".to_owned(),
        ));
    }
    for label in host.split('.') {
        if label.is_empty()
            || label.len() > 63
            || label.starts_with('-')
            || label.ends_with('-')
            || !label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(DiscoveryError::InvalidEndpoint(
                "invalid hostname".to_owned(),
            ));
        }
    }
    Ok(host.to_ascii_lowercase())
}

/// mDNS-backed provider.  mdns-sd owns its socket daemon; this adapter owns a
/// separate bridge thread that converts mdns-sd events to [`DiscoveryEvent`].
pub struct MdnsDiscovery {
    local_device_id: DeviceId,
    daemon_port: u16,
    service_type: &'static str,
    daemon: Option<ServiceDaemon>,
    sender: Option<Sender<DiscoveryEvent>>,
    handle: DiscoveryHandle,
    stop: Arc<AtomicBool>,
    bridge: Option<JoinHandle<()>>,
    browse: Option<Arc<Mutex<BrowseSession>>>,
    monitor_bridge: Option<JoinHandle<()>>,
    advertisement: Option<ServiceRecord>,
    advertised: Vec<String>,
    started: bool,
    stopped: bool,
}

struct BrowseSession {
    receiver: mdns_sd::Receiver<ServiceEvent>,
    generation: u64,
}

impl fmt::Debug for MdnsDiscovery {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MdnsDiscovery")
            .field("local_device_id", &self.local_device_id)
            .field("daemon_port", &self.daemon_port)
            .field("service_type", &self.service_type)
            .field("started", &self.started)
            .field("stopped", &self.stopped)
            .field("advertised_count", &self.advertised.len())
            .finish_non_exhaustive()
    }
}

impl MdnsDiscovery {
    /// Create a provider using the standard mDNS daemon port.
    pub fn new(local_device_id: DeviceId) -> Result<Self, DiscoveryError> {
        Self::new_with_type(local_device_id, mdns_sd::MDNS_PORT, SERVICE_TYPE)
    }

    /// Create the isolated input-service browser/advertiser.
    pub fn for_input_service(local_device_id: DeviceId) -> Result<Self, DiscoveryError> {
        Self::new_with_type(local_device_id, mdns_sd::MDNS_PORT, INPUT_SERVICE_TYPE)
    }

    /// Create a provider with a custom daemon port, useful for isolated tests.
    ///
    /// Construction is deliberately side-effect free: it stores only the
    /// configuration and allocates an in-process event channel.  The mDNS
    /// daemon, sockets, browse subscription, and bridge threads are created by
    /// [`DiscoveryProvider::start`] instead.
    pub fn new_with_port(local_device_id: DeviceId, port: u16) -> Result<Self, DiscoveryError> {
        Self::new_with_type(local_device_id, port, SERVICE_TYPE)
    }

    fn new_with_type(
        local_device_id: DeviceId,
        port: u16,
        service_type: &'static str,
    ) -> Result<Self, DiscoveryError> {
        let (sender, receiver) = mpsc::channel();
        Ok(Self {
            local_device_id,
            daemon_port: port,
            service_type,
            daemon: None,
            sender: Some(sender),
            handle: DiscoveryHandle::new(receiver),
            stop: Arc::new(AtomicBool::new(false)),
            bridge: None,
            browse: None,
            monitor_bridge: None,
            advertisement: None,
            advertised: Vec::new(),
            started: false,
            stopped: false,
        })
    }

    /// Return the local id used to filter self-discovery.
    #[must_use]
    pub const fn local_device_id(&self) -> DeviceId {
        self.local_device_id
    }

    /// Start the provider and return its event handle. This inherent method is
    /// available without importing [`DiscoveryProvider`].
    pub fn handle(&mut self) -> Result<DiscoveryHandle, DiscoveryError> {
        <Self as DiscoveryProvider>::start(self)
    }

    /// Return another event handle.
    #[must_use]
    pub fn event_handle(&self) -> DiscoveryHandle {
        self.handle.clone()
    }

    /// Return whether the daemon and bridge threads are currently running.
    #[must_use]
    pub const fn is_started(&self) -> bool {
        self.started
    }

    /// Return the configured mDNS daemon port.
    ///
    /// This is the port used by the discovery daemon itself (normally 5353),
    /// not the application service port carried in [`ServiceRecord::port`].
    #[must_use]
    pub const fn daemon_port(&self) -> u16 {
        self.daemon_port
    }

    /// Configure one local public record to publish automatically on start.
    ///
    /// The record is validated immediately and must belong to this provider's
    /// local device.  The record's service port does not need to equal
    /// [`Self::daemon_port`]: the former is the application SRV port, while
    /// the latter is the mDNS daemon's multicast socket port.
    pub fn with_advertisement(mut self, record: ServiceRecord) -> Result<Self, DiscoveryError> {
        self.set_advertisement(record)?;
        Ok(self)
    }

    /// Configure or replace the automatic local advertisement before start.
    pub fn set_advertisement(&mut self, record: ServiceRecord) -> Result<(), DiscoveryError> {
        if self.stopped {
            return Err(DiscoveryError::ProviderUnavailable("provider is shut down"));
        }
        if self.started {
            return Err(DiscoveryError::ProviderUnavailable(
                "provider is already started",
            ));
        }
        record.validate()?;
        if record.id != self.local_device_id {
            return Err(DiscoveryError::UnexpectedDevice);
        }
        self.advertisement = Some(record);
        Ok(())
    }

    /// Return the configured automatic advertisement, if any.
    #[must_use]
    pub fn advertisement(&self) -> Option<&ServiceRecord> {
        self.advertisement.as_ref()
    }

    /// Advertise a local public record.  The record's id must be this provider's
    /// local id; any extra fields are impossible because TXT encoding is fixed.
    pub fn advertise<I>(
        &mut self,
        record: &ServiceRecord,
        hostname: &str,
        addresses: I,
    ) -> Result<(), DiscoveryError>
    where
        I: IntoIterator<Item = IpAddr>,
    {
        self.ensure_running()?;
        record.validate()?;
        if record.id != self.local_device_id {
            return Err(DiscoveryError::UnexpectedDevice);
        }
        let hostname = validate_hostname(hostname.to_owned())?;
        let addresses = addresses.into_iter().collect::<Vec<_>>();
        if addresses.is_empty() {
            return Err(DiscoveryError::InvalidEndpoint(
                "advertisement has no addresses".to_owned(),
            ));
        }
        // The instance label is a stable 128-bit prefix of the device id.
        // DNS labels are limited to 63 octets, so the full 64-hex id cannot
        // be used as an instance name; the authoritative full id stays in TXT.
        let instance = service_instance_label(&record.id);
        let service = ServiceInfo::new(
            self.service_type,
            &instance,
            &hostname,
            &addresses[..],
            record.port,
            &record.encode_txt()[..],
        )?;
        let fullname = service.get_fullname().to_owned();
        self.daemon
            .as_ref()
            .ok_or(DiscoveryError::ProviderUnavailable("daemon is gone"))?
            .register(service)?;
        if !self.advertised.contains(&fullname) {
            self.advertised.push(fullname);
        }
        Ok(())
    }

    /// Unregister one local advertisement.
    pub fn unadvertise(&mut self, record: &ServiceRecord) -> Result<(), DiscoveryError> {
        self.ensure_running()?;
        if record.id != self.local_device_id {
            return Err(DiscoveryError::UnexpectedDevice);
        }
        let fullname = self
            .advertised
            .iter()
            .find(|name| name.starts_with(&record.id.to_hex()))
            .cloned();
        if let Some(fullname) = fullname {
            let daemon = self
                .daemon
                .as_ref()
                .ok_or(DiscoveryError::ProviderUnavailable("daemon is gone"))?;
            let _ = daemon.unregister(&fullname)?;
            self.advertised.retain(|name| name != &fullname);
        }
        Ok(())
    }

    fn ensure_running(&self) -> Result<(), DiscoveryError> {
        if self.stopped {
            return Err(DiscoveryError::ProviderUnavailable("provider is shut down"));
        }
        if !self.started {
            return Err(DiscoveryError::ProviderUnavailable(
                "provider is not started",
            ));
        }
        Ok(())
    }
}

impl DiscoveryProvider for MdnsDiscovery {
    fn start(&mut self) -> Result<DiscoveryHandle, DiscoveryError> {
        if self.stopped {
            return Err(DiscoveryError::ProviderUnavailable("provider is shut down"));
        }
        if self.started {
            return Err(DiscoveryError::ProviderUnavailable(
                "provider is already started",
            ));
        }

        let sender = self
            .sender
            .as_ref()
            .ok_or(DiscoveryError::ProviderUnavailable("event channel is gone"))?
            .clone();
        let pending_advertisement = self
            .advertisement
            .as_ref()
            .map(|record| automatic_service_info(record, self.service_type))
            .transpose()?;

        self.stop.store(false, Ordering::Release);
        let daemon = ServiceDaemon::new_with_port(self.daemon_port)?;
        let browse = match daemon.browse(self.service_type) {
            Ok(receiver) => receiver,
            Err(error) => {
                let _ = daemon.shutdown();
                return Err(error.into());
            }
        };
        let monitor = match daemon.monitor() {
            Ok(receiver) => receiver,
            Err(error) => {
                let _ = daemon.shutdown();
                return Err(error.into());
            }
        };

        let browse = Arc::new(Mutex::new(BrowseSession {
            receiver: browse,
            generation: 0,
        }));
        let bridge_browse = Arc::clone(&browse);
        let bridge_stop = Arc::clone(&self.stop);
        let bridge_sender = sender.clone();
        let local_device_id = self.local_device_id;
        let bridge = match thread::Builder::new()
            .name("deskfuse-discovery-mdns".to_owned())
            .spawn(move || bridge_loop(bridge_browse, bridge_sender, bridge_stop, local_device_id))
        {
            Ok(thread) => thread,
            Err(error) => {
                stop_mdns_resources(&self.stop, Some(daemon), None, None);
                self.stop.store(false, Ordering::Release);
                return Err(DiscoveryError::Mdns(error.to_string()));
            }
        };
        let monitor_stop = Arc::clone(&self.stop);
        let monitor_sender = sender;
        let monitor_bridge = match thread::Builder::new()
            .name("deskfuse-discovery-mdns-monitor".to_owned())
            .spawn(move || monitor_loop(monitor, monitor_sender, monitor_stop))
        {
            Ok(thread) => thread,
            Err(error) => {
                stop_mdns_resources(&self.stop, Some(daemon), Some(bridge), None);
                self.stop.store(false, Ordering::Release);
                return Err(DiscoveryError::Mdns(error.to_string()));
            }
        };

        let automatic_fullname = if let Some((service, fullname)) = pending_advertisement {
            if let Err(error) = daemon.register(service) {
                stop_mdns_resources(&self.stop, Some(daemon), Some(bridge), Some(monitor_bridge));
                self.stop.store(false, Ordering::Release);
                return Err(error.into());
            }
            Some(fullname)
        } else {
            None
        };

        self.daemon = Some(daemon);
        self.browse = Some(browse);
        self.bridge = Some(bridge);
        self.monitor_bridge = Some(monitor_bridge);
        if let Some(fullname) = automatic_fullname {
            self.advertised.push(fullname);
        }
        self.started = true;
        Ok(self.handle.clone())
    }

    fn refresh(&mut self) -> Result<(), DiscoveryError> {
        self.ensure_running()?;
        let daemon = self
            .daemon
            .as_ref()
            .ok_or(DiscoveryError::ProviderUnavailable("daemon is gone"))?;
        let mut browse = self
            .browse
            .as_ref()
            .ok_or(DiscoveryError::ProviderUnavailable("browse is gone"))?
            .lock()
            .map_err(|_| DiscoveryError::ProviderUnavailable("browse is poisoned"))?;
        // stop_browse clears mdns-sd's browse cache and pending queries. Keep
        // the daemon and advertisement alive, and replace only the receiver.
        daemon.stop_browse(self.service_type)?;
        browse.receiver = daemon.browse(self.service_type)?;
        browse.generation = browse.generation.wrapping_add(1);
        Ok(())
    }

    fn advertise(
        &mut self,
        record: &ServiceRecord,
        hostname: &str,
        addresses: &[IpAddr],
    ) -> Result<(), DiscoveryError> {
        MdnsDiscovery::advertise(self, record, hostname, addresses.iter().copied())
    }

    fn shutdown(&mut self) -> Result<(), DiscoveryError> {
        if self.stopped {
            return Ok(());
        }
        self.stopped = true;
        self.stop.store(true, Ordering::Release);
        if let Some(daemon) = self.daemon.take() {
            for fullname in self.advertised.drain(..) {
                let _ = daemon.unregister(&fullname);
            }
            let _ = daemon.shutdown();
        } else {
            self.advertised.clear();
        }
        if let Some(thread) = self.bridge.take() {
            let _ = thread.join();
        }
        self.browse.take();
        if let Some(thread) = self.monitor_bridge.take() {
            let _ = thread.join();
        }
        self.started = false;
        // Drop the provider's sender after workers have exited.  Existing
        // handles can drain queued events and then observe channel closure.
        self.sender.take();
        Ok(())
    }
}

/// Build the default service info used by [`MdnsDiscovery::with_advertisement`].
///
/// mdns-sd can discover the local addresses itself when `enable_addr_auto()` is
/// used.  The synthetic host name is stable for a device and remains within a
/// DNS label's length limit; the service instance still carries the full device
/// id.  `daemon_port` is intentionally not involved here because it is the mDNS
/// socket port, whereas `record.port` is the application service's SRV port.
fn automatic_service_info(
    record: &ServiceRecord,
    service_type: &'static str,
) -> Result<(ServiceInfo, String), DiscoveryError> {
    record.validate()?;
    let hostname = deskfuse_core::device_hostname(record.id);
    let instance = service_instance_label(&record.id);
    let service = ServiceInfo::new(
        service_type,
        &instance,
        &hostname,
        "",
        record.port,
        &record.encode_txt()[..],
    )?
    .enable_addr_auto();
    let fullname = service.get_fullname().to_owned();
    Ok((service, fullname))
}

fn stop_mdns_resources(
    stop: &Arc<AtomicBool>,
    daemon: Option<ServiceDaemon>,
    bridge: Option<JoinHandle<()>>,
    monitor_bridge: Option<JoinHandle<()>>,
) {
    stop.store(true, Ordering::Release);
    if let Some(daemon) = daemon {
        let _ = daemon.shutdown();
    }
    if let Some(thread) = bridge {
        let _ = thread.join();
    }
    if let Some(thread) = monitor_bridge {
        let _ = thread.join();
    }
}

fn bridge_loop(
    browse: Arc<Mutex<BrowseSession>>,
    sender: Sender<DiscoveryEvent>,
    stop: Arc<AtomicBool>,
    local_device_id: DeviceId,
) {
    let mut known = BTreeMap::<String, DiscoveredService>::new();
    let mut known_generation = 0;
    while !stop.load(Ordering::Acquire) {
        let (receiver, generation) = match browse.lock() {
            Ok(session) => (session.receiver.clone(), session.generation),
            Err(_) => break,
        };
        let event = receiver.recv_timeout(Duration::from_millis(100));
        let Ok(session) = browse.lock() else {
            break;
        };
        // An old receiver may still have queued removals after a refresh.
        // Serialize delivery with the swap so they cannot erase fresh results.
        if session.generation != generation {
            continue;
        }
        if known_generation != generation {
            // A consumer may have forgotten a peer without the mDNS service
            // changing. Re-publish the still-known services on explicit search;
            // normal duplicate announcements remain coalesced. Removals keep
            // expiring this cache, and hints never grant application trust.
            for service in known.values() {
                let _ = sender.send(DiscoveryEvent::Found(service.clone()));
            }
            known_generation = generation;
        }
        match event {
            Ok(ServiceEvent::ServiceResolved(service)) => {
                handle_resolved(service, &sender, &mut known, local_device_id);
            }
            Ok(ServiceEvent::ServiceRemoved(_, fullname)) => {
                let id = known
                    .remove(&fullname)
                    .map(|service| service.device_id())
                    .or_else(|| parse_instance_device_id(&fullname))
                    .or_else(|| match_instance_label(&known, &fullname));
                if let Some(id) = id
                    && id != local_device_id
                {
                    let _ = sender.send(DiscoveryEvent::Removed(id));
                }
            }
            Ok(ServiceEvent::ServiceFound(_, _))
            | Ok(ServiceEvent::SearchStarted(_))
            | Ok(ServiceEvent::SearchStopped(_))
            | Ok(_) => {}
            Err(mdns_sd::RecvTimeoutError::Timeout) => {}
            Err(mdns_sd::RecvTimeoutError::Disconnected) => {
                drop(session);
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

fn monitor_loop(
    receiver: mdns_sd::Receiver<DaemonEvent>,
    sender: Sender<DiscoveryEvent>,
    stop: Arc<AtomicBool>,
) {
    while !stop.load(Ordering::Acquire) {
        match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(DaemonEvent::Error(error)) => {
                let _ = sender.send(DiscoveryEvent::Error(DiscoveryError::Mdns(
                    error.to_string(),
                )));
            }
            Ok(_) => {}
            Err(mdns_sd::RecvTimeoutError::Timeout) => {}
            Err(mdns_sd::RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn handle_resolved(
    service: Box<ResolvedService>,
    sender: &Sender<DiscoveryEvent>,
    known: &mut BTreeMap<String, DiscoveredService>,
    local_device_id: DeviceId,
) {
    let fullname = service.get_fullname().to_owned();
    let record = match ServiceRecord::from_txt_properties(service.get_properties()) {
        Ok(record) => record,
        Err(error) => {
            let _ = sender.send(DiscoveryEvent::Error(error));
            return;
        }
    };
    if record.id == local_device_id {
        return;
    }
    if service.get_port() != record.port {
        let _ = sender.send(DiscoveryEvent::Error(DiscoveryError::InvalidPort));
        return;
    }
    let addresses = service
        .get_addresses()
        .iter()
        .map(mdns_sd::ScopedIp::to_ip_addr)
        .filter(is_dialable_peer_address)
        .collect::<Vec<_>>();
    // A service whose only advertised addresses are non-dialable (loopback,
    // link-local IPv6, APIPA, multicast) must not become a route hint.
    if addresses.is_empty() {
        return;
    }
    let discovered = match DiscoveredService::from_ips(record, service.get_hostname(), addresses) {
        Ok(service) => service,
        Err(error) => {
            let _ = sender.send(DiscoveryEvent::Error(error));
            return;
        }
    };
    let event = match known.insert(fullname, discovered.clone()) {
        Some(previous) if previous == discovered => return,
        Some(_) => DiscoveryEvent::Updated(discovered),
        None => DiscoveryEvent::Found(discovered),
    };
    let _ = sender.send(event);
}

/// Whether an advertised address can actually be dialed by the current
/// transport as a peer route.
///
/// The M1 transport binds an IPv4 socket and the [`Endpoint`] wire type does
/// not carry an IPv6 zone/scope id, so IPv6 link-local addresses cannot be
/// dialed and are intentionally excluded. Loopback, APIPA, multicast, and
/// broadcast addresses never identify a remote peer and are also excluded.
/// Global IPv6 route support is a later milestone.
fn is_dialable_peer_address(address: &IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            let octets = address.octets();
            !address.is_unspecified()
                && !address.is_loopback()
                && !address.is_multicast()
                && !address.is_broadcast()
                && !(octets[0] == 169 && octets[1] == 254)
        }
        IpAddr::V6(_) => false,
    }
}

/// Stable DNS-safe instance label: the first 128 bits of the device id hex.
///
/// The full device id is authoritative and is carried in the TXT `id` field;
/// the instance label only needs to be stable and unique enough to correlate
/// discovery add/remove events.
fn service_instance_label(id: &DeviceId) -> String {
    let hex = id.to_hex();
    hex[..32].to_owned()
}

/// Legacy fallback for peers that encode the complete 64-hex device id in the
/// instance label. Current DeskFuse advertisements use the 32-hex label and are
/// matched through [`match_instance_label`] instead.
fn parse_instance_device_id(fullname: &str) -> Option<DeviceId> {
    let instance = fullname.split_once("._deskfuse._udp.local.")?.0;
    DeviceId::from_hex(instance.trim_end_matches('.')).ok()
}

/// Match a removed service fullname against the known map using its instance
/// label, covering the truncated 32-hex instance format used on the wire.
fn match_instance_label(
    known: &BTreeMap<String, DiscoveredService>,
    fullname: &str,
) -> Option<DeviceId> {
    let label = fullname.split_once('.')?.0;
    let key = known.keys().find(|name| {
        name.split_once('.')
            .is_some_and(|(candidate, _)| candidate == label)
    })?;
    known.get(key).map(|service| service.device_id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    fn id(byte: u8) -> DeviceId {
        DeviceId::from_bytes([byte; 32])
    }

    fn record(byte: u8) -> ServiceRecord {
        ServiceRecord::v2(
            id(byte),
            "Test Mac",
            Capabilities::NETWORK | Capabilities::TRANSFER,
            9876,
        )
        .expect("valid")
    }

    #[test]
    fn txt_round_trip_is_allow_listed_and_secret_free() {
        let record = record(1);
        let txt = record.encode_txt();
        assert_eq!(
            txt.iter().map(|(key, _)| key.as_str()).collect::<Vec<_>>(),
            vec!["v", "id", "name", "protocol", "capabilities", "port"]
        );
        let joined = format!("{txt:?}");
        assert!(!joined.contains("secret"));
        assert!(!joined.contains("private"));
        assert_eq!(ServiceRecord::decode_txt(&txt).expect("decode"), record);
    }

    #[test]
    fn txt_missing_fields_and_bad_values_are_errors_not_panics() {
        assert!(matches!(
            ServiceRecord::decode_txt([("v", "2.0")]),
            Err(DiscoveryError::MissingTxtField("id"))
        ));
        let bad_id = record(2)
            .encode_txt()
            .into_iter()
            .map(|(key, value)| {
                (
                    key.clone(),
                    if key == "id" { "bad".to_owned() } else { value },
                )
            })
            .collect::<Vec<_>>();
        assert!(matches!(
            ServiceRecord::decode_txt(&bad_id),
            Err(DiscoveryError::InvalidDeviceId)
        ));
        let id_hex = id(2).to_hex();
        let bad_port = vec![
            ("v", "2.0"),
            ("id", id_hex.as_str()),
            ("name", "name"),
            ("protocol", "udp"),
            ("capabilities", "1"),
            ("port", "0"),
        ];
        assert!(matches!(
            ServiceRecord::decode_txt(&bad_port),
            Err(DiscoveryError::InvalidPort)
        ));
        assert!(matches!(
            ServiceRecord::decode_txt([
                ("v", "3.0"),
                ("id", id_hex.as_str()),
                ("name", "name"),
                ("protocol", "udp"),
                ("capabilities", "1"),
                ("port", "4")
            ]),
            Err(DiscoveryError::InvalidVersion)
        ));
    }

    #[test]
    fn multi_address_service_is_sorted_and_deduplicated() {
        let service = DiscoveredService::from_ips(
            record(3),
            "Mac.local.",
            [
                IpAddr::V6(Ipv6Addr::LOCALHOST),
                IpAddr::V4(Ipv4Addr::LOCALHOST),
                IpAddr::V4(Ipv4Addr::LOCALHOST),
            ],
        )
        .expect("service");
        assert_eq!(service.endpoints.len(), 2);
        assert!(service.endpoints[0].host < service.endpoints[1].host);
        assert_eq!(service.hostname, "mac.local");
    }

    #[test]
    fn in_memory_is_deterministic_filters_self_and_removes_only_discovery_state() {
        let mut discovery = InMemoryDiscovery::new(id(7));
        let handle = discovery.start().expect("start");
        discovery
            .upsert(
                DiscoveredService::from_ips(
                    record(7),
                    "self.local",
                    [IpAddr::V4(Ipv4Addr::LOCALHOST)],
                )
                .expect("self"),
            )
            .expect("filter");
        assert!(handle.try_recv().expect("poll").is_none());
        let service =
            DiscoveredService::from_ips(record(8), "peer.local", [IpAddr::V4(Ipv4Addr::LOCALHOST)])
                .expect("peer");
        discovery.upsert(service.clone()).expect("found");
        assert!(
            matches!(handle.try_recv().expect("found").expect("event"), DiscoveryEvent::Found(found) if found == service)
        );
        discovery.remove(id(8)).expect("remove");
        assert!(
            matches!(handle.try_recv().expect("removed").expect("event"), DiscoveryEvent::Removed(found) if found == id(8))
        );
    }

    #[test]
    fn mdns_provider_is_lazy_starts_once_and_shutdown_is_idempotent() {
        let mut discovery = MdnsDiscovery::new_with_port(id(10), 0)
            .expect("configuration")
            .with_advertisement(record(10))
            .expect("matching advertisement");
        assert!(!discovery.is_started());
        assert_eq!(discovery.daemon_port(), 0);
        assert_eq!(discovery.advertisement(), Some(&record(10)));
        assert!(discovery.advertised.is_empty());
        assert!(matches!(
            discovery.advertise(&record(10), "mac.local", std::iter::empty::<IpAddr>()),
            Err(DiscoveryError::ProviderUnavailable(
                "provider is not started"
            ))
        ));

        let _handle = discovery.start().expect("start");
        assert!(discovery.is_started());
        assert_eq!(discovery.advertised.len(), 1);
        assert!(matches!(
            discovery.start(),
            Err(DiscoveryError::ProviderUnavailable(
                "provider is already started"
            ))
        ));
        assert_eq!(discovery.advertised.len(), 1);

        discovery.shutdown().expect("shutdown");
        assert!(!discovery.is_started());
        assert!(discovery.advertised.is_empty());
        discovery.shutdown().expect("repeated shutdown");
        assert!(matches!(
            discovery.start(),
            Err(DiscoveryError::ProviderUnavailable("provider is shut down"))
        ));
    }

    #[test]
    fn refresh_issues_new_mdns_queries_without_replacing_advertisement_or_event_handle() {
        let mut discovery = MdnsDiscovery::new_with_port(id(10), 0)
            .expect("configuration")
            .with_advertisement(record(10))
            .expect("advertisement");
        let handle = discovery.start().expect("start");
        let advertised = discovery.advertised.clone();
        let metrics = || {
            discovery
                .daemon
                .as_ref()
                .expect("daemon")
                .get_metrics()
                .expect("request metrics")
                .recv_timeout(Duration::from_secs(2))
                .expect("metrics")
        };
        let before = metrics().get("browse").copied().unwrap_or_default();
        for expected in 1..=3 {
            discovery.refresh().expect("refresh");
            // mdns-sd intentionally jitters the first query; a queued browse
            // command is not evidence that a packet has been sent yet.
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            loop {
                let metrics = discovery
                    .daemon
                    .as_ref()
                    .expect("daemon")
                    .get_metrics()
                    .expect("request metrics")
                    .recv_timeout(Duration::from_secs(2))
                    .expect("metrics");
                if metrics.get("browse").copied().unwrap_or_default() >= before + expected {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "refresh did not send a query"
                );
                thread::sleep(Duration::from_millis(20));
            }
            assert_eq!(discovery.advertised, advertised);
            assert!(handle.try_recv().is_ok(), "existing handle must stay open");
        }
        discovery.shutdown().expect("shutdown");
        assert!(discovery.refresh().is_err());
    }

    #[test]
    fn automatic_advertisement_validates_identity_and_separates_ports() {
        let mismatch = MdnsDiscovery::new_with_port(id(10), 5454)
            .expect("configuration")
            .with_advertisement(record(11));
        assert!(matches!(mismatch, Err(DiscoveryError::UnexpectedDevice)));

        let discovery = MdnsDiscovery::new_with_port(id(10), 5454)
            .expect("configuration")
            .with_advertisement(record(10))
            .expect("matching advertisement");
        assert_eq!(discovery.daemon_port(), 5454);
        assert_eq!(discovery.advertisement().expect("advertisement").port, 9876);
    }

    #[test]
    fn service_instance_label_is_dns_safe_and_stable() {
        let device = id(7);
        let label = service_instance_label(&device);
        assert_eq!(label, device.to_hex()[..32]);
        assert!(label.len() <= 63);
        assert!(label.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(label, service_instance_label(&device));
    }

    #[test]
    fn peer_address_filter_rejects_non_dialable_addresses() {
        let keep = |address: &str| {
            is_dialable_peer_address(&address.parse::<std::net::IpAddr>().expect("ip"))
        };
        assert!(keep("10.0.0.88"));
        assert!(keep("192.168.1.7"));
        assert!(keep("8.8.8.8"));
        assert!(!keep("127.0.0.1"));
        assert!(!keep("0.0.0.0"));
        assert!(!keep("169.254.3.1"));
        assert!(!keep("224.0.0.251"));
        assert!(!keep("255.255.255.255"));
        // IPv6 is deferred until endpoints carry a zone/scope id; link-local
        // and global IPv6 must not leak into dial targets today.
        assert!(!keep("::1"));
        assert!(!keep("fe80::1"));
        assert!(!keep("2001:db8::1"));
    }

    #[test]
    fn manual_registry_validates_and_deduplicates() {
        let mut registry = ManualEndpointRegistry::for_device(id(9));
        assert!(registry.add(id(9), "Host.Example.", 42).expect("add"));
        assert!(!registry.add(id(9), "host.example", 42).expect("duplicate"));
        assert!(registry.add(id(9), "127.0.0.1", 42).expect("v4"));
        assert!(registry.add(id(9), "::1", 42).expect("v6"));
        assert_eq!(registry.len(), 3);
        assert!(matches!(
            registry.add(id(8), "other.example", 42),
            Err(DiscoveryError::UnexpectedDevice)
        ));
        assert!(matches!(
            registry.add(id(9), "bad host", 42),
            Err(DiscoveryError::InvalidEndpoint(_))
        ));
        assert!(registry.remove(id(9), "host.example", 42).expect("remove"));
        assert_eq!(registry.len(), 2);
    }
}
