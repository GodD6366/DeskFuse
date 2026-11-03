param(
  [Parameter(Mandatory = $true)] [string]$PackageDirectory
)
$ErrorActionPreference = 'Stop'

if (-not ([Security.Principal.WindowsPrincipal] [Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  throw 'DeskFuse 麦克风驱动安装需要管理员权限。'
}

$package = (Resolve-Path -LiteralPath $PackageDirectory).Path
$manifestPath = Join-Path $package 'manifest.json'
$manifest = Get-Content -Raw -LiteralPath $manifestPath | ConvertFrom-Json
if ($manifest.schema -ne 1 -or [string]::IsNullOrWhiteSpace($manifest.driver_inf)) {
  throw '麦克风驱动包 manifest.json 无效。'
}
$inf = Join-Path $package $manifest.driver_inf
if (-not (Test-Path -LiteralPath $inf)) {
  throw "驱动包缺少 INF：$($manifest.driver_inf)"
}
if ($manifest.sha256) {
  $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $inf).Hash.ToLowerInvariant()
  if ($actual -ne $manifest.sha256.ToLowerInvariant()) {
    throw '麦克风驱动包校验失败。'
  }
}

& pnputil.exe /add-driver $inf /install
if ($LASTEXITCODE -ne 0) { throw "pnputil 安装失败，退出码 $LASTEXITCODE。" }
Restart-Service -Name Audiosrv -Force -ErrorAction SilentlyContinue
Write-Output 'DeskFuse Windows 麦克风驱动安装完成。'
