# 跨设备麦克风（测试版）

## 当前交互

入口为“功能 → 麦克风共享”，位于鼠标下方；鼠标、麦克风共享的侧栏与页头均标记 Beta。旧的 microphone 设置链接会跳转到新功能页。页面首屏先选择这台 Mac 的身份：“我提供麦克风”或“我使用其他设备的麦克风”；之后才显示对应的设置步骤，避免同时暴露两端的输入、权限与驱动选项。提供端只需选择实体输入；所有可信的已配对电脑都可直接使用，不需要逐台允许，但同一时间只维持一个音频会话；接收端选择来源设备、安装虚拟麦克风并在语音软件中选择它。安装文件存在但服务不可达时显示“驱动服务未响应”，不再误报未安装。

DeskFuse 已移除独立语音快捷键（录制、捕获、按键保护、IPC 配置和专用转发通道）。直接使用语音软件自身的录音入口；普通键鼠共享保持主线行为，不能据此承诺 Fn 在远端输入法中兼容。音频仍由虚拟麦克风读取状态驱动，无需开启键鼠共享。旧配置中的快捷键字段会被忽略，协议 tag 36 与能力位 18 不再使用或复用。

以下是此前构建与实测记录：

## 当前双机测试方向（2026-10-18）

用户确认：`10.10.10.1`（godd-MBP-339，保存的设备名为 godd-MBP-108）提供实体麦克风，本机 macmini-m4-wanzhao 接收。此前反向测试不代表这一正确方向的音频验收；两端来源与允许列表已纠正。本机虚拟麦克风已通过设置页管理员授权重新安装并成功加载，实体麦克风权限应在 `.1` 授予。正确方向已建立 sender / receiver 会话。来源端已完成麦克风授权，采集器启动通过，但两次 12 秒非录音读取探测仍为静音（RMS/peak 均为 0）；来源端约 2 秒后报“实体麦克风已停止提供音频”。重启来源端测试版后仍可复现。目前只能确认采集队列未产出转换后的 PCM，尚未区分采集回调缺失与转换失败，不能归因于环境安静。用户决定返回设备旁后再进行实测，暂缓双机测试；下次先验证来源端采集，再验证虚拟输入及第三方语音软件。已补充转换错误与输入帧计数诊断，尚未部署到远端。驱动管理路径漏掉 `.driver` 后缀的问题已修复于 `1585c57`；build 168 不包含此修复，应使用已通过双架构打包校验的 build 169（Apple Development 签名，未公证）。

## 2026-10-18 微信输入法实测修复

- build 173 实测先遇到本机 HAL 控制 RPC 超时；重装并重启音频服务后恢复。系统日志记录恢复设备时回环端口 19721 的 EADDRINUSE，设备最终重建成功；端口占用及慢重建的根因尚未完全确认。
- 来源端系统日志明确报输入硬件 44100 Hz 与客户端 48000 Hz 格式不匹配，tap 未创建。采集改为使用 inputNode.inputFormat，随后转换为传输的 48 kHz。
- 回调恢复后，原来的 12 包传输队列不能容纳批量音频及 FEC，触发积压断线。改为每方向 64 包，保留 150 ms 队列过期及 250 ms 网络音频年龄限制；新增批量发送接收回归，原有过期/溢出终止测试继续有效。
- 两端修复版的 10 秒非录音 Core Audio 探测收到 481280 个采样，RMS 0.01530、峰值 0.17440，证明实体输入到本机 HAL 有非零音频。此探测只统计电平，不保存音频；用户随后确认微信输入法已能识别文字，完成本次双机收音与文字识别验收；其他快捷键与锁屏/断线场景未因此视为已验收。

两端随后安装并运行 `0.5.2 build 174`（`--auto --fast` 本地测试包，Apple Development 签名，未公证）。安装路径均为 `/Applications/DeskFuse.app`。安装后再次进行 10 秒非录音读取：481280 个采样，RMS 0.01164、峰值 0.09364。`./scripts/check.sh` 通过 818 项 Rust 测试、Clippy、Swift/原生构建和三组音频回归。

## DeskFuse 主线迁移（2026-10-18）

`codex/voice` 已迁移到 DeskFuse 目录和新版内嵌设置页。音频能力位为 19；已移除的语音快捷键能力位 18 保留不再使用，避开主线鼠标设置（17）、RDP 与工作流能力；两端必须使用迁移后的版本。已有 HAL Bundle ID、设备 UID 和驱动文件名保留在 `config/previous-installation.json`，避免遗留驱动无法管理。

