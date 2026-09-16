//! LAN web extension. axum is confined to this crate.
#![forbid(unsafe_code)]

pub mod server;

pub use server::LanWebServer;

use std::{net::SocketAddr, sync::Arc};
use tokio::task::JoinHandle;
use unidesk_collaboration::CollaborationCore;

pub struct LanWebExtension {
    core: Arc<CollaborationCore>,
    bind: SocketAddr,
    server: Option<JoinHandle<Result<(), std::io::Error>>>,
}

impl LanWebExtension {
    #[must_use]
    pub fn new(core: Arc<CollaborationCore>) -> Self {
        Self {
            core,
            bind: SocketAddr::from(([127, 0, 0, 1], 0)),
            server: None,
        }
    }

    pub fn bind(mut self, bind: SocketAddr) -> Self {
        self.bind = bind;
        self
    }

    pub async fn start(&mut self) -> unidesk_collaboration::Result<()> {
        let server = LanWebServer::new(Arc::clone(&self.core), self.bind);
        self.server = Some(server.spawn());
        Ok(())
    }

    pub async fn stop(&mut self) {
        if let Some(server) = self.server.take() {
            server.abort();
        }
    }
}
