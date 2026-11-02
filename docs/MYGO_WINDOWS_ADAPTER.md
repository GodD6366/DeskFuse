# MyGo UI · Windows 适配层

状态：传输与快照、设备/配对/文件/局域网共享、内容库读取与内容/缩略图、键鼠设置命令、工作流目录命令已接入；
桌面宿主原生能力已接入——本地剪贴板写入/粘贴、原生保存/定位/打开、工作流输出写入剪贴板、
文件暂存区（含拖出与发送）、更新检查（GitHub Releases，channel=manual 如实反映无自动安装器）、
卸载、应用选择与图标、全局快捷键录制、键鼠主控接管与本机壁纸、诊断导出、通知、
窗口动作、开机自启与保持唤醒、系统托盘图标与全局剪贴板快捷键（Ctrl+Shift+V）。
键鼠拖动预览吸附、文件设置/清缓存、传输取消/重试/发送、系统快捷指令目录等已通过
**扩展桌面协议**（`PreviewControlDeviceMove`/`SetFileSettings`/`ClearFileCache`/`CancelTransfer`/
`RetryTransfer`/`GetSystemShortcuts`）接入，服务与适配层同步实现。
仍**未接入**（平台无能力，见下表）：OCR、鼠标物理按键录制、设备产品图等。
实机验证见文末「实机验证（2026-10-31）」；双设备验收仍未完成。
边界事实来源：`docs/WINDOWS_DESKTOP_IPC.md`、`apps/deskfuse-service/src/desktop_protocol.rs`。

## 边界

```text
MyGo WebView UI (Go)  ──named pipe v2──▶  deskfuse-service.exe
        │                                        │
        └── 展示快照、提交命令                     └── 身份、信任、网络、剪贴板策略
```

- 服务是唯一权威。UI 不打开第二个运行时，不自行判定设备信任、路由或剪贴板归属。
- 服务在每次连接上校验内核管道会话、token SID、登录 LUID、物理控制台状态和会话代次；
  客户端不携带自己的凭据。
- 能力标记只来自服务 `Snapshot.capabilities`；不由 `runtime.GOOS` 推断，也不因页面存在而点亮。
- 适配层只做协议翻译与命令映射。服务没有的命令返回真实错误，不用空实现或模拟数据补齐。

## 协议

| 项 | 值 |
| --- | --- |
| 管道 | `\\.\pipe\com.deskfuse.app.desktop.v2`（`DESKFUSE_WINDOWS_PIPE` 可覆盖） |
| 帧 | 4 字节小端无符号长度 + JSON UTF-8，1…25,165,824 字节 |
| 版本 | `version: 2`；`request_id` 进程内单调递增 |
| 连接 | 一次请求一次响应；读完响应后写单字节 `0x06` 确认再关闭 |
| 请求过期 | 服务侧 3 秒 |
| 刷新 | 首次 `Snapshot` 打开剪贴板代次；之后每次用 `Poll{session_epoch, after_sequence}` 复用该代次并推进游标 |
| 拒绝 | `{"Rejected":{"code","message","boundary_error"}}`；`boundary_error` 为准，`code` 仅兼容旧客户端 |

实现：`apps/deskfuse-go/internal/winservice`

| 文件 | 职责 |
| --- | --- |
| `client.go` | 帧、请求编号、`0x06` 确认与拒绝映射 |
| `pipe_windows.go` / `pipe_other.go` | 命名管道连接；非 Windows 主机只保留可编译桩 |
| `service.go` | op 分发、会话打开、命令映射与 `{outcome,value?}` 回复 |
| `snapshot.go` | 服务快照到共享 UI 快照的字段翻译 |
| `library.go` | `library.*`、`clipboard.content/thumbnail`、`clipboard.share` |
| `input.go` | 键鼠设置读改写、输入命令映射 |
| `workflow.go` | 工作流目录命令 |
| `values.go` | 参数解析：兼容 `float64` / `json.Number` / 字符串 ID |

## 已接入

### 快照字段

