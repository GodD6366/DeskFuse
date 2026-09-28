# Windows / macOS 能力对齐开发计划与交接

> 目标：让 Windows 原生客户端在功能契约上尽量对齐 macOS。本文记录当前基线、剩余任务、实现边界和合并顺序。
>
> 本文是开发交接文档，不代表真实 Windows 双机、签名、公证或安全桌面验收已完成。

## 1. 当前基线

工作区：`/Users/godd/.worktrees/uniDesk/windows`
分支：`windows`

已完成或已有源码链路：

- WPF 原生客户端、托盘、单实例、设置、历史和暂存区
- 服务持有身份、信任、QUIC、mDNS 和桌面 IPC
- 文字、PNG、HTML、RTF 剪贴板同步及回执
- Deferred 文件传输、缓存、哈希校验和任务状态
- 普通桌面双向输入、租约、按键释放和紧急收回
- 多屏拓扑读取和只读布局画布
- RemoteApp 基础协议/目录接入
- 首次手动安装 NSIS 基础链路
- UIAccess manifest 基础接入（提交 `7ecfdda`）

尚未形成完整交付闭环：源码接入不等于 Windows 实机验收、签名、安装器升级或安全桌面可用。

## 2. 剩余能力清单

### P0：输入权限和安全边界

目标：管理员窗口可用，权限状态可解释，不能误宣称支持安全桌面。

任务：

1. helper 启动时读取自身 token、UIAccess、Integrity Level、PID 和所属 session。
2. 服务绑定 helper PID、用户 SID、登录 LUID、session epoch；PID 重用或 session 变化立即失效租约。
3. 重新探测当前 input desktop 和 thread desktop，区分：
   - 普通 Default desktop
   - UAC/高完整性桌面
   - Winlogon/登录桌面
   - 锁屏/非活动 session
4. 管理员窗口输入走 UIAccess helper；失败时返回明确 capability/reason。
5. 锁屏、登录、SAS 和安全桌面保持 `false`，除非另行设计 SYSTEM session agent 和明确用户授权策略。
6. 安装器必须把 UIAccess 二进制放在受信任目录，并要求 Authenticode 签名；开发构建不得静默降级成“看似支持”。

涉及：

- `apps/unidesk-service/src/native/input_helper.rs`
- `apps/unidesk-service/src/native/desktop.rs`
- `crates/unidesk-platform-windows/src/session.rs`
- `crates/unidesk-platform-windows/src/input.rs`
- `apps/unidesk-windows/InputHelperProcess.cs`
- `apps/unidesk-windows/app.manifest`
- `installer/windows/`

### P0：Explorer OLE 延迟文件

目标：远端文件可作为 Explorer 虚拟文件对象粘贴，避免应用层主动回放 Ctrl+V。

任务：

1. 在 WPF 进程内实现 `IDataObject`。
2. `EnumFormatEtc` / `QueryGetData` 只提供格式元数据，不主动下载。
3. 首次 `GetData(FileGroupDescriptorW)` 时请求 `RequestFileClipboard`，接收、校验并写入用户私有缓存。
4. 返回真实文件/目录描述符；`FileContents` 按 `lindex` 返回对应流。
5. 支持取消、超时、断线、重名和缓存清理。
6. deferred announcement 现在携带完整相对目录树：文件、目录、空目录和子树均有稳定的 `lindex`；旧版 names/total_bytes 二元组仍可解码，并合成顶层文件元数据。`FileGroupDescriptorW` 和 `FileContents` 使用同一 lindex，目录不提供 FileContents。
7. 预览程序触发 `FileGroupDescriptorW` 时可能导致下载，文档中必须明确这一行为；目录树元数据枚举本身不下载。

涉及：

- `apps/unidesk-windows/DesktopClipboardController.cs`
- `apps/unidesk-windows/NativeClipboard.cs`
- `apps/unidesk-service/src/native/file_transfer.rs`
- `apps/unidesk-service/src/desktop.rs`
- `crates/unidesk-transfer/`
- `apps/unidesk-service/Cargo.toml`

### P1：Windows 自动更新、签名和回滚

目标：对齐 macOS 的更新选择、下载、校验、安装和失败恢复。