当前范围是实体麦克风采集、认证音频传输和虚拟麦克风。转写由第三方软件负责；早期讨论的浮动转写输入框、最终文本投递和剪贴板回退不属于当前实现。

`./scripts/check.sh` 已通过：794 项 Rust 测试、Clippy、FFI/Swift/arm64 原生驱动构建及三组合成音频回归（包含 RS8M 丢包恢复、重新创建会话）。脚本单元测试 72 项通过。回归覆盖真实 QUIC 音频/快捷键连接、授权拒绝与撤销、超时和目标锁定。

此前反向测试记录（方向已由上节纠正）：本机与 10.10.10.1 已运行签名测试版；远端 HAL 加载、RPC 连通及 48kHz 单声道虚拟设备枚举已验证。首次 8 秒 Core Audio 读取为静音且控制通道 QUIC/Hello 超时。用户允许本地网络后，控制连接恢复；复测两端均记录 `session_active`（sender / receiver），说明授权音频会话已经建立。12 秒读取仍为静音，来源端 UI 明确显示“等待麦克风授权”；当时等待本机麦克风权限，尚未取得有效音频。实际语音转写、端到端延迟、Fn/单键/组合键和物理断线/锁屏验收仍需实测。`scripts/package-macos.sh --release` 已成功生成 0.5.2 build 168 的 arm64 / x86_64 DMG 与更新 ZIP；两架构的签名、原生依赖、DMG 内容和最终 SHA-256 校验均通过。使用 Apple Development 签名，未公证；Intel 构建通过不代表 Intel 实机运行通过。产物位于主 checkout 的 `dist/0.5.2-build168/`。


默认关闭，macOS 双端。A 选实体输入并开启共享；可信的已配对 B 可直接连接，不需要逐台允许。B 选 A，由第三方语音应用直接读取 `DeskFuse 麦克风（来自 A）`。DeskFuse 不转写、不保存音频、不操作转写文本或剪贴板。设置和来源持久化在 DeskFuse 自有 `microphone.json`；Swift 只保留 Rust 快照的 UI 投影及 Core Audio 资源。

## 边界

- `crates/deskfuse-engine/src/microphone.rs`：配置/信任与设备网络授权、独占来源会话、原生心跳、断线/锁屏/撤权/无读取者停止。Swift 每 200ms 报告原生状态，2s 没有新报告即关闭；来源端只接受可信且已配对的成员；旧 `allowed` 列表仅保留读取兼容，不再作为授权条件。
- `crates/deskfuse-transport/src/microphone.rs`：现有配对 QUIC 连接上的独立双向音频流，协议 tag 35 / capability bit 19，显式接收授权与忙碌答复。每方向最多 64 包、每包 4096 字节 Roc 数据和 1 字节接口标识。接口 1–3 为 Roc 包，4 为 200ms 一次的电平元数据，两端均显示电平。采集队列限制 100ms、传输队列限制 150ms；相对会话时钟检查 QUIC 内部滞留，超过 250ms 丢弃会话。重连使用全新流和编码器，清空驱动 decoder/环形缓冲，短缺包由 Roc 补静音。
- `MicrophoneAudio.swift`：Core Audio 选择实体输入，AVAudioEngine/AVAudioConverter 转 48kHz mono Float32。排除 DeskFuse UID 及虚拟/聚合设备以防回环；不覆盖实体输入的系统默认选择。系统默认为虚拟输入时回退首个可用实体输入。
- `MicrophoneSharing.swift`：专用后台线程编码/转发/回环反馈，实际 HAL StartIO/StopIO 通过私有驱动 RPC 查询；仅有读取者才请求音频会话。没有权限时由主应用申请 `NSMicrophoneUsageDescription`。操作系统睡眠、会话锁定分别追踪。
- `MicrophoneNative`：Roc sender encoder pt96、48k mono、5ms packet、RS8M 10/4；接收端 target latency 100ms。驱动 gRPC 仅 `127.0.0.1:19712`；RTP/repair/RTCP 仅回环 `19721/19722/19723`。反馈经同一认证音频通道反向传回。

## 驱动、许可与交包

