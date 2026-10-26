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
    collections::VecDeque,
    io,
    net::SocketAddr,
    pin::Pin,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll},
    time::Instant,
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

/// Quinn's Tokio UDP socket with the same semantics, plus error records.
#[derive(Debug)]
pub(crate) struct InputUdpSocket {
    io: tokio::net::UdpSocket,
    inner: UdpSocketState,
    local: SocketAddr,
    created: Instant,
    send_errors: ErrorSummary,
    recv_errors: ErrorSummary,
}

impl InputUdpSocket {
    pub(crate) fn new(socket: std::net::UdpSocket) -> io::Result<Self> {
        let local = socket.local_addr()?;
        Ok(Self {
            inner: UdpSocketState::new((&socket).into())?,
            io: tokio::net::UdpSocket::from_std(socket)?,
            local,
            created: Instant::now(),
            send_errors: ErrorSummary::new("send_error"),
            recv_errors: ErrorSummary::new("recv_error"),
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
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Err(error),
            // Expected for MTU probes; quinn-udp ignores it as well.
            Err(error) if error.raw_os_error() == Some(EMSGSIZE) => Ok(()),
            Err(error) => {
                // quinn-udp drops other send errors so one bad path cannot kill
                // the connection; keep that behaviour and only record it.
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
