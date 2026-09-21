# Windows 原生桌面客户端

原生 WPF / .NET 8 / Windows 11 x64，不包含 WebView。身份、信任与局域网运行时只由 `DeskFuseService` 持有；桌面用户进程不启动备用网络或重建身份。

## 已接入

- 四页功能侧栏、剪贴板历史/预览三栏、独立设置、系统托盘和按用户/会话约束的单实例。再次打开会激活原实例；关闭窗口隐藏，从托盘退出结束桌面进程。
- 托盘最近 20 条历史，可显式恢复；主窗口选择只预览。搜索、内容类型与来源筛选、文字/图片/本机文件可用性预览，以及原生暂存区。
- 本机剪贴板历史与本机偏好持久化；停止记录不关闭单独启用的同步。用户偏好位于 `%LOCALAPPDATA%\DeskFuse\desktop\settings-v1.json`，不复制服务业务状态。
- 通过经过 SCM 服务 PID 核对的桌面 Named Pipe v2，读取真实设备、邀请、连接、候选路径和能力状态。自动发现与手动 IPv4 邀请保留接收方确认。
- 文字/PNG/HTML/RTF 同步与应用回执；16 MiB 通道上限，选择或重启不重放历史。原生变更先失效旧远端意图，再选择性发布新内容；轮询前记录本机序列，旧任务不能代替新复制。锁屏/会话切换暂停原生读写。
- 独立 `deskfuse-input-helper.exe` 由 GUI 监督，只在当前已解锁 Default desktop 执行固定注入；服务在 UDP 42425 上运行 input-only runtime 并持有身份、信任与租约。当前先接入 Mac 主控 → Windows 被控、1.5 秒原生释放看门狗、界面收回和 Ctrl+Alt+Win+Esc 紧急收回。
- 只读显示器画布使用本机实际显示器与像素尺寸，不使用样例设备、壁纸或假布局。

## 明确未就绪

跨设备文件/OLE 按需提供、Windows 作为主控的跨屏捕获/布局、安全桌面输入和自动升级仍未接入，UI 不声明可用。本机暂存区只拖出已存在的本机文件。本机改名入口遵从服务答复；服务未支持时显示拒绝，不只修改界面名称。路径偏好尚未开放业务 IPC；候选和当前活动路径分开显示。普通桌面输入源码接线不等于管理员窗口、登录、UAC 或真实两机已验收。

## 构建与边界

在 Windows 使用仓库 `scripts/build-windows-desktop.ps1` 构建完整服务与桌面 payload，安装器保证客户端/服务版本和安装目录一致。桌面程序集也可在非 Windows 开发机交叉编译：

```sh
dotnet build apps/deskfuse-windows/DeskFuse.Windows.csproj -c Release
```

交叉编译只证明托管代码与 XAML 可构建，不证明 Windows 启动、服务授权、Explorer、真实剪贴板、多屏或跨系统验收。协议以 `docs/WINDOWS_DESKTOP_IPC.md` 为准。进程只请求常规用户权限，不启用 `uiAccess`。
