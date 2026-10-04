# Windows 原生桌面客户端（WinUI 3）

Windows 11 x64 原生客户端，使用 **WinUI 3 / Windows App SDK**（`net8.0-windows10.0.19041.0`，最低运行版本 Windows 10 1809）。身份、信任与局域网运行时只由 `DeskFuseService` 持有；桌面用户进程不启动备用网络或重建身份。

> 历史：本目录原为 WPF/WinForms 客户端，已就地转换为 WinUI 3；旧实现保留在 git 历史中。迁移细节与 Win32 替换对照见文末。

## 已接入

- 四页功能侧栏（剪贴板 / 键鼠共享 / 设备 / 传输 / 工作流）、剪贴板历史三栏（列表 / 筛选 / 预览）、设置窗口、暂存区窗口、RemoteApp 管理窗口、系统托盘和按用户/会话约束的单实例。再次打开会激活原实例；关闭窗口隐藏，从托盘退出结束桌面进程。
- 托盘最近 20 条历史可显式恢复；主窗口选择只预览。搜索、内容类型与来源筛选、文字/图片/本机文件可用性预览。
- 本机剪贴板历史与本机偏好持久化（`%LOCALAPPDATA%\DeskFuse\desktop`）；停止记录不关闭单独启用的同步。
- 通过经过 SCM 服务 PID 核对的桌面 Named Pipe v2 读取真实设备、邀请、连接、候选路径和能力状态。自动发现与手动 IPv4 邀请保留接收方确认。
- 文字/PNG/HTML/RTF 同步与应用回执；16 MiB 通道上限，选择或重启不重放历史。原生变更先失效旧远端意图，再选择性发布新内容。
- 跨设备文件按需传输：本机 CF_HDROP 复制或暂存区「发送到设备」按需报价；远端报价在「传输」页显示，选择目标文件夹后按需下载，桌面把校验后的文件复制到目标位置并放入剪贴板后才向对端确认。重名自动改名，不静默覆盖。
- 键鼠共享：设备页一键开启/停止主控，显示器画布使用真实显示器与像素尺寸，已启用主控可拖动远端屏幕；快捷键编辑走服务 IPC，由 Rust 运行时校验并持久化。`deskfuse-input-helper.exe` 由 GUI 监督并以 `uiAccess` 运行（仅签名版可用；CI 构建会在运行时验证其能力）。
- 工作流目录、工作流快捷键与执行链路（capability-gated Rust 服务链路）。
- 资源管理器 OLE 延迟渲染：deferred manifest 先枚举完整目录树与稳定 lindex，首次 `FileGroupDescriptorW` 请求时才下载并校验内容。

## 明确未就绪

安全桌面输入、登录桌面、SAS 与自动更新仍未接入，UI 不声明可用。服务已预留独立 session-agent 状态，但当前没有安装或授权生产 agent；锁屏、UAC、登录输入及 SAS 仍为 false，任何 session/epoch/身份探测失败都会 fail-closed。

## 构建与边界

```powershell
# 仅客户端（自包含 Windows App SDK，无需预装运行时）
dotnet build apps/deskfuse-windows/DeskFuse.Windows.csproj -c Release

# 完整离线载荷（服务 + 客户端 + helper + 安装器）
.\scripts\build-windows-desktop.ps1 -ReleaseMetadata .\release.json
.\scripts\package-windows.ps1 -SourceSnapshot <snapshot> -ReleaseMetadata .\release.json -Offline

# Microsoft Store / MSIX
.\scripts\package-windows-store.ps1
```

`ApplicationManifest` 默认是 `app.store.manifest`（`asInvoker`、无 uiAccess），因此未签名的离线载荷也能启动；只有签名并安装在受信任目录的发行版才应使用 `-p:ApplicationManifest=app.manifest`（或 `build-windows-desktop.ps1 -UiAccess`）请求 uiAccess。Windows 会拒绝启动未签名的 uiAccess 二进制（Win32 740）。

交叉编译只证明托管代码与 XAML 可构建，不证明 Windows 启动、服务授权、Explorer、真实剪贴板、多屏或跨系统验收。协议以 `docs/WINDOWS_DESKTOP_IPC.md` 为准；Store 发布范围与限制见 `docs/WINDOWS_STORE.md`。

## WinUI 3 迁移说明

Win32 替换对照（不引入 WinForms 依赖）：

| WPF/框架用法 | WinUI 3 实现 |
|---|---|
| `System.Windows.Clipboard` + `Clipboard.GetFileDropList` | `NativeClipboard`：Win32 `AddClipboardFormatListener`（经 `WindowMessageHook` 子类化）与 `CF_UNICODETEXT`/`CF_HDROP`/`CF_DIB(V5)`/`HTML Format`/`Rich Text Format`/`PNG` 读写；序列号、owner 标记、prepare→commit 与容量上限语义不变 |
| `PngBitmapEncoder/Decoder`、`BitmapFrame`、`BitmapSource` | `System.Drawing.Common`（CF_DIBV5 回退与 PNG 编解码）+ `BitmapImage`（预览） |
| `HwndSource.AddHook`（`WM_CLIPBOARDUPDATE`/`WM_HOTKEY`） | `WindowMessageHook`（`SetWindowLongPtrW` + `CallWindowProcW` 链式窗口过程） |
| `System.Windows.Forms.Screen.AllScreens` | `MonitorLayout`（`EnumDisplayMonitors`/`GetMonitorInfoW`） |
| `System.Windows.Forms.NotifyIcon` | `TrayIcon`（`Shell_NotifyIconW` + `TrackPopupMenu`） |
| `System.Windows.Forms.FolderBrowserDialog` | `Dialogs.PickFolder`（`IFileOpenDialog`，`FOS_PICKFOLDERS`） |
| `MessageBox.Show` | `Dialogs.Warning`（`MessageBoxW`，保持同步调用点） |
| `Dispatcher`/`DispatcherTimer` | `DispatcherQueue`/`DispatcherQueueTimer`（含 `DispatcherQueueExtensions.EnqueueAsync`） |
| `DockPanel`/`WrapPanel`/`GridSplitter`/`ContextMenu` | `Grid` 布局、`StackPanel` 行、固定分隔 `Border`、`MenuFlyout` |

迁移不改变安全边界：客户端仍不持有身份、信任或网络运行时，跨设备能力仍由 `DeskFuseService` 的认证 IPC 提供。

已实测（2026-10-05，解锁 console 会话）：客户端启动后与真实服务完成认证 IPC（界面「服务运行中」）；本机复制文本被 Win32 监听捕获、写入共享历史并在列表与预览中显示；MSIX 打包的 full-trust 客户端从 `WindowsApps` 启动后同样连通服务。锁屏或非 console 会话下服务按设计返回 `session_unavailable`。
