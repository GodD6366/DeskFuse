//! Process-wide executor reserved for latency-sensitive input and its QUIC I/O.
//! Clipboard/file workers use the application's ordinary executor. Keeping the
//! small input executor alive for the process also avoids dropping a runtime
//! from an async session/connection cleanup path.
use std::{
    future::Future,
    io,
    pin::Pin,
    sync::{Arc, OnceLock},
    time::Instant,
};
use tokio::runtime::{Handle, Runtime};

pub fn prioritize_current_thread() {
    #[cfg(target_os = "macos")]
    {
        use objc2_foundation::{NSQualityOfService, NSThread};
        NSThread::currentThread().setQualityOfService(NSQualityOfService::UserInteractive);
    }
}

pub fn handle() -> Result<Handle, io::Error> {
    static EXECUTOR: OnceLock<Result<Runtime, String>> = OnceLock::new();
    EXECUTOR
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("unidesk-input-io")
                .on_thread_start(prioritize_current_thread)
                .enable_all()
                .build()
                .map_err(|error| error.to_string())
        })
        .as_ref()
        .map(|runtime| runtime.handle().clone())
        .map_err(|error| io::Error::other(error.clone()))
}

/// Dedicated executor for the input coordinator itself.
///
/// The coordinator's turn rate is a hard ceiling on the delivered motion rate:
/// its per-session motion mailbox keeps only the newest position, so every turn
/// that waits behind QUIC packet processing silently discards positions instead
/// of merely adding latency. Sharing worker threads with the connection drivers
/// therefore turns a busy link (retransmits, congestion, pacing) into a visibly
/// lower cursor update rate. Give the coordinator its own thread; the connection
/// drivers keep `handle()`.
pub fn coordinator() -> Result<Handle, io::Error> {
    static EXECUTOR: OnceLock<Result<Runtime, String>> = OnceLock::new();
    EXECUTOR
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .thread_name("unidesk-input-actor")
                .on_thread_start(prioritize_current_thread)
                .enable_all()
                .build()
                .map_err(|error| error.to_string())
        })
        .as_ref()
        .map(|runtime| runtime.handle().clone())
        .map_err(|error| io::Error::other(error.clone()))
}

/// Dedicated executor for input-link *discovery* work: interface inspection,
/// address classification and outbound probe/dial handshakes.
///
/// A dead peer is retried forever, and every round used to run a full QUIC
/// handshake (up to the 3 s probe timeout per address) plus per-address
/// interface inspection on `handle()` - the same two workers that carry the
/// live session's datagram reader and reliable streams. A field capture showed
/// the resulting multi-hundred-millisecond holes in position delivery lining up
/// with reconnect churn for unrelated peers. Probing therefore gets its own
/// threads so a handshake to an unreachable address can never delay a live
/// session's packets. Sessions keep `handle()`.
pub fn probe() -> Result<Handle, io::Error> {
    static EXECUTOR: OnceLock<Result<Runtime, String>> = OnceLock::new();
    EXECUTOR
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("unidesk-input-probe")
                .on_thread_start(prioritize_current_thread)
                .enable_all()
                .build()
                .map_err(|error| error.to_string())
        })
        .as_ref()
        .map(|runtime| runtime.handle().clone())
        .map_err(|error| io::Error::other(error.clone()))
}

/// Run blocking interface/address inspection on the probe executor's blocking
/// pool rather than whichever runtime happens to host the caller. Inspection is
/// cheap per call but is issued per candidate address, per session watchdog and
/// per route measurement; keeping it off the session runtime protects the live
/// datagram reader from that jitter.
pub(crate) async fn inspect_blocking<T, F>(work: F) -> Result<T, tokio::task::JoinError>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    match probe() {
        Ok(handle) => handle.spawn_blocking(work).await,
        Err(_) => tokio::task::spawn_blocking(work).await,
    }
}

/// Entering a runtime just for Endpoint::new is insufficient: Quinn can spawn
/// connection drivers later from connect/accept on another thread. Pin all
/// driver spawns, sockets and timers explicitly to the input executor.
#[derive(Debug)]
pub(crate) struct InputQuicRuntime(Handle);
impl InputQuicRuntime {
    pub(crate) fn new() -> io::Result<Self> {
        handle().map(Self)
    }
    /// Endpoint runtime for outbound input dials only, so a handshake to an
    /// unreachable address never polls on the live session's workers.
    pub(crate) fn dial() -> io::Result<Self> {
        probe().map(Self)
    }
}
impl quinn::Runtime for InputQuicRuntime {
    fn new_timer(&self, deadline: Instant) -> Pin<Box<dyn quinn::AsyncTimer>> {
        let _entered = self.0.enter();
        quinn::TokioRuntime.new_timer(deadline)
    }
    fn spawn(&self, future: Pin<Box<dyn Future<Output = ()> + Send>>) {
        self.0.spawn(future);
    }
    fn wrap_udp_socket(
        &self,
        socket: std::net::UdpSocket,
    ) -> io::Result<Arc<dyn quinn::AsyncUdpSocket>> {
        let _entered = self.0.enter();
        quinn::TokioRuntime.wrap_udp_socket(socket)
    }
    fn now(&self) -> Instant {
        let _entered = self.0.enter();
        quinn::TokioRuntime.now()
    }
}
