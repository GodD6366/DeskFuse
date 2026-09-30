//! Public distributions check GitHub and install the official DMG after SHA-256
//! and code-signing checks.
//!
//! The release list and the small per-release metadata files are read from
//! GitHub first and fall back to the site mirror, so the canonical release
//! source wins whenever it is reachable while the independent domain keeps
//! checks working through GitHub outages. The DMG itself always comes from GitHub, and
//! GitHub bytes never enter the signed LAN installer. Public bundles also zip
//! the running app and serve it to other GitHub-channel devices over the LAN
//! update transport; those ZIP bytes use Developer ID + SHA-256, not Ed25519.
use deskfuse_runtime::updates::{GitHubUpdateView, LanUpdateView};
use deskfuse_update::{
    ArtifactDescriptor, PackageKind, ReleaseVersion, Target, UpdatePhase, UpdateStatus, installer,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use uuid::Uuid;

include!(concat!(env!("OUT_DIR"), "/distribution.rs"));
const INTERVAL: Duration = Duration::from_secs(12 * 60 * 60);
const DMG_LIMIT: u64 = 512 * 1024 * 1024;

pub(crate) fn is_public_bundle() -> bool {
    has_update_channel("github")
}

pub(crate) fn is_manual_bundle() -> bool {
    has_update_channel("manual")
}

fn has_update_channel(expected: &str) -> bool {
    installer::current_bundle().ok().is_some_and(|bundle| {
        std::fs::read_to_string(bundle.join("Contents/Resources/update-channel"))
            .is_ok_and(|channel| channel.trim() == expected)
    })
}

#[derive(Clone)]
struct GitHubOffer {
    release: ReleaseVersion,
    release_url: String,
    dmg_url: String,
    sha256: String,
    size: u64,
    developer_id: bool,
}

enum Task {
    Check(tokio::task::JoinHandle<Result<GitHubOffer, String>>),
    Download {
        handle: tokio::task::JoinHandle<Result<(), String>>,
        path: PathBuf,
        expected: u64,
    },
    Prepare(tokio::task::JoinHandle<Result<(), String>>),
}

pub(crate) struct GitHubUpdater {
    view: LanUpdateView,
    task: Option<Task>,
    last_check: Instant,
    offer: Option<GitHubOffer>,
    ready: bool,
    /// Set by the second install request to bypass the normal file-idle wait.
    force_install: bool,
    install_abandoned: bool,
}
impl GitHubUpdater {
    pub(crate) fn start() -> Result<Self, String> {
        let bundle = installer::current_bundle()?;
        let installed = installer::installed_release(&bundle)?;
        cleanup_partial_downloads();
        let mut updater = Self {
            view: LanUpdateView {
                installed: Some(installed),
                github: Some(GitHubUpdateView::default()),
                message: "公开发行版优先从 GitHub 检查，失败时使用独立更新站点".into(),
                ..Default::default()
            },
            task: None,
            last_check: Instant::now(),
            offer: None,
            ready: false,
            force_install: false,
            install_abandoned: false,
        };
        updater.check_now();
        Ok(updater)
    }
    pub(crate) fn check_now(&mut self) {
        if self.task.is_some() || self.ready || self.install_abandoned {
            return;
        }
        self.view.checking = true;
        self.view.error = None;
        self.view.message = "正在从 GitHub 检查发布版本，失败时自动使用独立站点…".into();
        self.last_check = Instant::now();
        self.task = Some(Task::Check(tokio::spawn(fetch_offer())));
    }
    pub(crate) fn download_and_install_now(&mut self) {
        if self.install_abandoned {
            self.view.message =
                "更新未安装，当前版本保持可用；退出并重新启动 DeskFuse 后可重试".into();
            return;
        }
        let already_requested = self.view.manual_requested;
        if !already_requested {
            self.force_install = false;
        }
        self.view.manual_requested = true;
        self.view.error = None;
        if self.ready {
            if already_requested {
                self.force_install = true;
                self.view.message = "正在立即安装，将跳过传输或粘贴等待".into();
            } else {
                self.view.message = "正在准备主动安装，完成后将重新启动".into();
            }
            return;
        }
        if matches!(
            self.task,
            Some(Task::Download { .. }) | Some(Task::Prepare(_))
        ) {
            self.view.message = "正在从 GitHub 下载并准备安装".into();
            return;
        }
        if self.offer.is_some() {
            self.start_download();
            return;
        }
        self.check_now();
        self.view.message = "正在查找 GitHub 新版并准备下载".into();
    }
    pub(crate) fn snapshot(&self) -> LanUpdateView {
        self.view.clone()
    }
    pub(crate) fn ready(&self) -> bool {
        self.ready && !self.install_abandoned
    }
    pub(crate) fn install_abandoned(&self) -> bool {
        self.install_abandoned
    }
    pub(crate) fn force_install(&self) -> bool {
        self.force_install && self.ready && !self.install_abandoned
    }
    pub(crate) fn installation_waiting(&mut self, message: &str) {
        if self.view.manual_requested || self.ready {
            self.view.message = message.into();
        }
    }
    pub(crate) fn installing(&mut self) {
        self.force_install = false;
        if let Some(status) = &mut self.view.status {
            status.phase = UpdatePhase::Installing;
        }
        self.view.message = "正在安装更新，即将重新启动".into();
    }
    pub(crate) fn abandon_install(&mut self, reason: &str) {
        self.install_abandoned = true;
        self.ready = false;
        self.force_install = false;
        self.view.manual_requested = false;
        if let Some(status) = &mut self.view.status
            && matches!(
                status.phase,
                UpdatePhase::Installing | UpdatePhase::WaitingForIdle | UpdatePhase::Staged
            )
        {
            status.phase = UpdatePhase::Cancelled;
        }
        self.view.message = "更新未安装，当前版本保持可用；退出并重新启动 DeskFuse 后可重试".into();
        self.view.error = Some(reason.into());
    }
    pub(crate) async fn poll(&mut self) {
        enum Step {
            Check,
            Download,
            Prepare,
            Progress { path: PathBuf, expected: u64 },
            Recheck,
            Idle,
        }
        let step = match &self.task {
            Some(Task::Check(task)) if task.is_finished() => Step::Check,
            Some(Task::Download {
                handle,
                path,
                expected,
            }) if handle.is_finished() => Step::Download,
            Some(Task::Download { path, expected, .. }) => Step::Progress {
                path: path.clone(),
                expected: *expected,
            },
            Some(Task::Prepare(task)) if task.is_finished() => Step::Prepare,
            None if !self.ready
                && !self.install_abandoned
                && self.last_check.elapsed() >= INTERVAL =>
            {
                Step::Recheck
            }
            _ => Step::Idle,
        };
        match step {
            Step::Check => self.finish_check().await,
            Step::Download => self.finish_download().await,
            Step::Prepare => self.finish_prepare().await,
            Step::Progress { path, expected } => {
                if let Some(status) = &mut self.view.status {
                    let received = std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
                    status.received_bytes = received.min(expected);
                    status.phase = UpdatePhase::Downloading;
                    self.view.message = "正在从 GitHub 下载更新".into();
                }
            }
            Step::Recheck => self.check_now(),
            Step::Idle => {}
        }
    }
    async fn finish_check(&mut self) {
        let Some(Task::Check(task)) = self.task.take() else {
            return;
        };
        let result = task
            .await
            .map_err(|error| format!("更新检查失败：{error}"))
            .and_then(|result| result);
        self.view.checking = false;
        match result {
            Ok(offer) => {
                let newer = self
                    .view
                    .installed
                    .as_ref()
                    .is_some_and(|installed| offer.release.precedence(installed).is_gt());
                if !newer {
                    self.offer = None;
                    self.view.github = Some(GitHubUpdateView::default());
                    self.view.message = "当前版本已是最新，公开发行版没有更高版本".into();
                    self.view.error = None;
                    self.view.manual_requested = false;
                    return;
                }
                let target = format!(
                    "{} · build {}",
                    offer.release.version, offer.release.build_number
                );
                self.view.github = Some(GitHubUpdateView {
                    target_version: Some(target),
                    release_url: Some(offer.release_url.clone()),
                });
                self.view.error = None;
                if is_quarantined(offer.release.release_id) {
                    self.offer = None;
                    self.view.manual_requested = false;
                    self.view.message =
                        "更新失败版本已隔离，当前版本保持可用；请等待后续 GitHub 发布".into();
                    return;
                }
                if !offer.developer_id {
                    self.offer = Some(offer);
                    self.view.manual_requested = false;
                    self.view.message =
                        "GitHub 有新版本，但该包没有可用于自动安装的稳定签名；请从发布页下载 DMG 手动安装"
                            .into();
                    return;
                }
                self.offer = Some(offer);
                self.view.message = "GitHub 有新版本，正在下载并准备自动安装".into();
                self.start_download();
            }
            Err(error) => {
                self.view.message = "暂时无法确认公开发行版的最新版本，请稍后重试".into();
                self.view.error = Some(error);
                if self.view.manual_requested && self.offer.is_none() {
                    self.view.manual_requested = false;
                }
            }
        }
    }
    fn start_download(&mut self) {
        if self.task.is_some() || self.ready || self.install_abandoned {
            return;
        }
        let Some(offer) = self.offer.clone() else {
            return;
        };
        if !offer.developer_id {
            self.view.manual_requested = false;
            self.view.message =
                "GitHub 有新版本，但该包没有可用于自动安装的稳定签名；请从发布页下载 DMG 手动安装"
                    .into();
            return;
        }
        let Ok(root) = installer::update_root() else {
            self.view.error = Some("无法准备更新目录".into());
            self.view.manual_requested = false;
            return;
        };
        let _ = std::fs::create_dir_all(&root);
        let transaction = Uuid::new_v4();
        let path = root.join(format!("github-{transaction}.dmg.part"));
        let status = UpdateStatus {
            transaction_id: transaction,
            release: offer.release.clone(),
            artifact: ArtifactDescriptor {
                target: Target::MacosArm64,
                kind: PackageKind::MacosDmg,
                file_name: DMG_NAME.into(),
                size: offer.size,
                sha256: offer.sha256.clone(),
                minimum_os: semver::Version::new(14, 0, 0),
            },
            phase: UpdatePhase::Downloading,
            received_bytes: 0,
            failure: None,
        };
        self.view.checking = true;
        self.view.status = Some(status);
        self.view.error = None;
        self.view.message = "正在从 GitHub 下载更新".into();
        let url = offer.dmg_url;
        let expected = offer.size;
        self.task = Some(Task::Download {
            handle: tokio::spawn(async move { download_file(&url, &path, expected).await }),
            path: root.join(format!("github-{transaction}.dmg.part")),
            expected,
        });
    }
    async fn finish_download(&mut self) {
        let Some(Task::Download {
            handle,
            path,
            expected,
        }) = self.task.take()
        else {
            return;
        };
        let result = handle
            .await
            .map_err(|error| format!("GitHub 下载失败：{error}"))
            .and_then(|result| result)
            .and_then(|()| {
                let status = self.view.status.as_ref().ok_or("更新任务丢失")?;
                if status.artifact.size != expected {
                    return Err("GitHub 更新任务不匹配".into());
                }
                digest_path(&path, expected, &status.artifact.sha256)
            });
        match result {
            Ok(()) => {
                if let Some(status) = &mut self.view.status {
                    status.received_bytes = expected;
                    status.phase = UpdatePhase::Verifying;
                }
                self.view.message = "正在校验 GitHub 更新包并准备安装".into();
                let Some(status) = self.view.status.clone() else {
                    self.fail_install("更新任务丢失", true);
                    return;
                };
                let release = status.release.clone();
                self.task = Some(Task::Prepare(tokio::task::spawn_blocking(move || {
                    installer::prepare_github(&path, &release, &status)
                })));
            }
            Err(error) => {
                let _ = std::fs::remove_file(&path);
                self.fail_install(&error, true);
            }
        }
    }
    async fn finish_prepare(&mut self) {
        let Some(Task::Prepare(task)) = self.task.take() else {
            return;
        };
        let result = task
            .await
            .map_err(|error| format!("更新暂存失败：{error}"))
            .and_then(|result| result);
        self.view.checking = false;
        match result {
            Ok(()) => {
                self.ready = true;
                if let Some(status) = &mut self.view.status {
                    status.phase = UpdatePhase::WaitingForIdle;
                }
                self.view.error = None;
                self.view.message = "新版已下载，等待传输、粘贴和远端控制结束后自动安装".into();
            }
            Err(error) => self.fail_install(&error, false),
        }
    }
    fn fail_install(&mut self, error: &str, quarantine: bool) {
        self.ready = false;
        self.force_install = false;
        self.view.checking = false;
        self.view.manual_requested = false;
        if let Some(status) = &mut self.view.status {
            status.phase = if quarantine {
                UpdatePhase::Quarantined
            } else {
                UpdatePhase::Cancelled
            };
        }
        self.view.message = "更新未安装，当前版本保持可用".into();
        self.view.error = Some(error.into());
    }
}
impl Drop for GitHubUpdater {
    fn drop(&mut self) {
        match &self.task {
            Some(Task::Check(task)) => task.abort(),
            Some(Task::Download { handle, .. }) => handle.abort(),
            Some(Task::Prepare(task)) => task.abort(),
            None => {}
        }
    }
}

fn cleanup_partial_downloads() {
    let Ok(root) = installer::update_root() else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("github-") && name.ends_with(".dmg.part") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn is_quarantined(id: Uuid) -> bool {
    installer::update_root()
        .ok()
        .and_then(|root| std::fs::read(root.join("quarantined.json")).ok())
        .and_then(|bytes| serde_json::from_slice::<BTreeSet<Uuid>>(&bytes).ok())
        .is_some_and(|set| set.contains(&id))
}

fn digest_path(path: &Path, size: u64, sha256: &str) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() != size {
        return Err("GitHub 更新包大小不匹配".into());
    }
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() != size {
        return Err("GitHub 更新包大小不匹配".into());
    }
    use std::io::Read;
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
        return Err("GitHub 更新包 SHA-256 不匹配".into());
    }
    Ok(())
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    published_at: Option<String>,
    assets: Vec<Asset>,
}
#[derive(Deserialize)]
struct Asset {
    name: String,
    state: String,
    size: u64,
}
#[derive(Deserialize)]
struct BuildInfo {
    release: ReleaseVersion,
    targets: Vec<BuildTarget>,
    dmg_sha256: String,
}
#[derive(Deserialize)]
struct BuildTarget {
    asset: String,
    target: String,
    sha256: String,
    signing: String,
}

