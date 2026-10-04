# 第三方来源与许可

项目的双许可证仅适用于自有代码／文档；下列第三方材料按各自许可保留，不因放入本仓库而重新授权。

## Rust 依赖

- 版本和来源以根 `Cargo.lock` 为准；Rust 共享引擎通过 UniFFI 生成 Swift 绑定，原生客户端使用 Apple SwiftUI/AppKit。
- `scripts/third-party-notices.py` 读取 locked Cargo metadata，导出目标平台依赖声明和可获得的许可证正文；它包含构建／开发依赖，因此不是“全部组件均链接入应用”的声明。
- DMG 内 `DeskFuse.app/Contents/Resources/THIRD_PARTY_DEPENDENCIES.txt` 包含导出的声明、来源和许可证，另附项目双许可证和 Lucide 许可。
- 部分 crate 发布归档不附独立许可文件。已从相应 upstream revision 补取的文本及哈希见 `resources/licenses/manifest.json`；归档无 VCS revision 的记录明确标注来源限制，不伪称为精确发行时文本。
- 少数锁定依赖只提供许可声明时，按已审阅记录保留 package 作者、声明与所选许可标准条款；声明原文哈希固定，不编造上游版权行。
- 标准 MIT 文本来自 SPDX license-list-data commit `3ac5a9c241d97f95b22a5e366c9c841404a35639`。标准模板不是上游版权声明；已移除不再属于当前依赖图的历史许可副本。

重新生成：

```sh
python3 scripts/third-party-notices.py --output dist/THIRD_PARTY_DEPENDENCIES.txt
```

未知依赖缺少许可文本且不在已审阅记录中时，生成器失败，要求补齐来源，而不是静默略过。

## UniFFI

UniFFI 0.32.1 及其绑定运行时按 MPL-2.0 分发，项目未修改其上游源码。对应源码可从 [Mozilla UniFFI 固定版本](https://github.com/mozilla/uniffi-rs/tree/35a47433d8f015302fbf608f2b395eff6972c36f) 获取；许可证全文随依赖声明打包。生成的应用接口使用项目自身类型，不改变上游文件的许可。

## 图片编码

图片工作流使用 `webp` crate 及其 `libwebp-sys` 依赖静态构建的 libwebp 编码 WebP；实际锁定版本、各自许可及版权声明由上述依赖声明生成器收录。`libwebp-sys` 绑定层缺少独立许可正文，按已审阅记录保留 MIT 声明、作者及标准条款；其发布归档中的 `vendor/COPYING`、`PATENTS`、`AUTHORS` 另行按固定哈希收录，不将 libwebp 误标为 MIT。PNG、JPEG、HEIC 和 TIFF 使用系统 ImageIO，不把 Apple 框架复制进应用。

## 图标与图形

- `resources/icons/`：Lucide SVG，见该目录 README 与 LICENSE；保留 Lucide／Feather 的原始署名。原生客户端控件使用系统 SF Symbols。
- `resources/DeskFuse-icon.svg`、`deskfuse-status-icon.svg`、`dmg-background.svg`：项目维护的品牌／安装图形源，按项目双许可证分发；派生 PNG、ICNS 和中心图由脚本生成。
- 原生系统符号／设备表现由平台 API 提供，不从 macOS 系统资源提取二进制素材加入仓库。
- `design/current/` 的 v5 与菜单栏原图来自 Atelier，来源标识与 SHA-256 见同目录清单；布局参考为用户提供的截图，图标预览来自项目 SVG。设计图不属于原生应用运行证据，不随应用打包。
- 早期设计参考中的风景照来自 [Unsplash 图片源](https://images.unsplash.com/photo-1476514525535-07fb3b4ae5f1)；原始示例照片及旧 HTML 样稿已清理，此处保留来源署名。[Unsplash License](https://unsplash.com/license) 单独适用，不属于项目双许可证。

## .NET / Windows 客户端依赖

Windows 客户端（`apps/deskfuse-windows`）通过 NuGet 引用下列包，版本在 `DeskFuse.Windows.csproj` 中固定，由 `dotnet restore` 解析：

- `Microsoft.WindowsAppSDK` 1.8.x（WinUI 3 / Windows App SDK，MIT）。自包含部署会把框架 DLL 随发行载荷分发。
- `Microsoft.Windows.SDK.BuildTools` 10.0.26100.x（Windows SDK 构建工具，按 Microsoft SDK 许可条款，含可再分发组件）。
- `Microsoft.Win32.SystemEvents` 8.0.0（MIT）：会话锁屏/解锁通知。
- `System.Drawing.Common` 8.0.0（MIT）：仅用于 Windows 剪贴板 `CF_DIBV5` 回退的 DIB 转换。

每个包自带其版权与许可文本（`~/.nuget/packages/<id>/<version>/`），上述列表不替代这些原文；本项目未修改任何上游源码。清单与哈希核对属于构建流程（`scripts/build-windows-desktop.ps1`、`scripts/package-windows-store.ps1`），不在此重复声明具体文件哈希。

## 参考项目

BlueSync 用于交互行为参考；DeskFuse 使用独立 Rust 实现、协议及状态格式。Deskflow 仅作为行为研究背景，不因参考行为把其代码或测试材料复制进本项目。来源声明不能替代对实际引入代码的逐项审查。

[OpenLogi](https://github.com/AprilNEA/OpenLogi)（MIT/Apache-2.0，© AprilNEA）是 Logitech Options+ 的开源替代项目，以 Rust 实现 HID++ 2.0 协议。DeskFuse 的鼠标 HID 支持参考了其设备兼容列表和 feature 寻址方式（reprogrammable controls 0x1b04、UnifiedBattery 0x1004、SmartShift 0x2110），协议知识来自其公开代码中针对各 feature 的逆向说明；DeskFuse 使用自有 Swift/IOKit 实现，未复制其 Rust 源码。

## Windows RDP RemoteApp 客户端

macOS 客户端静态链接 FreeRDP/WinPR 3.14.1（Apache-2.0）、OpenSSL 3.6.5（Apache-2.0）和 json-c 0.18（MIT）。固定源码提交及构建入口见 `scripts/build-freerdp-macos.sh`；打包时将上游许可证原文合并为 `DeskFuse.app/Contents/Resources/THIRD_PARTY_RDP.txt`。这些原生依赖不在 Cargo.lock 内，单独随产物附带声明。

## Roc 麦克风驱动与音频编码

跨设备麦克风使用 Roc VAD v0.0.4（MPL-2.0），以及固定提交的 Roc Toolkit（MPL-2.0）、libASPL（MIT）和 gRPC 等依赖。固定提交、源归档 SHA-256 和传递依赖记录位于 `vendor/roc-vad/sources.lock.json`。私有驱动标识、回环端点限制、读取状态、会话清理与构建适配保留在 vendored 源码及修改说明中，不改变上游文件的 MPL-2.0 许可。

完整覆盖源码、DeskFuse 修改、重建脚本及实际使用的依赖源码/许可证保存在维护者按构建号归档的构建缓存中，不放入 DMG 或应用/更新包。应用的 `Contents/Resources/Microphone/THIRD_PARTY_MICROPHONE.txt` 保留许可证及署名，`source-archive.json` 标识对应源码归档的文件名、大小、SHA-256 和获取方式。需要源码时，向提供测试版的维护者索取，并提供应用版本、构建号及该 SHA-256。驱动不自动安装，安装/卸载入口仅操作 兼容配置中的驱动 Bundle ID。Apple Core Audio 和系统 zlib 通过系统链接，不重新分发系统框架。
