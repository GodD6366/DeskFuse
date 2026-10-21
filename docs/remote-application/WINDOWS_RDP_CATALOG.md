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
         "checks":[{"name":"edition","passed":true,"message":"Windows 版本支持远程桌面"}]},
 "apps":[{"name":"notepad","display_name":"记事本","path":"C:\\Windows\\System32\\notepad.exe",
          "command_line":"","icon_path":"","icon_index":0}]}

{"type":"ready","version":2,"name":"notepad",
 "ticket":[0,1,2,…31],
 "rdp":"full address:s:deskfuse-rdp.invalid\r\n…",
 "host":{"port":3389,"certificate_sha256":"AA:BB:…:FF","checks":[]}}

{"type":"error","version":2,"code":"preflight_failed","message":"Windows 远程桌面预检未通过"}
```

- `available` 只有在版本受支持、`host.checks` 全部通过、端口有效且证书指纹可读时才为 `true`；否则 `reason` 给出可操作提示。
- `host.port` 来自注册表 `WinStations\RDP-Tcp\PortNumber`，不是硬编码 3389。
- `host.certificate_sha256` 是 RDP 监听证书 DER 的 SHA-256（95 字符、冒号分隔）。Mac 只接受该指纹；证书变化直接拒绝，没有手动放行流程。
- `apps` 只在 `available=true` 时返回。
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
- Swift：`RemoteRdpCatalog.swift`、`RemoteRdpSession.swift`、`RemoteRdpHostConnection.swift`。

## 边界

- 目录协议只交换元数据、预检结果和票据；不承载 RDP 字节流，也不承载 Windows 账户密码。
- RDP TLS/NLA 由 FreeRDP 与 Windows 端到端完成；DeskFuse 服务只转发密文。
- 服务端不读取 Windows 账户凭据；凭据由 Mac 端 RDP 客户端提示输入，或从用户明确启用的本机 Keychain 记录读取。
- Windows Home / Core 版不宣告该能力，也不提供目录。
