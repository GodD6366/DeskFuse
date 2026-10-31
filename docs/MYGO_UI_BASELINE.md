# MyGo UI 迁移基线：macOS 界面盘点

状态：第一步静态盘点完成。本文只记录当前 macOS 客户端的页面、窗口、菜单、快捷键、空状态与错误交互，作为 MyGo UI 替换的输入，不代表 MyGo 已接入或已完成实机验收。

- 盘点日期：2026-10-27
- 代码基线：`446c92a6`（`feat(windows): 对齐 macOS 剪贴板、键鼠共享与分享传输`）
- 主应用：`apps/deskfuse-macos/Sources/DeskFuse`（138 个 Swift 文件，30,971 行）
- 原生远程应用库：`apps/deskfuse-macos/RemoteApplication/Sources`（18 个 Swift 文件，4,026 行，另有 `RdpBridge.c/.h`、`DFRemoteVirtualDisplay.m/.h` 等 C/Objective-C 桥接）
- 构建入口：`apps/deskfuse-macos/Package.swift`、`apps/deskfuse-macos/RemoteApplication/Package.swift`

优先级定义：

- **P0**：MyGo 外壳可用所必需，缺失会阻断日常主路径或无法验证其他页面。
- **P1**：核心协作能力，在 P0 外壳稳定后按功能纵向接入。
- **P2**：原生依赖重、Beta/低频或可由原生组件长期托管的页面，最后迁移。

## 1. 入口、窗口与导航

| 界面/窗口 | 入口 | 源文件 | 形态与关键行为 |
| --- | --- | --- | --- |
| 主窗口 | 启动、Dock、菜单栏图标、`applicationShouldHandleReopen` | `DeskFuseApp.swift:10-21`、`:156-160`、`:192-260` | SwiftUI `Window("", id: "main")`，默认 1200×780，最小 960×568；`hiddenTitleBar` + `WindowChromeMarker` 手动定位红绿灯，窗口标识 `deskfuse-main` |
| 侧栏与工作区 | 主窗口左侧导航 | `RootView.swift:11-35`、`MainSidebar.swift:24-75`、`MainContentView.swift:42-87`、`AppStore.swift:449-479` | 页面枚举：设备、剪贴板、键鼠共享、局域网传输、远程应用、麦克风共享、鼠标自定义、设置；远程应用/麦克风/鼠标标记 Beta |
| 设置 | 侧栏“设置”、⌘,、健康中心/关于入口 | `AppStore.swift:391-401`、`SettingsView.swift:31-88`、`SettingsCategoryBar.swift:4-46`、`SettingsPageContainer.swift:8-53` | 设置与工作区共用主窗口；分类：通用、剪贴板、工作流、快捷键、文件、权限、关于与更新、高级；`NSTabViewController` 保留各页视图状态 |
| 初次设置 | 首次启动、设置 › 高级 › 重走初次设置 | `DeskFuseApp.swift:127-133`、`RootView.swift:11-13`、`OnboardingFlow.swift:187-293` | 主窗口内替换工作区，步骤为系统权限、功能开关；不是独立窗口 |
| 菜单栏剪贴板面板 | 状态栏图标左键、全局快捷键（默认 ⌥⌘V） | `ClipboardPanelController.swift:36-88`、`:209-308` | `NSStatusItem` + `NSPopover` + 独立非激活 `ClipboardPointerPanel`；面板高度 780，跟随屏幕定位 |
| 菜单栏快捷预览 | 面板内空格 | `ClipboardQuickLookController.swift:56-100` | `NSPopover`，定位到选中行，箭头键切记录，Esc 关闭 |
| 文件暂存区 | ⇧⌘D；拖动文件时快速左右摇动鼠标 | `ShelfController.swift:52-75`、`:108-124`、`DeskFuseApp.swift:45` | 非激活 `NSPanel`，240×240，展开 480×420；支持拖出为文件 |
| 传输进度浮窗 | 有进行中的传输时自动出现 | `TransferProgressOverlay.swift:6-70` | 右上角非激活 `NSPanel`，每个传输一个胶囊，取消的 3 秒后自动消失 |
| 配对确认 | 收到新的入站配对请求 | `PairingWindow.swift:5-32` | 480×320 浮动 `NSPanel`，关闭未决请求会视为拒绝 |
| 剪贴板内容编辑窗口 | 面板 ⌘E、历史行编辑动作 | `ClipboardContentEditor.swift:8-45` | 每条记录一个独立 `NSWindow`（560×400，最小 480×320），编辑期间菜单栏面板保持打开 |
| 标题编辑 | 双击记录标题 | `ClipboardTitleEditor.swift`、`ClipboardHistoryPane.swift` | SwiftUI sheet；Esc 取消、Return 保存 |
| 远程应用选择器 | 侧栏远程应用、设备页打开远程应用 | `RemoteApplicationManager.swift:359-397` | 844×577 `NSWindow`，隐藏标题、统一 `NSToolbar`，最小高度 420 |
| 远程应用画布 | 从选择器打开应用或虚拟屏幕 | `RemoteApplicationManager.swift:400-459`、`RemoteApplicationView.swift:59-153` | `RemoteCanvasWindow`，承载 Metal 视频视图；本地快捷键由窗口拦截 |
| Windows 远程应用（RDP） | 远程应用页选择 RDP 设备/应用 | `RemoteRdpViews.swift:425-450` 及后续 RDP window | 选择器 720×500、会话窗口 1024×720；FreeRDP 帧缓冲绘制到自定义 `NSView` |
| 安装器 | 应用位于非 `/Applications` 且需要安装时 | `DeskFuseLauncher.swift:7-19`、`InstallationWindow.swift:229-273` | 独立 `NSApplication` 会话，不初始化 AppStore/引擎；安装完成后打开已安装应用 |
| 其他 sheet | 卸载、旧版本历史导入、麦克风驱动、剪贴板列表编辑、RDP 账号、输入设备选择、鼠标数据贡献 | `UninstallSheet.swift`、`PreviousInstallationPrompt.swift`、`MicrophoneSettingsView.swift:109-114`、`ClipboardLibraryViews.swift:165-225`、`RemoteRDPAccountViews.swift`、`InputDevicePicker.swift`、`MouseDataContribution.swift` | 均为主窗口/页面上的 SwiftUI sheet 或浮层 |
| Dock 菜单 | 右键 Dock 图标 | `DeskFuseApp.swift:150-155`、`RemoteApplicationDock.swift:103-128` | 列出已打开的远程应用，切换到对应窗口；Dock 图标会临时显示当前远程应用图标与“远程”标记 |

