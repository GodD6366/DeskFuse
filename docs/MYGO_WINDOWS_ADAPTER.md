# MyGo UI · Windows 适配层

状态：传输与快照、设备/配对/文件/局域网共享、内容库读取与内容/缩略图、键鼠设置命令、工作流目录命令已接入；
本地剪贴板写入、键鼠拖动预览、更新/卸载和工作流输出应用仍未接入。**未在 Windows 实机验证。**
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

## 未接入（返回真实错误）

| 范围 | 错误码 | 原因 |
| --- | --- | --- |
| `clipboard.copy` / `paste` | `unsupported_platform` | 服务没有本地粘贴板写入/粘贴命令；原生粘贴板仍由 Windows 桌面客户端持有 |
| `clipboard.save` / `edit` / `reveal` / `openInPreview` / `stage` / `cancelPreparation` | `unsupported_platform` | 需要原生保存面板、编辑窗口或文件暂存区，服务 IPC 未提供 |
| `clipboard.choose_app` / `indexOCR` / `appInfo` / `configure_shortcut` | `unsupported_platform` | 没有原生应用选择、OCR 索引或全局快捷键命令 |
| `clipboard.updateRichText`（带 `runs`） | `unsupported_platform` | 需要把样式运行转换为 RTF；Go 适配层没有原生文本系统 |
| `workflow.run` | `capability_missing` | 服务有 `RunWorkflow`，但其约定要求桌面端把返回格式写入原生剪贴板后才报告成功；Go UI 尚未接入 Windows 粘贴板写入，因此不执行并明确报错 |
| `workflow.system_shortcuts` | `unsupported_platform` | 服务没有系统快捷指令目录命令 |
| `input.chooseController` | `unsupported_platform` | Windows 由控制端 `EnableInputControl{peer_id}` 建立方向，没有“按设备选择主控”的命令 |
| `input.previewDisplayMove` | `unsupported_platform` | 没有拖动预览/吸附计算命令；吸附只在 `MoveControlDevice` 时由服务执行 |
| `input.wallpaper` | `unsupported_platform` | 没有本机屏幕预览命令 |
| `input.set_recording` | `unsupported_platform` | 没有鼠标按键录制命令 |
| `updates.check` / `download_and_install_update` | `unsupported_platform` | 桌面 IPC 没有更新命令，`capabilities.updater` 始终为 `false` |
| `uninstall.plan` / `uninstall.perform` | `unsupported_platform` | 桌面 IPC 没有卸载命令 |
| `devices.leaveNetwork`、`devices.triggerUpdate(s)`、`devices.setInputPath` | `unsupported_platform` | 服务没有对应命令 |
| `devices.icon` | `unsupported_platform` | 设备产品图由 macOS 宿主用 CoreTypes 资源栅格化；Windows 没有对应图像来源（Apple 系统图像不随包分发），设备列表继续使用页面绘制的回退图标 |
| `set_files`、`clear_cache`、`transfers.cancel/retry/send` | `unsupported_platform` | 服务只有文件传输总开关、局域网共享和传输队列，没有阈值/目录设置、清缓存或桌面侧传输控制命令 |
| `settings.set`、`panel.hide`、`app.showMain`、`app.quit`、`clipboard.showInMain/showPanel/togglePanel`、`shelf.*`、权限/登录项/诊断/通知 | `unsupported_platform` | 属于 UI 宿主或 Windows 端尚未实现的平台能力，不是服务命令 |

工作流目录的 `catalog.conflicts` 沿用服务 `view_json` 的 `{id,record}` 形状；页面期望的
`name`/`message` 尚未对齐，因此冲突行可能只显示恢复按钮。这是字段投影缺口，不是伪造的成功状态。

这些条目不得用空实现或模拟数据补齐；需要先在 Windows 侧实现原生适配并通过实机验证。

## 验证

本次实际执行并通过：

```sh
cd apps/deskfuse-go
gofmt -l .                       # 无输出
go vet ./...
go test ./...                    # 80 passed / 11 packages
GOOS=windows GOARCH=amd64 go build ./...
GOOS=windows GOARCH=arm64 go build ./...
```

单元测试使用内存管道复现服务端行为：帧边界、请求编号回显、`0x06` 确认、拒绝映射、
会话自动打开、`Snapshot`/`Poll` 代次复用、内容库请求编号还原、内容与缩略图投影、
工作流目录命令、键鼠设置合并与保留字段、共享命令序列，以及不支持动作不打开连接。

**未在 Windows 实机验证。** 以下范围仍只是协议接线与单元测试结论：管道鉴权、UAC/锁屏/会话切换、
真实 `deskfuse-service.exe`、真实剪贴板内容与缩略图、Windows 输入辅助进程、双机配对/共享/传输、
页面在 Windows WebView 中的渲染与交互。不得把上述命令通过表述为实机验收通过。
