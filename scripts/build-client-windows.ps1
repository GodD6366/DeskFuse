param(
    [switch]$UseSdk
)
$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root
if (-not $UseSdk) {
    cargo build --locked --package unidesk-collaboration --package unidesk-input-windows
}
Write-Host "Windows host sources live in apps/unidesk-windows/UniDesk.Host"
