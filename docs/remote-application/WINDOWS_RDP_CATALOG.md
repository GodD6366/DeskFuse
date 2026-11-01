# Windows 远程应用目录协议 v2（Mac 主控 ↔ Windows 被控）

Mac 控制 Windows 远程应用不走 Mac↔Mac 的 ScreenCaptureKit 视频协议。Windows 服务自动发现本机应用，主控端通过已认证的 QUIC 控制连接取回目录和一次性启动票据；FreeRDP 的画面与输入经 DeskFuse 的 QUIC 隧道转发到 `Windows:127.0.0.1:<RDP 端口>`，TLS/NLA 仍由两端端到端完成。Mac 不再直连 `Windows:3389`，也不需要 Windows 防火墙开放该端口。

## 能力宣告

Windows 服务在控制 HELLO 中宣告 `REMOTE_APPLICATION`（`deskfuse_core::Capabilities`，`1 << 12`）和独立的 `REMOTE_APPLICATION_RDP`（`1 << 13`）。Mac 使用后者判断对端是「RDP RemoteApp 主机」。

## 连接与授权

1. Mac 对已配对 Windows 调用 `open_remote_application(peer)`，在既有认证 QUIC 连接上打开一条双向流（`RemoteSession`）。
2. Windows 服务仅向**已配对、已信任、未被阻止**的对端提供目录；版本不支持 RemoteApp（如 Home/Core）时返回 `available=false`。
3. 每条目录会话在独立阻塞线程上服务，`receive` 最多 250 ms/次轮询，不占用 Tokio worker。
4. 目录协议版本为 **2**。服务端拒绝 v1 请求并返回 `unsupported_version`，提示两端同时升级。

## 帧格式

`RemoteSession` 传输层已有 4 字节大端长度前缀；载荷是带 `type` 标签的 UTF-8 JSON。请求体上限 64 KiB。

### 服务端 → 主控端

```json
{"type":"catalog","version":2,"available":true,"reason":null,
 "host":{"port":3389,"certificate_sha256":"AA:BB:…:FF",
         "checks":[{"name":"edition","passed":true,"message":"Windows 版本支持远程桌面"}],
         "console_session":{"user":"godd","connected":true}},
 "apps":[{"name":"notepad","display_name":"记事本","path":"C:\\Windows\\System32\\notepad.exe",
          "command_line":"","icon_path":"","icon_index":0,
          "icon_png":"iVBORw0KGgo…"}]}

{"type":"ready","version":2,"name":"notepad",
 "ticket":[0,1,2,…31],
 "rdp":"full address:s:deskfuse-rdp.invalid\r\n…",
 "host":{"port":3389,"certificate_sha256":"AA:BB:…:FF","checks":[]}}

{"type":"error","version":2,"code":"preflight_failed","message":"Windows 远程桌面预检未通过"}
```

- `available` 只有在版本受支持、`host.checks` 全部通过、端口有效且证书指纹可读时才为 `true`；否则 `reason` 给出可操作提示。
- `host.port` 来自注册表 `WinStations\RDP-Tcp\PortNumber`，不是硬编码 3389。
- `host.certificate_sha256` 是 RDP 监听证书 DER 的 SHA-256（95 字符、冒号分隔）。Mac 只接受该指纹；证书变化直接拒绝，没有手动放行流程。
- `host.console_session` 是 Windows 活动控制台会话，可为 `null`。`connected=false` 表示会话已断开但用户仍保留登录；Mac 在建立新会话前据此提示会锁屏或结束该用户会话。旧版服务不发送该字段，按“无法判断”处理，不阻止连接。
- `apps` 只在 `available=true` 时返回。
- `apps[].icon_png` 是可选字段：64×64 PNG 的 base64。服务端用一次 PowerShell 批量从可执行文件提取，按路径与修改时间缓存；提取失败或超过 24 KB 的图标会省略，主控端回退到系统占位图标。
- `ready.ticket` 是 32 字节一次性票据，绑定请求设备和 30 秒有效期；隧道建立一次即作废。`rdp` 中的逻辑主机名不会被拨号，仅用于让 FreeRDP 解析 RemoteApp 配置。
- 错误码：`unsupported_version`、`request_too_large`、`invalid_request`、`unavailable`、`not_found`、`registration_failed`、`preflight_failed`、`prepare_failed`。

