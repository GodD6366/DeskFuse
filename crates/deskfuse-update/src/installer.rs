//! macOS replacement runs outside the application. Only authenticated release
//! bytes enter staging; a durable journal precedes every destructive step.
use crate::{
    PackageKind, ReleaseVersion, SignedReleaseManifest, Target, UpdateStatus, VerifiedRelease,
    pinned_release_key, verify_artifact, write_json_atomic,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallJournal {
    pub status: UpdateStatus,
    pub target: PathBuf,
    pub phase: Phase,
    pub parent_pid: u32,
    pub failure: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Phase {
    Prepared,
    BackedUp,
    Replaced,
    Committed,
    RolledBack,
}

/// # Errors
/// Returns a verification, storage, or platform operation failure.
pub fn update_root() -> Result<PathBuf> {
    Ok(
        PathBuf::from(std::env::var_os("HOME").ok_or("HOME unavailable")?)
            .join("Library/Application Support/DeskFuse/updates"),
    )
}
fn private_dir(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir_all(path).map_err(|e| e.to_string())?;
    if !path.symlink_metadata().map_err(|e| e.to_string())?.is_dir() {
        return Err("更新目录不是普通目录".into());
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())
}
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    use std::io::Read;
    let file = File::open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > 65536 {
        return Err("更新记录过大".into());
    }
    let mut data = Vec::new();
    file.take(65537)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    serde_json::from_slice(&data).map_err(|e| e.to_string())
}
fn command(command: &mut Command) -> Result<()> {
    let output = command.output().map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!("更新系统操作失败（{}）", output.status))
    }
}
/// # Errors
/// Returns a verification, storage, or platform operation failure.
pub fn current_bundle() -> Result<PathBuf> {
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let bundle = executable
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .ok_or("没有应用安装路径")?;
    if bundle.extension().is_none_or(|e| e != "app") {
        return Err("开发程序没有自动安装目标".into());
    }
    fs::canonicalize(bundle).map_err(|e| e.to_string())
}
/// # Errors
/// Returns a verification, storage, or platform operation failure.
pub fn installed_release(bundle: &Path) -> Result<ReleaseVersion> {
    read_json(&bundle.join("Contents/Resources/release.json"))
}
fn signature_requirement(bundle: &Path) -> Result<String> {
    command(
        Command::new("/usr/bin/codesign")
            .args(["--verify", "--deep", "--strict"])
            .arg(bundle),
    )?;
    let output = Command::new("/usr/bin/codesign")
        .args(["-dr", "-"])
        .arg(bundle)
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err("无法读取应用签名要求".into());
    }
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let requirement = text
        .lines()
        .find_map(|line| line.strip_prefix("designated => "))
        .ok_or("应用缺少签名要求")?;
    // Ad-hoc requirements are code hashes, not a stable publisher identity.
    if !requirement.contains("certificate") || !requirement.contains("com.deskfuse.app") {
        return Err("自动安装需要稳定的应用签名身份".into());
    }
    Ok(requirement.to_owned())
}
fn verify_bundle(bundle: &Path, release: &ReleaseVersion, requirement: &str) -> Result<()> {
    if installed_release(bundle)? != *release {
        return Err("应用版本与发布清单不一致".into());
    }
    command(
        Command::new("/usr/bin/codesign")
            .args(["--verify", "--deep", "--strict", "-R"])
            .arg(format!("={requirement}"))
            .arg(bundle),
    )?;
    let id = Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", "Print :CFBundleIdentifier"])
        .arg(bundle.join("Contents/Info.plist"))
        .output()
        .map_err(|e| e.to_string())?;
    if String::from_utf8_lossy(&id.stdout).trim() != "com.deskfuse.app" {
        return Err("应用标识不一致".into());
    }
    let arch = Command::new("/usr/bin/lipo")
        .arg("-archs")
        .arg(bundle.join("Contents/MacOS/deskfuse-app"))
        .output()
        .map_err(|e| e.to_string())?;
    if !arch.status.success() || String::from_utf8_lossy(&arch.stdout).trim() != "arm64" {
        return Err("更新应用架构不兼容".into());
    }
    Ok(())
}
fn paths(root: &Path, journal: &InstallJournal) -> Result<(PathBuf, PathBuf, PathBuf)> {
    if !journal.target.is_absolute()
        || journal.target.extension().is_none_or(|e| e != "app")
        || journal.target.file_name().is_none()
    {
        return Err("安装目标无效".into());
    }
    let parent = journal.target.parent().ok_or("安装目标无效")?;
    let id = journal.status.transaction_id;
    Ok((
        root.join(format!("install-{id}")),
        parent.join(format!(".DeskFuse-previous-{id}.app")),
        parent.join(format!(".DeskFuse-next-{id}.app")),
    ))
}
fn persist(root: &Path, journal: &InstallJournal) -> Result<()> {
    write_json_atomic(&root.join("install.json"), journal).map_err(|e| e.to_string())
}

