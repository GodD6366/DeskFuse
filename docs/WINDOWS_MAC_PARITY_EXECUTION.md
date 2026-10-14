# Windows / macOS 产品能力对齐执行清单

## 目标与判定

Windows 完整对齐当前 macOS 的功能、导航、操作路径、默认值、数据语义和失败行为。只有 Apple 专属系统集成可以例外；存在 Windows 等价实现的 OCR、文件预览、托盘、保持唤醒和 Wi-Fi 布局恢复仍在范围内。

本清单以两端当前源码为依据。旧 `WINDOWS_CLIENT.md` / `WINDOWS_MAC_PARITY_HANDOFF.md` 中 WPF 描述及能力状态存在滞后，不能作为当前验收结论。

状态约定：待实现、部分实现、已实现待实机验证、实机通过。构建通过不代表视觉、安装或双设备验证通过。每次后续功能修改需要补充实际检查结果。

## 第一批：导航与现有功能入口

| 项目 | 状态 | 当前结果 / 剩余项 |
| --- | --- | --- |
| 主导航分组与名称 | 已实现待实机验证 | 工作区：剪贴板、鼠标共享、远程应用；连接：设备、分享与传输 |
| 页面路由 | 已实现待实机验证 | 用具名页面 ID 替代固定数字索引，Ctrl+F 跟随页面 |
| 品牌与侧栏底部 | 部分实现 | 品牌、副标题、版本及设置进入侧栏；收起与更新状态仍待补齐 |
| 远程应用内嵌 | 部分实现 | 将现有本机应用发布管理器移入主页面，修复原窗口未挂载内容；远端目录/客户端会话仍未完成 |
| 工作流入口 | 已实现待实机验证 | 已有编辑器移入设置，剪贴板入口打开同一个编辑器 |
| 设置分类 | 部分实现 | 通用、剪贴板、存储、工作流、更新已有分类；权限、键鼠共享、快捷键分类及缺失设置待接入真实功能 |
| 历史清空确认 | 已实现待实机验证 | 在设置所属 XamlRoot 显示确认对话框，取消不清空 |
| 设置关闭重开 | 已实现待实机验证 | 关闭隐藏，重新显示保留同一窗口和编辑器 |
| 小屏启动 | 已实现待实机验证 | 修正工作区小于 1280×800 时 Math.Clamp 下限大于上限的问题 |
| 分享与传输 | 部分实现 | 迁移现有传输队列入口；二维码和局域网网页服务尚未接入，不能据页面名称判断完成 |

## 后续工作包与依赖

### B1：内容库与历史迁移

依赖：先确定按 Windows 用户隔离的 Rust 内容库访问契约，沿用服务身份与会话边界。

- [ ] 对照 `ClipboardView.swift`、`ClipboardWorkspaceSidebar.swift`、`ClipboardLibraryStore.swift` 逐项补充操作清单。
- [ ] 接入 Rust canonical 内容库；迁移旧 Windows 历史，保留时间、来源及多格式正文，迁移可恢复。
- [ ] 保留天数、到期清理、分页、去重、标题编辑、内容编辑。
- [ ] 手动列表与多列表归档；智能列表及全部/任一条件。
- [ ] 类型、应用、设备、文字、日期筛选及完整文字搜索。
- [ ] Windows OCR、识别文字索引与图片/代码/文件预览。
- [ ] 列表定义、成员、归档、索引同步、正文按需获取、删除墓碑、离线冲突。
- [ ] 应用来源标识与应用排除；索引同步开关和清除远端索引。

验收：Mac ↔ Windows 的创建、归档、编辑、删除、检索、离线重连与并发操作；正文未到齐不得误报可恢复。

### B2：快捷面板与暂存区

依赖 B1 的查询/恢复接口；UI 外壳和原生焦点适配可并行。

