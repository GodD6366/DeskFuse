//! System Bonjour backend. All DNSServiceRef values and callback contexts live
//! on one worker thread. Only owned messages cross the FFI boundary.
#![allow(unsafe_code)]

use super::*;
use std::ffi::{CStr, CString, c_char, c_void};
use std::net::Ipv4Addr;
use std::ptr;
use std::time::Instant;

type Ref = *mut c_void;
const INCLUDE_P2P: u32 = 0x20000;
const ADD: u32 = 2;

// Signatures and flag values follow the macOS SDK's dns_sd.h. Callbacks run
// synchronously inside DNSServiceProcessResult, never on another thread.
#[link(name = "System")]
unsafe extern "C" {
    fn DNSServiceBrowse(
        r: *mut Ref,
        flags: u32,
        interface: u32,
        kind: *const c_char,
        domain: *const c_char,
        cb: BrowseCallback,
        context: *mut c_void,
    ) -> i32;
    fn DNSServiceResolve(
        r: *mut Ref,
        flags: u32,
        interface: u32,
        name: *const c_char,
        kind: *const c_char,
        domain: *const c_char,
        cb: ResolveCallback,
        context: *mut c_void,
    ) -> i32;
    fn DNSServiceQueryRecord(
        r: *mut Ref,
        flags: u32,
        interface: u32,
        name: *const c_char,
        rrtype: u16,
        class: u16,
        cb: QueryCallback,
        context: *mut c_void,
    ) -> i32;
    fn DNSServiceRegister(
        r: *mut Ref,
        flags: u32,
        interface: u32,
        name: *const c_char,
        kind: *const c_char,
        domain: *const c_char,
        host: *const c_char,
        port: u16,
        len: u16,
        txt: *const c_void,
        cb: RegisterCallback,
        context: *mut c_void,
    ) -> i32;
    fn DNSServiceRefSockFD(r: Ref) -> i32;
    fn DNSServiceProcessResult(r: Ref) -> i32;
    fn DNSServiceRefDeallocate(r: Ref);
}
type BrowseCallback =
    extern "C" fn(Ref, u32, u32, i32, *const c_char, *const c_char, *const c_char, *mut c_void);
type ResolveCallback = extern "C" fn(
    Ref,
    u32,
    u32,
    i32,
    *const c_char,
    *const c_char,
    u16,
    u16,
    *const u8,
    *mut c_void,
);
type QueryCallback = extern "C" fn(
    Ref,
    u32,
    u32,
    i32,
    *const c_char,
    u16,
    u16,
    u16,
    *const c_void,
    u32,
    *mut c_void,
);
type RegisterCallback =
    extern "C" fn(Ref, u32, i32, *const c_char, *const c_char, *const c_char, *mut c_void);

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct Key {
    interface: u32,
    name: String,
    kind: String,
    domain: String,
}

enum NativeEvent {
    Browse {
        add: bool,
        key: Key,
    },
    Resolved {
        host: String,
        port: u16,
        txt: Vec<u8>,
    },
    Address {
        add: bool,
        address: IpAddr,
    },
    Error(i32),
}
struct Context {
    token: u64,
    tx: Sender<(u64, NativeEvent)>,
}

