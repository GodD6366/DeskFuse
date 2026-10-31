# MyGo UI 替换执行清单

状态：进行中。以当前 macOS UI 的功能、布局和交互为基准，统一 macOS 与 Windows 产品 UI；本阶段保留 Rust 业务与必要的原生组件。用户于 2026-10-27 授权开始实施。

## 执行规则

- 每完成一项立即更新本文件；只有达到该项完成标准才勾选。
- 实现与验证分别记录；不能用编译通过代替实际界面或双设备验收。
- 按功能纵向接入真实后端，不以静态演示、空实现或模拟数据宣称替换完成。
- 保留无关修改；旧实现只在替代路径完成对应验证后删除。
- macOS 是产品 UI 基准；Windows 根据真实 capability 展示差异，不虚构尚未实现的功能。
- 业务状态、命令与平台能力不依赖 MyGo；MyGo 只进入 UI 与应用宿主。
- 本阶段不重写通用 Rust 业务，不改网络协议，不降低更新和认证约束。

## 目标架构（2026-10-31 确认）

- **UI 层**：MyGo WebView + Vue/TypeScript，承载全部产品界面、浮窗与辅助窗口。
- **中间层**：Go（`apps/deskfuse-go`），持有 UI 状态、命令/状态编排、能力与功能策略、跨平台一致行为、窗口管理，以及平台层接口。
- **平台层**：macOS Swift/AppKit 与 Windows 服务/客户端，只暴露系统原语（剪贴板、权限、输入注入、屏幕采集、音频、窗口、安装更新）并回报真实 capability。
- **底层**：Rust 引擎保留，负责输入布局/租约/传输、剪贴板同步、音频共享、协议与身份等低层能力；本阶段不重写，也不移除。

`docs/DERUST_PLAN.md` 是早期“以 Swift/C# 替换 Rust”的备选方案，不属于本架构，不作为执行依据。

### 平台原语契约（2026-10-31）

- 调用方向：Go 中间层向原生只发送 `app.invoke {action, params}`；原生桥接只接受固定 op（`hello`/`app.snapshot`/`app.invoke`/`library.*`/`clipboard.content|thumbnail`），新增原语必须是 action。
- 返回形状：原生动作返回 `{outcome, value?}`；Go 的 `platformCall` 统一解包 `value`，中间层拿到的就是业务值。
- 错误与能力：原生错误码（`invalid_params`/`capability_missing`/`native_error`/`confirmation_required`）原样上抛，中间层不把失败改写成成功。
- 超时：普通动作 15 秒；保存面板、卸载、更新安装等用户操作 10 分钟；首次语音授权 2 分钟。
- 中间层职责：视图组装、模型分组、策略校验、Markdown 生成等放在 `apps/deskfuse-go/internal`；原生只保留平台 I/O 与真实 capability。

## 当前执行目标（2026-10-28 更新）

- [x] 获取并合并最新 `origin/main`（`6d9a9ae5`），保留未提交迁移修改，无冲突。
- [x] 主程序从 MyGo GPU 自绘切换为系统 WebView + Vue 3 + TypeScript。
- [x] 将 macOS `DFTokens.swift`、主窗口、侧栏与面包屑逐项映射到共享 CSS 和 Vue 组件。
- [x] 建立 MyGo 生成的强类型 TS IPC，接入真实 Go 服务与原生后端。
- [x] 建立受控 blob 资源协议，图片不经过 JSON Base64 传给前端。
- [x] macOS 主窗口与菜单栏剪贴板面板默认由 WebView 渲染，`deskfuse-mygo` 随 app bundle 分发。
- [ ] 以最新 main 的 macOS UI 为基准实现全部业务页面、设置、弹层与辅助窗口（文件暂存区拖出、远程应用视频窗口、安装器/驱动弹窗仍为原生；配对确认与传输进度已迁移）。
- [ ] 用实际 WebView 和页面截图逐项比对 macOS 基准；仅外观和行为均核验后宣称一致。
- [ ] 保留原生媒体/输入/剪贴板适配和现有发行脚本；日常前端改动不触发 Rust 构建。

前序 Go 自绘页面仅是探索成果，不再作为交付 UI。前序 stdio bridge 可用于过渡接线，不代表已经完成统一 UDS / Named Pipe v2 的最终边界。不在此任务重写稳定 Rust 业务。

## 1. 基线与技术接入

