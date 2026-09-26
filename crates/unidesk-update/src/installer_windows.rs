//! Machine-owned Windows replacement helper.
//!
//! The desktop process never receives machine write access.  It writes a signed
//! request into the update cache and launches this executable with `runas`.
//! This helper derives every installation/data path locally, closes the GUI and
//! input helper, quiesces SCM, and snapshots SQLite and its sidecars.  The
//! current signed artifact is NSIS-first-install only, so execution stops
//! before any replacement until a signed transactional payload contract exists.
//! A missing proof at any point is an error; there is deliberately no
//! best-effort update path.
use crate::{
    SignedReleaseManifest, Target, UpdateError, UpdatePhase, UpdateStatus, WindowsSelfTest,
    write_json_atomic,
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

const SERVICE_NAME: &str = "UniDeskService";
const INSTALL_NAME: &str = "uniDesk";
const UPDATE_DIR_NAME: &str = "updates";
const SERVICE_DATA_RELATIVE: &str = "com.unidesk.app\\service";
const MAX_REQUEST_BYTES: u64 = 128 * 1024;
const SERVICE_TIMEOUT: Duration = Duration::from_secs(30);
const PROCESS_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowsMachineRequest {
    pub transaction_id: Uuid,
    pub signed_release: SignedReleaseManifest,
    /// Must be a regular file below the machine-owned update cache.
    pub payload: PathBuf,
    /// The GUI and helper identify the exact processes they asked us to stop.
    pub gui_pid: Option<u32>,
    pub input_helper_pid: Option<u32>,
}

/// Start the external helper.  No arguments is not a valid update request: it
/// is important that a double-click, a stale shortcut, or an untrusted caller
/// cannot turn this binary into an installer.
pub fn run_helper() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let Some(flag) = args.next() else {
        return Err("Windows machine updater requires a signed --request file".into());
    };
    if flag != "--request" {
        return Err("Windows machine updater accepts only --request".into());
    }
    let Some(request_path) = args.next().map(PathBuf::from) else {
        return Err("Windows machine updater request is missing".into());
    };
    if args.next().is_some() {
        return Err("Windows machine updater received unexpected arguments".into());
    }
    let request = read_request(&request_path)?;
    if !is_elevated() {
        return relaunch_elevated(&request_path);
    }
    run_machine_update(request).map_err(|error| error.to_string())
}

/// Request a UAC broker without accepting an executable or destination from a
/// caller.  The request itself is checked to be under ProgramData before this
/// function is called; PowerShell is invoked directly, never through a shell.
pub fn relaunch_elevated(request: &Path) -> Result<(), String> {
    if !request.is_absolute() {
        return Err("更新请求必须是绝对路径".into());
    }
    let root = update_root()?;
    let canonical = fs::canonicalize(request).map_err(|_| "更新请求路径无效")?;
    if !canonical.starts_with(&root) {
        return Err("更新请求不在机器更新目录内".into());
    }
    let helper = std::env::current_exe().map_err(|_| "无法定位更新 helper")?;
    let escaped_helper = powershell_literal(&helper);
    let escaped_request = powershell_literal(request);
    let script = format!(
        "Start-Process -FilePath '{escaped_helper}' -ArgumentList @('--request','{escaped_request}') -Verb RunAs -Wait"
    );
    let result = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Restricted",
            "-Command",
            &script,
        ])
        .status()
        .map_err(|_| "无法请求 UAC 管理员授权")?;
    if result.success() {
        Ok(())
    } else {
        Err("用户未批准 UAC 授权，更新未执行".into())
    }
}

fn powershell_literal(path: &Path) -> String {
    path.to_string_lossy().replace('\'', "''")
}

fn update_root() -> Result<PathBuf, String> {
    let base = PathBuf::from(
        std::env::var_os("ProgramData").ok_or("Windows machine data path unavailable")?,
    );
    if !base.is_absolute()
        || base
            .components()
            .any(|part| part == std::path::Component::ParentDir)
    {
        return Err("Windows machine data path is invalid".into());
    }
    Ok(base.join("com.unidesk.app").join(UPDATE_DIR_NAME))
}

