# Windows 远程应用（Mac 主控 → Windows 被控）实施状态与打包测试

方向已确认：**核心是 Mac↔Windows，先做 Mac 控制 Windows**；Mac 端内嵌 **FreeRDP**（方向 A）。

## 架构

```
Mac (主控 A)                              Windows (被控 B)
────────────                              ────────────────
RemoteRdpSession ──JSON catalog/launch──▶ deskfuse-service (Rust)
   │  (已认证 QUIC RemoteSession 流)         ├─ remote_app.rs: TSAppAllowList 注册表 + 目录 + .rdp
   │                                        └─ network.rs: REMOTE_APPLICATION 能力 + 授权门禁
   │
   └─ RemoteRdpBridge ──RDP(3389)────────▶ Windows RemoteApp (rdpinit → 单个应用)
        (RdpBridge.c + FreeRDP 静态库)        注册在 TSAppAllowList 的应用
```

真正的 RDP 字节流不经过 DeskFuse QUIC；Mac 拿到 `.rdp` 后直连 `Windows:3389`。

## 已实现（本分支）

### Windows 服务侧（Rust，`apps/deskfuse-service/`）
- `src/native/remote_app.rs`：`TSAppAllowList` 注册表读写（`register_app`/`unregister_app`/`list_remote_apps`）、`App Paths` 应用枚举、Home/Core 版检测、`.rdp` 生成、**JSON 目录协议 + `serve()` 会话循环**。
- `src/native/network.rs`：控制 HELLO 加 `REMOTE_APPLICATION` 能力位；`serve_remote_applications()` + `remote_app_authorized()`（配对+信任+未阻止）。
- `src/native/mod.rs`：主循环 `desktop_tick` 轮询并服务远程应用会话。
- `src/desktop_protocol.rs`：`remote_app` 能力位 + `ListRemoteApps`/`ListInstalledApps`/`RegisterRemoteApp`/`UnregisterRemoteApp`/`RemoteAppConnection` 命令（Windows GUI 本地管理用）。
- 协议契约：`docs/remote-application/WINDOWS_RDP_CATALOG.md`。

### Mac 侧（C + Swift）
- `scripts/build-freerdp-macos.sh`：构建 FreeRDP 3.14.1（headless、静态、仅 RemoteApp 相关 channel）+ 编译 RdpBridge → `apps/deskfuse-macos/RemoteApplication/Vendor/FreeRDP/lib/libdeskfuse-rdp.a`（本机已验证，见下）。
- `RemoteApplication/Sources/DeskFuseRemoteApplication/RdpBridge/RdpBridge.c` / `.h`：FreeRDP 无头客户端桥（连接线程、GDI 帧缓冲拷贝、鼠标/键盘/Unicode 注入、证书自动接受、NLA 凭据回调）。
- `RemoteApplication/Sources/DeskFuseRemoteApplication/RemoteRdpBridge.swift`：`RdpBridge.c` 的 Swift 包装（帧拷贝 + 输入 + 状态）。
- `RemoteApplication/Sources/DeskFuseRemoteApplication/RemoteRdpCatalog.swift`：目录协议 JSON 类型（`RemoteRdpMessage`/`RemoteRdpRequest`/`RemoteRdpApp`）。
- `Sources/DeskFuse/RemoteRdpSession.swift`：主控端编排（取目录 → 发 launch → 收 `.rdp` → 替换 `{host}` → 启动桥）。
- `Package.swift`（主 app + spike）：`CDeskFuseRdp` systemLibrary；FreeRDP、WinPR、OpenSSL 和 json-c 合并为一个静态库，仅链接 macOS 系统库/框架，不依赖 Homebrew dylib。

## 早期验证记录（不代表当前完整验收）

| 项 | 结果 |
| --- | --- |
| FreeRDP 3.14.1 静态构建 | ✅ 通过（trim 后仅 3 个 .a，见脚本） |
| `RdpBridge.c` 编译 | ✅ 0 警告 |
| 组合 `libdeskfuse-rdp.a` + 链接测试 | ✅ 链接并运行（create/parse/free） |
| `swift build --target DeskFuseRemoteApplication` | ✅ 通过（含 RemoteRdpBridge/RemoteRdpCatalog） |

