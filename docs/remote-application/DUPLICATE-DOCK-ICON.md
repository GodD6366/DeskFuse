# 被控端重复 Dock 图标

## 原因与修复

远程应用的输入注入在 `deskfuse-input-service` 子进程中执行。该程序放在
`DeskFuse.app/Contents/MacOS`，此前只有独立的签名标识，没有独立的应用元数据。
当原生输入或进程查询接口使它向 LaunchServices 注册时，系统可能把它当成前台
应用，从而显示第二个 Dock 图标；主程序的单实例锁无法阻止这个辅助进程。

`crates/deskfuse-ffi/build.rs` 只为输入服务链接 `__TEXT,__info_plist`，声明
`com.deskfuse.app.input-service` 和 `LSBackgroundOnly=true`。辅助进程在首次调用
原生接口前就具有后台身份。主程序继续使用自己的 Dock 设置；无需修改主程序的
Info.plist，也不影响主控端远程窗口的 Dock 管理。

DMG 校验现在检查辅助程序实际嵌入的元数据，拒绝未嵌入后台声明、声明为前台或
冒用主程序 bundle identifier 的产物。

Apple 对后台声明的定义见
[Launch Services Keys](https://developer.apple.com/library/archive/documentation/General/Reference/InfoPlistKeyReference/Articles/LaunchServicesKeys.html)。

## 验证边界

使用放在独立测试应用包内的本机探针，调用 `CGEventSourceCreate`、
`GetFrontProcess`、`GetProcessPID` 和 `NSWorkspace`，不发送键鼠事件：

| 元数据 | 当前进程 activation policy | LaunchServices 类型 |
| --- | --- | --- |
| 未嵌入 Info.plist | 0（regular） | Foreground |
| 嵌入独立后台 Info.plist | 2（prohibited） | BackgroundOnly |

`python3 -m unittest scripts.tests.test_input_service_identity -v` 使用真实 Mach-O
文件覆盖正常后台声明、缺失声明、前台声明及主程序身份，四项通过。

上述验证确认了进程注册机制与包校验。两台 Mac 安装修复包后，仍需验收远程应用
打开、关闭、重复打开及断开连接期间，被控端始终只有一个 DeskFuse Dock 图标，
同时键鼠控制正常。

## 本地交付（2026-10-07）

- `./scripts/check.sh` 通过，包含格式、`cargo check --locked --workspace --all-targets`、
  工作区测试、Clippy 和 macOS 原生 debug 构建。
- `git diff --check`、`python3 scripts/release.py validate`、四项后台身份回归测试通过。
- 使用官方 `./scripts/package-macos.sh`，为本次构建独立分配 release descriptor，
  输出 `dist/0.5.0-build96/DeskFuse-arm64.dmg`。本次设置 `CI=true`，避免将验证包
  自动放入正在运行的 LAN 更新协调器；更新通道仍为 LAN。
- Release 构建、嵌套签名、后台身份校验及 `scripts/verify-dmg.sh` 通过。
  使用已有 Apple Development 证书，Team `UNZ234UP68`；未公证，未安装到当前机器。
- DMG SHA-256：`6b3c29cebd36f9ab5467858e797ba16a55f9fafcc249a1587177275fb98d9abd`。