补充：

- 应用关闭最后一个窗口后不退出（`DeskFuseApp.swift:136`）；可从菜单栏图标或 Dock 重新打开。
- 当前 Swift 源码未使用 `MenuBarExtra`，也未发现 `UserNotifications`/`UNUserNotificationCenter` 本地通知实现；状态主要通过侧栏健康中心、浮动提示、状态栏图标和 Dock 菜单表达。
- `FileTransferView.swift:78` 文案称可把文件拖到菜单栏图标，但 `ClipboardPanelController`/`MenuBarClipboardView` 未见拖放目标注册；实际可见的呼出路径是 ⇧⌘D 和 `ShelfController` 的全局拖拽摇动手势。迁移前需运行核实该文案。

## 2. 菜单

| 菜单 | 位置 | 内容/行为 |
| --- | --- | --- |
| 应用菜单 | `DeskFuseApp.swift:22-63` | 移除默认“新建”；`⌘,` 打开设置；保留系统标准编辑/窗口菜单 |
| “共享”菜单 | `DeskFuseApp.swift:44-50` | 文件暂存区（⇧⌘D）、立即收回键鼠（⌃⌥⌘Esc，按输入状态禁用）、刷新设备 |
| “帮助”菜单替换 | `DeskFuseApp.swift:52-62` | 反馈问题…（⌘?）、导出诊断信息…、复制版本与环境信息、已报告的问题、发行说明、安全问题反馈；避免系统“帮助不可用”弹窗 |
| 搜索命令 | `RootView.swift:85-97` | 搜索剪贴板（⌘F）、搜索设备（⌘F）；由 focused scene value 决定当前页是否可用 |
| 侧栏求助菜单 | `MainSidebar.swift:169-192` | 反馈问题、复制版本与环境信息、已报告的问题、发行说明 |
| 状态栏图标 | `ClipboardPanelController.swift:36-88` | 单击打开剪贴板面板；不是传统下拉菜单 |
| 剪贴板记录上下文菜单 | `ClipboardHistoryContextMenu.swift:75-217` | 复制、粘贴、编辑标题/内容、保存、在预览中打开、在访达中显示、删除、工作流、发送到设备等；Ctrl+单击或右键触发原生 `NSMenu` |
| 剪贴板列表/预览菜单 | `ClipboardHistoryPane.swift:104-130`、`ClipboardPreviewPane.swift:211-305` | 全选已加载、删除所选、来源筛选、更多操作、保留原始格式、在访达中显示等 |
| 剪贴板列表管理 | `ClipboardLibraryViews.swift:37-76`、`:138-160` | 列表重命名、前后移动、删除；新建列表 |
| 设备列表 | `DevicesView.swift:229-239`、`:117-180`、`:587-611` | 双击已连接已信任设备打开远程应用；手动 IPv4 配对、刷新、批量触发更新；设备“…”菜单（打开远程应用、刷新连接） |
| 远程应用菜单 | `RemoteApplicationView.swift:213-300` | 打开其他应用、释放控制、性能信息、详细诊断、顶部导航模式、分辨率/编码/刷新率/显示比例、最小化/最大化/重连/关闭 |
| RDP 账号菜单 | `RemoteRdpViews.swift:219-243` | 选择已保存账号、使用其他账号、管理账号 |
| Dock 菜单 | `RemoteApplicationDock.swift:103-128` | 打开远程应用列表与当前窗口切换 |
| 其他上下文菜单 | `DisplayLayout.swift:181`、`ShelfController.swift:255`、`MouseSettingsView.swift:314`、`WorkflowSettingsView.swift:55` | 显示器操作、暂存区发送目标、鼠标按键动作、工作流操作 |

## 3. 快捷键