- [ ] 托盘与全局快捷键共用面板，跟随鼠标屏幕和 DPI。
- [ ] 搜索、方向键、回车、数字快捷键、双击及右键操作。
- [ ] 仅复制与粘贴到原应用分离；校验焦点、剪贴板代次、修饰键与会话。
- [ ] 暂存区快捷键、拖文件晃动唤起、拖入拖出、展开和设备/工作流发送。

验收：浏览器、编辑器、Office、Explorer、中文输入法与多屏；无错贴、重复粘贴或搜索误删。

### C1：设备与鼠标共享

可与 B1 并行，修改输入服务和原生适配时独立分工。

- [ ] 发现、手动定位、配对、授权、连接路由、本机详情与撤销信任逐项核对。
- [ ] 主控/被控角色、参与设备与激活状态。
- [ ] 布局拖动、吸附、负坐标、旋转、混合 DPI 与保存失败恢复。
- [ ] 穿越模式、禁用角落、方向与锁定快捷键、冲突状态和紧急收回。
- [ ] 命名布局、Wi-Fi 绑定、主控偏好、恢复规则。
- [ ] UIAccess 实际权限、休眠/解锁/断线/显示器热插拔恢复。

验收：两种主控方向、混合 DPI 和至少两屏；安全桌面、登录与 SAS 不误报支持。

### C2：分享与传输

网页列表分享依赖 B1，其余传输与 LAN 服务接口可独立推进。

- [ ] 接入 LAN 服务状态、开关、二维码、地址、网卡与端口。
- [ ] 核对当前 Mac 网页列表分享与手机上传行为，补齐服务接口。
- [ ] 接收任务、队列、取消、错误、小文件预传、缓存容量和清理。
- [ ] Explorer OLE 按需粘贴/拖放：多文件、目录、空目录、重名、源变化与取消。

验收：手机 → Windows、Mac ↔ Windows；哈希和目录结构、断线、重试与缓存回收。

### D1：工作流

可与 C1/C2 并行；历史入口依赖 B1。

- [ ] 完整步骤编辑器、排序、类型检查、复制/删除/启停及快捷键录制。
- [ ] 文字、批量重命名、ZIP、图片压缩/缩放/格式转换及末尾输出。
- [ ] 本地运行、取消、原应用粘贴、共享结果与符合 Mac 契约的失败回退。
- [ ] 设备/类型自动规则、优先级及手动跳过。
- [ ] 定义/规则同步、删除标记、并发恢复、草稿保护与快捷键冲突。

验收：所有非 Apple 专属步骤真实执行；共享发送端只执行一次，接收端不重复执行。

### D2：远程应用

独立模块，可与内容库、输入开发并行。

- [ ] 分别审计 Windows 提供端和客户端，对照 Mac 目录、传输和会话实现。
- [ ] 可信设备目录、应用选择、启动、渲染、键盘/鼠标与焦点。
- [ ] 窗口大小、断线、关闭、重连、信任撤销及资源释放。

验收：Mac → Windows 和 Windows → Mac 真实应用操作。现有本机发布管理器不能代表客户端能力完成。

### E：设置与发行

各模块设置随对应能力合入；升级事务与签名可独立开发。

- [ ] 通用：启动项、托盘/任务栏显示、保持唤醒、日志和诊断导出。
- [ ] 权限：实际能力、原因及修复入口。
- [ ] 所有设置默认值、草稿、保存/撤销和失败回显。
- [ ] 快捷键总览、录制、冲突和恢复默认。
- [ ] 更新状态、检查/下载/安装、GitHub 优先与域名回退。
- [ ] Windows 签名载荷、GUI/service/helper 原子版本、安装升级、自检和回滚。

发行相关变更执行项目要求的 `git diff --check`、`python3 scripts/release.py validate`、`cargo check --locked`；Rust 工作区变更另执行 `./scripts/check.sh`，并补充 Windows 原生检查。

## 平台例外

