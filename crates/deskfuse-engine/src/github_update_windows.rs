//! Windows composition placeholder for the macOS in-process GitHub installer.
//! The production Windows updater belongs to the machine-owned transaction helper;
//! this type only keeps shared Tokio bridge signatures type-checked.

pub(crate) fn is_public_bundle() -> bool {
    false
}
pub(crate) fn is_manual_bundle() -> bool {
    true
}

pub(crate) struct GitHubUpdater;

impl GitHubUpdater {
    pub(crate) fn start() -> Result<Self, String> {
        Err("Windows 更新由机器事务协调器处理".into())
    }
    pub(crate) fn check_now(&mut self) {}
    pub(crate) fn download_and_install_now(&mut self) {}
    pub(crate) async fn poll(&mut self) {}
    pub(crate) fn ready(&self) -> bool {
        false
    }
    pub(crate) fn force_install(&self) -> bool {
        false
    }
    pub(crate) fn install_abandoned(&self) -> bool {
        false
    }
    pub(crate) fn abandon_install(&mut self, _reason: &str) {}
    pub(crate) fn installation_waiting(&mut self, _message: &str) {}
    pub(crate) fn installing(&mut self) {}
    pub(crate) fn snapshot(&self) -> deskfuse_runtime::updates::LanUpdateView {
        deskfuse_runtime::updates::LanUpdateView {
            github: Some(Default::default()),
            message: "Windows 更新由机器事务协调器处理".into(),
            ..Default::default()
        }
    }
}
