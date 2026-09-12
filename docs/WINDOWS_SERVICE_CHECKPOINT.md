# Windows 服务开发检查点

> 本包仅含 Windows x64 服务端和只读状态客户端，**不是 uniDesk GUI、完整安装包或自动更新载荷**。
> 源码、编译、IPC 回归与真实 SCM/两机验收分开记录。内部包未做发行签名；各 EXE 的 Authenticode 状态见 manifest.json。

## 已接入的真实实现

- `unidesk-service.exe`：仅接受 Windows Service Control Manager 启动。SCM 状态、停止/关机、会话变化通知；不存在直接 console/network fallback。
- 网络与身份属于服务：LocalSystem/session 0 的 DPAPI 身份；受保护 ProgramData 目录、排他的 runtime lease；真实 SQLite trust/state、QUIC control 与 mDNS。
- 服务只声明 `NETWORK`，不提前声明 clipboard、transfer、input、LAN updater。IPC 目前只有 `Status`，没有配对批准、用户内容、任意命令、任意路径、提权或安装接口。
- `unidesk-service-client.exe status`：连接固定 pipe；写请求前用 SCM 查询核对服务端 PID；不会在服务缺失时启动另一套身份或网络。

## IPC 与会话边界

- 固定 pipe：`\\.\pipe\com.unidesk.app.service.v1`；first-instance 排他创建；拒绝远程 pipe 客户端。
- DACL：SYSTEM full access；Interactive Users 只获数据读写、不含 `FILE_CREATE_PIPE_INSTANCE`。没有 Everyone/Network 授权。
- 客户端以具体数据权限 `FILE_GENERIC_READ | FILE_WRITE_DATA | FILE_WRITE_ATTRIBUTES | FILE_WRITE_EA` 打开，不申请 pipe-instance 位；使用 overlapped I/O，并只给 `SECURITY_IDENTIFICATION`。服务同步 impersonate 后查询实际 token session、logon LUID、Interactive SID；所有路径先 RevertToSelf 再返回 async executor，恢复失败终止服务进程。
- token session 必须与 pipe session、活动物理 console session 相同，且非 session 0、非 SYSTEM、未锁定、非 RDP。SID/LUID 只用于内存鉴权，不写入 IPC 状态或诊断。
- 读/写前校验 session epoch，SCM 会话变化或注销/锁屏使旧请求失效。当前只有元数据；这不是未来用户内容路由或系统桌面 helper 的完成证明。
- 单连接仅一项请求，JSON 严格字段与版本号，16 KiB 帧上限，最多 8 个处理任务，3 秒 I/O 期限；Win32 本地鉴权查询同步执行，不声称 timeout 能强行中断原生系统调用。

## 运行与安装边界

Windows 本机登录桌面 PowerShell：

```powershell
.\unidesk-service-client.exe status
```

服务尚未安装时会明确失败。直接运行 `unidesk-service.exe` 也会失败，这是预期行为。

目前未提供自动注册服务脚本或 NSIS 安装器。不要将服务注册到 Downloads、桌面或其他普通用户可写目录，也不要以管理员 GUI 常驻替代服务。完整安装器必须先将 EXE/helper/依赖安装到受限的 Program Files 目录，再注册 `UniDeskService` 为 LocalSystem、独立进程、自动启动；服务 DACL不得允许普通用户重配程序路径。首装提权、升级事务、回滚及移除服务保留配对数据仍待完成。

服务在现有内部 QUIC 42424 端口监听并使用 mDNS；本包不修改防火墙、系统策略、账户或配对数据。局域网能否连接必须在后续安装和两机验收中验证。

## SCM 错误码

服务使用 Win32 `ERROR_SERVICE_SPECIFIC_ERROR` 携带以下服务专用码；不把可能包含用户数据的错误字符串写到共享 IPC：

| 码 | 含义 |
| --- | --- |
| 1 | 未分类服务/IPC错误 |
| 2 | 服务入口捕获异常 |
| 20 | 服务目录、ACL、排他 runtime lease 校验失败 |
| 21 | 持久身份载入/创建失败；不自动重建已有身份 |
| 22 | SQLite 初始化或持久 trust/state 校验失败 |
| 23 | 本机签名 Hello 构造失败 |
| 24 | QUIC listener 绑定失败（包括端口占用） |
| 25 | mDNS 初始化/广告失败 |
| 26 | 网络 actor 构造失败 |
| 27 | 异步启动等待超时 |
| 30 | 网络退出超时；为防止后台写入与目录 lease 分离，终止服务进程 |

原生 SCM/pipe 错误也可能以原始系统错误码上报，需结合阶段判断。新版本安装自检/回滚尚未接入，不把服务运行状态当作升级成功回执。

## 未完成的部分

后续原生桌面/托盘与业务 IPC、配对操作、剪贴板/文件隔离路由、Windows 安全 receiver/OLE、输入 helper/抑制 handoff、锁屏/UAC/登录输入、NSIS、自动安装与回滚。此包不能用于验收这些功能。

## 构建与证据

- 检查：`scripts/check-windows-service.ps1`（固定 Rust/MSVC x64、service/identity 单测、目标包 strict clippy、两个 Release EXE）。
- 打包：`scripts/package-windows-service.ps1 -SourceSnapshot <source-snapshot.json>`；对固定源码列表逐文件校验，构建后再次校验，检查 PE x64、SHA-256 与 Authenticode 状态，只打包两个 EXE 和三份说明/元数据。
- 输出：`dist/windows-service/uniDesk-windows-service-x64.zip`。这是开发检查包，无发行签名，不可导入 LAN 自动更新。
- CI 验证仅覆盖源码、编译和隔离 fixture；不代表真实 LocalSystem SCM 启动与 GUI console token 接入已验收。
- ACL 回归验证具体权限客户端双向 I/O 正常，同款数据权限不能创建第二个 server 实例。构建机 fixture 中默认 generic-write 客户端也能打开，因此不把“默认 generic-write 必然被拒绝”作为已证实结论；生产客户端仍明确采用最小具体权限。
