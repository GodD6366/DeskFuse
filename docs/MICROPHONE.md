# 跨设备麦克风（测试版）

## DeskFuse 主线迁移（2026-10-18）

`codex/voice` 已迁移到 DeskFuse 目录和新版内嵌设置页。音频与快捷键能力位分别为 17、18，避开主线 RDP 与工作流能力；两端必须使用迁移后的版本。已有 HAL Bundle ID、设备 UID 和 `uniDeskMicrophone.driver` 文件名保留，避免遗留驱动无法管理。

当前范围是实体麦克风采集、认证音频传输、虚拟麦克风和第三方语音软件快捷键。转写由第三方软件负责；早期讨论的浮动转写输入框、最终文本投递和剪贴板回退不属于当前实现。

继续完成时需验证：新 FFI/Swift/原生驱动完整构建、双架构打包与签名、双机麦克风权限及 HAL 加载、按需启停和延迟、断线/锁屏/撤权释放，以及 Fn/单键/组合键在目标语音软件中的实际行为。当前源码没有针对麦克风会话与快捷键完整状态机的专项自动化测试，建议补充授权拒绝、超时停止和目标锁定回归。


默认关闭，macOS 双端。A 选实体输入并显式允许 B；B 选 A，由第三方语音应用直接读取 `uniDesk 麦克风（来自 A）`。uniDesk 不转写、不保存音频、不操作转写文本或剪贴板。设置、允许列表、来源、快捷键持久化在 uniDesk 自有 `microphone.json`；Swift 只保留 Rust 快照的 UI 投影及 Core Audio 资源。

## 边界

- `crates/deskfuse-engine/src/microphone.rs`：配置/信任与设备网络授权、独占来源会话、原生心跳、断线/锁屏/撤权/无读取者停止。Swift 每 200ms 报告原生状态，2s 没有新报告即关闭；来源端只接受允许列表内的可信成员。
- `crates/deskfuse-transport/src/microphone.rs`：现有配对 QUIC 连接上的独立双向音频流，协议 tag 35 / capability bit 17，显式接收授权与忙碌答复。每方向最多 12 包、每包 4096 字节 Roc 数据和 1 字节接口标识。接口 1–3 为 Roc 包，4 为 200ms 一次的电平元数据，两端均显示电平。采集队列限制 100ms、传输队列限制 150ms；相对会话时钟检查 QUIC 内部滞留，超过 250ms 丢弃会话。重连使用全新流和编码器，清空驱动 decoder/环形缓冲，短缺包由 Roc 补静音。
- `MicrophoneAudio.swift`：Core Audio 选择实体输入，AVAudioEngine/AVAudioConverter 转 48kHz mono Float32。排除 uniDesk UID 及虚拟/聚合设备以防回环；不覆盖实体输入的系统默认选择。系统默认为虚拟输入时回退首个可用实体输入。
- `MicrophoneSharing.swift`：专用后台线程编码/转发/回环反馈，实际 HAL StartIO/StopIO 通过私有驱动 RPC 查询；仅有读取者才请求音频会话。没有权限时由主应用申请 `NSMicrophoneUsageDescription`。操作系统睡眠、会话锁定分别追踪。
- `MicrophoneNative`：Roc sender encoder pt96、48k mono、5ms packet、RS8M 10/4；接收端 target latency 100ms。驱动 gRPC 仅 `127.0.0.1:19712`；RTP/repair/RTCP 仅回环 `19721/19722/19723`。反馈经同一认证音频通道反向传回。
- Fn/自定义组合键走独立输入进程通道，不依赖音频流存在。路由/释放契约见 [MICROPHONE_SHORTCUT.md](MICROPHONE_SHORTCUT.md)。快捷键需要现有键鼠授权，不能扩大设备输入权限。

## 驱动、许可与交包

基于 Roc VAD v0.0.4。私有 Bundle ID `com.unidesk.microphone.driver`、UUID `E88452E3-1C2E-42C1-A1A4-7B496BC3D527`、HAL 目录 `uniDeskMicrophone.driver`、设备 UID `com.unidesk.microphone.input`；与原版共存。固定来源及校验见 `vendor/roc-vad/sources.lock.json`，本地修改、源码/依赖许可证保存在按构建号归档的 `roc-microphone-sources.tar.gz` 中，可向提供测试版的维护者索取。

`scripts/build-roc-microphone.sh` 只编译，不安装驱动。`scripts/package-macos.sh` 交付 arm64 本地 DMG，设置页直接提供安装/重装/卸载，使用系统管理员授权弹窗并展示进度、取消及失败结果，无需打开终端；驱动入口在来源端和接收端都可见，已安装但未加载时仍可卸载。操作前关闭共享，完成后由用户重新开启。DMG 保留应用、Applications 链接和原有独立驱动入口；驱动也可从应用设置内管理。完整源码归档保存在 `target/package-macos/{version}-build{number}/{arch}/`，不进入 DMG、应用或 LAN 更新 ZIP；应用保留许可证/署名、源码 SHA-256 和索取方式。构建产物布局保持原样：DMG、更新 ZIP、发行描述、签名记录和校验清单仍输出到原交付目录，LAN 更新、公共发行与 CI 流程不变。安装器仅操作自有 HAL 目录，验证 Bundle ID 和签名；重启音频服务由用户主动确认。默认输入按钮由用户显式点击；关闭共享时只在默认输入仍为 uniDesk 时尝试恢复此前设备。

本轮按用户要求只做必要构建、签名/DMG完整性和差异检查，不运行单元、UI 或双机功能测试，不安装 HAL 驱动。真实麦克风授权、HAL 加载、双机转写/延迟、模拟 Fn 与第三方软件兼容性以随包清单实机验收。

## 连接排查

设置页首次等待麦克风服务状态最多 5 秒，超时后显示错误原因（若可用）和“重试”按钮。重试重新开始 5 秒等待，并复用原后台工作线程；过期尝试的迟到结果不会清除超时提示。若原生调用一直未返回，重试也会再次超时，不会额外创建录音线程。此超时仅针对设置页初始化，不代表跨设备音频连接的超时。

虚拟设备出现只说明 B 的 HAL 驱动已加载，不能说明 A 的音频会话已建立。两端必须运行支持麦克风协议的应用；A 开启“此 Mac”共享并允许 B。仅 B 更新应用或安装驱动时，旧版 A 无法接收音频请求。

音频打开前检查该条连接的签名 Hello 协商结果，未支持时提示更新两端，拒绝/请求队列满时发送明确回复，避免空流 EOF。音频时钟在协商完成后起算，排队时间不计入 250ms 音频过期判断。诊断的 `microphone` 分类记录协商或原生会话失败原因、会话及设备标识，不记录音频内容。
