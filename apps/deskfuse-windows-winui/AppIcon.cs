using Microsoft.UI.Xaml;

namespace DeskFuse.Windows;

/// <summary>One embedded brand resource for the executable, native windows and tray.</summary>
internal static class AppIcon
{
    private static string IconPath => Path.Combine(AppContext.BaseDirectory, "Assets", "DeskFuse.ico");

    /// <summary>Sets the taskbar/alt-tab/title-bar icon. WinUI 3 has no Window.Icon.</summary>
    public static void Apply(Window window)
    {
        try { window.AppWindow.SetIcon(IconPath); }
        catch (Exception) { /* a missing icon must never block startup */ }
    }

    /// <summary>
    /// Small notification-area icon at the system's physical size. The existing
    /// blue/cyan mark on its light base works in either theme; unlike the macOS
    /// template it never relies on Windows recoloring it.
    /// </summary>
    internal static IntPtr CreateTrayIcon() => TrayIcon.LoadTrayIcon(IconPath);
}
