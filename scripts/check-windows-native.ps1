# Native adapter verification only. Does not install a service, create an identity,
# read clipboard contents, or send real input. Not a complete app/installer build.
param([string]$ProjectRoot = (Split-Path $PSScriptRoot -Parent))
$ErrorActionPreference = 'Stop'
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
if (-not (Test-Path $vswhere)) { throw 'Run bootstrap-windows.ps1 first' }
$vs = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (-not $vs) { throw 'MSVC x64 tools missing' }
Import-Module (Join-Path $vs 'Common7\Tools\Microsoft.VisualStudio.DevShell.dll')
Enter-VsDevShell -VsInstallPath $vs -SkipAutomaticLocation -DevCmdArguments '-arch=x64 -host_arch=x64'
Set-Location $ProjectRoot
$env:CARGO_NET_GIT_FETCH_WITH_CLI = 'true'
& rustc -vV
if ($LASTEXITCODE -ne 0) { throw 'Pinned Rust unavailable' }
& cargo test --locked --target x86_64-pc-windows-msvc -p unidesk-platform -p unidesk-platform-windows -p unidesk-identity --lib
if ($LASTEXITCODE -ne 0) { throw 'Windows native adapter tests failed' }
& cargo test --locked --target x86_64-pc-windows-msvc -p unidesk-transfer windows_paths --lib
if ($LASTEXITCODE -ne 0) { throw 'Windows filename validation tests failed' }
& cargo clippy --locked --target x86_64-pc-windows-msvc -p unidesk-platform -p unidesk-platform-windows -p unidesk-identity --all-targets -- -D warnings
if ($LASTEXITCODE -ne 0) { throw 'Windows native adapter lint failed' }
& cargo build --locked --release --target x86_64-pc-windows-msvc -p unidesk-platform-windows --bin unidesk-native-probe
if ($LASTEXITCODE -ne 0) { throw 'Windows native probe build failed' }
$exe = Join-Path $ProjectRoot 'target\x86_64-pc-windows-msvc\release\unidesk-native-probe.exe'
Get-FileHash $exe -Algorithm SHA256
Write-Output 'Native adapter checks passed. This executable is a read-only probe, not the uniDesk application or installer.'
