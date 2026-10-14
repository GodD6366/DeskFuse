# Windows 原生客户端实施状态

2026-09-13：新增 WPF 原生桌面和受限业务 IPC，**这是分阶段实现，不是 macOS 全功能对齐完成声明**。
运行与交付证据独立存放在 `artifacts/validation/2026-09-13-windows-client/`。

## 组成

- `apps/deskfuse-windows/`：.NET 8 / WPF，普通用户 `asInvoker` 进程。原生窗口、单实例、托盘、当前用户剪贴板与本地历史、暂存区、设置。关闭主窗口不退出；托盘退出才结束客户端。
- `apps/deskfuse-service/`：Rust，SCM LocalSystem 服务；仍然唯一持有身份、信任、QUIC 和 mDNS，不在 GUI 中启动备用网络或生成第二套身份。独立 input-only runtime 使用 `_deskfuse-input._udp` / UDP 42425；`deskfuse-input-helper.exe` 是普通用户桌面的固定注入进程，不持有身份或网络运行时。
- `docs/WINDOWS_DESKTOP_IPC.md`：桌面 v2 pipe 的精确 JSON 契约。旧 v1 Status 工具保留。
- `installer/windows/` 与 `scripts/package-windows.ps1`：首次手动安装的离线 EXE；固定 Program Files 安装，注册服务；普通用户 GUI 不由提权安装器直接启动。

## 实施矩阵

“已接入”描述源码链路，不能替代 Windows 原生运行或两机验收。

| macOS 功能 | Windows 当前实现 |
| --- | --- |
| 主窗口、四功能导航、独立设置、托盘 | 已接入原生 WPF；主窗口 1200×780、最小 960×620；空态为真实状态 |
| 自动发现、设备详情、连接候选 | 已接入服务 runtime snapshot；不把候选地址当活动连接 |
| 手动 IPv4 定位、配对邀请及接收方同意 | 已接入现有 Rust runtime 命令与邀请状态；入队不等于配对完成 |
| 断开、撤销信任 | 已接入服务命令；通过后续快照观察结果 |
| 本机改名 | 与 macOS 对齐：显示 Windows 计算机主机名（`GetComputerNameExW`），不提供手工改名；`Rename` IPC 命令仍明确拒绝 |
| 文字、PNG、HTML、RTF 同步 | 已接入带 epoch / 分片 / 哈希 / 回执的 clipboard-sync-v2；当前通道合计最多 16 MiB |
| 跨设备文件按需传输 | **已接入源码链路**：本机 CF_HDROP 复制或暂存区发送 → 服务端 Deferred 元数据报价（每对端独立传输 ID、10 分钟有效期）；对端粘贴请求后服务端重验源文件、流式传 1 MiB 块并等待对端 Ack。接收方向：远端 Deferred 报价在“传输”页展示，用户选择目标文件夹后按需下载，服务端在保护目录内分块暂存、逐文件 SHA-256 校验后提交，桌面把文件复制到目标位置并放入剪贴板后 CommitFileClipboard 才回 Ack。与 macOS 引擎使用同一帧协议（环回状态机测试覆盖两端）。Explorer OLE 延迟渲染已接入：完整 deferred manifest（目录、子树、空目录与稳定 lindex）先枚举，首次粘贴请求时才下载并校验内容 |
| 多格式/图片转换 | 映射已实现的通用格式；不承诺任意原生格式、多项目或所有图片编码均可互换 |
| 本机历史、搜索、类型/来源筛选、选择与恢复 | 当前用户本地持久化；选择只预览，明确复制才恢复；默认按 30 天保留并后台清理，不设条数或累计大小上限；远端接收的文件落地后进入本机历史 |
| 文件复制 | 记录本机现有文件引用并按需跨设备传输；不把源机器绝对路径发给对端 |
| 240×240 暂存区、展开、拖入和本地拖出 | 已接入本机引用及原生 Copy 拖放；移除引用不删除源文件；文件发送菜单已接入跨设备文件协议 |
| 传输队列与缓存配额 | 传输页显示真实任务（方向、进度、状态、失败原因）与服务缓存用量（默认 1 GiB/任务上限）；服务端逐任务暂存目录在终态清理 |
| Explorer 按需虚拟文件/OLE 延迟渲染 | **已接入源码链路**；完整目录 manifest 先枚举，`FileGroupDescriptorW`/`FileContents` 使用稳定 lindex，首次数据请求才下载并校验 |
| 键鼠、多屏布局 | Windows 发布稳定显示器 ID、DPI、旋转和负坐标拓扑；双向均已接入：Mac 主控 → Windows 被控，以及 Windows 主控 → Mac 被控（设备页一键开启/停止，服务持久化角色与参与者，激活状态与控制会话实时显示；布局画布展示本机+远端显示器，运行时负责实际吸附定位） |
| 普通桌面输入 | 已接入独立 42425 input-only runtime、会话绑定 helper、1.5 秒原生租约、按键/按钮释放与 Ctrl+Alt+Win+Esc 紧急收回；双向支持：Mac 主控 → Windows 被控，以及 Windows 主控（本地低级钩子采集、跨平台 SharingController 边缘逻辑、会话/租约/去重回执）→ Mac 被控（对端仍需设备激活确认） |
| 管理员窗口输入 | 已接入签名 UIAccess helper；服务按 kernel PID、进程创建时间、路径、会话 SID/LUID 与 TokenUIAccess 实际验证后才报告 `elevated_input`，仅 Default desktop 可用 |
| UAC/锁屏/登录输入、安全桌面、SAS | **未接入**；协议 capability 明确为 false，helper 对非 Default desktop fail-closed |
| 自动更新、跨系统包分发、事务回滚 | 机器 helper 已具备 UAC/SCM admission、GUI/input-helper 停止协调、SQLite/WAL/SHM 备份恢复及启动自检证据链；当前发布物仍是仅首次安装的 NSIS，缺少签名事务载荷/activation contract，因此 replacement fail-closed，updater capability 为 false |
| 工作流、快捷键与工作流共享 | **尚未对齐** |