任务：

1. Windows updater 读取 canonical GitHub Release，独立 feed 仅作回退。
2. 校验版本、构建号、SHA-256、签名清单和发行元数据。
3. 下载到用户/机器隔离缓存，不覆盖当前安装。
4. 安装协调器执行：准备、停止服务、备份、替换、启动、自检、确认。
5. 自检失败自动恢复备份，保留失败诊断元数据，不记录剪贴板正文或文件内容。
6. 服务升级和 GUI/helper 版本必须原子匹配。
7. NSIS 只负责首次安装；更新器不能绕过签名校验直接导入任意 ZIP。
8. capability 只有在完整链路可用时才置 `true`。

涉及：

- `crates/unidesk-update/`
- `crates/unidesk-engine/src/app_update.rs`
- `apps/unidesk-service/`
- `installer/windows/`
- `scripts/package-windows.ps1`
- `scripts/build-client-windows.ps1`

### P1：工作流和快捷键共享

目标：Windows 能读取、编辑、执行并同步 macOS 已有 workflow 模型。

任务：

1. 复用 `unidesk-core::workflow`、`unidesk-engine::workflow_catalog` 和共享协议，不在 WPF 重写模型。
2. 为桌面 IPC 增加 workflow 列表、保存、删除、规则绑定、运行、取消和冲突恢复命令。
3. 增加 Windows 快捷键注册/监听层，执行前复用 Rust workflow service 的冲突检查。
4. 对文件、文本、图片和转换步骤做 Windows capability 检查。
5. UI 展示运行状态、失败原因和设备冲突，不把未支持步骤静默执行。
6. 工作流同步采用现有版本/冲突模型，不能只同步本地 JSON。

涉及：

- `apps/unidesk-service/src/desktop_protocol.rs`
- `apps/unidesk-service/src/native/desktop.rs`
- `crates/unidesk-core/src/workflow.rs`
- `crates/unidesk-engine/src/workflow_catalog.rs`
- `crates/unidesk-engine/src/workflow_sharing.rs`
- `apps/unidesk-windows/`

### P1：多屏布局编辑

目标：从只读画布升级为可保存的布局配置，并与输入 runtime 的布局模型一致。

任务：

1. 使用 `DisplayTopology` / `DisplayLayoutProfile` 的现有模型，不另造坐标格式。
2. 画布支持拖拽显示器、负坐标、DPI 缩放和吸附。
3. 通过现有 `MoveControlDevice` 或扩展 IPC 保存布局；服务端负责权限、设备存在性和拓扑版本校验。
4. 区分本机布局、远端设备布局和控制路由布局。
5. 设备断线时保留配置但不伪造在线位置；重连后重新校验。
6. 保存失败必须回滚 UI 草稿，不覆盖已生效配置。

涉及：

- `apps/unidesk-windows/MainWindow.xaml`
- `apps/unidesk-windows/MainWindow.xaml.cs`
- `apps/unidesk-service/src/desktop_protocol.rs`
- `apps/unidesk-service/src/native/desktop.rs`
- `crates/unidesk-input/src/layout/`
- `crates/unidesk-core/src/sharing.rs`

### P1：RemoteApp 和安装器对齐

任务：

1. 对照 macOS RemoteApp 的 catalog、连接、视频、输入和断线状态，逐项确认 Windows hosting/client 能力。
2. Windows 端暴露真实安装应用目录，不接受任意路径执行。
3. RemoteApp 连接必须绑定设备信任、会话和窗口生命周期。
4. 断线时释放输入、关闭远端窗口和清理临时资源。
5. 安装器注册服务、GUI、helper、RemoteApp 组件时保持版本一致。
6. 卸载保留用户身份、配对和历史数据；仅删除本安装器拥有的文件和规则。
7. 安装失败回滚不能被误报为更新成功。

涉及：

- `apps/unidesk-service/src/native/remote_app.rs`
- `apps/unidesk-windows/`
- `apps/unidesk-macos/RemoteApplication/`
- `docs/remote-application/`
- `installer/windows/`

## 3. 推荐实现顺序

