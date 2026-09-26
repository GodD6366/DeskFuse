# Windows 原生桌面客户端

原生 WPF / .NET 8 / Windows 11 x64，不包含 WebView。身份、信任与局域网运行时只由 `UniDeskService` 持有；桌面用户进程不启动备用网络或重建身份。

## 已接入

- 四页功能侧栏、剪贴板历史/预览三栏、独立设置、系统托盘和按用户/会话约束的单实例。再次打开会激活原实例；关闭窗口隐藏，从托盘退出结束桌面进程。
- 托盘最近 20 条历史，可显式恢复；主窗口选择只预览。搜索、内容类型与来源筛选、文字/图片/本机文件可用性预览，以及原生暂存区。
- 本机剪贴板历史与本机偏好持久化；停止记录不关闭单独启用的同步。用户偏好位于 `%LOCALAPPDATA%\uniDesk\desktop\settings-v1.json`，不复制服务业务状态。
- 通过经过 SCM 服务 PID 核对的桌面 Named Pipe v2，读取真实设备、邀请、连接、候选路径和能力状态。自动发现与手动 IPv4 邀请保留接收方确认。
- 文字/PNG/HTML/RTF 同步与应用回执；16 MiB 通道上限，选择或重启不重放历史。原生变更先失效旧远端意图，再选择性发布新内容；轮询前记录本机序列，旧任务不能代替新复制。锁屏/会话切换暂停原生读写。
- 跨设备文件按需传输（与 macOS 引擎同一帧协议）：本机 CF_HDROP 复制或暂存区「发送到设备」按需报价；远端报价在「传输」页显示（进度、状态、失败原因、缓存用量），选择目标文件夹后按需下载——服务端在保护目录内分块暂存并逐文件 SHA-256 校验，桌面把文件复制到目标位置并放入剪贴板后才向对端确认。接收文件重名自动改名，不静默覆盖；任务失败/取消/断线都会通知对端并清理暂存。
- 独立、签名的 `unidesk-input-helper.exe` 由 GUI 监督并以 `uiAccess` 运行，可跨 UIPI 向管理员窗口注入/采集输入；服务在 UDP 42425 上运行 input-only runtime 并持有身份、信任与租约。helper 仍不进入 Winlogon/安全桌面，安全桌面由服务会话边界单独管理。双向键鼠已接入：Mac 主控 → Windows 被控，以及 Windows 主控 → Mac 被控（设备页一键开启/停止，本地钩子采集走跨平台 SharingController 边缘逻辑，服务校验会话与单调批次去重）；1.5 秒原生释放看门狗、界面收回和 Ctrl+Alt+Win+Esc 紧急收回对两个方向同时生效。
- 显示器画布使用本机实际显示器与像素尺寸，并允许已启用主控拖动远端屏幕；位置通过 `MoveControlDevice` 交给 Rust 输入运行时吸附并持久化。输入快捷键编辑同样走服务 IPC，由 Rust 校验后保存。

## 明确未就绪

Explorer OLE 延迟渲染已接入：deferred manifest 先枚举完整目录树与稳定 lindex，首次 `FileGroupDescriptorW` 请求时才下载并校验内容；文件内容必须校验后才对 Explorer 可见。工作流目录、工作流快捷键和键鼠共享快捷键已接入认证桌面 IPC；工作流执行和跨设备规则共享已接入 capability-gated Rust 服务链路。安全桌面输入和自动升级仍未接入，UI 不声明可用。服务已预留独立 session-agent 状态，但当前没有安装或授权生产 agent；锁屏、UAC、登录输入及 SAS 仍为 false，任何 session/epoch/身份探测失败都会 fail-closed。agent 不能通过普通桌面 IPC 自授权，uniDesk 不注入或拦截 SAS，也不处理 Windows 凭据。本机改名与 macOS 一致使用操作系统主机名，设置页只读显示；`Rename` 服务命令仍明确拒绝。路径偏好尚未开放业务 IPC；候选和当前活动路径分开显示。普通桌面输入源码接线不等于管理员窗口、登录、UAC 或真实两机已验收。

## 构建与边界

在 Windows 使用仓库 `scripts/build-windows-desktop.ps1` 构建完整服务与桌面 payload，安装器保证客户端/服务版本和安装目录一致。桌面程序集也可在非 Windows 开发机交叉编译：

```sh
dotnet build apps/unidesk-windows/UniDesk.Windows.csproj -c Release
```

交叉编译只证明托管代码与 XAML 可构建，不证明 Windows 启动、服务授权、Explorer、真实剪贴板、多屏或跨系统验收。协议以 `docs/WINDOWS_DESKTOP_IPC.md` 为准。桌面进程与固定 helper 请求 `uiAccess`；只有受信任安装目录中的已签名镜像、且服务实际验证 helper token 的 UIAccess 位后才报告高完整性输入能力。安全桌面、登录桌面和 SAS 仍明确不支持。
