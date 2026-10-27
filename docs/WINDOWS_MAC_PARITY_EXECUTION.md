# Windows / macOS 产品能力对齐执行清单

## 当前交付范围：三大基础功能

以当前 macOS 源码为基准，优先严格对齐 **剪贴板、鼠标跨屏共享、分享与文件传输** 的操作、默认值、数据语义和失败行为。保留已有功能；远程应用、麦克风共享、鼠标自定义在 Windows 标记「即将支持」，不作为本阶段完成条件。下文旧批次记录保留作历史，不代表当前工作区的实现或验收状态。

### 当前核查与本批修复

| 基础功能 | 本批已落实（代码验证，不代表双机验收） | 尚未完成 / 待核对 |
| --- | --- | --- |
| 剪贴板 | Windows canonical 查询与数量统计共用日期窗口；按本地日历计算今天、昨天、最近 N 天和闭开日期范围，覆盖 23/25 小时夏令时日期 | canonical 迁移、列表/索引/正文同步、删除冲突、应用排除、快捷面板粘贴需逐项双机验收；原生格式与文件归档不可仅由页面存在判定完成 |
| 鼠标跨屏共享 | 按当前 Mac 页面收口为主控开关、Wi-Fi、添加设备和排布画布；移除旧右上角总开关及参与者/内部编码/布局配置区。修复开关启用后仍半透明和提交后旧快照回弹；接入本机配置壁纸与远端认证 JPEG 预览；视口容纳全部显示器，读取已保存坐标 | 设置入口仍需按当前 Mac 独立设置页继续核对；Windows 壁纸向 Mac 发布尚未接入；UIAccess helper 安装/授权、两种主控方向、负坐标/旋转/混合 DPI、休眠/热插拔仍需实机验收 |
| 分享与文件传输 | 增加文件夹选择入口；发送只移除本次提交的文件，保留等待期间新加入的项目；断线清除旧目标，发送按钮受真实连接和文件开关约束；提交提示不再冒充对端已接收 | Windows LAN 当前仅文字/图片上传，尚无 Mac 的网页列表与直接下载；缓存设置/清理、接收目录配置、失败重试与 Explorer OLE 按需传输仍需补齐或核对；目录/空目录/重名/源变化和取消需双机验证 |

### 本批实际检查

- `dotnet build apps/deskfuse-windows/DeskFuse.Windows.csproj -c Release --no-restore`：通过，0 警告、0 错误。
- `dotnet run --project tools/DeskFuse.HistoryChecks/DeskFuse.HistoryChecks.csproj --no-restore`：11 项通过；新增日历夏令时、日期范围和数量/页面请求一致性检查。测试使用临时目录，不访问系统剪贴板或用户历史。
- `git diff --check`：通过；已有 LF/CRLF 提示不是空白错误。
- 本批没有修改 Rust、发行元数据或安装器；没有将现有未提交 Rust 改动算作本批验证通过。
- 本机 SCM 的 `DeskFuseService` 为 Running；这不证明本次构建已部署、helper 已获授权或 Mac 已连接。未进行本批 WinUI 交互及真实 Mac ↔ Windows 两机验收。

### 后续完成条件

1. 按上述剩余项继续补齐三大功能，不以导航或按钮存在代替实现。
2. 每个操作核对服务回执与最终状态，拒绝/超时/断线保留草稿或恢复权威状态；不得显示虚假的成功。
3. 同源 Rust 行为、Windows 原生适配、UI 和双设备分别记录检查证据。只有双方向、离线恢复、失败处理都通过后，才标记「严格对齐完成」。

## 截图反馈修正：以当前 Mac 页面而非历史能力清单为准

前一批把内部快捷键编码、布局保存等配置堆在鼠标共享主页面，不符合当前 `InputView.swift`，已移除，不再将此视为对齐成果。角色开关独立于旧功能总开关，启用主控时一次提交必要字段；正在被控时仍不能切换主控，不降低输入授权。

