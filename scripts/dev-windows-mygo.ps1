# Local development loop for the MyGo WebView UI on Windows.
#
# Builds the Go WebView client and, unless -UiOnly is passed, the Rust desktop
# service with its helpers. The installed payload under Program Files is then
# replaced, the first copy of every replaced file is preserved next to the
# install directory, and the service plus desktop client are restarted.
#
# This never builds a signed installer and never touches user data.
#
# Typical use, from any PowerShell in the repository:
#   ./scripts/dev-windows-mygo.ps1
#   ./scripts/dev-windows-mygo.ps1 -UiOnly      # frontend/Go iterations only
#   ./scripts/dev-windows-mygo.ps1 -BuildOnly   # build, leave Program Files alone
#   ./scripts/dev-windows-mygo.ps1 -NoLaunch    # replace, do not start the UI
#
# Only the replace step prompts for elevation. The client is always launched
# non-elevated so paste and global hotkeys keep working against normal apps.
#
# If the execution policy blocks local scripts, run:
#   powershell -ExecutionPolicy Bypass -File .\scripts\dev-windows-mygo.ps1

[CmdletBinding()]
param(
    [string]$ProjectRoot = '',
    [string]$InstallDir = '',
    [switch]$UiOnly,
    [switch]$BuildOnly,
    [switch]$NoLaunch,
    # Internal: set when this script re-launches itself elevated.
    [switch]$InstallPhase,
    [string]$UiSource = '',
    [string]$ServiceSourceDir = '',
    [string]$LogPath = ''
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$script:ClientProcessNames = @('DeskFuse.Windows', 'deskfuse-mygo')
$script:ServiceFileNames = @(
    'deskfuse-service.exe',
    'deskfuse-service-client.exe',
    'deskfuse-input-helper.exe',
    'deskfuse-update-helper.exe'
)
$script:ServiceName = 'DeskFuseService'

function Test-Administrator {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($identity)
    return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Resolve-BashExecutable {
    $command = Get-Command bash.exe -ErrorAction SilentlyContinue
    $candidates = @()
    if ($command) { $candidates += $command.Source }
    $candidates += @(
        'C:\Program Files\Git\bin\bash.exe',
        'C:\Program Files\Git\usr\bin\bash.exe'
    )
    foreach ($candidate in $candidates) {
        if (Test-Path -LiteralPath $candidate -PathType Leaf) {
            return (Resolve-Path -LiteralPath $candidate).Path
        }
    }
    throw 'bash.exe not found. Install Git for Windows.'
}

function Resolve-GoExecutable {
    $command = Get-Command go.exe -ErrorAction SilentlyContinue
    $candidates = @()
    if ($command) { $candidates += $command.Source }
    $candidates += @(
        (Join-Path $env:USERPROFILE '.local\go\go\bin\go.exe'),
        (Join-Path $env:ProgramFiles 'Go\bin\go.exe'),
        (Join-Path $env:LOCALAPPDATA 'Programs\Go\bin\go.exe')
    )
    foreach ($candidate in $candidates) {
        if (Test-Path -LiteralPath $candidate -PathType Leaf) {
            return (Resolve-Path -LiteralPath $candidate).Path
        }
    }
    throw 'Go not found. Install Go 1.27.3+ or put go.exe on PATH.'
}

function Invoke-NativeBuild {
    param([scriptblock]$Command, [string]$Failure)
    # PowerShell 5.1 turns native stderr into a terminating error while
    # ErrorActionPreference is Stop; builds write progress to stderr.
    $ErrorActionPreference = 'Continue'
    try {
        # Keep build chatter off the success stream: callers capture only the
        # artifact path this function returns.
        & $Command | Out-Host
        $exitCode = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = 'Stop'
    }
    if ($exitCode -ne 0) { throw "$Failure (exit $exitCode)." }
}

function Build-WebViewClient {
    param([string]$Root)
    $bash = Resolve-BashExecutable
    $go = Resolve-GoExecutable
    $env:Path = "$(Split-Path -Parent $go);$env:Path"
    $goVersion = (& $go version) 2>&1
    if ($LASTEXITCODE -ne 0) { throw "go version failed: $goVersion" }
    Write-Host "Go: $goVersion"

    Push-Location -LiteralPath $Root
    try {
        Invoke-NativeBuild -Failure 'WebView client build failed' -Command {
            & $bash --noprofile --norc './scripts/build-mygo-ui.sh'
        }
    } finally {
        Pop-Location
    }

    $artifact = Join-Path $Root 'target\mygo-webview\deskfuse-mygo'
    if (!(Test-Path -LiteralPath $artifact -PathType Leaf)) {
        $withExtension = "$artifact.exe"
        if (Test-Path -LiteralPath $withExtension -PathType Leaf) {
            $artifact = $withExtension
        } else {
            throw "WebView build output missing: $artifact"
        }
    }
    Write-Host "WebView client: $artifact"
    return $artifact
}

function Build-WindowsService {
    param([string]$Root)
    $cargo = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
    if (!(Test-Path -LiteralPath $cargo -PathType Leaf)) {
        $command = Get-Command cargo.exe -ErrorAction SilentlyContinue
        if (!$command) { throw 'cargo not found. Install the Rust toolchain.' }
        $cargo = $command.Source
    }

    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    if (!(Test-Path -LiteralPath $vswhere -PathType Leaf)) {
        throw 'vswhere.exe not found. Install Visual Studio Build Tools with the MSVC x64 workload.'
    }
    $vsPath = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if (!$vsPath) { throw 'MSVC x64 build tools are missing.' }
    Import-Module (Join-Path $vsPath 'Common7\Tools\Microsoft.VisualStudio.DevShell.dll')
    Enter-VsDevShell -VsInstallPath $vsPath -SkipAutomaticLocation -DevCmdArguments '-arch=x64 -host_arch=x64' | Out-Host

    # Match scripts/build-windows-desktop.ps1: the shipped service links the
    # static CRT, and CARGO_ENCODED_RUSTFLAGS takes precedence over RUSTFLAGS.
    $previousTarget = $env:CARGO_TARGET_DIR
    $previousEncoded = [Environment]::GetEnvironmentVariable('CARGO_ENCODED_RUSTFLAGS', 'Process')
    $previousRustFlags = [Environment]::GetEnvironmentVariable('RUSTFLAGS', 'Process')
    try {
        $env:CARGO_TARGET_DIR = Join-Path $Root 'target'
        $separator = [string][char]0x1f
        if ($null -ne $previousEncoded) {
            $effectiveFlags = $previousEncoded
        } elseif (![string]::IsNullOrWhiteSpace($previousRustFlags)) {
            $effectiveFlags = (($previousRustFlags.Trim() -split '\s+') -join $separator)
        } else {
            $effectiveFlags = ''
        }
        $staticCrtFlags = '-C' + $separator + 'target-feature=+crt-static'
        $env:CARGO_ENCODED_RUSTFLAGS = if ($effectiveFlags.Length) {
            $effectiveFlags + $separator + $staticCrtFlags
        } else {
            $staticCrtFlags
        }

        Push-Location -LiteralPath $Root
        try {
            Invoke-NativeBuild -Failure 'Service build failed' -Command {
                & $cargo build --locked --release --target x86_64-pc-windows-msvc -p deskfuse-service --bins
            }
            Invoke-NativeBuild -Failure 'Update helper build failed' -Command {
                & $cargo build --locked --release --target x86_64-pc-windows-msvc -p deskfuse-update-platform --bin deskfuse-update-helper
            }
        } finally {
            Pop-Location
        }
    } finally {
        $env:CARGO_TARGET_DIR = $previousTarget
        [Environment]::SetEnvironmentVariable('CARGO_ENCODED_RUSTFLAGS', $previousEncoded, 'Process')
        [Environment]::SetEnvironmentVariable('RUSTFLAGS', $previousRustFlags, 'Process')
    }

    $outputDirectory = Join-Path $Root 'target\x86_64-pc-windows-msvc\release'
    foreach ($name in $script:ServiceFileNames) {
        $path = Join-Path $outputDirectory $name
        if (!(Test-Path -LiteralPath $path -PathType Leaf)) {
            throw "Service build output missing: $path"
        }
    }
    Write-Host "Service payload: $outputDirectory"
    return $outputDirectory
}

function Stop-InstalledClient {
    param([string]$InstallDir)
    $prefix = $InstallDir + [IO.Path]::DirectorySeparatorChar
    foreach ($name in $script:ClientProcessNames) {
        foreach ($process in @(Get-Process -Name $name -ErrorAction SilentlyContinue)) {
            $path = $null
            try { $path = $process.Path } catch { }
            if (!$path) { continue }
            if (!$path.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) { continue }
            Stop-Process -Id $process.Id -Force
            if (!$process.WaitForExit(10000)) { throw "Client did not exit: $($process.Id)" }
            Write-Output "Stopped client PID $($process.Id): $path"
        }
    }
}

function Copy-FileWithRetry {
    param([string]$Source, [string]$Destination, [int]$Attempts = 5)
    for ($attempt = 1; $attempt -le $Attempts; $attempt++) {
        try {
            Copy-Item -LiteralPath $Source -Destination $Destination -Force
            return
        } catch {
            if ($attempt -eq $Attempts) {
                throw "Could not replace $Destination : $($_.Exception.Message)"
            }
            Start-Sleep -Milliseconds 500
        }
    }
}

function Backup-Once {
    param([string]$Path, [string]$BackupDirectory)
    $destination = Join-Path $BackupDirectory (Split-Path -Leaf $Path)
    if (!(Test-Path -LiteralPath $destination -PathType Leaf)) {
        Copy-Item -LiteralPath $Path -Destination $destination -Force
        Write-Output "Backup: $destination"
    }
}

function Assert-FileHashEqual {
    param([string]$Source, [string]$Destination)
    $sourceHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $Source).Hash
    $destinationHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $Destination).Hash
    if ($sourceHash -ne $destinationHash) {
        throw "Copy verification failed: $Destination"
    }
}

