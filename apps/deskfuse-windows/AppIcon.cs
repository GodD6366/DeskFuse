using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using DrawingIcon = System.Drawing.Icon;

namespace DeskFuse.Windows;

/// <summary>One embedded brand resource for the executable, native windows and tray.</summary>
internal static class AppIcon
{
    private static readonly Uri ResourceUri = new("pack://application:,,,/DeskFuse.Windows;component/Assets/DeskFuse.ico", UriKind.Absolute);
    private static ImageSource? windowIcon;
    private static bool installed;

    internal static ImageSource WindowIcon
    {
        get
        {
            if (windowIcon is not null) return windowIcon;
            using var stream = OpenResource();
            var decoder = new IconBitmapDecoder(stream, BitmapCreateOptions.PreservePixelFormat, BitmapCacheOption.OnLoad);
            var frame = decoder.Frames.OrderByDescending(item => item.PixelWidth).First();
            frame.Freeze();
            return windowIcon = frame;
        }
    }

    internal static void InstallWindowDefaults()
    {
        if (installed) return;
        installed = true;
        // Window subclasses do not reliably inherit an implicit Window style.
        // A class handler covers the main window, settings and standalone shelf
        // without editing each view, and preserves any explicit per-window icon.
        EventManager.RegisterClassHandler(typeof(Window), FrameworkElement.LoadedEvent, new RoutedEventHandler((sender, args) =>
        {
            if (ReferenceEquals(sender, args.OriginalSource) && sender is Window { Icon: null } window)
                window.SetCurrentValue(Window.IconProperty, WindowIcon);
        }));
    }

    internal static DrawingIcon CreateTrayIcon()
    {
        // Pick the system's small-icon physical size from the multi-frame ICO.
        // The existing blue/cyan mark on its light base works in either theme;
        // unlike the macOS template, it never relies on Windows recoloring it.
        int width = Math.Clamp(GetSystemMetrics(49), 16, 256); // SM_CXSMICON
        int height = Math.Clamp(GetSystemMetrics(50), 16, 256); // SM_CYSMICON
        using var stream = OpenResource();
        using var selected = new DrawingIcon(stream, width, height);
        return (DrawingIcon)selected.Clone();
    }

    private static Stream OpenResource() => System.Windows.Application.GetResourceStream(ResourceUri)?.Stream
        ?? throw new InvalidOperationException("Missing embedded DeskFuse icon resource.");

    [DllImport("user32.dll")]
    private static extern int GetSystemMetrics(int index);
}
