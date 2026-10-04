# Build a Microsoft Store / MSIX package for the DeskFuse desktop client from a
# published payload. Packaging only: never installs, launches, submits, or signs
# with a real Store identity unless one is supplied.
#
# This script deliberately uses MakeAppx instead of the single-project MSIX
# build so the same packaging path works for the WPF client today and the WinUI 3
# client after the conversion. Scope is the client only: the machine service and
# the uiAccess input helper cannot be delivered by an MSIX package; see
# docs/WINDOWS_STORE.md.
[CmdletBinding()]
param(
    [string]$ProjectRoot = '',
    [string]$PayloadDirectory = '',
    [string]$OutputDirectory = '',
    [string]$IdentityName = '',
    [string]$Publisher = '',
    [string]$PublisherDisplayName = '',
    [string]$Version = '',
    [string]$SigningPfx = '',
    [string]$SigningPassword = '',
    [string]$WackReport = '',
    [switch]$SkipBuild,
    [switch]$RunWack,
    [switch]$RequireStoreIdentity
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if (!$ProjectRoot) { $ProjectRoot = Split-Path -Parent $PSScriptRoot }
$ProjectRoot = (Resolve-Path -LiteralPath $ProjectRoot).Path
$clientProject = Join-Path $ProjectRoot 'apps\deskfuse-windows\DeskFuse.Windows.csproj'
$manifestPath = Join-Path $ProjectRoot 'apps\deskfuse-windows\Package.appxmanifest'
$assetsDirectory = Join-Path $ProjectRoot 'apps\deskfuse-windows\Assets'
$releaseMetadata = Join-Path $ProjectRoot 'release.json'
if (!$OutputDirectory) { $OutputDirectory = Join-Path $ProjectRoot 'dist\windows-store' }
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)

if (!(Test-Path -LiteralPath $manifestPath -PathType Leaf)) { throw 'Missing Package.appxmanifest' }
$manifest = New-Object System.Xml.XmlDocument
$manifest.PreserveWhitespace = $true
$manifest.Load($manifestPath)
$namespaceManager = New-Object System.Xml.XmlNamespaceManager($manifest.NameTable)
$namespaceManager.AddNamespace('f', 'http://schemas.microsoft.com/appx/manifest/foundation/windows10')
$identity = $manifest.SelectSingleNode('/f:Package/f:Identity', $namespaceManager)
if (!$identity) { throw 'Package.appxmanifest has no Identity element' }
if (!$IdentityName) { $IdentityName = $identity.GetAttribute('Name') }
if (!$Publisher) { $Publisher = $identity.GetAttribute('Publisher') }
if (!$PublisherDisplayName) { $PublisherDisplayName = $manifest.SelectSingleNode('/f:Package/f:Properties/f:PublisherDisplayName', $namespaceManager).InnerText }
if (!$Version) {
    if (!(Test-Path -LiteralPath $releaseMetadata -PathType Leaf)) { throw 'Pass -Version or provide release.json' }
    $release = Get-Content -Raw -LiteralPath $releaseMetadata | ConvertFrom-Json
    # Store submissions require Revision 0 and a monotonically increasing version.
    # Map Major.Minor from the product version and Build from the release build
    # number (the same counter the macOS CFBundleVersion uses), so every release
    # produces a strictly newer package without inventing a second counter.
    $product = @($release.version -split '\.')
    if ($product.Count -ne 3) { throw "release.json version must be X.Y.Z: $($release.version)" }
    $Version = "$($product[0]).$($product[1]).$($release.build_number).0"
}
$versionParts = @($Version -split '\.')
if ($versionParts.Count -ne 4 -or ($versionParts | Where-Object { $_ -notmatch '^\d+$' })) { throw "MSIX version must be Major.Minor.Build.Revision: $Version" }
if ($versionParts[3] -ne '0') { throw 'Microsoft Store submissions require Revision 0' }
foreach ($part in $versionParts) { if ([int]$part -gt 65535) { throw "MSIX version component exceeds 65535: $Version" } }

