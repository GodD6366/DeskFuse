# 设置与剪贴板 UI 验证（2026-10-14）

本地基线：`7ca95f1 chore: snapshot current work before UI refresh`。开始时工作树干净，因此该提交是明确的空检查点。UI 改动留在工作树，没有推送、发布或生成发行 DMG。

## 悬停按钮、预览滚动条与屏幕画布（最新）

- 按最新反馈移除复制/删除按钮外的整体圆角矩形、边框和阴影；每个按钮独立保留实心圆底。实测截图中是两个独立圆按钮，没有外框，来源背景水印保留。
- 详情外层 ScrollView 采用 `.scrollIndicators(.never)`；实测仅 SwiftUI 设置仍会出现原生指示条，因此添加只作用于此滚动容器的 AppKit 标记，全局 overlay 配置尊重该标记，不再恢复指示条。标记不参与命中测试，保留原 ScrollView 和正文结构。最终图片详情辅助功能树及截图均无右侧滚动条；滚轮自动化未观察到可靠的位置变化，不将其表述为物理滚轮/触控板验收。
- 屏幕画布使用原自适应高度的两倍，空画布也按同样倍率增加。实际两台 Mac 页面确认屏幕明显放大，名称与尺寸完整显示；流畅度模块随页面滚动，不再要求紧凑首屏容纳全部内容。没有调整屏幕位置或共享开关。
- `git diff --check`、Swift 前端解析及 `./scripts/build_and_run.sh --verify` 通过，开发版构建、签名和启动通过；本轮未添加复述样式的测试。
- 截图：`dist/ui-review/hover-buttons-solid-circles.png`、`preview-indicators-hidden.png`、`input-canvas-double-height.png`；最终构建日志：`button-canvas-preview-build.log`。没有提交、推送或生成发行 DMG。

## 流畅度曲线平滑过渡（此前验证）

用户继续反馈“波动大、不丝滑”，本轮在此前量程稳定改动上继续修改，只涉及 Swift 展示层；保留已有未提交修改。

- 此前只是固定量程并关闭刷新动画，曲线仍硬切。本轮曲线使用 trailing 3 秒均值，明确标注“3 秒平滑趋势”；典型延迟、P95 与原始峰值保持即时原始采样，峰值覆盖图表中的空闲连接采样。不跨断档、连接分段或重置的时间窗口做平均。
- 首次运行尝试 Swift Charts 隐式动画时，实测看到新旧曲线路径重叠和端点脱离；最终改为 `InputLatencyTransitionView` 逐帧线性插值，系统图表动画关闭，每帧只绘制一条曲线。持续 0.9 秒，新数据从当前插值位置接续；结束后停止时间线，非键鼠页与减弱动态效果不播放。`Motion.telemetryDuration` 为共享时长 token。
- 最终 `git diff --check`、`./scripts/build_and_run.sh --verify` 通过，开发版构建、签名和启动通过。
- `DEVELOPER_DIR=/Applications/Xcode-beta.app/Contents/Developer DESKFUSE_RUST_LIBRARY_DIR="$PWD/target/aarch64-apple-darwin/debug" swift test --package-path apps/deskfuse-macos --filter 'InputLatencyTrendTests|InputLatencyChartScaleTests|InputLatencyTransitionTests'`：9 项通过，覆盖 3 秒平滑及原始值保持、断档/分段/时间轴重置、量程稳定、逐帧中间值/不超调、插值中断续接和直接落定。
- 最终实际页面的静态截图与连续帧均显示单条平滑曲线及贴合端点，没有此前的淡出重影；纵轴维持 50 ms。录制 78 帧、约 4.5 秒并按真实帧间时间编码为 `dist/ui-review/chart-smooth.mp4`，静态图为 `chart-smooth.png`。这是实际开发版 UI 记录，不是人工生成的动效示意；截图采集约 17 fps，不能据此宣称已完成 60 fps 帧率验收或长期网络压力验收。
- 最终日志为 `dist/ui-review/smooth-chart-build.log` 与 `smooth-chart-tests.log`。本轮没有修改 Rust、连接协议、共享开关或屏幕排布，没有提交/发布。

## 来源水印恢复与流畅度图表稳定（此前验证）

本轮基线为 `ed4f92b feat: 统一工作区界面并内嵌设置`；开始时 `ClipboardPreviewPane.swift` 及两份设计文档已有未提交修改，保留。此节覆盖下方早先的来源角标外观记录。

