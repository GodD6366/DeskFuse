# 定位 DeskFuse 内存占用

导出的诊断文件只含导出时刻的 `footprint` 分类和峰值，没有分配归因，所以它只能回答“内存长在哪一类”，不能回答“谁分配的”。要定位到具体子系统，需要在运行中采样，并在增长时抓 `vmmap` / `heap`。

仓库里有两条路径：一条给用户电脑（一次点击式报告），一条给自己的机器（长时间观察）。

## 用户电脑：一次性报告

`scripts/deskfuse-mem-report.sh` 是给最终用户运行的版本：只读、不安装、不需要管理员权限、不联网，默认采样 2 分钟，结束后把报告放到桌面并自动选中。

把脚本发给用户，让他在“终端”里粘贴：

```bash
bash ~/Downloads/deskfuse-mem-report.sh
```

如果用户不熟悉终端，也可以让他在访达里右键脚本文档 → 打开方式 → 终端。脚本不需要可执行权限，用 `bash` 运行即可。

报告文件名是 `DeskFuse-mem-<时间>.txt`，用户可以像普通文件一样发回来。支持人员可以加长采样：

```bash
bash ~/Downloads/deskfuse-mem-report.sh --duration 600
```

报告包含：

- macOS 版本、机型、内存、swap、内存压力。
- DeskFuse 版本、PID、运行时长、CPU、footprint、历史峰值、RSS。
- footprint 分类（Malloc Large / Malloc Small / Untagged / OS Alloc Once…）。
- 采样区间内的 footprint 曲线（每 3 秒一行）和分类变化。
- `deskfuse-mygo`、`deskfuse-input-service` 的当前值和峰值。
- 采样期间检测到上涨时、或报告生成时的前 40 个内存区域（按 dirty+swapped）。
- 前 25 个堆对象。

不含剪贴板内容、文件名、窗口标题、设备名称、账号名或网络信息。不要在用户电脑上开 `MallocStackLogging` 或长时间挂采样脚本；需要调用栈时只在自己的机器上做。

## 自己的机器：长时间观察

`scripts/deskfuse-mem-watch.sh` 单文件、只用 macOS 自带工具，可以拷到任意一台 Mac 上运行：

```bash
./deskfuse-mem-watch.sh --once --heap
./deskfuse-mem-watch.sh --duration 7200 --threshold-mb 250
./deskfuse-mem-watch.sh --role deskfuse-mygo --duration 3600
caffeinate -is ./deskfuse-mem-watch.sh --duration 28800
```

输出在 `./deskfuse-memory-<时间>/`：

- `samples.csv`：每次采样的 footprint、峰值、RSS、分类明细（含子进程）。
- `summary.txt`：起止差异、峰值、各分类增长排序。
- `snapshots/<时间>-<MB>/`：触发时刻的 `vmmap -summary`、按 dirty+swapped 排序的前 40 个区域、`heap` 类表、线程列表、`vm_stat` 与 swap。

## 判读顺序

1. 看分类排序：涨在 `Malloc Large`/`Malloc Small` 说明是应用自己 malloc 的对象；涨在 `Untagged`/`OS Alloc Once`/`IOSurface` 说明是框架或 `vm_allocate` 的缓冲。
2. 看区域表：`vmmap-regions.txt` 按 dirty+swapped 排序，先看类型（`Malloc Large`、`Untagged`、`IOSurface`、`ImageIO`）再对应地址。
3. 看堆表：`heap-classes.txt` 从 `CLASS_NAME` 表头往下读，已按总字节排序；`NSConcreteData` 大块通常是图片或协议载荷，同尺寸类数量持续增加则更像缓存无上限。
4. `footprint.txt` 保留完整分类，`vm-stat.txt` 记录当时的系统 swap，便于区分应用自身占用和整机压力。

## 需要调用栈时（仅自有机）

```bash
launchctl setenv MallocStackLogging 1
open -a /Applications/DeskFuse.app
./deskfuse-mem-watch.sh --duration 3600 --threshold-mb 250
launchctl unsetenv MallocStackLogging
```

开启后 `vmmap -stacks <pid>` / `malloc_history <pid> <地址 | 地址范围>` 能给出分配该区域的调用栈。

## 瞬时峰值

1 GB 级的峰值页面退出后就消失，用 Instruments 的 Allocations 模板录一段：

```bash
xctrace record --template 'Allocations' --attach <pid> \
  --time-limit 60s --output artifacts/allocations.trace --no-prompt
open artifacts/allocations.trace
```

## 没有脚本时的等价命令

```bash
footprint -p <pid> --sample 3 --sample-duration 1800
vmmap -sortBySize <pid> | head -60
heap -s -H <pid> | sed -n '/CLASS_NAME/,+12p'
```

## 已知基线（0.8.1）

- 主进程冷启动后 1–2 小时：footprint 约 190–210 MB；`Malloc Small` ≈ 59 MB，`Malloc Large` ≈ 59–67 MB。
- 峰值 footprint 是进程生命周期高水位，可能远高于当前值（本机实测 1.2 GB），不代表常驻占用。
- `footprint` 的 Dirty 含被压缩/换出的脏页，RSS 只算常驻页，两者差距大不代表泄漏；比较时始终用同一指标。
- 诊断文件 `DeskFuse-diagnostics-*.jsonl` 里的内存记录是导出瞬间的快照，不能当作历史曲线。
