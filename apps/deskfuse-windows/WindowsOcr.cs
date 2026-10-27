using Windows.Graphics.Imaging;
using Windows.Media.Ocr;
using Windows.Storage.Streams;

namespace DeskFuse.Windows;

/// Local Windows OCR. Images and recognized text never leave the trusted-device library channel.
internal static class WindowsOcr
{
    private const int MaximumCharacters = 64 * 1024;

    public static async Task<string> RecognizeAsync(byte[] png, CancellationToken token = default)
    {
        if (png.Length == 0) return "";
        var engine = OcrEngine.TryCreateFromUserProfileLanguages()
            ?? throw new InvalidOperationException("Windows 未安装可用的 OCR 语言，请在系统语言设置中安装识别语言。");
        using var stream = new InMemoryRandomAccessStream();
        var writer = new DataWriter(stream);
        writer.WriteBytes(png);
        await writer.StoreAsync().AsTask(token);
        writer.DetachStream();
        writer.Dispose();
        stream.Seek(0);
        var decoder = await BitmapDecoder.CreateAsync(stream).AsTask(token);
        uint maximum = OcrEngine.MaxImageDimension;
        double scale = Math.Min(1.0, maximum / (double)Math.Max(decoder.PixelWidth, decoder.PixelHeight));
        var transform = new BitmapTransform
        {
            ScaledWidth = Math.Max(1, (uint)Math.Round(decoder.PixelWidth * scale)),
            ScaledHeight = Math.Max(1, (uint)Math.Round(decoder.PixelHeight * scale)),
            InterpolationMode = BitmapInterpolationMode.Fant,
        };
        using var bitmap = await decoder.GetSoftwareBitmapAsync(
            BitmapPixelFormat.Bgra8,
            BitmapAlphaMode.Premultiplied,
            transform,
            ExifOrientationMode.IgnoreExifOrientation,
            ColorManagementMode.DoNotColorManage).AsTask(token);
        var result = await engine.RecognizeAsync(bitmap).AsTask(token);
        string text = string.Join("\n", result.Lines.Select(line => line.Text.Trim()).Where(line => line.Length > 0));
        return text.Length <= MaximumCharacters ? text : text[..MaximumCharacters];
    }
}