fn read_request(path: &Path) -> Result<WindowsMachineRequest, String> {
    let metadata = path
        .symlink_metadata()
        .map_err(|_| "更新请求不存在或不可读")?;
    if !metadata.is_file() || metadata.len() > MAX_REQUEST_BYTES {
        return Err("更新请求大小或类型无效".into());
    }
    let root = update_root()?;
    let request = fs::canonicalize(path).map_err(|_| "更新请求路径无效")?;
    if !request.starts_with(&root) {
        return Err("更新请求不在机器更新目录内".into());
    }
    let mut file = File::open(request).map_err(|_| "更新请求不可读")?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|_| "更新请求读取失败")?;
    serde_json::from_slice(&bytes).map_err(|_| "更新请求格式无效")
}

fn is_elevated() -> bool {
    // `fltmc` is a Windows-provided, non-localized administrator probe.  It
    // does not mutate machine state and avoids duplicating token FFI in a crate
    // that forbids unsafe code.
    Command::new("fltmc.exe")
        .arg("instances")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn run_machine_update(request: WindowsMachineRequest) -> Result<(), UpdateError> {
    if request.transaction_id.is_nil() {
        return Err(UpdateError::Invalid("nil Windows transaction"));
    }
    let release = request
        .signed_release
        .verify(&crate::pinned_release_key()?)?;
    let descriptor = release
        .artifact(Target::WindowsX64)
        .ok_or(UpdateError::Incompatible)?;
    if descriptor.file_name
        != request
            .payload
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
        || descriptor.kind != crate::PackageKind::WindowsNsisExe
    {
        return Err(UpdateError::Incompatible);
    }

    let paths = MachinePaths::derive(request.transaction_id)?;
    paths.prepare()?;
    let payload = fs::canonicalize(&request.payload).map_err(UpdateError::Io)?;
    if !payload.starts_with(&paths.update_root) {
        return Err(UpdateError::Invalid("payload outside machine update cache"));
    }
    crate::verify_artifact(&release, Target::WindowsX64, &payload)?;
    // The currently published Windows artifact is the first-install NSIS
    // package.  No signed transactional payload/activation contract exists
    // yet, so never turn this artifact into an updater by inference.
    if descriptor.kind == crate::PackageKind::WindowsNsisExe {
        return Err(UpdateError::Invalid(
            "transactional Windows payload unavailable; NSIS is first-install only",
        ));
    }

    let mut admission = MachineAdmission::acquire(&paths)?;
    admission.persist("admission_closed")?;
    coordinate_desktop(&request)?;
    let service = ServiceController::open()?;
    service.stop()?;
    admission.persist("service_stopped")?;

    let database = DatabaseBackup::capture(&paths.database, &paths.database_backup)?;
    let _installation_backup =
        move_installation_to_backup(&paths.installation, &paths.installation_backup)?;
    admission.persist("backed_up")?;
    let journal = MachineJournal::new(&request, &paths, &release)?;
    journal.persist(&paths.journal)?;

    let install_result = install_signed_package(&payload);
    if install_result.is_err() {
        restore_after_failure(&paths, &service, &database, &journal)?;
        return install_result;
    }
    service.start()?;
    journal.persist_phase(&paths.journal, MachinePhase::AwaitingSelfTest)?;
    let evidence = startup_self_test(&request, &paths, &release)?;
    if !evidence.service_ready
        || !evidence.desktop_ready
        || !evidence.database_ready
        || !evidence.input_helper_ready
        || !evidence.business_admission_closed
    {
        restore_after_failure(&paths, &service, &database, &journal)?;
        return Err(UpdateError::Invalid("Windows startup self-test failed"));
    }
    journal.persist_phase(&paths.journal, MachinePhase::Committed)?;
    fs::remove_dir_all(&paths.installation_backup).map_err(UpdateError::Io)?;
    database.remove_backup()?;
    admission.persist("committed")?;
    // Keep the journal as durable local evidence; a later cleanup pass may
    // remove only committed records after the remote receipt is delivered.
    Ok(())
}

fn coordinate_desktop(request: &WindowsMachineRequest) -> Result<(), UpdateError> {
    // The GUI owns the helper. Killing the GUI's process tree therefore also
    // releases native input hooks and held keys/buttons. Exact image names are
    // checked before taskkill to avoid PID-reuse surprises.
    if let Some(pid) = request.input_helper_pid {
        stop_process(pid, "unidesk-input-helper.exe")?;
    }
    if let Some(pid) = request.gui_pid {
        stop_process(pid, "uniDesk.exe")?;
    }
    Ok(())
}

fn stop_process(pid: u32, image: &str) -> Result<(), UpdateError> {
    if pid == 0 || !process_has_image(pid, image) {
        return Err(UpdateError::Invalid("desktop process identity changed"));
    }
    let pid_arg = pid.to_string();
    let status = Command::new("taskkill.exe")
        .args(["/PID", &pid_arg, "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(UpdateError::Io)?;
    if !status.success() {
        return Err(UpdateError::Invalid("desktop process did not stop"));
    }
    let deadline = Instant::now() + PROCESS_TIMEOUT;
    while Instant::now() < deadline {
        if !process_has_image(pid, image) {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err(UpdateError::Invalid("desktop process stop timed out"))
}

fn process_has_image(pid: u32, image: &str) -> bool {
    let filter = format!("PID eq {pid}");
    let output = Command::new("tasklist.exe")
        .args(["/FI", &filter, "/FO", "CSV", "/NH"])
        .output();
    let Ok(output) = output else { return false };
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines().any(|line| {
        line.split(',')
            .next()
            .is_some_and(|name| name.trim_matches('"').eq_ignore_ascii_case(image))
    })
}

fn install_signed_package(_payload: &Path) -> Result<(), UpdateError> {
    // Deliberately unreachable for the current WindowsNsisExe artifact.  Keep
    // the execution seam explicit so a future signed transactional package can
    // implement it without silently broadening this helper's authority.
    Err(UpdateError::Invalid(
        "transactional Windows payload unavailable; NSIS is first-install only",
    ))
}

fn move_installation_to_backup(installation: &Path, backup: &Path) -> Result<bool, UpdateError> {
    if !installation.exists() {
        return Err(UpdateError::Invalid(
            "installed uniDesk directory is missing",
        ));
    }
    if backup.exists() {
        return Err(UpdateError::Invalid("installation backup already exists"));
    }
    fs::rename(installation, backup).map_err(UpdateError::Io)?;
    Ok(true)
}

fn restore_after_failure(
    paths: &MachinePaths,
    service: &ServiceController,
    database: &DatabaseBackup,
    journal: &MachineJournal,
) -> Result<(), UpdateError> {
    let _ = service.stop();
    let _ = fs::remove_dir_all(&paths.installation);
    if paths.installation_backup.exists() {
        fs::rename(&paths.installation_backup, &paths.installation).map_err(UpdateError::Io)?;
    }
    database.restore()?;
    service.start()?;
    journal.persist_phase(&paths.journal, MachinePhase::RolledBack)?;
    Ok(())
}

fn startup_self_test(
    request: &WindowsMachineRequest,
    paths: &MachinePaths,
    release: &crate::VerifiedRelease,
) -> Result<WindowsSelfTest, UpdateError> {
    let service_ready = ServiceController::open()?.is_running()?;
    let desktop_ready = paths.installation.join("uniDesk.exe").is_file();
    let input_helper_ready = paths
        .installation
        .join("unidesk-input-helper.exe")
        .is_file();
    let database_ready = sqlite_header_valid(&paths.database)?;
    // The helper deliberately does not open business admission itself.  The
    // signed self-test evidence is persisted while admission is still closed;
    // the GUI/service coordinator must consume this record before resuming.
    let evidence = WindowsSelfTest {
        transaction_id: request.transaction_id,
        release: release.manifest().release.clone(),
        artifact: release
            .artifact(Target::WindowsX64)
            .ok_or(UpdateError::Incompatible)?
            .clone(),
        service_ready,
        desktop_ready,
        database_ready,
        input_helper_ready,
        business_admission_closed: true,
    };
    write_json_atomic(&paths.self_test, &evidence)?;
    Ok(evidence)
}

fn sqlite_header_valid(path: &Path) -> Result<bool, UpdateError> {
    let mut file = File::open(path).map_err(UpdateError::Io)?;
    let mut header = [0u8; 16];
    file.read_exact(&mut header).map_err(UpdateError::Io)?;
    Ok(&header == b"SQLite format 3\0")
}

struct MachinePaths {
    update_root: PathBuf,
    installation: PathBuf,
    installation_backup: PathBuf,
    database: PathBuf,
    database_backup: PathBuf,
    journal: PathBuf,
    self_test: PathBuf,
}
impl MachinePaths {
    fn derive(transaction: Uuid) -> Result<Self, UpdateError> {
        let program_data = absolute_env("ProgramData")?;
        let program_files = absolute_env("ProgramFiles")?;
        let update_root = program_data.join("com.unidesk.app").join(UPDATE_DIR_NAME);
        let installation = program_files.join(INSTALL_NAME);
        let service_data = program_data.join(SERVICE_DATA_RELATIVE);
        Ok(Self {
            update_root: update_root.clone(),
            installation: installation.clone(),
            installation_backup: update_root.join(format!("installation-{transaction}.bak")),
            database: service_data.join("network.sqlite"),
            database_backup: update_root.join(format!("database-{transaction}.bak")),
            journal: update_root.join(format!("transaction-{transaction}.json")),
            self_test: update_root.join(format!("self-test-{transaction}.json")),
        })
    }
    fn prepare(&self) -> Result<(), UpdateError> {
        fs::create_dir_all(&self.update_root).map_err(UpdateError::Io)?;
        for path in [&self.update_root, &self.installation, &self.database] {
            if let Ok(metadata) = path.symlink_metadata() {
                if metadata.file_type().is_symlink() {
                    return Err(UpdateError::Invalid(
                        "machine updater refuses reparse paths",
                    ));
                }
            }
        }
        harden_directory(&self.update_root)
    }
}

fn absolute_env(name: &str) -> Result<PathBuf, UpdateError> {
    let path = PathBuf::from(
        std::env::var_os(name).ok_or(UpdateError::Invalid("Windows path unavailable"))?,
    );
    if !path.is_absolute()
        || path
            .components()
            .any(|part| part == std::path::Component::ParentDir)
    {
        return Err(UpdateError::Invalid("Windows machine path is invalid"));
    }
    Ok(path)
}

fn harden_directory(path: &Path) -> Result<(), UpdateError> {
    let status = Command::new("icacls.exe")
        .arg(path)
        .args([
            "/inheritance:r",
            "/grant:r",
            "SYSTEM:(OI)(CI)(F)",
            "Administrators:(OI)(CI)(F)",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(UpdateError::Io)?;
    if status.success() {
        Ok(())
    } else {
        Err(UpdateError::Invalid("machine update ACL is unsafe"))
    }
}

struct MachineAdmission {
    file: File,
}
impl MachineAdmission {
    fn acquire(paths: &MachinePaths) -> Result<Self, UpdateError> {
        let marker = paths.update_root.join("admission.json");
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&marker)
            .map_err(UpdateError::Io)?;
        file.lock().map_err(UpdateError::Io)?;
        Ok(Self { file })
    }
    fn persist(&mut self, phase: &str) -> Result<(), UpdateError> {
        self.file.set_len(0).map_err(UpdateError::Io)?;
        self.file
            .write_all(phase.as_bytes())
            .map_err(UpdateError::Io)?;
        self.file.sync_all().map_err(UpdateError::Io)
    }
}

struct DatabaseBackup {
    database: PathBuf,
    main: PathBuf,
    wal: Option<PathBuf>,
    shm: Option<PathBuf>,
}
impl DatabaseBackup {
    fn capture(database: &Path, destination: &Path) -> Result<Self, UpdateError> {
        fs::create_dir_all(destination).map_err(UpdateError::Io)?;
        let main = destination.join("network.sqlite");
        copy_flush(database, &main)?;
        let wal = copy_sidecar(
            &database.with_extension("sqlite-wal"),
            &destination.join("network.sqlite-wal"),
        )?;
        let shm = copy_sidecar(
            &database.with_extension("sqlite-shm"),
            &destination.join("network.sqlite-shm"),
        )?;
        Ok(Self {
            database: database.to_owned(),
            main,
            wal,
            shm,
        })
    }
    fn restore(&self) -> Result<(), UpdateError> {
        copy_flush(&self.main, &self.database)?;
        restore_sidecar(&self.wal, &self.database.with_extension("sqlite-wal"))?;
        restore_sidecar(&self.shm, &self.database.with_extension("sqlite-shm"))
    }
    fn remove_backup(&self) -> Result<(), UpdateError> {
        if let Some(parent) = self.main.parent() {
            fs::remove_dir_all(parent).map_err(UpdateError::Io)?;
        }
        Ok(())
    }
}
fn copy_sidecar(source: &Path, destination: &Path) -> Result<Option<PathBuf>, UpdateError> {
    if source.exists() {
        copy_flush(source, destination)?;
        Ok(Some(destination.to_owned()))
    } else {
        Ok(None)
    }
}
fn restore_sidecar(source: &Option<PathBuf>, destination: &Path) -> Result<(), UpdateError> {
    if destination.exists() {
        fs::remove_file(destination).map_err(UpdateError::Io)?;
    }
    if let Some(source) = source {
        copy_flush(source, destination)?;
    }
    Ok(())
}
fn copy_flush(source: &Path, destination: &Path) -> Result<(), UpdateError> {
    let metadata = source.symlink_metadata().map_err(UpdateError::Io)?;
    if !metadata.is_file() {
        return Err(UpdateError::Invalid("database backup source is not a file"));
    }
    let mut input = File::open(source).map_err(UpdateError::Io)?;
    let mut output = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(destination)
        .map_err(UpdateError::Io)?;
    std::io::copy(&mut input, &mut output).map_err(UpdateError::Io)?;
    output.sync_all().map_err(UpdateError::Io)
}

struct ServiceController;
impl ServiceController {
    fn open() -> Result<Self, UpdateError> {
        let output = sc(&["query", SERVICE_NAME])?;
        if !output.status.success() {
            return Err(UpdateError::Invalid("UniDeskService is not installed"));
        }
        Ok(Self)
    }
    fn is_running(&self) -> Result<bool, UpdateError> {
        Ok(service_state(&sc(&["query", SERVICE_NAME])?)? == 4)
    }
    fn stop(&self) -> Result<(), UpdateError> {
        if !self.is_running()? {
            return Ok(());
        }
        let _ = sc(&["stop", SERVICE_NAME])?;
        wait_service(self, 1)
    }
    fn start(&self) -> Result<(), UpdateError> {
        if self.is_running()? {
            return Ok(());
        }
        let output = sc(&["start", SERVICE_NAME])?;
        if !output.status.success()
            && !String::from_utf8_lossy(&output.stdout).contains("already running")
        {
            return Err(UpdateError::Invalid("UniDeskService failed to start"));
        }
        wait_service(self, 4)
    }
}
fn sc(args: &[&str]) -> Result<std::process::Output, UpdateError> {
    Command::new("sc.exe")
        .args(args)
        .output()
        .map_err(UpdateError::Io)
}
fn service_state(output: &std::process::Output) -> Result<u32, UpdateError> {
    let text = String::from_utf8_lossy(&output.stdout);
    let state = text.lines().find_map(|line| {
        line.split_once(':')
            .and_then(|(_, value)| value.trim().split_whitespace().next())
            .and_then(|value| value.parse().ok())
    });
    state.ok_or(UpdateError::Invalid("SCM returned no service state"))
}
fn wait_service(service: &ServiceController, wanted: u32) -> Result<(), UpdateError> {
    let deadline = Instant::now() + SERVICE_TIMEOUT;
    while Instant::now() < deadline {
        if service_state(&sc(&["query", SERVICE_NAME])?)? == wanted {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(250));
    }
    Err(UpdateError::Invalid(
        "SCM service state transition timed out",
    ))
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum MachinePhase {
    AdmissionClosed,
    BackedUp,
    AwaitingSelfTest,
    Committed,
    RolledBack,
}
#[derive(Serialize)]
struct MachineJournal {
    transaction_id: Uuid,
    status: UpdateStatus,
    database_backup: PathBuf,
    installation_backup: PathBuf,
    phase: MachinePhase,
}
impl MachineJournal {
    fn new(
        request: &WindowsMachineRequest,
        paths: &MachinePaths,
        release: &crate::VerifiedRelease,
    ) -> Result<Self, UpdateError> {
        let artifact = release
            .artifact(Target::WindowsX64)
            .ok_or(UpdateError::Incompatible)?
            .clone();
        Ok(Self {
            transaction_id: request.transaction_id,
            status: UpdateStatus {
                transaction_id: request.transaction_id,
                release: release.manifest().release.clone(),
                artifact: artifact.clone(),
                phase: UpdatePhase::Installing,
                received_bytes: artifact.size,
                failure: None,
            },
            database_backup: paths.database_backup.clone(),
            installation_backup: paths.installation_backup.clone(),
            phase: MachinePhase::BackedUp,
        })
    }
    fn persist(&self, path: &Path) -> Result<(), UpdateError> {
        write_json_atomic(path, self)
    }
    fn persist_phase(&self, path: &Path, phase: MachinePhase) -> Result<(), UpdateError> {
        let next = Self {
            transaction_id: self.transaction_id,
            status: self.status.clone(),
            database_backup: self.database_backup.clone(),
            installation_backup: self.installation_backup.clone(),
            phase,
        };
        next.persist(path)
    }
}
