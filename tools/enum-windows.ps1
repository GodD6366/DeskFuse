param([int]$ProcessId)
$src = @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public class WE2 {
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc cb, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, uint a, out int v, int sz);
  public delegate bool EnumWindowsProc(IntPtr h, IntPtr l);
  public struct RECT { public int Left, Top, Right, Bottom; }
}
'@
Add-Type -TypeDefinition $src
$cb = [WE2+EnumWindowsProc]{
  param($h, $l)
  $p = [uint32]0
  [void][WE2]::GetWindowThreadProcessId($h, [ref]$p)
  if ($p -eq $ProcessId) {
    $t = New-Object System.Text.StringBuilder 256
    [void][WE2]::GetWindowTextW($h, $t, 256)
    $r = New-Object WE2+RECT
    [void][WE2]::GetWindowRect($h, [ref]$r)
    $cloak = 0
    [void][WE2]::DwmGetWindowAttribute($h, 14, [ref]$cloak, 4)
    Write-Output ("{0:x} vis={1} cloak={2} {3},{4} {5}x{6} title={7}" -f $h.ToInt64(), [WE2]::IsWindowVisible($h), $cloak, $r.Left, $r.Top, ($r.Right-$r.Left), ($r.Bottom-$r.Top), $t.ToString())
  }
  return $true
}
[void][WE2]::EnumWindows($cb, [IntPtr]::Zero)