- 来源应用恢复为原有右下角旋转/裁切的背景水印，透明度从 4% 提到 7%，移除卡片顶部的图标与名称行；详情来源应用不变。实测用户截图中的数字卡片顶部只有内容，水印位于背景。
- 图表旧实现每次快照按当前峰值选择 1/2/5/10/20/50…ms 刻度，峰值变化或过期会重新缩放整个曲线；刻度文字宽度也会改变绘图区位置。新增展示层量程状态，最低 50 ms，大峰值时扩大，在当前页面生命周期内不自动缩回；纵轴文字固定 64 pt 宽度，关闭质量区域的刷新动画。不修改 Rust 采样、延迟数值、注入或传输。
- `git diff --check`、`xcrun swiftc -frontend -parse apps/deskfuse-macos/Sources/DeskFuse/{ClipboardMasonryGrid,BrandStyle,InputLatencyChartScale,InputView}.swift apps/deskfuse-macos/Tests/DeskFuseTests/InputLatencyChartScaleTests.swift`、`./scripts/build_and_run.sh --verify` 通过。开发版构建、签名与启动通过。
- `DEVELOPER_DIR=/Applications/Xcode-beta.app/Contents/Developer DESKFUSE_RUST_LIBRARY_DIR="$PWD/target/aarch64-apple-darwin/debug" swift test --package-path apps/deskfuse-macos --filter InputLatencyChartScaleTests`：3 项通过，覆盖小波动不缩放、峰值扩大后过期不缩回，以及无效数值不破坏量程。
- 实际流畅度页观察到末端数据从约 38 ms 更新到约 19 ms，顶部刻度保持 50 ms、绘图区位置不变。数据依然每秒更新；这次观察不等于长时间网络压力或双设备功能验收。没有变更共享开关、屏幕位置或配对状态。
- 临时剪贴板搜索已清除，保留用户原来的网格视图偏好。截图为 `dist/ui-review/watermark-restored.png`、`chart-stable-range-first.png`、`chart-stable-range-second.png`；日志为 `watermark-chart-build.log` 与 `watermark-chart-tests.log`。没有提交或发布。

## 三页反馈落实（此前验证）

本轮开始时工作树已有较多 UI 与 Rust 改动，均保留；本轮增量改动限 Swift 展示、少量本机展示偏好与回归测试，没有提交或推送。

- 最终 `git diff --check`、`./scripts/build_and_run.sh --verify` 通过；开发包签名与启动通过，不是发行签名/公证/DMG 或实机双设备验收。
- `DEVELOPER_DIR=/Applications/Xcode-beta.app/Contents/Developer DESKFUSE_RUST_LIBRARY_DIR="$PWD/target/aarch64-apple-darwin/debug" swift test --package-path apps/deskfuse-macos --filter 'ClipboardPresentationTests|DisplayLayoutTests|SettingsNavigationTests|RemoteRdpAppPresentationTests'`：22 项通过（17 项 XCTest + 5 项 Swift Testing）。新增测试覆盖系统分组 ID 与自定义名称区分、仅图片筛选、Windows 哈希别名/路径名称/上游显示名与内部组件误判边界。
- 实测 53 bytes 短文本：正文后紧跟元数据与复制动作；无“可复制”徽标。长文本网格最多四行且显示省略，来源应用角标可读；图片筛选切到独立网格偏好，图片详情保持大预览。临时搜索已清除，“全部”恢复列表，外观恢复跟随系统。
- 实测键鼠页浅/深色：两开关说明可见，画布上方提示不遮屏幕名称；紧凑窗口首屏包含流畅度状态与典型延迟。没有操作共享开关或保存屏幕排布；拖拽参考线尚未做实机手势验收。
- 真实 Windows 目录就绪时：用户名与密码为空，启动按钮禁用；锁定提示、非 PIN 说明、默认记住本机凭据、独立星标、Google Chrome/Internet Explorer/Microsoft Edge/记事本及默认折叠的两个内部组件均在辅助功能树中确认。浅/深截图确认默认应用图标。未填写、保存或发送凭据，未远程启动；收藏落盘/重开持久性及凭据错误没有实机验收。
- Windows 目录多次刷新后再次出现“Windows 应用连接已断开，请点击刷新重试”，发生于读取目录后，根因未定位；本轮未修改该连接协议。刷新可短暂读取应用，截图仅证明该就绪阶段的 UI。
- 图片驻留放大已实现并编译；计算机控制中的坐标操作未能可靠触发驻留，尚未取得放大气泡实测证据。
- 主要辅助文字色在浅色画布/表面的理论对比度分别为 6.35/6.98:1，深色画布/表面为 9.46/8.54:1；不将此表述为所有控件和浮窗的完整无障碍审计。
- 中途最终编译曾因新增无障碍标签引用旧 `displayTitle` 函数失败；已改为展示 helper 并重新构建、签名、启动通过。