## 剪贴板生命周期

1. 启动与解锁以当前剪贴板为基线，不回放旧历史或自动发送已有正文。
2. 本机剪贴板由桌面 STA 监听；读写冲突明确失败，后台历史保存不参与网络线程。
3. 服务绑定实际 SID、登录 LUID、物理 console session 和 epoch；锁屏、注销、服务停止、桌面心跳超时清空正文与未完成同步任务。
4. Poll 只是交付候选内容。桌面成功写入系统剪贴板后才 Commit，服务此时才向对端发送 APPLIED；失败或较新本机复制拒绝旧内容。
5. 文件项不会退化成路径文本跨设备同步；文件内容通过独立的按需帧协议传输，仅当接收端逐文件校验并确认落地后才向对端回 Ack。历史清理只删除应用历史索引中的引用，不删除源文件、接收目标文件或用户粘贴目的地。
6. 不记录剪贴板正文到日志、诊断、服务状态或构建清单。

## 构建与交付

Windows MSVC x64 / .NET 8 / NSIS 3：

```powershell
.\scripts\snapshot-windows-source.ps1 -OutputPath "$env:TEMP\deskfuse-source.json"
.\scripts\package-windows.ps1 -SourceSnapshot "$env:TEMP\deskfuse-source.json"
```

脚本默认不运行测试、不安装服务、不启动应用。安装器实际执行时按安装说明注册服务和仅 Private / LocalSubnet 的程序端口规则。移除只处理本安装器的载荷和规则，保留身份、配对和用户数据。

没有 Authenticode 或发行签名时明确记录未签名。**该安装器不是 LAN 自动更新载荷**，不得绕过发行验证导入更新库存。机器 helper 已接线 UAC/SCM admission、GUI/service/input-helper 协调、SQLite/WAL/SHM 备份恢复和启动自检证据；当前 NSIS 仍只允许首次安装，缺少签名事务载荷/activation contract 时 helper 在 replacement 前拒绝执行，updater 继续为 false。Windows 安全桌面、Windows 作为主控、Explorer 按需拖放、真实两机与视觉验收仍分别记录，不用跨编译成功替代。

## 剪贴板去回环修正（build 105）

Windows 自写入以剪贴板锁内的 HWND owner 与进程随机来源标记联合识别，不仅比较 sequence。WM、轮询及读后复核使用 Own/Foreign/Unavailable 三态，占用时不触发本机采集或发布；真实外部窗口即使保留标记也不视为本机自写。显式恢复仍可发布一次，远端应用不回传；本地文件恢复统一 CF_HDROP + COPY，远端入口仍拒绝文件引用。

历史指纹按 MIME/完整数据 canonical 排序，去掉重复 primary 表示；只折叠相同来源、全部格式一致的条目，保留富文本/图片差异。加载时相同规则整理完全重复记录，不清空历史，不修改其他设备已有记录。