- [x] 将执行清单写入仓库。
- [x] 盘点 macOS 页面、窗口、菜单、快捷键、空状态和错误交互，记录对应源文件。
- [x] 盘点 Windows 已有能力与平台差异：`apps/deskfuse-service/src/desktop_protocol.rs`、`docs/WINDOWS_DESKTOP_IPC.md` 与共享页面逐项对照，结论写入 `docs/MYGO_WINDOWS_ADAPTER.md`。
- [x] 确认 MyGo 固定版本、Go 工具链和目标系统要求。
- [x] 建立 Go module、MyGo 应用入口、共享 UI 和平台目录。
- [ ] 定义业务命令、快照、结果、事件与 capability 接口。
- [ ] 中间层 Go 化：把 Swift `MyGoBridgeServer` 中的快照组装、命令校验、功能策略与跨平台行为逐步移入 `apps/deskfuse-go/internal`，Swift 只保留平台原语。
- [ ] 平台原语契约：macOS 与 Windows 通过同一组“能力、错误、取消”契约提供系统原语；Windows 服务接入同一契约，而不是各自维护快照语义。
- [ ] 确认并实现 macOS 旧引擎桥接及原生组件宿主方案。
- [ ] 接入 Windows 现有服务 IPC：命名管道 v2 客户端、快照翻译与设备/配对/局域网共享命令已接入（`internal/winservice`），剪贴板、键鼠、工作流、更新与实机验证待完成。见 `docs/MYGO_WINDOWS_ADAPTER.md`。
- [ ] 验证中文输入、富文本和原生视频 surface 接入路径。
- [ ] 验证延迟文件剪贴板与原生拖放接入路径。

## 2. 共享产品界面

每项完成要求：实现对应界面、接入真实命令与状态、检查主要交互，并记录验证范围。

- [x] 主窗口、侧栏、导航、设计 tokens、主题与窗口生命周期。
- [ ] 设备列表、配对、连接、路由详情。
- [x] 剪贴板历史、搜索、分页、筛选和多选。
- [ ] 剪贴板预览、富文本编辑、复制与粘贴（预览与富文本编辑已实现；粘贴回原应用的焦点回放未做实机验收）。
- [x] 剪贴板列表、归档与设置。
- [ ] 剪贴板快捷浮窗、菜单栏与焦点恢复（WebView 面板已替代 popover；目标应用回放未做实机验收）。
- [ ] 文件传输、暂存区、拖放与进度浮窗（传输页与 LAN 分享页已迁移，Finder 拖入改走 MyGo `onFileDrop`；暂存区拖出仍是原生窗口）。
- [ ] 键鼠共享、显示器布局、Wi-Fi 场景与紧急收回（布局画布、主控/被控与 Wi-Fi 标识详情已迁移；原生本无场景规则编辑器，规则只由引擎按 `chooseController(remember:)` 维护，页面不提供额外编辑入口）。
- [x] 鼠标设备与按键配置。
- [ ] 工作流编辑、快捷键、共享与运行状态（设置页、运行状态与“工作流处理后再发送”已迁移；未做实机编辑验收）。
- [x] 麦克风设置、来源选择、状态与驱动维护。
- [ ] 远程应用目录、会话、RDP 账户与原生视频呈现（目录、账号管理、一次性账号打开已迁移；视频窗口仍是原生 surface，未做实机 RDP 验收）。
- [x] 通用设置、权限、登录项与快捷键。
- [ ] 更新、安装、卸载与历史导入入口（关于页与安装/卸载入口已迁移，更新下载安装与卸载的桥接动作已接通；更新安装、卸载与旧安装历史导入未实机验收）。
- [ ] 引导、空状态、错误恢复、诊断与反馈入口（引导页已在 WebView 路由；未做全新安装引导验收）。
- [ ] 应用菜单、托盘、通知、全局快捷键与多窗口行为（状态栏、全局快捷键与菜单仍是原生 AppKit，触发 WebView 窗口）。

## 3. 验证与切换

- [ ] Go 接口与状态转换的针对性测试。
- [ ] 共享 UI 的针对性视图测试。
- [ ] macOS 实际运行：中文输入、富文本、键盘导航、焦点、多选。
- [ ] macOS 实际运行：多显示器缩放、全屏 Space、浮窗与快捷键。
- [ ] macOS 实际运行：原生拖放、Finder 粘贴、媒体窗口与权限交互。
- [ ] Windows 构建与实际运行，核对真实 capability。
- [ ] 无障碍、系统主题、高对比度与减少动态效果。
- [ ] 对照 macOS 基准核验功能和交互，无遗漏页面或操作。
- [ ] 检查启动、内存、CPU、列表滚动和媒体呈现有无明显回退。
- [x] 接入正式构建入口，保持发布和签名合同：`scripts/package-macos.sh` 构建并按同一身份签名 `deskfuse-mygo`，`verify-app.sh`/`verify-dmg.sh` 校验其存在、架构与签名，`scripts/ui-third-party-notices.py` 导出 Go 模块与 npm 生产依赖许可。
- [ ] 清理仅用于本次实施的临时文件。
- [ ] 更新最终已验证范围、阻塞项与残留原生窗口清单。

## 验证记录

