param([int]$ProcessId, [string]$AutomationId, [string]$Name = "")
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$root = [System.Windows.Automation.AutomationElement]::RootElement
$condition = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty, $ProcessId)
$window = $root.FindFirst([System.Windows.Automation.TreeScope]::Children, $condition)
if ($null -eq $window) { Write-Error "no window"; exit 1 }
if ($AutomationId) {
  $cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::AutomationIdProperty, $AutomationId)
} else {
  $cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::NameProperty, $Name)
}
$el = $window.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $cond)
if ($null -eq $el) { Write-Error "element not found: $AutomationId$Name"; exit 1 }
try {
  $invoke = $el.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
  $invoke.Invoke()
  Write-Output "invoked $AutomationId$Name"
} catch {
  # 退化为坐标点击
  $r = $el.Current.BoundingRectangle
  Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Clicker { [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y); [DllImport("user32.dll")] public static extern void mouse_event(int f, int dx, int dy, int d, System.IntPtr e); }
"@
  $x = [int]($r.X + $r.Width / 2); $y = [int]($r.Y + $r.Height / 2)
  [Clicker]::SetCursorPos($x, $y) | Out-Null
  Start-Sleep -Milliseconds 100
  [Clicker]::mouse_event(0x0002, 0, 0, 0, [System.IntPtr]::Zero)
  [Clicker]::mouse_event(0x0004, 0, 0, 0, [System.IntPtr]::Zero)
  Write-Output "clicked $AutomationId$Name at $x,$y"
}