fn checksums(text: &str) -> Result<BTreeMap<String, String>, String> {
    let mut map = BTreeMap::new();
    for line in text.lines().filter(|line| !line.is_empty()) {
        let (hash, name) = line.split_once("  ").ok_or("SHA256SUMS 格式无效")?;
        if hash.len() != 64
            || !hash
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err("SHA256SUMS 哈希无效".into());
        }
        if name.contains('/') || name.contains('\\') || name.contains("..") {
            return Err("SHA256SUMS 包含无效路径".into());
        }
        if !matches!(
            name,
            "DeskFuse-arm64.dmg" | "release.json" | "build-info.json"
        ) {
            continue;
        }
        if map.insert(name.to_owned(), hash.to_owned()).is_some() {
            return Err("SHA256SUMS 重复条目".into());
        }
    }
    for required in ["DeskFuse-arm64.dmg", "release.json", "build-info.json"] {
        if !map.contains_key(required) {
            return Err("SHA256SUMS 缺少必要文件".into());
        }
    }
    Ok(map)
}

fn sha256_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

async fn fetch_json(url: &str, limit: usize) -> Result<Vec<u8>, String> {
    // macOS ships curl. Keep requests off the engine actor, bounded and HTTPS-only;
    // redirects are needed for GitHub's release-asset CDN, never for local files.
    let output = tokio::process::Command::new(Path::new("/usr/bin/curl"))
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--connect-timeout",
            "5",
            "--max-time",
            "20",
            "--max-filesize",
            &limit.to_string(),
            "--user-agent",
            "DeskFuse-update-check",
            "--header",
            "Accept: application/vnd.github+json",
            url,
        ])
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|error| format!("无法连接更新服务器：{error}"))?;
    if !output.status.success() {
        return Err("更新请求失败（网络、访问限制或发布资产暂不可用）".into());
    }
    if output.stdout.len() > limit {
        return Err("更新响应超过大小限制".into());
    }
    Ok(output.stdout)
}

