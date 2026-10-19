# DeskFuse 当前界面与行为约定

本文替换此前逐轮叠加的界面记录，只写**现在**生效的约定和设计系统说明。历史版本可在 git 历史中查看，不再作为并行设计稿维护。

## 目标与判断标准

以“普通用户三分钟内连上第二台电脑并用起来”为唯一标准。任何新增入口、开关或文案都要先回答：它是否让第一次使用更快完成配对并开始同步。

设计不参照旧布局，也不参照 `deskfuse-redesign-demo.html`（仅本地设计草稿，不提交）。

## 信息架构

侧栏是扁平结构，去掉“功能 / 配置”分组标题与页头面包屑：

```
DeskFuse
 ├ 设备          默认首页：本机 + 我的设备 + 附近可添加
 ├ 剪贴板
 ├ 键鼠共享
 ├ 文件传输      发送区 + 传输队列 + 手机扫码
 ├ ── 分隔 ──
 ├ 远程应用      Beta
 ├ 麦克风共享    Beta
 ├ 鼠标按键      Beta
 └ 设置（底部）
底部：健康指示“一切正常” / “N 项需要留意 →”
```

设置分类固定为：通用、剪贴板、快捷键、文件、权限、关于与更新、高级。旧深链接（如 `storage`、`updates`）映射到合并后的新分类，不再单独存在。

- 通用：开机自启、隐藏 Dock 图标、保持唤醒（避免作为被控端时休眠离线）。
- 剪贴板：保留天数、快捷键、排除 App、清空历史；工作流分区受“高级 → 启用工作流”控制。
- 快捷键：键鼠方向切换与锁定可改、唤起剪贴板、工作流（启用时）。键鼠快捷键编辑后立即提交引擎。
- 文件：接收位置、缓存上限、清除历史传输文件。
- 权限：汇总页，只展示 DeskFuse 实际使用的系统权限。
- 关于与更新：版本、更新、反馈问题（导出诊断）、更新通道。
- 高级：启用工作流、跨设备索引入口、清空剪贴板历史、诊断导出、构建号（仅内部通道）。

## 首次使用

`OnboardingFlow` 三步向导，可跳过、可返回：

1. 欢迎：一句话说明产品，显示本机名称。
2. 连接另一台电脑：实时列出可添加设备，行内“添加”直接配对；等待对方确认；次要入口“找不到？手动输入地址”。
3. 选择功能：剪贴板同步与文件传输默认开启；打开键鼠共享时当场申请辅助功能权限，并同时设置 `enabled` 与 `controller`，避免停在“等待主控”。

旧安装迁移弹窗保留，排在向导之前，不与权限提示连续弹出。

## 权限按需申请

启动时不再弹出全量权限清单。权限只在两种时机出现：功能开关打开时，或功能被权限阻断时，在页面内显示可点击的授权行。

| 权限 | 触发时机 |
|---|---|
| 辅助功能 | 开启键鼠共享，或剪贴板“粘贴回原应用” |
| 屏幕录制 | 远程应用 |
| Wi-Fi（定位） | 高级里开启“按 Wi-Fi 记住主控”时 |
| 完全磁盘访问 | 访问受保护文件失败时提示，不出现在引导 |
| 麦克风、输入监控 | 维持按需申请 |

“重新打开 DeskFuse 以启用…”只在确实需要重启的权限（屏幕录制、完全磁盘访问）已授予但尚未生效时出现。

## 状态与错误

- **健康指示**：`HealthCenter` 从引擎快照、权限与偏好派生问题列表；侧栏底部只显示一句整体状态，点开才展开清单，每条都带“修复”动作。
- **错误常驻**：错误不再数秒后自动消失，一直保留到用户处理或显式关闭；“复制详情”保留原始诊断文本。
- **错误中文化**：Rust 侧 `runtime_error_label` 覆盖握手不兼容、超时、连接关闭、拒绝连接、权限、磁盘、传输中断、存储与命令通道等类别；Swift 侧再由 `friendlyError` 做一次面向用户的改写。
- **统一空态**：`DFEmptyState` 提供图标、标题、说明与“下一步”动作。设备为空 →“添加电脑”；剪贴板为空 → 说明 ⌥⌘V；传输为空 →“选择文件发送”；引擎启动失败 →“重试”。

## 页面约定

