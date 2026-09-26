# uniDesk

[English](README.en.md) · [开发指南](CONTRIBUTING.md) · [发行指南](docs/RELEASING.md) · [计划与待办](TODO.md)

uniDesk 是 macOS 优先的原生桌面协作工具（Rust 2024 + SwiftUI）：在可信设备间同步剪贴板、传输文件、暂存并拖出文件，并提供独立开关的键鼠共享。交互约定见[当前界面与行为约定](design/current/README.md)。

> 开发预发行：macOS 提供 Apple Silicon 测试包，Windows 仅为实验性实现；构建通过不代表真实两机验收。

## 能做什么

- 自动发现或手动 IPv4 配对可信设备，连接状态与权限阻碍可见。
- 同步文本、图片和原生剪贴板格式。
- 文件按需传输，可选小文件提前传输，保留名称与文件夹结构；历史缓存有容量限制和清理回执。
- 文件暂存抽屉：拖入暂存、整批拖出，也可发送给其他设备。
- 独立的键鼠共享工作区：真实显示器布局、设备激活、权限状态与紧急收回。

行为以[文件传输契约](docs/FILE_TRANSFER.md)、[架构](docs/ARCHITECTURE.md)和[路线图](docs/ROADMAP.md)为准。

## 对比 macOS 连续互通与 Deskflow

| 功能 | macOS 连续互通 | Deskflow | uniDesk |
| --- | --- | --- | --- |
| 键鼠跨屏 | ✅ 仅限同 Apple 账号 | ⚠️ 屏幕位置无法自由摆放 | ✅ |
| 剪贴板同步 | ✅ 依赖 iCloud 稳定性不佳 | ⚠️ 仅基础文字、小图片 | ✅ |
| 文件传输 | ✅ AirDrop | ❌ | ✅ 支持文件夹 |
| 文件暂存抽屉 | ❌ | ❌ | ✅ |
| 剪贴板历史 | ❌ | ❌ | ✅ 跨设备同步，支持图片内容搜索 |
| 加密传输 | ✅ | ✅ TLS | ✅ QUIC |
| 开源 | ❌ | ✅ GPL-2.0 | ✅ MIT／Apache-2.0 |

对比依据（2026-09-15）：[Apple 通用控制](https://support.apple.com/en-us/HT212757)、[通用剪贴板](https://support.apple.com/en-us/102430)、[Deskflow](https://github.com/deskflow/deskflow)（文件拖放移除见 [#8459](https://github.com/deskflow/deskflow/issues/8459)）。

## 支持平台

| 平台 | 状态 |
| --- | --- |
| macOS 14+、Apple Silicon | 主平台，发行 arm64 DMG |
| macOS 26+ | 导航与操作层使用原生 Liquid Glass，旧系统回退标准材质 |
| Intel Mac | 暂不发行，不声明兼容 |
| Windows x64 | 原生适配器、SCM 服务与桌面客户端；文字/图片/富文本同步、Mac↔Windows 按需文件传输、双向键鼠共享已接入源码链路，安装器为实验性；构建通过不代表真实两机验收 |
| Linux | 不在首发范围 |

## 下载与安装

下载见 [GitHub Releases](https://github.com/GodD6366/uniDesk/releases)。

### Homebrew（macOS 14+、Apple Silicon）

```sh
brew trust --cask godd6366/unidesk/unidesk
brew tap godd6366/unidesk https://github.com/GodD6366/uniDesk
brew install --cask godd6366/unidesk/unidesk
```

`brew trust` 执行一次即可（旧版本无此命令时跳过）。cask 固定对应 DMG 的 SHA-256。卸载：`brew uninstall --cask godd6366/unidesk/unidesk`，不再使用时 `brew untap godd6366/unidesk`。

### 更新

Homebrew / GitHub 公开包在“设置 → 更新”中检查 GitHub Releases（包括预发行版），发现新版后可打开对应发布页。Homebrew 安装请执行：

```sh
brew update && brew upgrade --cask unidesk
```

内部签名包继续使用可信局域网自动更新；公开包不使用该安装通道。尚未包含 GitHub 自动安装功能的旧版需要先通过 Homebrew 或手动下载安装新版。

### 手动安装 DMG

1. 从同一 Release 下载 `uniDesk-arm64.dmg`、`SHA256SUMS`、`release.json`、`build-info.json`，运行 `shasum -a 256 -c SHA256SUMS`。
2. 打开 DMG，把 `uniDesk.app` 拖进 Applications；同一设备只运行一份实例。
3. 按提示允许局域网访问；键鼠共享还需辅助功能授权、可信局域网会话、设备激活和可用显示器布局。

配置 Apple 凭据后的 GitHub 预发行使用 Developer ID 签名并公证，可直接打开；不要关闭 Gatekeeper 或 SIP。更早的 ad-hoc 预发行若被拦截，用系统“隐私与安全性 → 仍要打开”，或 `sudo xattr -rd com.apple.quarantine "/Applications/uniDesk.app"`。维护者凭据准备见 [公证凭据](docs/NOTARIZE.md)。

## 从源码运行

需要 Apple Silicon Mac、完整 **Xcode 27+（macOS 27+ SDK）**、Rustup、Python 3.11+；Command Line Tools 不足。最低运行目标 macOS 14，工具链与依赖分别固定在 `rust-toolchain.toml` 和 `Cargo.lock`。

```sh
git clone https://github.com/GodD6366/uniDesk.git
cd uniDesk
rustup show active-toolchain
rustup target add aarch64-apple-darwin
rustup component add rust-src
CODESIGN_IDENTITY=- ./scripts/build_and_run.sh
```

无签名证书时显式加 `CODESIGN_IDENTITY=-`；加 `--build` 只构建不启动，多套 Xcode 用 `DEVELOPER_DIR=...` 单次选择。独立测试包用 `CODESIGN_IDENTITY=- ./scripts/package-macos.sh --bootstrap`，输出到 `dist/bootstrap/`，不参与局域网更新。

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

- Windows 服务包不是 GUI 或安装器，真实 SCM、会话切换和跨平台功能需独立验收；Windows 与 macOS 的跨设备文件传输已接入同一协议，但 OLE 延迟渲染、真实两机验收未完成。
- 原生 Finder 粘贴、拖出、权限变化、Wi-Fi 键鼠跨屏和真实两机行为不能由 CI 代替。
- GitHub CI 不签发局域网更新载荷；本机协调端正式包支持 macOS 局域网自动更新，旧版需先手动安装一次基础包，见[发行指南](docs/RELEASING.md)。
- 日志只记录元数据、状态和哈希；提交问题前检查附件，不上传剪贴板内容、设备密钥或凭据。

漏洞披露与传输认证边界见 [SECURITY.md](SECURITY.md)。

## 许可证

自有代码按 **MIT 或 Apache-2.0** 双许可证分发，由使用者选择；见 [LICENSE-MIT](LICENSE-MIT) 和 [LICENSE-APACHE](LICENSE-APACHE)。依赖与第三方资源保留各自许可证，见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
