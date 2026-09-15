# Build/test service boundary without installing a service or creating machine keys.
param([string]$ProjectRoot = '')
$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrWhiteSpace($ProjectRoot)) { $ProjectRoot = Split-Path -Parent $PSScriptRoot }
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$vs = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (-not $vs) { throw 'MSVC x64 tools missing' }
Import-Module (Join-Path $vs 'Common7\Tools\Microsoft.VisualStudio.DevShell.dll')
Enter-VsDevShell -VsInstallPath $vs -SkipAutomaticLocation -DevCmdArguments '-arch=x64 -host_arch=x64'
Set-Location $ProjectRoot
& rustc -vV
if ($LASTEXITCODE -ne 0) { throw 'Pinned Rust unavailable' }
& cargo test --locked --target x86_64-pc-windows-msvc -p unidesk-service -p unidesk-identity --all-targets
if ($LASTEXITCODE -ne 0) { throw 'Service tests failed' }
& cargo clippy --locked --target x86_64-pc-windows-msvc -p unidesk-service -p unidesk-identity --all-targets --no-deps -- -D warnings
if ($LASTEXITCODE -ne 0) { throw 'Service lint failed' }
& cargo build --locked --release --target x86_64-pc-windows-msvc -p unidesk-service --bins
if ($LASTEXITCODE -ne 0) { throw 'Service release build failed' }
foreach ($name in @('unidesk-service.exe', 'unidesk-service-client.exe')) {
    Get-FileHash (Join-Path $ProjectRoot "target\x86_64-pc-windows-msvc\release\$name") -Algorithm SHA256
}
Write-Output 'Service/client build checks passed. No service was installed or started; GUI/helper/installer are not included.'