/// Staging is repeatable while user work continues. No installed bundle or DB is modified.
/// # Errors
/// Returns a verification, storage, or platform operation failure.
pub fn prepare(release: &VerifiedRelease, payload: &Path, status: &UpdateStatus) -> Result<()> {
    if release.manifest().release != status.release
        || release.artifact(Target::MacosArm64) != Some(&status.artifact)
    {
        return Err("安装任务不匹配".into());
    }
    let target = current_bundle()?;
    if target.starts_with("/Volumes") {
        return Err("请先将 DeskFuse 拖入应用程序目录，磁盘映像内无法自动安装".into());
    }
    let installed = installed_release(&target)?;
    if !release.manifest().release.precedence(&installed).is_gt() {
        return Err("更新版本不高于已安装版本".into());
    }
    let requirement = signature_requirement(&target)?;
    let root = update_root()?;
    private_dir(&root)?;
    let mut journal = InstallJournal {
        status: status.clone(),
        target,
        phase: Phase::Prepared,
        parent_pid: std::process::id(),
        failure: None,
    };
    journal.status.phase = crate::UpdatePhase::Staged;
    let (stage, _, candidate) = paths(&root, &journal)?;
    if stage.exists() || candidate.exists() {
        return Err("更新暂存目录已存在".into());
    }
    private_dir(&stage)?;
    persist(&root, &journal)?;
    let result = (|| {
        let zip = stage.join(&status.artifact.file_name);
        fs::copy(payload, &zip).map_err(|e| e.to_string())?;
        verify_artifact(release, Target::MacosArm64, &zip).map_err(|e| e.to_string())?;
        write_json_atomic(&stage.join("manifest.json"), release.signed())
            .map_err(|e| e.to_string())?;
        // Reject traversal before ditto sees the archive. The signature authorizes
        // the publisher, but is not permission to write outside the staging tree.
        let entries = Command::new("/usr/bin/unzip")
            .args(["-Z", "-1"])
            .arg(&zip)
            .output()
            .map_err(|e| e.to_string())?;
        if !entries.status.success()
            || !String::from_utf8_lossy(&entries.stdout).lines().all(|p| {
                !p.starts_with('/')
                    && !p.split('/').any(|c| c == "..")
                    && (p.starts_with("DeskFuse.app/") || p.starts_with("__MACOSX/"))
            })
        {
            return Err("更新压缩包包含无效路径".into());
        }
        command(
            Command::new("/usr/bin/ditto")
                .args(["-x", "-k"])
                .arg(&zip)
                .arg(&stage),
        )?;
        verify_bundle(&stage.join("DeskFuse.app"), &status.release, &requirement)?;
        command(
            Command::new("/usr/bin/ditto")
                .arg(stage.join("DeskFuse.app"))
                .arg(&candidate),
        )?;
        verify_bundle(&candidate, &status.release, &requirement)?;
        fs::write(stage.join("requirement.txt"), requirement).map_err(|e| e.to_string())?;
        fs::copy(
            journal.target.join("Contents/MacOS/deskfuse-update-helper"),
            stage.join("deskfuse-update-helper"),
        )
        .map_err(|e| e.to_string())?;
        persist(&root, &journal)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&candidate);
        let _ = fs::remove_dir_all(&stage);
    }
    result
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GitHubInstallSource {
    sha256: String,
    file_name: String,
}

struct MountGuard(PathBuf);
impl Drop for MountGuard {
    fn drop(&mut self) {
        let _ = Command::new("/usr/bin/hdiutil")
            .args(["detach", "-quiet", "-force"])
            .arg(&self.0)
            .status();
    }
}

fn digest_file(path: &Path, size: u64, sha256: &str) -> Result<()> {
    let metadata = path.symlink_metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() != size {
        return Err("更新包大小不匹配".into());
    }
    let file = File::open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() != size {
        return Err("更新包大小不匹配".into());
    }
    let mut file = file.take(size + 1);
    let mut digest = Sha256::new();
    let mut buffer = vec![0; 64 * 1024];
    let mut bytes = 0u64;
    loop {
        let read = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
        bytes += read as u64;
    }
    if bytes != size || hex::encode(digest.finalize()) != sha256 {
        return Err("更新包 SHA-256 不匹配".into());
    }
    Ok(())
}

fn verify_staged_payload(stage: &Path, journal: &InstallJournal) -> Result<()> {
    let github = stage.join("github-source.json");
    let lan = stage.join("manifest.json");
    match (github.is_file(), lan.is_file()) {
        (true, false) => {
            let source: GitHubInstallSource = read_json(&github)?;
            if source.file_name != "DeskFuse-arm64.dmg"
                || source.file_name != journal.status.artifact.file_name
                || source.sha256 != journal.status.artifact.sha256
                || journal.status.artifact.kind != PackageKind::MacosDmg
                || journal.status.artifact.target != Target::MacosArm64
            {
                return Err("GitHub 安装证明不匹配".into());
            }
            digest_file(
                &stage.join(source.file_name),
                journal.status.artifact.size,
                &source.sha256,
            )
        }
        (false, true) => {
            if journal.status.artifact.kind != PackageKind::MacosAppZip {
                return Err("局域网安装包类型无效".into());
            }
            let signed: SignedReleaseManifest = read_json(&lan)?;
            let release = signed
                .verify(&pinned_release_key().map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            verify_artifact(
                &release,
                Target::MacosArm64,
                &stage.join(&journal.status.artifact.file_name),
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        }
        _ => Err("安装暂存缺少唯一的来源证明".into()),
    }
}

/// Public GitHub DMGs are not LAN-signed. Trust is SHA-256 plus the already-running
/// app's designated code-signing requirement. Never write a LAN manifest here.
/// # Errors
/// Returns a verification, storage, or platform operation failure.
pub fn prepare_github(
    payload: &Path,
    release: &ReleaseVersion,
    status: &UpdateStatus,
) -> Result<()> {
    if status.release != *release
        || status.artifact.kind != PackageKind::MacosDmg
        || status.artifact.target != Target::MacosArm64
        || status.artifact.file_name != "DeskFuse-arm64.dmg"
    {
        return Err("安装任务不匹配".into());
    }
    digest_file(payload, status.artifact.size, &status.artifact.sha256)?;
    let target = current_bundle()?;
    if target.starts_with("/Volumes") {
        return Err("请先将 DeskFuse 拖入应用程序目录，磁盘映像内无法自动安装".into());
    }
    let installed = installed_release(&target)?;
    if !release.precedence(&installed).is_gt() {
        return Err("更新版本不高于已安装版本".into());
    }
    let requirement = signature_requirement(&target)?;
    let root = update_root()?;
    private_dir(&root)?;
    let mut journal = InstallJournal {
        status: status.clone(),
        target,
        phase: Phase::Prepared,
        parent_pid: std::process::id(),
        failure: None,
    };
    journal.status.phase = crate::UpdatePhase::Staged;
    let (stage, _, candidate) = paths(&root, &journal)?;
    if stage.exists() || candidate.exists() {
        return Err("更新暂存目录已存在".into());
    }
    private_dir(&stage)?;
    persist(&root, &journal)?;
    let result = (|| {
        let dmg = stage.join(&status.artifact.file_name);
        fs::copy(payload, &dmg).map_err(|e| e.to_string())?;
        digest_file(&dmg, status.artifact.size, &status.artifact.sha256)?;
        write_json_atomic(
            &stage.join("github-source.json"),
            &GitHubInstallSource {
                sha256: status.artifact.sha256.clone(),
                file_name: status.artifact.file_name.clone(),
            },
        )
        .map_err(|e| e.to_string())?;
        let mount = stage.join("mount");
        private_dir(&mount)?;
        command(
            Command::new("/usr/bin/hdiutil")
                .args(["attach", "-nobrowse", "-readonly", "-mountpoint"])
                .arg(&mount)
                .arg(&dmg),
        )?;
        let _mounted = MountGuard(mount.clone());
        let app = mount.join("DeskFuse.app");
        if app
            .symlink_metadata()
            .map_err(|e| e.to_string())?
            .file_type()
            .is_symlink()
        {
            return Err("磁盘映像中的应用不是普通目录".into());
        }
        verify_bundle(&app, &status.release, &requirement)?;
        command(Command::new("/usr/bin/ditto").arg(&app).arg(&candidate))?;
        verify_bundle(&candidate, &status.release, &requirement)?;
        let _ = Command::new("/usr/bin/xattr")
            .args(["-dr", "com.apple.quarantine"])
            .arg(&candidate)
            .status();
        fs::write(stage.join("requirement.txt"), requirement).map_err(|e| e.to_string())?;
        fs::copy(
            journal.target.join("Contents/MacOS/deskfuse-update-helper"),
            stage.join("deskfuse-update-helper"),
        )
        .map_err(|e| e.to_string())?;
        persist(&root, &journal)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&candidate);
        let _ = fs::remove_dir_all(&stage);
    }
    result
}
/// Called only after input and transfer admission have closed and work has drained.
/// The caller owns the child so a handoff that never completes can be cancelled
/// before the helper leaves its wait for this process to exit.
/// # Errors
/// Returns a verification, storage, or platform operation failure.
pub fn launch_helper() -> Result<std::process::Child> {
    let root = update_root()?;
    let journal: InstallJournal = read_json(&root.join("install.json"))?;
    let (stage, _, _) = paths(&root, &journal)?;
    let child = Command::new(stage.join("deskfuse-update-helper"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(child)
}
fn database_paths(root: &Path) -> Result<[PathBuf; 3]> {
    let parent = root.parent().ok_or("数据库位置无效")?;
    Ok([
        parent.join("state.sqlite3"),
        parent.join("state.sqlite3-wal"),
        parent.join("state.sqlite3-shm"),
    ])
}
fn backup_database(root: &Path, stage: &Path) -> Result<()> {
    let backup = stage.join("database");
    private_dir(&backup)?;
    for source in database_paths(root)? {
        if source.exists() {
            fs::copy(
                &source,
                backup.join(source.file_name().ok_or("数据库位置无效")?),
            )
            .map_err(|e| e.to_string())?;
            File::open(backup.join(source.file_name().ok_or("数据库位置无效")?))
                .and_then(|f| f.sync_all())
                .map_err(|e| e.to_string())?;
        }
    }
    write_json_atomic(&stage.join("database-ready.json"), &true).map_err(|e| e.to_string())
}
/// `quarantine` separates evidence about the artifact from an interrupted
/// attempt. A release whose bytes failed verification or self-test must never be
/// offered again; a release whose installation was merely cut short stays
/// selectable, so a power loss or a force quit cannot blacklist a good build.
fn rollback(root: &Path, journal: &mut InstallJournal, quarantine: bool) -> Result<()> {
    let (stage, previous, candidate) = paths(root, journal)?;
    if previous.exists() {
        if journal.target.exists() {
            fs::remove_dir_all(&journal.target).map_err(|e| e.to_string())?;
        }
        fs::rename(&previous, &journal.target).map_err(|e| e.to_string())?;
    }
    if stage.join("database-ready.json").exists() {
        for destination in database_paths(root)? {
            let source = stage
                .join("database")
                .join(destination.file_name().ok_or("数据库位置无效")?);
            if destination.exists() {
                fs::remove_file(&destination).map_err(|e| e.to_string())?;
            }
            if source.exists() {
                fs::copy(source, destination).map_err(|e| e.to_string())?;
            }
        }
    }
    if quarantine {
        let mut quarantined: BTreeSet<Uuid> = if root.join("quarantined.json").exists() {
            read_json(&root.join("quarantined.json"))?
        } else {
            BTreeSet::new()
        };
        quarantined.insert(journal.status.release.release_id);
        write_json_atomic(&root.join("quarantined.json"), &quarantined)
            .map_err(|e| e.to_string())?;
    }
    journal.phase = Phase::RolledBack;
    journal.status.phase = if quarantine {
        crate::UpdatePhase::Quarantined
    } else {
        crate::UpdatePhase::Cancelled
    };
    persist(root, journal)?;
    let _ = fs::remove_dir_all(candidate);
    let _ = fs::remove_dir_all(stage);
    Ok(())
}
fn lock(root: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join("installer.lock"))
        .map_err(|e| e.to_string())?;
    file.try_lock().map_err(|_| "安装器正在运行")?;
    Ok(file)
}
/// External helper entry. A lost network connection after local success never rolls back.
/// # Errors
/// Returns a verification, storage, or platform operation failure.
pub fn run_helper() -> Result<()> {
    let root = update_root()?;
    let install_lock = lock(&root)?;
    let mut journal: InstallJournal = read_json(&root.join("install.json"))?;
    if journal.phase != Phase::Prepared {
        return Err("安装事务不在待安装状态".into());
    }
    let (stage, previous, candidate) = paths(&root, &journal)?;
    let deadline = Instant::now() + Duration::from_mins(1);
    while Command::new("/bin/kill")
        .args(["-0", &journal.parent_pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
    {
        if Instant::now() >= deadline {
            return Err("原应用未退出，保留当前安装".into());
        }
        thread::sleep(Duration::from_millis(100));
    }
    let result = (|| {
        let requirement =
            fs::read_to_string(stage.join("requirement.txt")).map_err(|e| e.to_string())?;
        if requirement != signature_requirement(&journal.target)? {
            return Err("原应用签名已变化".into());
        }
        verify_staged_payload(&stage, &journal)?;
        verify_bundle(&candidate, &journal.status.release, &requirement)?;
        backup_database(&root, &stage)?;
        // Journal + directory existence jointly recover a crash between rename and persist.
        fs::rename(&journal.target, &previous).map_err(|e| e.to_string())?;
        File::open(journal.target.parent().ok_or("安装目标无效")?)
            .and_then(|dir| dir.sync_all())
            .map_err(|e| e.to_string())?;
        journal.phase = Phase::BackedUp;
        persist(&root, &journal)?;
        fs::rename(&candidate, &journal.target).map_err(|e| e.to_string())?;
        File::open(journal.target.parent().ok_or("安装目标无效")?)
            .and_then(|dir| dir.sync_all())
            .map_err(|e| e.to_string())?;
        journal.phase = Phase::Replaced;
        persist(&root, &journal)?;
        let mut child = Command::new(journal.target.join("Contents/MacOS/deskfuse-app"))
            .arg("--update-self-test")
            .arg(journal.status.transaction_id.to_string())
            .spawn()
            .map_err(|e| e.to_string())?;
        let timeout = Instant::now() + Duration::from_millis(crate::SELF_TEST_TIMEOUT_MS);
        loop {
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                if !status.success() {
                    return Err("新版启动自检失败".into());
                }
                break;
            }
            if Instant::now() >= timeout {
                let _ = child.kill();
                let _ = child.wait();
                return Err("新版启动自检超时".into());
            }
            thread::sleep(Duration::from_millis(100));
        }
        let confirmed: UpdateStatus = read_json(&stage.join("self-test.json"))?;
        if confirmed != journal.status {
            return Err("新版启动确认不匹配".into());
        }
        journal.phase = Phase::Committed;
        journal.status.phase = crate::UpdatePhase::AwaitingReconnect;
        persist(&root, &journal)?;
        Ok(())
    })();
    if let Err(error) = result {
        journal.failure = Some(error);
        // This path only runs on a verification, requirement, or self-test
        // failure, which is evidence about the artifact itself.
        rollback(&root, &mut journal, true)?;
    }
    if journal.phase == Phase::Committed {
        // Keep the previous app/DB through self-test only. Signed cache retains previous bytes.
        let _ = fs::remove_dir_all(previous);
        let _ = fs::remove_dir_all(stage);
    }
    drop(install_lock);
    Command::new(journal.target.join("Contents/MacOS/deskfuse-app"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}
/// Run before identity, clipboard or network startup. Recovery never resets identity.
/// # Errors
/// Returns a verification, storage, or platform operation failure.
pub fn recover() -> Result<()> {
    let root = update_root()?;
    if !root.join("install.json").exists() {
        return Ok(());
    }
    let Ok(_lock) = lock(&root) else {
        return Err("安装器尚未结束，请稍后启动".into());
    };
    let mut journal: InstallJournal = read_json(&root.join("install.json"))?;
    if matches!(
        journal.phase,
        Phase::Prepared | Phase::BackedUp | Phase::Replaced
    ) {
        // Reaching startup mid-transaction means the attempt was interrupted, not
        // that the artifact is bad. The journal keeps the report; the release
        // stays selectable.
        rollback(&root, &mut journal, false)?;
    }
    supersede_stale(&root, &journal, running_release().as_ref())
}
/// A rollback the running bundle already meets or exceeds is history, not a
/// current problem: that release was installed later by other means. Reporting it
/// would keep an unusable error in front of the user, and its quarantine entry
/// would keep a release the user is already running out of every future offer.
fn supersede_stale(
    root: &Path,
    journal: &InstallJournal,
    installed: Option<&ReleaseVersion>,
) -> Result<()> {
    if journal.phase != Phase::RolledBack
        || !installed.is_some_and(|installed| supersedes(&journal.status.release, installed))
    {
        return Ok(());
    }
    let quarantined = root.join("quarantined.json");
    if quarantined.exists() {
        let mut releases: BTreeSet<Uuid> = read_json(&quarantined)?;
        if releases.remove(&journal.status.release.release_id) {
            write_json_atomic(&quarantined, &releases).map_err(|e| e.to_string())?;
        }
    }
    match fs::remove_file(root.join("install.json")) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}
/// True when the running release is the failed one or newer.
fn supersedes(failed: &ReleaseVersion, installed: &ReleaseVersion) -> bool {
    !failed.precedence(installed).is_gt()
}
fn running_release() -> Option<ReleaseVersion> {
    current_bundle()
        .and_then(|bundle| installed_release(&bundle))
        .ok()
}
/// # Errors
/// Returns a verification, storage, or platform operation failure.
pub fn self_test_status(id: &str) -> Result<UpdateStatus> {
    let root = update_root()?;
    let journal: InstallJournal = read_json(&root.join("install.json"))?;
    if journal.phase != Phase::Replaced
        || journal.status.transaction_id.to_string() != id
        || current_bundle()? != journal.target
    {
        return Err("启动自检事务无效".into());
    }
    let (stage, _, _) = paths(&root, &journal)?;
    verify_bundle(
        &journal.target,
        &journal.status.release,
        &fs::read_to_string(stage.join("requirement.txt")).map_err(|e| e.to_string())?,
    )?;
    Ok(journal.status)
}
/// # Errors
/// Returns a verification, storage, or platform operation failure.
pub fn acknowledge_self_test(status: &UpdateStatus) -> Result<()> {
    write_json_atomic(
        &update_root()?
            .join(format!("install-{}", status.transaction_id))
            .join("self-test.json"),
        status,
    )
    .map_err(|e| e.to_string())
}
/// # Errors
/// Returns a verification, storage, or platform operation failure.
pub fn completed_status() -> Result<Option<UpdateStatus>> {
    let root = update_root()?;
    if !root.join("install.json").exists() {
        return Ok(None);
    }
    let journal: InstallJournal = read_json(&root.join("install.json"))?;
    Ok((journal.phase == Phase::Committed).then_some(journal.status))
}

/// # Errors
/// Returns a storage or journal decoding failure.
pub fn previous_failure() -> Result<Option<String>> {
    let root = update_root()?;
    if !root.join("install.json").exists() {
        return Ok(None);
    }
    let journal: InstallJournal = read_json(&root.join("install.json"))?;
    Ok(unresolved_failure(&journal, running_release().as_ref()))
}
/// Only a rollback the running bundle does not already cover is a current problem.
fn unresolved_failure(
    journal: &InstallJournal,
    installed: Option<&ReleaseVersion>,
) -> Option<String> {
    if journal.phase != Phase::RolledBack {
        return None;
    }
    if installed.is_some_and(|installed| supersedes(&journal.status.release, installed)) {
        return None;
    }
    Some(
        journal
            .failure
            .clone()
            .unwrap_or_else(|| "上次更新中断，已保留旧版本".into()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(phase: Phase) -> (tempfile::TempDir, PathBuf, InstallJournal) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("state/updates");
        private_dir(&root).unwrap();
        let release = ReleaseVersion {
            version: semver::Version::new(0, 1, 0),
            build_number: 2,
            release_id: Uuid::new_v4(),
        };
        let status = UpdateStatus {
            transaction_id: Uuid::new_v4(),
            release,
            artifact: crate::ArtifactDescriptor {
                target: Target::MacosArm64,
                kind: crate::PackageKind::MacosAppZip,
                file_name: "DeskFuse-update.zip".into(),
                size: 4,
                sha256: "0".repeat(64),
                minimum_os: semver::Version::new(14, 0, 0),
            },
            phase: crate::UpdatePhase::Installing,
            received_bytes: 4,
            failure: None,
        };
        let journal = InstallJournal {
            status,
            target: dir.path().join("DeskFuse.app"),
            phase,
            parent_pid: 0,
            failure: Some("self-test timeout".into()),
        };
        let (stage, _, _) = paths(&root, &journal).unwrap();
        private_dir(&stage).unwrap();
        (dir, root, journal)
    }
    #[test]
    fn rollback_restores_bundle_database_and_wal_without_touching_identity() {
        let (_dir, root, mut journal) = fixture(Phase::Replaced);
        let (stage, previous, _) = paths(&root, &journal).unwrap();
        fs::create_dir(&previous).unwrap();
        fs::write(previous.join("version"), b"old").unwrap();
        fs::create_dir(&journal.target).unwrap();
        fs::write(journal.target.join("version"), b"new").unwrap();
        let db = database_paths(&root).unwrap();
        fs::write(&db[0], b"old schema").unwrap();
        fs::write(&db[1], b"old WAL").unwrap();
        let identity = root.parent().unwrap().join("identity.key");
        fs::write(&identity, b"persistent identity").unwrap();
        backup_database(&root, &stage).unwrap();
        fs::write(&db[0], b"new schema").unwrap();
        fs::write(&db[1], b"new WAL").unwrap();
        fs::write(&db[2], b"new SHM").unwrap();
        rollback(&root, &mut journal, true).unwrap();
        assert_eq!(fs::read(journal.target.join("version")).unwrap(), b"old");
        assert_eq!(fs::read(&db[0]).unwrap(), b"old schema");
        assert_eq!(fs::read(&db[1]).unwrap(), b"old WAL");
        assert!(!db[2].exists());
        assert_eq!(fs::read(identity).unwrap(), b"persistent identity");
        let isolated: BTreeSet<Uuid> = read_json(&root.join("quarantined.json")).unwrap();
        assert!(isolated.contains(&journal.status.release.release_id));
        assert_eq!(journal.phase, Phase::RolledBack);
    }
    #[test]
    fn crash_after_first_rename_recovers_even_before_phase_was_flushed() {
        let (_dir, root, mut journal) = fixture(Phase::Prepared);
        let (_, previous, _) = paths(&root, &journal).unwrap();
        fs::create_dir(&previous).unwrap();
        fs::write(previous.join("version"), b"old").unwrap();
        rollback(&root, &mut journal, false).unwrap();
        assert_eq!(fs::read(journal.target.join("version")).unwrap(), b"old");
    }
    #[test]
    fn staging_failure_preserves_original_and_database() {
        let (_dir, root, mut journal) = fixture(Phase::Prepared);
        fs::create_dir(&journal.target).unwrap();
        fs::write(journal.target.join("version"), b"old").unwrap();
        let db = database_paths(&root).unwrap();
        fs::write(&db[0], b"original").unwrap();
        rollback(&root, &mut journal, false).unwrap();
        assert_eq!(fs::read(journal.target.join("version")).unwrap(), b"old");
        assert_eq!(fs::read(&db[0]).unwrap(), b"original");
    }
    #[test]
    fn unsuccessful_rollback_keeps_recovery_backup() {
        let (_dir, root, mut journal) = fixture(Phase::Replaced);
        let (_, previous, _) = paths(&root, &journal).unwrap();
        fs::create_dir(&previous).unwrap();
        fs::write(&journal.target, b"unexpected file").unwrap();
        assert!(rollback(&root, &mut journal, true).is_err());
        assert!(previous.is_dir());
        assert_eq!(journal.phase, Phase::Replaced);
    }
    fn release(build: u64) -> ReleaseVersion {
        ReleaseVersion {
            version: semver::Version::new(0, 1, 0),
            build_number: build,
            release_id: Uuid::new_v4(),
        }
    }
    #[test]
    fn staged_payload_requires_exactly_one_source() {
        let (_dir, _root, journal) = fixture(Phase::Prepared);
        let (stage, _, _) = paths(&_root, &journal).unwrap();
        assert!(verify_staged_payload(&stage, &journal).is_err());
        fs::write(stage.join("manifest.json"), b"{}").unwrap();
        fs::write(
            stage.join("github-source.json"),
            br#"{"sha256":"0","file_name":"DeskFuse-arm64.dmg"}"#,
        )
        .unwrap();
        assert!(verify_staged_payload(&stage, &journal).is_err());
    }
    #[test]
    fn interrupted_rollback_keeps_the_release_selectable() {
        let (_dir, root, mut journal) = fixture(Phase::Prepared);
        rollback(&root, &mut journal, false).unwrap();
        assert_eq!(journal.phase, Phase::RolledBack);
        assert_eq!(journal.status.phase, crate::UpdatePhase::Cancelled);
        assert!(!root.join("quarantined.json").exists());
    }
    #[test]
    fn rollback_of_a_failed_artifact_stays_quarantined() {
        let (_dir, root, mut journal) = fixture(Phase::Prepared);
        rollback(&root, &mut journal, true).unwrap();
        assert_eq!(journal.status.phase, crate::UpdatePhase::Quarantined);
        let isolated: BTreeSet<Uuid> = read_json(&root.join("quarantined.json")).unwrap();
        assert!(isolated.contains(&journal.status.release.release_id));
    }
    #[test]
    fn rollback_the_installed_build_already_covers_is_cleared() {
        let (_dir, root, journal) = fixture(Phase::RolledBack);
        persist(&root, &journal).unwrap();
        let installed = journal.status.release.clone();
        fs::write(
            root.join("quarantined.json"),
            serde_json::to_vec(&BTreeSet::from([installed.release_id])).unwrap(),
        )
        .unwrap();
        supersede_stale(&root, &journal, Some(&installed)).unwrap();
        assert!(!root.join("install.json").exists());
        let isolated: BTreeSet<Uuid> = read_json(&root.join("quarantined.json")).unwrap();
        assert!(isolated.is_empty());
        assert_eq!(unresolved_failure(&journal, Some(&installed)), None);
    }
    #[test]
    fn rollback_newer_than_the_installed_build_stays_visible() {
        let (_dir, root, mut journal) = fixture(Phase::RolledBack);
        journal.status.release = release(9);
        persist(&root, &journal).unwrap();
        let installed = release(8);
        supersede_stale(&root, &journal, Some(&installed)).unwrap();
        assert!(root.join("install.json").exists());
        assert_eq!(
            unresolved_failure(&journal, Some(&installed)).as_deref(),
            Some("self-test timeout")
        );
    }
    #[test]
    fn failure_is_still_reported_without_a_readable_bundle() {
        let (_dir, _root, journal) = fixture(Phase::RolledBack);
        assert_eq!(
            unresolved_failure(&journal, None).as_deref(),
            Some("self-test timeout")
        );
        let mut committed = journal;
        committed.phase = Phase::Committed;
        assert_eq!(unresolved_failure(&committed, None), None);
    }
    /// The journal shape an interrupted update leaves behind on a real machine:
    /// the release it failed on was installed later by hand, so nothing about it
    /// is a current problem.
    #[test]
    fn journal_from_a_superseded_real_install_is_parsed_and_cleared() {
        let (_dir, root, _) = fixture(Phase::RolledBack);
        let release_id = Uuid::new_v4();
        let journal: InstallJournal = serde_json::from_str(&format!(
            r#"{{"status":{{"transaction_id":"{transaction}","release":{{"version":"0.1.0","build_number":72,"release_id":"{release_id}"}},"artifact":{{"target":"macos_arm64","kind":"macos_app_zip","file_name":"DeskFuse-arm64-update.zip","size":9756431,"sha256":"{sha}","minimum_os":"14.0.0"}},"phase":"quarantined","received_bytes":9756431,"failure":null}},"target":"/Applications/DeskFuse.app","phase":"RolledBack","parent_pid":62122,"failure":null}}"#,
            transaction = Uuid::new_v4(),
            sha = "e".repeat(64),
        ))
        .unwrap();
        persist(&root, &journal).unwrap();
        fs::write(
            root.join("quarantined.json"),
            serde_json::to_vec(&BTreeSet::from([release_id])).unwrap(),
        )
        .unwrap();
        let installed = ReleaseVersion {
            version: semver::Version::new(0, 1, 0),
            build_number: 72,
            release_id,
        };
        assert_eq!(unresolved_failure(&journal, Some(&installed)), None);
        supersede_stale(&root, &journal, Some(&installed)).unwrap();
        assert!(!root.join("install.json").exists());
        let isolated: BTreeSet<Uuid> = read_json(&root.join("quarantined.json")).unwrap();
        assert!(isolated.is_empty());
    }
}
