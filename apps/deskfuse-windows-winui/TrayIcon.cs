using System.Runtime.InteropServices;

namespace DeskFuse.Windows;

/// <summary>One tray menu row; children render as a submenu ("-" is a separator).</summary>
public sealed record TrayMenuEntry(string Label, Action? Invoke = null, IReadOnlyList<TrayMenuEntry>? Children = null, bool Enabled = true);

/// <summary>
/// Win32 notification-area icon. WinUI 3 has no NotifyIcon, so this uses
/// Shell_NotifyIcon against the main window and dispatches its callback message
/// through the shared window-procedure hook.
/// </summary>
public sealed class TrayIcon : IDisposable
{
    private const uint CallbackMessage = 0x8000 + 1; // WM_APP + 1
    private const uint NifMessage = 0x00000001, NifIcon = 0x00000002, NifTip = 0x00000004;
    private const uint NimAdd = 0x00000000, NimModify = 0x00000001, NimDelete = 0x00000002, NimSetVersion = 0x00000004;
    private const uint NotifyIconVersion4 = 4;
    private const uint WmRButtonUp = 0x0205, WmLButtonDblClk = 0x0203;
    private const uint MfString = 0x00000000, MfPopup = 0x00000010, MfSeparator = 0x00000800, MfGrayed = 0x00000001;
    private const uint TpmRightButton = 0x0002, TpmReturnCmd = 0x0100, TpmNonotify = 0x0080;
    private const uint ImageIcon = 1, LrLoadFromFile = 0x00000010;
    private const int SmCxSmIcon = 49, SmCySmIcon = 50;
    private readonly IntPtr window;
    private readonly IntPtr icon;
    private readonly uint id = 1;
    private bool disposed;

    public Func<IReadOnlyList<TrayMenuEntry>>? MenuProvider { get; set; }
    public Action? Activated { get; set; }

    public TrayIcon(IntPtr hwnd, string tooltip)
    {
        window = hwnd;
        icon = AppIcon.CreateTrayIcon();
        WindowMessageHook.Attach(hwnd, WindowProcedure);
        var data = CreateData(tooltip);
        Shell_NotifyIconW(NimAdd, ref data);
        data.TimeoutOrVersion = NotifyIconVersion4;
        Shell_NotifyIconW(NimSetVersion, ref data);
    }

    private NotifyIconData CreateData(string tooltip)
    {
        var data = new NotifyIconData
        {
            Size = Marshal.SizeOf<NotifyIconData>(),
            Window = window,
            Id = id,
            Flags = NifMessage | NifIcon | NifTip,
            CallbackMessage = CallbackMessage,
            Icon = icon,
        };
        data.Tip = tooltip.Length > 127 ? tooltip[..127] : tooltip;
        return data;
    }

    private bool WindowProcedure(uint message, IntPtr wParam, IntPtr lParam)
    {
        if (message != CallbackMessage || disposed) return false;
        uint mouse = (uint)(lParam.ToInt64() & 0xFFFF);
        if (mouse is WmLButtonDblClk) { Activated?.Invoke(); return true; }
        if (mouse is WmRButtonUp) { ShowMenu(); return true; }
        return true;
    }

    private void ShowMenu()
    {
        if (MenuProvider is not { } provider) return;
        var callbacks = new Dictionary<uint, Action>();
        uint next = 1;
        IntPtr menu = CreatePopupMenu();
        if (menu == IntPtr.Zero) return;
        try
        {
            foreach (var entry in provider())
            {
                if (entry.Label == "-") { AppendMenuW(menu, MfSeparator, UIntPtr.Zero, null); continue; }
                if (entry.Children is { Count: > 0 } children)
                {
                    IntPtr submenu = CreatePopupMenu();
                    if (submenu == IntPtr.Zero) continue;
                    if (children.Count == 0) AppendMenuW(submenu, MfString | MfGrayed, UIntPtr.Zero, "（空）");
                    foreach (var child in children)
                    {
                        if (child.Label == "-") { AppendMenuW(submenu, MfSeparator, UIntPtr.Zero, null); continue; }
                        uint childId = next++;
                        callbacks[childId] = child.Invoke ?? (() => { });
                        AppendMenuW(submenu, MfString | (child.Enabled ? 0 : MfGrayed), (UIntPtr)childId, child.Label);
                    }
                    AppendMenuW(menu, MfPopup | MfString, (UIntPtr)submenu, entry.Label);
                    continue;
                }
                uint commandId = next++;
                callbacks[commandId] = entry.Invoke ?? (() => { });
                AppendMenuW(menu, MfString | (entry.Enabled ? 0 : MfGrayed), (UIntPtr)commandId, entry.Label);
            }
            GetCursorPos(out var cursor);
            SetForegroundWindow(window);
            uint selected = TrackPopupMenu(menu, TpmRightButton | TpmReturnCmd | TpmNonotify, cursor.X, cursor.Y, 0, window, IntPtr.Zero);
            if (selected != 0 && callbacks.TryGetValue(selected, out var action)) action();
        }
        finally { DestroyMenu(menu); }
    }

    public void Dispose()
    {
        if (disposed) return;
        disposed = true;
        var data = CreateData(string.Empty);
        Shell_NotifyIconW(NimDelete, ref data);
        if (icon != IntPtr.Zero) DestroyIcon(icon);
        WindowMessageHook.Detach(window);
    }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct NotifyIconData
    {
        public int Size;
        public IntPtr Window;
        public uint Id;
        public uint Flags;
        public uint CallbackMessage;
        public IntPtr Icon;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)] public string Tip;
        public uint State;
        public uint StateMask;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 256)] public string Info;
        public uint TimeoutOrVersion;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 64)] public string InfoTitle;
        public uint InfoFlags;
        public Guid ItemGuid;
        public IntPtr BalloonIcon;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct Point { public int X, Y; }

    [DllImport("shell32.dll", EntryPoint = "Shell_NotifyIconW", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool Shell_NotifyIconW(uint message, ref NotifyIconData data);

    [DllImport("user32.dll")] private static extern IntPtr CreatePopupMenu();
    [DllImport("user32.dll")] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool DestroyMenu(IntPtr menu);
    [DllImport("user32.dll", EntryPoint = "AppendMenuW", CharSet = CharSet.Unicode)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool AppendMenuW(IntPtr menu, uint flags, UIntPtr item, string? text);
    [DllImport("user32.dll")]
    private static extern uint TrackPopupMenu(IntPtr menu, uint flags, int x, int y, int reserved, IntPtr window, IntPtr rectangle);
    [DllImport("user32.dll")] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool GetCursorPos(out Point point);
    [DllImport("user32.dll")] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool DestroyIcon(IntPtr icon);
    [DllImport("user32.dll")] private static extern int GetSystemMetrics(int index);
    [DllImport("user32.dll", EntryPoint = "LoadImageW", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern IntPtr LoadImage(IntPtr instance, string name, uint type, int width, int height, uint flags);

    internal static IntPtr LoadTrayIcon(string path)
    {
        if (File.Exists(path))
        {
            var loaded = LoadImage(IntPtr.Zero, path, ImageIcon, GetSystemMetrics(SmCxSmIcon), GetSystemMetrics(SmCySmIcon), LrLoadFromFile);
            if (loaded != IntPtr.Zero) return loaded;
        }
        return LoadImage(IntPtr.Zero, path, ImageIcon, GetSystemMetrics(SmCxSmIcon), GetSystemMetrics(SmCySmIcon), LrLoadFromFile);
    }
}
