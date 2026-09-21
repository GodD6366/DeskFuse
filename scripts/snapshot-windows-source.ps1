# Create the JSON input required by package-windows.ps1, including untracked source.
[CmdletBinding()]
param([string]$ProjectRoot = '', [Parameter(Mandatory=$true)][string]$OutputPath)
$ErrorActionPreference = 'Stop'
if (!$ProjectRoot) { $ProjectRoot = Split-Path -Parent $PSScriptRoot }
Push-Location $ProjectRoot
try {
    $paths = & git ls-files --cached --others --exclude-standard -- Cargo.toml Cargo.lock rust-toolchain.toml release.json .cargo resources crates apps/deskfuse-service apps/deskfuse-windows scripts/build-windows.ps1 scripts/build-windows-desktop.ps1 scripts/package-windows.ps1 scripts/snapshot-windows-source.ps1 installer/windows
    if ($LASTEXITCODE -ne 0) { throw 'Cannot enumerate source snapshot' }
    $paths = @($paths | Where-Object { $_ -notmatch '/(bin|obj|target)/' } | Sort-Object -Unique)
    if (!$paths.Count) { throw 'No source files found' }
    $snapshot = @($paths | ForEach-Object { [ordered]@{ path=$_; sha256=(Get-FileHash -Algorithm SHA256 -LiteralPath $_).Hash.ToLowerInvariant() } })
    ConvertTo-Json -InputObject $snapshot -Depth 3 | Set-Content -LiteralPath $OutputPath -Encoding UTF8
    Get-FileHash -LiteralPath $OutputPath -Algorithm SHA256
} finally { Pop-Location }