- 2026-10-31：设备列表恢复原生产品图。WebView 不能读取 CoreTypes 资源，此前只按名字画通用轮廓；现在宿主新增 `devices.icon` 原语，用与原生 `DeviceProductIconView` 相同的解析（本机 `hw.model` 精确匹配、对端按显示名匹配，找不到时同一个 SF Symbol 回退）栅格化为 PNG，Go 中间层 `internal/devices` 缓存并经 `deskfuse://blob/device-icon/<key>` 提供，快照的 `local` 与每台设备带上 `icon_url`，`DeviceIcon.vue` 有图时直接显示。缓存按“类型+名称+型号”生成 key，改名或换型号会重新解析；平台缺图或报错不影响快照，页面保留绘制回退。冒烟 `target/mygo-webview/smoke-icons/`（`bridged=true`、`network_ready=true`）的 `devices.png` 中本机与对端 Mac 已显示 CoreTypes 产品图（MacBook Pro 刘海与金属底座可见），Windows 对端显示原生同款 SF Symbol 显示器。Windows 适配层没有产品图来源，仍使用页面绘制的回退图标；Apple 系统图像不随包分发，若要两端完全一致需要让对端在协议里自报图标，属于引擎协议改动，未在本轮实施。
- 2026-10-31：工作流与快捷键细节对齐。`workflow.list` 的定义带 `input_type`/`steps` 时，前端按 Swift `accepts(_:)` 同构计算可接受类型（`acceptsContent`），不再把不适用于当前内容的工作流列进“使用工作流/发送到设备”；定义缺少这些字段时保持旧行为。录制快捷键时 `input.set_recording` 新增 `suspend_shortcut`，同时暂停剪贴板全局热键（此前只停引擎，正在录的组合可能被面板抢走），界面进程异常退出时恢复热键；键码表补齐 F13–F20、Home/End/PgUp/PgDn/Insert、小键盘、`Intl*` 与菜单键，与原生“任意键码”一致。面板“粘贴为纯文本”补上防重入。
- 2026-10-31：修复同一个 bundle 出现两个 Dock 图标。根因：原生宿主和 `deskfuse-mygo` 都以普通应用身份运行，而 UI 进程从 `Contents/MacOS` 解析到同一个 bundle，Dock 于是显示两个同名的 DeskFuse。现在 Dock 身份归持有产品窗口的 UI 进程：UI 激活时宿主固定为 `accessory`，可见性仍由宿主计算（`hide_dock_icon` 设置 + 远程应用覆盖）并通过 `ui.dock` 事件推给界面进程，由它 `Dock.Show/Hide`。同轮修正窗口生命周期与安装衔接：关闭主窗口只收起界面（引擎、共享与状态栏继续运行），点击 Dock 图标重新打开；正常退出时 UI 进程结束会一并结束宿主；界面进程在原生桥接管道关闭（宿主被直接结束）时自行退出；`scripts/macos-app-lifecycle.sh` 安装前同时结束 `deskfuse-mygo`。验证：`System Events` 中只有 `deskfuse-mygo` 是前台应用（宿主 `background only=true`）、Dock 只有一个 “DeskFuse” 图标、关闭窗口后两个进程仍在、点 Dock 图标窗口以 1120×720 恢复、`quit` 后两个进程都退出、`kill -9` 宿主后界面进程自行退出；`./scripts/package-macos.sh --auto` 安装 0.7.2 build 271 到 `/Applications/DeskFuse.app`，安装后无旧版本残留进程。仍存差异：远程应用的 Dock 图标徽标/菜单属于宿主，UI 进程持有图标后不可见。
- 2026-10-31：并行审计发现 10 条真实差距并逐项修复：局域网传输改用 MyGo `onFileDrop` 获取 Finder 拖入路径（DOM `File` 在 WKWebView 没有 path），宿主新增 `transfers.inspectPaths` 补名称/大小/目录标记；`system.openUrl` 允许应用生成的 http 局域网地址；“退出设备网络”补 `confirm:true`；高级设置改用 `feature.setEnabled`；剪贴板排除 App/列表条件读取 `{bundle_id}`；工作流设置改用快照 `workflow_runs` 展示运行状态并可取消；“发送到设备”改为每台设备下提供“直接发送 / 使用某工作流发送”；剪贴板图片保存后保留真实路径并允许加入暂存区；远程应用账号支持编辑/删除/一次性账号直接打开（桥接 `remoteApps.open` 新增 username/password/save_account/account_name，保存失败以 `account_error` 回传且不阻止本次登录；账号管理对话框支持新增/编辑/删除与二次确认）。其中 Finder 拖入只验证了 MyGo 事件接线与宿主路径解析，未做真实 Finder 拖放验收；一次性账号与账号管理未做实机 RDP 验收。
- 2026-10-31：最终真实引擎冒烟 `target/mygo-webview/smoke-final4/`：`bridged=true`；鼠标中间层快照（models 6、options 1、G502 位置 11 项）、`deskfuse://blob/mouse/g502-lightspeed-top` 解码 1384×1038、`mouse.contribute.state` 返回 0.7.2/系统版本；`microphone.speech.state` 为 idle 并读到 DeskFuse 麦克风输入；`uninstall.plan` 返回 8 个可卸载项与默认勾选；`transfers.inspectPaths /tmp` 返回真实大小/目录标记；剪贴板面板 19 行、网格 19 张卡片、预览与五个辅助窗口正常。各项页面截图已更新到该目录。
- 2026-10-31：真实引擎冒烟 `target/mygo-webview/smoke-final2/`（1120×720，无 `--mygo-ui`）：`bridged=true`，鼠标快照由 Go 中间层组装（models 6、options 1、G502 位置 11 项），`mouse.locations` 返回真实位置表；`deskfuse://blob/mouse/g502-lightspeed-top` 解码为 1384×1038；`mouse.contribute.state` 返回版本 0.7.2/系统版本，`microphone.speech.state` 为 `idle` 且读到默认输入设备；剪贴板面板 19 行/19 卡片、预览跟随与关闭、五个辅助页面均渲染。限制：本机当前没有受支持的罗技鼠标（只有通用鼠标），照片热点编辑器只验证了组件编译、图片加载与中间层位置/策略，未在真实受支持设备上点按验收。
- 2026-10-31：修复中间层平台调用方式。原生桥接只接受固定 op 集合，`mouse.photo` 等原语以直接 op 发送会被拒绝（冒烟日志 `op=mouse.photo … 不支持的 op`，页面图片 `onerror`）。Go 侧改为统一经 `app.invoke` 调用并解包 `value`，中间层的 `mouse.applyBinding`、`mouse.locations`、`mouse.saveLocations`、`mouse.photo`、`mouse.setWheelMode` 等原语现在走同一通道；集成测试同步断言最后一次 `app.invoke` 的 action。
- 2026-10-31：设置页动作补齐与无障碍样式。桥接新增 `download_and_install_update`（此前关于页按钮会 `unsupported_action`）、`uninstall.plan` / `uninstall.perform`（返回真实可卸载项、默认勾选与 Homebrew 提示，执行走 `DeskFuseUninstaller.perform`，取消管理员授权返回 `completed:false`）；用户驱动的保存/卸载/更新动作在 Go 侧放宽等待时限，首次语音授权 2 分钟。旧安装迁移 sheet 仍挂在原生主窗口上：WebView 模式下由 `MyGoUIHost.revealNativeMainWindow()` 在需要时临时放回前台，完成后重新收起，避免用户看不到提示导致引擎永不启动。全局样式补 `prefers-contrast: more` 与 `forced-colors: active`（减少动态效果此前已有）。新增 `internal/webview` 集成测试覆盖快照鼠标组装、鼠标图片 blob 与 `mouse.setMapping` 中间层路由。
- 2026-10-31：中间层 Go 化起步（鼠标域）+ 麦克风语音测试接线。新增 `apps/deskfuse-go/internal/mouse`：由 Go 组装鼠标视图（型号分组、`options`、电池与 SmartShift 能力连接、G502 位置缓存），执行映射策略（实体键禁止映射鼠标键/停用、G502 实体键自动清理同位置通用映射、`locateG502` 交换被占事件），生成适配数据 Markdown 与文件名；Swift 侧只保留平台原语 `mouse.applyBinding`/`mouse.locations`/`mouse.saveLocations`/`mouse.locateG502`/`mouse.beginDetection`/`mouse.endDetection`/`mouse.clearCapture`/`mouse.setWheelMode`/`mouse.photo`/`mouse.contribute.*`，快照新增 `battery`/`wheel`/`g502_locations`/`last_button`/`capture_released`。`deskfuse://blob/mouse/<name>` 由 Go 缓存四张内置鼠标图片。前端 `MousePage.vue` 改用中间层 `models`/`options`/`g502`，新增原生同构的 `MousePhotoEditor.vue`（顶视/侧视、热点、滚轮三段选择、重新识别位置）与 `MouseContributionDialog.vue`（探针采集、实时 Markdown、复制/保存）；保存 Markdown 走真实保存面板结果。麦克风语音测试由占位改为可用：桥接新增 `microphone.speech.{state,prepare,toggle,stop,clear,dismiss,settings}`，复用原生 `MicrophoneSpeechTester`（同一状态机，避免并行录音），页面轮询电平与识别文字、聆听中只读、离开页面即停止。Go 测试 61 项、`vue-tsc`、前端构建、Swift `swift build` 均通过。
- 2026-10-31：剪贴板面板第二轮对齐。Go 新增 `Preview` 服务（`cmd/deskfuse/preview.go`）：空格预览改成贴住选中行的系统 NSPopover（MyGo `Popover` flyout），材质、圆角、阴影与箭头由 AppKit 绘制；显示后立即把键盘还给面板，系统 popover 再次夺走键盘时由预览页把空格/方向键/回车/Esc 转回面板列表。页面 `?page=quick-look` 按原生 `ClipboardQuickLook` 的尺寸与内容规则渲染（图片等比缩放 696×536、文本 360–600 × 160–460、代码 640×480、文件行 300×116），只有远端索引条目才向来源设备请求内容。面板网格改为原生 `MenuBarClipboardCard` 同构卡片与双列瀑布流（来源 App 水印、居中时间、⌘n 保留位），列表行把 ⌘n 放到时间下方并去掉悬停高亮；标签补齐计数与「新建列表」，⌘F 提示只在按住 Command 时显示；空状态、页脚错误与「查看全部 / 使用工作流… / 发送到设备」对齐原生；右键菜单补「粘贴至“目标”」「粘贴为纯文本」「预览」；标题编辑改为页面内对话框；计数改为在 `state` 之后串行请求，避免 `counts_revision` 竞态丢弃结果。`clipboard.save` 现在等待原生保存面板结果（取消返回 `saved:false`，失败抛出真实错误；Go 侧仅该动作把等待时限放宽到 10 分钟）。冒烟 `target/mygo-webview/smoke-popover-final/`：面板 20 行真实记录与计数（全部 5520 / 纯文本 3987 / 图片 1428 / 文件 84）、瀑布流 20 张卡片、文本预览 popover（frame 453×186、内容 427×160）、图片预览（frame 746×563、内容 720×537）、方向键跟随 181054→275685、空格关闭为 true。`DESKFUSE_UI_SMOKE_SAVE=1` 冒烟 `target/mygo-webview/smoke-save3/` 自动取消保存面板，验证 `clipboard.save` 回传 `{saved:false}`（保存结果回传走通）。
- 2026-10-31：键鼠页拖动吸附改由引擎计算：桥接新增 `input.previewDisplayMove`（冻结的屏幕集合 + 逻辑位移 + attach），拖动时按帧合并请求，松手再用 attach 模式吸附一次；吸附参考线按引擎返回的偏移重算。动作探针（`smoke-layout2/`）返回 `{x:10,y:10}`。离开键鼠页时清空壁纸缓存、重试计数与未完成的定时器。
- 2026-10-31：远程应用目录补齐原生字段：桥接目录新增 `running`/`opened`/`icon`（PNG data URL）与 `favorites`，新增 `remoteApps.setFavorite`（与原生选择器共用 `remote-rdp-favorites.<peerId>`）与 `remoteApps.setVirtualDisplay`。前端按原生 `RemoteApplicationPicker` 增加 10 个分类（全部/运行中/效率与财务/工具/开发者工具/社交/娱乐/信息与阅读/创意/其他）、运行中优先排序、「已打开/运行中」状态点、64px 应用图标、虚拟屏幕开关与底部「N 个已打开」。冒烟 `target/mygo-webview/smoke-remote/`：真实目录 124 个应用、124 个图标、14 个运行中；`setVirtualDisplay` 往返成功；`input.previewDisplayMove` 返回 `{x:10,y:10}`。
- 2026-10-31：预览浮窗的真实键盘归属无法在冒烟里判定：从终端启动的应用无法被激活，`isKeyWindow` 恒为 false；实现沿用原生 `makeKey` 方案，并额外加了一层 popover 页面按键转发。保存面板成功路径需要用户点“保存”（NSSavePanel 没有可用的程序化确认入口），只验证了取消路径与写入代码本身。以上为剩余真实差异。
- 2026-10-30：剪贴板继续对齐。桥接新增 `clipboard.appInfo {bundle_id}`（NSWorkspace 图标 data URL + 显示名），快照新增 `clipboard.automatic_ocr`、`workflow_runs`（含 `step_index`）与 `workflow_error`。前端：历史行缩略图回退显示来源 App 图标、瀑布流卡片 App 水印、预览「来源应用」图标与名称；预览与面板底部显示最近一条工作流运行（运行中转圈、步骤、message、输出尺寸/类型、字节对比、失败/取消、运行中可取消）；自动 OCR 受开关控制并按 id 去重。冒烟截图 `target/mygo-webview/smoke-clip2/`（页面与面板均显示真实 App 图标）。
- 2026-10-30：剪贴板仍存的差异：原生保存面板的完成结果无法回传（`clipboard.save` 只确认派发，失败经 `snapshot.clipboard.error` 展示）；面板网格模式为简化卡片；快速预览为页内浮层而非原生 NSPopover。以上为剩余差异记录。

