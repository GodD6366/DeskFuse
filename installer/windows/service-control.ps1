# Only the elevated installer invokes this fixed-operation helper. No arbitrary commands.
[CmdletBinding()]
param([Parameter(Mandatory=$true)][ValidateSet('Prepare', 'Register', 'Remove')][string]$Action)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$step = 'Initialize'
try {
    # PowerShell's provider location and the native process working directory are
    # distinct. Keep both away from NSIS's native System.dll plugin directory.
    $frameworkDirectory = [Runtime.InteropServices.RuntimeEnvironment]::GetRuntimeDirectory()
    Set-Location -LiteralPath $frameworkDirectory
    [Environment]::CurrentDirectory = $frameworkDirectory
    $principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
    if (!$principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Installer administrator privileges required' }
    $install = Join-Path ([Environment]::GetFolderPath('ProgramFiles')) 'DeskFuse'
    $binary = Join-Path $install 'deskfuse-service.exe'
    $firewallGroup = 'DeskFuse Desktop Installer'
    $firewallRules = @(
        @{ Name='DeskFuse.Installer.Private.QUIC.UDP42424'; Port=42424; Label='DeskFuse private LAN QUIC'; Protocol='UDP' },
        @{ Name='DeskFuse.Installer.Private.Input.UDP42425'; Port=42425; Label='DeskFuse private LAN input'; Protocol='UDP' },
        @{ Name='DeskFuse.Installer.Private.mDNS.UDP5353'; Port=5353; Label='DeskFuse private LAN discovery'; Protocol='UDP' },
        @{ Name='DeskFuse.Installer.Private.LanWeb.TCP42426'; Port=42426; Label='DeskFuse private LAN web uploads'; Protocol='TCP' }
    )
    function Assert-NoAncestorLinks([string]$Path) {
        # Inspect each ancestor before descending. Never enumerate via a junction.
        $current = [IO.Path]::GetPathRoot($Path)
        $parts = $Path.Substring($current.Length).Split('\', [StringSplitOptions]::RemoveEmptyEntries)
        $ancestors = @($current)
        foreach ($part in $parts) { $current = Join-Path $current $part; $ancestors += $current }
        foreach ($ancestor in $ancestors) {
            try { $attributes = [IO.File]::GetAttributes($ancestor) }
            catch [IO.FileNotFoundException] { continue }
            catch [IO.DirectoryNotFoundException] { continue }
            if (!($attributes -band [IO.FileAttributes]::Directory) -or ($attributes -band [IO.FileAttributes]::ReparsePoint)) { throw "Unsafe installation ancestor: $ancestor" }
        }
    }
    function Protect-InstallTree {
        Assert-NoAncestorLinks $install
        [IO.Directory]::CreateDirectory($install) | Out-Null
        $pending = New-Object 'Collections.Generic.Queue[string]'
        $pending.Enqueue($install)
        while ($pending.Count -gt 0) {
            $path = $pending.Dequeue()
            $item = Get-Item -LiteralPath $path -Force
            if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Refusing installation reparse point: $path" }
            # Each directory is secured BEFORE listing immediate children. Do not use -Recurse.
            $acl = if ($item.PSIsContainer) { New-Object Security.AccessControl.DirectorySecurity } else { New-Object Security.AccessControl.FileSecurity }
            $inherit = if ($item.PSIsContainer) { 'OICI' } else { '' }
            $sddl = "O:BAG:BAD:P(A;$inherit;FA;;;SY)(A;$inherit;FA;;;BA)(A;$inherit;0x1200a9;;;BU)"
            $acl.SetSecurityDescriptorSddlForm($sddl)
            Set-Acl -LiteralPath $path -AclObject $acl
            $verified = Get-Acl -LiteralPath $path
            if (!$verified.AreAccessRulesProtected -or $verified.GetOwner([Security.Principal.SecurityIdentifier]).Value -ne 'S-1-5-32-544') { throw "Installation ACL/owner was not applied: $path" }
            $rules = @($verified.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier]))
            $expected = @{ 'S-1-5-18'=0x1f01ff; 'S-1-5-32-544'=0x1f01ff; 'S-1-5-32-545'=0x1200a9 }
            if ($rules.Count -ne 3) { throw "Unexpected installation ACL entries: $path" }
            foreach ($rule in $rules) {
                $sid = $rule.IdentityReference.Value
                if (!$expected.ContainsKey($sid) -or [int]$rule.FileSystemRights -ne $expected[$sid] -or $rule.AccessControlType -ne 'Allow' -or $rule.IsInherited) { throw "Unsafe installation ACL: $path" }
                $expected.Remove($sid)
            }
            if ($item.PSIsContainer) {
                foreach ($child in Get-ChildItem -LiteralPath $path -Force) {
                    if ($child.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Refusing installation reparse point: $($child.FullName)" }
                    $pending.Enqueue($child.FullName)
                }
            }
        }
    }
    function Protect-Service {
        # SYSTEM/Admin full control; Users QUERY_CONFIG/QUERY_STATUS/START/READ_CONTROL only.
        # In particular no CHANGE_CONFIG, STOP, WRITE_DAC or WRITE_OWNER for users.
        # Owner is also restricted: a previous ordinary-user owner could otherwise
        # rewrite even a restrictive DACL using the implicit owner WRITE_DAC right.
        if (!('DeskFuseInstaller.ServiceSecurity' -as [type])) {
            # Absolute references avoid C# resolving NSIS's unrelated System.dll.
            $frameworkSystem = Join-Path $frameworkDirectory 'System.dll'
            if (!(Test-Path -LiteralPath $frameworkSystem -PathType Leaf)) { throw 'Missing .NET Framework System.dll' }
            Add-Type -ReferencedAssemblies $frameworkSystem -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
namespace DeskFuseInstaller {
    public static class ServiceSecurity {
        [DllImport("advapi32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
        static extern IntPtr OpenSCManagerW(string machine, string database, uint access);
        [DllImport("advapi32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
        static extern IntPtr OpenServiceW(IntPtr manager, string name, uint access);
        [DllImport("advapi32.dll", SetLastError=true)]
        static extern bool SetServiceObjectSecurity(IntPtr service, uint information, IntPtr descriptor);
        [DllImport("advapi32.dll", SetLastError=true)]
        static extern bool QueryServiceObjectSecurity(IntPtr service, uint information, byte[] descriptor, uint size, out uint needed);
        [DllImport("advapi32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
        static extern bool ConvertStringSecurityDescriptorToSecurityDescriptorW(string sddl, uint revision, out IntPtr descriptor, out uint size);
        [DllImport("advapi32.dll")] static extern bool CloseServiceHandle(IntPtr handle);
        [DllImport("kernel32.dll")] static extern IntPtr LocalFree(IntPtr memory);
        public static byte[] Apply() {
            IntPtr manager=IntPtr.Zero, service=IntPtr.Zero, descriptor=IntPtr.Zero;
            try {
                manager=OpenSCManagerW(null, null, 1);
                if (manager==IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error());
                service=OpenServiceW(manager, "DeskFuseService", 0x000e0000);
                if (service==IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error());
                uint size;
                if (!ConvertStringSecurityDescriptorToSecurityDescriptorW("O:BAG:BAD:P(A;;0xf01ff;;;SY)(A;;0xf01ff;;;BA)(A;;0x20015;;;BU)", 1, out descriptor, out size))
                    throw new Win32Exception(Marshal.GetLastWin32Error());
                // OWNER | GROUP | DACL. No SACL privilege or system policy change.
                if (!SetServiceObjectSecurity(service, 7, descriptor)) throw new Win32Exception(Marshal.GetLastWin32Error());
                uint needed;
                if (!QueryServiceObjectSecurity(service, 7, null, 0, out needed) && Marshal.GetLastWin32Error()!=122)
                    throw new Win32Exception(Marshal.GetLastWin32Error());
                if (needed==0 || needed>65536) throw new InvalidOperationException("Invalid SCM security descriptor size");
                byte[] result=new byte[needed];
                if (!QueryServiceObjectSecurity(service, 7, result, needed, out needed)) throw new Win32Exception(Marshal.GetLastWin32Error());
                return result;
            } finally {
                if (descriptor!=IntPtr.Zero) LocalFree(descriptor);
                if (service!=IntPtr.Zero) CloseServiceHandle(service);
                if (manager!=IntPtr.Zero) CloseServiceHandle(manager);
            }
        }
    }
}
'@
        }
        $actual = [DeskFuseInstaller.ServiceSecurity]::Apply()
        $descriptor = [Security.AccessControl.RawSecurityDescriptor]::new($actual, 0)
        if ($descriptor.Owner.Value -ne 'S-1-5-32-544') { throw 'SCM owner verification failed' }
        $expected = @{ 'S-1-5-18'=0xf01ff; 'S-1-5-32-544'=0xf01ff; 'S-1-5-32-545'=0x20015 }
        if ($descriptor.DiscretionaryAcl.Count -ne 3) { throw 'Unexpected SCM DACL entries' }
        foreach ($ace in $descriptor.DiscretionaryAcl) {
            $sid = $ace.SecurityIdentifier.Value
            if (!$expected.ContainsKey($sid) -or $ace.AccessMask -ne $expected[$sid] -or $ace.AceType -ne 'AccessAllowed') { throw 'Unsafe SCM DACL' }
            $expected.Remove($sid)
        }
    }
    function Configure-ServiceRecovery {
        # SCM owns recovery; no second watchdog process is installed. The final
        # action is none so a persistent failure remains visible as stopped.
        & "$env:SystemRoot\System32\sc.exe" failure DeskFuseService reset= 86400 actions= restart/5000/restart/30000/""/0 | Out-Null
        if ($LASTEXITCODE -ne 0) { throw 'SCM recovery configuration failed' }
        & "$env:SystemRoot\System32\sc.exe" failureflag DeskFuseService 1 | Out-Null
        if ($LASTEXITCODE -ne 0) { throw 'SCM failure flag configuration failed' }
    }
    function Remove-InstallerFirewallRules {
        foreach ($definition in $firewallRules) {
            $existing = Get-NetFirewallRule -PolicyStore PersistentStore -Name $definition.Name -ErrorAction SilentlyContinue
            foreach ($rule in $existing) {
                if ($rule.Group -ne $firewallGroup) { throw "Firewall rule name belongs to another owner: $($definition.Name)" }
                Remove-NetFirewallRule -InputObject $rule
            }
        }
    }
    $step = 'Protect installation files'
    Write-Output "DeskFuse installer: $Action / $step"
    Assert-NoAncestorLinks $install
    if ($Action -ne 'Remove') { Protect-InstallTree }
    # Uninstall also walks without following links, but does not create/reset absent directories.
    if ($Action -eq 'Remove' -and (Test-Path -LiteralPath $install)) { Protect-InstallTree }
    $service = Get-CimInstance Win32_Service -Filter "Name='DeskFuseService'"
    if ($service) {
        $current = $service.PathName.Trim().Trim('"')
        if (![string]::Equals($current, $binary, [StringComparison]::OrdinalIgnoreCase)) { throw 'Existing DeskFuseService has an unexpected binary path; refusing to modify it' }
    }
    if ($service -and $Action -ne 'Remove') { Protect-Service }
    if ($Action -eq 'Prepare' -or $Action -eq 'Remove') {
        $desktop = @(Get-Process -Name 'DeskFuse.Windows' -ErrorAction SilentlyContinue)
        $inputHelpers = @(Get-Process -Name 'deskfuse-input-helper' -ErrorAction SilentlyContinue)
        if ($desktop.Count -gt 0 -or $inputHelpers.Count -gt 0) { throw 'Quit DeskFuse from its tray menu before installing or uninstalling.' }
        if ($service) {
            $controller = Get-Service -Name DeskFuseService
            if ($controller.Status -ne 'Stopped') {
                Stop-Service -Name DeskFuseService
                $controller.WaitForStatus([ServiceProcess.ServiceControllerStatus]::Stopped, [TimeSpan]::FromSeconds(30))
            }
            $controller.Dispose()
        }
    }
    if ($Action -eq 'Register') {
        if (!(Test-Path -LiteralPath $binary -PathType Leaf)) { throw 'Missing service payload' }
        $step = 'Register service with SCM'
        Write-Output "DeskFuse installer: $Action / $step"
        if ($service) {
            $result = Invoke-CimMethod -InputObject $service -MethodName Change -Arguments @{
                PathName=('"' + $binary + '"'); StartMode='Automatic'; StartName='LocalSystem'
            }
            if ($result.ReturnValue -ne 0) { throw "SCM configuration failed: $($result.ReturnValue)" }
        } else {
            # BinaryPathName is handed directly to SCM, preserving the required quotes.
            New-Service -Name DeskFuseService -DisplayName 'DeskFuse Service' -BinaryPathName ('"' + $binary + '"') -StartupType Automatic | Out-Null
        }
        $step = 'Apply service security'
        Write-Output "DeskFuse installer: $Action / $step"
        Protect-Service
        $step = 'Configure SCM recovery'
        Write-Output "DeskFuse installer: $Action / $step"
        Configure-ServiceRecovery
        $step = 'Configure private LAN firewall rules'
        Write-Output "DeskFuse installer: $Action / $step"
        Remove-InstallerFirewallRules
        foreach ($definition in $firewallRules) {
            New-NetFirewallRule -PolicyStore PersistentStore -Name $definition.Name -DisplayName $definition.Label -Group $firewallGroup -Direction Inbound -Action Allow -Enabled True -Profile Private -Program $binary -Protocol $definition.Protocol -LocalPort $definition.Port -RemoteAddress LocalSubnet -EdgeTraversalPolicy Block | Out-Null
        }
        $step = 'Start service'
        Write-Output "DeskFuse installer: $Action / $step"
        Start-Service -Name DeskFuseService
        $controller = Get-Service -Name DeskFuseService
        try { $controller.WaitForStatus([ServiceProcess.ServiceControllerStatus]::Running, [TimeSpan]::FromSeconds(30)) } finally { $controller.Dispose() }
    }
    if ($Action -eq 'Remove') { Remove-InstallerFirewallRules }
    if ($Action -eq 'Remove' -and $service) {
        & "$env:SystemRoot\System32\sc.exe" delete DeskFuseService
        if ($LASTEXITCODE -ne 0) { throw 'SCM removal failed' }
        # `sc delete` is asynchronous while old handles drain. NSIS must not
        # remove the binary until SCM has released the service registration.
        $deadline = [Environment]::TickCount64 + 30000
        while ($true) {
            $remaining = Get-CimInstance Win32_Service -Filter "Name='DeskFuseService'" -ErrorAction SilentlyContinue
            if (!$remaining) { break }
            if ([Environment]::TickCount64 -ge $deadline) { throw 'SCM removal did not complete before timeout' }
            Start-Sleep -Milliseconds 250
        }
    }
    # Deliberately never touch ProgramData, AppData, device identity, pairing or user files.
    exit 0
} catch {
    # Short lines survive the NSIS details pane without Format-Table truncation.
    Write-Output "DeskFuse installer FAILED: $Action / $step"
    Write-Output ("Error: " + $_.Exception.Message)
    Write-Output ("At line: " + $_.InvocationInfo.ScriptLineNumber)
    Write-Output ("Error ID: " + $_.FullyQualifiedErrorId)
    exit 1
}