- 只读 UI Automation 检查发现，用户当前安装版两个可见 ToggleButton 都是 `Enabled=True, State=On`，不是系统把主控按钮禁用了。`DfToggle` 初始禁用时设为 0.45 透明度，却未在 IsEnabled 变化时刷新；现已修复。命令入队后读取旧快照会把切换外观改回去，现保留待确认状态，确认/失败/超时再更新。
- 本机壁纸使用 `IDesktopWallpaper` 按显示器读取配置文件；不捕获桌面、图标或应用窗口。远端壁纸透传现有输入运行时已认证、限额的 JPEG，客户端限制字节数与解码尺寸，旧服务缺字段时兼容回退。没有图片时显示中性背景，不伪造壁纸。
- 画布去掉中央设备路径、数字与大色块，用壁纸和底部设备名展示；全体显示器共同计算视口，读取浮点布局坐标，避免把远端排到可见区外。真实混合 DPI 行为仍待双机验证。
- 当前运行程序仍是 `C:\Program Files\DeskFuse\DeskFuse.Windows.exe`；本轮未替换运行程序或服务。检查时未发现活跃的 input helper，安装目录的 helper Authenticode 状态为 `NotSigned`；旧调试日志不是当前进程启动结果，不据此宣称已确定全部运行失败原因。UI 不再同时报告输入未就绪和可以跨屏，也不再将该状态显示为「一切正常」。
- 验证通过：`dotnet build apps/deskfuse-windows/DeskFuse.Windows.csproj -c Release --no-restore`（0 警告/错误）、MSVC 环境中的 `cargo check --locked -p deskfuse-service`、`cargo test --locked -p deskfuse-service --lib input_state_carries_layout_and_control_views`（含壁纸字段往返与旧快照兼容）、`git diff --check`。
- `./scripts/check.sh` 补齐 Git Bash 的 Rust PATH 后，`cargo fmt` 通过，首个失败阶段为 workspace `cargo check`：`core-foundation` 在 Windows 引用了 `std::os::unix`，随后全量测试/Clippy 未执行。原因为工作区 Apple 专属依赖；不等同于本次 Windows 服务检查失败。
- 尚未验收：本次 UI 的实际渲染、远端壁纸到达、Windows → Mac 壁纸发布、已安装版本升级、真实鼠标共享。编译成功不代表这些已完成。

## 剪贴板、设备选择与暂存区修复（当前批）

- **设备选择**：原弹窗过滤掉已加入的参与者，因此 `godd-MBP-108` 虽在线却不在候选列表，其他设备又默认全选。现在按 Mac 的 `InputDevicePicker.swift` 显示全部已配对设备，逐台添加/移除，明确区分「已添加」「本机输入未就绪」「等待对端屏幕」「离线」；提交后等待实际快照确认。
- **内容库路径根因**：`user_library_dir` 去掉 UTF-16 NUL 后仍调用 `ExpandEnvironmentStringsW`，越界读取导致用户目录后出现随机字符、服务创建多个孤立库。现保留终止符、校验输出长度/绝对路径；新增带哨兵字符的回归测试。新库固定在 `%LOCALAPPDATA%\DeskFuse\state.sqlite3`；迁移标记升为 `canonical-migration-v2-profile-path.done`，让旧本机历史重新迁入正确库。错误目录保留，不自动删除或宣称全部孤立库已合并。
- **实时剪贴板协议**：Windows 原先手写 HELLO/DATA，漏掉 Mac 当前要求的来源策略版本。现统一使用共享 `SyncMessage::hello/data`，协商接收上限，校验分块来源元数据，保留来源 App/设备/日期以及 `no_history`；被排除的内容仍可写系统剪贴板但不保存历史。显式历史发送缺少可靠来源时保守不新增历史，不伪造来源。
- **列表刷新**：连接事件只在重连时发送；快照携带 `library_revision`，没有数据变化时不重查。初次实际安装复测仍发现大量远端索引到达时整表重绑，因此进一步改成 `StableCollection` 的逐条 Insert/Move/Replace/Remove；分组对象、未变化行、选中项和预览保持，不发 Reset、不重建 ItemsSource。缩略图改为顺序请求，避免堵塞心跳与同步请求。
- **暂存区重做**：以 `ShelfController.swift` 为基准，240×240 DIP 无边框置顶浮窗、480×420 DIP 四列展开、真实文件图标、折叠文件堆、逐台发送、清空关闭；仅存文件/文件夹引用，不把它变成另一套剪贴板历史。拖入/拖出使用异步 deferral，目录用 StorageFolder，成功拖出才移除引用，拖回自身不丢文件。关闭发送中的暂存区会取消对应入站/出站任务；原始文件不删除。
- **剪贴板快捷键**：设置 → 快捷键 → 剪贴板面板，支持录制、启停、恢复默认与保存。默认 Ctrl+Shift+V，只存本机配置。新组合成功注册并持久化后才释放旧组合；冲突/保存失败保持旧键。固定暂存区 Ctrl+Shift+D 和工作流组合不可被覆盖。