- 2026-10-30：设备 / 剪贴板 / 键鼠共享 / 剪贴板悬浮窗按「功能对齐」补齐。桥接新增 `clipboard.save/edit/share`（含 `workflow_id`）、`workflow.run`、`clipboard.sources`、`clipboard.chooseApp`、`clipboard.indexOCR`、`clipboard.cancelPreparation`、`clipboard.showInMain`、`panel.hide`、`app.showMain`、`app.quit`、`devices.leaveNetwork`、`remoteApps.openPicker`、`input.setParticipant`（同步自动加入抑制）、`input.chooseController`、`feature.setEnabled(windows_remote_applications)`；`clipboard.copy/paste` 改为等待真实完成并返回 change count/错误。快照新增 `clipboard.history/paste_target/navigation`、`input.rules/peer_ssids/quality.samples/quality.suggestions`、`displays[].wallpaper_jpeg`、`input.remote_application_restriction`、`remote_applications.windows_enabled/error`、`local.model_identifier`。
- 2026-10-30：面板按原生 `MenuBarClipboardView` 重写：标题行（显示模式/来源/暂存区/更多）、⌘F 搜索与清除、列表标签（全部+类型+我的列表）、单击选中/双击粘贴、⌘1-9 粘贴、⌘C/⌘S/⌘E/⌘O/⇧⌘R/空格快速预览/Delete/方向键/Esc、右键菜单（复制/粘贴/标题/保存/编辑/预览/访达/加入列表/发送到设备/工作流/删除）、分页、30 秒库刷新、打开时重新查询。页面补齐：首次自动选中、删除落点、⌘C、保存图片、适合/填满、访达按钮、来源 `clipboard.sources`、OCR 调度、快照即时合并、分页重试、双显示模式记忆、`clipboard.navigation` 一次性应用、来源显示名规范化、功能关闭引导页、拖拽框选。键鼠页补齐：延迟曲线（迷你 + 弹层大图、0–100ms 固定刻度、>2s 断段、峰值标注、网络/采集卡顿色条）、P50/P95/网络卡顿/慢注入、数据报降级徽标、优化建议详情、Wi-Fi 首选主控/对端上报、重连同时重新拨号、参与设备走 `input.setParticipant`、主控暂停走 `chooseController`。
- 2026-10-30：动作探针（冒烟内 `DESKFUSE_UI_SMOKE_ACTIONS=1`）验证新桥接真实存在：`clipboard.sources` 与 `panel.hide` 成功，其余动作按原生校验返回预期错误（“内容 ID 无效”“需要内容与设备”“需要工作流与内容”“退出设备网络需要确认”“设备不存在”“需要 id”等）。冒烟产物 `target/mygo-webview/smoke-actions/`；面板截图显示标签、来源、页脚菜单与 18 行真实记录。
- 2026-10-30：四块界面仍未对齐的细节：剪贴板来源 App 图标与工作流运行状态未桥接；保存面板结果无法回传完成态；键鼠页拖动吸附仍用前端近似算法（未接引擎 `previewDisplayMove`）；离开键鼠页未清理壁纸缓存；远程应用目录的运行中状态/应用图标/收藏同步仍缺。以上保持真实缺口记录。