| 范围 | 快捷键 | 行为 | 源文件 |
| --- | --- | --- | --- |
| 全局剪贴板 | ⌥⌘V（默认，`key_code 9`，`modifiers 0x180000`） | 打开/关闭菜单栏剪贴板面板；可在设置 › 快捷键修改或移除，冲突时拒绝保存 | `ClipboardPanelController.swift:189-205`、`ClipboardShortcutSettings.swift:9-29`、`ShortcutSettingsPage.swift:80-153` |
| 固定保底 | ⌃⌥⌘Esc | 立即收回键鼠，不受自定义快捷键影响 | `DeskFuseApp.swift:46`、`ShortcutSettingsPage.swift:50` |
| 固定暂存区 | ⇧⌘D | 打开/收起文件暂存区 | `DeskFuseApp.swift:45`、`ShortcutSettingsPage.swift:51` |
| 应用命令 | ⌘,；⌘?；⌘F | 设置；反馈问题；搜索当前剪贴板/设备 | `DeskFuseApp.swift:27`、`:54`、`RootView.swift:92-95` |
| 菜单栏面板 | ⌘F、⌘C、⌘S、⌘E、⌘O、⌘1–9、⇧⌘R、空格、Esc、Return、Delete、方向键 | 搜索、复制、保存、编辑、预览、按序号粘贴、在访达中显示、快速预览、关闭/取消预览、粘贴、删除、选择 | `MenuBarClipboardView.swift:746-815` |
| 远程应用窗口 | ⌃⌥Esc；⌃⌥⌘空格；⌃⌥⌘W；⌃⌥⌘M；⌃⌥⌘F | 释放控制；打开远程应用菜单；关闭；最小化；最大化/还原 | `RemoteApplicationManager.swift:410-423`、`RemoteApplicationView.swift:253-286` |
| RDP 画布 | ⌃⌥Esc | 释放输入并交还焦点 | `RemoteRdpViews.swift:589-592` |
| 键鼠共享 | 由引擎设置驱动，设置页可编辑并立即提交 | 方向切换/锁定等全局快捷键；Swift 侧只负责展示和提交，注册在 Rust 引擎 | `ShortcutSettingsPage.swift:23-44`、`ShortcutEditor.swift` |
| 工作流 | 每个工作流一个全局快捷键，需含 ⌘/⌥/⌃ | 录制器只接受带修饰键组合；保存后由引擎注册并执行 | `WorkflowShortcutRecorder.swift:52-97`、`WorkflowStore.swift:115` |
| 鼠标按键 | 通过 CGEventTap 拦截并映射 | 可与系统快捷键、复制/粘贴、切换桌面等动作绑定；组合修饰键时保留原始按钮 | `MouseButtonController.swift:358-380`、`MouseSettingsView.swift:314-337` |

## 4. 空状态清单

| 功能域 | 条件 | 文案/表现 | 源文件:行 |
| --- | --- | --- | --- |
| 启动 | 引擎快照未就绪且启动失败 | “无法启动”，提供重试与反馈问题 | `MainContentView.swift:81-87` |
| 启动 | 自检流程 | “正在确认更新启动” | `MainContentView.swift:45-48` |
| 功能未开启 | 功能总开关关闭 | `FeatureGateView`：图标/演示、说明、权限行、启用按钮 | `FeatureOnboardingView.swift:4-65`、`FeatureGateView.swift:4-120` |
| 剪贴板历史 | 无历史或筛选无结果 | “还没有剪贴板记录”/“没有匹配内容”，可清除筛选 | `ClipboardHistoryPane.swift:436-440`、`ClipboardEmptyState.swift:4-22` |
| 剪贴板预览 | 未选择记录 | “复制内容，从这里接着用”/“选择一条历史” | `ClipboardView.swift:389-394` |
| 剪贴板预览 | 读取失败 | “无法读取预览”，提供重试 | `ClipboardView.swift:378-379` |
| 剪贴板文件 | 文件尚未在本地/不可用 | “文件尚未在本地”/“文件不可用”，提示保持来源设备连接 | `ClipboardPreviewPane.swift:114`、`:401` |
| 菜单栏面板 | 无历史、搜索失败或无匹配 | “引擎尚未就绪”/“搜索失败”/“还没有剪贴板记录”/“没有匹配内容” | `MenuBarClipboardView.swift:365-370` |
| 设备 | 无已配对/附近/阻止设备，或搜索无匹配 | “尚未发现其他设备”/“没有匹配的设备” | `DevicesView.swift:217-224` |
| 设备路由 | 未发现候选地址 | “暂未发现连接地址…” | `DeviceRoutesPanel.swift:24` |
| 设备页传输活动 | 无最近传输 | “暂无传输任务” | `DevicesView.swift:618` |
| 文件传输 | 无已连接电脑 | “还没有已连接的电脑” | `FileTransferView.swift:179` |
| 文件传输 | 无传输任务 | “还没有传输任务” | `FileTransferView.swift:251` |
| 文件传输扫码 | 共享未开启/二维码不可用 | “二维码不可用”及说明 | `FileTransferView.swift:298`、`:318` |
| 暂存区 | 无在线目标 | “没有可发送的在线设备” | `ShelfController.swift:257` |
| 显示器布局 | 无显示器信息 | “等待显示器布局” | `DisplayLayout.swift:53-54` |
| 添加设备屏幕 | 无可添加设备 | “还没有可添加的设备” | `InputDevicePicker.swift:31-32` |
| Wi-Fi 身份 | 无对端标识 | “暂无；对端需与本机建立输入链路后才会上报” | `InputWifiIdentityView.swift:193` |
| 工作流 | 无已保存流程/无工作流/无适配工作流/无设备 | “尚未保存流程”/“还没有工作流”/“没有适合此内容的已启用工作流”/“请先配对一台设备” | `WorkflowSettingsView.swift:48`、`WorkflowShortcutRecorder.swift:130`、`:189`、`:226` |
| 麦克风 | 无已配对电脑/无音频 | “还没有已配对的电脑”；“当前没有采集或传输音频…” | `MicrophoneSettingsView.swift:583`、`:515-525` |
| 远程应用 | 目录未就绪/未配对/无应用/无收藏/无账号 | “远程应用尚未就绪”/“请先配对设备”/“没有可用的远程应用”/“还没有收藏的应用”/“还没有保存的账号” | `RemoteApplicationCatalog.swift:45`、`DevicesView.swift:457-478`、`RemoteRdpViews.swift:145-163`、`RemoteRDPAccountViews.swift:43-44` |
| 鼠标 | 未检测到扩展键/未记录按键/未找到设备 | “尚未检测到此扩展键”/“尚未记录按键”/“没有找到你的鼠标？” | `MXMasterMouseEditor.swift:116`、`MouseDataContribution.swift:355`、`MouseSettingsView.swift:262` |