// SAFETY: Only callbacks receive this pointer; Operation owns its Box until
// after deallocation and all callbacks finish on the same worker thread.
fn deliver(context: *mut c_void, event: NativeEvent) {
    if !context.is_null() {
        let context = unsafe { &*context.cast::<Context>() };
        let _ = context.tx.send((context.token, event));
    }
}
// SAFETY: Used only on successful DNS-SD callbacks, whose non-null string
// arguments are NUL-terminated and valid for the callback duration.
fn string(value: *const c_char) -> String {
    if value.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(value) }
        .to_string_lossy()
        .into_owned()
}
extern "C" fn browse_cb(
    _: Ref,
    flags: u32,
    interface: u32,
    error: i32,
    name: *const c_char,
    kind: *const c_char,
    domain: *const c_char,
    ctx: *mut c_void,
) {
    if error != 0 {
        deliver(ctx, NativeEvent::Error(error));
        return;
    }
    deliver(
        ctx,
        NativeEvent::Browse {
            add: flags & ADD != 0,
            key: Key {
                interface,
                name: string(name),
                kind: string(kind),
                domain: string(domain),
            },
        },
    );
}
extern "C" fn resolve_cb(
    _: Ref,
    _: u32,
    _: u32,
    error: i32,
    _: *const c_char,
    host: *const c_char,
    port: u16,
    len: u16,
    txt: *const u8,
    ctx: *mut c_void,
) {
    if error != 0 {
        deliver(ctx, NativeEvent::Error(error));
        return;
    }
    let txt = if len == 0 || txt.is_null() {
        Vec::new()
    } else {
        // SAFETY: DNS-SD supplies len initialized bytes valid during callback.
        unsafe { std::slice::from_raw_parts(txt, usize::from(len)) }.to_vec()
    };
    deliver(
        ctx,
        NativeEvent::Resolved {
            host: string(host),
            port: u16::from_be(port),
            txt,
        },
    );
}
extern "C" fn query_cb(
    _: Ref,
    flags: u32,
    _: u32,
    error: i32,
    _: *const c_char,
    kind: u16,
    class: u16,
    len: u16,
    data: *const c_void,
    _: u32,
    ctx: *mut c_void,
) {
    if error != 0 {
        deliver(ctx, NativeEvent::Error(error));
        return;
    }
    if kind == 1 && class == 1 && len == 4 && !data.is_null() {
        // SAFETY: An A record contains exactly four network-order bytes.
        let bytes = unsafe { std::slice::from_raw_parts(data.cast::<u8>(), 4) };
        deliver(
            ctx,
            NativeEvent::Address {
                add: flags & ADD != 0,
                address: IpAddr::V4(Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3])),
            },
        );
    }
}
extern "C" fn register_cb(
    _: Ref,
    _: u32,
    error: i32,
    _: *const c_char,
    _: *const c_char,
    _: *const c_char,
    ctx: *mut c_void,
) {
    if error != 0 {
        deliver(ctx, NativeEvent::Error(error));
    }
}