function Find-SdkTool([string]$Name) {    $roots = @("${env:ProgramFiles(x86)}\Windows Kits\10\bin", "$env:ProgramFiles\Windows Kits\10\bin")
    $found = foreach ($root in $roots) {
        if (Test-Path -LiteralPath $root) {
            Get-ChildItem -LiteralPath $root -Directory -ErrorAction SilentlyContinue |
                Sort-Object Name -Descending |
                ForEach-Object { Join-Path $_.FullName "x64\$Name" } |
                Where-Object { Test-Path -LiteralPath $_ -PathType Leaf }
        }
    }
    $first = @($found)[0]
    if (!$first) { throw "$Name was not found in the Windows SDK; install the Windows 10/11 SDK." }
    return $first
}
$makeAppx = Find-SdkTool 'makeappx.exe'
$signTool = Find-SdkTool 'signtool.exe'
# MSIX signing requires the certificate Subject to exactly match the manifest
# Publisher; otherwise SignTool fails with the opaque 0x8007000b. Fail before the
# publish/pack work instead of after it.
if ($SigningPfx) {
    if (!(Test-Path -LiteralPath $SigningPfx -PathType Leaf)) { throw "Signing certificate not found: $SigningPfx" }
    $certificate = New-Object System.Security.Cryptography.X509Certificates.X509Certificate2($SigningPfx, $SigningPassword)
    if ($certificate.Subject -ne $Publisher) {
        throw "MSIX Publisher must equal the signing certificate Subject. Publisher='$Publisher' certificate='$($certificate.Subject)'. Re-run with -Publisher '$($certificate.Subject)' (or use the certificate that owns the reserved identity)."
    }
}