## 5. 错误交互清单

| 层级/类型 | 位置 | 行为 |
| --- | --- | --- |
| 窗口级浮动错误 | `ErrorNotice.swift:6-44`、`ErrorNoticeView.swift:6-58`、`RootView.swift:26-31` | 右侧浮动卡片“操作未完成”；可关闭、复制原始错误、带原文反馈；相同运行时错误被关闭后不因重复快照复活 |
| 页面级功能错误 | `MainContentView.swift:21-37` | 远程应用/麦克风页顶部 `DFNotice` 警告，可重试或关闭 |
| 全局健康状态 | `HealthCenter.swift:21-100`、`MainSidebar.swift:128-166` | 侧栏底部汇总“一切正常/N 项需要留意”；点开问题清单，每条带可直接执行的下一步（添加电脑、授权、刷新、查看） |
| 支持与诊断 | `SupportCenter.swift:39-76`、`:115-136`、`AboutSettingsView.swift:53-70` | 反馈问题打开 GitHub Issue（带环境、错误、诊断），导出诊断 `NSSavePanel`，复制环境，打开日志目录；Issue URL 有长度上限 |
| 剪贴板 | `ClipboardView.swift:96-101`、`:378-379`、`ClipboardItemActions.swift:256-267` | 页面内警告标签、预览重试、复制/打开/保存失败转全局错误 |
| 设备与配对 | `DevicesView.swift:75-100`、`PairingWindow.swift:66-78` | 忘记设备/拉黑/退出网络/批量更新的确认对话框；配对面板内错误条 |
| 文件传输 | `FileTransferView.swift:29`、`:458`、`LanWebSharing.swift:56-114` | 共享/读取错误条，失败任务可重试，取消状态 |
| 键鼠共享 | `InputView.swift`、`DisplayLayout.swift`、`HealthCenter.swift:41-54` | 权限缺失、输入错误、布局更新未确认；健康中心可跳转授权/页面 |
| 鼠标 | `MouseSettingsView.swift:120-280` | 输入监控权限提示、配置同步说明、恢复默认确认、按键录制失败提示 |
| 工作流 | `WorkflowSettingsView.swift:119-189`、`WorkflowStepEditor.swift:109`、`WorkflowSyncView.swift:27-34`、`WorkflowRunStatusView.swift:39` | 删除/舍弃确认、未保存提示、步骤/同步/运行错误文本 |
| 麦克风 | `MicrophoneSettingsView.swift:40-90` | “无法连接麦克风服务”“操作未完成”通知卡，含重试、友好文案和原始细节 |
| 远程应用/RDP | `RemoteApplicationCatalog.swift:45`、`RemoteRdpViews.swift:371`、`RemoteDiagnosticsOverlay.swift` | 目录/连接/凭据错误，诊断面板展示帧率、P95、丢包、解码队列等 |
| 权限与重启 | `Permissions.swift:11-48`、`:248-291`、`OnboardingFlow.swift:483`、`:537` | 权限缺失/未授权提示；需要重新打开时显示 `DFNotice`；模态期间允许系统设置“退出并重新打开” |
| 安装/更新 | `InstallationWindow.swift:7-58`、`UpdateControls.swift:44-107` | 安装失败原因与重试；检查/下载/安装状态、进度、手动安装说明 |
| 旧版本导入 | `PreviousInstallationPrompt.swift:150-196` | 导入失败、旧应用未退出、移入废纸篓确认、已导入/无历史数据库状态 |
| 卸载 | `UninstallSheet.swift:137-160` | 输入“卸载”确认；失败信息；管理员授权取消说明 |
| 通用错误文案 | `HealthCenter.swift:103-120` | 把超时、连接拒绝、权限、磁盘、数据库、运行时等内部错误映射为用户可读说明；中文原文保留 |

## 6. 按功能域的迁移盘点与验证风险

### D0 应用外壳、导航、设计系统与设置框架（P0）

