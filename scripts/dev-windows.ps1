# UI-only development loop. Uses the installed SCM service and existing user
# data; never installs, signs, or substitutes an input helper or service.
[CmdletBinding()]
param(
    [string]$ProjectRoot = '',
    [switch]$BuildOnly
)
$ErrorActionPreference = 'Stop'
if (!$ProjectRoot) { $ProjectRoot = Split-Path -Parent $PSScriptRoot }
$ProjectRoot = (Resolve-Path -LiteralPath $ProjectRoot).Path
$project = Join-Path $ProjectRoot 'apps\deskfuse-windows\DeskFuse.Windows.csproj'
$output = Join-Path $ProjectRoot 'dist\windows-dev'
$executable = Join-Path $output 'DeskFuse.Windows.exe'
$installed = Join-Path $env:ProgramFiles 'DeskFuse\DeskFuse.Windows.exe'
# Do not kill an unrelated checkout. Known instances must exit before publish
# because WinUI binaries and resources may be held open by the dev process.
$known = @(Get-Process -Name DeskFuse.Windows -ErrorAction SilentlyContinue | Where-Object {
    $_.Path -and $_.Path -eq $executable
})
foreach ($process in $known) {
    Stop-Process -Id $process.Id
    if (!$process.WaitForExit(10000)) { throw "Client did not exit: $($process.Id)" }
}
$args = @('publish', $project, '-c', 'Debug', '-r', 'win-x64', '--self-contained', 'true', '-o', $output, '-p:ApplicationManifest=app.store.manifest')
$descriptor = Join-Path $ProjectRoot 'release.json'
if (Test-Path -LiteralPath $descriptor) {
    $release = Get-Content -LiteralPath $descriptor -Raw | ConvertFrom-Json
    $args += @("-p:Version=$($release.version)", "-p:FileVersion=$($release.version).$($release.build_number)")
}
$ErrorActionPreference = 'Continue'
& dotnet @args
$exitCode = $LASTEXITCODE
$ErrorActionPreference = 'Stop'
if ($exitCode -ne 0) { throw "Windows dev client build failed (exit $exitCode); installed files and service are unchanged." }
if (Test-Path -LiteralPath $descriptor) { Copy-Item -LiteralPath $descriptor -Destination (Join-Path $output 'release.json') -Force }
Write-Output "Debug client: $executable"
Write-Output 'UI-only: uses the installed DeskFuseService and existing data. No service/helper deployment or input permission bypass. Re-run after edits; this is not hot reload.'
if (!$BuildOnly) {
    if (!(Get-Service -Name DeskFuseService -ErrorAction SilentlyContinue)) {
        throw 'DeskFuseService is not installed. Install the matching desktop package first.'
    }
    Get-Process -Name DeskFuse.Windows -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $installed } | ForEach-Object {
        Stop-Process -Id $_.Id
        if (!$_.WaitForExit(10000)) { throw "Installed client did not exit: $($_.Id)" }
    }
    Start-Process -FilePath $executable -WorkingDirectory $output
}
