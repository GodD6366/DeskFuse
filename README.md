# DeskFuse

DeskFuse 使用独立的应用身份、协议和数据目录。首次启动可确认导入旧安装的剪贴板历史，导入后可选择移除旧应用；不兼容旧协议。源码、安装包、Homebrew tap 和问题反馈都在本仓库 `GodD6366/DeskFuse`。

[English](README.en.md) · [开发指南](CONTRIBUTING.md) · [发行指南](docs/RELEASING.md) · [计划与待办](TODO.md)

DeskFuse 是 macOS 优先的原生桌面协作工具（Rust 2024 + SwiftUI）：在可信设备间同步剪贴板、传输文件、暂存并拖出文件，并提供独立开关的键鼠共享。交互约定见[当前界面与行为约定](design/current/README.md)。

> 开发预发行：GitHub 提供 macOS arm64、macOS Intel 和 Windows x64 安装包；Windows x64 为 Beta，安装器必须通过 Authenticode 门禁，构建通过不代表安装、NLA 或真实两机验收。

## 能做什么

- 两步首次运行设置：先逐项引导系统权限，再确认默认开启的功能；剪贴板、键鼠共享与远程应用默认开启，鼠标自定义与麦克风默认关闭，每个功能带演示动画。即使应用已安装，只要本机尚未完成当前引导也会运行一次。
- 自动发现或手动 IPv4 配对可信设备，连接状态与权限阻碍可见；后续使用权限按需申请。
- 同步文本、图片和原生剪贴板格式；剪贴板历史按类型分组，支持手动列表。
- 文件传输页合并发送区、传输队列与手机扫码；文件按需传输，保留名称与文件夹结构，历史缓存有容量限制和清理回执。
- 文件暂存抽屉：拖入暂存、整批拖出，也可发送给其他设备。
- 键鼠共享工作区：真实显示器布局、设备激活、权限状态与紧急收回；已配对且在线的电脑自动加入。
- 侧栏底部健康指示汇总需要处理的问题，错误提示保留到处理或关闭。
- 工作流为进阶功能，默认关闭，可在设置 → 高级启用。
- Beta：远程应用（macOS 窗口与 Windows RemoteApp）。

行为以[文件传输契约](docs/FILE_TRANSFER.md)、[架构](docs/ARCHITECTURE.md)和[路线图](docs/ROADMAP.md)为准。

## 对比 macOS 连续互通与 Deskflow

| 功能 | macOS 连续互通 | Deskflow | DeskFuse |
| --- | --- | --- | --- |
| 键鼠跨屏 | ✅ 仅限同 Apple 账号 | ⚠️ 屏幕位置无法自由摆放 | ✅ |
| 剪贴板同步 | ✅ 依赖 iCloud 稳定性不佳 | ⚠️ 仅基础文字、小图片 | ✅ |
| 文件传输 | ✅ AirDrop | ❌ | ✅ 支持文件夹 |
| 文件暂存抽屉 | ❌ | ❌ | ✅ |
| 剪贴板历史 | ❌ | ❌ | ✅ 跨设备同步，支持图片内容搜索 |
| 加密传输 | ✅ | ✅ TLS | ✅ QUIC |
| 开源 | ❌ | ✅ GPL-2.0 | ❌ 私有源码 |

