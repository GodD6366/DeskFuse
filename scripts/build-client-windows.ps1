param(
    [switch]$UseSdk
)
$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root
if (-not $UseSdk) {
    cargo build --locked --package deskfuse-collaboration --package deskfuse-input-windows
}
Write-Host "Windows host sources live in apps/deskfuse-windows/DeskFuse.Host"
