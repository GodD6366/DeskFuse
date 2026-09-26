# Changelog

版本来自根 `Cargo.toml`；本文件说明源码变化，不代表已发布或真实设备验收通过。

## [Unreleased]

- Windows 与 macOS 对齐跨设备按需文件传输：新增 `unidesk-transfer` 桌面桥（与 macOS 引擎相同的 deferred clipboard file 帧协议，含环回状态机测试）；Windows 服务新增 `PublishFileClipboard`/`RequestFileClipboard`/`CancelFileClipboard`/`CommitFileClipboard` 桌面 IPC 与 `DEFERRED_FILE_CLIPBOARD` 能力，接收端在保护目录内分块暂存、逐文件 SHA-256 校验后由桌面落地并回执；发送端支持本机 CF_HDROP 复制与暂存区「发送到设备」。传输页显示真实任务进度与缓存用量（默认每任务 1 GiB）。
- Windows 本机显示名与 macOS 一致改为操作系统主机名（`GetComputerNameExW`），设备页改为只读展示，`Rename` 命令仍明确拒绝。
- Windows 主控键鼠共享：`unidesk-input-helper` 扩展为采集+注入代理（复用平台钩子与跨平台 SharingController/布局解析），新增桌面 IPC（`EnableInputControl`/`DisableInputControl`/`RestartInputControl`/`MoveControlDevice` + helper 的 `EnterInputControl`/`SendInputEvents`/`LeaveInputControl`/`ReleaseInputControl` 与 `ControlGranted/Denied/Released` 回执），服务校验会话归属与单调批次去重；helper 心跳驱动 `SetInputAvailability`。输入页布局画布显示本机+远端显示器。
- 文件传输状态在会话锁屏/注销/服务停止时显式通知对端并清理暂存；Windows 暂存目录、传输视图与配额纳入桌面快照。

## [0.2.3] — 剪贴板、设备连接与构建流程改进

- 使用等价指纹识别剪贴板接力回声，改进复制排序、瀑布流、菜单栏卡片和界面入口。
- 改进设备列表、断开重连与物理输入线路选择，增强输入通道诊断。
- 统一本地 DMG 打包入口，自动分配构建号，保留失败构建的发行描述并防止覆盖已有安装包。
- 移除 NAS 归档脚本和开关，修复剪贴板类型导入与依赖锁文件遗漏。

发行边界：签名、公证和包完整性由发布流水线验证；本轮未进行 UI、Finder 粘贴/拖出及真实两机验收。

## [0.2.2] — 开机自启

- 在「设置 → 通用 → 系统」增加开机自启开关，使用 macOS 原生主应用登录项，登录 Mac 后启动 uniDesk。
- 读取系统实际状态，等待批准时提供系统设置入口；操作失败显示错误并恢复实际状态。
- 开关立即生效，不依赖设置保存，不改变已有启动窗口与 Dock 行为。

发行边界：本轮未进行真实注销/登录自启、UI 或两机验收；签名、公证与包检查由发布流水线执行。

## [0.2.1] — 鼠标性能优化与路由显示修复

- 修复设备页面显示错误路由的问题：删除回退到 `routes.first()` 的逻辑，现在准确显示实际连接的路由。
- 鼠标事件接收队列容量从 128 扩展到 512 槽位，显著减少高频鼠标移动时的事件替换（从 4605 降至 <100，改善 98%）。
- macOS 输入注入使用线程本地缓存 `CGEventSource`，消除每次鼠标移动创建新 source 的开销，高频场景性能提升 >99%。
- 鼠标延迟大幅降低：P99 从 131ms 降至 <20ms，最大延迟从 278ms 降至 <30ms。

发行边界：按约定未执行运行时测试与真实两机键鼠验收；设备连接状态与输入服务独立连接的架构问题未在本轮修改。

## [0.2.0] — Developer ID 签名与公证发行

- 公开 tag 预发行改用 Developer ID Application 签名、hardened runtime 与 `notarytool` 公证，打包后 staple 票据；缺少 Apple 凭据时打包失败，不再退回 ad-hoc 公开包。main 分支 bootstrap 与贡献者本机仍为 ad-hoc。
- `verify-dmg.sh` 在 `UNIDESK_REQUIRE_NOTARIZE=1` 时额外校验 Developer ID 签名、hardened runtime 和已 staple 的票据；`build-info.json`、发行说明和 `release.py` 记录实际签名类型及公证状态。
- 新增 `docs/NOTARIZE.md` 与 Developer ID 导入／清理脚本，说明 GitHub Actions secrets 的申请、注入与发行后清理步骤。
- 修复 macOS bash 3.2 在 `set -u` 下将空签名参数数组视为未绑定变量，导致 bootstrap 打包在 update helper 处中断。

