#[cfg(not(test))]
use deskfuse_runtime::{
    RuntimeHandle,
    updates::{LanUpdateConfig, LanUpdater, UpdateChannel},
};
#[cfg(not(test))]
use deskfuse_update::Target;
#[cfg(not(windows))]
use deskfuse_update::installer;
use std::path::PathBuf;
#[cfg(not(test))]
use std::sync::Arc;

#[cfg(all(not(test), not(windows)))]
fn macos_version() -> Result<semver::Version, String> {
    let version = std::process::Command::new("/usr/bin/sw_vers")
        .arg("-productVersion")
        .output()
        .map_err(|e| e.to_string())?;
    let mut os = String::from_utf8_lossy(&version.stdout).trim().to_owned();
    while os.matches('.').count() < 2 {
        os.push_str(".0");
    }
    os.parse().map_err(|_| "无法识别 macOS 版本".into())
}

#[cfg(all(not(test), not(windows)))]
fn lan_receipt(
    installed: &deskfuse_update::ReleaseVersion,
) -> Result<Option<deskfuse_protocol::update::UpdateReceipt>, String> {
    Ok(installer::completed_status()?
        .filter(|s| s.release == *installed)
        .map(|status| deskfuse_protocol::update::UpdateReceipt {
            transaction_id: status.transaction_id.as_bytes().to_vec(),
            release_id: installed.release_id.as_bytes().to_vec(),
            installed_version: installed.version.to_string(),
            installed_build: installed.build_number,
            target: 1,
            sha256: hex::decode(&status.artifact.sha256).unwrap_or_default(),
        }))
}

#[cfg(all(not(test), not(windows)))]
pub fn start(runtime: RuntimeHandle) -> Result<LanUpdater, String> {
    let bundle = installer::current_bundle()?;
    let installed = installer::installed_release(&bundle)?;
    let root = installer::update_root()?;
    let os = macos_version()?;
    let mut seed_dirs = vec![
        bundle.join("Contents/Resources/LanUpdate"),
        root.join("inbox"),
    ];
    // Manual installations carry the original signed ZIP inside the app.
    // Network installations retain it in the verified cache. Mounted installers
    // remain an additional source, not a requirement for peer distribution.
    if let Ok(volumes) = std::fs::read_dir("/Volumes") {
        seed_dirs.extend(
            volumes
                .flatten()
                .filter(|v| v.file_name().to_string_lossy().starts_with("DeskFuse "))
                .map(|v| v.path().join(".updates")),
        );
    }
    LanUpdater::start(
        runtime,
        LanUpdateConfig {
            installed: installed.clone(),
            target: Target::MacosArm64,
            os,
            root,
            seed_dirs,
            key: deskfuse_update::pinned_release_key().map_err(|e| e.to_string())?,
            receipt: lan_receipt(&installed)?,
            previous_failure: installer::previous_failure()?,
            channel: UpdateChannel::Lan,
        },
    )
}

/// Public GitHub bundles still check GitHub, and also package the running app
/// so a peer can install it over the LAN update transport.
#[cfg(all(not(test), not(windows)))]
pub fn start_github(runtime: RuntimeHandle) -> Result<LanUpdater, String> {
    start_github_with_trigger(runtime, None)
}

/// Start the public-channel LAN package server and optionally notify the
/// canonical GitHub updater when a paired device asks for an update.
#[cfg(all(not(test), not(windows)))]
pub fn start_github_with_trigger(
    runtime: RuntimeHandle,
    remote_trigger: Option<Arc<dyn Fn() + Send + Sync>>,
) -> Result<LanUpdater, String> {
    let bundle = installer::current_bundle()?;
    let installed = installer::installed_release(&bundle)?;
    let root = installer::update_root()?;
    LanUpdater::start_with_remote_trigger(
        runtime,
        LanUpdateConfig {
            installed: installed.clone(),
            target: Target::MacosArm64,
            os: macos_version()?,
            root,
            seed_dirs: Vec::new(),
            key: deskfuse_update::pinned_release_key().map_err(|e| e.to_string())?,
            receipt: lan_receipt(&installed)?,
            previous_failure: installer::previous_failure()?,
            channel: UpdateChannel::GitHub,
        },
        remote_trigger,
    )
}