| 服务字段 | 共享 UI 字段 | 说明 |
| --- | --- | --- |
| `local_device_id` / `local_name` / `last_error` | `local.id` / `local.name` / `local.error` | 本机身份与最近错误 |
| `peers[].trust` | `devices[].trusted` / `blocked` | `trusted`、`blocked` 两个布尔量 |
| `peers[].connection` | `devices[].connected` / `link_status` | `connected` 为真；其余给出未连接原因 |
| `peers[].selected_route` | `devices[].endpoint` | 当前线路标识 |
| `peers[].routes` | `devices[].routes` | 原样透传，由页面渲染 |
| `pending[]` | `pairing[]` | `request_id` 同时给出 `id`，供页面确认/拒绝 |
| `file_transfers[]` | `transfers[]` | `state` 映射为页面使用的 `phase`，未知状态原样保留 |
| `file_settings` / `file_cache` | `files` / `cache` | 接收位置、阈值与缓存占用 |
| `capabilities` | `features` / `capabilities` | 鼠标、麦克风、远程应用固定为 `false`（本端尚无实现） |
| `capabilities.workflow_execution` + `workflow_sync` | `app.workflows_enabled` / `capabilities.workflow_*` | 只有两项都为真时页面才显示工作流入口 |
| `input.settings` | `input.settings` | 核心模型转 macOS 桥接模型，见下表 |
| `input.managed` / `status` / `reason` | `input.managed` / `managed_paused` / `summary` / `error` | `PausedByUser` 映射为已暂停；`summary` 由真实状态派生 |
| `input.remote_topologies` / `links` | `input.displays[]` | 远端屏幕原样投影；`ready` 来自对应链路 |
| `input.remote_wallpapers` | `input.displays[].wallpaper_jpeg` | 认证后的对端预览，原样透传 |
| `input.environment` | `input.environment_message` / `ssid` / `observed_ssid` / `environment_conflict` / `peer_ssids` / `rules` | Wi-Fi 标识与场景规则 |
| `input.quality` | `input.quality` | 微秒换算为毫秒；服务窗口为 60 秒，`hitches` 即每分钟卡顿数 |

`input.settings` 的字段翻译：

| 服务（`deskfuse_core::InputSharingSettings`） | 共享 UI（macOS 桥接） |
| --- | --- |
| `feature_enabled` / `enabled` | 同名布尔量 |
| `role == "Controller"` | `controller: true`（其余为 `false`） |
| `participants` | `participants` |
| `switch_mode`（`Instant`/`Dwell250Ms`/`DoubleTap500Ms`） | `instant`/`dwell`/`double_tap` |
| `disabled_corners` / `corner_size` | 同名数值 |
| `placements[].display.{device_id,display_id}` + `x`/`y` | `placements[]` 扁平字段 |
| `shortcuts.{left,right,up,down,lock,emergency}` | 顺序固定的 6 项 `shortcuts` 数组 |
| 未知或未编辑字段（`layout_profiles`、`input_paths`、`authorized_controller`） | 不下发，但命令回写时原样保留 |

未投影的输入数据：服务快照只给出 `display_count`，没有本机屏幕拓扑，因此 `input.displays`
只包含远端屏幕，不伪造本机屏幕；指针锁定、录制状态、远程应用输入也没有协议字段，保持缺省。
`input.quality` 没有逐秒卡顿归因、慢注入计数和 `datagram_ready`，这些字段不下发。

其他未投影字段：`clipboard.history` 由 `library.query` 提供，`clipboard.automatic_ocr` /
`paste_target` / `navigation` 没有对应的 Windows 服务命令或原生粘贴板适配，保持缺省；
`workflow_runs` / `workflow_error` 也没有服务投影（`RunWorkflow` 是同步命令），页面显示为空。
`permissions` 固定为空数组，因为 Windows 服务不提供 macOS 式权限清单。

`engine_ready` 只有取得真实快照后才为 `true`，页面据此决定是否加载历史库。

### op

| op | 服务命令 | 说明 |
| --- | --- | --- |
| `hello` | `Snapshot` | 返回协议、包版本与服务能力 |
| `app.snapshot` | `Snapshot` / `Poll` | 首次打开代次，之后复用 epoch 与游标 |
| `app.invoke` | 见下节 | 固定 `{action,params}`，回复 `{outcome,value?}` |
| `library.query` | `LibraryQuery{request}` | 请求对象序列化为服务要求的 JSON 字符串 |
| `library.mutate` | `LibraryMutate{request}` | 同上；同时把页面字符串化的数字 ID 还原为 JSON number |
| `clipboard.content` | `LibraryContent{id}` | 见内容形状 |
| `clipboard.thumbnail` | `LibraryThumbnail{id,max_pixel_size}` | 返回 `{mime,data_base64}` |
| `cancel` | — | 返回 `unsupported_platform`；管道没有独立取消命令，超时关闭连接 |