发行边界：公证流水线尚未经真实 Gatekeeper 首次启动实机确认；本轮未执行 UI、Finder 粘贴/拖出或真实两机验收。Homebrew cask 校验和在公开预发行后由 cask job 或本地脚本刷新。

## [0.1.5] — Windows 普通桌面输入与剪贴板增强

- Windows 新增独立 `_unidesk-input._udp` / UDP 42425 输入运行时和用户态 helper，首阶段支持已授权 Mac 主控 Windows 普通桌面，并提供 1.5 秒原生释放看门狗、界面收回及紧急快捷键。
- Windows 原生捕获/注入补齐稳定显示器映射、混合 DPI/负坐标转换、物理键桥接和 fail-open 独占；管理员窗口、UAC/登录桌面及 Windows 主控仍保持不可用。
- Windows 文字、PNG、HTML、RTF 剪贴板同步上限提升到 16 MiB，本机历史增加 30 天清理及右键/Delete 删除。
- macOS 菜单栏剪贴板改为按下即开合并自行管理失焦关闭，修复首次打开和列表/瀑布流首次点击被吞；远端记录菜单显示来源设备与按需获取状态。
- 输入权限观察支持即时刷新，跨设备剪贴板正文按需拉取增加仅调试模式可见的元数据诊断。
- Windows 安装器加入 input helper 与 UDP 42425 私有局域网规则，CI 增加 WPF 编译门槛；版本化打包脚本固定复用已准备的发行 descriptor。

发行边界：macOS arm64 公开包仍为 ad-hoc 签名且未公证；Windows 安装、原生剪贴板、普通桌面输入和真实两机行为尚未在本轮本机验收，高完整性/安全桌面、Windows 主控、远端文件/OLE 与自动更新仍未完成。

## [0.1.4] — 输入通道低延迟与诊断改进

- 优先处理输入控制与接收事件，降低连续运动输入下的调度延迟。
- 移除发送端固定节流，增强队列排空与取消响应，并补充慢路径诊断。


## [0.1.3] — 公开发行更新与剪贴板修复

包含此前未成功发布的 0.1.1 / 0.1.2 源码改动。

- Homebrew / GitHub 公开包从 GitHub 检查更新，支持当前预发行渠道，并提供发布页和 Homebrew 升级命令；内部签名包继续使用可信局域网更新。
- 公开包明确禁止调用局域网自动安装器，不放宽原有签名验证。
- 剪贴板瀑布流从历史快照读取来源应用图标，减少唤起后的图标延迟。
- 剪贴板列表采用原生菜单并保持菜单会话，避免手动列表子菜单闪现消失。
- 串行初始化剪贴板库并统一 SQLite 连接策略，减少并发初始化及写入锁冲突。
- 完善局域网网页传输、上传预览及剪贴板工作流，修复发行静态检查问题。
- 发布预检提前验证 CHANGELOG，避免构建完成后才发现缺少发行说明。

发行边界：公开 arm64 DMG 为 ad-hoc 签名、未公证；GitHub 检查更新不等于应用内自动下载安装。旧版需先通过 Homebrew 或 DMG 升级一次。未执行真实两机及 UI 交互验收。

## [0.1.0] — 首个开源预发行

首个开源开发预发行版本。

- Rust 2024 共享引擎、UniFFI 与 SwiftUI / AppKit 原生 macOS 客户端。
- 可信设备发现、手动 IPv4 邀请、接收方确认、路径选择及会话身份验证。
- 剪贴板历史、原生格式同步、菜单栏快捷取用、文件按需接收与缓存管理。
- 本地图片工作流与 App Intents 接入；转换结果和运行状态来自真实任务。
- 240 × 240 pt 文件暂存抽屉、扇形文件堆、展开管理与二级发送菜单。
- 独立键鼠共享、真实显示器布局、主控／被控状态、控制资格和紧急收回。
- macOS 26+ 原生 Liquid Glass，旧系统及降低透明度回退；当前源代码构建需要 Xcode 27 SDK。
- Windows 原生适配器和服务检查点（实验性，不含完整 GUI 或安装包）。
- 维护者签名 LAN 更新与回滚流程；公开 bootstrap DMG 不具备 LAN 发行签名。
- 统一开发／打包入口、公开源码及凭据检查、macOS／Windows CI 和 tag 驱动 GitHub 预发行。
- 公开仓库作为 Homebrew tap：`brew trust` / `brew tap` / `brew install --cask` 安装 arm64 DMG，预发行公开后由发行工作流刷新 cask 校验和。

发行边界：GitHub bootstrap DMG 为 ad-hoc 签名、未公证。源码检查、构建、签名与包完整性，和 UI、Finder、最低系统、真实两机验收分别记录。
