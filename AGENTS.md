# DeskFuse 项目协作与发布约束

## 沟通

- 面向用户的叙述默认使用简体中文；代码、命令和技术标识保持英文。
- 先给影响与结论，再给行动、待决策和必要证据。
- 发布、构建或部署问题必须说明：首个失败阶段、直接错误、根因、已验证范围和仍未验证的范围。

## 变更与验证

- 保留与当前任务无关的工作树修改；只提交本次任务需要的文件。
- 修改后运行与改动相称的检查，并在结论中列出实际通过的命令。
- 发布相关改动至少执行 `git diff --check`、`python3 scripts/release.py validate`、`cargo check --locked`；涉及 Rust 工作区时再执行 `./scripts/check.sh`。
- 不把“CI 通过”表述为实机双设备验收通过；签名、公证、DMG 完整性、公开 Release、Homebrew cask 和实机功能验收是独立结论。

## 版本发布顺序（必须遵守）

1. 修改 `Cargo.toml` 的 workspace version。
2. 更新 `CHANGELOG.md`。
3. 运行 `cargo check --locked`，确保 `Cargo.lock` 已同步。
4. 删除仅用于新版本生成的旧 `release.json`，使用 `python3 scripts/release.py prepare` 生成新的 release descriptor；不要手工复制上一版本的 build number 或 release ID。
5. 运行 `python3 scripts/release.py validate`。
6. 提交并推送 main，等待 main CI 通过。
7. 只从包含上述全部版本文件的最新提交创建 `vX.Y.Z` tag，再推送 tag。
8. 等待 prerelease workflow 的 preflight、checks、双架构 DMG、publish、mirror-legacy（过渡期）、cask 和 pages jobs 全部成功。

禁止先创建 tag、再提交版本 bump 或 `release.json`/`Cargo.lock`。发布 tag 必须同时满足：

- tag 为 `vX.Y.Z`；
- tag 所指提交中的 `Cargo.toml` workspace version 为 `X.Y.Z`；
- tag 所指提交中的 `release.json.version` 为 `X.Y.Z`；
- `Cargo.lock` 中所有 workspace crates 与 workspace version 一致。

## 0.4.5 发布事故记录（2026-09-24）

- 失败原因一：`v0.4.5` 先指向旧提交 `2434a11`，版本 bump 提交 `1965edd` 在 tag 之后，导致 tag、workspace version 和 release descriptor 不一致。
- 失败原因二：版本 bump 后没有同步 `Cargo.lock`，`cargo check --locked` 因禁止自动修改 lock file 失败。
- 修复提交：`9960595 chore(release): align 0.4.5 metadata`。
- 修复内容：同步 workspace crate 版本、生成 `0.4.5` release descriptor（build `64`），将远程 `v0.4.5` tag 移到修复提交。
- 结果：Prerelease run `35983236623` 的 preflight、Rust checks、arm64 DMG、x86_64 DMG、Developer ID 签名、公证、publish 和 cask 全部成功。
- 经验：版本元数据错误应在打 tag 前由发布脚本或 checklist 阻断；不要通过放宽 preflight 校验来“修复”发布。

## 0.4.5 更新接口事故记录（2026-09-24）

- 现象：`https://deskfuse.godd.cc/api/releases.json` 在 `v0.4.5` Release 和 Pages workflow 成功后仍返回 `v0.4.4`。
- 根因：Pages 的 `release` 事件使用 release tag 的 `GITHUB_SHA` 作为 `pages_build_version`；`v0.4.5` tag 当时仍指向公开仓库旧的 Cask 提交 `7a01054`，与此前定时部署相同。随后 cask job 提交 `dc5382e` 只改了 `Casks/**`，原 Pages workflow 的 `push.paths` 只监听 `web/**`，因此没有产生新的 Pages 部署，旧 CDN 内容继续提供服务。
- 修复：公开仓库提交 `3b71062`，将 `Casks/**` 加入 Pages push 触发路径；同时提交 `v0.4.5` feed、`release.json`、`build-info.json`、`SHA256SUMS` 基线，并把网站 fallback/说明更新为 `v0.4.5`。
- 结果：Pages push run `35987764276` 成功；线上 feed 首条为 `v0.4.5`，线上三个元数据文件与仓库基线逐字节一致。
- 预防：每次 release 后必须同时确认 cask commit 之后 pages job 已触发并完成 Pages 部署，并直接请求自定义域名的 `/api/releases.json` 和 `/api/releases/vX.Y.Z/{SHA256SUMS,release.json,build-info.json}`；GitHub Release 或 Pages job 成功不等于自定义域名已更新。

## 发行仓库合并（2026-10-22）

- 源码仓库 `GodD6366/DeskFuse` 公开后，Release、Homebrew tap（`Casks/`）、站点（`web/`、`.github/workflows/pages.yml`）和问题反馈都并入本仓库；`config/distribution.json` 的 `release_repository` 为 `GodD6366/DeskFuse`。
- 已安装的 v0.7.0 及更早客户端只查询 `GodD6366/deskfuse-release`。保留 `legacy_release_repository` 期间，发布流程镜像同一批资产到旧仓库，旧 tap 的 cask 提示用户重新 tap。
- 过渡版本发布后，必须确认旧仓库 Release、旧 tap cask 和站点 feed 都出现新版本，再删除 `legacy_release_repository`、重新生成 `web/src/config/distribution.json` 并归档旧仓库。不要在过渡版本发布前归档旧仓库。
- `GITHUB_TOKEN` 推送的提交不会触发其他 workflow；站点部署由 Prerelease 的 pages job 在 main 上显式触发，不依赖 push 事件。

## 更新源优先级（2026-09-24）

- 公开发行版更新检查必须先读取 GitHub Releases canonical source；只有 GitHub 请求失败或返回空列表时，才读取 `release_feed_url` 的独立域名镜像。
- `SHA256SUMS`、`release.json`、`build-info.json` 同样遵守 GitHub 资产优先、独立域名回退；最终仍必须通过哈希、版本描述、build-info、签名和公证校验。
- Homebrew tap 的 raw cask 只提供版本号和 arm64 SHA-256，不提供完整 Release 元数据，不能单独作为更新源。发布或更新检查改动后要分别验证 GitHub Releases API、独立 feed、元数据镜像和 Homebrew cask endpoint。
