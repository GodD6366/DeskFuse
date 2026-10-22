param([string]$Out = "$env:TEMP\deskfuse-shot.png", [int]$ProcessId = 0, [int]$MaxWidth = 1600)
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Win32Cap {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc cb, System.IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(System.IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(System.IntPtr h);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(System.IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(System.IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(System.IntPtr h, int cmd);
  public delegate bool EnumWindowsProc(System.IntPtr h, System.IntPtr l);
  public struct RECT { public int Left, Top, Right, Bottom; }
  public static System.IntPtr FindByPid(uint pid) {
    System.IntPtr found = System.IntPtr.Zero;
    EnumWindows((h, l) => {
      uint p; GetWindowThreadProcessId(h, out p);
      if (p == pid && IsWindowVisible(h)) {
        RECT r; GetWindowRect(h, out r);
        if (r.Right - r.Left > 200 && r.Bottom - r.Top > 200) { found = h; return false; }
      }
      return true;
    }, System.IntPtr.Zero);
    return found;
  }
}
"@
[void][Win32Cap]::SetProcessDPIAware()
$hwnd = [Win32Cap]::FindByPid([uint32]$ProcessId)
if ($hwnd -eq [System.IntPtr]::Zero) { Write-Error "no window for pid $ProcessId"; exit 1 }
[void][Win32Cap]::ShowWindow($hwnd, 9)
[void][Win32Cap]::SetForegroundWindow($hwnd)
Start-Sleep -Milliseconds 600
$r = New-Object Win32Cap+RECT
[void][Win32Cap]::GetWindowRect($hwnd, [ref]$r)
$w = $r.Right - $r.Left; $h = $r.Bottom - $r.Top
$bmp = New-Object System.Drawing.Bitmap($w, $h)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($r.Left, $r.Top, 0, 0, $bmp.Size)
if ($bmp.Width -gt $MaxWidth) {
  $scale = $MaxWidth / $bmp.Width
  $thumb = New-Object System.Drawing.Bitmap([int]($bmp.Width * $scale), [int]($bmp.Height * $scale))
  $g2 = [System.Drawing.Graphics]::FromImage($thumb)
  $g2.InterpolationMode = 'HighQualityBicubic'
  $g2.DrawImage($bmp, 0, 0, $thumb.Width, $thumb.Height)
  $bmp.Dispose(); $bmp = $thumb
}
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
Write-Output "saved $Out ($w x $h physical)"