- Apple Shortcuts / App Intents：系统集成可例外；同步步骤保留定义并明确指出本机依赖不可用。
- Universal Clipboard 的 Apple 私有来源标记：机制可例外，去回环的用户结果仍须实现。
- Finder / Quick Look、菜单栏 / Dock、Apple 权限面板：用 Windows 等价入口和实际能力实现。
- OCR、保持唤醒、Wi-Fi 恢复、通用图片格式转换不得因平台 API 不同而豁免。

## 第二批：历史保留、编辑与 Windows 用户存储

已实现、待实机验证：

- Windows 共享 SQLite 默认路径改为 `%LOCALAPPDATA%/DeskFuse/state.sqlite3`，拒绝缺失或相对目录；服务继续使用现有显式机器存储路径。此改动只准备用户存储位置，不代表 JSON 历史已迁入共享内容库。
- Windows 历史保留天数可保存/撤销；缩短期限需确认，保存偏好失败不执行新规则清理。启动时先读取配置，再加载历史；每小时维护，恢复和标题修改前复核过期状态。历史不设条数或累计大小上限；跨设备传输仍遵守传输通道和单次内容限制。
- 条目自定义标题可编辑/清空、持久化和搜索，按 200 个 Unicode 标量限制；同来源相同内容再次复制保留标题，原始格式、来源和复制时间不因标题编辑改变。
- 独立内容编辑窗口支持粗体、斜体、下划线、删除线和纯文本；保存写入系统剪贴板，按已开启的记录/同步规则处理，不原地覆盖原历史条目。
- 修正历史清理：只按保留天数移除到期条目，不按历史条数或累计字节数截断。

- [x] Windows 本地历史去除条数和累计大小上限；保留天数仍负责到期清理。
- [x] Windows 保持唤醒设置：`SetThreadExecutionState` 阻止系统与显示器休眠，退出释放。
- [ ] 共享内容库迁移、手动/智能列表、OCR、应用排除、索引和正文同步。

本批验证：

- `dotnet build apps/deskfuse-windows/DeskFuse.Windows.csproj -c Debug -p:Platform=x64 --no-restore`：通过，0 警告、0 错误。
- `dotnet run --project tools/DeskFuse.HistoryChecks/DeskFuse.HistoryChecks.csproj --no-restore`：8 项通过；覆盖旧文件/多格式往返、标题去重与 Unicode、保留期、源文件保护、容量清理和损坏文件保护，全部使用临时目录。
- `cargo check --locked -p deskfuse-store --lib`：在 MSVC 开发环境中通过。
- `rustfmt --edition 2024 --check crates/deskfuse-store/src/lib.rs`、`git diff --check`：通过。
- `cargo test --locked -p deskfuse-store --lib`：首次在依赖构建脚本执行阶段失败，`getrandom` 的 `build-script-build` 被 Windows 应用程序控制策略阻止（`os error 4551`），测试未运行。
- `cargo check --locked`：依赖编译阶段失败，工作区包含 Apple 专属 `objc2` / `core-foundation` 依赖，不能在 Windows 全量编译。
- `./scripts/check.sh`：首个失败阶段为 `cargo fmt`，已有 `crates/deskfuse-engine/src/library_sync.rs` 存在格式差异；脚本后续检查未执行。
- 未进行真实应用剪贴板写入、富文本粘贴、UI 视觉、双设备、安装或更新验收。

## 第二批核查：合并 origin/main（0.5.x 最新能力）后的 Windows 对齐审查

合并 `ed4f92b..a1711c9`（10 个提交）后逐项核查，结论如下。

已对齐（本次合并落实）：