- 2026-10-30：以原生窗口为基准完成逐页对齐。新增开发期截图基线 `DESKFUSE_CAPTURE_UI=<目录>`（`Sources/DeskFuse/UICapture.swift`，等待引擎加入设备网络后逐页 `cacheDisplay` 出图），在相同 1120×720 下与 WebView 冒烟截图逐页比对（原生 `target/ui-baseline-now/`，WebView `target/mygo-webview/smoke-final8/`）。对齐内容：剪贴板默认瀑布流与真实缩略图、⌘F 提示、「所有设备」筛选、溢出菜单、预览空状态；设备页 380px 列表与详情卡/线路卡/操作按钮；键鼠页质量徽标与本机屏幕壁纸；局域网页真实二维码与地址；设置 8 个分类的表单宽度、行高与文案；侧栏品牌图标、运行状态摘要（与 `HealthCenter` 一致的“设备离线/一切正常/N 项需要留意”）与帮助菜单四项。
- 2026-10-30：为对齐补齐桥接：`lanWeb.qr`（CoreImage 生成二维码 data URL）、`input.wallpaper`（ScreenCaptureKit 截图，缓存未命中时后台采集并由页面重试）、`input.quality` 快照字段、`support.copy_environment`、`pairing.approve/reject`，以及 `MyGoBridgeServer` 的配对窗口/传输进度窗口事件 `ui.aux`。`scripts/package-macos.sh --fast` 实测通过（176 s）：包内含 `deskfuse-mygo` 与 `THIRD_PARTY_UI.txt`，`verify-app.sh` 校验 Mach-O 最低 13.0、SDK 26.2 与架构一致。
- 2026-10-30：排查启动异常时确认：若上一次冒烟结束后父进程仍在运行，`open -n` 会拉起第二个实例，两个实例共用同一引擎状态会导致网络/主控状态异常（`network_id` 为空、`can_configure=false`）。冒烟脚本现改为强制清理实例后再启动，并等待 `local.network_id` 就绪后截图；这不是 UI 缺陷。
- 2026-10-30：尚未对齐的差异：设备产品图用同形态 SVG 替代原生 CoreTypes 图标；远程应用目录缺少运行中状态、应用图标与收藏同步；鼠标照片热点编辑器、电量/滚轮信息、麦克风语音测试仍缺桥接数据与动作；剪贴板源应用图标、保存/分享动作、工作流运行状态未接入。以上均为真实缺口，未用静态占位冒充。