基于 Roc VAD v0.0.4。驱动 Bundle ID（`com.deskfuse.microphone.driver`）、HAL 文件名（`DeskFuseMicrophone.driver`）和设备 UID（`com.deskfuse.microphone.input`）由 `resources/microphone/driver-identity.json` 统一提供；旧安装的驱动身份只保存在 `config/previous-installation.json`，安装或卸载时校验 Bundle ID 后移除遗留的旧驱动（两者共用插件 UUID，不能并存）；插件 UUID 为 `E88452E3-1C2E-42C1-A1A4-7B496BC3D527`，与原版共存。固定来源及校验见 `vendor/roc-vad/sources.lock.json`，本地修改、源码/依赖许可证保存在按构建号归档的 `roc-microphone-sources.tar.gz` 中，可向提供测试版的维护者索取。

`scripts/build-roc-microphone.sh` 只编译，不安装驱动。`scripts/package-macos.sh` 默认交付 arm64 本地 DMG，`--release` 构建 arm64 / x86_64 手动安装包，设置页直接提供安装/重装/卸载，使用系统管理员授权弹窗并展示进度、取消及失败结果，无需打开终端；接收端在缺少驱动时显示安装入口；已安装驱动的设备在折叠的“驱动维护”中提供重装和卸载，未加载时仍可卸载。安装或卸载前关闭共享，完成后由用户重新开启；独立重启音频服务保留共享设置。DMG 保留应用、Applications 链接和原有独立驱动入口；驱动也可从应用设置内管理。完整源码归档保存在 `target/package-macos/{version}-build{number}/{arch}/`，不进入 DMG、应用或 LAN 更新 ZIP；应用保留许可证/署名、源码 SHA-256 和索取方式。构建产物布局保持原样：DMG、更新 ZIP、发行描述、签名记录和校验清单仍输出到原交付目录，LAN 更新、公共发行与 CI 流程不变。安装器仅操作自有 HAL 目录，验证 Bundle ID 和签名；重启音频服务由用户主动确认。默认输入按钮由用户显式点击；关闭共享时只在默认输入仍为 DeskFuse 时尝试恢复此前设备。

自动化检查不代替真实麦克风授权、HAL 加载、双机转写/延迟及目标语音软件的快捷键兼容性验收。实机步骤见随包清单。

## 连接排查

设置页首次等待麦克风服务状态最多 5 秒，超时后显示错误原因（若可用）和“重试”按钮。重试重新开始 5 秒等待，并复用原后台工作线程；过期尝试的迟到结果不会清除超时提示。若原生调用一直未返回，重试也会再次超时，不会额外创建录音线程。此超时仅针对设置页初始化，不代表跨设备音频连接的超时。

虚拟设备出现只说明 B 的 HAL 驱动已加载，不能说明 A 的音频会话已建立。两端必须运行支持麦克风协议的应用；A 开启“此 Mac”共享并保持与 B 的配对。仅 B 更新应用或安装驱动时，旧版 A 无法接收音频请求。

音频打开前检查该条连接的签名 Hello 协商结果，未支持时提示更新两端，拒绝/请求队列满时发送明确回复，避免空流 EOF。音频时钟在协商完成后起算，排队时间不计入 250ms 音频过期判断。诊断的 `microphone` 分类记录协商或原生会话失败原因、会话及设备标识，不记录音频内容。

## 2026-10-18 麦克风设置整理

移除 DeskFuse 专用语音快捷键及其按键拦截、IPC 和网络消息；使用语音应用自身的录音操作，音频仍按虚拟麦克风读取者自动启停。设置页按提供端/接收端分组，集中显示状态和电平，驱动维护默认折叠。旧配置中的快捷键字段可兼容读取并忽略。

`cargo check --locked --workspace --all-targets`、`./scripts/check.sh`（813 项 Rust 测试、Clippy、Swift/FFI 和原生音频检查）、`git diff --check`、`python3 scripts/release.py validate` 通过。`./scripts/package-macos.sh --auto --fast` 生成 0.5.2 build 175，已安装到本机及 10.10.10.1，两端应用签名校验通过。界面整理后的开发版已观察到远端音频传输和非零电平；build 175 的微信输入法人工转写尚待复测。此包为本地 fast 测试包，未公证或公开发布。

## 麦克风状态与占用应用（2026-10-18）

麦克风页以来源 Mac → 使用端的声音路径为主体，独立显示按需连接、传输和待机状态；配置采用对齐行，默认输入状态直接读取系统，驱动维护为次级折叠入口。电平为紧凑的分段指示，不使用进度条。