- 入口：启动主窗口、侧栏导航、设置分类、应用菜单、窗口生命周期。
- 主要文件：`DeskFuseApp.swift`、`RootView.swift`、`MainContentView.swift`、`MainSidebar.swift`、`AppStore.swift`（`Page`）、`SettingsPageContainer.swift`、`HostingEnvironment.swift`、`WindowVisibilityObserver.swift`、`DesignSystem/DFTokens.swift`、`DFComponents.swift`、`DFButtons.swift`、`NativeGlass.swift`、`HorizontalWheelScrollView.swift`、`NativeListSelection.swift`。
- AppKit/native 依赖：`NSWindow` 标题栏/红绿灯/拖拽区、`NSViewControllerRepresentable` + `NSTabViewController`、`NSSplitViewController`、`NSVisualEffectView`、`NSFont`/`NSColor`、AppKit 无障碍与键盘遍历。
- 空状态/错误：启动失败、加载遮罩、自检；全局错误浮层与健康中心见第 4/5 节。
- MyGo 验证风险：标题栏几何与全屏/Space 重排、`NSTabViewController` 的页面状态保留、焦点与键盘遍历、最小尺寸、窗口重建时标题栏材质、设计 token 的字体/间距/材质一致性、减少动态效果与高对比度。

### D1 设备列表、配对与路由（P1）

- 入口：侧栏“设备”（未配对时置顶）；手动 IPv4 配对浮层；入站配对自动弹窗；设备“…”菜单。
- 主要文件：`DevicesView.swift`、`DeviceRoutesPanel.swift`、`PairingWindow.swift`、`DeviceProductIcon.swift`、`HealthCenter.swift`（设备/权限问题）、`Permissions.swift`（Wi-Fi 名称）。
- AppKit/native 依赖：`NSPanel` + `NSHostingView`、SwiftUI `List` 选择/双击主操作、`NSPopover`、`NSWorkspace`、`PermissionFlow`、`CoreWLAN`/`CoreLocation`（Wi-Fi SSID）。
- 空状态/错误：设备空列表、路由空列表、配对错误条、忘记/拉黑/退出网络确认。
- MyGo 验证风险：列表单选/双击与上下文菜单冲突、配对面板焦点与关闭即拒绝语义、手动 IPv4 校验、Wi-Fi 定位授权、路由探测结果刷新、阻止/信任状态与批量更新。

### D2 剪贴板历史工作区（P0）

- 入口：侧栏“剪贴板”；⌘F 聚焦搜索；列表选择/预览；右键菜单；⌘E 编辑独立窗口。
- 主要文件：`ClipboardView.swift`、`ClipboardHistoryPane.swift`、`ClipboardMasonryGrid.swift`、`ClipboardPreviewPane.swift`、`ClipboardWorkspaceSidebar.swift`、`ClipboardMedia.swift`、`ClipboardCodePreview.swift`、`ClipboardRecordLabel.swift`、`ClipboardItemActions.swift`、`ClipboardHistoryContextMenu.swift`、`HistorySplitView.swift`、`NativeFileDrag.swift`、`ClipboardContentEditor.swift`、`ClipboardTitleEditor.swift`。
- AppKit/native 依赖：`NSSplitViewController`（原生分栏）、`NSImage`、`NSMenu`、`NSPasteboard`、`NSWorkspace`、`QuickLookThumbnailing`、`UniformTypeIdentifiers`、`NSAttributedString` RTF、`NSDraggingSource`。
- 空状态/错误：历史为空/无匹配、未选择、预览失败、文件不在本地；“删除选中”确认对话框。
- MyGo 验证风险：文字/RTF/HTML/图片/文件多格式还原、延迟文件与缓存、缩略图与 Quick Look、分页与总数、多选/框选/Shift 连选、中文输入法、搜索焦点、复制回原应用与剪贴板 changeCount 校验。

### D3 菜单栏剪贴板面板与快捷预览（P0）

- 入口：状态栏图标左键；全局快捷键（默认 ⌥⌘V）；面板空格预览；面板 ⌘1–9/Return/双击粘贴。
- 主要文件：`ClipboardPanelController.swift`、`MenuBarClipboardView.swift`、`MenuBarClipboardRow.swift`、`MenuBarPanelBridge.swift`、`ClipboardQuickLookController.swift`、`ClipboardEmptyState.swift`、`ClipboardShortcutSettings.swift`。
- AppKit/native 依赖：`NSStatusItem`、`NSPopover`、非激活 `NSPanel`、Carbon `RegisterEventHotKey`/`InstallEventHandler`、`NSEvent` 全局/本地监视器、`NSRunningApplication`、`NSPasteboard`。
- 空状态/错误：引擎未就绪、搜索失败、无历史/无匹配；粘贴失败通过面板错误和全局错误提示。
- MyGo 验证风险：状态项按下即开与激活竞态、面板关闭规则（sheet/编辑器/预览/原生菜单跟踪时不得关闭）、全局热键冲突与注销、粘贴目标应用恢复、Space/全屏 Auxiliary、键盘事件路由、辅助功能/输入监控权限。

### D4 剪贴板列表、归档与设置（P1）

- 入口：剪贴板工作区左侧列表栏；设置 › 剪贴板；新建/重命名/排序/删除列表。
- 主要文件：`ClipboardLibraryViews.swift`、`ClipboardLibrarySettingsView.swift`、`ClipboardLibraryStore.swift`、`ClipboardWorkspaceSidebar.swift`、`ClipboardShortcutSettings.swift`。
- AppKit/native 依赖：SwiftUI sheet、`NSOpenPanel`（`ClipboardLibraryAppPicker.choose()`）、`NSWorkspace` 应用图标、引擎 JSON 命令。
- 空状态/错误：列表编辑校验、保留天数/缓存范围校验、保存失败通过 `SettingsSaveFooter` 显示；远端索引清理。
- MyGo 验证风险：手动列表与智能列表语义、成员长期保留、排除 App 选择、拖拽/重排、保存草稿与快捷键页并发修改、跨设备索引清理。