### 主控端 → 服务端

```json
{"type":"catalog","version":2}
{"type":"prepare","version":2,"name":"notepad"}
```

- `catalog` 重新扫描并返回当前目录，而不是重发会话建立时的快照。
- `prepare` 只接受目录 ID，不接受远端传入的可执行文件路径或命令行；服务端重新解析目录，首次启动时按需注册到 `TSAppAllowList`，完成预检后返回 `ready`。
- 后续应用复用同一条 RDP 会话：Mac 仍先发送 `prepare` 确保 Windows 侧已注册，再通过 RAIL `ClientExecute("||<name>")` 启动；不会建立第二条 RDP 登录。

## RDP 隧道

`ready` 之后，Mac 打开新的双向 QUIC 流，首帧为：

```text
Envelope.RemoteRdpTunnel { version: 1, ticket }
```

服务端校验票据（设备、时效、一次性）后回一帧 `RemoteRdpTunnel { version: 1, ticket: [] }` 作为确认，随后双方转发原始字节；每块不超过 64 KiB。Windows 服务连接 `127.0.0.1:<host.port>`，不连接任何对端提供的地址。任一端 EOF、授权撤销或设备被阻止时，两端同时关闭；双向使用有界缓冲，QUIC 流控会施加背压。

Mac 的 C 桥通过 `freerdp_set_io_callbacks` 替换 `TCPConnect`/`ConnectLayer`，返回 `socketpair` 的一端；另一端交给 Rust 隧道。FreeRDP 的网关、代理、UDP multitransport 和自动重连均关闭，因此不存在绕过隧道建立连接的回退路径。

## 剪贴板（cliprdr）

- 会话内启用 cliprdr，双向交换文本（`CF_UNICODETEXT`，UTF-16LE，结尾一个 NUL）和图片（`CF_DIB`）。
- 图片使用 32 位 BI_RGB、自下而上的行；接收端也接受 24 位 BI_RGB 和自上而下的负高度。压缩（RLE/JPEG/PNG）、调色板和位域变体不宣告也不解析，避免猜测。
- 图片按 4096 像素边长、约 33 MB 像素量截断/拒绝；接收时统一按不透明处理，因为 Windows 常见 32 位剪贴板 DIB 的 alpha 字节是 0。
- Mac 端每 0.4 秒比较一次 `NSPasteboard.changeCount`；文本优先，其次图片。变化时先公布格式列表，再提供数据。Windows 请求时若数据尚未就绪，桥接层暂存该请求，Mac 补齐数据后立即应答。
- 远端剪贴板变化时，桥接层把格式 id 交给 Mac；Mac 按文本、图片的顺序取第一个可用格式写入本机剪贴板，同时同步 `changeCount`，避免把刚写入的内容再回传形成回环。
- HTML/RTF 和文件列表尚未宣告，Windows 不会收到无法应答的格式。`ClipboardFeatureMask` 只打开本地↔远端的数据方向，不含文件列表。
- 会话期间 DeskFuse 自身的剪贴板同步对这台 Windows 保持停用（`SessionBlocker::RemoteSession`），避免两条路径互相覆盖。

## 声音

- `.rdp` 使用 `audiomode:i:0`；FreeRDP 的 `rdpsnd` 通道把服务端音频交给 macOS `mac` 子系统（AVAudioEngine）在本机播放，桥接层打开 `AudioPlayback`。
- `rdpsnd` 在 FreeRDP 的静态通道表中声明依赖 `rdpdr`，加载时会强制打开设备重定向通道。构建包含 `rdpdr`，但没有配置驱动器、打印机或智能卡，因此不会转发任何设备。
- FreeRDP 只向服务端宣告本构建支持的音频格式；未启用 GSM 解码，服务端会改用双方都支持的格式（通常是 PCM）。

## 断线重连

