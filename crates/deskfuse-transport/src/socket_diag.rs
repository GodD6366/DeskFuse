//! Process-global lifecycle and error records for the control (42424) and
//! input-path UDP sockets.
//!
//! Answers, from an exported diagnostics file, why a 42425 probe keeps timing
//! out until the process restarts: which listener/dialer sockets exist, how old
//! a reused dialer is, whether an accept loop ended, and which OS errors the
//! sockets hit. Quinn's own UDP layer only logs send errors and silently retries
//! receive errors, so without these records the cause is invisible.
//! Metadata only: addresses, interface names and errno values.
use std::{
    collections::{BTreeMap, VecDeque},
    io,
    net::{IpAddr, SocketAddr},
    pin::Pin,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};

use quinn::udp::{RecvMeta, Transmit, UdpSocketState};
use tokio::io::Interest;

const CAPACITY: usize = 512;
/// `EMSGSIZE`: expected for MTU probes and ignored like quinn-udp does.
#[cfg(target_os = "linux")]
const EMSGSIZE: i32 = 90;
#[cfg(windows)]
const EMSGSIZE: i32 = 10040;
#[cfg(not(any(target_os = "linux", windows)))]
const EMSGSIZE: i32 = 40;
/// The same failing socket is summarised at most once per interval.
const ERROR_INTERVAL_SECS: u64 = 10;

#[derive(Clone, Debug)]
pub struct SocketDiagRecord {
    pub timestamp_ms: u128,
    /// `listener_bound`, `listener_closed`, `listener_accept_ended`,
    /// `dialer_created`, `dialer_forgotten`, `send_error`, `recv_error`.
    pub event: &'static str,
    pub local_addr: String,
    pub detail: String,
}

static RECORDS: Mutex<VecDeque<SocketDiagRecord>> = Mutex::new(VecDeque::new());
static DROPPED_RECORDS: AtomicU64 = AtomicU64::new(0);

pub(crate) fn record(event: &'static str, local_addr: SocketAddr, detail: String) {
    let Ok(mut records) = RECORDS.lock() else {
        DROPPED_RECORDS.fetch_add(1, Ordering::Relaxed);
        return;
    };
    if records.len() == CAPACITY {
        records.pop_front();
        DROPPED_RECORDS.fetch_add(1, Ordering::Relaxed);
    }
    records.push_back(SocketDiagRecord {
        timestamp_ms: crate::motion_diag::timestamp_ms(),
        event,
        local_addr: local_addr.to_string(),
        detail,
    });
}

/// Copy every retained record, plus how many were evicted before export.
#[must_use]
pub fn snapshot() -> (Vec<SocketDiagRecord>, u64) {
    let records = RECORDS
        .lock()
        .map(|records| records.iter().cloned().collect())
        .unwrap_or_default();
    (records, DROPPED_RECORDS.load(Ordering::Relaxed))
}

/// Rate-limited error counter for one socket and one direction.
#[derive(Debug)]
struct ErrorSummary {
    event: &'static str,
    total: AtomicU64,
    pending: AtomicU64,
    /// Seconds since `created` of the last emitted record; `u64::MAX` = never.
    last_emitted: AtomicU64,
}

impl ErrorSummary {
    const fn new(event: &'static str) -> Self {
        Self {
            event,
            total: AtomicU64::new(0),
            pending: AtomicU64::new(0),
            last_emitted: AtomicU64::new(u64::MAX),
        }
    }

    fn observe(&self, created: Instant, local: SocketAddr, error: &io::Error) {
        let total = self.total.fetch_add(1, Ordering::Relaxed) + 1;
        let pending = self.pending.fetch_add(1, Ordering::Relaxed) + 1;
        let now = created.elapsed().as_secs();
        let last = self.last_emitted.load(Ordering::Relaxed);
        if last != u64::MAX && now.saturating_sub(last) < ERROR_INTERVAL_SECS {
            return;
        }
        if self
            .last_emitted
            .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
        {
            return;
        }
        self.pending.fetch_sub(pending, Ordering::Relaxed);
        record(
            self.event,
            local,
            format!(
                "errno={:?} kind={:?} error={error} count={pending} total={total} socket_age_secs={now}",
                error.raw_os_error(),
                error.kind(),
            ),
        );
    }
}

/// Whether a socket has ever handed a datagram to the kernel, and how often the
/// kernel refused one with `EHOSTUNREACH` before it left the host.
#[derive(Debug, Default)]
struct SendHealth {
    sent: AtomicU64,
    unreachable: AtomicU64,
}

static SEND_HEALTH: Mutex<BTreeMap<SocketAddr, Weak<SendHealth>>> = Mutex::new(BTreeMap::new());

/// The kernel refused every datagram this socket tried to send with
/// `EHOSTUNREACH`, synchronously and before anything reached the wire.
///
/// That is a local verdict, not a silent peer. A long-lived process can get it
/// for every *new* socket while its older sockets, and other processes, still
/// reach the same address (macOS local-network/NECP policy is evaluated per
/// process). The process only recovers once it is relaunched.
pub(crate) fn send_refused_locally(local: SocketAddr) -> bool {
    SEND_HEALTH
        .lock()
        .ok()
        .and_then(|sockets| sockets.get(&local).and_then(Weak::upgrade))
        .is_some_and(|health| {
            health.sent.load(Ordering::Relaxed) == 0
                && health.unreachable.load(Ordering::Relaxed) > 0
        })
}

