use super::{Engine, EngineError, device, failure};
use deskfuse_engine::remote_application::service;
use std::sync::Arc;
#[derive(uniffi::Record)]
pub struct RemoteApplicationNetworkStats {
    pub sent_packets: u64,
    pub lost_packets: u64,
    pub rtt_milliseconds: f64,
    pub remote_address: String,
}
#[derive(uniffi::Object)]
pub struct RemoteApplicationChannel {
    session: Arc<deskfuse_transport::remote_application::RemoteSession>,
    /// Incoming hosting is unaffected by the local viewer-role restriction.
    viewer_service: Option<Arc<deskfuse_engine::remote_application::Service>>,
}
impl RemoteApplicationChannel {
    fn check_viewer_role(&self) -> Result<(), EngineError> {
        if let Some(service) = &self.viewer_service
            && let Err(error) = service.require_viewer_allowed()
        {
            self.session.close();
            return Err(failure(error));
        }
        Ok(())
    }
}
#[uniffi::export]
impl RemoteApplicationChannel {
    pub fn network_stats(&self) -> RemoteApplicationNetworkStats {
        let (sent_packets, lost_packets, rtt_milliseconds) = self.session.network_stats();
        RemoteApplicationNetworkStats {
            sent_packets,
            lost_packets,
            rtt_milliseconds,
            remote_address: self.session.remote_address(),
        }
    }
    pub fn peer_id(&self) -> String {
        self.session.peer.to_string()
    }
    /// Compressed packets only. Never a pixel buffer and never a UI snapshot.
    pub fn send(&self, bytes: Vec<u8>) -> Result<(), EngineError> {
        self.check_viewer_role()?;
        self.session.send_blocking(bytes).map_err(failure)
    }
    pub fn receive(&self, wait_ms: u64) -> Result<Option<Vec<u8>>, EngineError> {
        self.check_viewer_role()?;
        self.session.receive(wait_ms).map_err(failure)
    }
    pub fn close(&self) {
        self.session.close();
    }
}
#[uniffi::export]
impl Engine {
    pub fn remote_application_diagnostic(&self, event: String, detail: String) {
        if event.len() <= 64 && detail.len() <= 512 {
            deskfuse_platform::timing::record(
                "remote_app",
                std::time::Duration::ZERO,
                format!("event={event} {detail}"),
            );
        }
    }
    pub fn set_remote_application_hosting(&self, enabled: bool) -> Result<(), EngineError> {
        self.active()?;
        service().map_err(failure)?.enable(enabled);
        Ok(())
    }
    pub fn open_remote_application(
        &self,
        peer_id: String,
    ) -> Result<Arc<RemoteApplicationChannel>, EngineError> {
        self.active()?;
        let service = service().map_err(failure)?;
        Ok(Arc::new(RemoteApplicationChannel {
            session: service.open(device(&peer_id)?).map_err(failure)?,
            viewer_service: Some(service),
        }))
    }
    pub fn take_remote_application(
        &self,
    ) -> Result<Option<Arc<RemoteApplicationChannel>>, EngineError> {
        self.active()?;
        Ok(service().map_err(failure)?.incoming().map(|session| {
            Arc::new(RemoteApplicationChannel {
                session,
                viewer_service: None,
            })
        }))
    }
    pub fn remote_application_viewer_control(&self, active: bool) -> Result<(), EngineError> {
        self.active()?;
        service()
            .map_err(failure)?
            .viewer_control(active)
            .map_err(failure)
    }
    pub fn remote_application_viewer_ready(&self) -> Result<bool, EngineError> {
        self.active()?;
        Ok(service().map_err(failure)?.viewer_ready())
    }
    pub fn remote_application_prepare_control(&self, peer_id: String) -> Result<(), EngineError> {
        self.active()?;
        service()
            .map_err(failure)?
            .prepare_control(device(&peer_id)?)
            .map_err(failure)
    }
    pub fn remote_application_control_ready(
        &self,
        peer_id: String,
        session: String,
        generation: u64,
    ) -> Result<bool, EngineError> {
        self.active()?;
        Ok(service()
            .map_err(failure)?
            .control_ready(device(&peer_id)?, &session, generation))
    }
    pub fn remote_application_input(
        &self,
        peer_id: String,
        packet_json: String,
    ) -> Result<(), EngineError> {
        self.active()?;
        service()
            .map_err(failure)?
            .input(device(&peer_id)?, &packet_json)
            .map_err(failure)
    }
    pub fn remote_application_control_problem(
        &self,
        peer_id: String,
        session: String,
        generation: u64,
    ) -> Result<String, EngineError> {
        self.active()?;
        Ok(service()
            .map_err(failure)?
            .control_problem(device(&peer_id)?, &session, generation))
    }
}

#[derive(uniffi::Object)]
pub struct RemoteRdpTunnelConnection {
    #[cfg(unix)]
    connection: Arc<deskfuse_engine::remote_application::RdpTunnelConnection>,
}
#[uniffi::export]
impl RemoteRdpTunnelConnection {
    pub fn close(&self) {
        #[cfg(unix)]
        self.connection.close();
    }
    /// 隧道异常结束的原因；仍在转发、本地关闭或正常结束时为 None。
    pub fn closed_reason(&self) -> Option<String> {
        #[cfg(unix)]
        {
            self.connection.closed_reason()
        }
        #[cfg(not(unix))]
        {
            Some("此平台不支持原生 RDP 隧道".into())
        }
    }
}
#[uniffi::export]
impl Engine {
    /// Takes ownership of fd on every path, including invalid ticket/peer errors.
    /// The native bridge transfers this socketpair endpoint exactly once.
    pub fn open_remote_rdp_tunnel(
        &self,
        peer_id: String,
        ticket: Vec<u8>,
        fd: i32,
    ) -> Result<Arc<RemoteRdpTunnelConnection>, EngineError> {
        #[cfg(unix)]
        {
            use std::os::fd::FromRawFd;
            if fd < 0 {
                return Err(failure("无效的 RDP 隧道描述符"));
            }
            // SAFETY: the in-process C bridge transfers a uniquely owned socketpair fd.
            #[allow(unsafe_code)]
            let socket = unsafe { std::os::unix::net::UnixStream::from_raw_fd(fd) };
            self.active()?;
            let service = service().map_err(failure)?;
            let connection = service
                .open_rdp_tunnel(device(&peer_id)?, ticket, socket)
                .map_err(failure)?;
            Ok(Arc::new(RemoteRdpTunnelConnection { connection }))
        }
        #[cfg(not(unix))]
        {
            let _ = (peer_id, ticket, fd);
            Err(failure("此平台不支持原生 RDP 隧道"))
        }
    }
}