struct Operation {
    raw: Ref,
    _context: Box<Context>,
}
impl Operation {
    fn create(
        token: u64,
        tx: &Sender<(u64, NativeEvent)>,
        start: impl FnOnce(*mut Ref, *mut c_void) -> i32,
    ) -> Result<Self, DiscoveryError> {
        let mut context = Box::new(Context {
            token,
            tx: tx.clone(),
        });
        let mut raw = ptr::null_mut();
        let code = start(&mut raw, (&mut *context as *mut Context).cast());
        if code != 0 {
            return Err(native_error(code));
        }
        Ok(Self {
            raw,
            _context: context,
        })
    }
    fn browse(
        token: u64,
        tx: &Sender<(u64, NativeEvent)>,
        service_type: &'static std::ffi::CStr,
    ) -> Result<Self, DiscoveryError> {
        // SAFETY: Valid output slot, static C strings, and a stable owned context.
        Self::create(token, tx, |r, c| unsafe {
            DNSServiceBrowse(
                r,
                INCLUDE_P2P,
                0,
                service_type.as_ptr(),
                c"local.".as_ptr(),
                browse_cb,
                c,
            )
        })
    }
    fn resolve(
        token: u64,
        tx: &Sender<(u64, NativeEvent)>,
        key: &Key,
    ) -> Result<Self, DiscoveryError> {
        let name = cstring(&key.name)?;
        let kind = cstring(&key.kind)?;
        let domain = cstring(&key.domain)?;
        // SAFETY: DNS-SD copies these strings before returning; context outlives ref.
        Self::create(token, tx, |r, c| unsafe {
            DNSServiceResolve(
                r,
                INCLUDE_P2P,
                key.interface,
                name.as_ptr(),
                kind.as_ptr(),
                domain.as_ptr(),
                resolve_cb,
                c,
            )
        })
    }
    fn query(
        token: u64,
        tx: &Sender<(u64, NativeEvent)>,
        interface: u32,
        host: &str,
    ) -> Result<Self, DiscoveryError> {
        let host = cstring(host)?;
        // SAFETY: Query A/IN on the interface that supplied this service.
        Self::create(token, tx, |r, c| unsafe {
            DNSServiceQueryRecord(r, INCLUDE_P2P, interface, host.as_ptr(), 1, 1, query_cb, c)
        })
    }
    fn register(
        token: u64,
        tx: &Sender<(u64, NativeEvent)>,
        record: &ServiceRecord,
        service_type: &'static std::ffi::CStr,
    ) -> Result<Self, DiscoveryError> {
        let name = cstring(&service_instance_label(&record.id))?;
        let txt: Vec<u8> = record.encode_txt_wire()?.into_iter().flatten().collect();
        let len = u16::try_from(txt.len()).map_err(|_| DiscoveryError::InvalidTxtField("wire"))?;
        // SAFETY: DNS-SD copies input buffers. Null host selects the system's
        // actual local hostname and addresses, including interface changes.
        Self::create(token, tx, |r, c| unsafe {
            DNSServiceRegister(
                r,
                INCLUDE_P2P,
                0,
                name.as_ptr(),
                service_type.as_ptr(),
                c"local.".as_ptr(),
                ptr::null(),
                record.port.to_be(),
                len,
                txt.as_ptr().cast(),
                register_cb,
                c,
            )
        })
    }
}
impl Drop for Operation {
    fn drop(&mut self) {
        // SAFETY: Unique live reference, deallocated on its creating thread.
        // The callback context is dropped only after this call returns.
        unsafe { DNSServiceRefDeallocate(self.raw) };
    }
}
fn native_error(code: i32) -> DiscoveryError {
    DiscoveryError::Mdns(format!("system Bonjour error {code}"))
}
fn cstring(value: &str) -> Result<CString, DiscoveryError> {
    CString::new(value).map_err(|_| DiscoveryError::InvalidEndpoint("NUL in DNS name".into()))
}
fn decode_record(txt: &[u8], port: u16) -> Result<ServiceRecord, DiscoveryError> {
    let mut chunks = Vec::new();
    let mut remaining = txt;
    while !remaining.is_empty() {
        let len = usize::from(remaining[0]) + 1;
        if len > remaining.len() {
            return Err(DiscoveryError::InvalidTxtField("wire"));
        }
        chunks.push(&remaining[..len]);
        remaining = &remaining[len..];
    }
    let record = ServiceRecord::decode_txt_wire(chunks)?;
    if record.port != port {
        return Err(DiscoveryError::InvalidPort);
    }
    Ok(record)
}