function Invoke-InstallPhase {
    param(
        [string]$InstallDir,
        [string]$UiSource,
        [string]$ServiceSourceDir
    )
    if (!(Test-Path -LiteralPath $InstallDir -PathType Container)) {
        throw "Install directory not found: $InstallDir"
    }
    if (!(Test-Path -LiteralPath $UiSource -PathType Leaf)) {
        throw "WebView client build missing: $UiSource"
    }
    $installedClient = Join-Path $InstallDir 'DeskFuse.Windows.exe'
    if (!(Test-Path -LiteralPath $installedClient -PathType Leaf)) {
        throw "Installed client not found: $installedClient"
    }
    $service = Get-Service -Name $script:ServiceName -ErrorAction SilentlyContinue
    if (!$service) {
        throw "$($script:ServiceName) is not installed. Install the desktop package first."
    }

    $serviceChanges = @()
    if ($ServiceSourceDir) {
        foreach ($name in $script:ServiceFileNames) {
            $source = Join-Path $ServiceSourceDir $name
            if (!(Test-Path -LiteralPath $source -PathType Leaf)) {
                throw "Service build missing: $source"
            }
            $destination = Join-Path $InstallDir $name
            $changed = !(Test-Path -LiteralPath $destination -PathType Leaf)
            if (!$changed) {
                $sourceHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $source).Hash
                $destinationHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $destination).Hash
                $changed = $sourceHash -ne $destinationHash
            }
            if ($changed) { $serviceChanges += $name }
        }
    }

    # Free the installed executable before replacing it.
    Stop-InstalledClient -InstallDir $InstallDir

    $backupDirectory = "$InstallDir-dev-backup"
    if (!(Test-Path -LiteralPath $backupDirectory -PathType Container)) {
        New-Item -ItemType Directory -Path $backupDirectory -Force | Out-Null
    }
    Write-Output "Backup directory: $backupDirectory"

    if ($serviceChanges.Count -gt 0) {
        Write-Output "Service files changed: $($serviceChanges -join ', ')"
        if ((Get-Service -Name $script:ServiceName).Status -ne 'Stopped') {
            Stop-Service -Name $script:ServiceName
            (Get-Service -Name $script:ServiceName).WaitForStatus('Stopped', [TimeSpan]::FromSeconds(30))
        }
        $helperPrefix = $InstallDir + [IO.Path]::DirectorySeparatorChar
        foreach ($helper in @(Get-Process -Name 'deskfuse-input-helper' -ErrorAction SilentlyContinue)) {
            $path = $null
            try { $path = $helper.Path } catch { }
            if ($path -and $path.StartsWith($helperPrefix, [StringComparison]::OrdinalIgnoreCase)) {
                Stop-Process -Id $helper.Id -Force
                if (!$helper.WaitForExit(10000)) { throw "Input helper did not exit: $($helper.Id)" }
            }
        }
        foreach ($name in $serviceChanges) {
            $destination = Join-Path $InstallDir $name
            Backup-Once -Path $destination -BackupDirectory $backupDirectory
            Copy-FileWithRetry -Source (Join-Path $ServiceSourceDir $name) -Destination $destination
            Assert-FileHashEqual -Source (Join-Path $ServiceSourceDir $name) -Destination $destination
        }
    } else {
        Write-Output 'Service files already match the local build.'
    }

    Backup-Once -Path $installedClient -BackupDirectory $backupDirectory
    Copy-FileWithRetry -Source $UiSource -Destination $installedClient
    Assert-FileHashEqual -Source $UiSource -Destination $installedClient
    Write-Output "Installed client: $installedClient"

    if ($serviceChanges.Count -gt 0 -or (Get-Service -Name $script:ServiceName).Status -ne 'Running') {
        Start-Service -Name $script:ServiceName
        (Get-Service -Name $script:ServiceName).WaitForStatus('Running', [TimeSpan]::FromSeconds(30))
    }
    Write-Output "Service: $((Get-Service -Name $script:ServiceName).Status)"
    Write-Output "First-run originals are preserved in: $backupDirectory"
    Write-Output "To restore them: stop $($script:ServiceName), copy that directory's files back to $InstallDir, then start the service."
}