/// Run before constructing the engine. A transaction is acknowledged only by the native UI.
#[cfg(not(windows))]
pub fn prepare_startup(
    transaction: Option<String>,
) -> Result<Option<deskfuse_update::UpdateStatus>, String> {
    if let Some(transaction) = transaction {
        let status = installer::self_test_status(&transaction)?;
        let store = deskfuse_store::SQLiteStore::new().map_err(|e| e.to_string())?;
        store.schema_version().map_err(|e| e.to_string())?;
        Ok(Some(status))
    } else {
        installer::recover()?;
        Ok(None)
    }
}
#[cfg(not(windows))]
pub fn acknowledge_startup(status: &deskfuse_update::UpdateStatus) -> Result<(), String> {
    installer::acknowledge_self_test(status)
}

pub type StageTask = tokio::task::JoinHandle<Result<(), String>>;
#[cfg(not(windows))]
pub fn stage(
    release: deskfuse_update::VerifiedRelease,
    payload: PathBuf,
    status: deskfuse_update::UpdateStatus,
) -> StageTask {
    tokio::task::spawn_blocking(move || installer::prepare(&release, &payload, &status))
}
#[cfg(not(windows))]
pub fn stage_github_zip(
    release: deskfuse_update::ReleaseVersion,
    payload: PathBuf,
    status: deskfuse_update::UpdateStatus,
) -> StageTask {
    tokio::task::spawn_blocking(move || installer::prepare_github_zip(&payload, &release, &status))
}

#[cfg(all(not(test), windows))]
use deskfuse_runtime::RuntimeHandle;

#[cfg(all(not(test), windows))]
pub fn start(_runtime: RuntimeHandle) -> Result<deskfuse_runtime::updates::LanUpdater, String> {
    Err("Windows LAN updates require a signed machine-owned transaction".into())
}

#[cfg(all(not(test), windows))]
pub fn start_github(
    _runtime: RuntimeHandle,
) -> Result<deskfuse_runtime::updates::LanUpdater, String> {
    Err("Windows GitHub updates require a signed machine-owned transaction".into())
}

#[cfg(all(not(test), windows))]
pub fn start_github_with_trigger(
    _runtime: RuntimeHandle,
    _remote_trigger: Option<Arc<dyn Fn() + Send + Sync>>,
) -> Result<deskfuse_runtime::updates::LanUpdater, String> {
    Err("Windows GitHub updates require a signed machine-owned transaction".into())
}

#[cfg(windows)]
pub fn prepare_startup(
    transaction: Option<String>,
) -> Result<Option<deskfuse_update::UpdateStatus>, String> {
    if transaction.is_some() {
        Err("Windows updater startup evidence is unavailable".into())
    } else {
        Ok(None)
    }
}

#[cfg(windows)]
pub fn acknowledge_startup(_status: &deskfuse_update::UpdateStatus) -> Result<(), String> {
    Err("Windows updater acknowledgement requires machine-owned evidence".into())
}

#[cfg(windows)]
pub fn stage(
    _release: deskfuse_update::VerifiedRelease,
    _payload: PathBuf,
    _status: deskfuse_update::UpdateStatus,
) -> StageTask {
    tokio::task::spawn_blocking(|| {
        Err("Windows transactional payload unavailable; NSIS is first-install only".into())
    })
}

#[cfg(windows)]
pub fn stage_github_zip(
    _release: deskfuse_update::ReleaseVersion,
    _payload: PathBuf,
    _status: deskfuse_update::UpdateStatus,
) -> StageTask {
    tokio::task::spawn_blocking(|| {
        Err("Windows transactional payload unavailable; NSIS is first-install only".into())
    })
}
