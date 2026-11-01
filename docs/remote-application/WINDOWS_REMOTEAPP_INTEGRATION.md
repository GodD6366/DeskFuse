# Windows 远程应用（Mac 主控 → Windows 被控）实施状态

方向：**Mac 控制 Windows**，Mac 端内嵌 FreeRDP。当前完成 P0、P1 的代码与自动测试，并接入剪贴板文本与本机会话占用提示；P1 实机验收、无缝窗口、声音和其余 P4–P7 尚未完成，进度以 `RDP_COMPLETION_PLAN.md` 为准。

## 架构

```
Mac (主控 A)                                   Windows (被控 B)
────────────                                    ────────────────
RemoteRdpSession ──JSON catalog/prepare v2──▶ deskfuse-service (Rust)
   │  (已认证 QUIC 目录流)                        ├─ remote_app.rs: 预检 + TSAppAllowList + 目录 + .rdp
   │                                              └─ network.rs: 票据校验 + 回环 RDP 连接
   │
   └─ RdpBridge.c / FreeRDP ──socketpair──▶ Rust 隧道 ──QUIC RemoteRdpTunnel──▶ 127.0.0.1:<RDP 端口>
        (TLS/NLA 端到端，DeskFuse 只转发密文)              Windows RemoteApp
```

RDP 字节流经过 DeskFuse 的 QUIC 隧道，不再直连 `Windows:3389`，也不需要为局域网开放 RDP 防火墙端口。隧道只允许连接 Windows 服务本机的 RDP 监听端口，端口从注册表读取。

## 已实现

### Windows 服务侧（Rust，`apps/deskfuse-service/`）

- `src/remote_rdp_protocol.rs`：目录协议 v2 schema、64 KiB 请求上限、版本/应用名校验，跨平台单测。
- `src/native/remote_app.rs`：`TSAppAllowList` 注册表读写、`App Paths` 应用枚举、Home/Core 检测、`.rdp` 生成、`rdp_port()`、`host_status()`（版本/`fDenyTSConnections`/监听端口/证书指纹）、`serve()` 目录循环。
- `src/native/network.rs`：控制 HELLO 的 `REMOTE_APPLICATION_RDP` 能力；目录会话和隧道都经过配对+信任+未阻止门禁；撤销授权时隧道在 100 ms 内关闭。
- `src/native/mod.rs`：服务主循环轮询目录会话和隧道。

### Mac 侧（C + Swift）

- `RdpBridge.c`：FreeRDP 3.14.1 无头客户端；通过 `freerdp_set_io_callbacks` 替换 `TCPConnect`/`ConnectLayer`，只使用 `socketpair`；证书只接受服务端下发的 SHA-256；关闭网关、代理、UDP multitransport、自动重连和重定向；提供 `rdp_bridge_take_tunnel_fd()` 与 `rdp_bridge_execute()`。
- `RemoteRdpBridge.swift`：创建会话、交出隧道 fd、启动、帧拷贝、输入、RAIL 追加启动、剪贴板事件透传。
- `RemoteRdpClipboard.swift`：文本（UTF-16LE）与图片（`CF_DIB` 24/32 位 BI_RGB）双向桥接；桥接层负责 cliprdr 握手、格式列表和延迟应答，本机只公布这两类格式。
- 声音：FreeRDP 的 `rdpsnd` 通道加 macOS `mac` 子系统（AVAudioEngine）在本机播放；桥接层打开 `AudioPlayback`，`.rdp` 使用 `audiomode:i:0`。`rdpsnd` 依赖 `rdpdr`，构建同时启用该通道，但没有配置任何设备重定向。
- 自动重连：`RemoteRdpHostConnection` 在隧道或会话失败后保持窗口，按 `RemoteRdpReconnectPolicy`（1/2/4/8 秒退避，60 秒预算）重建目录会话、隧道和 RDP 会话，再用 RAIL 重新打开之前的应用。凭据只存在内存，`stop()` 立即丢弃。
- 应用图标：Windows 服务批量提取 64×64 PNG（System.Drawing），按路径与修改时间缓存；目录以 base64 传输 `icon_png`，Mac 选择器渲染真实图标，失败时回退系统符号。
- 设置与诊断：选择器齿轮菜单可关闭声音或剪贴板（下次会话生效）；`RemoteRdpSessionMetrics` 记录连接/首帧/在线时长与帧率，写入 `rdp_session_started`、`rdp_connected`、`rdp_first_frame`、`rdp_session_closed` 诊断。
- `RemoteRdpCatalog.swift`：协议 v2 类型（`RemoteRdpHost`/`RemoteRdpHostCheck`/`RemoteRdpMessage`/`RemoteRdpRequest`）。
- `RemoteRdpSession.swift`：目录/预检浏览；`prepare` 返回一次性票据。
- `RemoteRdpHostConnection.swift`：**每台 Windows 只保留一条 RDP 连接**；第一个应用用 `.rdp` 启动，后续应用用 RAIL `ClientExecute` 在同一会话启动。
- `RemoteApplicationManager.swift` / `RemoteRdpViews.swift`：每台 Windows 一个 RDP 窗口；删除 `peerHost`、`windowsHost()`、`{host}` 替换、手动证书信任和 Keychain 指纹条目。