- 2026-10-30：macOS 产品界面切换完成。`deskfuse-mygo` 打进 `Contents/MacOS`，`MyGoUIHost` 无参数默认启动它（`--mygo-ui` 仅在覆盖路径时使用），原生 `deskfuse-main` 窗口在 WebView 激活后被收起；`openMain`/`openSettings`/菜单跳转改发 `ui.present` 事件，由 Go 聚焦窗口并切页。菜单栏面板改为 MyGo 面板窗口（`?page=clipboard-panel`），原生只保留状态栏、全局快捷键、目标应用捕获与 `pasteRestoredContent` 的辅助功能校验和 change-count 校验；面板失焦自动收起，`panel.toggle` 状态由 Go 持有。引导页在 WebView 中显示（`?page=onboarding`），`onboarding.complete` 写回功能开关。
- 2026-10-30：真实引擎冒烟（默认启动，无 `--mygo-ui`）通过：`bridged=true`，快照 10 KB，剪贴板首屏 306 ms 内返回真实分页（5460+ 条、按类型分组计数、真实来源与时间），设置页 9 个开关读到真实状态（登录项“已开启”），面板窗口渲染 20 行真实记录。产物 `target/mygo-webview/smoke-default/`（各页面 PNG、`panel.png`、`snapshot.json`）。未点击业务按钮，未做双设备、权限授予、拖放与远程视频验收。
- 2026-10-30：MyGo v0.7.1 的 `StartDataDrag` 要求窗口显示 native UI，WebView 窗口不能发起原生数据拖拽；因此暂存区的拖出仍由原生 `ShelfController` 承担，不用普通文件列表替代。