/// Rounds in which every fresh dialer to an address was refused locally.
struct Refusal {
    rounds: u32,
    last: Instant,
}

static REFUSALS: Mutex<BTreeMap<IpAddr, Refusal>> = Mutex::new(BTreeMap::new());
/// A route flap clears within a round or two; three in a row is persistent.
const REFUSAL_ROUNDS: u32 = 3;
/// Older evidence no longer describes this process's current state.
const REFUSAL_WINDOW: Duration = Duration::from_secs(60);

/// Count one link attempt in which every dialer to `remote` was refused locally.
pub(crate) fn record_local_refusal(remote: IpAddr) {
    if let Ok(mut refusals) = REFUSALS.lock() {
        let now = Instant::now();
        let refusal = refusals.entry(remote).or_insert(Refusal {
            rounds: 0,
            last: now,
        });
        if now.duration_since(refusal.last) > REFUSAL_WINDOW {
            refusal.rounds = 0;
        }
        refusal.rounds += 1;
        refusal.last = now;
    }
}

/// A dialer reached `remote`, so this process can send there again.
pub(crate) fn clear_local_refusal(remote: IpAddr) {
    if let Ok(mut refusals) = REFUSALS.lock() {
        refusals.remove(&remote);
    }
}

/// Addresses this process has persistently been unable to send to.
///
/// Only the host process can tell whether that is the peer being offline or
/// this process being denied: it checks whether its own live control session
/// reaches the same address right now.
#[must_use]
pub fn locally_refused_hosts() -> Vec<IpAddr> {
    let Ok(mut refusals) = REFUSALS.lock() else {
        return Vec::new();
    };
    refusals.retain(|_, refusal| refusal.last.elapsed() <= REFUSAL_WINDOW);
    refusals
        .iter()
        .filter(|(_, refusal)| refusal.rounds >= REFUSAL_ROUNDS)
        .map(|(ip, _)| *ip)
        .collect()
}

/// Quinn's Tokio UDP socket with the same semantics, plus error records.
#[derive(Debug)]
pub(crate) struct InputUdpSocket {
    io: tokio::net::UdpSocket,
    inner: UdpSocketState,
    local: SocketAddr,
    created: Instant,
    send_errors: ErrorSummary,
    recv_errors: ErrorSummary,
    health: Arc<SendHealth>,
}

impl InputUdpSocket {
    pub(crate) fn new(socket: std::net::UdpSocket) -> io::Result<Self> {
        let local = socket.local_addr()?;
        let health = Arc::<SendHealth>::default();
        if let Ok(mut sockets) = SEND_HEALTH.lock() {
            sockets.retain(|_, health| health.strong_count() > 0);
            sockets.insert(local, Arc::downgrade(&health));
        }
        Ok(Self {
            inner: UdpSocketState::new((&socket).into())?,
            io: tokio::net::UdpSocket::from_std(socket)?,
            local,
            created: Instant::now(),
            send_errors: ErrorSummary::new("send_error"),
            recv_errors: ErrorSummary::new("recv_error"),
            health,
        })
    }
}

#[derive(Debug)]
struct WritablePoller(std::sync::Arc<InputUdpSocket>);

impl quinn::UdpPoller for WritablePoller {
    fn poll_writable(self: Pin<&mut Self>, cx: &mut Context) -> Poll<io::Result<()>> {
        self.0.io.poll_send_ready(cx)
    }
}

impl quinn::AsyncUdpSocket for InputUdpSocket {
    fn create_io_poller(self: std::sync::Arc<Self>) -> Pin<Box<dyn quinn::UdpPoller>> {
        Box::pin(WritablePoller(self))
    }

    fn try_send(&self, transmit: &Transmit) -> io::Result<()> {
        match self.io.try_io(Interest::WRITABLE, || {
            self.inner.try_send((&self.io).into(), transmit)
        }) {
            Ok(()) => {
                self.health.sent.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Err(error),
            // Expected for MTU probes; quinn-udp ignores it as well.
            Err(error) if error.raw_os_error() == Some(EMSGSIZE) => Ok(()),
            Err(error) => {
                // quinn-udp drops other send errors so one bad path cannot kill
                // the connection; keep that behaviour and only record it.
                if error.kind() == io::ErrorKind::HostUnreachable {
                    self.health.unreachable.fetch_add(1, Ordering::Relaxed);
                }
                self.send_errors.observe(self.created, self.local, &error);
                Ok(())
            }
        }
    }

    fn poll_recv(
        &self,
        cx: &mut Context,
        bufs: &mut [io::IoSliceMut<'_>],
        meta: &mut [RecvMeta],
    ) -> Poll<io::Result<usize>> {
        loop {
            std::task::ready!(self.io.poll_recv_ready(cx))?;
            match self.io.try_io(Interest::READABLE, || {
                self.inner.recv((&self.io).into(), bufs, meta)
            }) {
                Ok(count) => return Poll::Ready(Ok(count)),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => self.recv_errors.observe(self.created, self.local, &error),
            }
        }
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.io.local_addr()
    }

    fn may_fragment(&self) -> bool {
        self.inner.may_fragment()
    }

    fn max_transmit_segments(&self) -> usize {
        self.inner.max_gso_segments()
    }

    fn max_receive_segments(&self) -> usize {
        self.inner.gro_segments()
    }
}
