# 开源提交说明

本项目保留 Rust 2024 共享引擎与 SwiftUI 原生客户端的现有分层；整理提交不代表发布完成。

## 公开文件边界

- 提交：`apps/`、`crates/`、`scripts/`、`Casks/`（公开 Homebrew cask）、当前 `docs/` 与 `design/current/`、必要原始资源、根 lockfile、许可证、贡献文档及 `.github/` 工作流。
- 不提交：`target/`、`dist/`、`artifacts/`、历史设计、历史开发方案（`INPUT_DATAGRAM_PLAN.md`、`NATIVE_MIGRATION.md`）、本机 `AGENTS.md`／`.codex/`、自动生成的 Swift/C 绑定、个人 agent 安装／状态、外部软链接、设备数据、`.env`、签名证书和私钥。
- `resources/unidesk-transport-*.der` 是按固定哈希核对的公开 TLS 引导材料；`resources/unidesk-release-public-key.hex` 仅是发行公钥。含义和认证边界见 [SECURITY](../SECURITY.md)。不能以此为其他密钥文件添加排除规则。
- 自有贡献沿用 MIT OR Apache-2.0；第三方许可正文和来源不改写。生成绑定、公开设计图和原生实机证据不混用。

## 提交前检查

```sh
# 包含未跟踪的新源码和待删除文件，不修改暂存区。
python3 scripts/check-repository.py --worktree
./scripts/check-ci.sh
cargo fmt --all -- --check
python3 scripts/third-party-notices.py --output artifacts/validation/THIRD_PARTY_DEPENDENCIES.txt
```

公开边界检查会拒绝软链接、生成绑定、私有材料路径、断开的文档链接以及过期或哈希不符的许可副本。Gitleaks 使用固定版本检查源码快照及 Git 历史；扫描通过不是任意形式凭据的完整审计。

完整 Rust 测试、Clippy、原生构建及 Windows 实验性检查由 CI 定义；本地按本轮明确要求执行，不把未运行的检查写成通过。贡献者无需维护者证书和发行私钥，开发包／bootstrap DMG 命令见 [CONTRIBUTING](../CONTRIBUTING.md) 与 [发行指南](RELEASING.md)。

## 暂存、提交和远端

确认上述检查与公开文件列表后，再暂存与检查最终版本：

```sh
git add -A
python3 scripts/check-repository.py
git diff --cached --check
git diff --cached --stat
```

维护者确认最终内容后执行 `git commit`、推送分支。提交源码、修改仓库可见性、启用 GitHub 设置及推送发行 tag 是不同操作；不会因为完成本地整理就自动发生。tag 发布流程见 [发行指南](RELEASING.md)。

GitHub Actions 的 macOS 源码构建使用 `xcode-27`（预览）以满足当前 SDK 27 API，运行目标仍为 macOS 14；Windows job 不安装服务、不生成公开安装包。真实 UI、Finder、权限和两机行为另行验收。

主分支 push 与手动 CI 的独立打包 job 在源码边界及包检查成功后生成 bootstrap arm64 DMG 和 SHA-256 清单，作为 Actions artifact 保留 14 天；测试 job 独立报告结果，存在构建产物不代表测试全部通过；PR 只检查与构建，不发布安装包。该 main 分支包仍为 ad-hoc，不包含 LAN 自动更新签名清单。公开 tag 预发行使用 Developer ID 并公证，凭据见 [公证凭据](NOTARIZE.md)。
