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
8. 等待 prerelease workflow 的 preflight、checks、双架构 DMG、publish 和 cask jobs 全部成功。

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
