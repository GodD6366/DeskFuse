# Windows 远程应用目录协议（Mac 主控 ↔ Windows 被控）

Mac 控制 Windows 远程应用不走 Mac↔Mac 那套 ScreenCaptureKit 视频协议，而是：
Windows 服务自动发现本机应用，在主控端点击启动时按需注册为原生 **RDP RemoteApp**（`TSAppAllowList`），Mac 端通过现有已认证 QUIC 控制连接取回应用目录与 `.rdp`，再交给 Mac 内嵌的 **RDP 客户端（FreeRDP）** 连 `Windows:3389` 无缝启动单个应用。

## 能力宣告

Windows 服务在控制 HELLO 的能力位中额外宣告 `REMOTE_APPLICATION`（`deskfuse_core::Capabilities`，`1 << 12`），并宣告独立的 `REMOTE_APPLICATION_RDP` 能力。Mac 端使用后者判断对端是「RDP RemoteApp 主机」而非「媒体帧主机」。

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

服务端对 `catalog` 重新扫描并返回当前目录，而不是重发会话建立时的快照。

- 自动发现范围：HKLM 的 64/32 位 `App Paths`，以及存在于系统目录中的记事本、画图、计算器可执行入口；尚不覆盖所有开始菜单、用户级安装和 Store 应用。
- 合并人工注册项，路径去重；人工注册名及启动参数优先。自动发现项使用规范化路径的 SHA-256 生成稳定 ID，不依赖显示名称。
- 扫描仅读取，不修改注册表。`launch` 只接受目录 ID，不接受远端传入的可执行文件路径或命令行；服务端重新解析目录，首次启动时注册后再返回 `.rdp`。
- 自动发现路径必须是存在的本机绝对 `.exe` 路径，拒绝 UNC 路径和换行注入。未知或已移除项返回 `error/not_found`，注册失败返回 `error/registration_failed`。
- 目录会话在轮询及处理请求时重新检查设备授权，撤销信任后关闭会话。独立 RDP 会话的 Windows 认证仍由 Windows 管理。
- Mac 独立窗口与侧栏内嵌目录都按 `remoteAppRdp` 选择 Windows JSON 协议，不再将 Windows 目录交给 Mac 媒体协议解析。

此自动目录改动不等于 Windows 单点登录：当前仍使用 NLA 用户凭据，尚未实现首次免密登录集成。

## 服务端实现位置

- 注册表/目录/`.rdp`：`apps/deskfuse-service/src/native/remote_app.rs`（`register_app` / `list_remote_apps` / `rdp_file` / `serve`）。
- 能力宣告与授权：`apps/deskfuse-service/src/native/network.rs`（`serve_remote_applications` / `remote_app_authorized`）。
- 会话轮询：`apps/deskfuse-service/src/native/mod.rs` 服务主循环的 `desktop_tick` 分支。

## 边界

- 本协议只交换元数据（应用名、路径、`.rdp`），不承载 RDP 字节流；RDP 由主控端直接连 `Windows:3389`。
- 服务端不读取 Windows 账户凭据；凭据由主控端 RDP 客户端在连接时提示输入。
- Windows Home / Core 版不宣告该能力，也不提供目录。