`library.query` / `library.mutate` 会在首次调用前发送一次 `Snapshot` 注册客户端会话；
页面随后轮询的 `app.snapshot` 继续复用同一 epoch。

`clipboard.content` 的返回形状与 macOS 桥接一致：

```json
{"id":"42","text":"...","truncated":false,"file_paths":[],"rtf_available":true,
 "has_image":false,"can_request_files":false,"image_png_base64":"...","rtf_base64":"..."}
```

- `image_png_base64` / `rtf_base64` 只在请求里 `include_image` / `include_rtf` 为真时下发。
- `truncated` 为 `false`：服务对超过 16 MiB 的条目直接拒绝，不返回截断的成功结果。
- `can_request_files` 为 `false`：Go UI 没有延迟文件取回通道；服务在内容未本地化时直接返回拒绝。
- `rich_runs` / `html` 不下发：服务内容库只提供原始 MIME，适配层不做 RTF/HTML 解析。

### app.invoke 动作

| UI 动作 | 服务命令 | 说明 |
| --- | --- | --- |
| `devices.refresh` / `refresh` | `RefreshPeers` | |
| `devices.connect` / `disconnect` | `Connect` / `Disconnect` | |
| `devices.pair`（`ip`） | `RequestPairingAt` | |
| `devices.pair`（`peer_id`） | `RequestPairing` | |
| `devices.accept` / `reject` | `AcceptPairing` / `RejectPairing` | |
| `pairing.approve` / `reject`（`id`） | `AcceptPairing` / `RejectPairing` | 配对窗口的动作，参数名为 `id` |
| `devices.forget` | `Forget` | |
| `devices.block`（需 `confirm:true`） | `Revoke` | |
| `devices.unblock` | `AllowPairing` | |
| `feature.setEnabled`（`files`） | `SetFileTransferEnabled` | |
| `feature.setEnabled`（`input`） | `SetInputSharingSettings` | 只改 `feature_enabled`，其余设置原样保留 |
| `lanWeb.configure` | `ConfigureLanWeb` | |
| `clipboard.peers` | —（快照投影） | 从最近快照返回可信且未阻止的设备 `{id,name,connected}` |
| `clipboard.sources` | `LibraryQuery{op:"state"}` | 取 `devices[]` 的 `{id,name}` |
| `clipboard.share`（无工作流） | `ShareLibraryEntry{id,peer_id}` | 由服务执行已确认的剪贴板/文件通道 |
| `clipboard.share`（带 `workflow_id`） | `LibraryContent` → `RunWorkflow` → `ShareContent` | 只支持 MIME 内容；文件条目拒绝 |
| `clipboard.updateRichText`（纯文本） | `LibraryMutate{op:"set_item_text"}` | 带 `runs` 的样式编辑拒绝，避免静默丢格式 |
| `workflow.list` | `GetWorkflows` + `GetWorkflowCatalog` | 返回 `{definitions,catalog}` |
| `workflow.save` | `SaveWorkflow{workflow}` | 由服务校验核心工作流模型 |
| `workflow.delete`（需 `confirm:true`） | `DeleteWorkflow{id}` | |
| `workflow.restore_conflict` | `RestoreWorkflowConflict{conflict_id}` | |
| `workflow.cancel` | `CancelWorkflow{run_id}` | 转发命令；服务当前以 `not_supported` 拒绝 |
| `input.setSettings` / `set_input` | `SetInputSharingSettings{settings}` | 合并到最近快照的设置；保留未编辑字段 |
| `input.setParticipant` | `SetInputSharingSettings{settings}` | 增删 `participants`，保留角色与其余设置 |
| `input.reclaim` | `ReclaimInput` | |
| `input.moveDevice`（`deviceId/displayId/x/y`） | `MoveControlDevice` | 吸附由服务侧执行 |
| `input.reconnect` | `RestartInputControl{peer_id}` | 页面会先调用 `devices.connect` |

