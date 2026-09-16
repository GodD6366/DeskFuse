use axum::{Router, routing::get};
use std::{net::SocketAddr, sync::Arc};
use tokio::task::JoinHandle;
use unidesk_collaboration::CollaborationCore;

pub struct LanWebServer {
    core: Arc<CollaborationCore>,
    bind: SocketAddr,
}

impl LanWebServer {
    #[must_use]
    pub fn new(core: Arc<CollaborationCore>, bind: SocketAddr) -> Self {
        Self { core, bind }
    }

    pub fn spawn(&self) -> JoinHandle<Result<(), std::io::Error>> {
        let core = Arc::clone(&self.core);
        let bind = self.bind;
        tokio::spawn(async move {
            let app = Router::new().route("/", get(index)).with_state(core);
            let listener = tokio::net::TcpListener::bind(bind).await?;
            axum::serve(listener, app).await
        })
    }
}

async fn index(
    axum::extract::State(_core): axum::extract::State<Arc<CollaborationCore>>,
) -> &'static str {
    "uniDesk LAN web"
}
