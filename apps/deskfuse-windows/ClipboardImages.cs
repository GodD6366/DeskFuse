using System.Runtime.InteropServices.WindowsRuntime;
using Microsoft.UI.Xaml.Media.Imaging;
using Windows.Storage.Streams;

namespace DeskFuse.Windows;

internal static class ClipboardImages
{
    // WinRT I/O and image decoding must never synchronously wait on the UI
    // dispatcher. Decode only display-sized pixels, not the original bitmap.
    public static async Task<BitmapImage?> CreateAsync(byte[] bytes, int pixelSize, CancellationToken token = default)
    {
        if (bytes.Length < 24 || bytes.Length > NativeClipboard.MaxContentBytes) return null;
        try
        {
            using var stream = new InMemoryRandomAccessStream();
            await stream.WriteAsync(bytes.AsBuffer()).AsTask(token);
            stream.Seek(0);
            var image = new BitmapImage
            {
                DecodePixelWidth = pixelSize,
                DecodePixelHeight = pixelSize,
            };
            await image.SetSourceAsync(stream).AsTask(token);
            return image;
        }
        catch (Exception ex) when (ex is ArgumentException or IOException or InvalidOperationException
            or System.Runtime.InteropServices.COMException or OutOfMemoryException) { return null; }
    }
}