`Queued` 映射为 `{"outcome":"queued"}`：它只表示服务已接受命令，页面继续以快照为准，
不得把 `queued` 当成对端已完成。返回值的动作使用 `{"outcome":"applied","value":...}`，
页面 `backend.invoke` 只解包 `value`。

快照刷新不是每次都发 `Snapshot`。服务每次 `Snapshot` 都会递增会话代次并丢弃已缓冲的剪贴板内容，
所以适配层只在首次读取或 `Poll` 被拒绝（锁屏、切换用户、桌面代次过期）时重新 `Snapshot`，
其余刷新都用同一个 `session_epoch` 和上一轮的 `clipboard_sequence` 游标；重新打开只重试一次，
真实失败原样返回。

## 桌面宿主原生能力（已接入）

下列动作不属于服务命令，由桌面进程（Go 宿主）通过 `internal/native` 在本机完成；
服务仍是内容的唯一权威，宿主只负责把内容写进系统剪贴板与本机 UI。

| UI 动作 | 实现 | 说明 |
| --- | --- | --- |
| `clipboard.copy` / `clipboard.paste` | `LibraryContent` → 系统剪贴板 | 格式映射 `text/plain`→`CF_UNICODETEXT`、`text/html`→`HTML Format`、`text/rtf`→`Rich Text Format`、`image/png`→`PNG`+`CF_DIB`（JPEG 先转 PNG）、文件→`CF_HDROP`；写入后立即 `Flush`，退出前再 `Flush` 一次。`paste` 额外隐藏面板并把 Ctrl+V 注入上一个前台窗口（按进程跟踪前台，激活失败则如实报错“已复制，请手动粘贴”） |
| `clipboard.save` | 原生保存/选目录对话框 | 图片存 PNG、文本存 `.txt`、文件选目标目录后复制（重名自动加序号）；用户取消返回 `{saved:false}` |
| `clipboard.reveal` | 资源管理器 `/select` | 仅文件条目；无文件返回真实错误 |
| `clipboard.openInPreview` / `clipboard.edit` | 临时文件 + 系统默认应用 | 文件直接打开；图片/文本先写临时文件再打开 |
| `workflow.run` | `RunWorkflow` → 写剪贴板 | 先取 `LibraryContent` 作输入，服务执行后把返回格式写进系统剪贴板再报告成功；文件条目拒绝 |
| `app.quit` / `app.showMain` / `native.openPage` / `panel.hide` / `clipboard.showInMain` | 宿主窗口回调 | `HostActions` 由窗口所有者注入：退出置 `quitting` 后 `App.Quit`，显示主窗口并可 `__deskfuseNavigate` 跳页 |
| `system.open_url` | shell 打开 | 只接受 `http`/`https` |
| `support.report` / `support.reportProblem` | 预填 GitHub issue | 环境摘要 + 原始错误，URL 截断到 7,000 字符；只打开浏览器，不声称已提交 |
| `support.copy_environment` | 复制环境摘要 | 版本/本机/网络/设备，不含剪贴板正文与信任材料 |
| `login_item.status` / `login_item.set` | `HKCU\…\Run` 的 `DeskFuse` 值 | `enabled`/`not_registered`（Windows 无批准流程）；`open_settings` 打开 `ms-settings:startupapps` |
| `settings.set` | 本机偏好 | `keep_awake`（`SetThreadExecutionState`）、`workflows_enabled`（覆盖快照开关）持久化到 `%LOCALAPPDATA%\DeskFuse\mygo-ui-preferences.json`；macOS 专属键如实拒绝 |
| `shelf.add` / `shelf.remove` / `shelf.clear` / `shelf.show` / `clipboard.stage` | 宿主暂存区状态 | 路径引用列表注入快照 `shelf`；`clear` 清空并关闭窗口；`stage` 从内容库取本地文件入暂存区 |
| `shelf.send` | `PublishFileClipboard{paths,target_peer_id}` | 会话内递增 `sequence` 避免 stale；服务报价后拥有传输 |
| 暂存区拖出 | `Backend.ShelfDrag` → `StartDataDrag`（`CF_HDROP`） | 页面 pointer-down 超过阈值后经 `shelfDrag` 发起系统拖拽，可拖到资源管理器等（仅 Windows 宿主） |
| `updates.check` | GitHub Releases API | 语义化版本比较；`channel` 恒为 `manual`（Windows 无已发行自动安装器），界面显示新版本与发布页链接；结果经快照 `update` 投影 |
| `uninstall.plan` / `uninstall.perform` | 本机枚举 + NSIS 卸载器 | 组件：application/login_item/user_data/caches/preferences（无 mac 专属项）；perform 先删所选数据，选中 application 时静默启动 `Uninstall.exe /S` 并退出自身 |
| `support.export_diagnostics` | 保存 `.json` | 版本/网络/设备/偏好/更新状态，排除剪贴板正文与信任材料 |
| `clipboard.choose_app` / `clipboard.appInfo` | exe 选择器 + `Shell.FileIcon` | exe 路径作为应用标识（与服务捕获的来源进程一致）；图标为 PNG data URL，取不到返回 null |
| `clipboard.configure_shortcut` | mac kVK → MyGo accelerator | macOS 默认 ⌃⌘V 映射为 Ctrl+Shift+V，其余字面映射（⌘→Win）；未知键码/无修饰键如实拒绝；热键由宿主注册器管理 |
| `input.set_recording` | 宿主暂停面板热键 | 录制期间注销全局快捷键，避免录到的组合被抢先处理；服务侧输入快捷键不受影响 |
| `input.chooseController` | `EnableInputControl{peer_id}` | Windows 的“接管主控”即对指定设备启用控制 |
| `input.wallpaper` | 注册表壁纸路径 → JPEG data URL | 尽力而为：当前报告共享桌面壁纸（逐显示器需 `IDesktopWallpaper` COM，未接入），无壁纸返回 null |
| 通知 | 快照差量监视 | 新配对请求、传输完成/失败时发系统通知 |
| `input.previewDisplayMove` | `PreviewControlDeviceMove` → `LayoutOffset` | 与引擎同一 `snap_device_move` 吸附算法；冻结屏幕集+位移由页面提供，返回吸附后偏移，不提交 |
| `set_files` | `SetFileSettings{settings}` | 页面形状（enabled/prefetch/threshold/maximum/download_directory）映射到核心设置并校验持久化 |
| `clear_cache` | `ClearFileCache` → `CacheCleared` | 删除缓存根下已完成传输数据；在途接收目录按传输 ID 排除 |
| `transfers.cancel` / `transfers.retry` | `CancelTransfer` / `RetryTransfer` | 取消双向生效；重试用同一源 roots 换新传输 ID 重新报价（outgoing offer 完成前保留源） |
| `transfers.send` | `PublishFileClipboard{paths,target_peer_id}` | 页面显式路径发送，与暂存区发送同通道 |
| `transfers.chooseFiles` / `transfers.inspectPaths` | 宿主文件选择器 + `os.Stat` | 返回 `{path,name,size,isDirectory}`；拖入路径经 `inspectPaths` 补元数据 |
| `workflow.system_shortcuts` | `GetSystemShortcuts` → `SystemShortcuts` | Windows 无 App Intents 式目录源，服务返回空列表，步骤编辑器显示“无可用快捷指令” |

