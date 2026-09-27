# Rust 公共底层与平台层分离开发计划

## 目标

将项目拆成：

```text
Rust 公共底层
├── 协议、发现、配对、信任
├── 剪贴板/文件传输状态机
├── 工作流模型、执行计划和跨设备同步
├── 输入布局、租约和事件策略
└── 更新验证与事务状态机

Windows 平台层
├── SCM/UAC/UIAccess/session agent
├── Win32 输入、OLE、系统剪贴板
├── WPF UI
└── 原生安装/提权/替换

macOS 平台层
├── SwiftUI/AppKit
├── Finder/Pasteboard
├── 输入 helper 和权限
└── 原生安装/提权/替换
```

Rust 公共层只表达业务状态、协议和平台请求；Windows/macOS 原生层负责实际操作系统调用并回报结果。

## 并行分工

### Agent A：更新器和平台执行端口

范围：

- 从 `unidesk-update` 抽取平台无关的 manifest、签名、版本选择、事务状态机和恢复策略。
- 定义 `PlatformInstaller` / `PlatformUpdateExecutor` 接口。
- 将 Windows UAC、SCM、进程停止、文件替换、SQLite/WAL/SHM 备份恢复留在 Windows 实现。
- 将 macOS 安装、helper、备份和回滚适配到同一接口。
- capability 只能由 admission/self-test/签名状态共同决定。

禁止：

- 公共 Rust 直接调用 Win32/AppKit。
- 放宽当前 fail-closed 更新策略。
- 把 NSIS 首次安装包当事务更新包。

当前交付：`unidesk-update` 仅保留 manifest、签名、版本选择、事务状态机和
`PlatformInstaller`/`PlatformUpdateExecutor` 端口；`unidesk-update-platform`
是过渡性的 Rust 执行适配层，承载现有 macOS bundle/helper/备份回滚和
Windows UAC/SCM/进程/SQLite-WAL-SHM 实现。该适配层不代表原生 Swift/C# 实机
能力已验收；后续迁移时必须保留 helper 名称/请求 ABI、签名校验和 NSIS
fail-closed 约束。待迁移函数集中在该 crate 的 `macos.rs` 与 `windows.rs`，
公共事务策略不得重新引入操作系统调用。

### Agent B：输入和权限平台端口

范围：

- 保留 Rust 的 layout、lease、SharingController、事件批次和路由策略。
- 定义平台无关的 capture/inject/desktop capability 端口。
- Windows 实现 UIAccess、session、desktop、OLE 外的输入注入、session-agent 授权。
- macOS 实现 helper、Accessibility/Input Monitoring、CGEvent 等原生适配。
- 平台层返回真实 capability、失败原因和释放结果。

禁止：

- Rust 公共层包含 Win32/AppKit 调用。
- 通过普通 UIAccess helper 宣称支持 Winlogon、锁屏、SAS 或凭据输入。
- 把平台权限探测结果硬编码进公共状态机。

### Agent C：剪贴板、文件、工作流和 FFI 边界

范围：

- 抽取公共剪贴板/文件传输状态机和 workflow execution plan。
- 定义 typed platform port：读取/写入剪贴板、创建文件数据对象、执行文本/图片/文件步骤。
- Windows 保留 WPF/OLE/CF_HDROP 实现；macOS 保留 Pasteboard/Finder 实现。
- 统一 FFI/IPC 命令、事件、错误和 capability 映射。
- 确保 workflow sync 只同步模型和规则，平台执行只处理已授权的计划。

禁止：

- 在 WPF 或 Swift 中复制协议、信任和同步逻辑。
- 在 Rust 公共层创建 IDataObject、NSPasteboard 或系统快捷键。
- 静默把不支持的平台步骤当作成功。

## 合并顺序

1. 先合并三组接口和类型定义，解决命名与错误模型冲突。
2. 合并更新器端口，确认 `updater=false` 的 fail-closed 行为不变。
3. 合并输入端口，确认 lease/reclaim/release 语义不变。
4. 合并剪贴板、文件、workflow 和 FFI 适配。
5. 删除公共 Rust 中剩余平台 API 和重复系统逻辑。
6. 更新 `ARCHITECTURE.md`、Windows/macOS 文档和 capability 矩阵。

## 交付边界

每个 agent 必须：

- 只修改分配范围。
- 提交独立 commit。
- 不运行测试（本轮按要求只修改代码）；至少执行 `git diff --check`。
- 在交接摘要中列出未迁移代码、兼容性风险和后续删除项。
- 不把“接口已抽取”描述为“实机能力已验收”。

## 完成定义

- 公共 crates 不依赖 Windows/AppKit 系统 API。
- 平台权限、剪贴板、OLE、Finder、安装替换均由平台层实现。
- Rust 只负责协议、状态机、策略、计划和可序列化事件。
- 两个平台的 capability、错误和回执模型一致。
- Windows/macOS 可以分别构建平台层而不改变公共业务协议。
