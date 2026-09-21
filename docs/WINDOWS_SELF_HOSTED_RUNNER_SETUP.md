# DeskFuse Windows Self-hosted Runner 配置提示词

将下面这段提示词交给 Windows 机器上的 AI 执行。

```text
请把本机 Windows 机器（IP：10.0.0.11）配置成 GitHub Actions self-hosted runner，服务于：

https://github.com/GodD6366/DeskFuse

目标：

1. 注册 Windows x64 self-hosted runner。
2. 安装为 Windows 服务，开机自动运行。
3. 保持现有 `runs-on: windows-2022` workflow 兼容。
4. 不泄露 runner registration token，不把 token、密码或凭据写入文件、日志或 Git。
5. 不修改无关源码，不自动重启机器。

## 1. 检查系统和依赖

使用管理员 PowerShell 检查：

```powershell
whoami
[Environment]::Is64BitOperatingSystem
Get-ComputerInfo | Select-Object WindowsProductName, WindowsVersion, OsArchitecture
git --version
rustup --version
cargo --version
dotnet --info
Get-Command makensis.exe -ErrorAction SilentlyContinue
Test-NetConnection github.com -Port 443
```

必须准备：

- x64 Windows
- Visual Studio Build Tools 2022
- MSVC x64 工具链和 Windows SDK
- .NET 8 SDK
- NSIS 3
- Rust toolchain `1.97.1`
- Rust target `x86_64-pc-windows-msvc`

如果 Visual Studio Build Tools 或 Rust 缺失，优先运行仓库脚本：

```powershell
.\scripts\bootstrap-windows.ps1
```

不要自动重启机器；如果需要重启，只报告 `REBOOT_REQUIRED` 并停止。

## 2. 下载并校验官方 Runner

安装目录：

```text
C:\actions-runner\deskfuse-windows-x64
```

从 GitHub 官方 `actions/runner` release 下载最新的 `actions-runner-win-x64-*.zip`，并用 GitHub Release API 提供的 SHA-256 digest 校验：

```powershell
$release = Invoke-RestMethod https://api.github.com/repos/actions/runner/releases/latest
$asset = $release.assets | Where-Object { $_.name -match '^actions-runner-win-x64-.*\.zip$' } | Select-Object -First 1
if (-not $asset) { throw "GitHub Actions Windows runner asset not found" }

$archive = Join-Path $env:TEMP $asset.name
Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $archive

$actual = (Get-FileHash $archive -Algorithm SHA256).Hash.ToLowerInvariant()
$expected = ($asset.digest -replace '^sha256:', '').ToLowerInvariant()
if ($actual -ne $expected) { throw "Runner archive checksum mismatch" }

New-Item -ItemType Directory -Force C:\actions-runner\deskfuse-windows-x64 | Out-Null
Expand-Archive -LiteralPath $archive -DestinationPath C:\actions-runner\deskfuse-windows-x64 -Force
Set-Location C:\actions-runner\deskfuse-windows-x64
```

## 3. 获取 registration token

先检查：

```powershell
gh auth status
```

如果未登录，请在本机完成 `gh auth login`，不要索要或输出 GitHub 密码、PAT 或双因素验证码。

登录后只在内存变量中获取 token，绝对不要打印：

```powershell
$token = gh api --method POST `
  repos/GodD6366/DeskFuse/actions/runners/registration-token `
  --jq .token

if ([string]::IsNullOrWhiteSpace($token)) {
    throw "Failed to obtain GitHub Actions runner registration token"
}
```

## 4. 注册 Runner

使用：

- name：`deskfuse-windows-10-0-0-11`
- labels：`self-hosted`、`windows`、`x64`、`deskfuse-windows`、`windows-2022`

执行：

```powershell
.\config.cmd `
  --unattended `
  --replace `
  --url https://github.com/GodD6366/DeskFuse `
  --token $token `
  --name "deskfuse-windows-10-0-0-11" `
  --labels "self-hosted,windows,x64,deskfuse-windows,windows-2022" `
  --work "_work"

$token = $null
```

不要把 token 写入 `.env`、PowerShell 历史、日志或 Git。

## 5. 安装并启动服务

使用管理员 PowerShell：

```powershell
Set-Location C:\actions-runner\deskfuse-windows-x64
.\svc.cmd install
.\svc.cmd start
```

检查：

```powershell
Get-Service | Where-Object {
    $_.Name -like "actions.runner*" -or $_.DisplayName -like "*GitHub Actions*"
} | Format-Table Name, Status, StartType, DisplayName
```

服务必须是 `Running`，启动类型必须是自动。

## 6. 验证 GitHub 在线状态

```powershell
gh api repos/GodD6366/DeskFuse/actions/runners --jq '
.runners[] |
select(.name=="deskfuse-windows-10-0-0-11") |
{name,status,busy,os,labels:[.labels[].name]}
'
```

期望：

```text
name: deskfuse-windows-10-0-0-11
status: online
os: Windows
busy: false
labels 包含 self-hosted、Windows、X64、deskfuse-windows、windows-2022
```

如果是 `offline`，检查：

```text
C:\actions-runner\deskfuse-windows-x64\_diag
```

## 7. 验证 Windows 构建环境

在 DeskFuse 仓库目录执行：

```powershell
.\scripts\check-windows-native.ps1
.\scripts\check-windows-service.ps1
dotnet build apps\deskfuse-windows\DeskFuse.Windows.csproj -c Release
```

这些检查不应安装 Windows 服务、不应启动 DeskFuse GUI、不应读取真实剪贴板或发送真实输入。

如需打 Windows 安装包：

```powershell
.\scripts\snapshot-windows-source.ps1 -OutputPath "${env:TEMP}\deskfuse-source.json"

.\scripts\package-windows.ps1 `
  -SourceSnapshot "${env:TEMP}\deskfuse-source.json" `
  -ReleaseMetadata .\release.json `
  -OutputDirectory .\dist-windows
```

## 8. workflow 说明

当前 Windows job 使用：

```yaml
runs-on: windows-2022
```

由于 runner 添加了自定义 label `windows-2022`，现有 Windows job 可以匹配它。

如果要强制只使用这台自有 runner，可把以下两个文件中的 Windows job 改为：

```yaml
runs-on: [self-hosted, windows, x64, deskfuse-windows]
```

文件：

```text
.github/workflows/ci.yml
.github/workflows/release.yml
```

只修改 Windows job，不要修改 macOS 和 Ubuntu job。不要移动、删除或强推已经存在的 `v0.3.0` 标签。

最后报告：

- runner 名称
- runner 在线状态
- Windows 服务名称和状态
- labels
- 依赖检查结果
- 仍未验证的项目
```