$stage = Join-Path ([IO.Path]::GetTempPath()) ('deskfuse-store-package-' + [guid]::NewGuid().ToString('N'))
$layout = Join-Path $stage 'layout'
[IO.Directory]::CreateDirectory($layout) | Out-Null
$publishedPayload = $PayloadDirectory
try {
    if (!$publishedPayload) {
        if ($SkipBuild) { throw 'Pass -PayloadDirectory when -SkipBuild is used' }
        $publishedPayload = Join-Path $stage 'payload'
        Write-Output "Publishing client payload to $publishedPayload"
        # The Store variant must not embed the uiAccess manifest.
        & dotnet publish $clientProject -c Release -r win-x64 --self-contained true -o $publishedPayload `
            -p:PublishSingleFile=false -p:PublishTrimmed=false -p:ApplicationManifest=app.store.manifest
        if ($LASTEXITCODE -ne 0) { throw 'Client publish failed' }
    }
    $publishedPayload = (Resolve-Path -LiteralPath $publishedPayload).Path
    if (!(Test-Path -LiteralPath (Join-Path $publishedPayload 'DeskFuse.Windows.exe') -PathType Leaf)) {
        throw 'Payload does not contain DeskFuse.Windows.exe'
    }

    Copy-Item -Path (Join-Path $publishedPayload '*') -Destination $layout -Recurse -Force
    $assetsTarget = Join-Path $layout 'Assets'
    [IO.Directory]::CreateDirectory($assetsTarget) | Out-Null
    foreach ($asset in 'StoreLogo.png', 'Square44x44Logo.png', 'Square150x150Logo.png', 'Wide310x150Logo.png', 'SplashScreen.png') {
        $source = Join-Path $assetsDirectory $asset
        if (!(Test-Path -LiteralPath $source -PathType Leaf)) { throw "Missing Store asset: $asset (run scripts/generate-store-assets.ps1)" }
        Copy-Item -LiteralPath $source -Destination (Join-Path $assetsTarget $asset) -Force
    }

    $identity.SetAttribute('Name', $IdentityName)
    $identity.SetAttribute('Publisher', $Publisher)
    $identity.SetAttribute('Version', $Version)
    $identity.SetAttribute('ProcessorArchitecture', 'x64')
    $manifest.SelectSingleNode('/f:Package/f:Properties/f:PublisherDisplayName', $namespaceManager).InnerText = $PublisherDisplayName
    $layoutManifest = Join-Path $layout 'AppxManifest.xml'
    $settings = New-Object System.Xml.XmlWriterSettings
    $settings.Encoding = New-Object System.Text.UTF8Encoding($false)
    $settings.Indent = $true
    $writer = [System.Xml.XmlWriter]::Create($layoutManifest, $settings)
    try { $manifest.Save($writer) } finally { $writer.Dispose() }

    $placeholderIdentity = $IdentityName -eq 'DeskFuse.Desktop' -or $Publisher -eq 'CN=DeskFuse Development'
    if ($placeholderIdentity) {
        Write-Warning 'Package uses placeholder Identity/Publisher; replace them with the Partner Center reservation before submitting.'
        if ($RequireStoreIdentity) { throw 'Store identity is still the placeholder value' }
    }

    [IO.Directory]::CreateDirectory($OutputDirectory) | Out-Null
    $packageName = "DeskFuse-$Version-x64.msix"
    $packagePath = Join-Path $OutputDirectory $packageName
    Remove-Item -LiteralPath $packagePath -Force -ErrorAction SilentlyContinue
    & $makeAppx pack /d $layout /p $packagePath /o
    if ($LASTEXITCODE -ne 0) { throw 'MakeAppx pack failed' }
    # MakeAppx pack already schema-validates AppxManifest.xml and the payload;
    # this SDK has no `makeappx validate` verb. Full certification uses WACK.
    $wackRan = $false
    $wackPassed = $false
    if ($RunWack) {
        $appCert = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\App Certification Kit\appcert.exe'
        if (!(Test-Path -LiteralPath $appCert -PathType Leaf)) { throw 'Windows App Certification Kit (appcert.exe) is not installed' }
        if (!$WackReport) { $WackReport = $packagePath + '.wack.xml' }
        & $appCert test -packagepath $packagePath -reportoutputpath $WackReport
        $wackExit = $LASTEXITCODE
        $wackRan = $true
        $wackPassed = $wackExit -eq 0
        Write-Output "WACK exit code: $wackExit; report: $WackReport"
    }

    $signature = 'NotSigned'
    if ($SigningPfx) {
        $arguments = @('sign', '/fd', 'SHA256', '/f', $SigningPfx)
        if ($SigningPassword) { $arguments += @('/p', $SigningPassword) }
        $arguments += $packagePath
        & $signTool @arguments
        if ($LASTEXITCODE -ne 0) { throw 'SignTool signing failed' }
        $signature = 'Signed'
    }

    $hash = (Get-FileHash -LiteralPath $packagePath -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $packageName" | Set-Content -LiteralPath ($packagePath + '.sha256') -Encoding ASCII
    $metadata = [ordered]@{
        schema_version = 1
        kind = 'windows-desktop-msix'
        identity_name = $IdentityName
        publisher = $Publisher
        publisher_display_name = $PublisherDisplayName
        version = $Version
        architecture = 'x64'
        capabilities = @('runFullTrust', 'internetClient', 'privateNetworkClientServer')
        package = $packageName
        package_sha256 = $hash
        signature = $signature
        contains_client_only = $true
        service_included = $false
        uiaccess_helper_included = $false
        store_submission_ready = !$placeholderIdentity
        verification = [ordered]@{
            packaged = $true
            manifest_validated_by_pack = $true
            signed = ($signature -eq 'Signed')
            installed = $false
            launched = $false
            windows_app_certification_kit = $wackRan
            windows_app_certification_kit_passed = $wackPassed
            store_submission = $false
        }
        created_at = [DateTime]::UtcNow.ToString('o')
    }
    $metadata | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath ($packagePath + '.store.json') -Encoding UTF8

    Write-Output "MSIX: $packagePath"
    Write-Output "SHA-256: $hash; signature: $signature; Store identity placeholders: $placeholderIdentity"
    Write-Output 'Packaged only. No installation, launch, WACK run, or Partner Center submission was performed.'
} finally {
    if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue }
}