- 每个页面只有一个标题，由 `MainContentView` 用 `DFPageHeader` 统一提供；页面内不再重复大标题。
- **设备**：列表 + 详情；附近设备行内直接“添加”；单台设备操作在“…”菜单；手动 IPv4 是“+”次要入口；连接与路由表折叠为高级信息。
- **键鼠共享**：一个开关 + 屏幕排布画布；已配对且在线的主机自动加入参与者（`AppStore` 在快照更新时提交 `.setInput`），仍可手动移除；未就绪屏幕直接显示“重新连接”；流畅度收敛为“连接质量：良好 / 一般 / 较差”，P50/P95、慢注入、数据报等进“高级信息”。
- **剪贴板**：三栏；同步开关只在历史页头一处；左栏只有“全部、文字、图片、文件、我的列表”；智能列表不再新建或编辑，已有列表照常使用；列表/瀑布流偏好统一；右键菜单“粘贴为纯文本”“发送到设备”。
- **文件传输**：发送区（拖入/选择 + 选择设备发送）→ 传输队列 → 手机扫码（开关 + 二维码 + 地址，网络接口与端口固定自动分配）→ 手机发送记录。阈值类设置不再暴露，缓存上限在设置 → 文件。
- **远程应用**：虚拟屏幕模式、分辨率、编码、刷新率、比例、性能与详细诊断收进窗口“高级”子菜单；徽标统一 Beta。
- **麦克风共享**：首屏选择“我提供麦克风 / 我使用其他设备的麦克风”；提供端选择实体输入，可信已配对电脑默认直接可用，接收端选择来源并安装虚拟麦克风；驱动维护收进接收端的故障排除。
- **鼠标按键**：不再显示 `saveStatus`、`lastExecution` 与 HID++ CID 调试文案；组合键分层与“Mac 间自动同步”说明收进“更多选项”。

## 设计系统

`Sources/DeskFuse/DesignSystem/` 是唯一的样式来源，不保留兼容层。

- `DFTokens.swift`：`DFType` 字号、`DFRadius` 圆角、`DFSpace` 间距、`DFPalette` 颜色、`DFMotion` 动效、`DFMetrics` 尺寸。
- `DFComponents.swift`：`DFPageHeader`、`DFSection`、`DFCard`、`DFSettingRow`、`DFToggleRow`、`DFValueRow`、`DFEmptyState`、`DFNotice`、`DFStatusPill`、`DFBadge`、`DFDisclosure`、`DFSidebarRow`、`DeskFuseBrandMark`。
- `DFButtons.swift`：`.dfPrimary`、`.dfSecondary`、`.dfPlain`、`.dfDestructive`、`.dfIcon`、`DFActionButton`。

### 令牌

| 类别 | 规则 |
|---|---|
| 字号 | 页标题 20 semibold，卡片标题 15 semibold，分区标签 13 semibold（次要色），正文 13，辅助 12，指标 24，键帽 13 rounded |
| 圆角 | 控件 8，卡片 12，浮层 16，细条 4 |
| 间距 | 4、8、12、16、24、32；页面左右内边距 24，表单最大宽度 680 |
| 颜色 | 唯一强调色 `DFPalette.accent`；语义色 success / warning / danger / beta；选中态用 `DFPalette.selection`，悬停用 `DFPalette.hover` |
| 动效 | `DFMotion.snappy` 交互反馈，`DFMotion.settle` 状态落定，`DFMotion.telemetryDuration` 遥测跟随；`withMotion` 在“减弱动态效果”下直接执行 |

`NativeGlass` 只用于浮层（菜单栏面板、暂存区、弹出内容）。

### 防止回退

`scripts/check-design-system.sh` 在 `scripts/check.sh` 中执行。`DesignSystem/` 之外禁止出现：

- `.font(.system(size: <数字>` 写死字号
- `cornerRadius: <数字>` 写死圆角
- `Color.orange`、`.teal`、`.purple`、`.indigo`
- `.borderedProminent`
- `Color.accentColor`

## 删除与隐藏

已删除：`LanTransferView`、`MouseButtonCaptureView`、`TransfersView`（及 `Page.transfers`）、`ClipboardHoverPreview`、`LaunchPermissionPrompt`、`QuietWorkspaceStyle`、`SystemSettingsStyle`、`WorkspaceChrome`、`BrandStyle`、`WorkflowRulesView`（共享自动规则）、`OnboardingPrompt`（由 `OnboardingFlow` 取代）、`ShareAndTransferView`（由 `FileTransferView` 取代）。

隐藏：工作流全部入口受“高级 → 启用工作流”控制，默认关闭；关闭时已注册的全局快捷键与 App Intents 不生效。智能列表不能再新建或编辑。

折叠到高级/故障排除：跨设备索引、按 Wi-Fi 记住主控、路由表、流畅度详情、远程应用参数、麦克风驱动维护、构建号与内部更新通道。

## 验证

每期改动结束执行：

```bash
git diff --check
python3 scripts/release.py validate
cargo check --locked
./scripts/check.sh          # 含设计系统守卫
```

界面结论必须区分：已通过构建与检查、已看过截图、仍需真实两台 Mac 验收（配对、键鼠穿越、文件传输、远程应用、麦克风）。
