using Microsoft.UI.Xaml.Media.Imaging;
using Windows.Storage.Streams;

namespace DeskFuse.Windows;

/// <summary>
/// PNG to WinUI image conversion for clipboard previews. The PNG dimensions were
/// already bounded when the clipboard content was read, so this only wraps the
/// bytes in an in-memory stream; no HTML and no file body is touched.
/// </summary>
internal static class ClipboardImages
{
    public static BitmapImage? Create(byte[] png)
    {
        if (png.Length < 24 || png.Length > NativeClipboard.MaxContentBytes) return null;
        try
        {
            var stream = new InMemoryRandomAccessStream();
            using (var writer = new DataWriter(stream))
            {
                writer.WriteBytes(png);
                writer.StoreAsync().AsTask().GetAwaiter().GetResult();
                writer.DetachStream();
            }
            stream.Seek(0);
            var image = new BitmapImage();
            image.SetSource(stream);
            return image;
        }
        catch (Exception ex) when (ex is ArgumentException or IOException or InvalidOperationException or OutOfMemoryException) { return null; }
    }
}
