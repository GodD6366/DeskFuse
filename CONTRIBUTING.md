# 参与开发

本项目处于开发阶段，不承诺兼容历史实验协议。面向贡献者的规范以本文、功能契约和架构文档为准。本机 agent 操作配置不纳入公开源码。

## 环境与检查

- macOS 主平台：Apple Silicon、macOS 14+、Rustup、完整 Xcode 27+（macOS 27+ SDK）、Python 3.11+。
- `rustup show active-toolchain` 安装／确认仓库固定工具链；打包前运行 `rustup target add aarch64-apple-darwin` 和 `rustup component add rust-src`。脚本不会在发行过程中隐式安装工具链。
- Windows 原生检查使用 Windows x64、PowerShell、VS 2022 MSVC x64 工具链和 Windows SDK；引导说明见 `scripts/bootstrap-windows.ps1`，CI 使用已有 runner 环境。

```sh
./scripts/check.sh
python3 scripts/check-repository.py --worktree
./scripts/check-ci.sh
python3 -m unittest discover -s scripts/tests -v
CODESIGN_IDENTITY=- ./scripts/build_and_run.sh --build
```

`check-ci.sh` 下载并校验固定版本的 actionlint／Gitleaks，在临时目录运行后清理，不全局安装。它检查工作流、有效工作树（含未暂存的新源码）及 Git 历史，不扫描被忽略的个人验证产物，也不跟随本机软链接。`check-repository.py --worktree` 不修改暂存区；省略该参数则检查已跟踪文件，适合暂存后和 CI 使用。

`check.sh` 统一执行 workspace rustfmt、locked check、test、Clippy 和原生 Swift 构建。只修复格式时可运行 `cargo fmt --all`；独立 include 文件另外运行 `rustfmt --edition 2024`。禁止用删除 lockfile 解决依赖冲突。Release 构建期依赖禁用 stripping，以规避部分新版 macOS 对 proc-macro dylib 的 LINKEDIT 对齐拒绝；不改应用优化级别。

Windows：分别运行 `scripts/check-windows-native.ps1` 和 `scripts/check-windows-service.ps1`。这些检查不安装服务、不创建机器身份、不向桌面发送输入。

本地开发与 DMG 打包共同使用 `scripts/codesign-identity.sh`，证书指纹保存在 Git 忽略的 `.codesign-identity`。首次构建只有一张可用 Apple Development 证书时自动固定；多张证书时显式设置 `CODESIGN_IDENTITY=<SHA-1>`。已有配置的证书不可用时构建失败，不自动换证书或退回 ad-hoc。证书轮换时更新本地配置；显式环境变量覆盖只影响当前调用。`./scripts/build_and_run.sh --build` 只构建并验证签名，不启动或停止应用。公开 tag 预发行由 CI 使用 Developer ID 并公证，见 [公证凭据](docs/NOTARIZE.md)。没有证书的贡献者可显式 `CODESIGN_IDENTITY=-`；无需取得维护者证书、LAN 发行私钥或 NAS。独立 DMG 使用 `./scripts/package-macos.sh --bootstrap`，与默认维护者 LAN 交付流程隔离。

## 结构和变更边界

- `apps/` 组合平台适配器与 runtime；协议和信任策略留在 `crates/`，不要放进 UI。
- core/protocol 不依赖 SwiftUI 或具体操作系统；平台层负责原生能力与权限，不虚构可用状态。
- workspace 成员继承公共 package 元数据，不单独维护版本／锁文件；这是应用源码仓库，不向 crates.io 发布内部包。
- UniFFI 版本由 `Cargo.lock` 固定，Swift 绑定由同版本生成器生成；不得手改生成文件。SwiftUI/AppKit 使用 Xcode 系统 SDK，macOS 26+ 使用原生 Liquid Glass，低版本与降低透明度必须回退。
- `scripts/` 是唯一操作脚本入口；生成资源存入被忽略的 `resources/generated/`。`resources/DeskFuse-icon.svg` 和 `deskfuse-status-icon.svg` 是图标源，不编辑 PNG 或派生中心图。

## 代码与验证

Rust 2024、显式错误处理、保持 async 任务取消和关闭路径。unsafe/FFI 的注释应说明所有权、线程要求和调用前提。日志只记录状态、元数据和哈希。

修复需要回归证明的问题时补有针对性的测试；可逆且低风险的文字或纯格式修改不为覆盖率机械加测试。运行与改动相称的检查后，不重复无关测试。

UI 修改先读[当前设计基准](design/current/README.md)，打开原图；实际应用核对正常窗口及 960 × 620 pt 的可读、点击、焦点、滚动与空态。构建通过不是 UI 验收。

文件操作遵循[文件传输契约](docs/FILE_TRANSFER.md)。原生复制／粘贴、拖出、输入权限、两机行为分别验收。不要用删除本地身份文件、伪造权限或运行多份实例修复环境。

## 提交与评审

1. 分支按主题命名；agent 默认使用 `codex/` 前缀。
2. 每次提交围绕一个可检查目标，消息说明变化与原因。不要求贡献者签署 CLA。
3. PR 说明影响、验证层级与未验证项；UI 附截图，协议说明行为变化。
4. 先检查 `git diff --cached --stat` 与文件列表；禁止把运行证据、用户文件、设备状态、签名材料或整个构建目录加入提交。
5. 原始验证记录留在被忽略的 `artifacts/validation/`。需要公开的说明提炼为脱敏文字或经检查的 `docs/images/` 图片，文档不要链接本地证据目录。

按仓库所声明的双许可证提交自有贡献，保留第三方来源和署名。提交前检查与公开边界见[开源提交说明](docs/OPEN_SOURCE.md)，发布步骤见[发行指南](docs/RELEASING.md)。