macOS 14.2 及以上通过 Core Audio process object 的 `IsRunningInput` 与 input-scope `Devices` 查询当前读取此虚拟麦克风的应用，每秒刷新一次。该查询不打开输入流，名称仅在本机界面展示，不写配置、日志或网络；不参与启停决策。macOS 14.0/14.1 或无法识别进程时沿用通用使用状态。真正的启停仍以 HAL 首次 StartIO/最后 StopIO 为准。

实测 build 175 被系统“声音”设置页持续读取，唯一进程为 `com.apple.Sound-Settings.extension`，不是微信输入法。离开该页后读取者归零，日志记录 `session_ended reason=readers_stopped`。新界面开发版再次验证打开声音页 → 显示该占用者并建立音频会话 → 离开声音页 → 自动待机，未保留录音文件。任何应用若始终保持输入流打开（包括电平预览），仍算实际读取；DeskFuse 无法据此判断应用内部的“录音”按钮状态，不以静音截断会话。

交付验证：`git diff --check`、`cargo check --locked`、`python3 scripts/release.py validate`、`./scripts/build_and_run.sh --build` 通过；最终 `./scripts/package-macos.sh --auto --fast` 使用 build 176 的 descriptor 重建成功。两端 `/Applications/DeskFuse.app` 均为 0.5.2 build 176，深度签名验证通过。占用应用查询在独立后台任务中完成，避免阻塞音频 worker。开发版完成 UI 与声音设置 StartIO/StopIO 验证；安装后本机驱动启动阶段曾短暂出现 RPC Deadline Exceeded，之后探针确认 available=1、readers=0，远端也没有活动输入流。已安装版的 UI 二次切换验证被电脑控制超时及用户窗口交互中断，没有重做微信人工转写验收。

## 独立重启音频服务入口（2026-10-18）

“驱动维护”使用与连接设置相同的实心卡片、左侧说明和右侧操作行；卸载入口采用普通按钮，破坏性强调保留在确认弹窗中。服务未响应时，主状态区直接提供“重启音频服务…”；维护卡片中也始终保留此入口。

独立重启通过已有管理员授权流程调用 `manage-driver.sh restart-audio`，仅向 `coreaudiod` 发送终止信号，由系统重新启动服务；不安装、删除或替换任何驱动文件，也不更改共享开关。弹窗明确说明本机所有录音、播放会暂时中断。执行成功只表示已发出重启请求；音频 worker 清除会话和驱动缓存并重新查询服务，界面显示实际连接状态。取消与命令失败单独提示；语音应用可能需要重新开始录音。

交付为本地测试版 0.5.2 build 177，两端已安装并通过深度签名校验，本机随包管理脚本与源码逐字节一致。通过 `git diff --check`、`bash -n resources/microphone/manage-driver.sh`、`python3 -m unittest discover -s scripts/tests -p 'test_microphone_driver_management.py'`（2 项隔离测试）、`cargo check --locked`、`python3 scripts/release.py validate`、`./scripts/build_and_run.sh --build`、`./scripts/package-macos.sh --auto --fast`。实际检查维护卡片展开布局、状态区快捷恢复入口、重启说明弹窗及取消后共享开关保持开启；未执行管理员重启，真实恢复结果仍需用户在应用内确认后验证。

## 功能导航与 Beta 标记（2026-10-18）

麦克风共享从设置分类移到功能列表，紧跟鼠标；鼠标、麦克风共享在侧栏和页头统一显示 Beta。旧麦克风设置链接自动跳转到新功能页，随包双机测试说明同步更新导航路径。

`git diff --check`、`python3 scripts/release.py validate`、`cargo check --locked`、`./scripts/package-macos.sh --auto --fast` 通过。本机与 10.10.10.1 均安装 0.5.2 build 178，深度签名校验通过；本机随包双机测试说明与源码一致。实际界面已确认入口顺序、两个页面的 Beta 标记以及设置列表中移除麦克风分类。当前本机仍显示驱动服务未响应，本次未执行管理员重启或重新验收语音转写；本次只调整导航，未改变音频逻辑。

## 应用重启后驱动控制服务无响应（2026-10-18）

