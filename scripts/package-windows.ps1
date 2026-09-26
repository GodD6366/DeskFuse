# Offline-capable x64 desktop installer. Packaging never tests, installs or launches.
[CmdletBinding()]
param(
    [string]$ProjectRoot = '',
    [string]$ReleaseMetadata = '',
    [Parameter(Mandatory=$true)][string]$SourceSnapshot,
    [string]$OutputDirectory = '',
    [string]$MakeNSIS = '',
    [switch]$Offline
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

$SourceSnapshot = (Resolve-Path -LiteralPath $SourceSnapshot).Path
if (!$OutputDirectory) { $OutputDirectory = Join-Path $ProjectRoot 'dist\windows' }
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
if (!$MakeNSIS) {
    $command = Get-Command makensis.exe -ErrorAction SilentlyContinue
    if ($command) { $MakeNSIS = $command.Source } else { $MakeNSIS = "${env:ProgramFiles(x86)}\NSIS\makensis.exe" }
}
if (!(Test-Path -LiteralPath $MakeNSIS -PathType Leaf)) { throw 'NSIS 3 is required; install it before packaging.' }
$sourceBytesHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $SourceSnapshot).Hash
$source = @(Get-Content -Raw -LiteralPath $SourceSnapshot | ConvertFrom-Json)
if (!$source.Count) { throw 'Empty source snapshot' }
$rootPrefix = $ProjectRoot.TrimEnd('\') + '\'
$paths = @{}
foreach ($entry in $source) {
    if ([IO.Path]::IsPathRooted($entry.path) -or $entry.path -match '(^|[\\/])\.\.([\\/]|$)' -or $entry.sha256 -notmatch '^[a-fA-F0-9]{64}$') { throw 'Invalid source snapshot entry' }
    $full = [IO.Path]::GetFullPath((Join-Path $ProjectRoot $entry.path))
    if (!$full.StartsWith($rootPrefix, [StringComparison]::OrdinalIgnoreCase)) { throw 'Source path outside checkout' }
    if ($paths.ContainsKey($full)) { throw "Duplicate source path: $($entry.path)" }
    $paths[$full] = $entry.sha256
}
function Assert-Snapshot {
    if ((Get-FileHash -Algorithm SHA256 -LiteralPath $ReleaseMetadata).Hash -ne $releaseHash) { throw 'Shared release descriptor changed during packaging' }
    if ((Get-FileHash -Algorithm SHA256 -LiteralPath $SourceSnapshot).Hash -ne $sourceBytesHash) { throw 'Snapshot itself changed during build' }
    foreach ($path in $paths.Keys) {
        if ((Get-FileHash -Algorithm SHA256 -LiteralPath $path).Hash -ne $paths[$path]) { throw "Source changed: $path" }
    }
    # Reject omitted or newly-added build inputs, not just mismatches in supplied entries.
    $required = & git -C $ProjectRoot ls-files --cached --others --exclude-standard -- Cargo.toml Cargo.lock rust-toolchain.toml release.json .cargo resources crates apps/unidesk-service apps/unidesk-windows scripts/build-windows.ps1 scripts/build-windows-desktop.ps1 scripts/package-windows.ps1 scripts/snapshot-windows-source.ps1 installer/windows
    if ($LASTEXITCODE -ne 0) { throw 'Cannot enumerate source inputs' }
    foreach ($path in $required) {
        if ($path -match '/(bin|obj|target)/') { continue }
        if (!$paths.ContainsKey([IO.Path]::GetFullPath((Join-Path $ProjectRoot $path)))) { throw "Build input omitted from snapshot: $path" }
    }
}
function Assert-X64PE([string]$Path) {
    $stream = [IO.File]::OpenRead($Path)
    try {
        $reader = New-Object IO.BinaryReader($stream)
        if ($reader.ReadUInt16() -ne 0x5a4d) { throw "Missing MZ header: $Path" }
        $stream.Position = 0x3c
        $stream.Position = $reader.ReadUInt32()
        if ($reader.ReadUInt32() -ne 0x00004550 -or $reader.ReadUInt16() -ne 0x8664) { throw "Expected PE x64: $Path" }
    } finally { $stream.Dispose() }
}
function Assert-ExecutableImports([string]$Path, [string]$PayloadDirectory) {
    # dumpbin is provided by the same MSVC toolchain used above. /DEPENDENTS lists
    # ordinary and delay-load imports. Never accept a DLL just because it exists
    # on this developer's machine, and never copy DLLs out of System32 or VS.
    $dumpbin = Get-Command dumpbin.exe -ErrorAction Stop
    $output = @(& $dumpbin.Source /NOLOGO /DEPENDENTS $Path 2>&1)
    if ($LASTEXITCODE -ne 0) { throw "PE import inspection failed: $Path" }
    $imports = @($output | ForEach-Object {
        if ([string]$_ -match '^\s+([A-Za-z0-9_.-]+\.(?:dll|drv))\s*$') { $Matches[1].ToLowerInvariant() }
    } | Sort-Object -Unique)
    if (!$imports.Count) { throw "No PE imports parsed; refusing an unverified executable: $Path" }
    # Inbox Windows 11 system libraries only. New non-system dependencies must be
    # explicitly deployed beside the app. API-set names are OS loader contracts.
    $systemDlls = @('advapi32.dll','bcrypt.dll','bcryptprimitives.dll','cfgmgr32.dll',
        'combase.dll','comctl32.dll','comdlg32.dll','crypt32.dll','cryptbase.dll','cryptsp.dll',
        'd2d1.dll','d3d9.dll','d3d11.dll','d3d12.dll','dcomp.dll','dbghelp.dll','dnsapi.dll',
        'dwmapi.dll','dwrite.dll','dxgi.dll','gdi32.dll','gdi32full.dll','glu32.dll','imm32.dll',
        'iphlpapi.dll','kernel32.dll','kernelbase.dll','mpr.dll','msimg32.dll','msvcrt.dll',
        'mswsock.dll','ncrypt.dll','netapi32.dll','normaliz.dll','ntdll.dll','ole32.dll',
        'oleaut32.dll','opengl32.dll','pdh.dll','powrprof.dll','propsys.dll','psapi.dll',
        'rpcrt4.dll','rstrtmgr.dll','sechost.dll','secur32.dll','setupapi.dll','shell32.dll',
        'shlwapi.dll','sspicli.dll','ucrtbase.dll','user32.dll','userenv.dll','usp10.dll',
        'uxtheme.dll','version.dll','winhttp.dll','wininet.dll','winmm.dll','winscard.dll',
        'winspool.drv','wldap32.dll','ws2_32.dll','wtsapi32.dll')
    foreach ($dependency in $imports) {
        $deployed = Join-Path $PayloadDirectory $dependency
        if (Test-Path -LiteralPath $deployed -PathType Leaf) { continue }
        # The VC redistributable is NOT an inbox dependency even if this build host
        # happens to have it. vcruntime140_cor3.dll is not a substitute for vcruntime140.dll.
        if ($dependency -match '^(vcruntime|msvcp|msvcr|concrt|vcomp|mfc)\d.*\.dll$') { throw "Missing packaged VC runtime $dependency imported by $Path; rebuild Rust with static CRT" }
        if ($dependency -match '^(api-ms-win-|ext-ms-win-).+\.dll$' -or $dependency -in $systemDlls) { continue }
        throw "Missing non-system DLL $dependency imported by $Path"
    }
    return ,$imports
}

Assert-Snapshot
if ($release.version -notmatch '^\d+\.\d+\.\d+$' -or $release.build_number -lt 0 -or $release.build_number -gt 65535) { throw 'Invalid Windows release version/build' }
foreach ($part in $release.version.Split('.')) { if ([int]$part -gt 65535) { throw 'Version component exceeds Windows limit' } }
$stage = Join-Path ([IO.Path]::GetTempPath()) ('unidesk-windows-package-' + [guid]::NewGuid().ToString('N'))
$payload = Join-Path $stage 'payload'
[IO.Directory]::CreateDirectory($payload) | Out-Null
try {
    & (Join-Path $ProjectRoot 'scripts\build-windows-desktop.ps1') -ProjectRoot $ProjectRoot -ReleaseMetadata $ReleaseMetadata -OutputDirectory $payload -Offline:$Offline
    if (!$?) { throw 'Desktop build failed' }
    Assert-Snapshot
    $executableImports = [ordered]@{}
    foreach ($name in @('UniDesk.Windows.exe', 'unidesk-service.exe', 'unidesk-service-client.exe', 'unidesk-input-helper.exe', 'unidesk-update-helper.exe')) {
        Assert-X64PE (Join-Path $payload $name)
        $executableImports[$name] = Assert-ExecutableImports (Join-Path $payload $name) $payload
    }
    # A self-contained deployment must actually contain its runtime, not just an apphost.
    foreach ($name in @('coreclr.dll', 'hostfxr.dll', 'UniDesk.Windows.runtimeconfig.json', 'UniDesk.Windows.dll')) {
        if (!(Test-Path -LiteralPath (Join-Path $payload $name) -PathType Leaf)) { throw "Incomplete self-contained payload: $name" }
    }
    Copy-Item -LiteralPath $SourceSnapshot -Destination (Join-Path $payload 'source-snapshot.json')
    Copy-Item -LiteralPath (Join-Path $ProjectRoot 'installer\windows\README.md') -Destination (Join-Path $payload 'INSTALLATION.md')
    $artifacts = @()
    $deletes = @()
    foreach ($file in Get-ChildItem -LiteralPath $payload -File -Recurse) {
        $relative = $file.FullName.Substring($payload.Length + 1)
        if ($relative -notmatch '^[A-Za-z0-9_.\\-]+$') { throw "Unexpected payload filename: $relative" }
        $signature = if ($file.Extension -in @('.exe', '.dll')) { (Get-AuthenticodeSignature -LiteralPath $file.FullName).Status.ToString() } else { 'NotApplicable' }
        $artifacts += [ordered]@{ path=$relative; bytes=$file.Length; sha256=(Get-FileHash -Algorithm SHA256 -LiteralPath $file.FullName).Hash.ToLowerInvariant(); authenticode=$signature }
        $deletes += 'Delete "$INSTDIR\' + $relative + '"'
    }
    foreach ($dir in Get-ChildItem -LiteralPath $payload -Directory -Recurse | Sort-Object { $_.FullName.Length } -Descending) { $deletes += 'RMDir "$INSTDIR\' + $dir.FullName.Substring($payload.Length + 1) + '"' }
    $deleteInclude = Join-Path $stage 'delete-payload.nsh'
    $deletes | Set-Content -LiteralPath $deleteInclude -Encoding UTF8
    $installer = Join-Path $stage "uniDesk-$($release.version)-build$($release.build_number)-windows-x64-setup.exe"
    & $MakeNSIS '/V3' "/DPAYLOAD_DIR=$payload" "/DOUTPUT_FILE=$installer" "/DDELETE_INCLUDE=$deleteInclude" "/DVERSION=$($release.version)" "/DBUILD_NUMBER=$($release.build_number)" (Join-Path $ProjectRoot 'installer\windows\uniDesk.nsi')
    if ($LASTEXITCODE -ne 0 -or !(Test-Path -LiteralPath $installer)) { throw 'NSIS packaging failed' }
    Assert-Snapshot
    $installerSignature = (Get-AuthenticodeSignature -LiteralPath $installer).Status.ToString()
    $installerHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $installer).Hash.ToLowerInvariant()
    # This metadata is descriptive only. LAN eligibility remains false until an
    # external release manifest signs the installer and the service-owned helper
    # admission/self-test protocol is enabled.
    $manifest = [ordered]@{ schema_version=1; kind='windows-desktop-offline-installer'; version=$release.version; build_number=$release.build_number; release_id=$release.release_id; target='win-x64'; source_snapshot_sha256=$sourceBytesHash.ToLowerInvariant(); source_file_count=$source.Count; created_at=[DateTime]::UtcNow.ToString('o'); is_installer=$true; release_signed=$false; lan_update_eligible=$false; updater_helper_present=$true; updater_helper_enabled=$false; authenticode=$installerSignature; installer_sha256=$installerHash; installer_bytes=(Get-Item -LiteralPath $installer).Length; payload=$artifacts; executable_imports=$executableImports; verification=@{ build=$true; pe_x64=$true; executable_imports=$true; installation=$false; functional_tests=$false; two_machine=$false; updater_admission=$false; updater_self_test=$false; updater_rollback=$false } }
    [IO.Directory]::CreateDirectory($OutputDirectory) | Out-Null
    $destination = Join-Path $OutputDirectory ([IO.Path]::GetFileName($installer))
    Copy-Item -LiteralPath $installer -Destination $destination -Force
    if ((Get-FileHash -Algorithm SHA256 -LiteralPath $destination).Hash.ToLowerInvariant() -ne $installerHash) { throw 'Output copy hash mismatch' }
    $manifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath ($destination + '.manifest.json') -Encoding UTF8
    "$installerHash  $([IO.Path]::GetFileName($destination))" | Set-Content -LiteralPath ($destination + '.sha256') -Encoding ASCII
    Write-Output "Installer: $destination"
    Write-Output "SHA-256: $installerHash; Authenticode: $installerSignature; LAN updater eligible: false"
    Write-Output 'Built and packaged only. Installation, GUI, Explorer, input and two-machine behavior remain unverified.'
} finally { if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force } }
