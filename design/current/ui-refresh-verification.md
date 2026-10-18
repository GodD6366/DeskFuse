# 设置与剪贴板 UI 验证（2026-10-14）

本地基线：`7ca95f1 chore: snapshot current work before UI refresh`。开始时工作树干净，因此该提交是明确的空检查点。UI 改动留在工作树，没有推送、发布或生成发行 DMG。

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
