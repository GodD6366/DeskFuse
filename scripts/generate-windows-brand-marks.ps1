# Generate the theme-aware sidebar/About brand marks from the same two SVG
# sources the macOS app uses (light artwork in light appearance, dark artwork
# in dark appearance). Store/MSIX tiles stay in generate-store-assets.ps1.
[CmdletBinding()]
param(
    [string]$ProjectRoot = '',
    [string]$AssetsDirectory = ''
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
Add-Type -AssemblyName System.Drawing

if (!$ProjectRoot) { $ProjectRoot = Split-Path -Parent $PSScriptRoot }
$ProjectRoot = (Resolve-Path -LiteralPath $ProjectRoot).Path
if (!$AssetsDirectory) { $AssetsDirectory = Join-Path $ProjectRoot 'apps\deskfuse-windows\Assets' }
$temporary = Join-Path ([IO.Path]::GetTempPath()) ('deskfuse-brand-marks-' + [guid]::NewGuid().ToString('N'))
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

function Render-Svg([string]$Path, [string]$Destination) {
    $edge = Find-Edge
    if (!$edge) { throw 'Microsoft Edge is required to rasterize the brand SVGs.' }
    $url = 'file:///' + ($Path -replace '\\', '/' -replace ' ', '%20')
    for ($attempt = 1; $attempt -le 2; $attempt++) {
        Remove-Item -LiteralPath $Destination -Force -ErrorAction SilentlyContinue
        Start-Process -FilePath $edge -WindowStyle Hidden -Wait -ArgumentList @(
            '--headless=new', '--disable-gpu', '--hide-scrollbars', '--default-background-color=00000000',
            '--window-size=1024,1024', "--screenshot=$Destination", $url) | Out-Null
        if ((Test-Path -LiteralPath $Destination -PathType Leaf) -and (Get-Item -LiteralPath $Destination).Length -gt 0) { return }
        Start-Sleep -Milliseconds 400
    }
    throw "Edge did not render $Path"
}

$marks = @(
    @{ Name = 'BrandMark.Light.png'; Svg = 'resources\DeskFuse-icon-light.svg' },
    @{ Name = 'BrandMark.Dark.png'; Svg = 'resources\DeskFuse-icon.svg' }
)

try {
    [IO.Directory]::CreateDirectory($AssetsDirectory) | Out-Null
    foreach ($mark in $marks) {
        $source = Join-Path $ProjectRoot $mark.Svg
        if (!(Test-Path -LiteralPath $source -PathType Leaf)) { throw "Missing $source" }
        $raster = Join-Path $temporary ($mark.Name + '.1024.png')
        Render-Svg -Path $source -Destination $raster
        $artwork = [System.Drawing.Image]::FromFile($raster)
        try {
            $bitmap = New-Object System.Drawing.Bitmap 256, 256, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
            try {
                $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
                try {
                    $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
                    $graphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
                    $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
                    $graphics.CompositingQuality = [System.Drawing.Drawing2D.CompositingQuality]::HighQuality
                    $graphics.Clear([System.Drawing.Color]::Transparent)
                    $graphics.DrawImage($artwork, 0, 0, 256, 256)
                } finally { $graphics.Dispose() }
                $destination = Join-Path $AssetsDirectory $mark.Name
                $bitmap.Save($destination, [System.Drawing.Imaging.ImageFormat]::Png)
                Write-Output "brand: $($mark.Name) 256x256 sha256=$((Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash.ToLowerInvariant())"
            } finally { $bitmap.Dispose() }
        } finally { $artwork.Dispose() }
    }
} finally {
    if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Recurse -Force }
}
