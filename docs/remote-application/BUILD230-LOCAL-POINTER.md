# Build 230：本地鼠标与输入就绪诊断

用户反馈 build 229 仍提示「输入服务尚未就绪」，远程时鼠标无法操作。本轮没有新的 A/B 日志；以下是源码路径修正，不能视为此次现场故障已被双机复现或验收。

## 修正

- A 的 `RemoteVideoView` 接收普通 AppKit 鼠标移动、点击、拖动、滚轮和键盘事件，按当前画面映射到 B。A 不进入锁鼠标或隐藏鼠标模式；B 的视频不再烧录远端光标，避免双光标和光标延迟。
- 画布保持焦点时，即使 B 控制申请失败或画面换代，A 仍暂停跨屏共享，保证本地鼠标可用。失焦、返回应用列表、关闭或断开会话时才恢复跨屏共享。释放 B 的输入租约与恢复 A 的跨屏共享成为两个独立动作。
- 输入 helper 只有 Tokio 主循环，没有 AppKit 主循环。远程输入前台检查改用公开的同步 Process Manager 查询，避免依赖 `NSWorkspace` 的运行循环更新。保留真实前台切换、权限撤销、锁屏、租约超时的释放保护。该 API 已被 Apple 标记弃用，当前支持目标仍提供，未来应迁移到具有受控 AppKit 主循环的观察器。
- helper 上报与当前 peer/session/generation 绑定的配置请求和失败原因。A 收到具体的权限、进程、输入占用、前台切换或心跳失败信息，不再把所有失败都归为权限问题。
- 画面换代使旧控制申请失效；正在控制时，A 在新画面实际呈现后申请新控制，避免窗口大小变化后必须反复手动申请。

## 交付与待验证

按项目默认脚本 `scripts/package-macos.sh`，固定本地 Apple Development 身份，输出到主项目 `dist/0.2.3-build230/`。只进行构建、签名和包完整性检查，不执行本机交互或双机功能测试。

构建、嵌套签名及 `verify-dmg.sh` 均已通过。签名为 Apple Development: changchun.dcc@gmail.com (5GM2LLJFA6)，Team UNZ234UP68；未公证。DMG：`dist/0.2.3-build230/uniDesk-arm64.dmg`，SHA-256：`adb3549262dde8521c0f2ee136e51ab213b9c6671b698f6ade397aa025929a9f`。构建日志：`artifacts/remote-application/local-pointer-package.log`。

两端更新后重点验证：本机鼠标能自由进出远程窗口；画面内移动、左右键、拖动、滚轮和连续键入作用到 B；切换本机窗口停止转发；控制申请失败时本机鼠标仍可用；B 前台切换时明确释放。若失败，立即导出两端日志，核对具体失败原因、`remote_foreground` 和 `input_progress`，不要先重启清掉现场。
