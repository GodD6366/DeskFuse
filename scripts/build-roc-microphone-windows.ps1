param(
  [Parameter(Mandatory = $true)] [string]$RocRoot,
  [string]$BuildDirectory = 'target/roc-microphone/windows'
)
$ErrorActionPreference = 'Stop'

$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$build = Join-Path $root $BuildDirectory
$roc = (Resolve-Path $RocRoot).Path
cmake -S (Join-Path $root 'apps/deskfuse-service/native') -B $build `
  -G 'Visual Studio 17 2022' -A x64 "-DROC_ROOT=$roc"
cmake --build $build --config Release --target deskfuse-roc-microphone
Write-Output "Windows Roc microphone sender: $build/Release/deskfuse-roc-microphone.dll"