- 会话中断后主程序保留窗口，按 1/2/4/8 秒退避重试，总预算 60 秒；关闭窗口立即取消。
- 每次尝试重新走目录 `prepare`、打开新隧道并创建新的 RDP 会话；成功后用 RAIL 重新启动之前打开的应用，窗口不重建。
- 重连使用打开会话时输入的凭据，只保存在内存；`stop()` 丢弃。失败 60 秒后窗口显示最终原因，用户可重新打开应用。
- 本机会话占用提示只在首次建立会话时出现，自动重连不重复弹窗。
- 密码错误、密码过期或凭据缺失会触发一次重新输入密码的弹窗，确认后用新密码立即重连；取消则停止重试并保留错误。账户被锁定、禁用或过期不弹窗，直接显示原因。
- 系统睡眠前主动断开隧道并记为 `sleep`，唤醒后按同一重连策略恢复；窗口保留，不重复本机会话提示。

## 设置与诊断

- 选择器工具栏的齿轮菜单提供「播放 Windows 声音」「同步剪贴板」两个开关，保存在本机 `UserDefaults`（`remoteRdp.audioEnabled`、`remoteRdp.clipboardEnabled`），默认开启，下次建立会话时生效。
- 关闭声音时桥接层不加载 `rdpsnd`；关闭剪贴板时 Mac 不挂载 cliprdr 桥接，Windows 侧也不会收到格式列表。
- 会话诊断事件：`rdp_session_started`（peer/audio/clipboard）、`rdp_connected`（`connect_ms`）、`rdp_first_frame`（`first_frame_ms`）、`rdp_session_closed`（`reason`、`uptime_ms`、`frames`、`fps`）；另有 `rdp_bridge_failed`、`rdp_bridge_disconnected`、`rdp_auth_failed`、`rdp_reconnected`、`rdp_reconnect_failed`、`rdp_catalog_*`。
- 远程窗口右上角的诊断按钮显示同一组指标（连接/首帧耗时、在线时长、帧数、平均帧率、声音/剪贴板开关），不需要打开日志文件。

## 兼容与发现范围

- 自动发现范围：HKLM 的 64/32 位 `App Paths`，以及系统目录中的记事本、画图、计算器入口；尚不覆盖所有开始菜单、用户级安装和 Store 应用。
- 合并人工注册项，路径去重；人工注册名及启动参数优先。自动发现项使用规范化路径的 SHA-256 生成稳定 ID。
- 自动发现路径必须是存在的本机绝对 `.exe` 路径，拒绝 UNC 路径和换行注入。

## 实现位置

- 目录 schema 与校验：`apps/deskfuse-service/src/remote_rdp_protocol.rs`（跨平台回归测试）。
- 注册表/预检/目录/`.rdp`：`apps/deskfuse-service/src/native/remote_app.rs`（`rdp_port` / `host_status` / `rdp_file` / `serve`）。
- 隧道接收与回环转发：`apps/deskfuse-service/src/native/network.rs`（`serve_remote_applications`）。
- 票据、隧道收发与背压：`crates/deskfuse-transport/src/rdp_tunnel.rs`。
- FFI 与 fd 所有权：`crates/deskfuse-ffi/src/remote_application.rs`（`open_remote_rdp_tunnel`）。
- C 桥：`apps/deskfuse-macos/RemoteApplication/Sources/DeskFuseRemoteApplication/RdpBridge/RdpBridge.c`。
- Swift：`RemoteRdpCatalog.swift`、`RemoteRdpSession.swift`、`RemoteRdpHostConnection.swift`、`RemoteRdpClipboard.swift`。

## 边界

- 目录协议只交换元数据、预检结果和票据；不承载 RDP 字节流，也不承载 Windows 账户密码。
- RDP TLS/NLA 由 FreeRDP 与 Windows 端到端完成；DeskFuse 服务只转发密文。
- 服务端不读取 Windows 账户凭据；凭据由 Mac 端 RDP 客户端提示输入，或从用户明确启用的本机 Keychain 记录读取。
- Windows Home / Core 版不宣告该能力，也不提供目录。
