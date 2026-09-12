# Run over SSH on the Windows x64 build host. Never auto-reboot the host.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$root = Join-Path $env:LOCALAPPDATA 'uniDesk-build-tools'
New-Item -ItemType Directory -Force $root | Out-Null
Start-Transcript -Path (Join-Path $root 'bootstrap.log') -Append
try {
    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    $vs = if (Test-Path $vswhere) { & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath }
    if (-not $vs) {
        $installer = Join-Path $root 'vs-buildtools.exe'
        Invoke-WebRequest 'https://aka.ms/vs/17/release/vs_buildtools.exe' -OutFile $installer
        $signature = Get-AuthenticodeSignature $installer
        if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'Microsoft Corporation') { throw 'Invalid Microsoft installer signature' }
        $process = Start-Process $installer -ArgumentList '--quiet --wait --norestart --nocache --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended' -Wait -PassThru
        if ($process.ExitCode -notin @(0,3010)) { throw "VS installer failed: $($process.ExitCode)" }
        if ($process.ExitCode -eq 3010) { Write-Output 'REBOOT_REQUIRED: host was not rebooted' }
    }
    $rustup = Join-Path $env:USERPROFILE '.cargo\bin\rustup.exe'
    if (-not (Test-Path $rustup)) {
        $installer = Join-Path $root 'rustup-init.exe'
        Invoke-WebRequest 'https://win.rustup.rs/x86_64' -OutFile $installer
        & $installer -y --default-host x86_64-pc-windows-msvc --default-toolchain none --profile minimal --no-modify-path
        if ($LASTEXITCODE -ne 0) { throw 'rustup initialization failed' }
    }
    & $rustup toolchain install 1.97.1 --profile minimal --component rustfmt --component clippy
    if ($LASTEXITCODE -ne 0) { throw 'Pinned Rust toolchain installation failed' }
    & $rustup run 1.97.1 rustc -vV
    & $rustup run 1.97.1 cargo -V
    if (Test-Path $vswhere) { & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -format json }
} finally { Stop-Transcript }