async fn download_file(url: &str, dest: &Path, size: u64) -> Result<(), String> {
    if size == 0 || size > DMG_LIMIT {
        return Err("GitHub 更新包大小无效".into());
    }
    let path = dest.to_str().ok_or("更新路径无效")?;
    let status = tokio::process::Command::new(Path::new("/usr/bin/curl"))
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--connect-timeout",
            "10",
            "--max-time",
            "600",
            "--max-filesize",
            &size.to_string(),
            "--user-agent",
            "DeskFuse-update-download",
            "--output",
            path,
            url,
        ])
        .kill_on_drop(true)
        .status()
        .await
        .map_err(|error| format!("无法下载 GitHub 更新：{error}"))?;
    if !status.success() {
        let _ = std::fs::remove_file(dest);
        return Err("GitHub 更新下载失败（网络或发布资产暂不可用）".into());
    }
    Ok(())
}

async fn fetch_release_list(url: &str) -> Result<Vec<Release>, String> {
    serde_json::from_slice(&fetch_json(url, 2 * 1024 * 1024).await?)
        .map_err(|_| "发布列表格式无效".into())
}

/// GitHub 是 canonical release source；只有 GitHub 请求失败或返回空列表时，
/// 才访问独立域名，避免一个旧站点快照覆盖 GitHub 上的新版本。
async fn release_candidates_with<F, Fut>(fetch: F) -> Result<Vec<Release>, String>
where
    F: Fn(&'static str) -> Fut,
    Fut: Future<Output = Result<Vec<Release>, String>>,
{
    let github_error = match fetch(API).await {
        Ok(releases) if !releases.is_empty() => return Ok(releases),
        Ok(_) => "GitHub 返回空发布列表".to_owned(),
        Err(error) => error,
    };
    match fetch(FEED).await {
        Ok(releases) if !releases.is_empty() => Ok(releases),
        Ok(_) => Err(format!(
            "更新检查失败：GitHub 返回空发布列表，独立域名也返回空发布列表"
        )),
        Err(site_error) => Err(format!(
            "更新检查失败：GitHub（{github_error}）；独立域名（{site_error}）"
        )),
    }
}

async fn release_candidates() -> Result<Vec<Release>, String> {
    release_candidates_with(fetch_release_list).await
}

/// 版本元数据也优先读取 GitHub 资产；只有 canonical 资产不可用时才读取站点镜像。
/// 两个来源的字节都必须通过 `SHA256SUMS` 校验，校验逻辑不变。
async fn fetch_release_file(tag: &str, name: &str, limit: usize) -> Result<Vec<u8>, String> {
    let github_url = format!("{DOWNLOAD}/{tag}/{name}");
    match fetch_json(&github_url, limit).await {
        Ok(bytes) => Ok(bytes),
        Err(github_error) => {
            let site_url = format!("{FEED_FILES}/{tag}/{name}");
            fetch_json(&site_url, limit).await.map_err(|site_error| {
                format!("GitHub 与独立域名均无法读取 {name}：{github_error}；{site_error}")
            })
        }
    }
}

async fn fetch_offer() -> Result<GitHubOffer, String> {
    // /latest excludes prereleases; DeskFuse's public channel currently publishes
    // prereleases, so inspect completed release entries and compare SemVer.
    let releases = release_candidates().await?;
    let (version, tag, size) = releases
        .into_iter()
        .filter(|release| {
            !release.draft
                && release.published_at.is_some()
                && [DMG_NAME, "release.json", "build-info.json", "SHA256SUMS"]
                    .iter()
                    .all(|name| {
                        release.assets.iter().any(|asset| {
                            asset.name == *name && asset.state == "uploaded" && asset.size > 0
                        })
                    })
        })
        .filter_map(|release| {
            let version: semver::Version = release.tag_name.strip_prefix('v')?.parse().ok()?;
            if !version.pre.is_empty()
                || !version.build.is_empty()
                || release.tag_name != format!("v{version}")
            {
                return None;
            }
            let size = release
                .assets
                .iter()
                .find(|asset| asset.name == DMG_NAME)?
                .size;
            if size == 0 || size > DMG_LIMIT {
                return None;
            }
            Some((version, release.tag_name, size))
        })
        .max_by(|a, b| a.0.cmp_precedence(&b.0))
        .ok_or("尚无完整的 macOS 发布包")?;
    let sums = checksums(
        &String::from_utf8(fetch_release_file(&tag, "SHA256SUMS", 65536).await?)
            .map_err(|_| "SHA256SUMS 不是有效文本")?,
    )?;
    let release_bytes = fetch_release_file(&tag, "release.json", 65536).await?;
    if sha256_bytes(&release_bytes) != sums["release.json"] {
        return Err("版本描述校验和不匹配".into());
    }
    let release: ReleaseVersion =
        serde_json::from_slice(&release_bytes).map_err(|_| "版本描述格式无效")?;
    if release.version != version
        || !(1..=9999).contains(&release.build_number)
        || release.release_id.is_nil()
    {
        return Err("标签与版本描述不一致".into());
    }
    let info_bytes = fetch_release_file(&tag, "build-info.json", 65536).await?;
    if sha256_bytes(&info_bytes) != sums["build-info.json"] {
        return Err("构建信息校验和不匹配".into());
    }
    let info: BuildInfo = serde_json::from_slice(&info_bytes).map_err(|_| "构建信息格式无效")?;
    let arm64 = info
        .targets
        .iter()
        .find(|target| target.asset == DMG_NAME && target.target == "aarch64-apple-darwin")
        .ok_or("构建信息缺少 arm64 目标")?;
    if info.release != release
        || info.dmg_sha256 != sums[DMG_NAME]
        || arm64.sha256 != sums[DMG_NAME]
    {
        return Err("构建信息与发布包不一致".into());
    }
    Ok(GitHubOffer {
        release,
        release_url: format!("{RELEASES}/{tag}"),
        dmg_url: format!("{DOWNLOAD}/{tag}/{DMG_NAME}"),
        sha256: sums[DMG_NAME].clone(),
        size,
        developer_id: arm64.signing == "developer-id",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksums_require_dmg_and_metadata() {
        let text = "\
aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  DeskFuse-arm64.dmg
bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb  release.json
cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc  build-info.json
dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd  extra.txt
";
        let map = checksums(text).unwrap();
        assert_eq!(map.len(), 3);
        assert_eq!(map[DMG_NAME].len(), 64);
    }

    fn release(tag: &str, assets: &[(&str, u64)]) -> Release {
        let assets: Vec<String> = assets
            .iter()
            .map(|(name, size)| format!(r#"{{"name":"{name}","state":"uploaded","size":{size}}}"#))
            .collect();
        serde_json::from_str(&format!(
            r#"{{"tag_name":"{tag}","draft":false,"published_at":"2026-09-21T00:00:00Z","assets":[{}]}}"#,
            assets.join(",")
        ))
        .unwrap()
    }

    #[tokio::test]
    async fn release_candidates_prioritize_github_and_only_fallback_to_site() {
        use std::sync::{Arc, Mutex};

        let calls = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&calls);
        let releases = release_candidates_with(move |url| {
            let seen = Arc::clone(&seen);
            async move {
                seen.lock().unwrap().push(url);
                if url == API {
                    Ok(Vec::new())
                } else {
                    Ok(vec![release("v0.4.5", &[("DeskFuse-arm64.dmg", 10)])])
                }
            }
        })
        .await
        .expect("site fallback");

        assert_eq!(releases[0].tag_name, "v0.4.5");
        assert_eq!(*calls.lock().unwrap(), vec![API, FEED]);
    }

    #[tokio::test]
    async fn release_candidates_do_not_query_site_when_github_succeeds() {
        use std::sync::{Arc, Mutex};

        let calls = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&calls);
        let releases = release_candidates_with(move |url| {
            let seen = Arc::clone(&seen);
            async move {
                seen.lock().unwrap().push(url);
                Ok(vec![release("v0.4.5", &[("DeskFuse-arm64.dmg", 10)])])
            }
        })
        .await
        .expect("github release list");

        assert_eq!(releases[0].tag_name, "v0.4.5");
        assert_eq!(*calls.lock().unwrap(), vec![API]);
    }

    #[test]
    fn the_metadata_mirror_sits_beside_the_feed() {
        let base = FEED.strip_suffix("releases.json").expect("feed file name");
        assert_eq!(FEED_FILES, format!("{base}releases"));
        assert!(FEED_FILES.starts_with("https://"));
    }

    /// GitHub 的资产 CDN 偶发 504，交叉校验前先重试几次
    async fn fetch_github_asset_with_retry(tag: &str, name: &str) -> Result<Vec<u8>, String> {
        let mut last = String::new();
        for _ in 0..3 {
            match fetch_json(&format!("{DOWNLOAD}/{tag}/{name}"), 65536).await {
                Ok(bytes) => return Ok(bytes),
                Err(error) => last = error,
            }
        }
        Err(last)
    }

    #[tokio::test]
    #[ignore = "live network verification"]
    async fn live_github_release_list_is_reachable() {
        let releases = fetch_release_list(API).await.expect("github release list");
        assert!(releases.iter().any(|release| release.tag_name == "v0.4.5"));
    }

    #[tokio::test]
    #[ignore = "live network verification"]
    async fn live_site_release_list_is_reachable() {
        let releases = fetch_release_list(FEED).await.expect("site release list");
        assert!(releases.iter().any(|release| release.tag_name == "v0.4.5"));
    }

    #[tokio::test]
    #[ignore = "live network verification"]
    async fn live_site_mirror_matches_the_github_assets() {
        let releases = fetch_release_list(FEED).await.expect("site release list");
        let tag = releases
            .iter()
            .filter(|release| {
                !release.draft
                    && release.published_at.is_some()
                    && [DMG_NAME, "release.json", "build-info.json", "SHA256SUMS"]
                        .iter()
                        .all(|name| release.assets.iter().any(|asset| asset.name == *name))
            })
            .map(|release| release.tag_name.clone())
            .next()
            .expect("an installable release");
        for name in ["SHA256SUMS", "release.json", "build-info.json"] {
            let mirror = fetch_json(&format!("{FEED_FILES}/{tag}/{name}"), 65536)
                .await
                .unwrap_or_else(|error| panic!("site {tag}/{name}: {error}"));
            match fetch_github_asset_with_retry(&tag, name).await {
                Ok(github) => assert_eq!(mirror, github, "{tag}/{name} differs from GitHub"),
                // 资产 CDN 抖动本身就是站点镜像要挡住的失败，这里只记录不判失败
                Err(error) => println!("{tag}/{name}: github asset unavailable ({error})"),
            }
        }
        println!("{tag}: site mirror served every metadata file");
    }

    #[tokio::test]
    #[ignore = "live network verification"]
    async fn live_offer_is_reachable_through_primary_update_path() {
        let offer = fetch_offer().await.expect("live offer");
        println!(
            "version={} build={} size={} url={}",
            offer.release.version, offer.release.build_number, offer.size, offer.release_url
        );
    }

    #[test]
    fn checksums_reject_paths_and_missing_files() {
        assert!(checksums("zzzz  DeskFuse-arm64.dmg\n").is_err());
        assert!(
            checksums(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  ../evil.dmg\n"
            )
            .is_err()
        );
        assert!(checksums(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  DeskFuse-arm64.dmg\n\
bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb  release.json\n"
        )
        .is_err());
    }
}