- 2026-10-29：完成首次真实引擎联调。`CODESIGN_IDENTITY=- ./scripts/build_and_run.sh build` 生成 `dist/DeskFuse-dev.app`，以 `--mygo-ui target/mygo-webview/deskfuse-mygo` 启动后，WebView 通过 stdio 桥读取真实快照：侧栏显示“运行正常”，设备页列出 4 台已配对设备（2 台在线，含线路与 24 ms 延迟）、本机信息与配对状态；设置页读取到真实功能开关与登录项状态。冒烟产物在 `target/mygo-webview/smoke-real/`（`snapshot.json`、各页面 PNG）。
- 2026-10-29：修复设置页开关状态不显示。`AsyncToggle`、剪贴板设置的自动 OCR、工作流启用开关使用 `label.toggle > input + span`，而全局 `.toggle` 样式作用于 input 本身，导致 `:checked` 永远不命中、开关一直显示关闭；三处统一改为 `input.toggle`。修复后剪贴板/键鼠/局域网/远程应用/鼠标显示为开启，麦克风显示为关闭，与真实设置一致。
- 2026-10-29：观察到两个未完成项：设备页首次进入时快照中设备列表可能尚未到达（下一次 1.5 s 轮询后补齐），局域网传输页仍是简化空态（只有“启用局域网传输”入口，缺发送区、传输队列与扫码）。剪贴板历史为空是因为本机 `clipboards.db` 无记录，未构造测试数据。

- 2026-10-28：新增 Windows 桌面服务适配层 `apps/deskfuse-go/internal/winservice`：小端长度前缀帧、`0x06` 读确认、请求编号校验、`Rejected.boundary_error` 映射、快照字段翻译，以及设备/配对/文件开关/局域网共享命令。未支持范围显式返回 `unsupported_platform`，不打开连接、不伪造状态。通过 `go -C apps/deskfuse-go test ./...`、`vet`（含 `GOOS=windows`）、`GOOS=windows GOARCH=amd64/arm64 go build ./...`、`git diff --check`。未运行 Windows 实机，管道鉴权、会话切换、真实服务与双机行为仍未验证。

- 2026-10-28：快进合并最新 `origin/main`（`6d9a9ae5`），保留已有未提交改动。主程序切换系统 WebView，新增独立 Vue/Vite 前端、macOS tokens、生成的 MyGo TS 绑定及 `scripts/build-mygo-ui.sh`；脚本不构建 Rust。Vue `build`、Go `test`/`vet` 与 blob/ID 精度边界测试通过。
- 2026-10-28：启动真实 MyGo WKWebView，完成剪贴板页加载、设置页切换、页面状态和截图检查，产物 `target/mygo-webview/smoke/`；1120×720 下 scrollWidth=1120。该次未接通原生后端，不证明真实功能或视觉完全一致。浏览器空间 1、2 均已显式关闭并返回 closedSpace=true。
- 2026-10-28：部分并行任务因模型服务限额中断，已恢复其草稿并进行类型/Vite 编译检查；剪贴板主页面修复模板缺少闭合标签、分页依赖不精确总数、迟到结果覆盖等问题；删除列表新增确认。功能页仍有明显简化，保持对应完整迁移项未勾选。
- 2026-10-28：此前原生全量构建在下载 boringssl 期间被会话终止，未完成。现复用本机已有 FreeRDP 静态库和已安装音频库，在当前 worktree 构建最新 main Rust 及生成绑定；真实原生 target 结果待记录。

- 2026-10-27：剪贴板只读页已接入 stdio 客户端，包含搜索、游标分页、迟到结果丢弃、加载/空/错误状态、历史列表和文本摘要预览；尚缺完整内容、多选、筛选及真实宿主联调，因此整项不勾选。集成时修复 Go 与 Swift 操作名不一致，统一为 `library.query`。主工作树通过 `go -C apps/deskfuse-go test -race ./...`、`go -C apps/deskfuse-go vet ./...`、`git diff --check`。
- 2026-10-27：原生标准构建在准备 Roc 等依赖期间达到后台时限被停止，不代表源码编译失败或通过。已将时限延长至两小时继续增量构建；真实 Swift target 与真实引擎查询仍待验证。

