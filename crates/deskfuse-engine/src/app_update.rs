#[cfg(not(test))]
use deskfuse_runtime::{
    RuntimeHandle,
    updates::{LanUpdateConfig, LanUpdater},
};
#[cfg(not(test))]
use deskfuse_update::Target;
use deskfuse_update::installer;
use std::path::PathBuf;

#[cfg(not(test))]
pub fn start(runtime: RuntimeHandle) -> Result<LanUpdater, String> {
    let bundle = installer::current_bundle()?;
    let installed = installer::installed_release(&bundle)?;
    let root = installer::update_root()?;
    let version = std::process::Command::new("/usr/bin/sw_vers")
        .arg("-productVersion")
        .output()
        .map_err(|e| e.to_string())?;
    let mut os = String::from_utf8_lossy(&version.stdout).trim().to_owned();
    while os.matches('.').count() < 2 {
        os.push_str(".0");
    }
    let os = os.parse().map_err(|_| "无法识别 macOS 版本")?;
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
    let receipt = installer::completed_status()?
        .filter(|s| s.release == installed)
        .map(|status| deskfuse_protocol::update::UpdateReceipt {
            transaction_id: status.transaction_id.as_bytes().to_vec(),
            release_id: installed.release_id.as_bytes().to_vec(),
            installed_version: installed.version.to_string(),
            installed_build: installed.build_number,
            target: 1,
            sha256: hex::decode(&status.artifact.sha256).unwrap_or_default(),
        });
    LanUpdater::start(
        runtime,
        LanUpdateConfig {
            installed,
            target: Target::MacosArm64,
            os,
            root,
            seed_dirs,
            key: deskfuse_update::pinned_release_key().map_err(|e| e.to_string())?,
            receipt,
            previous_failure: installer::previous_failure()?,
        },
    )
}

/// Run before constructing the engine. A transaction is acknowledged only by the native UI.
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
pub fn acknowledge_startup(status: &deskfuse_update::UpdateStatus) -> Result<(), String> {
    installer::acknowledge_self_test(status)
}

pub type StageTask = tokio::task::JoinHandle<Result<(), String>>;
pub fn stage(
    release: deskfuse_update::VerifiedRelease,
    payload: PathBuf,
    status: deskfuse_update::UpdateStatus,
) -> StageTask {
    tokio::task::spawn_blocking(move || installer::prepare(&release, &payload, &status))
}
