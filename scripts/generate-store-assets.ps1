# Generate the Microsoft Store / MSIX image assets from the existing app mark.
#
# The macOS pipeline renders resources/generated/DeskFuse-icon-1024.png from
# resources/DeskFuse-icon.svg with sips. This Windows-side generator prefers that
# PNG when present, otherwise rasterizes the same SVG through Edge headless, and
# falls back to the largest ICO frame only to keep builds unblocked. No artwork is
# redesigned and no color theme is guessed.
[CmdletBinding()]
param(
    [string]$ProjectRoot = '',
    [string]$SourcePng = '',
    [string]$AssetsDirectory = '',
    [string]$IconPath = '',
    [int]$SourceSize = 1024
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
Add-Type -AssemblyName System.Drawing

if (!$ProjectRoot) { $ProjectRoot = Split-Path -Parent $PSScriptRoot }
$ProjectRoot = (Resolve-Path -LiteralPath $ProjectRoot).Path
if (!$SourcePng) { $SourcePng = Join-Path $ProjectRoot 'resources\generated\DeskFuse-icon-1024.png' }
if (!$AssetsDirectory) { $AssetsDirectory = Join-Path $ProjectRoot 'apps\deskfuse-windows\Assets' }
if (!$IconPath) { $IconPath = Join-Path $AssetsDirectory 'DeskFuse.ico' }
$svg = Join-Path $ProjectRoot 'resources\DeskFuse-icon.svg'
$temporary = Join-Path ([IO.Path]::GetTempPath()) ('deskfuse-store-assets-' + [guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($temporary) | Out-Null

function Find-Edge {
    foreach ($candidate in @(
            "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe",
            "$env:ProgramFiles\Microsoft\Edge\Application\msedge.exe",
            "$env:LOCALAPPDATA\Microsoft\Edge\Application\msedge.exe")) {
        if (Test-Path -LiteralPath $candidate -PathType Leaf) { return $candidate }
    }
    return $null
}

function Render-Svg([string]$Path, [string]$Destination, [int]$Size) {
    $edge = Find-Edge
    if (!$edge) { return $false }
    $url = 'file:///' + ($Path -replace '\\', '/' -replace ' ', '%20')
    for ($attempt = 1; $attempt -le 2; $attempt++) {
        Remove-Item -LiteralPath $Destination -Force -ErrorAction SilentlyContinue
        # Start-Process keeps Edge's stderr out of the PowerShell error stream;
        # a native stderr write would otherwise terminate under ErrorActionPreference=Stop.
        Start-Process -FilePath $edge -WindowStyle Hidden -Wait -ArgumentList @(
            '--headless=new', '--disable-gpu', '--hide-scrollbars', '--default-background-color=00000000',
            "--window-size=$Size,$Size", "--screenshot=$Destination", $url) | Out-Null
        if ((Test-Path -LiteralPath $Destination -PathType Leaf) -and (Get-Item -LiteralPath $Destination).Length -gt 0) { return $true }
        Start-Sleep -Milliseconds 400
    }
    return $false
}

$sourceKind = 'resources/generated/DeskFuse-icon-1024.png'
$sourcePath = $SourcePng
if (!(Test-Path -LiteralPath $sourcePath -PathType Leaf)) {
    $rendered = Join-Path $temporary 'DeskFuse-icon-1024.png'
    if ((Test-Path -LiteralPath $svg -PathType Leaf) -and (Render-Svg -Path $svg -Destination $rendered -Size $SourceSize)) {
        $sourcePath = $rendered
        $sourceKind = 'resources/DeskFuse-icon.svg (Edge headless raster)'
    } else {
        $sourcePath = $IconPath
        $sourceKind = 'apps/deskfuse-windows/Assets/DeskFuse.ico (fallback frame)'
        Write-Warning 'Store assets are derived from the ICO fallback; regenerate from resources/generated/DeskFuse-icon-1024.png.'
    }
}

try {
    if ($sourcePath -like '*.ico') {
        $icon = New-Object System.Drawing.Icon($sourcePath, 256, 256)
        try { $artwork = $icon.ToBitmap() } finally { $icon.Dispose() }
    } else {
        $artwork = [System.Drawing.Image]::FromFile($sourcePath)
    }
    if ($artwork.Width -ne $artwork.Height) { throw "App mark must be square: $sourcePath" }

    # Keep the artwork inside a transparent tile; the mark already carries its own
    # ice-white tile, so wide/splash surfaces only need horizontal padding.
    $targets = @(
        @{ Name = 'StoreLogo.png'; Width = 50; Height = 50; Padding = 0.12 },
        @{ Name = 'Square44x44Logo.png'; Width = 44; Height = 44; Padding = 0.12 },
        @{ Name = 'Square150x150Logo.png'; Width = 150; Height = 150; Padding = 0.12 },
        @{ Name = 'Wide310x150Logo.png'; Width = 310; Height = 150; Padding = 0.12 },
        @{ Name = 'SplashScreen.png'; Width = 620; Height = 300; Padding = 0.14 },
        @{ Name = 'StoreListingLogo-300x300.png'; Width = 300; Height = 300; Padding = 0.10 },
        @{ Name = 'Square44x44Logo.targetsize-256.png'; Width = 256; Height = 256; Padding = 0.10 }
    )
    [IO.Directory]::CreateDirectory($AssetsDirectory) | Out-Null
    $generated = @()
    foreach ($target in $targets) {
        $bitmap = New-Object System.Drawing.Bitmap($target.Width, $target.Height, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
        try {
            $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
            try {
                $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
                $graphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
                $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
                $graphics.CompositingQuality = [System.Drawing.Drawing2D.CompositingQuality]::HighQuality
                $graphics.Clear([System.Drawing.Color]::Transparent)
                $available = [Math]::Min($target.Width, $target.Height) * (1 - $target.Padding)
                $scale = $available / $artwork.Width
                $drawWidth = [int][Math]::Round($artwork.Width * $scale)
                $drawHeight = [int][Math]::Round($artwork.Height * $scale)
                $graphics.DrawImage($artwork, [int](($target.Width - $drawWidth) / 2), [int](($target.Height - $drawHeight) / 2), $drawWidth, $drawHeight)
            } finally { $graphics.Dispose() }
            $destination = Join-Path $AssetsDirectory $target.Name
            $bitmap.Save($destination, [System.Drawing.Imaging.ImageFormat]::Png)
        } finally { $bitmap.Dispose() }
        $generated += [ordered]@{
            path = "apps/deskfuse-windows/Assets/$($target.Name)"
            width = $target.Width
            height = $target.Height
            bytes = (Get-Item -LiteralPath $destination).Length
            sha256 = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash.ToLowerInvariant()
        }
        Write-Output "assets: $($target.Name) $($target.Width)x$($target.Height)"
    }
    # Release the source image before the temp directory is deleted: FromFile
    # keeps the file mapped until the Image is disposed.
    $artwork.Dispose()
    $manifest = [ordered]@{
        schema_version = 1
        purpose = 'Microsoft Store / MSIX image assets derived from the existing app mark'
        source = $sourceKind
        source_sha256 = (Get-FileHash -LiteralPath $sourcePath -Algorithm SHA256).Hash.ToLowerInvariant()
        generated_at = [DateTime]::UtcNow.ToString('o')
        generated_on = 'windows-powershell-system.drawing'
        assets = $generated
        notes = @(
            'Square tiles and the wide/splash variants keep a transparent background; the artwork already contains the app tile.',
            'Regenerate on a machine with resources/generated/DeskFuse-icon-1024.png (macOS asset pipeline) for canonical pixels.'
        )
    }
    $manifestPath = Join-Path $AssetsDirectory 'store-assets.json'
    $manifest | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $manifestPath -Encoding UTF8
    Write-Output "Store asset manifest: $manifestPath"
} finally {
    if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Recurse -Force }
}
