# Windows 离线桌面安装包

本目录用于 WPF .NET 8 桌面客户端与 Rust `DeskFuseService` 的首次手动安装。Windows updater helper 与签名/事务策略已随包构建，但在服务 admission、机器级提权和启动自检接线完成前保持 fail-closed；该安装器不是 LAN 自动更新器，不会把 NSIS 进程内退出码当作提交成功。

## 构建

在 Windows x64 构建机提前准备仓库锁定的 Rust、MSVC x64 / Windows SDK、.NET 8 SDK 和 NSIS 3。脚本不下载或安装工具，不执行测试，不安装服务、不启动客户端。

```powershell
# snapshot 应写在仓库外，避免把它本身纳入快照。
.\scripts\snapshot-windows-source.ps1 -OutputPath "$env:TEMP\deskfuse-source.json"
.\scripts\package-windows.ps1 -SourceSnapshot "$env:TEMP\deskfuse-source.json" -ReleaseMetadata .\dist\release.json
```

构建依赖已经缓存并且 WPF 工程已针对 `win-x64` restore 时，可附加 `-Offline`：Cargo 使用 `--offline`，dotnet 使用 `--no-restore`。首次构建未缓存依赖时，需要正常 restore；“离线安装包”指安装时无须下载 .NET runtime，不代表首次构建不需要依赖缓存。源码快照覆盖 `resources/`（包含服务编译所需的固定 transport cert/key）、Rust crates、WPF 工程及安装脚本；SHA-256 在构建前、构建后、生成安装器后核对，并拒绝快照遗漏的构建输入。新增源码必须重新生成快照。

Rust Windows target 全依赖使用静态 CRT（`-C target-feature=+crt-static`），保留调用方有效 Rust flags；结束后恢复原 `RUSTFLAGS` / `CARGO_ENCODED_RUSTFLAGS`。打包器对五个应用/helper EXE 检查普通及 delay-load imports；非系统依赖必须以准确 DLL 名称随包存在，不能依赖构建机已经安装的 VC runtime，也不会从构建机复制任意 DLL。该检查不等价于所有随包 DLL 的递归依赖或实际启动验证。

输出至 `dist/windows/`：EXE、`.manifest.json` 和 `.sha256`。清单分别记录每个载荷的 SHA-256 与 Authenticode 状态。开发机未传 `-AuthenticodeCertificateThumbprint` 时仍输出 `NotSigned`；正式发布 job 从受保护的 PFX secret 导入证书，签署载荷和安装器并要求状态为 `Valid`。没有发行清单签名的安装器仍标记 `lan_update_eligible: false`；即使 helper 已随包存在，当前 `updater_helper_enabled` 仍为 `false`，不会绕过服务协调器执行更新。

## 安装与卸载

- 安装器请求一次管理员授权，固定写入 `%ProgramFiles%\DeskFuse`，不接受用户可写路径或 `/D` 目录覆盖。
- 安装器先要求退出托盘客户端，再停止旧服务、复制完整载荷并注册 `DeskFuseService` 为自动启动的 LocalSystem 服务。SCM recovery 配置为失败后 5 秒、30 秒各重启一次，最终保持停止并显示故障；安装时会启动服务；**打包脚本本身不会**。卸载等待 SCM 异步删除完成后才清理本次载荷。
- 从卷根逐级检查安装路径祖先，目录或载荷含 reparse point 时拒绝操作；只逐层枚举直接子项，不递归跟随链接。安装前和注册服务前将安装树 owner 设为 Administrators，使用受保护 ACL：SYSTEM/Administrators 完全控制、Users 只读执行；应用后读回核对。服务 owner 同样限制为 Administrators，DACL 仅允许 SYSTEM/Administrators 完全控制，Users 查询配置/状态及启动固定服务，不得停止服务或改服务路径。已有服务指向其他路径时拒绝操作；此安装器不迁移此前手工注册在其他目录的实验服务。
- 仅此新桌面安装器创建三条固定命名的 Windows Defender Firewall 入站规则：服务程序的 UDP 42424（控制/剪贴板 QUIC）、UDP 42425（独立键鼠 QUIC）与 UDP 5353（mDNS），限定 Private profile、LocalSubnet 来源、禁止 edge traversal。没有 TCP 或任意端口放行，不更改网络 profile、全局防火墙设置或其他应用规则。卸载只移除 `DeskFuse.Installer.Private.QUIC.UDP42424` / `DeskFuse.Installer.Private.Input.UDP42425` / `DeskFuse.Installer.Private.mDNS.UDP5353` 且 group 为 `DeskFuse Desktop Installer` 的规则。旧的 service-checkpoint ZIP 仍不修改防火墙。
- GUI 通过开始菜单由普通用户主动启动；安装器不会从管理员会话启动 GUI，不创建管理员常驻任务。GUI 登录启动策略由客户端负责。桌面启动时只查询并在必要时请求固定服务的 START 权限；不会执行任意 SCM 命令。
- 卸载只删除本次载荷清单中的程序文件、快捷方式和服务注册；不会递归删除安装目录，更不会删除 ProgramData/AppData 中的身份、配对、设置或任何用户源文件。
- 这是首次安装/手动重新安装路径。复制或服务启动失败会明确中止；`deskfuse-update` 的 Windows 策略要求固定发行 Ed25519 清单、Windows 安装器 Authenticode、程序与 SQLite/WAL/SHM 备份、服务/GUI/input helper admission 及 60 秒启动自检，全部证据齐全前不得接入永久自动更新流程。现有 NSIS 安装路径仍不是事务安装器。

## 验证边界

脚本执行成功仅证明 Release 构建、四个应用 EXE 的 PE x64 结构、自包含 runtime 文件存在和安装器/输出哈希检查完成，不代表安装、SCM 生命周期、GUI、Explorer、剪贴板、键鼠或两机验收通过。NSIS 自身可能使用 x86 引导程序，其内含的客户端、服务与输入 helper 必须是 x64。

## 安装器修复 1（2026-09-13）

修复 Windows PowerShell `Add-Type` 在 NSIS 临时插件目录误读原生 `System.dll` 的失败。启动 helper 前切换工作目录；helper 同时固定 provider/native 工作目录到当前 .NET Framework 目录，并用其绝对 `System.dll` 路径编译服务权限桥。失败详情增加步骤、错误信息和行号。本次只修改安装器，不变更应用二进制或用户持久数据；安装器文件名以 `-installer-fix1.exe` 区分原包。

## 启动和图标修正（desktop-fix2）

安装后从桌面或开始菜单打开 DeskFuse 即可，GUI 自动启动并连接已安装的后台组件，不要求另开 service EXE。用户权限仅新增固定服务 START。应用/窗口/托盘使用品牌图标而非系统默认占位。此版保留 installer-fix1 的程序集修正。

## 与 macOS 同步发布版本

Windows 构建和打包使用 `-ReleaseMetadata` 指定同一份 macOS 发布描述（version/build_number/release_id），默认读取 `dist/release.json`，缺失即失败，不再回退仓库的 bootstrap build 8。发布期间核对描述哈希不变，并将该描述随 Windows 载荷安装。设置页显示编译时文件版本及 build。新一轮 macOS 发布后，Windows 使用新描述重新构建，不仅重命名旧 EXE。版本一致不代表功能或验收范围一致。
