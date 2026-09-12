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

/// Entering a runtime just for Endpoint::new is insufficient: Quinn can spawn
/// connection drivers later from connect/accept on another thread. Pin all
/// driver spawns, sockets and timers explicitly to the input executor.
#[derive(Debug)]
pub(crate) struct InputQuicRuntime(Handle);
impl InputQuicRuntime {
    pub(crate) fn new() -> io::Result<Self> {
        handle().map(Self)
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
