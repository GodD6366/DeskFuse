# SwiftUI hosting 环境内存滞留修复

## 现场与定位

- 环境：macOS 27.2，DeskFuse 0.5.2 本地 Debug 开发版（Apple Development 签名），同一台 Mac、同一份用户数据。
- 旧主进程 PID 96875，运行约 2 小时。23:24:52 physical footprint 1025 MB；23:30:06 为 1074 MB，历史峰值 1.4 GB。输入服务约 35 MB，不是主要来源。
- `heap` 找到 `Swift._ContiguousArrayStorage<SwiftUI.AnyTrackedValue>` 合计 1,226,377,008 bytes，其中两个数组分别为 1,073,741,824 与 152,567,808 bytes。
- `leaks --noContent --trace=0x7d8b400000 96875` 的引用链包含 `ClipboardHistoryPane` → `_EnvironmentKeyWritingModifier<EnvironmentValues>` → `PropertyList.Tracker` → `invalidValues` → 1 GiB 数组。
- `leaks` 普通扫描返回 0，但提示目标不可调试；该结果不能排除仍被强引用的对象堆积。没有用“0 leaks”作为修复判据。

## 修复

`HistorySplitView` 与 `SettingsPageContainer` 不再通过 `.environment(\.self, context.environment)` 把父图的完整环境及 tracker 保存在新的 hosting root 中。改为 `HostingEnvironment`，仅提取明确使用的值，逐项传递；系统只读无障碍偏好由 native hosting 环境提供，应用的 tint 与按钮样式显式应用。

保留原有 NSHostingController、页面身份、分栏尺寸与页面草稿，不以反复销毁页面或清空用户数据降低内存。

## 验证方法

- `HostingEnvironmentTests`：父环境中无关对象可释放；外观、字号、控制状态、布局方向、locale、缩放及 scene phase 的值能提取。
- `ClipboardPreviewStabilityTests`：与以上合计 4 项测试通过。
- `./scripts/build_and_run.sh --build`、`codesign --verify --deep --strict dist/DeskFuse-dev.app`、`git diff --check` 通过。
- 修复版 PID 4261；启动后每 15 秒采集 `footprint` 和 RSS，持续 10 分钟。footprint 是主要比较指标，RSS 受压缩和页面驻留影响，不能混为同一数值。
- 实际通过 UI 完成 20 轮剪贴板 → 设置 → 鼠标（60 次导航动作）；G502 分类选中状态保留，设置外观及剪贴板布局可用。之后停留在剪贴板页继续接收后台快照更新。
- 早期堆采样：目标数组合计 27,840 bytes；约第 4 分钟为 24,512 bytes，而非原来的 GiB 级数组。

## 10 分钟结果

原始采样：[CSV](2026-10-17-hosting-memory.csv)，北京时间 23:30:44–23:40:44，41 个点。

| 阶段 | Physical footprint |
| --- | --- |
| 旧进程重启前 | 1074 MB |
| 修复版全程（含冷启动与 60 次切页） | 172–221 MB，中位数 184 MB |
| 修复版第 2–10 分钟 | 174–202 MB，起点 188 MB、终点 186 MB |
| 修复版后 5 分钟 | 177–188 MB，中位数 184 MB |
| 修复版第 10 分钟 | 186 MB，RSS 112.8 MB |

约第 8 分钟及完成 10 分钟采样后再次采集，目标数组合计均为 24,512 bytes，与第 4 分钟一致。观察期内未复现原先 tracker 的持续累积，没有用清空页面、周期重启或强制清缓存制造下降。修复后的进程保持运行。

结论只覆盖当前版本、当前工作负载的 10 分钟观察区间；旧进程已运行约 2 小时，不能把冷启动下降单独当成对照证明。实际判据是重复操作后回落、后半段的稳定区间及目标数组不再扩张。更长时间运行和其他功能路径尚未覆盖。
