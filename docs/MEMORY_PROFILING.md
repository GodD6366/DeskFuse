# 定位 DeskFuse 内存占用

导出的诊断文件只含导出时刻的 `footprint` 分类（Malloc Large / Untagged / OS Alloc Once…）和峰值，没有分配归因，所以它只能回答“内存长在哪一类”，不能回答“谁分配的”。要定位到具体子系统，需要在运行中采样，并在增长时抓取 `vmmap` / `heap`。

## 工具

`scripts/deskfuse-mem-watch.sh` 是单文件脚本，只用 macOS 自带工具（`footprint`、`vmmap`、`heap`、`ps`、`pgrep`、`awk`），可以拷到任意一台 Mac 上直接运行，不依赖仓库或 Python。

```bash
# 拷到测试机
scp scripts/deskfuse-mem-watch.sh <user>@<mac>:/tmp/

# 当前状态 + 最大的堆分配
./deskfuse-mem-watch.sh --once --heap

# 观察 2 小时，每 15 秒一次；超过 300 MB 或比基线多 60 MB 时自动抓快照
./deskfuse-mem-watch.sh --duration 7200

# 主进程空闲约 200 MB 时，用 250 MB 作阈值更敏感
./deskfuse-mem-watch.sh --duration 7200 --threshold-mb 250

# 单独盯子进程（例如 deskfuse-mygo）
./deskfuse-mem-watch.sh --role deskfuse-mygo --duration 3600

# 通宵观察，同时阻止测试机休眠
caffeinate -is ./deskfuse-mem-watch.sh --duration 28800
```

输出在 `./deskfuse-memory-<时间>/`（可用 `--out` 指定）：

- `samples.csv`：每次采样的 footprint、峰值、RSS、分类明细（含 `deskfuse-mygo`、`deskfuse-input-service`）。
- `summary.txt`：起止差异、峰值、各分类的增长排序。
- `snapshots/<时间>-<MB>/`：触发时刻的 `vmmap -summary`、按 dirty+swapped 排序的前 40 个区域、`heap` 类表、线程列表、`vm_stat` 与 swap。
- `.state/`：最后一次采样的解析结果，`/tmp` 清理后也能回看。

## 判读顺序

1. 看 `summary.txt` 的分类排序：涨在 `Malloc Large`/`Malloc Small` 说明是应用自己 malloc 的对象；涨在 `Untagged`/`OS Alloc Once`/`IOSurface` 说明是框架或 `vm_allocate` 的缓冲。
2. 进入对应快照：
   - `vmmap-regions.txt` 是 dirty+swapped 最大的 40 个区域，先看类型（`Untagged`、`OS Alloc Once`、`IOSurface`、`ImageIO`、`Malloc Large`）再对应地址。
   - `heap-classes.txt` 从 `CLASS_NAME` 表头往下读，已按总字节排序；`NSConcreteData` 大块通常是图片或协议载荷，同尺寸类数量持续增加则更像缓存无上限。
   - `footprint.txt` 保留完整分类，`vm-stat.txt` 记录当时的系统 swap，便于区分应用自身占用和整机压力。
3. 需要调用栈时，带 `MallocStackLogging` 重启后复现：

   ```bash
   launchctl setenv MallocStackLogging 1
   open -a /Applications/DeskFuse.app
   ./deskfuse-mem-watch.sh --duration 3600 --threshold-mb 250
   launchctl unsetenv MallocStackLogging
   ```

   开启后 `vmmap -stacks <pid>` / `malloc_history <pid> <地址 | 地址范围>` 能给出分配该区域的调用栈。
4. 只想抓瞬时峰值（例如 1 GB 级，页面退出后就消失）：用 Instruments 的 Allocations 模板录一段：

   ```bash
   xctrace record --template 'Allocations' --attach <pid> \
     --time-limit 60s --output artifacts/allocations.trace --no-prompt
   open artifacts/allocations.trace
   ```

## 没有脚本时的等价命令

```bash
# 3 秒一次的总量曲线，30 分钟
footprint -p <pid> --sample 3 --sample-duration 1800

# 最大的区域和类型
vmmap -sortBySize <pid> | head -60

# 最大的堆类
heap -s -H <pid> | sed -n '/CLASS_NAME/,+12p'
```

## 已知基线（0.8.1）

- 主进程冷启动后 1–2 小时：footprint 约 190–210 MB；`Malloc Small` ≈ 59 MB，`Malloc Large` ≈ 59–67 MB。
- 峰值 footprint 是进程生命周期高水位，可能远高于当前值（本机实测 1.2 GB），不代表常驻占用。
- `footprint` 的 Dirty 含被压缩/换出的脏页，RSS 只算常驻页，两者差距大不代表泄漏；比较时始终用同一指标。
- 诊断文件 `DeskFuse-diagnostics-*.jsonl` 里的内存记录是导出瞬间的快照，不能当作历史曲线。
