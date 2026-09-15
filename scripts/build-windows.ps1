# Compatibility entry point for the native WPF desktop and Rust service build.
[CmdletBinding()]
param([string]$ProjectRoot = '', [string]$OutputDirectory = '', [switch]$Offline)
$ErrorActionPreference = 'Stop'
& (Join-Path $PSScriptRoot 'build-windows-desktop.ps1') -ProjectRoot $ProjectRoot -OutputDirectory $OutputDirectory -Offline:$Offline