- 2026-10-27：合并 `apps/deskfuse-go` 独立 module，固定 MyGo v0.7.1，本机 Go 1.27.3。子任务通过 macOS arm64 与 Windows amd64 的无 cgo 编译；主工作树复核通过 `go -C apps/deskfuse-go test ./...`、`go -C apps/deskfuse-go vet ./...`、`git diff --check`。当前仅有可编译导航骨架，未接入真实业务或 capability，未启动图形窗口，不能算产品 UI 替换完成。目标系统要求见模块 README，实机兼容性仍待验证。

- 2026-10-27：已检查工作树，起始无修改；仓库尚无 Go module；本机存在 `/opt/homebrew/bin/go`。尚未验证 Go 版本或 MyGo 构建。
- 2026-10-27：完成 macOS 界面静态盘点并写入 `docs/MYGO_UI_BASELINE.md`。范围为 `apps/deskfuse-macos/Sources/DeskFuse`（138 个 Swift 文件、30,971 行）与 `apps/deskfuse-macos/RemoteApplication/Sources`（18 个 Swift 文件、4,026 行，另有 C/Objective-C 桥接），基线提交 `446c92a6`。已按功能域记录入口、主要源文件、AppKit/native 依赖、迁移优先级和 MyGo 验证风险，并单独列出窗口/面板、菜单、快捷键、空状态与错误交互。仅完成静态盘点和源码核对，未运行应用，未做实际界面、权限、拖放、多显示器或双设备验收；这些项目保持未勾选。

## 仍在原生的窗口（2026-10-30）

| 窗口 | 原因 | 下一步 |
| --- | --- | --- |
| 文件暂存区 | MyGo v0.7.1 的 `StartDataDrag` 要求窗口显示 native UI，WebView 窗口无法发起原生文件拖出 | 用 `mygo/ui` 原生 UI 重写该小窗，或等框架支持 WebView 数据拖拽 |
| ~~配对确认窗口~~ | 已迁移为 MyGo `Panel` 窗口（`?page=pairing`），由原生以 `ui.aux` 事件驱动显示；接受/拒绝走 `pairing.approve` / `pairing.reject` | 实机双设备配对验收 |
| ~~传输进度浮窗~~ | 已迁移为 MyGo `Panel` 窗口（`?page=transfer-progress`），置顶于光标所在屏幕右上角，失败/取消可关闭，取消 3 秒后自动收起 | 实机传输验收 |
| 远程应用视频窗口 | ScreenCaptureKit/VideoToolbox/Metal 与 FreeRDP 必须留在原生 | 保持原生 surface，仅工具栏与状态可逐步迁移 |
| 安装器与驱动维护弹窗 | 需要管理员授权与签名校验路径 | 保持原生，界面文案已在 Vue 设置页复述 |

## 阻塞与待验证事项

- 2026-10-31 当前遗留（均为实机/硬件/授权门槛，不是未实现代码）：
  - Finder 真实拖入只验证了 MyGo `onFileDrop` 接线与 `transfers.inspectPaths` 返回，未在 Finder 里实际拖放。
  - 本机没有受支持的罗技鼠标，照片热点编辑器未在真实设备上点按；图片 blob、G502 位置表与映射策略已由单元测试和冒烟覆盖。
  - 语音测试验证了状态机与页面接线（idle、默认输入设备），真实识别需要用户授权并说话，未自动执行。
  - 更新下载安装、卸载、RDP 账号保存/删除与一次性账号登录涉及管理员授权、钥匙串或远端 Windows，未执行破坏性/授权流程。
  - 粘贴回原应用的焦点回放、中文输入、多显示器/全屏 Space 仍需 macOS 实机验收。
  - 远程应用的 Dock 图标徽标与 Dock 菜单仍由原生宿主实现；WebView 模式下 Dock 图标归 UI 进程，这两项不可见（远程应用自身的窗口列表仍在页面与菜单栏入口里）。
  - Windows 实机与双设备配对/传输验收未做；Windows 剪贴板、键鼠、工作流命令仍未接入。

- 2026-10-27：直接运行 `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer swift test --package-path apps/deskfuse-macos --disable-sandbox` 首先在 SwiftPM target 路径检查失败：`invalid custom path 'RemoteApplication/Vendor/FreeRDP' for target 'CDeskFuseRdp'`。当前工作树缺少生成的 FreeRDP 依赖，尚未进入 Swift 源码编译。已启动项目 `scripts/build-native-macos.sh debug` 准备依赖并检查真实宿主；结果待记录。桥接的临时替身测试不能替代真实 target 验证。

- 原生视频 surface 与高级延迟文件拖放需验证，不能以普通图片或文件列表替代。
- macOS 旧引擎目前使用 UniFFI 进程内边界，桥接方案需要同时考虑主线程原生回调、权限归属与应用生命周期。
- Windows 与双设备实际验收取决于可用设备；未执行前保持未勾选。
