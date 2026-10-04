# Build only: never tests, installs, starts a service, or launches the desktop app.
[CmdletBinding()]
param(
    [string]$ProjectRoot = '',
    [string]$ReleaseMetadata = '',
    [string]$OutputDirectory = '',
    [switch]$Offline,
    # Only meaningful for a signed, trusted-location build: an unsigned uiAccess
    # binary is rejected by Windows at launch (Win32 740).
    [switch]$UiAccess
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (!$ProjectRoot) { $ProjectRoot = Split-Path -Parent $PSScriptRoot }
$ProjectRoot = (Resolve-Path -LiteralPath $ProjectRoot).Path
# Use the paired macOS release, never silently fall back to the bootstrap build 8.
if (!$ReleaseMetadata) { $ReleaseMetadata = Join-Path $ProjectRoot 'dist\release.json' }
if (!(Test-Path -LiteralPath $ReleaseMetadata -PathType Leaf)) { throw 'Pass -ReleaseMetadata with the shared macOS release descriptor.' }
$ReleaseMetadata = (Resolve-Path -LiteralPath $ReleaseMetadata).Path
$releaseHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $ReleaseMetadata).Hash
$release = Get-Content -Raw -LiteralPath $ReleaseMetadata | ConvertFrom-Json
if ($release.version -notmatch '^\d+\.\d+\.\d+$' -or $release.build_number -lt 1 -or $release.build_number -gt 9999 -or [guid]$release.release_id -eq [guid]::Empty) { throw 'Invalid shared release descriptor' }

if (!$OutputDirectory) { $OutputDirectory = Join-Path $ProjectRoot 'dist\windows-desktop' }
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
if (!(Test-Path -LiteralPath $vswhere)) { throw 'Install Visual Studio Build Tools with MSVC x64 and Windows SDK first.' }
$vs = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (!$vs) { throw 'MSVC x64 tools missing' }
Import-Module (Join-Path $vs 'Common7\Tools\Microsoft.VisualStudio.DevShell.dll')
Enter-VsDevShell -VsInstallPath $vs -SkipAutomaticLocation -DevCmdArguments '-arch=x64 -host_arch=x64'
$previousTarget = $env:CARGO_TARGET_DIR
$previousRustFlags = [Environment]::GetEnvironmentVariable('RUSTFLAGS', 'Process')
$previousEncodedRustFlags = [Environment]::GetEnvironmentVariable('CARGO_ENCODED_RUSTFLAGS', 'Process')
$env:CARGO_TARGET_DIR = Join-Path $ProjectRoot 'target'
Push-Location $ProjectRoot
try {
    # --target confines these flags to target crates, including all Rust dependencies.
    # Cargo gives ENCODED precedence over RUSTFLAGS: append to the effective flags,
    # rather than setting RUSTFLAGS where an existing ENCODED value could bypass it.
    $separator = [string][char]0x1f
    if ($null -ne $previousEncodedRustFlags) {
        $effectiveFlags = $previousEncodedRustFlags
    } elseif (![string]::IsNullOrWhiteSpace($previousRustFlags)) {
        # Cargo itself splits RUSTFLAGS on whitespace, not shell quotation syntax.
        $effectiveFlags = (($previousRustFlags.Trim() -split '\s+') -join $separator)
    } else { $effectiveFlags = '' }
    $staticCrtFlags = '-C' + $separator + 'target-feature=+crt-static'
    $env:CARGO_ENCODED_RUSTFLAGS = if ($effectiveFlags.Length) { $effectiveFlags + $separator + $staticCrtFlags } else { $staticCrtFlags }
    $cargoArgs = @('build', '--locked', '--release', '--target', 'x86_64-pc-windows-msvc', '-p', 'deskfuse-service', '--bins')
    if ($Offline) { $cargoArgs += '--offline' }
    # PowerShell 5.1 turns native stderr output into a terminating error under
    # ErrorActionPreference=Stop, and cargo/dotnet write progress to stderr.
    # Relax it for each native call and keep the explicit exit-code checks.
    $ErrorActionPreference = 'Continue'
    & cargo @cargoArgs
    $ErrorActionPreference = 'Stop'
    if ($LASTEXITCODE -ne 0) { throw 'Service Release build failed' }
    $helperArgs = @('build', '--locked', '--release', '--target', 'x86_64-pc-windows-msvc', '-p', 'deskfuse-update-platform', '--bin', 'deskfuse-update-helper')
    if ($Offline) { $helperArgs += '--offline' }
    $ErrorActionPreference = 'Continue'
    & cargo @helperArgs
    $ErrorActionPreference = 'Stop'
    if ($LASTEXITCODE -ne 0) { throw 'Windows updater helper build failed' }
    $publishArgs = @('publish', 'apps\deskfuse-windows\DeskFuse.Windows.csproj', '-c', 'Release', '-r', 'win-x64', '--self-contained', 'true', '-o', $OutputDirectory, '-p:PublishSingleFile=false', '-p:PublishTrimmed=false', "-p:Version=$($release.version)", "-p:FileVersion=$($release.version).$($release.build_number)", "-p:InformationalVersion=$($release.version)+build.$($release.build_number)")
    # WinUI 3 client: the default manifest is the packaged/Store variant (no
    # uiAccess) so the unsigned offline payload still launches.
    $publishArgs += if ($UiAccess) { '-p:ApplicationManifest=app.manifest' } else { '-p:ApplicationManifest=app.store.manifest' }
    # Offline builds require a prior restore of the exact project/runtime on this host.
    if ($Offline) { $publishArgs += '--no-restore' }
    $ErrorActionPreference = 'Continue'
    & dotnet @publishArgs
    $ErrorActionPreference = 'Stop'
    if ($LASTEXITCODE -ne 0) { throw 'Windows desktop publish failed' }
    foreach ($name in @('deskfuse-service.exe', 'deskfuse-service-client.exe', 'deskfuse-input-helper.exe', 'deskfuse-update-helper.exe')) {
        Copy-Item -LiteralPath (Join-Path $ProjectRoot "target\x86_64-pc-windows-msvc\release\$name") -Destination $OutputDirectory -Force
    }
    if ((Get-FileHash -Algorithm SHA256 -LiteralPath $ReleaseMetadata).Hash -ne $releaseHash) { throw 'Shared release descriptor changed during build' }
    Copy-Item -LiteralPath $ReleaseMetadata -Destination (Join-Path $OutputDirectory 'release.json') -Force
    Write-Output "Release payload: $OutputDirectory. Build only; no functional tests or installation performed."
} finally {
    $env:CARGO_TARGET_DIR = $previousTarget
    [Environment]::SetEnvironmentVariable('RUSTFLAGS', $previousRustFlags, 'Process')
    [Environment]::SetEnvironmentVariable('CARGO_ENCODED_RUSTFLAGS', $previousEncodedRustFlags, 'Process')
    Pop-Location
}