function Invoke-ElevatedInstall {
    param(
        [string]$ProjectRoot,
        [string]$InstallDir,
        [string]$UiSource,
        [string]$ServiceSourceDir,
        [string]$LogPath
    )
    $arguments = @(
        '-NoProfile',
        '-ExecutionPolicy', 'Bypass',
        '-File', ('"{0}"' -f $PSCommandPath),
        '-InstallPhase',
        '-ProjectRoot', ('"{0}"' -f $ProjectRoot),
        '-InstallDir', ('"{0}"' -f $InstallDir),
        '-UiSource', ('"{0}"' -f $UiSource),
        '-LogPath', ('"{0}"' -f $LogPath)
    )
    if ($ServiceSourceDir) {
        $arguments += @('-ServiceSourceDir', ('"{0}"' -f $ServiceSourceDir))
    }
    try {
        $process = Start-Process -FilePath 'powershell.exe' -Verb RunAs -ArgumentList ($arguments -join ' ') -Wait -PassThru
    } catch {
        throw "Elevation was cancelled or failed; installed files are unchanged. $($_.Exception.Message)"
    }
    if ($process.ExitCode -ne 0) {
        if (Test-Path -LiteralPath $LogPath -PathType Leaf) {
            Write-Output '--- elevated install log (last 40 lines) ---'
            Get-Content -LiteralPath $LogPath | Select-Object -Last 40 | ForEach-Object { Write-Output $_ }
            Write-Output "Full log: $LogPath"
        }
        throw "Install phase failed (exit $($process.ExitCode)); installed files may be partially replaced."
    }
}

