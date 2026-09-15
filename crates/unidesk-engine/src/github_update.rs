//! Public distributions check GitHub; installation remains with Homebrew or the
//! user. GitHub metadata is never fed into the signed LAN installer.
use serde::Deserialize;
use std::{
    path::Path,
    time::{Duration, Instant},
};
use unidesk_runtime::updates::{GitHubUpdateView, LanUpdateView};
use unidesk_update::{ReleaseVersion, installer};

const API: &str = "https://api.github.com/repos/GodD6366/uniDesk/releases?per_page=100";
const DOWNLOAD: &str = "https://github.com/GodD6366/uniDesk/releases/download";
const RELEASES: &str = "https://github.com/GodD6366/uniDesk/releases/tag";
const INTERVAL: Duration = Duration::from_secs(12 * 60 * 60);

pub(crate) fn is_public_bundle() -> bool {
    installer::current_bundle().ok().is_some_and(|bundle| {
        std::fs::read_to_string(bundle.join("Contents/Resources/update-channel"))
            .is_ok_and(|channel| channel.trim() == "github")
    })
}

pub(crate) struct GitHubUpdater {
    view: LanUpdateView,
    task: Option<tokio::task::JoinHandle<Result<(ReleaseVersion, String), String>>>,
    last_check: Instant,
}
impl GitHubUpdater {
    pub(crate) fn start() -> Result<Self, String> {
        let bundle = installer::current_bundle()?;
        let installed = installer::installed_release(&bundle)?;
        let mut updater = Self {
            view: LanUpdateView {
                installed: Some(installed),
                github: Some(GitHubUpdateView::default()),
                message: "公开发行版通过 GitHub 检查更新".into(),
                ..Default::default()
            },
            task: None,
            last_check: Instant::now(),
        };
        updater.check_now();
        Ok(updater)
    }
    pub(crate) fn check_now(&mut self) {
        if self.task.is_some() {
            return;
        }
        self.view.checking = true;
        self.view.error = None;
        self.view.message = "正在检查 GitHub 发布版本…".into();
        self.last_check = Instant::now();
        self.task = Some(tokio::spawn(fetch_release()));
    }
    pub(crate) fn snapshot(&self) -> LanUpdateView {
        self.view.clone()
    }
    pub(crate) async fn poll(&mut self) {
        if self.task.as_ref().is_some_and(|task| task.is_finished()) {
            let result = self
                .task
                .take()
                .expect("completed GitHub check")
                .await
                .map_err(|error| format!("GitHub 更新检查失败：{error}"))
                .and_then(|result| result);
            self.view.checking = false;
            match result {
                Ok((release, url)) => {
                    let newer = self
                        .view
                        .installed
                        .as_ref()
                        .is_some_and(|installed| release.precedence(installed).is_gt());
                    self.view.github = Some(GitHubUpdateView {
                        target_version: newer.then(|| {
                            format!("{} · build {}", release.version, release.build_number)
                        }),
                        release_url: newer.then_some(url),
                    });
                    self.view.message = if newer {
                        "GitHub 有新版本；请通过 Homebrew 或发布页安装".into()
                    } else {
                        "当前版本已是最新，GitHub 没有更高版本".into()
                    };
                    self.view.error = None;
                }
                Err(error) => {
                    // Retain the last successful update offer during network failures.
                    self.view.message = "暂时无法确认 GitHub 最新版本，请稍后重试".into();
                    self.view.error = Some(error);
                }
            }
        }
        if self.task.is_none() && self.last_check.elapsed() >= INTERVAL {
            self.check_now();
        }
    }
}
impl Drop for GitHubUpdater {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
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
            "uniDesk-update-check",
            "--header",
            "Accept: application/vnd.github+json",
            url,
        ])
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|error| format!("无法连接 GitHub：{error}"))?;
    if !output.status.success() {
        return Err("GitHub 请求失败（网络、访问限制或发布资产暂不可用）".into());
    }
    if output.stdout.len() > limit {
        return Err("GitHub 响应超过大小限制".into());
    }
    Ok(output.stdout)
}

async fn fetch_release() -> Result<(ReleaseVersion, String), String> {
    // /latest excludes prereleases; uniDesk's public channel currently publishes
    // prereleases, so inspect completed release entries and compare SemVer.
    let releases: Vec<Release> = serde_json::from_slice(&fetch_json(API, 2 * 1024 * 1024).await?)
        .map_err(|_| "GitHub 发布列表格式无效")?;
    let (version, tag) = releases
        .into_iter()
        .filter(|release| {
            !release.draft
                && release.published_at.is_some()
                && [
                    "uniDesk-arm64.dmg",
                    "release.json",
                    "build-info.json",
                    "SHA256SUMS",
                ]
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
            Some((version, release.tag_name))
        })
        .max_by(|a, b| a.0.cmp_precedence(&b.0))
        .ok_or("GitHub 尚无完整的 macOS 发布包")?;
    let release: ReleaseVersion = serde_json::from_slice(
        &fetch_json(&format!("{DOWNLOAD}/{tag}/release.json"), 65536).await?,
    )
    .map_err(|_| "GitHub 版本描述格式无效")?;
    if release.version != version
        || !(1..=9999).contains(&release.build_number)
        || release.release_id.is_nil()
    {
        return Err("GitHub 标签与版本描述不一致".into());
    }
    Ok((release, format!("{RELEASES}/{tag}")))
}