桌面集成：通知区域托盘图标（打开主窗口 / 剪贴板面板 / 退出）、全局快捷键 Ctrl+Shift+V
在光标处开关剪贴板面板（与 macOS 默认一致）、单实例锁（第二次启动把命令行交给已有实例）、
主窗口与面板按主屏工作区缩放收敛尺寸（高 DPI 下不超出屏幕）。

## 未接入（返回真实错误）

| 范围 | 错误码 | 原因 |
| --- | --- | --- |
| `clipboard.indexOCR` / `automatic_ocr` | `unsupported_platform` | Windows 无 OCR 索引能力（用户明确豁免）；界面已隐藏该开关 |
| `clipboard.updateRichText`（带 `runs`） | `unsupported_platform` | 需要把样式运行转换为 RTF；Go 适配层没有原生文本系统 |
| `workflow.system_shortcuts` | `unsupported_platform` | 服务没有系统快捷指令目录命令 |
| `input.set_recording`（鼠标物理按键录制） | `unsupported_platform` | 服务没有鼠标物理按键录制命令；键盘快捷键录制（暂停全局热键）已接入 |
| `download_and_install_update` | `unsupported_platform` | Windows 尚未发行自动安装器；`updates.check` 走 manual 通道给发布页链接 |
| `devices.leaveNetwork`、`devices.triggerUpdate(s)`、`devices.setInputPath`、`devices.icon` | `unsupported_platform` | 服务没有对应命令；设备产品图无 Windows 图像来源 |
| `settings.set`（`hide_dock_icon`）、权限框架、诊断引擎导出（内存/鼠标探针）、`clipboard.showPanel`/`togglePanel` | `unsupported_platform` | macOS 专属或需引擎诊断通道；界面已隐藏对应入口 |