/// macOS system Bonjour discovery with IncludeP2P. Discovery remains an
/// untrusted hint; only IPv4 endpoints supported by the QUIC transport escape.
#[derive(Debug)]
pub struct BonjourDiscovery {
    local_id: DeviceId,
    service_type: &'static std::ffi::CStr,
    advertisement: Option<ServiceRecord>,
    handle: DiscoveryHandle,
    sender: Option<Sender<DiscoveryEvent>>,
    commands: Option<Sender<Command>>,
    worker: Option<JoinHandle<()>>,
    started: bool,
    stopped: bool,
}
#[derive(Debug)]
enum Command {
    Refresh(Sender<Result<(), DiscoveryError>>),
    Stop,
}
impl BonjourDiscovery {
    pub fn new(local_id: DeviceId) -> Result<Self, DiscoveryError> {
        let (sender, receiver) = mpsc::channel();
        Ok(Self {
            local_id,
            service_type: c"_unidesk._udp",
            advertisement: None,
            handle: DiscoveryHandle::new(receiver),
            sender: Some(sender),
            commands: None,
            worker: None,
            started: false,
            stopped: false,
        })
    }
    /// A separate browse/advertisement namespace for the input helper.
    pub fn for_input_service(local_id: DeviceId) -> Result<Self, DiscoveryError> {
        let mut provider = Self::new(local_id)?;
        provider.service_type = c"_unidesk-input._udp";
        Ok(provider)
    }
    pub fn set_advertisement(&mut self, record: ServiceRecord) -> Result<(), DiscoveryError> {
        if self.started || self.stopped {
            return Err(DiscoveryError::ProviderUnavailable(
                "provider is unavailable",
            ));
        }
        record.validate()?;
        record.encode_txt_wire()?;
        if record.id != self.local_id {
            return Err(DiscoveryError::UnexpectedDevice);
        }
        self.advertisement = Some(record);
        Ok(())
    }
    pub fn with_advertisement(mut self, record: ServiceRecord) -> Result<Self, DiscoveryError> {
        self.set_advertisement(record)?;
        Ok(self)
    }
}
impl DiscoveryProvider for BonjourDiscovery {
    fn start(&mut self) -> Result<DiscoveryHandle, DiscoveryError> {
        if self.started || self.stopped {
            return Err(DiscoveryError::ProviderUnavailable(
                "provider is unavailable",
            ));
        }
        let sender = self
            .sender
            .as_ref()
            .ok_or(DiscoveryError::ChannelClosed)?
            .clone();
        let record = self.advertisement.clone();
        let local_id = self.local_id;
        let service_type = self.service_type;
        let (commands, rx) = mpsc::channel();
        let (ready, result) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("unidesk-bonjour".into())
            .spawn(
                move || match Worker::with_service(local_id, record, sender, service_type) {
                    Ok(mut worker) => {
                        let _ = ready.send(Ok(()));
                        worker.run(rx);
                    }
                    Err(error) => {
                        let _ = ready.send(Err(error));
                    }
                },
            )
            .map_err(|e| DiscoveryError::Mdns(e.to_string()))?;
        match result
            .recv()
            .map_err(|_| DiscoveryError::ChannelClosed)
            .and_then(|r| r)
        {
            Ok(()) => {
                self.commands = Some(commands);
                self.worker = Some(worker);
                self.started = true;
                Ok(self.handle.clone())
            }
            Err(error) => {
                let _ = worker.join();
                Err(error)
            }
        }
    }
    fn refresh(&mut self) -> Result<(), DiscoveryError> {
        let (tx, rx) = mpsc::channel();
        self.commands
            .as_ref()
            .ok_or(DiscoveryError::ProviderUnavailable(
                "provider is not running",
            ))?
            .send(Command::Refresh(tx))
            .map_err(|_| DiscoveryError::ChannelClosed)?;
        rx.recv().map_err(|_| DiscoveryError::ChannelClosed)?
    }
    fn shutdown(&mut self) -> Result<(), DiscoveryError> {
        self.stopped = true;
        if let Some(commands) = self.commands.take() {
            let _ = commands.send(Command::Stop);
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        self.started = false;
        self.sender.take();
        Ok(())
    }
}
impl Drop for BonjourDiscovery {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

struct Service {
    resolve: u64,
    query: Option<u64>,
    record: Option<ServiceRecord>,
    host: String,
    addresses: BTreeSet<IpAddr>,
}
struct Worker {
    service_type: &'static std::ffi::CStr,
    local_id: DeviceId,
    sender: Sender<DiscoveryEvent>,
    tx: Sender<(u64, NativeEvent)>,
    rx: Receiver<(u64, NativeEvent)>,
    operations: BTreeMap<u64, Operation>,
    services: BTreeMap<Key, Service>,
    known: BTreeMap<DeviceId, DiscoveredService>,
    browse: u64,
    next: u64,
    advertisement: Option<ServiceRecord>,
    retry_at: Instant,
    unseen: BTreeSet<Key>,
    refresh_deadline: Option<Instant>,
}
impl Worker {
    fn with_service(
        local_id: DeviceId,
        record: Option<ServiceRecord>,
        sender: Sender<DiscoveryEvent>,
        service_type: &'static std::ffi::CStr,
    ) -> Result<Self, DiscoveryError> {
        let (tx, rx) = mpsc::channel();
        let mut operations = BTreeMap::new();
        operations.insert(1, Operation::browse(1, &tx, service_type)?);
        if let Some(record) = &record {
            operations.insert(2, Operation::register(2, &tx, record, service_type)?);
        }
        Ok(Self {
            service_type,
            local_id,
            sender,
            tx,
            rx,
            operations,
            services: BTreeMap::new(),
            known: BTreeMap::new(),
            browse: 1,
            next: 3,
            advertisement: record,
            retry_at: Instant::now() + Duration::from_secs(30),
            unseen: BTreeSet::new(),
            refresh_deadline: None,
        })
    }
    fn token(&mut self) -> u64 {
        let token = self.next;
        self.next += 1;
        token
    }
    fn run(&mut self, commands: Receiver<Command>) {
        loop {
            loop {
                match commands.try_recv() {
                    Ok(Command::Stop) | Err(TryRecvError::Disconnected) => return,
                    Err(TryRecvError::Empty) => break,
                    Ok(Command::Refresh(reply)) => {
                        let token = self.token();
                        let result =
                            Operation::browse(token, &self.tx, self.service_type).map(|browse| {
                                self.operations.remove(&self.browse);
                                self.operations.insert(token, browse);
                                self.browse = token;
                                self.begin_refresh();
                                for service in self.known.values() {
                                    let _ =
                                        self.sender.send(DiscoveryEvent::Found(service.clone()));
                                }
                            });
                        let _ = reply.send(result);
                    }
                }
            }
            if Instant::now() >= self.retry_at {
                self.retry_at = Instant::now() + Duration::from_secs(30);
                let token = self.token();
                match Operation::browse(token, &self.tx, self.service_type) {
                    Ok(op) => {
                        self.operations.remove(&self.browse);
                        self.operations.insert(token, op);
                        self.browse = token;
                        self.begin_refresh();
                    }
                    Err(error) => {
                        let _ = self.sender.send(DiscoveryEvent::Error(error));
                        self.retry_at = Instant::now() + Duration::from_secs(2);
                    }
                }
                if !self.operations.contains_key(&2)
                    && let Some(record) = &self.advertisement
                {
                    match Operation::register(2, &self.tx, record, self.service_type) {
                        Ok(op) => {
                            self.operations.insert(2, op);
                        }
                        Err(error) => {
                            let _ = self.sender.send(DiscoveryEvent::Error(error));
                            self.retry_at = Instant::now() + Duration::from_secs(2);
                        }
                    }
                }
            }
            let mut fds = self
                .operations
                .iter()
                .map(|(token, op)| {
                    // SAFETY: Operation owns a live DNSServiceRef on this thread.
                    (
                        *token,
                        libc::pollfd {
                            fd: unsafe { DNSServiceRefSockFD(op.raw) },
                            events: libc::POLLIN,
                            revents: 0,
                        },
                    )
                })
                .collect::<Vec<_>>();
            let mut pollfds = fds.iter().map(|(_, fd)| *fd).collect::<Vec<_>>();
            // SAFETY: Valid writable array, length matches allocation. Bounded
            // polling makes shutdown/refresh independent of network activity.
            let result =
                unsafe { libc::poll(pollfds.as_mut_ptr(), pollfds.len() as libc::nfds_t, 100) };
            if result < 0 {
                if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                let _ = self.sender.send(DiscoveryEvent::Error(DiscoveryError::Mdns(
                    "Bonjour poll failed".into(),
                )));
                return;
            }
            for ((token, _), fd) in fds.drain(..).zip(pollfds) {
                if fd.revents == 0 {
                    continue;
                }
                let error = if fd.revents & libc::POLLIN != 0 {
                    // SAFETY: The fd was readable and the ref/context stay
                    // alive until all synchronous callbacks have returned.
                    unsafe { DNSServiceProcessResult(self.operations[&token].raw) }
                } else {
                    -65563
                };
                if error != 0 {
                    let _ = self.tx.send((token, NativeEvent::Error(error)));
                }
            }
            while let Ok((token, event)) = self.rx.try_recv() {
                if !self.operations.contains_key(&token) {
                    continue;
                }
                if let Err(error) = self.event(token, event) {
                    let _ = self.sender.send(DiscoveryEvent::Error(error));
                }
            }
            self.expire_unseen();
            self.publish();
        }
    }
    fn begin_refresh(&mut self) {
        self.unseen = self.services.keys().cloned().collect();
        self.refresh_deadline = Some(Instant::now() + Duration::from_secs(5));
    }
    fn expire_unseen(&mut self) {
        if self
            .refresh_deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            for key in std::mem::take(&mut self.unseen) {
                self.remove(&key);
            }
            self.refresh_deadline = None;
        }
    }
    fn event(&mut self, token: u64, event: NativeEvent) -> Result<(), DiscoveryError> {
        match event {
            NativeEvent::Error(code) => {
                self.operations.remove(&token);
                self.retry_at = Instant::now() + Duration::from_secs(2);
                // Stop a failed service's address watcher as well. Explicit
                // refresh can resolve it again; stale addresses never survive.
                let keys = self
                    .services
                    .iter()
                    .filter(|(_, s)| s.resolve == token || s.query == Some(token))
                    .map(|(k, _)| k.clone())
                    .collect::<Vec<_>>();
                for key in keys {
                    self.remove(&key);
                }
                return Err(native_error(code));
            }
            NativeEvent::Browse { add, key } => {
                if token != self.browse {
                    return Ok(());
                }
                self.unseen.remove(&key);
                if !add {
                    self.remove(&key);
                } else if !self.services.contains_key(&key) && self.services.len() < 512 {
                    let token = self.token();
                    let op = Operation::resolve(token, &self.tx, &key)?;
                    self.operations.insert(token, op);
                    self.services.insert(
                        key,
                        Service {
                            resolve: token,
                            query: None,
                            record: None,
                            host: String::new(),
                            addresses: BTreeSet::new(),
                        },
                    );
                }
            }
            NativeEvent::Resolved { host, port, txt } => {
                let Some(key) = self
                    .services
                    .iter()
                    .find(|(_, s)| s.resolve == token)
                    .map(|(key, _)| key.clone())
                else {
                    return Ok(());
                };
                let record = match decode_record(&txt, port) {
                    Ok(record) => record,
                    Err(error) => {
                        self.remove(&key);
                        return Err(error);
                    }
                };
                if record.id == self.local_id {
                    self.remove(&key);
                    return Ok(());
                }
                if self.services[&key].host != host || self.services[&key].query.is_none() {
                    let query = self.token();
                    let op = match Operation::query(query, &self.tx, key.interface, &host) {
                        Ok(op) => op,
                        Err(error) => {
                            self.remove(&key);
                            self.retry_at = Instant::now() + Duration::from_secs(2);
                            return Err(error);
                        }
                    };
                    self.operations.insert(query, op);
                    let service = self.services.get_mut(&key).expect("existing service");
                    if let Some(old) = service.query.replace(query) {
                        self.operations.remove(&old);
                    }
                    service.addresses.clear();
                    service.host = host;
                }
                self.services
                    .get_mut(&key)
                    .expect("existing service")
                    .record = Some(record);
            }
            NativeEvent::Address { add, address } => {
                if let Some(service) = self.services.values_mut().find(|s| s.query == Some(token)) {
                    if add && is_dialable_peer_address(&address) {
                        service.addresses.insert(address);
                    } else {
                        service.addresses.remove(&address);
                    }
                }
            }
        }
        Ok(())
    }
    fn remove(&mut self, key: &Key) {
        if let Some(service) = self.services.remove(key) {
            self.operations.remove(&service.resolve);
            if let Some(query) = service.query {
                self.operations.remove(&query);
            }
        }
    }
    fn publish(&mut self) {
        let mut current: BTreeMap<DeviceId, DiscoveredService> = BTreeMap::new();
        for service in self.services.values() {
            let Some(record) = &service.record else {
                continue;
            };
            if service.addresses.is_empty() {
                continue;
            }
            let Ok(discovered) = DiscoveredService::from_ips(
                record.clone(),
                &service.host,
                service.addresses.iter().copied(),
            ) else {
                continue;
            };
            current
                .entry(record.id)
                .and_modify(|existing| {
                    for endpoint in &discovered.endpoints {
                        if !existing.endpoints.contains(endpoint) {
                            existing.endpoints.push(endpoint.clone());
                        }
                    }
                })
                .or_insert(discovered);
        }
        for (id, service) in &current {
            let event = match self.known.get(id) {
                Some(previous) if previous == service => continue,
                Some(_) => DiscoveryEvent::Updated(service.clone()),
                None => DiscoveryEvent::Found(service.clone()),
            };
            let _ = self.sender.send(event);
        }
        for id in self.known.keys().filter(|id| !current.contains_key(id)) {
            let _ = self.sender.send(DiscoveryEvent::Removed(*id));
        }
        self.known = current;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: u8) -> ServiceRecord {
        ServiceRecord::v1(
            DeviceId::from_bytes([id; 32]),
            "Bonjour test",
            Capabilities::NETWORK,
            49152,
        )
        .unwrap()
    }
    fn worker() -> (Worker, Receiver<DiscoveryEvent>) {
        let (sender, receiver) = mpsc::channel();
        let (tx, rx) = mpsc::channel();
        (
            Worker {
                service_type: c"_unidesk._udp",
                local_id: record(1).id,
                sender,
                tx,
                rx,
                operations: BTreeMap::new(),
                services: BTreeMap::new(),
                known: BTreeMap::new(),
                browse: 1,
                next: 3,
                advertisement: None,
                retry_at: Instant::now(),
                unseen: BTreeSet::new(),
                refresh_deadline: None,
            },
            receiver,
        )
    }
    fn service(interface: u32, address: &str) -> (Key, Service) {
        (
            Key {
                interface,
                name: "peer".into(),
                kind: "_unidesk._udp".into(),
                domain: "local.".into(),
            },
            Service {
                resolve: 3,
                query: Some(4),
                record: Some(record(2)),
                host: "peer.local.".into(),
                addresses: BTreeSet::from([address.parse().unwrap()]),
            },
        )
    }
    #[test]
    fn txt_requires_complete_wire_and_matching_srv_port() {
        let record = record(2);
        let mut bytes: Vec<u8> = record
            .encode_txt_wire()
            .unwrap()
            .into_iter()
            .flatten()
            .collect();
        assert_eq!(decode_record(&bytes, record.port).unwrap(), record);
        assert_eq!(decode_record(&bytes, 42), Err(DiscoveryError::InvalidPort));
        bytes.pop();
        assert!(decode_record(&bytes, record.port).is_err());
    }
    #[test]
    fn subnet_prefix_does_not_filter_routes_and_interfaces_are_merged() {
        let (mut worker, receiver) = worker();
        let (first, service1) = service(4, "172.18.236.18");
        let (second, service2) = service(9, "172.18.238.215");
        worker.services.insert(first.clone(), service1);
        worker.services.insert(second.clone(), service2);
        worker.publish();
        let DiscoveryEvent::Found(found) = receiver.try_recv().unwrap() else {
            panic!("found");
        };
        assert_eq!(found.endpoints.len(), 2);
        worker.publish();
        assert!(receiver.try_recv().is_err());
        worker.remove(&first);
        worker.publish();
        let DiscoveryEvent::Updated(found) = receiver.try_recv().unwrap() else {
            panic!("updated");
        };
        assert_eq!(found.endpoints.len(), 1);
        worker.remove(&second);
        worker.publish();
        assert!(matches!(
            receiver.try_recv().unwrap(),
            DiscoveryEvent::Removed(_)
        ));
    }
    #[test]
    fn address_removal_and_undialable_addresses_never_leave_stale_routes() {
        let (mut worker, receiver) = worker();
        let (key, mut service) = service(4, "172.18.236.18");
        service.addresses.clear();
        worker.services.insert(key, service);
        worker
            .event(
                4,
                NativeEvent::Address {
                    add: true,
                    address: "::1".parse().unwrap(),
                },
            )
            .unwrap();
        worker.publish();
        assert!(receiver.try_recv().is_err());
        let address = "172.18.238.215".parse().unwrap();
        worker
            .event(4, NativeEvent::Address { add: true, address })
            .unwrap();
        worker.publish();
        assert!(matches!(
            receiver.try_recv().unwrap(),
            DiscoveryEvent::Found(_)
        ));
        worker
            .event(
                4,
                NativeEvent::Address {
                    add: false,
                    address,
                },
            )
            .unwrap();
        worker.publish();
        assert!(matches!(
            receiver.try_recv().unwrap(),
            DiscoveryEvent::Removed(_)
        ));
    }
    #[test]
    fn refresh_expires_services_missing_from_the_new_browser() {
        let (mut worker, receiver) = worker();
        let (key, service) = service(4, "172.18.238.215");
        worker.services.insert(key, service);
        worker.publish();
        assert!(matches!(
            receiver.try_recv().unwrap(),
            DiscoveryEvent::Found(_)
        ));
        worker.begin_refresh();
        worker.refresh_deadline = Some(Instant::now());
        worker.expire_unseen();
        worker.publish();
        assert!(worker.services.is_empty());
        assert!(matches!(
            receiver.try_recv().unwrap(),
            DiscoveryEvent::Removed(_)
        ));
    }