function Start-InstalledClient {
    param([string]$InstallDir, [bool]$LaunchFromElevatedShell)
    $client = Join-Path $InstallDir 'DeskFuse.Windows.exe'
    if ($LaunchFromElevatedShell) {
        # ShellExecute through Explorer drops the elevated token, so clipboard
        # paste and hotkeys keep working against normal-integrity applications.
        Start-Process -FilePath (Join-Path $env:SystemRoot 'explorer.exe') -ArgumentList ('"{0}"' -f $client)
    } else {
        Start-Process -FilePath $client -WorkingDirectory $InstallDir
    }

    $deadline = (Get-Date).AddSeconds(15)
    $clientProcess = $null
    do {
        Start-Sleep -Milliseconds 300
        foreach ($process in @(Get-Process -Name 'DeskFuse.Windows' -ErrorAction SilentlyContinue)) {
            $path = $null
            try { $path = $process.Path } catch { }
            if ($path -and [string]::Equals($path, $client, [StringComparison]::OrdinalIgnoreCase)) {
                $clientProcess = $process
                break
            }
        }
    } until ($clientProcess -or (Get-Date) -gt $deadline)
    if (!$clientProcess) { throw "Client did not start: $client" }

    Start-Sleep -Seconds 2
    if (!(Get-Process -Id $clientProcess.Id -ErrorAction SilentlyContinue)) {
        throw "Client exited immediately: $client"
    }
    Write-Output "Client running (PID $($clientProcess.Id))."
}

