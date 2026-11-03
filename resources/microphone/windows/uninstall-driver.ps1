param(
  [Parameter(Mandatory = $true)] [string]$PublishedName
)
$ErrorActionPreference = 'Stop'

if (-not ([Security.Principal.WindowsPrincipal] [Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  throw 'DeskFuse 麦克风驱动卸载需要管理员权限。'
}

& pnputil.exe /delete-driver $PublishedName /uninstall /force
if ($LASTEXITCODE -ne 0) { throw "pnputil 卸载失败，退出码 $LASTEXITCODE。" }
Restart-Service -Name Audiosrv -Force -ErrorAction SilentlyContinue
Write-Output 'DeskFuse Windows 麦克风驱动卸载完成。'