    #[test]
    fn self_and_invalid_txt_remove_pending_resolvers() {
        let (mut worker, _) = worker();
        let (key, pending) = service(4, "172.18.238.215");
        worker.services.insert(key.clone(), pending);
        let local = record(1);
        worker
            .event(
                3,
                NativeEvent::Resolved {
                    host: "self.local.".into(),
                    port: local.port,
                    txt: local
                        .encode_txt_wire()
                        .unwrap()
                        .into_iter()
                        .flatten()
                        .collect(),
                },
            )
            .unwrap();
        assert!(worker.services.is_empty());
        let (_, service) = service(4, "172.18.238.215");
        worker.services.insert(key, service);
        assert!(
            worker
                .event(
                    3,
                    NativeEvent::Resolved {
                        host: "bad.local.".into(),
                        port: 49152,
                        txt: vec![255],
                    }
                )
                .is_err()
        );
        assert!(worker.services.is_empty());
    }

    #[test]
    fn synchronous_address_query_failure_releases_service_for_retry() {
        let (mut worker, _) = worker();
        let (key, service) = service(4, "172.18.238.215");
        worker.services.insert(key, service);
        let remote = record(2);
        // An invalid C hostname fails before invoking DNS-SD, exercising the
        // same query-start error cleanup without requiring a live daemon.
        assert!(
            worker
                .event(
                    3,
                    NativeEvent::Resolved {
                        host: "bad\0.local.".into(),
                        port: remote.port,
                        txt: remote
                            .encode_txt_wire()
                            .unwrap()
                            .into_iter()
                            .flatten()
                            .collect(),
                    }
                )
                .is_err()
        );
        assert!(worker.services.is_empty());
        assert!(worker.retry_at <= Instant::now() + Duration::from_secs(2));
    }