if (!$ProjectRoot) { $ProjectRoot = Split-Path -Parent $PSScriptRoot }
$ProjectRoot = (Resolve-Path -LiteralPath $ProjectRoot).Path
if (!$InstallDir) { $InstallDir = Join-Path $env:ProgramFiles 'DeskFuse' }
$InstallDir = [IO.Path]::GetFullPath($InstallDir).TrimEnd([char[]]@('\', '/'))
if (!$InstallDir) { throw 'InstallDir resolved to an empty path.' }

if ($InstallPhase) {
    $transcribing = $false
    try {
        if ($LogPath) {
            try {
                Start-Transcript -LiteralPath $LogPath -Force | Out-Null
                $transcribing = $true
            } catch {
                $transcribing = $false
            }
        }
        Invoke-InstallPhase -InstallDir $InstallDir -UiSource $UiSource -ServiceSourceDir $ServiceSourceDir
        exit 0
    } catch {
        Write-Error -ErrorRecord $_ -ErrorAction Continue
        exit 1
    } finally {
        if ($transcribing) { Stop-Transcript | Out-Null }
    }
}

$isAdministrator = Test-Administrator

Write-Output '[1/3] Building the MyGo WebView client...'
$uiSource = Build-WebViewClient -Root $ProjectRoot

$serviceSourceDir = ''
if (!$UiOnly) {
    Write-Output '[2/3] Building the Windows service and helpers...'
    $serviceSourceDir = Build-WindowsService -Root $ProjectRoot
} else {
    Write-Output '[2/3] Skipping the service build (-UiOnly).'
}

if ($BuildOnly) {
    Write-Output "Client: $uiSource"
    if ($serviceSourceDir) { Write-Output "Service: $serviceSourceDir" }
    Write-Output 'Build only; installed files are unchanged.'
    return
}

Write-Output '[3/3] Replacing the installed payload...'
if ($isAdministrator) {
    Invoke-InstallPhase -InstallDir $InstallDir -UiSource $uiSource -ServiceSourceDir $serviceSourceDir
} else {
    $logPath = Join-Path ([IO.Path]::GetTempPath()) ("deskfuse-dev-install-{0}.log" -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
    Invoke-ElevatedInstall -ProjectRoot $ProjectRoot -InstallDir $InstallDir -UiSource $uiSource -ServiceSourceDir $serviceSourceDir -LogPath $logPath
    Remove-Item -LiteralPath $logPath -Force -ErrorAction SilentlyContinue
}

if ($NoLaunch) {
    Write-Output 'Installed; skipped launch (-NoLaunch).'
} else {
    Start-InstalledClient -InstallDir $InstallDir -LaunchFromElevatedShell $isAdministrator
}