### D5 文件传输、暂存区与进度浮窗（P1）

- 入口：侧栏“局域网传输”；⇧⌘D；拖动文件时摇动鼠标；手机扫码/浏览器局域网页面；设备页传输活动。
- 主要文件：`FileTransferView.swift`、`ShelfController.swift`、`TransferProgressOverlay.swift`、`NativeFileDrag.swift`、`LanWebSharing.swift`。
- AppKit/native 依赖：非激活 `NSPanel`、`NSEvent` 全局拖拽监视、`NSPasteboard(.drag)`、`NSWorkspace` 文件图标、`NSDraggingSource`、`NSPasteboard` 二维码地址复制。
- 空状态/错误：无已连接电脑、无传输任务、二维码不可用、暂存区无在线目标；失败任务重试、取消状态。
- MyGo 验证风险：Finder 拖入/拖出、文件 lease 与取消、移动 vs 复制修饰键、暂存区摇动手势、非激活浮窗不抢焦点、进度多任务堆叠、受保护目录的完全磁盘访问、`FileTransferView.swift:78` 菜单栏拖放文案与实际实现不一致。

### D6 键鼠共享、显示器布局与 Wi-Fi 场景（P1）

- 入口：侧栏“键鼠共享”；设置 › 快捷键（键鼠控制）；添加设备屏幕 sheet；显示器画布拖拽/吸附/重连。
- 主要文件：`InputView.swift`、`DisplayLayout.swift`、`DisplayLayoutGeometry.swift`、`DisplayWallpaperPublisher.swift`、`InputWifiIdentityView.swift`、`InputDevicePicker.swift`、`InputLatencyChartScale.swift`、`InputLatencyTransition.swift`、`InputLatencyTrend.swift`、`ShortcutSettingsPage.swift`、`ShortcutEditor.swift`、`ShortcutKeycaps.swift`。
- AppKit/native 依赖：Swift Charts；`NSScreen`/`NSWorkspace.desktopImageURL`；ScreenCaptureKit（`SCShareableContent`/`SCStream`）生成桌面预览；`CoreWLAN`/`CoreLocation`（Wi-Fi 名称）；实际输入转发由 Rust 引擎完成。
- 空状态/错误：等待显示器布局、无可添加屏幕、Wi-Fi 标识暂无、主控/被控/已停止被控状态、布局更新未确认。
- MyGo 验证风险：多显示器坐标/缩放/旋转映射、拖拽吸附与跨屏边界、全屏 Space、显示器重连、Wi-Fi SSID 授权、紧急收回快捷键、延迟图表性能与固定 0–100 ms 刻度。

### D7 鼠标设备与按键配置（P2）

- 入口：侧栏“鼠标自定义”（Beta）；鼠标数据贡献 sheet。
- 主要文件：`MouseSettingsView.swift`、`MouseButtonController.swift`、`MouseSettings.swift`、`MouseShortcut.swift`、`MouseDeviceInventory.swift`、`MouseDataContribution.swift`、`LogitechHIDDevice.swift`、`LogitechHIDManager.swift`、`LogitechHIDProtocol.swift`、`G502ButtonLayout.swift`、`G502MouseEditor.swift`、`MXMasterMouseEditor.swift`。
- AppKit/native 依赖：IOKit HID 设备枚举、`CGEvent.tapCreate`（会话级事件 tap）、CoreGraphics、AppKit 输入监控权限、OSLog。
- 空状态/错误：未检测到扩展键、尚未记录按键、未找到设备、输入监控权限提示、恢复默认确认。
- MyGo 验证风险：Input Monitoring 授权、事件 tap 的拦截/放行、按钮编号与设备型号识别、修饰键分层、系统快捷键解析、跨 Mac 配置同步；低层鼠标钩子大概率需要保留原生 helper。

### D8 工作流（P2）

- 入口：设置 › 工作流（功能开启后出现在侧栏）；设置 › 快捷键；剪贴板右键“使用工作流…”；发送到设备菜单；Apple 快捷指令/App Intents。
- 主要文件：`WorkflowSettingsView.swift`、`WorkflowStepEditor.swift`、`WorkflowShortcutRecorder.swift`、`WorkflowRunStatusView.swift`、`WorkflowSyncView.swift`、`WorkflowStore.swift`、`WorkflowModel.swift`、`WorkflowCatalog.swift`、`WorkflowIntents.swift`。
- AppKit/native 依赖：`AppIntents`、`NSEvent` 本地键盘监视器、`NSOpenPanel`（快捷指令绑定）、AppKit 菜单；全局快捷键注册在 Rust 引擎（`WorkflowStore.swift:115` 注释明确包含 Rust 全局快捷键监听启动的任务）。
- 空状态/错误：尚未保存流程、没有工作流、没有适合此内容的工作流、没有可发送设备；删除/舍弃未保存确认、步骤/同步/运行错误。
- MyGo 验证风险：全局快捷键注册与冲突、录制器修饰键限制、工作流与剪贴板/暂存区菜单的联动、运行状态跨窗口、跨设备同步与版本冲突、App Intents 在后台被系统唤起时的焦点。

### D9 麦克风共享（P2）