对比依据（2026-09-15）：[Apple 通用控制](https://support.apple.com/en-us/HT212757)、[通用剪贴板](https://support.apple.com/en-us/102430)、[Deskflow](https://github.com/deskflow/deskflow)（文件拖放移除见 [#8459](https://github.com/deskflow/deskflow/issues/8459)）。

## 支持平台

| 平台 | 状态 |
| --- | --- |
| macOS 14+、Apple Silicon | 主平台，发行 arm64 DMG |
| macOS 26+ | 导航与操作层使用原生 Liquid Glass，旧系统回退标准材质 |
| Intel Mac | 发行 x86_64 DMG，功能和实机验收与 arm64 分开记录 |
| Windows x64 | Beta；提供 Authenticode 签名安装器；安装、SCM、NLA、RemoteApp 和实机验收单独记录 |
| Linux | 不在首发范围 |

## 下载与安装

下载见 [GitHub Releases](https://github.com/GodD6366/DeskFuse/releases) 或 [官网](https://deskfuse.godd.cc)。v0.7.0 及更早的安装包保留在已停止更新的 [旧发行仓库](https://github.com/GodD6366/deskfuse-release/releases)。

### Homebrew（macOS 14+、Apple Silicon）

```sh
brew trust --cask godd6366/deskfuse/deskfuse
brew tap godd6366/deskfuse https://github.com/GodD6366/DeskFuse
brew install --cask godd6366/deskfuse/deskfuse
```

`brew trust` 执行一次即可（旧版本无此命令时跳过）。之前从旧发行仓库添加过 tap 的，请先执行 `brew untap godd6366/deskfuse` 再按上面重新添加。cask 固定对应 DMG 的 SHA-256。卸载：`brew uninstall --cask godd6366/deskfuse/deskfuse`，不再使用时 `brew untap godd6366/deskfuse`。

### 更新

Homebrew / GitHub 公开包在“设置 → 更新”中检查 GitHub Releases（包括预发行版），发现新版后可打开对应发布页。Homebrew 安装请执行：

```sh
brew update && brew upgrade --cask deskfuse
```

内部签名包继续使用可信局域网自动更新；公开包不使用该安装通道。尚未包含 GitHub 自动安装功能的旧版需要先通过 Homebrew 或手动下载安装新版。

### 手动安装 DMG

1. 按设备从同一 Release 下载 `DeskFuse-arm64.dmg` 或 `DeskFuse-x86_64.dmg`，同时下载 `SHA256SUMS`、`release.json`、`build-info.json`，运行 `shasum -a 256 -c SHA256SUMS`。
2. 打开 DMG，把 `DeskFuse.app` 拖进 Applications；同一设备只运行一份实例。
3. 按提示允许局域网访问；键鼠共享还需辅助功能授权、可信局域网会话、设备激活和可用显示器布局。

配置 Apple 凭据后的 GitHub 预发行使用 Developer ID 签名并公证，可直接打开；不要关闭 Gatekeeper 或 SIP。更早的 ad-hoc 预发行若被拦截，用系统“隐私与安全性 → 仍要打开”，或 `sudo xattr -rd com.apple.quarantine "/Applications/DeskFuse.app"`。维护者凭据准备见 [公证凭据](docs/NOTARIZE.md)。

## 从源码运行

需要 Apple Silicon Mac、完整 **Xcode 27+（macOS 27+ SDK）**、Rustup、Python 3.11+；Command Line Tools 不足。最低运行目标 macOS 14，工具链与依赖分别固定在 `rust-toolchain.toml` 和 `Cargo.lock`。

```sh
git clone git@github.com:GodD6366/DeskFuse.git
cd DeskFuse
rustup show active-toolchain
rustup target add aarch64-apple-darwin
rustup component add rust-src
CODESIGN_IDENTITY=- ./scripts/build_and_run.sh
```

无签名证书时显式加 `CODESIGN_IDENTITY=-`；加 `--build` 只构建不启动，多套 Xcode 用 `DEVELOPER_DIR=...` 单次选择。本地局域网安装包直接运行 `./scripts/package-macos.sh`（等同 `--lan`），输出到 `dist/{version}-build{number}/`。本机迭代运行 `./scripts/package-macos.sh --auto`：默认使用增量 Swift、复用 Rust 和麦克风缓存，签名并安装应用，保留 LAN 更新包，跳过 DMG 和公证。首次构建或缓存失效仍需编译；需要完整打包时使用 `--auto --full`，仅需额外生成 DMG 时使用 `--auto --with-dmg`。快速模式产物仅供本地测试，不用于公开发布；CI 或要求公证时默认保留完整流程。Homebrew/GitHub 渠道需显式指定 `--homebrew`；无发行证书的渠道测试可使用 `CODESIGN_IDENTITY=- ./scripts/package-macos.sh --homebrew`，输出到 `dist/homebrew/`，不参与局域网更新。

## 项目结构

```text
apps/           SwiftUI 应用、Windows 服务、诊断探针
crates/         领域模型、协议、身份、存储、网络、传输、输入及平台实现
scripts/        开发、检查、资源生成、打包与发行工具
Casks/          公开 Homebrew cask（同时作为 tap）
resources/      原始 SVG、bundle 元数据、第三方图标、公开传输引导材料
design/current/ 当前界面行为规范与图标资源
docs/           架构、功能契约、发布与支持边界
.github/        CI、预发行工作流、贡献模板和依赖更新
```

`target/`、`dist/`、`artifacts/` 和历史设计不属于公开源码。

## 已知限制

- Windows 安装器为离线桌面包；安装、SCM、会话切换、NLA 和跨平台功能需独立验收。
- 原生 Finder 粘贴、拖出、权限变化、Wi-Fi 键鼠跨屏和真实两机行为不能由 CI 代替。
- GitHub CI 不签发局域网更新载荷；本机协调端正式包支持 macOS 局域网自动更新，旧版需先手动安装一次基础包，见[发行指南](docs/RELEASING.md)。
- 日志只记录元数据、状态和哈希；提交问题前检查附件，不上传剪贴板内容、设备密钥或凭据。

漏洞披露与传输认证边界见 [SECURITY.md](SECURITY.md)。

## 许可证

当前源码仅在私有仓库维护，不再公开分发。历史已公开版本继续适用其原有 [MIT](LICENSE-MIT)／[Apache-2.0](LICENSE-APACHE) 许可；新增自有代码见 [LICENSE](LICENSE)。依赖与第三方资源保留各自许可证，见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