1. 先合并并审查各子 agent 的源码修改，解决 Cargo/IPC 基线问题。
2. P0 输入权限：先完成 helper 身份绑定和 capability，安全桌面保持明确不支持。
3. P0 OLE：完成 `IDataObject` 生命周期和文件传输回执。
4. P1 更新器：先做签名/回滚核心，再接 NSIS 安装流程。
5. P1 工作流 IPC 与 WPF UI。
6. P1 多屏布局编辑。
7. RemoteApp、安装器和版本能力矩阵收尾。
8. 最后统一更新 `WINDOWS_CLIENT.md`、`WINDOWS_DESKTOP_IPC.md`、`TODO.md` 和 README。

## 4. 合并规则

- 每个子任务只能修改自己的 worktree，并提交独立 commit。
- 合并顺序按 P0 → P1；协议/模型先于 UI。
- 冲突时优先保留 `windows` 分支已有的服务安全边界，不能通过删除拒绝逻辑来“对齐”。
- capability 必须由真实可用链路决定，不能因为 UI 已出现按钮就置 `true`。
- 不把 Windows CI、交叉编译或源码链路描述为实机验收。
- 不提交凭据、签名私钥、用户剪贴板正文、文件内容或本机配置。

## 5. 子 agent 当前分工

| 任务 | 状态 |
|---|---|
| secure-input | 处理中：UIAccess、token/desktop 探测、helper 绑定；安全桌面保持 false |
| updater | 处理中：Windows 更新、签名和事务回滚 |
| ole-files | 处理中：`IDataObject`、FileGroupDescriptorW、FileContents 延迟读取 |
| workflow-layout | 处理中：工作流/快捷键与布局编辑 |
| remoteapp-installer | 处理中：RemoteApp 和安装器生命周期 |

## 6. 平台边界收尾审计（2026-09）

- Windows v2 `Rejected.code` 是仍在使用的兼容字段，不能删除。服务现在同时
  输出 `boundary_error` typed `BoundaryErrorCode`；旧 WPF 客户端忽略新增字段，
  新客户端应优先使用 typed code。
- 平台错误统一入口是 `BoundaryErrorCode`，能力名称统一入口是
  `PlatformCapability`。未知 legacy code 仍 fail-closed 为 `native`，不能按
  成功处理。`GuardedPaste` 已纳入 macOS workflow capability 声明。
- `unidesk-platform` 不引入 AppKit/Win32；其中的 filesystem-backed memory
  ports 是当前 host assembly 的过渡实现。`unidesk-engine` 的旧 update/LAN/file
  worker 与 `unidesk-transport` 的 macOS 网络探测仍是未迁移项，不要在没有
  host adapter 和实机回归前移动或删除。
- `PlatformCommand`/`PlatformEvent` 已接入 Windows 的 authenticated host payload
  和 typed receipt；`FileDataObjectPort` 由 WPF `DesktopClipboardController` 实现，
  但 Rust collaboration 尚无 file-object consumer。`WorkflowExecutionPort` 由
  engine 注入真实 macOS adapter，`WorkflowOutputPort` 已由 macOS `host_ports`
  注入 `MacOSWorkflowOutputPort`，但 extracted workflow service 尚未消费它。
  这些未使用路径不能被字符串 IPC 的局部替换冒充完成，也不能因为暂未消费而
  删除；接通时必须同时更新 native host、协议消费者和 capability 矩阵。

交接检查清单：

- [ ] 列出保留的 legacy 命令/字段及兼容客户端。
- [ ] 为每个 capability 标明真实 adapter、权限和 session 来源。
- [ ] 为每个未迁移 OS API 标明 owner 和删除前置条件。
- [ ] 只在 typed event 消费者上线后删除字符串投影。
- [ ] 执行 `git diff --check`；本轮按计划不运行测试。

## 7. 明确不应直接承诺的能力

以下能力除非新增专用 SYSTEM session agent、明确授权和完整安全设计，否则不得标记为已支持：

- Winlogon 登录桌面输入
- 锁屏桌面输入
- UAC 安全桌面上的任意控制
- SAS（Ctrl+Alt+Del）注入或拦截
- 绕过 Windows UIPI、凭据保护或安全策略

这些能力可以作为后续独立项目，不应通过普通 UIAccess helper 偷渡实现。