检查：Windows 服务 42 项测试通过（包含来源策略/路径修复），文件发送取消回归通过，历史/增量集合/快捷键/未知复制时间/日期查询 14 项检查通过；Windows Release 构建通过。完整工作区检查通过 fmt 后仍在 Windows 编译 Apple 专属 core-foundation/objc2 阶段失败，不代表 Windows 定向测试失败。

首次替换后已直接验证：正确路径数据库存在，接收到 Mac 索引（一次观察为 4661 个 remote 索引行、9684 个同步操作）；这只是当时的同步进度，不代表全部正文已下载。

最终 GUI 已替换到安装目录：514 个文件哈希一致，四个 native EXE（service/input-helper/service-client/update-helper）哈希前后一致、SCM 未重启，保留权限线程已签名并修复的 helper。备份 `C:\Program Files\DeskFuse-ui-backup-20261027-000658`。

运行版检查：设置 → 快捷键的剪贴板录制/启停/保存入口已实际打开；暂存区实际窗口为 600×600 和 1200×1050 物理像素（250% DPI，对应 240×240 和 480×420 DIP）。最终列表检查先等待布局完成，再对可见行的显示内容作本地哈希匹配（不输出正文）：跨 2.4 秒心跳，8 行匹配、8 个原 UI Automation 容器保留。不要把导航后的首帧、虚拟化的离屏行或真实新增记录当成整表重置。未知复制时间的 canonical 行不再投影为 DateTimeOffset.UtcNow，使用稳定排序时间但显示「复制时间未知」。

未完成的独立验收：暂存区真实跨设备发送/拖出到不同目标应用、全量图片正文下载、两机键鼠控制、PixPin 与其他剪贴板工具并行行为；没有通过替换 GUI 来声称这些已通过。