- 入口：侧栏“麦克风共享”（Beta）；设置 › 通用功能开关；驱动维护 sheet。
- 主要文件：`MicrophoneSettingsView.swift`、`MicrophoneSharing.swift`、`MicrophoneAudio.swift`、`MicrophoneSignalStage.swift`、`MicrophoneSpeechTest.swift`、`MicrophoneDriverControls.swift`、`MicrophoneDriverManager.swift`。
- AppKit/native 依赖：AVFoundation（`AVAudioEngine`、`AVCaptureDevice`）、CoreAudio、AudioToolbox、Speech（`SFSpeechRecognizer`）、`MicrophoneNative` C 模块、`/usr/bin/osascript` 管理员授权安装虚拟驱动、`NSWorkspace` 系统设置跳转。
- 空状态/错误：无已配对电脑、无音频、驱动未安装、连接超时、语音测试未检测到语音；通知卡区分友好文案与原始细节。
- MyGo 验证风险：麦克风/语音识别权限、虚拟音频驱动安装与卸载、CoreAudio 设备切换、音频格式与电平、系统输入设备路由、身份切换（发送/接收）、语音测试在远端共享场景下的可用性。

### D10 远程应用与 Windows RDP（P2）

- 入口：侧栏“远程应用”（Beta）；设备页双击/设备菜单；Dock 菜单；设置 › 高级开启 Windows 远程应用。
- 主要文件（客户端）：`DevicesView.swift`（`showsApplications`）、`RemoteApplicationCatalog.swift`、`RemoteApplicationManager.swift`、`RemoteApplicationPicker.swift`、`RemoteApplicationSession.swift`、`RemoteApplicationTransport.swift`、`RemoteApplicationView.swift`、`RemoteDiagnosticsOverlay.swift`、`RemoteRdpSession.swift`、`RemoteRdpHostConnection.swift`、`RemoteRdpViews.swift`、`RemoteRDPAccountStore.swift`、`RemoteRDPAccountViews.swift`、`RemoteApplicationDock.swift`。
- 主要文件（原生库）：`RemoteVideoView.swift`、`RemoteAppCapture.swift`、`RemoteApplicationDesktop.swift`、`RemoteInput.swift`、`VideoDecoder.swift`、`VideoEncoder.swift`、`Surface.swift`、`RdpBridge/RdpBridge.c/.h`、`DeskFuseVirtualDisplay/DFRemoteVirtualDisplay.m`。
- AppKit/native 依赖：`NSWindow`/`NSToolbar`/`NSHostingController`、MetalKit `MTKView`、VideoToolbox `VTDecompressionSession`、ScreenCaptureKit `SCStream`、CoreMedia、ApplicationServices AX API（`AXUIElement`、`AXIsProcessTrusted`）、CoreGraphics `CGEvent` 注入、FreeRDP C 桥、CoreImage。
- 空状态/错误：目录未就绪、未配对、Windows 未开启、无可用应用、无收藏、无账号；连接/凭据/证书错误、诊断面板。
- MyGo 验证风险：原生视频 surface 不能被 Web/普通图片替代；Metal/VideoToolbox 呈现与缩放；ScreenCaptureKit 权限与窗口捕获；AX API 管理远端窗口（移动/最小化/还原）；CGEvent 输入注入；FreeRDP 帧缓冲与 XRGB32 绘制；多窗口/Dock 图标/菜单；macOS 14.2 可用性与 Beta 状态；`RemoteApplication/Package.swift` 注释要求 P0 双设备门禁通过后才并入正式客户端。

### D11 设置页：通用、权限、登录项、快捷键、文件、关于（P0/P1）

- 入口：侧栏“设置”、⌘,、健康中心跳转、关于与更新。
- 主要文件：`SettingsView.swift`、`SettingsCategoryBar.swift`、`SettingsPageContainer.swift`、`SettingsSaveFooter.swift`、`GeneralSettingsView.swift`、`ClipboardLibrarySettingsView.swift`、`WorkflowSettingsView.swift`、`ShortcutSettingsPage.swift`、`SettingsPermissionsView.swift`、`AboutSettingsView.swift`、`UpdateControls.swift`、`LoginItemSettingsView.swift`、`UninstallSheet.swift`。
- AppKit/native 依赖：`NSOpenPanel`/`NSSavePanel`、`SMAppService`（登录项）、`NSWorkspace`（系统设置/日志目录）、`PermissionFlow`、`Security`（钥匙串/RDP 账号）、AppKit `NSPopover` 与文件选择器。
- 空状态/错误：权限读取中、快捷键读取失败/重试、文件设置未保存/保存失败、缓存/历史清理确认、卸载确认；`SettingsSaveFooter` 统一“未保存/保存中/已保存/错误”状态。
- MyGo 验证风险：设置页状态保留与脏值比较、保存/撤销/错误回显、权限深链与重启、登录项注册/注销、更新通道与手动安装、卸载管理员授权、对话框键盘默认/取消动作；通用/权限/关于属于 P0，工作流/快捷键/文件属于 P1。

### D12 初次设置与功能引导（P0）

- 入口：首次启动、设置 › 高级 › 重走初次设置、功能总开关关闭时的页面。
- 主要文件：`OnboardingFlow.swift`、`OnboardingController`（同文件）、`FeatureOnboardingView.swift`、`FeatureGateView.swift`、`FeatureDemos.swift`、`FeatureDemoKit.swift`、`Permissions.swift`、`SettingsPermissionsView.swift`。
- AppKit/native 依赖：`NSApplication.didBecomeActiveNotification`、`NSWorkspace` 打开系统设置、`PermissionFlow`、`SystemRestartAllowance`（`NSViewRepresentable` 放开模态期间的终止限制）。
- 空状态/错误：权限未授权提示、可先继续、重新启动提示；功能未开启引导。
- MyGo 验证风险：首次启动判定、两步流程、权限状态刷新、系统设置往返、模态期间重启、功能默认值、演示动画在减少动态效果下的表现、中文文案与焦点顺序。