工作流目录的 `catalog.conflicts` 沿用服务 `view_json` 的 `{id,record}` 形状；页面期望的
`name`/`message` 尚未对齐，因此冲突行可能只显示恢复按钮。这是字段投影缺口，不是伪造的成功状态。

这些条目不得用空实现或模拟数据补齐；需要先在 Windows 侧实现原生适配并通过实机验证。

## 验证

本次实际执行并通过：

```sh
cd apps/deskfuse-go
gofmt -l .                       # 无输出
go vet ./...
go test ./...                    # 全部通过（含剪贴板动作与管道忙重试）
GOOS=windows GOARCH=amd64 go build ./...
GOOS=windows GOARCH=arm64 go build ./...
./scripts/build-mygo-ui.sh       # WebView 客户端 target/mygo-webview/deskfuse-mygo
```

单元测试使用内存管道复现服务端行为：帧边界、请求编号回显、`0x06` 确认、拒绝映射、
会话自动打开、`Snapshot`/`Poll` 代次复用、内容库请求编号还原、内容与缩略图投影、
工作流目录命令、键鼠设置合并与保留字段、共享命令序列，以及不支持动作不打开连接。
新增：剪贴板复制/粘贴动作（格式解码、粘贴前先隐藏面板）、`ERROR_PIPE_BUSY` 重试、
暂存区增删/清空/投影与 `PublishFileClipboard` 序列、版本比较、快捷键映射（mac kVK →
accelerator）、`EnableInputControl` 映射、取消选择应用与未知键码拒绝。

## 实机验证（2026-10-31）

在 Windows 11（250% 缩放，2048×1152）实机通过：

- 客户端以服务管道 `com.deskfuse.app.desktop.v2` 连接真实 `deskfuse-service.exe`，
  历史库加载 5,500+ 条、设备在线状态与快照轮询正常；系统剪贴板变更被服务实时捕获进历史。
- 托盘图标与 Ctrl+Shift+V 全局快捷键已注册；快捷键在光标处开关剪贴板面板
  （视口 440×426 DIP，dpr 2.5，布局正确）。
- 高 DPI 下主窗口与面板按工作区收敛，不再超出屏幕；窗口居中。
- 服务管道单实例导致并发请求 `ERROR_PIPE_BUSY` 的问题已通过客户端串行化 + 忙重试解决。
- 桌面进程经 `mygo.Clipboard.Write`+`Flush` 写入 `CF_UNICODETEXT`/`HTML Format` 能到达
  Windows 系统剪贴板（探针程序写入后用 `Get-Clipboard` 读回一致），`native.WriteClipboard`
  的格式映射即建立在这条通路上。
- 扩展协议命令已对真实服务逐一验证：`PreviewControlDeviceMove` 返回吸附后偏移
  （小位移在吸附距离内对齐到共享边）、`GetSystemShortcuts` 返回空目录、
  `ClearFileCache` 返回清理字节数、`SetFileSettings`/`CancelTransfer` 入队。
- 修复服务既有 bug：`open_transfer_cache` 把自相对 SECURITY_DESCRIPTOR 指针当作 PACL
  传给 `SetNamedSecurityInfoW`，导致 transfer-cache 被装上**空的受保护 DACL**（连服务
  自己都被拒绝访问，文件接收落地必然失败）。现改为 `GetSecurityDescriptorDacl` 取出真正
  的 DACL 再设置；已用脚本修复线上目录 ACL，服务每次启动也会重放正确 DACL。

**仍未验收**（需要双设备或特定条件，不得声称已通过）：把历史条目写回系统剪贴板后在
第三方应用 Ctrl+V 的真实效果、向对端设备共享/传输、双机键鼠控制、锁屏/会话切换、
`app.quit` 后托盘与快捷键的清理。这些需要实机双设备验收。
