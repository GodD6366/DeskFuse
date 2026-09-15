# uniDesk

[English](README.en.md) · [开发指南](CONTRIBUTING.md) · [发行指南](docs/RELEASING.md) · [计划与待办](TODO.md)

uniDesk 是面向可信设备的原生桌面协作工具：同步剪贴板、传输文件、暂存与拖出文件，并提供独立开关的键鼠共享。使用 **Rust 2024 + SwiftUI**，macOS 优先。

> 当前为开发预发行项目。macOS 首发提供 Apple Silicon 测试包；Windows 仅为实验性平台／服务实现，不是可安装的完整桌面产品。构建成功不代表真实两机验收完成。

界面以当前实际应用为准；交互约定见 [当前界面与行为约定](design/current/README.md)。首个公开构建后补充真实界面截图，当前不放设计稿或示意图。

## 能做什么

- 发现与配对可信设备，连接状态与权限阻碍可见。
- 同步文本、图片和原生剪贴板格式；保留文件／文件夹名称和结构。
- 文件按需传输，可选择小文件提前传输；历史缓存有容量限制和清理回执。
- 240 × 240 pt 文件暂存抽屉：拖入、展开、整批拖出，发送通过二级设备菜单。
- 独立的键鼠共享工作区：真实显示器布局、设备激活、权限状态与紧急收回。

文件行为以[文件传输契约](docs/FILE_TRANSFER.md)为准；实现边界与计划见[架构](docs/ARCHITECTURE.md)及[路线图](docs/ROADMAP.md)。

## 支持平台

| 平台 | 状态 |
| --- | --- |
| macOS 14+、Apple Silicon | 当前主平台，发行 arm64 DMG |
| macOS 26+ | 导航与操作层使用原生 Liquid Glass；旧系统使用标准材质回退 |
| Intel Mac | 暂不提供发行包，不声明兼容性 |
| Windows x64 | 原生适配器与服务检查点；GUI、安装器、完整跨平台协作尚未交付 |
| Linux | 不在首发范围 |

## 下载与安装

下载入口：[GitHub Releases](https://github.com/GodD6366/uniDesk/releases)。如果没有预发行版，请先从源码构建。

### Homebrew（macOS 14+、Apple Silicon）

公开仓库同时作为 Homebrew tap，cask 指向同一 GitHub Release 的 arm64 DMG：

```sh
brew trust --cask godd6366/unidesk/unidesk
brew tap godd6366/unidesk https://github.com/GodD6366/uniDesk
brew install --cask godd6366/unidesk/unidesk
```

Homebrew 7 及以上会先要求信任第三方 tap，`brew trust` 只需执行一次（旧版本没有该命令时跳过）。cask 固定对应版本 DMG 的 SHA-256，由发行工作流在预发行公开后刷新。

本阶段包为 ad-hoc 签名、未公证，Homebrew 安装后仍带隔离属性；首次安装后清除标记再启动，不要关闭 Gatekeeper 或 SIP：

```sh
sudo xattr -rd com.apple.quarantine "/Applications/uniDesk.app"
```

卸载：`brew uninstall --cask godd6366/unidesk/unidesk`；不再使用时 `brew untap godd6366/unidesk`。

### 手动安装 DMG

1. 下载同一版本的 `uniDesk-arm64.dmg`、`SHA256SUMS`、`release.json` 和 `build-info.json`，在下载目录运行 `shasum -a 256 -c SHA256SUMS`。
2. 打开 DMG，将 `uniDesk.app` 拖到 Applications。同一设备只运行一份实例。
3. 本阶段包为 **ad-hoc 签名、未公证**，macOS 可能阻止启动。请先核对来源与校验和；若系统提供“隐私与安全性 → 仍要打开”，可自行确认。不要关闭 Gatekeeper 或 SIP。正式 Developer ID 分发尚未接入。
4. 按系统提示允许局域网访问；键鼠共享需要辅助功能授权，并要求可信物理局域网会话、设备激活和显示器布局可用。

设备身份保存在 `~/Library/Application Support/uniDesk/identity.key`，权限为 `0600`，应用不访问钥匙串。首次从钥匙串版本切换时创建本地身份，需要重新配对；旧钥匙串条目不读取、不删除。升级或遇到配对问题时，不要删除身份文件、清空系统权限或同时运行旧包与开发包来绕过错误。

## 从源码运行

需要 Apple Silicon Mac、完整 **Xcode 27+（macOS 27+ SDK）**、Rustup 和 Python 3.11+；仅安装 Command Line Tools 不足以构建当前 SwiftUI / App Intents 客户端。构建 SDK 与运行系统不同：应用最低运行目标仍为 macOS 14。Rust 工具链固定在 `rust-toolchain.toml`，依赖固定在根 `Cargo.lock`。

```sh
git clone https://github.com/GodD6366/uniDesk.git
cd uniDesk
rustup show active-toolchain
rustup target add aarch64-apple-darwin
rustup component add rust-src
CODESIGN_IDENTITY=- ./scripts/build_and_run.sh
```

没有签名证书的贡献者显式使用 `CODESIGN_IDENTITY=-` 进行 ad-hoc 本地构建；有既有签名配置时省略该变量。只构建、不启动：`CODESIGN_IDENTITY=- ./scripts/build_and_run.sh --build`。多套 Xcode 可通过 `DEVELOPER_DIR=/Applications/Xcode-beta.app/Contents/Developer` 为本次命令选择 Xcode 27，不修改系统全局设置。

独立构建不参与 LAN 自动更新的测试 DMG：`CODESIGN_IDENTITY=- ./scripts/package-macos.sh --bootstrap`，默认输出到 `dist/bootstrap/`，不读取维护者发行私钥、不写发行 ledger、不上传 NAS、不导入本机更新。完整检查与提交准备见 [开发指南](CONTRIBUTING.md) 和 [开源提交说明](docs/OPEN_SOURCE.md)。

## 项目结构

```text
apps/           SwiftUI 应用、Windows 服务、诊断探针
crates/         领域模型、协议、身份、存储、网络、传输、输入及平台实现
scripts/        统一的开发、检查、资源生成、打包与发行工具
Casks/          公开 Homebrew cask（同时作为 tap）
resources/      原始 SVG、bundle 元数据、第三方图标、公开传输引导材料
design/current/ 当前界面行为规范与图标资源
docs/           架构、功能契约、发布与支持边界
.github/        CI、预发行工作流、贡献模板和依赖更新
```

`target/`、`dist/`、`artifacts/` 和历史设计不属于公开源码。详细开发约定见 [CONTRIBUTING.md](CONTRIBUTING.md)。

## 已知限制与安全反馈

- Windows 服务包不是 GUI／安装器；真实 SCM、会话切换与跨平台功能需要独立验收。
- 原生 Finder 粘贴、拖出、权限变化、Wi-Fi 下键鼠跨屏及真实两机行为不能由 CI 代替。
- GitHub CI 不签发局域网更新载荷。本机协调端正式包已支持 macOS 局域网自动更新；旧版需手动安装一次基础包，流程见 [发行指南](docs/RELEASING.md)。
- 日志只应包含元数据、状态与哈希；提交问题前检查附件，不上传剪贴板内容、设备密钥或凭据。

漏洞披露方式和传输认证边界见 [SECURITY.md](SECURITY.md)。

## 许可证

自有代码按 **MIT 或 Apache-2.0** 双许可证分发，由使用者选择其一；见 [LICENSE-MIT](LICENSE-MIT) 和 [LICENSE-APACHE](LICENSE-APACHE)。依赖和第三方资源保留各自许可证，见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