### Windows UI 开发模式

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\dev-windows.ps1
# 只生成调试客户端，不启动：
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\dev-windows.ps1 -BuildOnly
```

发布 Debug 客户端到 `dist\windows-dev`，复用已安装的 SCM 服务及本机数据；不安装服务，不生成另一套设备身份，不复制或绕过输入 helper 授权。重新运行脚本会重建并重启 UI，不是热重载。客户端与安装版互斥；服务协议改动仍需更新匹配服务。`-BuildOnly` 已实际通过。

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

仍不对齐（已知缺口；标注「已补全」的条目见第三批）：

- 键鼠共享 Wi-Fi 绑定布局恢复（0.4.5 能力）：Windows 客户端与 MonitorLayout 均无 SSID 绑定，归 C1。（已补全）
- 设备阻止（block）与忘记分离（0.5.0 能力）：设备详情仅有「撤销信任」，无阻止操作，归 C1。（已补全）
- 主动触发单个/批量配对设备检查更新（0.5.0 能力）：设置页仅有手动说明文案，归 E。客户端自身更新状态检查已补全；「触发对端设备更新」需要 engine github_update 级别的服务侧更新模块，仍归 E。
- 建议展示细粒度：macOS 按 priority（act/info/ok）分级图标 + trigger/expected/retest 展开；Windows 已补全优先级标记（act ● / info ○），trigger/expected/retest 展开随 C1 UI 验收一并处理。
- 内存诊断与诊断文件（0.5.2，engine/FFI 侧）：Windows 客户端暂无导出入口，归 E（日志和诊断导出）。（客户端侧诊断导出已补全；服务进程内存采集仍归 E）
- 直接文件下载（ca0b975 的 Mac 网页列表分享/手机上传）：Windows 暂无，归 C2。
- HEIC/HEIF 编解码（0.5.0 工作流）：Windows 无解码路径，归 D1。

合并完整性验证：

- `dotnet build apps/deskfuse-windows/DeskFuse.Windows.csproj`：通过，0 警告 0 错误（冲突解决后；解决前 19 个错误全部消除）。
- `cargo check --locked -p deskfuse-service`：通过。
- `cargo check --locked -p deskfuse-platform-windows`：通过。
- workspace 全量 `cargo check --locked` 在 Windows 不可行：`deskfuse-engine` 对 `deskfuse-platform-macos` 的依赖未按 target 门控（main 既有结构，非本次合并引入），engine/FFI 只能在 macOS 侧 `./scripts/check.sh` 全量验证。
- 未启动桌面应用，未做双设备、视觉与安装验收。

## 第三批：缺口补全（设备信任、Wi-Fi 布局绑定、更新检查、诊断导出）

已实现的缺口补全（编译与协议测试通过，实机待验收）：

- 设备阻止/忘记分离（C1，对齐 0.5.0）：桌面协议新增 `Forget`（→ `ForgetPeer`，从列表移除）与 `AllowPairing`（→ `AllowPairing`，解除阻止）；`Revoke` 保持 macOS `Block` 语义（阻止并保留在列表）。客户端设备页按 已配对/附近设备/已阻止 分组排序，阻止与忘记均先确认；已阻止设备提供「解除阻止」。旧版本服务会拒绝新命令，客户端在失败文案中注明需要匹配版本的服务。
- 布局绑定 Wi-Fi（C1，对齐 0.4.5）：服务新增 `native/wifi.rs`，通过 wlanapi 每 5 秒采样当前 WLAN SSID（与 engine CoreWLAN 轮询同周期）并发送 `ObserveWifiEnvironment`；桌面协议新增 `SaveEnvironmentLayout`（绑定/刷新当前网络布局）、`SaveScene`/`ApplyScene`/`RenameScene`/`DeleteScene`（命名布局）；`InputState` 新增 `environment`（ssid/conflict/message）与 `scenes` 视图。键鼠共享页新增「布局与 Wi-Fi」区块：显示当前 Wi-Fi 与绑定状态、保存布局到当前 Wi-Fi、场景列表的应用/删除。
- 更新状态检查（E）：设置 → 更新新增「检查更新」，与 engine 同源同优先级——GitHub Releases API 为 canonical，失败或空列表才回退 `deskfuse.godd.cc` 镜像；只认完整元数据资产（release.json/build-info.json/SHA256SUMS）的 `vX.Y.Z` 标签。只报告状态不下载安装（Windows 安装器尚未发行）。
- 诊断导出（E，对齐 0.5.2 概念）：设置 → 通用新增「导出诊断文件」，输出带时间戳的 JSON（服务状态快照、客户端内存计数、历史统计），明确排除剪贴板正文与信任材料，导出后在资源管理器中定位。
- 流畅度建议分级展示：按 priority（act ● / info ○）标记，与 macOS 面板同级。

本批验证：

- `dotnet build apps/deskfuse-windows/DeskFuse.Windows.csproj`：通过，0 警告 0 错误。
- `cargo check --locked -p deskfuse-service`：通过；`cargo clippy` 警告数与基线一致，无新增。
- `cargo test --locked -p deskfuse-service --lib`：28 通过；`windows_store_rejects_reserved_and_oversize_paths` 在本机失败为既有环境问题（改动前的 HEAD 同样失败，与本次无关）。
- 新增命令往返序列化测试（含 C# 侧命令名一致性约束）通过。
- 未进行实机验收：Wi-Fi 绑定恢复、阻止/解除阻止对端行为、更新检查真实网络、双设备场景均待实机。

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
