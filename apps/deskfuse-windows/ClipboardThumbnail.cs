using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;

namespace DeskFuse.Windows;

/// One display-sized cache shared by list and masonry. Serialize reads through
/// decoding (macOS parity); recycled views cancel their queued work.
internal sealed class ClipboardThumbnailCache(DesktopClipboardController clipboard)
{
    private readonly BoundedCache<(string Id, bool Available, long Size, int Pixels), BitmapImage> images =
        new(96, 16 * 1024 * 1024);
    private readonly SemaphoreSlim worker = new(1, 1);

    public async Task<BitmapImage?> GetAsync(ClipboardEntry entry, int pixels, CancellationToken token)
    {
        var key = (entry.Id, entry.Available, entry.SizeBytes, pixels);
        if (images.TryGet(key, out var cached)) return cached;
        await worker.WaitAsync(token);
        try
        {
            token.ThrowIfCancellationRequested();
            if (images.TryGet(key, out cached)) return cached;
            var bytes = await clipboard.ThumbnailAsync(entry, pixels, token);
            if (bytes is not { Length: > 0 }) return null;
            var image = await ClipboardImages.CreateAsync(bytes, pixels, token);
            token.ThrowIfCancellationRequested();
            if (image is not null) images.Put(key, image, (long)image.PixelWidth * image.PixelHeight * 4);
            // No negative cache: a missing remote blob or transient IPC error
            // must be retried when the row next becomes visible.
            return image;
        }
        finally { worker.Release(); }
    }
}

public sealed class ClipboardThumbnail : Grid
{
    public static readonly DependencyProperty EntryProperty = DependencyProperty.Register(
        nameof(Entry), typeof(ClipboardEntry), typeof(ClipboardThumbnail),
        new PropertyMetadata(null, (sender, _) => ((ClipboardThumbnail)sender).Refresh()));
    public ClipboardEntry? Entry { get => (ClipboardEntry?)GetValue(EntryProperty); set => SetValue(EntryProperty, value); }
    /// <summary>One decode size serves the 42 pt list row and the ~150 pt grid
    /// card, so switching display mode reuses the cache instead of decoding
    /// every visible image again.</summary>
    public int PixelSize { get; set; } = 320;
    internal ClipboardThumbnailCache? Cache { get; set; }
    private readonly Image image = new() { Stretch = Stretch.Uniform };
    private readonly FontIcon placeholder = new() { FontSize = 20 };
    private CancellationTokenSource? request;

    public ClipboardThumbnail()
    {
        Children.Add(placeholder);
        Children.Add(image);
        Loaded += (_, _) => Refresh();
        Unloaded += (_, _) =>
        {
            Cancel();
            image.Source = null;
        };
    }

    private void Cancel()
    {
        request?.Cancel();
        request?.Dispose();
        request = null;
    }

    internal void Refresh()
    {
        Cancel();
        image.Source = null;
        placeholder.Glyph = Entry?.KindGlyph ?? "";
        placeholder.Foreground = DfUi.Muted;
        placeholder.Visibility = Visibility.Visible;
        if (!IsLoaded || Cache is null || Entry is not { } entry || !WantsThumbnail(entry)) return;
        request = new CancellationTokenSource();
        _ = LoadAsync(entry, request.Token);
    }

    private static bool WantsThumbnail(ClipboardEntry entry) => entry.Kind switch
    {
        "image" => true,
        "files" => entry.Content.Files is { Count: 1 } files
            ? IsImageFile(files[0])
            : entry.CanonicalFileCount == 1 && IsImageFile(entry.Title),
        _ => false,
    };

    private static bool IsImageFile(string path) =>
        Path.GetExtension(path).ToLowerInvariant() is ".png" or ".jpg" or ".jpeg" or ".gif" or ".webp" or ".bmp" or ".heic" or ".heif" or ".tif" or ".tiff";

    private async Task LoadAsync(ClipboardEntry entry, CancellationToken token)
    {
        try
        {
            var source = await Cache!.GetAsync(entry, PixelSize, token);
            if (token.IsCancellationRequested || !ReferenceEquals(Entry, entry) || !IsLoaded) return;
            image.Source = source;
            placeholder.Visibility = source is null ? Visibility.Visible : Visibility.Collapsed;
        }
        catch (OperationCanceledException) { }
        catch (Exception ex) when (ex is IOException or InvalidOperationException or TimeoutException
            or System.Runtime.InteropServices.COMException) { }
    }
}