    #[test]
    fn construction_and_shutdown_do_not_require_bonjour() {
        let mut provider = BonjourDiscovery::new(record(1).id).unwrap();
        assert!(provider.worker.is_none());
        let handle = provider.handle.clone();
        provider.shutdown().unwrap();
        provider.shutdown().unwrap();
        assert!(provider.start().is_err());
        assert_eq!(handle.try_recv(), Err(DiscoveryError::ChannelClosed));
    }
    #[test]
    #[ignore = "requires live macOS Bonjour and a routable IPv4 interface"]
    fn system_bonjour_publish_resolve_refresh_and_shutdown() {
        let local = record(246);
        let remote = record(247);
        let mut browser = BonjourDiscovery::new(local.id).unwrap();
        let events = browser.start().unwrap();
        assert!(browser.start().is_err());
        let mut publisher = BonjourDiscovery::new(remote.id)
            .unwrap()
            .with_advertisement(remote.clone())
            .unwrap();
        publisher.start().unwrap();
        fn wait(events: &DiscoveryHandle, id: DeviceId, removed: bool) {
            let deadline = Instant::now() + Duration::from_secs(15);
            while Instant::now() < deadline {
                if let Some(event) = events.recv_timeout(Duration::from_millis(200)).unwrap() {
                    if event.device_id() == Some(id)
                        && matches!(event, DiscoveryEvent::Removed(_)) == removed
                    {
                        return;
                    }
                    if let DiscoveryEvent::Error(error) = event {
                        panic!("{error}");
                    }
                }
            }
            panic!("Bonjour event timed out, removed={removed}");
        }
        wait(&events, remote.id, false);
        browser.refresh().unwrap();
        wait(&events, remote.id, false);
        publisher.shutdown().unwrap();
        wait(&events, remote.id, true);
        browser.shutdown().unwrap();
        while events.try_recv().is_ok_and(|e| e.is_some()) {}
        assert_eq!(events.try_recv(), Err(DiscoveryError::ChannelClosed));
    }
}
