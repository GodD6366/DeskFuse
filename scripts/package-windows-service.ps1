# Explicit development checkpoint package: service + metadata client, NOT a GUI installer.
param(
    [string]$ProjectRoot = '',
    [Parameter(Mandatory=$true)][string]$SourceSnapshot
)
$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrWhiteSpace($ProjectRoot)) { $ProjectRoot = Split-Path -Parent $PSScriptRoot }
Set-Location $ProjectRoot
$root = [IO.Path]::GetFullPath($ProjectRoot).TrimEnd('\') + '\'
$source = Get-Content -Raw -LiteralPath $SourceSnapshot | ConvertFrom-Json
if ($source.Count -lt 1) { throw 'Empty source snapshot' }
foreach ($entry in $source) {
    $full = [IO.Path]::GetFullPath((Join-Path $ProjectRoot $entry.path))
    if (-not $full.StartsWith($root, [StringComparison]::OrdinalIgnoreCase)) { throw 'Source path outside checkout' }
    if ((Get-FileHash -Algorithm SHA256 -LiteralPath $full).Hash.ToLowerInvariant() -ne $entry.sha256) { throw "Source mismatch: $($entry.path)" }
}
# Always rebuild from the just-verified source. No stale binaries accepted.
& (Join-Path $ProjectRoot 'scripts\check-windows-service.ps1') -ProjectRoot $ProjectRoot
if (-not $?) { throw 'Service checks failed' }
# Revalidate after build to reject a checkout changed while compiling.
foreach ($entry in $source) {
    if ((Get-FileHash -Algorithm SHA256 -LiteralPath $entry.path).Hash.ToLowerInvariant() -ne $entry.sha256) { throw "Source changed during build: $($entry.path)" }
}
$stage = Join-Path ([IO.Path]::GetTempPath()) ('unidesk-service-package-' + [guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($stage) | Out-Null
try {
    $artifacts = @()
    foreach ($name in @('unidesk-service.exe','unidesk-service-client.exe')) {
        $file = Join-Path $ProjectRoot "target\x86_64-pc-windows-msvc\release\$name"
        $stream = [IO.File]::OpenRead($file)
        try {
            $reader = New-Object IO.BinaryReader($stream)
            if ($reader.ReadUInt16() -ne 0x5a4d) { throw 'Missing MZ header' }
            $stream.Position = 0x3c
            $pe = $reader.ReadUInt32()
            $stream.Position = $pe
            if ($reader.ReadUInt32() -ne 0x00004550 -or $reader.ReadUInt16() -ne 0x8664) { throw 'Expected PE x64 executable' }
        } finally { $stream.Dispose() }
        $signature = Get-AuthenticodeSignature -LiteralPath $file
        $artifacts += [ordered]@{ name=$name; bytes=(Get-Item $file).Length;
            sha256=(Get-FileHash -Algorithm SHA256 -LiteralPath $file).Hash.ToLowerInvariant();
            authenticode=$signature.Status.ToString() }
        Copy-Item -LiteralPath $file -Destination (Join-Path $stage $name)
    }
    Copy-Item -LiteralPath (Join-Path $ProjectRoot 'docs\WINDOWS_SERVICE_CHECKPOINT.md') -Destination (Join-Path $stage 'README.md')
    Copy-Item -LiteralPath $SourceSnapshot -Destination (Join-Path $stage 'source-snapshot.json')
    [ordered]@{ schema_version=1; kind='development-service-checkpoint'; target='x86_64-pc-windows-msvc';
        is_installer=$false; release_signed=$false; created_at=[DateTime]::UtcNow.ToString('o');
        source_snapshot_sha256=(Get-FileHash -Algorithm SHA256 -LiteralPath $SourceSnapshot).Hash.ToLowerInvariant();
        source_file_count=$source.Count; artifacts=$artifacts } |
        ConvertTo-Json -Depth 6 | Set-Content -Encoding UTF8 -LiteralPath (Join-Path $stage 'manifest.json')
    $out = Join-Path $ProjectRoot 'dist\windows-service'
    [IO.Directory]::CreateDirectory($out) | Out-Null
    $archive = Join-Path $out 'uniDesk-windows-service-x64.zip'
    Compress-Archive -LiteralPath @((Join-Path $stage 'unidesk-service.exe'), (Join-Path $stage 'unidesk-service-client.exe'),
        (Join-Path $stage 'README.md'), (Join-Path $stage 'source-snapshot.json'), (Join-Path $stage 'manifest.json')) -DestinationPath $archive -Force
    Get-FileHash -Algorithm SHA256 -LiteralPath $archive
    Write-Output 'Development service checkpoint packaged. NOT an installer; no application/service was installed.'
} finally {
    # This directory was created with a unique name by this invocation only.
    Remove-Item -LiteralPath $stage -Recurse
}
