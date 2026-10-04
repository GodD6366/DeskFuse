# DeskFuse Windows 客户端（WinUI 3 迁移中）

这是 `apps/deskfuse-windows`（WPF）的 WinUI 3 / Windows App SDK 迁移目标工程。迁移完成前两个工程并存：**WPF 仍是离线安装包与 NSIS 流程使用的权威客户端**，本工程只在 `-ClientProject` 指向它时参与打包。

## 当前状态

已接入：

- WinUI 3 宿主（`Microsoft.WindowsAppSDK` 1.8.260921001，`net8.0-windows10.0.19041.0`，`TargetPlatformMinVersion` 17763），自包含非打包部署（`WindowsAppSDKSelfContained`），无需预装 Windows App SDK 运行时。
- 与 WPF 版一致的应用调色板与导航骨架（剪贴板 / 键鼠共享 / 设备 / 传输 / 工作流），未接入的页面明确标注"迁移中"而不是静默降级。
- 认证命名管道客户端 `ServiceClient` 已完整移植：`DispatcherQueue` 取代 WPF `Dispatcher`（新增 `DispatcherQueueExtensions.EnqueueAsync`），服务 PID 核对、SCM 启动探测、请求/响应帧校验与错误映射保持不变。
- exe 清单使用 `app.store.manifest`（`asInvoker`、无 uiAccess），与 MSIX/Store 变体一致。
- Win32 剪贴板层 `NativeClipboard`（Win32 `AddClipboardFormatListener` + `WindowMessageHook` 子类化；`CF_UNICODETEXT`/`CF_HDROP`/`CF_DIB(V5)`/`HTML Format`/`Rich Text Format`/`PNG` 读写；保留原有的序列号、owner 标记、尺寸上限与回滚语义），PNG 编解码与 DIBV5 回退改用 GDI+。
- `ClipboardHistory`（含 WinUI 用的 `CreatedLabel`，因为 WinUI 绑定没有 `StringFormat`）、`ClipboardContent`/`ClipboardRepresentation` 模型、`ExplorerOleFileDataObject`（OLE 延迟渲染）。

已实测：在解锁的 console 会话上，本工程编译产物直接启动并与真实 `DeskFuseService` 完成认证 IPC（界面显示"服务运行中"，本机名与服务版本来自快照）。锁屏或非 console 会话下服务按设计返回 `session_unavailable`，与 WPF 客户端、PowerShell 探针行为一致。

尚未接入（WPF 版本仍提供）：

- `DesktopClipboardController`（本机捕获/同步/远端应用回执）与剪贴板页交互。
- 暂存区、设置、设备列表与配对、传输队列、工作流编辑。
- 托盘图标、全局快捷键、多显示器布局。
- 单实例与登录自启策略。

## 互操作替换计划

WPF 版依赖若干 WindowsDesktop/框架 API，迁移到 WinUI 3 时按下表替换，不引入 WinForms 依赖：

| WPF/框架用法 | WinUI 3 替换 |
|---|---|
| `System.Windows.Clipboard` + `Clipboard.GetFileDropList` | Win32 `OpenClipboard`/`GetClipboardData`（`CF_UNICODETEXT`/`CF_DIBV5`/`CF_HDROP`/`HTML Format`/`PNG`），沿用现有的序列号与 owner 校验逻辑 |
| `PngBitmapEncoder/Decoder`、`BitmapFrame`、`BitmapSource` | `System.Drawing.Common`（PNG 编解码、BGRA 像素取回）+ `Microsoft.UI.Xaml.Media.Imaging.BitmapImage`（预览） |
| `HwndSource.AddHook`（`WM_CLIPBOARDUPDATE`/`WM_HOTKEY`） | `WinRT.Interop.WindowNative.GetWindowHandle` + `SetWindowLongPtrW(GWLP_WNDPROC)` 子类化 |
| `System.Windows.Forms.Screen.AllScreens`、`System.Drawing.Rectangle` | `EnumDisplayMonitors`/`GetMonitorInfoW` |
| `System.Windows.Forms.NotifyIcon` 托盘 | `Shell_NotifyIconW` + `TrackPopupMenu`（图标经 `LoadImageW` 读取 `Assets/DeskFuse.ico`） |
| `System.Windows.Forms.FolderBrowserDialog` | `IFileOpenDialog`（`FOS_PICKFOLDERS`）COM |
| `MessageBox.Show` | `MessageBoxW`（避免把同步代码改成异步 `ContentDialog`） |
| `DispatcherTimer`、`Dispatcher.BeginInvoke` | `DispatcherQueue.CreateTimer`、`DispatcherQueue.TryEnqueue` |

## 边界

- 迁移不改变安全边界：客户端仍不持有身份、信任或网络运行时，跨设备能力仍由 `DeskFuseService` 的认证 IPC 提供；本工程不做任何绕过服务校验的本地回退。
- 迁移期间不修改服务、协议或 WPF 客户端行为；本工程未接入的页面必须显式标注不可用。
- 本工程与 `docs/WINDOWS_STORE.md` 的 MSIX 打包链路配合使用；UIAccess（跨 UIPI 输入）在 Store 变体中不可用，见该文档第 4 节。
