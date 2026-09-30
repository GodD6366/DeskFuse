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
- `scripts/build-freerdp-macos.sh`：构建 FreeRDP 3.14.1（headless、静态、仅 RemoteApp 相关 channel）+ 编译 RdpBridge → `apps/deskfuse-macos/RemoteApplication/Vendor/FreeRDP/libdeskfuse-rdp.a`（本机已验证，见下）。
- `RemoteApplication/Sources/DeskFuseRemoteApplication/RdpBridge/RdpBridge.c` / `.h`：FreeRDP 无头客户端桥（连接线程、GDI 帧缓冲拷贝、鼠标/键盘/Unicode 注入、证书自动接受、NLA 凭据回调）。
- `RemoteApplication/Sources/DeskFuseRemoteApplication/RemoteRdpBridge.swift`：`RdpBridge.c` 的 Swift 包装（帧拷贝 + 输入 + 状态）。
- `RemoteApplication/Sources/DeskFuseRemoteApplication/RemoteRdpCatalog.swift`：目录协议 JSON 类型（`RemoteRdpMessage`/`RemoteRdpRequest`/`RemoteRdpApp`）。
- `Sources/DeskFuse/RemoteRdpSession.swift`：主控端编排（取目录 → 发 launch → 收 `.rdp` → 替换 `{host}` → 启动桥）。
- `Package.swift`（主 app + spike）：`CDeskFuseRdp` systemLibrary + 链接 `-ldeskfuse-rdp -lssl -lcrypto -lusb-1.0 -ljson-c` + Foundation/CoreServices/Security。

## 已验证（本机 macOS）

| 项 | 结果 |
| --- | --- |
| FreeRDP 3.14.1 静态构建 | ✅ 通过（trim 后仅 3 个 .a，见脚本） |
| `RdpBridge.c` 编译 | ✅ 0 警告 |
| 组合 `libdeskfuse-rdp.a` + 链接测试 | ✅ 链接并运行（create/parse/free） |
| `swift build --target DeskFuseRemoteApplication` | ✅ 通过（含 RemoteRdpBridge/RemoteRdpCatalog） |

未能在本机验证：Windows 服务 Rust 代码（需 Windows/MSVC 构建机，见 `docs/WINDOWS_SELF_HOSTED_RUNNER_SETUP.md`）；主 app 打包（需完整 Xcode 27+，本机只有 CommandLineTools）。

## 打包测试步骤

### 1. Windows（被控 B）
```powershell
.\scripts\snapshot-windows-source.ps1 -OutputPath "$env:TEMP\deskfuse-source.json"
.\scripts\package-windows.ps1 -SourceSnapshot "$env:TEMP\deskfuse-source.json"
```
安装后：把要共享的应用注册进 RemoteApp（当前通过桌面 IPC 命令；GUI 管理页是后续项）。确认 Windows 为 Pro/Enterprise/Server（Home 不支持）。

### 2. Mac（主控 A）
```sh
./scripts/build-freerdp-macos.sh          # 产出 Vendor/FreeRDP/libdeskfuse-rdp.a
./scripts/package-macos.sh                # 正常打包（需完整 Xcode + Rust FFI）
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