build 179 现场：本机独立进程加载随包 `libroc-microphone.dylib` 连续三次调用 `ud_mic_driver_available()`，均在 150–160 ms 返回 `Deadline Exceeded`；10.10.10.1 的同类探针在 1–4 ms 成功，更新到 build 179 后仍成功。说明故障不只存在于 UI 投影或应用内缓存。

本机驱动运行在系统独立进程 `Core Audio Driver`（当时使用旧安装的驱动 bundle 名称，见 `config/previous-installation.json` 的 `microphone.bundleName`），当时 PID 87049、启动于 22:05:21。日志显示 HAL 设备创建和多次 StartIO/StopIO 正常；19712 端口由该进程监听，TCP 连接已建立，服务端接收队列保留 82 字节，多条旧连接处于 CLOSE_WAIT。控制服务尚未处理客户端握手/请求；这还不能证明具体死锁点。普通用户 `sample` 被 macOS 权限拒绝，`sudo -n` 没有缓存授权；已请用户手动执行只读的 `sudo /usr/bin/sample 87049 2 -file /tmp/deskfuse-driver-stall.sample.txt`。用户随后提供了堆栈，下面记录定位与修复。


用户提供的 `sample` 显示音频、Roc 和 RPC 工作线程仍在等待事件，没有发现设备锁死锁。利用与安装版 `__text` 完全一致的本地目标文件还原符号后，继续检查固定版本 gRPC 1.63 的监听错误分支。

隔离回归复现了两条永久失联路径：`accept()` 返回 `ECONNRESET` 时，监听器停止接收；在本机真实 TCP 客户端设置 `SO_LINGER={1,0}` 后突然退出，macOS 还会返回已接受但对端地址为空的连接，地址转换失败同样终止整个监听器。两者均会留下仍在监听、但不处理新请求的端口，与现场表现一致。采样本身无法追溯当时发生的是哪条分支。

`patch-grpc.py` 同时修复旧 POSIX 与 EventEngine 监听器：连接重置继续 accept；无效对端地址关闭当前连接后继续监听。补丁 SHA-256 纳入依赖锁定；CMake 将补丁内容哈希加入 patch 命令，确保普通和 disconnected 增量构建都重新编译依赖。没有增加常驻录音或自动重启系统音频。

`./scripts/test-microphone-native.sh --built` 通过三组合成音频测试，以及两种监听实现各自的连接重置注入、空地址注入和 40 次真实突然断连/重新 RPC 连接。旧实现分别复现 `Failed accept4: Connection reset by peer` 和 `Invalid address: ... Empty address` 后持续超时。测试使用独立临时端口与模拟 RPC 服务，不加载 HAL、不采集实体麦克风。驱动修复必须重新安装 HAL 并重新加载才会对现有机器生效；仅替换应用不会更新正在运行的驱动。

交付检查：`cargo check --locked`、`git diff --check`、`bash -n scripts/test-microphone-native.sh`、`python3 vendor/roc-vad/verify-sources.py target/roc-microphone/build-arm64`、`python3 scripts/release.py validate`、`./scripts/test-microphone-native.sh --built`、`./scripts/package-macos.sh --auto --fast` 通过。本机应用已替换为 0.5.2 build 180（release ID `60f9dfc9-4142-45d3-9aa5-0bce91d54f28`），深度签名校验通过；随包驱动与最终构建 UUID 均为 `D9DA9864-9CC2-3837-9C93-717A99802AD6`。这是 Apple Development 签名的本地 arm64 测试包，未公证、未发布。

用户完成管理员授权后，安装目录和新进程 PID 82929 的 sample 均确认加载 build 180 的驱动 UUID；但独立 RPC 探针仍持续超时，实际 HAL 恢复验收未通过。新堆栈显示请求已进入 gRPC 的握手及通道创建路径；系统日志于 22:55:22 记录该驱动超过 15 MB 内存高水位（ActiveSoft / non-fatal）。这些证据不能把剩余超时归因于已修复的 accept 分支，也不能单凭非致命内存告警断定全部根因。


### 控制服务资源收敛

原 gRPC ExternalProject 未传递 `CMAKE_BUILD_TYPE`，即使驱动选择 Release，依赖仍使用默认未优化构建。现在传递构建模式；驱动二进制由约 9.9 MB 缩到 6.0 MB。异步日志队列从 10,000 条缩到 256 条，保留原来的覆盖旧日志策略，避免阻塞音频。专用控制网络库的常驻工作线程改用上游最小值 4，不再随 CPU 核数预分配，仍允许按负载增长；Roc 音频线程不受影响。