### 已验证的自动测试（2026-10-21）

| 项 | 结果 |
| --- | --- |
| `cargo check --locked --workspace` | 通过 |
| `cargo test --workspace --all-targets --locked` | 通过 |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 通过 |
| `build-freerdp-macos.sh --test` | 通过（证书 pin、RAIL 握手的单次启动、错误回调、socketpair fd 一次性交接、GDI 帧边界、重复清理） |
| `test-remote-application.py` | 44 个 Swift 测试逐个通过（含目录 v2、票长、预检、指纹测试） |

### 尚未完成（按里程碑）

- P1 实机项：Windows 防火墙关闭 3389 仍可连接；记事本+画图共用一条连接；雷电/Wi-Fi 切换。
- P2：RAIL 窗口表与一窗一 Mac 窗口、脏区/IOSurface、几何换算、窗口拖动/缩放/层级。
- P3：完整扫描码/输入法/⌘ 映射、滚轮和光标。
- P4：富文本/文件剪贴板、RDPGFX/disp 画质（文本与图片剪贴板、声音已接入）。
- P5：完成（错误分类、本机会话占用提示、自动重连、睡眠唤醒与凭据重新输入）。
- P6：开始菜单/用户级/Store 发现、注册表清理、Windows 一键开启（真实图标已接入）。
- P7：发布验收（设置开关、会话级诊断与窗口诊断面板已接入）。

## 打包测试步骤

### Windows（被控 B）

```powershell
.\scripts\snapshot-windows-source.ps1 -OutputPath "$env:TEMP\deskfuse-source.json"
.\scripts\package-windows.ps1 -SourceSnapshot "$env:TEMP\deskfuse-source.json"
```

安装后确认 Windows 为 Pro/Enterprise/Server。远程桌面未开启时会出现在目录的 `host.checks` 中；P6 的一键开启尚未实现，目前需在 Windows 本机设置中开启。

### Mac（主控 A）

```sh
./scripts/build-native-macos.sh release
./scripts/package-macos.sh --lan
./scripts/build-freerdp-macos.sh --test
python3 scripts/test-remote-application.py
```

## UI 与能力边界

- `Device` 的活动路由能力包含 `REMOTE_APPLICATION_RDP` 时进入 Windows picker；否则使用 Mac↔Mac 媒体会话。
- 凭据只传给本次 FreeRDP 连接；用户明确启用“记住凭据”时保存在本机 macOS Keychain，不经过 Windows 服务或 QUIC 目录协议。
- RDP 证书指纹来自 Windows 服务的监听证书查询；Mac 自动固定，证书变化直接拒绝，没有手动放行入口。
- 当前仍是一个 RDP 会话对应一个 Mac 窗口，多个远程应用在一个画布内显示；P2 才会拆成一远端窗口对应一个 Mac 原生窗口。
- RDP RemoteApp 不等价于 Mac↔Mac 的 ScreenCaptureKit 媒体流：没有媒体协议的暂停、HEVC/H.264 偏好、Surface layout 或共享控制租约。

## 可复现的 macOS RDP 依赖构建

- 需要完整 Xcode 27+、CMake、Git、Perl、Make；首次准备需访问上游 Git 仓库，不要求 Homebrew 的 OpenSSL、json-c 或 libusb。
- FreeRDP 3.14.1、OpenSSL、json-c 固定上游提交；按 `DESKFUSE_MACOS_ARCH=arm64|x86_64`、SDK 和 macOS 14.0 部署目标构建。缓存默认在 `~/.cache/deskfuse-freerdp`。
- 桥接源码变化只重编译 `RdpBridge.c`，不会让固定版本的 FreeRDP 编译缓存失效。
- RAIL、DISP、RDPGFX、CLIPRDR、RDPSND、RDPDR 已编进静态库；CLIPRDR 只接入文本（图片、富文本、文件列表未宣告），RDPSND 通过 macOS 后端播放，DISP/RDPGFX 尚未启用；USB、打印机、智能卡等设备重定向不配置。
- 打包拒绝非系统 dylib 依赖，并携带第三方许可证。签名、公证、公开发布和双设备功能验收分别判定。