### D13 更新、安装、卸载与旧版本历史导入（P2）

- 入口：设置 › 关于与更新；应用未安装时的安装器；设置 › 高级 › 卸载；启动时检测旧版本。
- 主要文件：`UpdateControls.swift`、`AboutSettingsView.swift`、`InstallationWindow.swift`、`DeskFuseLauncher.swift`、`AppInstallation.swift`、`UninstallSheet.swift`、`DeskFuseUninstaller.swift`、`PreviousInstallationPrompt.swift`、`HistoryImport.swift`。
- AppKit/native 依赖：独立 `NSApplication` 安装器窗口、`NSSavePanel`、`NSMenu`、`NSWorkspace`、`SMAppService`、`Security`、`ServiceManagement`、SQLite3/CryptoKit（历史导入）、`Darwin`（进程/权限）。
- 空状态/错误：安装失败/重试、更新检查/下载/安装状态、旧版本无历史数据库、导入失败、旧应用未退出、卸载确认与管理员授权取消。
- MyGo 验证风险：安装器必须不初始化业务引擎；更新安装/重启与签名、公证、DMG、Homebrew cask 合同；卸载脚本在进程退出后清理；历史导入需退出旧应用并处理 SQLite/文件备份；独立结论，不能用“编译通过”替代。

### D14 支持、健康中心、诊断与全局错误（P0）

- 入口：窗口右下错误浮层、侧栏健康状态、侧栏问号菜单、帮助菜单、关于页诊断按钮。
- 主要文件：`ErrorNotice.swift`、`ErrorNoticeView.swift`、`HealthCenter.swift`、`SupportCenter.swift`、`AboutSettingsView.swift`、`RemoteDiagnosticsOverlay.swift`、`MemoryDiagnostics.swift`。
- AppKit/native 依赖：`NSWorkspace`（打开 URL/访达）、`NSPasteboard`（复制详情）、`NSSavePanel`（导出诊断）、`NSAlert`（旧应用移入废纸篓确认）、SwiftUI overlay。
- 空状态/错误：健康中心“一切正常”/“未连接电脑”/“设备离线”/权限/更新等问题；错误浮层可复制原文和反馈。
- MyGo 验证风险：错误去重与关闭语义、原始错误与友好文案分离、诊断文件脱敏与导出、Issue URL 长度限制、健康问题动作路由、无障碍标签、日志目录定位。

## 7. 迁移优先级汇总

| 优先级 | 功能域 | 原因 |
| --- | --- | --- |
| P0 | D0 应用外壳/导航/设计系统、D2 剪贴板历史、D3 菜单栏面板、D11 设置框架与通用/权限/关于、D12 初次设置与功能引导、D14 支持/健康/错误 | 决定 MyGo 是否能作为可启动、可导航、可回退的 macOS 客户端；也是验证其他页面的前置条件 |
| P1 | D1 设备/配对、D4 剪贴板列表与设置、D5 文件传输/暂存区、D6 键鼠共享/显示器布局、D11 其余设置页 | “基础”协作主路径；先纵向接入真实命令与状态，再替换窗口 |
| P2 | D7 鼠标、D8 工作流、D9 麦克风、D10 远程应用/RDP、D13 更新/安装/卸载/历史导入 | Beta、原生依赖重或涉及独立进程/权限/签名；适合先用原生组件托管，逐项验证后再迁移 |

## 8. 静态验证记录与待验证项

本次已执行：

- 枚举 `apps/deskfuse-macos/Sources/DeskFuse` 与 `apps/deskfuse-macos/RemoteApplication/Sources` 的全部 Swift/C/Objective-C 源文件与行数。
- 用 `grep -n` 核对 `keyboardShortcut`、`NSWindow`/`NSPanel`/`NSPopover`/`NSStatusItem`、`RegisterEventHotKey`、`CGEventTap`、`DFEmptyState`/`ContentUnavailableView`、`.contextMenu`/`NSMenu`、`DFNotice`/`ErrorNotice`、`friendlyError` 等界面入口。
- 阅读 `Package.swift` 与 `RemoteApplication/Package.swift`，核对 AppKit、AVFoundation、ScreenCaptureKit、VideoToolbox、MetalKit、IOKit、Security、ServiceManagement、PermissionFlow、FreeRDP 等链接依赖。
- 核对设置分类、页面枚举、侧栏分组、窗口标识、快捷键默认值与冲突规则。

尚未验证：

- 未运行应用，未做实际界面截图或交互巡检。
- 未验证中文输入法、富文本/剪贴板多格式、Finder 拖放/粘贴、Quick Look、全屏 Space、多显示器缩放、菜单栏面板焦点恢复、远程视频与输入注入、麦克风驱动安装、权限授权/重启、安装/更新/卸载和旧版本历史导入。
- 未做 Windows 端能力与平台差异盘点（属于执行清单下一项）。
- 未验证 MyGo 的 Go 版本、构建链、窗口宿主、全局事件与原生 surface 接入方案。