gRPC 错误转发到驱动已有系统日志，弥补系统服务标准错误输出不可见的问题；不采集音频、转写内容或用户文本。以上资源调整需要新驱动实机验证，不能仅凭体积下降认定超时已解决。

相同的独立控制服务回归（含客户端与服务端）的 `/usr/bin/time -l` 峰值 footprint 从 9,994,888 降为 4,178,544 字节，约下降 58%；这不是系统 HAL 宿主的实测内存。三组合成音频、两种监听实现的错误注入与各 40 次断连重连仍通过。

### build 181 实机复测与剩余问题

本机已安装 0.5.2 build 181（release ID `73a837a7-fff8-4bad-92f6-b809cf6ba092`），用户完成管理员授权；安装的 HAL UUID 为 `D5F65EFB-C0DF-3950-9EB2-0E69E6AF5F90`。应用和驱动签名校验、`./scripts/package-macos.sh --auto --fast`、发布描述验证通过，仍为未公证、未公开发布的本地测试包。

重新加载后的真实 HAL 控制服务通过 40 次 TCP 重置连接与独立客户端 RPC 检查，最长约 1.6 ms。此时实际宿主约 4.5 MB、15 个线程，界面显示驱动运行正常、等待使用且没有采集或传输音频。一次正常退出应用后独立 RPC 仍成功。但随后应用退出、重新打开的复测中再次出现持续 `Deadline Exceeded`；并未完成五轮重启验收，不能认定故障修复。故障进程 PID 40975 的内存仍约 5 MB，端口仍监听，连接积压，当前没有证据将其归为此前内存高水位问题。

隔离回归追加独立客户端进程：每个进程持有连接完成 10 次 RPC，然后正常退出或不运行析构直接退出；两种监听实现分别通过 20 次进程退出后重新连接。此项不加载 HAL，说明仍需定位真实宿主或设备生命周期相关差异。新的只读现场采样请求为 `/tmp/deskfuse-driver-build181.sample.txt`；尚待采样后定位，不自动重启系统音频来掩盖失联。

## 2026-10-19 身份优先与多设备设置

麦克风页改为先选择身份，再进入对应流程：

- 首屏只显示“我提供麦克风”和“我使用其他设备的麦克风”两个选择。身份保存在本机偏好；已有 `source` 的旧接收配置直接按接收端打开，不需要重新选择。
- 提供端只需选择麦克风；可信的已配对电脑默认全部可用，不再逐台授权。只读设备列表区分在线、离线、正在使用和当前设备占用中；页面顶部显示已配对数量、在线数量和当前使用者。设备全部离线时显示“等待上线”，不提前显示“已就绪”。引擎仍只维持一个音频会话，其他设备得到占用中答复，不排队。
- 接收端步骤为“选择来源设备 → 安装虚拟麦克风”。驱动安装、重启音频服务、重装和卸载只在接收端显示；驱动未运行时不启用顶部开关，避免显示已开启却无法录音。
- 切换身份会关闭共享；切到提供端时清空 `source`，保留已安装驱动，便于再次切回。提供端不显示驱动维护。旧 `allowed` 配置继续可读，但不再限制连接。
- 页面统一加载卡、故障卡与状态行：首次连接显示准备说明；服务错误显示中文摘要，原始错误收进“技术详情”；驱动异常只在驱动步骤保留一个可操作状态；未完成步骤标记“待完成”，已完成步骤标记“已完成”；仅在设置完成且尚未开启时显示一次“开启共享”提示。

验证：`git diff --check`、`./scripts/check-ui-tokens.sh`（当时名为 check-design-system.sh）、`xcrun swiftc -parse apps/deskfuse-macos/Sources/DeskFuse/MicrophoneSettingsView.swift`、`cargo fmt --all -- --check`、`cargo test -p deskfuse-engine microphone --locked`、`cargo check --locked`、`./scripts/build_and_run.sh --build` 通过。开发版应用实际打开身份选择、接收端和提供端页面；确认接收端驱动未就绪时只显示一处可操作状态，提供端不再出现逐台允许开关，设备列表为只读并显示已配对数量。未触发真实的设备占用中会话，也未完成新的双机音频、微信输入法或驱动加载实测；本次调整设置交互与状态呈现。
