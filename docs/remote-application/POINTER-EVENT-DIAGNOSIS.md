# 鼠标事件在转发前读取不适用的 AppKit 属性

用户反馈 build 230 仍无法点击。本轮共享目录没有新的 A/B 导出，因此没有把具体现场的失败环节当作已复现。

源码发现：`RemoteVideoView.pointer` 对所有鼠标输入统一读取 `buttonNumber`、`clickCount`、`scrollingDeltaX/Y`、`phase` 和 `momentumPhase`。macOS SDK 的 `NSEvent.h` 明确限定：按钮/点击数属于 down/up/drag；滚轮增量及阶段属于 scrollWheel。普通点击/移动读取滚轮属性、滚轮读取点击数，不符合 AppKit 契约，可能在 `onInput` 之前抛出 Objective-C 异常。`key` 对 flagsChanged 读取 `isARepeat` 也不适用。

修正仅按事件类型读取：down/up 读取按钮与点击数，scroll 读取增量/阶段，move 使用位置和修饰键，只有 keyDown 读取重复标志。拖动的按住按钮状态仍由 B 的注入器维护。

新增元数据日志可定位点击路径，不记录坐标、键值或文本：

- A：`pointer_down_received` → `pointer_down_forwarded` → `canvas_down` → `pointer_down_queued`。未转发时记录 disabled、surface_pending、outside_video、not_focused 或 gate_rejected。
- B：`pointer_down_helper_queued` → `pointer_down_native_queued` → `remote_pointer_post kind=down`。
- 抬起事件记录相同的 up 路径。native_queued 只证明入队，remote_pointer_post 只证明已调用系统投递；两者都不声称目标应用已消费点击。

沿用用户指定的本地固定签名和主项目 dist 输出。仅运行官方打包中的构建、签名、完整性检查；没有执行本机交互或双机测试。

交付 build 231：`/Users/godd/workspace/uniDesk/dist/0.2.3-build231/uniDesk-arm64.dmg`。构建、签名和 `verify-dmg.sh` 已通过，签名为 Apple Development: changchun.dcc@gmail.com (5GM2LLJFA6)，Team UNZ234UP68，未公证。SHA-256：`a6c216eb1fa2b1a11584896ab5d645829bb5edcc8972c5661d2918adb7953a35`。日志：`artifacts/remote-application/pointer-event-package.log`。
