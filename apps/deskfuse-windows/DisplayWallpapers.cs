using System.Runtime.InteropServices;
using Microsoft.UI.Xaml.Media.Imaging;
using Windows.Storage.Streams;

namespace DeskFuse.Windows;

/// Reads configured wallpaper files only, never the desktop or application pixels.
internal sealed class DisplayWallpapers
{
    private readonly Dictionary<string, (string Signature, BitmapImage Image)> images = [];
    private readonly Dictionary<string, string> localPaths = [];
    private DateTime nextRefresh;

    public void Clear()
    {
        images.Clear();
        localPaths.Clear();
        nextRefresh = default;
    }

    public BitmapImage? Local(MonitorInfo monitor)
    {
        if (DateTime.UtcNow >= nextRefresh) RefreshLocal();
        if (!localPaths.TryGetValue(monitor.DeviceName, out var path) || string.IsNullOrEmpty(path)) return null;
        try
        {
            var file = new FileInfo(path);
            if (!file.Exists || file.Length > 64 * 1024 * 1024) return null;
            string key = "local:" + monitor.DeviceName;
            string signature = path + "|" + file.LastWriteTimeUtc.Ticks;
            if (images.TryGetValue(key, out var cached) && cached.Signature == signature) return cached.Image;
            var image = new BitmapImage { DecodePixelWidth = 512, UriSource = new Uri(path) };
            images[key] = (signature, image);
            return image;
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or ArgumentException) { return null; }
    }

    public BitmapImage? Remote(string peer, string display, string encoded)
    {
        if (encoded.Length == 0 || encoded.Length > 87384) return null;
        string key = peer + ":" + display;
        if (images.TryGetValue(key, out var cached) && cached.Signature == encoded) return cached.Image;
        if (images.Count >= 32) images.Clear();
        try
        {
            byte[] bytes = Convert.FromBase64String(encoded);
            if (bytes.Length < 3 || bytes.Length > 65536 || bytes[0] != 0xff || bytes[1] != 0xd8 || bytes[2] != 0xff) return null;
            var image = new BitmapImage { DecodePixelWidth = 512 };
            images[key] = (encoded, image);
            _ = LoadAsync(image, bytes);
            return image;
        }
        catch (FormatException) { return null; }
    }

    private static async Task LoadAsync(BitmapImage image, byte[] bytes)
    {
        try
        {
            using var stream = new InMemoryRandomAccessStream();
            using (var writer = new DataWriter(stream.GetOutputStreamAt(0)))
            {
                writer.WriteBytes(bytes);
                await writer.StoreAsync();
            }
            stream.Seek(0);
            await image.SetSourceAsync(stream);
        }
        catch (Exception) { /* An unavailable wallpaper leaves the neutral tile visible. */ }
    }

    private void RefreshLocal()
    {
        nextRefresh = DateTime.UtcNow.AddSeconds(15);
        localPaths.Clear();
        IDesktopWallpaper? wallpaper = null;
        try
        {
            wallpaper = (IDesktopWallpaper)Activator.CreateInstance(Type.GetTypeFromCLSID(new Guid("C2CF3110-460E-4FC1-B9D0-8A1C0C9CC4BD"))!)!;
            var monitors = MonitorLayout.All();
            uint count = Math.Min(wallpaper.GetMonitorDevicePathCount(), 32);
            for (uint index = 0; index < count; index++)
            {
                string id = wallpaper.GetMonitorDevicePathAt(index);
                wallpaper.GetMonitorRECT(id, out var rect);
                var monitor = monitors.FirstOrDefault(m => m.Left == rect.Left && m.Top == rect.Top
                    && m.Width == rect.Right - rect.Left && m.Height == rect.Bottom - rect.Top);
                if (monitor is not null) localPaths[monitor.DeviceName] = wallpaper.GetWallpaper(id);
            }
        }
        catch (COMException) { }
        finally { if (wallpaper is not null) Marshal.ReleaseComObject(wallpaper); }
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct Rect { public int Left, Top, Right, Bottom; }

    [ComImport, Guid("B92B56A9-8B55-4E14-9A89-0199BBB6F93B"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IDesktopWallpaper
    {
        void SetWallpaper([MarshalAs(UnmanagedType.LPWStr)] string monitorId, [MarshalAs(UnmanagedType.LPWStr)] string wallpaper);
        [return: MarshalAs(UnmanagedType.LPWStr)] string GetWallpaper([MarshalAs(UnmanagedType.LPWStr)] string monitorId);
        [return: MarshalAs(UnmanagedType.LPWStr)] string GetMonitorDevicePathAt(uint index);
        uint GetMonitorDevicePathCount();
        void GetMonitorRECT([MarshalAs(UnmanagedType.LPWStr)] string monitorId, out Rect rect);
    }
}
