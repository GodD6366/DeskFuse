//! Control QUIC I/O must outlive, and run independently of, the application
//! bridge runtime that constructs a transport. Native clipboard and app work
//! can block that bridge's single thread without stopping the device listener.
use std::{
    future::Future,
    io,
    pin::Pin,
    sync::{Arc, OnceLock},
    time::Instant,
};
use tokio::runtime::{Handle, Runtime};

pub(crate) fn handle() -> io::Result<Handle> {
    static EXECUTOR: OnceLock<Result<Runtime, String>> = OnceLock::new();
    EXECUTOR
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("deskfuse-control-io")
                .enable_all()
                .build()
                .map_err(|error| error.to_string())
        })
        .as_ref()
        .map(|runtime| runtime.handle().clone())
        .map_err(|error| io::Error::other(error.clone()))
}

/// Pin connection-driver spawns as well as sockets and timers. Merely entering
/// a runtime during bind does not cover a later dial from another runtime.
#[derive(Debug)]
pub(crate) struct ControlQuicRuntime(pub(crate) Handle);

impl quinn::Runtime for ControlQuicRuntime {
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
        quinn::TokioRuntime.now()
    }
}
