# Windows 远程应用目录协议（Mac 主控 ↔ Windows 被控）

Mac 控制 Windows 远程应用不走 Mac↔Mac 那套 ScreenCaptureKit 视频协议，而是：
Windows 服务把应用注册为原生 **RDP RemoteApp**（`TSAppAllowList`），Mac 端通过现有已认证 QUIC 控制连接取回应用目录与 `.rdp`，再交给 Mac 内嵌的 **RDP 客户端（FreeRDP）** 连 `Windows:3389` 无缝启动单个应用。

## 能力宣告

Windows 服务在控制 HELLO 的能力位中额外宣告 `REMOTE_APPLICATION`（`deskfuse_core::Capabilities`，`1 << 12`）。Mac 端据此判断对端是「RDP RemoteApp 主机」而非「媒体帧主机」。

## 连接与授权

1. Mac 对已配对 Windows 调用 `open_remote_application(peer)`，在既有认证 QUIC 连接上打开一条双向流（`RemoteSession`）。
2. Windows 服务仅向**已配对、已信任、未被阻止**的对端提供目录（`remote_app_authorized`）；且当前 Windows 版本必须支持 RemoteApp（`EditionID` 不以 `Core` 开头，即非 Home 版）。不满足即关闭会话。
3. 每条会话在独立阻塞线程上服务，`receive` 最多 250 ms/次轮询，不占用 Tokio worker。

## 帧格式

`RemoteSession` 传输层已做 4 字节大端长度前缀。本协议在载荷内直接使用 **UTF-8 JSON**（不用 Apple PropertyList，保证 Rust 与 Swift 共享同一 schema）。两端消息均为**带 `type` 标签的 JSON 对象**。

### 服务端 → 主控端

```json
{"type":"catalog","version":1,"available":true,"reason":null,"apps":[
  {"name":"Notepad","display_name":"记事本","path":"C:\\Windows\\System32\\notepad.exe",
   "command_line":"","icon_path":"","icon_index":0}
]}

{"type":"rdp","version":1,"rdp":"full address:s:{host}:3389\r\nprompt for credentials on client:i:1\r\nremoteapplicationmode:i:2\r\nremoteapplicationprogram:s:||Notepad\r\nremoteapplicationname:s:记事本\r\ndisableremoteappcapscheck:i:1\r\nalternate shell:s:rdpinit.exe\r\naudiomode:i:2\r\n"}

{"type":"error","version":1,"code":"not_found","message":"该应用尚未注册为 RemoteApp"}
```

`catalog.available=false` 时附带 `reason`，`apps` 为空。`rdp` 中的 `{host}` 由主控端替换为对端解析出的 IPv4（连接目标 `{host}:3389`）。

### 主控端 → 服务端

```json
{"type":"catalog","version":1}
{"type":"launch","version":1,"name":"Notepad"}
```

服务端对 `catalog` 重发当前目录；对 `launch` 返回对应 `.rdp`（未注册返回 `error/not_found`）。

## 服务端实现位置

- 注册表/目录/`.rdp`：`apps/deskfuse-service/src/native/remote_app.rs`（`register_app` / `list_remote_apps` / `rdp_file` / `serve`）。
- 能力宣告与授权：`apps/deskfuse-service/src/native/network.rs`（`serve_remote_applications` / `remote_app_authorized`）。
- 会话轮询：`apps/deskfuse-service/src/native/mod.rs` 服务主循环的 `desktop_tick` 分支。

## 边界

- 本协议只交换元数据（应用名、路径、`.rdp`），不承载 RDP 字节流；RDP 由主控端直接连 `Windows:3389`。
- 服务端不读取 Windows 账户凭据；凭据由主控端 RDP 客户端在连接时提示输入。
- Windows Home / Core 版不宣告该能力，也不提供目录。