截图：`dist/ui-review/feedback-clipboard-{light|dark}.png`、`feedback-clipboard-grid.png`、`feedback-input-{light|dark}.png`、`feedback-windows-{light|dark}.png`。最终构建及测试日志：`feedback-build.log`、`feedback-tests.log`。均为本机证据，没有发布。

## 主窗口与浮动界面统一（最新）

按用户确认扩大至所有主功能页：主窗口采用稳定底色与实心分组；菜单栏面板、暂存区保留原生玻璃，内部动作沿用共享字体、尺寸、圆角与按下反馈。

- `git diff --check`、`cargo check --locked`、官方 `./scripts/build_and_run.sh --verify`（构建、签名、启动）通过。
- `SettingsNavigationTests` 的 3 项 XCTest 与 `WifiPermissionTests` 的 3 项 Swift Testing 回归通过；没有新增复述外观实现的测试。
- 主窗口 5 个功能目的地及设置通用页分别检查浅色/深色与 1200 × 780 / 960 × 620。远程应用读取了实际在线 Mac 应用目录，没有启动远程应用或修改凭据。
- 设备路由在紧凑窗口改为纵向信息行，宽窗口保留表格；IP、来源、状态和时间保持可读。剪贴板分组栏按剩余空间收缩，给历史和预览保留最小宽度。
- 真实菜单栏 popover 检查浅色及深色。修正 popover 沿用状态栏锚点浅色外观的问题；搜索 `seedance` 返回结果，列表/瀑布流切换正常，检查后恢复瀑布流并清除搜索。
- 实际点击面板“更多操作 → 设置…”后，面板关闭并定位 `deskfuse-main` 的内嵌设置，没有新增独立设置窗口。
- 暂存区实际打开、展开，并检查最终浅色和深色外观；保留 AppKit 单层 regular 玻璃与半透明窗口，同步原生外观后深色文字对比正常。关闭主窗口后 `⌘,` 重新打开主窗口并定位设置。
- 最终开发版再次验证剪贴板搜索 → 工作流入口 → 设置 → 返回剪贴板：工作流分段定位正确，隐藏历史控件退出辅助功能树，搜索内容保留；检查后清空临时搜索并恢复跟随系统外观。
- 既有搜索、草稿、保存、确认和权限回归范围见下方最初验证记录。本次没有执行不可撤销删除、配对、共享开关变更或实机双设备传输/输入验收。
- 界面约定更新于 `design/current/README.md`；仍为本地工作树改动，不提交、不推送、不发布、不生成 DMG。

最终截图保存在 `dist/ui-review/`，`unified-{页面}-{light|dark}[-compact].png`：页面包括 `settings`、`clipboard`、`input`、`applications`、`sharing`、`devices`。浮窗为 `unified-menubar-light.png`、`unified-menubar-dark.png`、`unified-menubar-dark-search.png`、`unified-menubar-dark-list.png`、`unified-shelf-light.png`、`unified-shelf-dark.png`。

## 后续设置整理

按用户反馈统一通用页所有开关尺寸、主菜单左边距，以及设置行的名称/说明/右侧控件。设置中的键鼠共享与主功能页重复，已移除这一分类和“共享设置”按钮；旧 input 设置路由与快捷键页入口定位主功能页，现有主控、共享键鼠开关保留。

