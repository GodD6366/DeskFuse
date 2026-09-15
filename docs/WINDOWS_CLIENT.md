# Windows 原生客户端实施状态

2026-09-13：新增 WPF 原生桌面和受限业务 IPC，**这是分阶段实现，不是 macOS 全功能对齐完成声明**。
运行与交付证据独立存放在 `artifacts/validation/2026-09-13-windows-client/`。

## 组成

- `apps/unidesk-windows/`：.NET 8 / WPF，普通用户 `asInvoker` 进程。原生窗口、单实例、托盘、当前用户剪贴板与本地历史、暂存区、设置。关闭主窗口不退出；托盘退出才结束客户端。
- `apps/unidesk-service/`：Rust，SCM LocalSystem 服务；仍然唯一持有身份、信任、QUIC 和 mDNS，不在 GUI 中启动备用网络或生成第二套身份。
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
| 本机改名 | 尚缺 runtime 权威改名/Hello/mDNS 更新 API，不假成功 |
| 文字、PNG、HTML、RTF 同步 | 已接入带 epoch / 分片 / 哈希 / 回执的 clipboard-sync-v2；当前通道合计最多 1 MiB，不是 macOS 的全容量支持 |
| 多格式/图片转换 | 映射已实现的通用格式；不承诺任意原生格式、多项目或所有图片编码均可互换 |
| 本机历史、搜索、类型/来源筛选、选择与恢复 | 当前用户本地持久化；选择只预览，明确复制才恢复；最多 200 条 / 64 MiB |
| 文件复制 | 仅记录本机现有文件引用，优先于伴随路径文本；不把源机器绝对路径发给对端 |
| 240×240 暂存区、展开、拖入和本地拖出 | 已接入本机引用及原生 Copy 拖放；移除引用不删除源文件；文件发送菜单明确不可用 |
| Explorer 按需虚拟文件/远端文件接收 | **未接入**；不能以本地 FileDrop 代替异步 OLE/完整文件传输 |
| 传输队列与缓存配额 | 页面显示未接入；不伪造任务、进度或远端完成 |
| 键鼠、多屏布局 | 展示本机真实显示器与缺失的 helper 原因；不宣称可跨屏控制 |
| 普通/管理员/UAC/锁屏/登录输入 | **未接入服务会话 helper 和控制协调器**，input capability 为 false |
| 自动更新、跨系统包分发、事务回滚 | **未接入 Windows 安装协调器**，updater capability 为 false |
| 工作流、快捷键与工作流共享 | **尚未对齐** |

## 剪贴板生命周期

1. 启动与解锁以当前剪贴板为基线，不回放旧历史或自动发送已有正文。
2. 本机剪贴板由桌面 STA 监听；读写冲突明确失败，后台历史保存不参与网络线程。
3. 服务绑定实际 SID、登录 LUID、物理 console session 和 epoch；锁屏、注销、服务停止、桌面心跳超时清空正文与未完成同步任务。
4. Poll 只是交付候选内容。桌面成功写入系统剪贴板后才 Commit，服务此时才向对端发送 APPLIED；失败或较新本机复制拒绝旧内容。
5. 文件项不会退化成路径文本跨设备同步。历史清理只删除应用历史索引中的引用，不删除源文件或用户粘贴目的地。
6. 不记录剪贴板正文到日志、诊断、服务状态或构建清单。

## 构建与交付

Windows MSVC x64 / .NET 8 / NSIS 3：

```powershell
.\scripts\snapshot-windows-source.ps1 -OutputPath "$env:TEMP\unidesk-source.json"
.\scripts\package-windows.ps1 -SourceSnapshot "$env:TEMP\unidesk-source.json"
```

脚本默认不运行测试、不安装服务、不启动应用。安装器实际执行时按安装说明注册服务和仅 Private / LocalSubnet 的程序端口规则。移除只处理本安装器的载荷和规则，保留身份、配对和用户数据。

没有 Authenticode 或发行签名时明确记录未签名。**该安装器不是 LAN 自动更新载荷**，不得绕过发行验证导入更新库存。安装失败回滚、Windows 安全桌面、Explorer 按需拖放、真实两机与视觉验收仍分别记录，不用跨编译成功替代。

## 剪贴板去回环修正（build 105）

Windows 自写入以剪贴板锁内的 HWND owner 与进程随机来源标记联合识别，不仅比较 sequence。WM、轮询及读后复核使用 Own/Foreign/Unavailable 三态，占用时不触发本机采集或发布；真实外部窗口即使保留标记也不视为本机自写。显式恢复仍可发布一次，远端应用不回传；本地文件恢复统一 CF_HDROP + COPY，远端入口仍拒绝文件引用。

历史指纹按 MIME/完整数据 canonical 排序，去掉重复 primary 表示；只折叠相同来源、全部格式一致的条目，保留富文本/图片差异。加载时相同规则整理完全重复记录，不清空历史，不修改其他设备已有记录。
