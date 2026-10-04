using System.Runtime.InteropServices;

namespace DeskFuse.Windows;

internal sealed record MonitorInfo(string DeviceName, int Left, int Top, int Width, int Height, bool Primary);

/// <summary>Win32 replacement for System.Windows.Forms.Screen (WinUI 3 does not
/// reference WinForms). Coordinates are physical pixels in virtual-screen space.</summary>
internal static class MonitorLayout
{
    private const int MonitorInfoPrimary = 0x00000001;
    private delegate bool MonitorEnumProcedure(IntPtr monitor, IntPtr hdc, ref Rect rectangle, IntPtr data);

    [StructLayout(LayoutKind.Sequential)]
    private struct Rect { public int Left, Top, Right, Bottom; }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct MonitorInfoEx
    {
        public int Size;
        public Rect Monitor;
        public Rect Work;
        public int Flags;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)]
        public string DeviceName;
    }

    public static IReadOnlyList<MonitorInfo> All()
    {
        var monitors = new List<MonitorInfo>();
        bool Collect(IntPtr monitor, IntPtr hdc, ref Rect rectangle, IntPtr data)
        {
            var info = new MonitorInfoEx { Size = Marshal.SizeOf<MonitorInfoEx>(), DeviceName = string.Empty };
            if (GetMonitorInfoW(monitor, ref info))
            {
                monitors.Add(new MonitorInfo(
                    info.DeviceName,
                    info.Monitor.Left,
                    info.Monitor.Top,
                    info.Monitor.Right - info.Monitor.Left,
                    info.Monitor.Bottom - info.Monitor.Top,
                    (info.Flags & MonitorInfoPrimary) != 0));
            }
            return true;
        }
        MonitorEnumProcedure callback = Collect;
        if (!EnumDisplayMonitors(IntPtr.Zero, IntPtr.Zero, callback, IntPtr.Zero)) return [];
        GC.KeepAlive(callback);
        return monitors;
    }

    [DllImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool EnumDisplayMonitors(IntPtr hdc, IntPtr clip, MonitorEnumProcedure callback, IntPtr data);

    [DllImport("user32.dll", EntryPoint = "GetMonitorInfoW", CharSet = CharSet.Unicode, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GetMonitorInfoW(IntPtr monitor, ref MonitorInfoEx info);
}