- `git diff --check` 和 `./scripts/build_and_run.sh --verify` 通过。
- 导航 3 项与 Wi-Fi 权限 3 项测试通过；导航测试增加移除 input 分类及旧入口定位主功能页的回归检查。
- 实测设置侧栏为 7 个分类；快捷键页“打开键鼠共享”定位含主控、共享键鼠开关的主页面，不再有“共享设置”按钮。
- 剪贴板设置保留天数临时从 30 改为 31，经快捷键、键鼠共享及剪贴板工作区往返仍保留 31 和“未保存”；已撤销回 30，没有保存临时配置。
- 检查通用、剪贴板和传输设置行的说明与控件对齐；检查标准/紧凑尺寸的浅色和深色布局、长说明换行和底部操作。没有改变权限、共享状态或执行清理。
- 更新截图：`dist/ui-review/settings-refined-light.png`、`settings-refined-dark.png`、`settings-refined-light-compact.png`、`settings-refined-dark-compact.png`。下方为最初改造的验证记录与截图，原 8 分类验收已由本节的 7 分类覆盖。

## 自动检查

- `git diff --check`：通过。
- `cargo check --locked`：通过。
- `./scripts/build_and_run.sh --build`：通过。
- `./scripts/build_and_run.sh --verify`：最终源码构建、签名并启动开发版通过。
- `swift test --package-path apps/deskfuse-macos --filter SettingsNavigationTests`：3 项通过；覆盖默认分类、指定分类、工作流分段、重复打开和主窗口路由。
- 最终 `swift test --package-path apps/deskfuse-macos --filter 'SettingsNavigationTests|WifiPermissionTests'`：导航 3 项与 Wi-Fi 权限 3 项均通过。
- Swift 测试使用 `DEVELOPER_DIR=/Applications/Xcode-beta.app/Contents/Developer`、`DESKFUSE_RUST_LIBRARY_DIR=$PWD/target/aarch64-apple-darwin/debug`。首次测试编译因现有 Wi-Fi 测试引用 macOS 不支持的 `authorizedWhenInUse` 失败；已加平台条件修正。

## 开发版实测

| 检查 | 结果 |
| --- | --- |
| 主侧栏设置、应用菜单“设置…”、⌘, | 在 `deskfuse-main` 中定位设置，没有独立设置窗口 |
| 关闭主窗口后 ⌘, | 重新打开主窗口，显示设置 |
| 剪贴板工作流入口 | 定位“设置 / 剪贴板 / 工作流” |
| 全部 8 个设置分类 | 可切换，标题与分类一致 |
| 设置草稿往返 | 保留天数由 30 临时改成 31，切回剪贴板后再进入设置仍为 31；已撤销回 30 |
| 设置保存 | 现有值保存成功，显示“已保存，快捷键已生效” |
| 隐藏页面辅助功能树 | 设置显示时不包含剪贴板历史控件；剪贴板显示时不包含设置控件 |
| 剪贴板搜索、选择、滚动、分页 | 搜索和选中内容在设置往返后保留；列表中段保留，追加分页后滚动比例随总长度调整 |
| 来源筛选 | 可以按来源切换结果；验证后恢复所有设备 |
| 列表和瀑布流 | 可切换；窄历史栏使用单列，宽栏双列；方向键按实际列数导航 |
| 多选删除 | 选中 2 条，出现明确的不可撤销确认；已取消，没有删除历史 |
| 复制 | 中性文本复制成功，按钮显示“已复制” |
| 权限入口 | “查看设置”打开 macOS 对应权限页，没有更改权限 |
| 外观与尺寸 | 浅色、深色，1200 × 780、960 × 620；修正了窄栏条数挤压和瀑布流时间换行 |

验证结束时恢复原有瀑布流偏好、清空临时搜索、恢复跟随系统外观和标准窗口尺寸。开发版停留在设置页。

初次验证时菜单栏设置按钮只确认源码；后续已实际点击并完成主窗口路由验收，见上方最新记录。没有实际执行不可撤销删除，也没有重新验收跨设备图片/文件正文获取；历史中的部分远端图片预览返回原有“无法读取预览”提示。

## 截图

截图保存于 `dist/ui-review/`（本地交付文件，未纳入 Git）。

| 页面 | 浅色标准 | 深色标准 | 浅色紧凑 | 深色紧凑 |
| --- | --- | --- | --- | --- |
| 设置 | `settings-light.png` | `settings-dark.png` | `settings-light-compact.png` | `settings-dark-compact.png` |
| 剪贴板列表 | `clipboard-light.png` | `clipboard-dark.png` | `clipboard-light-compact.png` | `clipboard-dark-compact.png` |
| 剪贴板瀑布流 | `clipboard-light-grid.png` | `clipboard-dark-grid.png` | `clipboard-light-grid-compact.png` | `clipboard-dark-grid-compact.png` |
