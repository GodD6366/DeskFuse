# Windows / macOS 功能对齐矩阵

这份矩阵只描述当前 Go/WebView Windows 宿主的真实边界。`unsupported_platform` 不代表服务端没有同名 Rust 类型，而是当前宿主没有能够完成该 action 的运行时链路。

| 功能面 | Windows 状态 | 证据 / 入口 |
| --- | --- | --- |
| 剪贴板同步、复制、粘贴、保存、预览 | 已对齐 | `internal/winservice/clipboard_actions.go`、`internal/native/windows.go` |
| 剪贴板弹窗 | 已对齐 | `cmd/deskfuse/windows_integration_windows.go`、`internal/winservice/host_actions.go` |
| 剪贴板总开关 | 已对齐 | `commandFor` -> `SetClipboardSync` |
| 富文本编辑 | 已对齐 | `internal/winservice/library.go` 的 RTF 生成器 |
| 文件传输、文件选择、暂存区 | 已对齐 | `internal/winservice/transfers.go`、`shelf.go`、`internal/native/windows.go` |
| 键鼠共享设置、参与设备、线路选择 | 已对齐 | `internal/winservice/input.go` 的 `devices.setInputPath` |
| 自动 OCR | 当前 Go/WebView 宿主未接入 | 快照显式返回 `clipboard.automatic_ocr=false`；`clipboard.indexOCR` 无 Windows 服务命令 |
| RemoteApp 主机 | Windows 服务端、Go 管理 bridge 与 WebView 管理页已可用 | `apps/deskfuse-service/src/native/remote_app.rs`、`internal/winservice/remote_apps.go`、`frontend/src/pages/features/WindowsRemoteAppHost.vue`；支持预检、目录、已安装应用选择、注册/移除和 RDP 配置读取 |
| RemoteApp 控制端目录 / RDP viewer | 目录、ticket、loopback 隧道和系统 RDP 启动已接入 | Windows desktop IPC 复用 Rust `RemoteApplication` QUIC；Go bridge 将逻辑 RDP 文档改写为 loopback 后交给 native `OpenPath`，隧道在转发结束后自动回收；仍需 Windows 实机双端验收 |
| 麦克风共享 | WASAPI 采集、PCM→Roc session pump 和驱动安装协议已新增，虚拟设备仍未接入 | `native/microphone.rs` 提供真实输入设备与 PCM；`native/microphone_transport.rs`（`windows-roc`）按现有 `MicrophoneSession` 发送 source/repair/control 并回灌 feedback；`resources/microphone/windows` 提供签名 INF 校验、`pnputil` 安装/卸载；仍缺已签名虚拟音频驱动和 Windows 实机验收 |
| 鼠标扩展按键 | 当前宿主未接入 | Windows snapshot 不声明 `mouse` 能力，现有实现依赖 macOS 原生控制器 |
| 自动安装更新 | Windows GitHub NSIS 安装包下载并交给 native shell | `updates.check` 记录 canonical GitHub release，`download_and_install_update` 下载 `*-windows-x64-setup.exe` 后调用 `native.OpenPath`；仍需 Windows 签名安装和自更新实机验收 |
| 工作流处理文件条目 | 当前协议只接受 MIME 输入 | Windows `RunWorkflow` 不接收文件引用；普通文件发送仍可用 |

验证基线：`cd apps/deskfuse-go && go test ./...`、Windows amd64 `go test -c`、`git diff --check`。