- 流畅度诊断（相对运动质量上报）：main 的 Windows 侧改动基于已废弃的 WPF 客户端，已移植到 WinUI 3 客户端——XAML 在键鼠共享页补入 `InputQualitySummary/Metrics/Mode/Advice/Canvas`，曲线绘制改用 `Microsoft.UI.Xaml.Shapes.Polyline` + `SolidColorBrush`。客户端解析字段与 `deskfuse-input/src/quality.rs` 的 `InputQualityReport`（grade/mode/fallback/rtt_p50/p95_micros/hitches/samples[].rtt_ms/suggestions[].action）及 `desktop_protocol.rs` 的 links（control_session/motion_mode/motion_fallback_reason）逐一对齐；`quality` 缺省时清空面板的行为与 `skip_serializing_if = is_idle` 语义一致。状态：已实现待实机验证。
- 流畅度诊断服务端通路：`deskfuse-service` 状态 payload 携带 `quality` 字段，随既有 status 轮询到达客户端，无需额外 IPC。
- 剪贴板文字去重：`ClipboardHistory.Add` 按同来源指纹去重并保留自定义标题，语义与 macOS 0.5.1/0.5.2 去重一致（Windows 历史在客户端侧，不经过 engine `clipboard_library`，去重逻辑需各自维护，本次核对一致）。pasteboard 占位符丢弃属 Apple 私有机制，Windows 无对应物。
- 剪贴板筛选（文字/图片/文件 + 来源 + 搜索）：WinUI 客户端已具备，与 ca0b975 的 Mac 筛选能力对应。
- 资源：`Assets/DeskFuse.ico` 与 `icons-manifest.json` 已随合并更新。
- 剪贴板回环抑制：Windows 控制器以 ownership/generation/nativeRevision 令牌抑制本地回环，机制存在；与 macOS 0.4.2「仅抑制远端指纹组」的精确语义差异需在双设备验收中核对。

仍不对齐（已知缺口，维持原工作包归属）：

- 键鼠共享 Wi-Fi 绑定布局恢复（0.4.5 能力）：Windows 客户端与 MonitorLayout 均无 SSID 绑定，归 C1。
- 设备阻止（block）与忘记分离（0.5.0 能力）：设备详情仅有「撤销信任」，无阻止操作，归 C1。
- 主动触发单个/批量配对设备检查更新（0.5.0 能力）：设置页仅有手动说明文案，归 E。
- 建议展示细粒度：macOS 按 priority（act/info/ok）分级图标 + trigger/expected/retest 展开；Windows 仅平铺 action 文本，属展示层差距，随 C1 UI 验收一并处理。
- 内存诊断与诊断文件（0.5.2，engine/FFI 侧）：Windows 客户端暂无导出入口，归 E（日志和诊断导出）。
- 直接文件下载（ca0b975 的 Mac 网页列表分享/手机上传）：Windows 暂无，归 C2。
- HEIC/HEIF 编解码（0.5.0 工作流）：Windows 无解码路径，归 D1。

合并完整性验证：

- `dotnet build apps/deskfuse-windows/DeskFuse.Windows.csproj`：通过，0 警告 0 错误（冲突解决后；解决前 19 个错误全部消除）。
- `cargo check --locked -p deskfuse-service`：通过。
- `cargo check --locked -p deskfuse-platform-windows`：通过。
- workspace 全量 `cargo check --locked` 在 Windows 不可行：`deskfuse-engine` 对 `deskfuse-platform-macos` 的依赖未按 target 门控（main 既有结构，非本次合并引入），engine/FFI 只能在 macOS 侧 `./scripts/check.sh` 全量验证。
- 未启动桌面应用，未做双设备、视觉与安装验收。

## 全量交付门槛

- [ ] 所有 Mac 可见功能具备逐项对应记录或明确平台例外。
- [ ] 所有非例外项目已实现且经过 Windows 实机验证。
- [ ] Mac ↔ Windows 双设备场景及离线/并发/失败行为通过。
- [ ] 导航、视觉结构、多屏 DPI、键盘与辅助功能验收通过。
- [ ] 数据迁移、安装、升级、回滚和公开更新源分别验证。

## 本批验证

- `dotnet build apps/deskfuse-windows/DeskFuse.Windows.csproj -c Debug -p:Platform=x64 --no-restore`：通过，0 警告、0 错误。
- `git diff --check`：通过；工作区 LF/CRLF 提示不属于空白错误。
- 未启动桌面应用，未读写实际剪贴板，未进行视觉、服务、双设备、安装或升级验收。