上述记录来自早期环境。当前构建需完整 Xcode 27+；Windows 服务、真实 NLA 登录、RemoteApp 画面和输入仍需 Windows/MSVC 及双设备验收，不由本地桥接测试替代。

## 打包测试步骤

### 1. Windows（被控 B）
```powershell
.\scripts\snapshot-windows-source.ps1 -OutputPath "$env:TEMP\deskfuse-source.json"
.\scripts\package-windows.ps1 -SourceSnapshot "$env:TEMP\deskfuse-source.json"
```
安装后：把要共享的应用注册进 RemoteApp（当前通过桌面 IPC 命令；GUI 管理页是后续项）。确认 Windows 为 Pro/Enterprise/Server（Home 不支持）。

### 2. Mac（主控 A）
```sh
./scripts/build-native-macos.sh release   # 自动准备 RDP 依赖并构建主程序
./scripts/package-macos.sh --lan           # 正常打包，不自动安装
./scripts/build-freerdp-macos.sh --test     # 原生 RAIL/GDI 回归测试
```

## 已接入主 UI（RDP 与媒体能力的边界）

- `Device` 的活动路由能力宣告包含 `REMOTE_APPLICATION_RDP` 时，`RemoteApplicationManager` 进入 Windows RDP picker；没有该能力的设备继续使用 Mac↔Mac PropertyList 媒体会话。不会根据设备名猜测平台，也不会把两种首帧协议混在一条会话中。
- `RemoteRdpPicker` 展示 Windows 注册目录、NLA 用户名/密码字段和可用性错误；每个应用建立自己的 `RemoteRdpSession` 与原生 RDP 窗口。RDP framebuffer 按服务端实际 resize 动态分配并在 C mutex 下复核尺寸。
- RDP 输入映射包含鼠标、滚轮、键盘扫描码和 Unicode；凭据只传给本次 FreeRDP 连接，不写入 DeskFuse。RDP 证书仍按配对 LAN 边界自动接受。

RDP RemoteApp 是 Windows 原生 `rdpinit.exe`/NLA 会话，不等价于 Mac↔Mac 的 ScreenCaptureKit 编码流：没有媒体协议的暂停、HEVC/H.264 偏好、Surface layout 或共享控制租约。UI 明确标注该差异，不能宣称跨平台能力完全相同。

Windows 桌面端通过 `RemoteAppWindow` 调用受认证的服务 IPC 管理 `TSAppAllowList`，不会由普通用户 GUI 直接写 HKLM。
5. **`{host}` 解析**：`RemoteRdpSession.peerHost` 由调用方从 `Device.endpoint`/route 去掉端口得到。

## 已知边界

- 证书当前**自动接受**（LAN 配对设备信任边界内）；持久化证书固定是后续项。
- Home/Core 版 Windows 不支持 RemoteApp（服务端 `availability()` 返回不支持并拒绝目录）。
- RDP 凭据只由 Mac 端 RDP 客户端处理，DeskFuse 服务不读 Windows 账户凭据。

## 可复现的 macOS RDP 依赖构建

- 需要完整 Xcode、CMake、Git、Perl、Make；首次准备需访问上游 Git 仓库。不要求安装 Homebrew 的 OpenSSL、json-c 或 libusb。
- FreeRDP 3.14.1、OpenSSL 3.6.5、json-c 0.18 均固定上游提交；按 `DESKFUSE_MACOS_ARCH=arm64|x86_64`、SDK 和 macOS 14.0 部署目标构建。缓存默认在 `~/.cache/deskfuse-freerdp`，可用 `FREERDP_CACHE_DIR` 修改。
- `build-native-macos.sh` 和独立 spike 入口先准备静态依赖；Swift 构建缓存包含 RDP 指纹。依赖升级、桥接源码或工具链改变后自动重建。
- 未实现的 USB、音频等重定向通道不启用；没有通过关闭 RDP target 绕过构建。RAIL 握手发送应用启动请求，通道事件接入 GDI graphics pipeline，保留 bitmap/glyph 解码。
- `--test` 使用真实 FreeRDP 库检查 `.rdp` 解析、RAIL 两种握手及失败回调、GDI 帧拷贝边界和重复清理，不连接 Windows、不使用账户凭据。
- 打包拒绝非系统 dylib 依赖，并携带 `THIRD_PARTY_RDP.txt` 中的原生库许可证。公开发布、签名公证和双设备功能验证分别判定。
