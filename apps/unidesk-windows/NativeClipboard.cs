using System.IO;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Text;
using System.Windows;
using System.Windows.Interop;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using Clipboard = System.Windows.Clipboard;
using DataFormats = System.Windows.DataFormats;
using TextDataFormat = System.Windows.TextDataFormat;

namespace UniDesk.Windows;

public sealed record ClipboardRepresentation(string Mime, byte[] Data);
public sealed record ClipboardContent(string Mime, byte[] Data,
    IReadOnlyList<string>? Files = null,
    IReadOnlyList<ClipboardRepresentation>? Representations = null)
{
    public const string LocalFilesMime = "application/x-unidesk-local-files";
    public long SizeBytes => Data.LongLength + (Representations?.Sum(r => r.Data.LongLength) ?? 0);
    public string Fingerprint()
    {
        using var hash = IncrementalHash.CreateHash(HashAlgorithmName.SHA256);
        void Add(byte[] bytes) { hash.AppendData(BitConverter.GetBytes(bytes.Length)); hash.AppendData(bytes); }
        if (Files is not null)
        {
            Add(Encoding.UTF8.GetBytes(LocalFilesMime));
            foreach (var file in Files) Add(Encoding.UTF8.GetBytes(file));
        }
        else
        {
            // Primary is a view of one representation, not an extra format.
            // Stable order preserves rich formats while ignoring wire enumeration order.
            var representations = (Representations ?? Array.Empty<ClipboardRepresentation>())
                .Append(new ClipboardRepresentation(Mime, Data))
                .GroupBy(r => (r.Mime, Hash: Convert.ToHexString(SHA256.HashData(r.Data))))
                .Select(g => g.First()).OrderBy(r => r.Mime, StringComparer.Ordinal)
                .ThenBy(r => Convert.ToHexString(SHA256.HashData(r.Data)), StringComparer.Ordinal);
            foreach (var rep in representations) { Add(Encoding.UTF8.GetBytes(rep.Mime)); Add(rep.Data); }
        }
        return Convert.ToHexString(hash.GetHashAndReset());
    }
}

/// <summary>Native clipboard listener on the WPF STA. Never evaluates HTML or reads file bodies.</summary>
public sealed class NativeClipboard : IDisposable
{
    private const int WmClipboardUpdate = 0x031D;
    private const string OriginFormat = "com.unidesk.clipboard-origin";
    public const int MaxContentBytes = 16 * 1024 * 1024;
    private HwndSource? source;
    private readonly string origin = Guid.NewGuid().ToString("N");
    private readonly DispatcherTimer debounce = new() { Interval = TimeSpan.FromMilliseconds(100) };
    private uint sequence;
    private IntPtr ownClipboardOwner;
    private uint lastInvalidatedSequence;
    private int retries;
    private bool paused;
    private bool disposed;
    public event Action<ClipboardContent>? Changed;
    public event Action? LocalInvalidated;
    public event Action<string>? Error;
    public uint SequenceNumber => GetClipboardSequenceNumber();

    public NativeClipboard() => debounce.Tick += ReadChanged;
    public void Start(Window window)
    {
        ObjectDisposedException.ThrowIf(disposed, this);
        if (source is not null) return;
        var handle = new WindowInteropHelper(window).EnsureHandle();
        source = HwndSource.FromHwnd(handle) ?? throw new InvalidOperationException("无法连接剪贴板窗口");
        source.AddHook(WindowProcedure);
        if (!AddClipboardFormatListener(handle))
        { source.RemoveHook(WindowProcedure); source = null; throw new InvalidOperationException("无法注册 Windows 剪贴板监听"); }
        // Existing clipboard is a baseline, not a new copy to replay on connection.
        sequence = GetClipboardSequenceNumber();
        lastInvalidatedSequence = sequence;
    }
    private IntPtr WindowProcedure(IntPtr hwnd, int msg, IntPtr wParam, IntPtr lParam, ref bool handled)
    {
        if (msg == WmClipboardUpdate && !paused && !disposed)
        {
            var ownership = TryAcknowledgeOwnWrite(out _);
            if (ownership == true) { debounce.Stop(); return IntPtr.Zero; }
            if (ownership is null) { debounce.Start(); return IntPtr.Zero; }
            var current = GetClipboardSequenceNumber();
            if (current != sequence && current != lastInvalidatedSequence)
            {
                lastInvalidatedSequence = current;
                // Every local change retires older remote work, even formats we
                // cannot decode, oversized data, or temporary clipboard lock errors.
                LocalInvalidated?.Invoke();
            }
            retries = 0; debounce.Stop(); debounce.Start();
        }
        return IntPtr.Zero;
    }
    private void ReadChanged(object? sender, EventArgs args)
    {
        debounce.Stop();
        if (paused || disposed) return;
        var ownership = TryAcknowledgeOwnWrite(out _);
        if (ownership == true) return;
        if (ownership is null) { if (++retries < 5) debounce.Start(); else Error?.Invoke("剪贴板正被占用，暂未确认变更来源"); return; }
        var now = GetClipboardSequenceNumber();
        if (now == sequence) return;
        try
        {
            // Owner + marker, not sequence alone, identifies our completed native write.
            // A foreign writer copying our marker must still be captured.
            var content = Read();
            var afterRead = TryAcknowledgeOwnWrite(out uint checkedSequence);
            if (afterRead == true) return;
            if (afterRead is null || now != checkedSequence || now != GetClipboardSequenceNumber()) { debounce.Start(); return; }
            sequence = now;
            if (content is not null) Changed?.Invoke(content);
        }
        catch (COMException)
        {
            if (++retries < 5) debounce.Start();
            else Error?.Invoke("剪贴板正被其他程序占用，本次复制未读取");
        }
        catch (Exception ex) when (ex is InvalidOperationException or IOException or ArgumentException or NotSupportedException)
        { sequence = now; Error?.Invoke("剪贴板格式无法读取或超出当前容量限制"); }
    }
    public ClipboardContent? Read()
    {
        // File references always win over the shell's accompanying path-as-text.
        if (Clipboard.ContainsFileDropList())
        {
            var paths = Clipboard.GetFileDropList().Cast<string>().ToArray();
            ValidateLocalFiles(paths);
            return new(ClipboardContent.LocalFilesMime, Array.Empty<byte>(), paths);
        }
        var formats = new List<ClipboardRepresentation>();
        if (Clipboard.ContainsText(TextDataFormat.UnicodeText))
            formats.Add(new("text/plain", Bounded(Encoding.UTF8.GetBytes(Clipboard.GetText(TextDataFormat.UnicodeText)))));
        var png = ReadPng();
        if (png is not null) formats.Add(new("image/png", png));
        if (Clipboard.ContainsText(TextDataFormat.Html))
            formats.Add(new("text/html", Bounded(Encoding.UTF8.GetBytes(ExtractHtml(Clipboard.GetText(TextDataFormat.Html))))));
        if (Clipboard.ContainsText(TextDataFormat.Rtf))
            formats.Add(new("text/rtf", Bounded(Encoding.UTF8.GetBytes(Clipboard.GetText(TextDataFormat.Rtf)))));
        if (formats.Count == 0) return null;
        if (formats.Sum(f => (long)f.Data.Length) > MaxContentBytes) throw new InvalidOperationException("剪贴板过大");
        var primary = formats.FirstOrDefault(f => f.Mime == "image/png")
                      ?? formats.FirstOrDefault(f => f.Mime == "text/plain") ?? formats[0];
        return new(primary.Mime, primary.Data, Representations: formats);
    }
    private static byte[] Bounded(byte[] bytes) => bytes.Length <= MaxContentBytes
        ? bytes : throw new InvalidOperationException("剪贴板过大");
    private static byte[]? ReadPng()
    {
        if (Clipboard.ContainsData("PNG") && Clipboard.GetData("PNG") is MemoryStream stream)
        {
            if (stream.Length > MaxContentBytes) throw new InvalidOperationException("图片过大");
            return Bounded(stream.ToArray());
        }
        if (!Clipboard.ContainsImage()) return null;
        var image = Clipboard.GetImage();
        if (image is null) return null;
        if ((long)image.PixelWidth * image.PixelHeight > 24_000_000) throw new InvalidOperationException("图片过大");
        var encoder = new PngBitmapEncoder(); encoder.Frames.Add(BitmapFrame.Create(image));
        using var output = new MemoryStream(); encoder.Save(output); return Bounded(output.ToArray());
    }
    public void Write(ClipboardContent content)
    {
        ObjectDisposedException.ThrowIf(disposed, this);
        if (paused) throw new InvalidOperationException("当前会话不能写入剪贴板");
        if (!WriteCore(content, GetClipboardSequenceNumber(), requireProcessed: false, allowLocalFiles: true))
            throw new InvalidOperationException("剪贴板已变化或被占用，请重试复制");
    }
    // CloseClipboard/format materialization can advance sequence after a write.
    // Match both live native owner and process-random marker under one lock.
    // Never suppress a later genuine copy merely because it carries our marker.
    public bool? TryAcknowledgeOwnWrite(out uint current)
    {
        current = GetClipboardSequenceNumber();
        if (paused || disposed || source is null) return null;
        if (ownClipboardOwner == IntPtr.Zero) return false;
        if (!OpenClipboard(source.Handle)) return null;
        try
        {
            current = GetClipboardSequenceNumber();
            if (GetClipboardOwner() != ownClipboardOwner) return false;
            var marker = HasOwnMarker();
            if (marker != true) return marker;
            sequence = current; lastInvalidatedSequence = current;
            return true;
        }
        finally { CloseClipboard(); }
    }
    private bool? HasOwnMarker()
    {
        var memory = GetClipboardData(RegisterClipboardFormat(OriginFormat));
        var expected = Encoding.Unicode.GetBytes(origin + "\0");
        if (memory == IntPtr.Zero || GlobalSize(memory) < (nuint)expected.Length) return false;
        var pointer = GlobalLock(memory);
        if (pointer == IntPtr.Zero) return null;
        try
        {
            var actual = new byte[expected.Length]; Marshal.Copy(pointer, actual, 0, actual.Length);
            return actual.AsSpan().SequenceEqual(expected);
        }
        finally { GlobalUnlock(memory); }
    }
    /// <summary>Prepare first, then compare and commit under the Windows clipboard lock.</summary>
    public bool TryWrite(ClipboardContent content, uint expectedSequence) =>
        WriteCore(content, expectedSequence, requireProcessed: true, allowLocalFiles: false);
    private bool WriteCore(ClipboardContent content, uint expectedSequence, bool requireProcessed, bool allowLocalFiles)
    {
        if (paused || disposed || source is null || (!allowLocalFiles && content.Files is not null)) return false;
        var formats = content.Representations ?? new[] { new ClipboardRepresentation(content.Mime, content.Data) };
        if (content.Files is null && (formats.Count is 0 or > 4 || formats.Sum(f => (long)f.Data.Length) > MaxContentBytes))
            throw new InvalidOperationException("剪贴板超过容量限制");
        var prepared = new List<(uint Format, IntPtr Memory)>();
        void Add(uint format, byte[] bytes)
        {
            if (format == 0 || prepared.Any(p => p.Format == format)) throw new InvalidOperationException("剪贴板格式无效");
            var memory = GlobalAlloc(0x0002, (nuint)Math.Max(1, bytes.Length)); // GMEM_MOVEABLE
            if (memory == IntPtr.Zero) throw new OutOfMemoryException();
            var pointer = GlobalLock(memory);
            if (pointer == IntPtr.Zero) { GlobalFree(memory); throw new InvalidOperationException("剪贴板内存不可用"); }
            try { Marshal.Copy(bytes, 0, pointer, bytes.Length); }
            finally { GlobalUnlock(memory); }
            prepared.Add((format, memory));
        }
        try
        {
            // WIC decode and memory allocation occur before the sequence check,
            // never after it: another application can copy while this work runs.
            if (content.Files is { } files)
            {
                if (content.Mime != ClipboardContent.LocalFilesMime) throw new InvalidOperationException("文件引用无效");
                ValidateLocalFiles(files);
                // Native CF_HDROP: DROPFILES header + double-NUL terminated UTF-16 paths.
                // No file contents are read and only COPY is advertised.
                var paths = Bounded(Encoding.Unicode.GetBytes(string.Join("\0", files) + "\0\0"));
                var drop = new byte[checked(20 + paths.Length)];
                System.Buffers.Binary.BinaryPrimitives.WriteInt32LittleEndian(drop.AsSpan(0, 4), 20);
                System.Buffers.Binary.BinaryPrimitives.WriteInt32LittleEndian(drop.AsSpan(16, 4), 1);
                paths.CopyTo(drop, 20);
                Add(15, drop);
                Add(RegisterClipboardFormat("Preferred DropEffect"), new byte[] { 1, 0, 0, 0 });
            }
            else foreach (var rep in formats)
            {
                switch (rep.Mime)
                {
                    case "text/plain": Add(13, Encoding.Unicode.GetBytes(StrictUtf8(rep.Data) + "\0")); break;
                    case "text/html": Add(RegisterClipboardFormat("HTML Format"), Encoding.UTF8.GetBytes(FormatHtml(StrictUtf8(rep.Data)) + "\0")); break;
                    case "text/rtf": Add(RegisterClipboardFormat("Rich Text Format"), Encoding.UTF8.GetBytes(StrictUtf8(rep.Data) + "\0")); break;
                    case "image/png":
                        var image = DecodeImage(rep.Data);
                        Add(RegisterClipboardFormat("PNG"), rep.Data);
                        Add(17, DibV5(image)); // CF_DIBV5 fallback for native Windows consumers
                        break;
                    default: throw new NotSupportedException("暂不支持该剪贴板格式");
                }
            }
            Add(RegisterClipboardFormat(OriginFormat), Encoding.Unicode.GetBytes(origin + "\0"));
            if (!OpenClipboard(source.Handle)) return false;
            var mutated = false;
            try
            {
                // EmptyClipboard/SetClipboardData stay in this same critical section.
                // A later local copy can only happen after ours and therefore wins.
                if (paused || GetClipboardSequenceNumber() != expectedSequence || (requireProcessed && sequence != expectedSequence)) return false;
                if (!EmptyClipboard()) throw new InvalidOperationException("无法更新剪贴板");
                mutated = true;
                for (var i = 0; i < prepared.Count; i++)
                {
                    var item = prepared[i];
                    if (SetClipboardData(item.Format, item.Memory) == IntPtr.Zero) throw new InvalidOperationException("剪贴板写入失败");
                    prepared[i] = (item.Format, IntPtr.Zero); // ownership transferred to Windows
                }
            }
            finally
            {
                // Sample while locked, otherwise a subsequent external copy could
                // be mistaken for our own write and never captured by the listener.
                if (mutated) { ownClipboardOwner = source.Handle; sequence = GetClipboardSequenceNumber(); }
                CloseClipboard();
            }
            return true;
        }
        finally { foreach (var item in prepared) if (item.Memory != IntPtr.Zero) GlobalFree(item.Memory); }
    }
    private static byte[] DibV5(BitmapSource image)
    {
        var pixels = new System.Windows.Media.Imaging.FormatConvertedBitmap(image, System.Windows.Media.PixelFormats.Bgra32, null, 0);
        var stride = checked(pixels.PixelWidth * 4);
        var size = checked(stride * pixels.PixelHeight);
        var dib = new byte[checked(124 + size)];
        void I32(int offset, int value) => System.Buffers.Binary.BinaryPrimitives.WriteInt32LittleEndian(dib.AsSpan(offset, 4), value);
        I32(0, 124); I32(4, pixels.PixelWidth); I32(8, -pixels.PixelHeight);
        dib[12] = 1; dib[14] = 32; // planes=1, bit count=32
        I32(16, 3); I32(20, size); // BI_BITFIELDS, image size
        I32(40, 0x00ff0000); I32(44, 0x0000ff00); I32(48, 0x000000ff); I32(52, unchecked((int)0xff000000));
        I32(56, 0x73524742); // LCS_sRGB
        I32(108, 4); // LCS_GM_IMAGES
        pixels.CopyPixels(new Int32Rect(0, 0, pixels.PixelWidth, pixels.PixelHeight), dib, stride, 124);
        return dib;
    }
    public static BitmapSource DecodeImage(byte[] png)
    {
        // Check PNG IHDR dimensions before invoking WIC to bound decompression.
        ReadOnlySpan<byte> signature = [137, 80, 78, 71, 13, 10, 26, 10];
        if (png.Length < 24 || png.Length > MaxContentBytes || !png.AsSpan(0, 8).SequenceEqual(signature)
            || !png.AsSpan(12, 4).SequenceEqual("IHDR"u8)) throw new InvalidOperationException("PNG 无效");
        var width = System.Buffers.Binary.BinaryPrimitives.ReadUInt32BigEndian(png.AsSpan(16, 4));
        var height = System.Buffers.Binary.BinaryPrimitives.ReadUInt32BigEndian(png.AsSpan(20, 4));
        if (width == 0 || height == 0 || (ulong)width * height > 24_000_000) throw new InvalidOperationException("图片过大");
        using var input = new MemoryStream(png, writable: false);
        var decoder = new PngBitmapDecoder(input, BitmapCreateOptions.PreservePixelFormat, BitmapCacheOption.OnLoad);
        var image = decoder.Frames[0]; image.Freeze(); return image;
    }
    private static string StrictUtf8(byte[] bytes) => new UTF8Encoding(false, true).GetString(bytes);
    public static void ValidateLocalFiles(IReadOnlyList<string> files)
    {
        if (files.Count == 0 || files.Count > 512) throw new InvalidOperationException("文件数量必须为 1–512");
        foreach (var path in files)
        {
            if (string.IsNullOrWhiteSpace(path) || !Path.IsPathFullyQualified(path)
                || path.StartsWith(@"\\", StringComparison.Ordinal) || path.Length > 32760
                || (!File.Exists(path) && !Directory.Exists(path)))
                throw new IOException("文件不可用；本地暂存区只接收现有本机文件");
        }
    }
    // CF_HTML offsets are byte offsets in UTF-8, not C# character positions.
    private static string ExtractHtml(string html)
    {
        var bytes = Encoding.UTF8.GetBytes(html);
        int Offset(string key)
        {
            var start = html.IndexOf(key, StringComparison.Ordinal);
            if (start < 0) return -1;
            var end = html.IndexOf('\n', start);
            return int.TryParse(html.AsSpan(start + key.Length, (end < 0 ? html.Length : end) - start - key.Length).Trim(), out var n) ? n : -1;
        }
        var from = Offset("StartFragment:"); var to = Offset("EndFragment:");
        if (from >= 0 && to >= from && to <= bytes.Length) return Encoding.UTF8.GetString(bytes, from, to - from);
        // Do not sync the Windows-only header or SourceURL to another platform.
        var tag = html.IndexOf('<'); return tag >= 0 ? html[tag..] : html;
    }
    private static string FormatHtml(string fragment)
    {
        const string header = "Version:1.0\r\nStartHTML:{0:D10}\r\nEndHTML:{1:D10}\r\nStartFragment:{2:D10}\r\nEndFragment:{3:D10}\r\n";
        const string before = "<html><body><!--StartFragment-->";
        const string after = "<!--EndFragment--></body></html>";
        var start = Encoding.UTF8.GetByteCount(string.Format(System.Globalization.CultureInfo.InvariantCulture, header, 0, 0, 0, 0));
        var fragmentStart = start + Encoding.UTF8.GetByteCount(before);
        var fragmentEnd = fragmentStart + Encoding.UTF8.GetByteCount(fragment);
        return string.Format(System.Globalization.CultureInfo.InvariantCulture, header, start, fragmentEnd + Encoding.UTF8.GetByteCount(after), fragmentStart, fragmentEnd)
               + before + fragment + after;
    }
    public void Pause() { paused = true; debounce.Stop(); sequence = GetClipboardSequenceNumber(); lastInvalidatedSequence = sequence; }
    public void Resume() { sequence = GetClipboardSequenceNumber(); lastInvalidatedSequence = sequence; paused = false; }
    public void Dispose()
    {
        if (disposed) return; disposed = true; debounce.Stop();
        if (source is not null) { RemoveClipboardFormatListener(source.Handle); source.RemoveHook(WindowProcedure); source = null; }
    }
    [DllImport("kernel32.dll", SetLastError = true)] private static extern IntPtr GlobalAlloc(uint flags, nuint size);
    [DllImport("kernel32.dll", SetLastError = true)] private static extern IntPtr GlobalLock(IntPtr handle);
    [DllImport("kernel32.dll")] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool GlobalUnlock(IntPtr handle);
    [DllImport("kernel32.dll")] private static extern IntPtr GlobalFree(IntPtr handle);
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)] private static extern uint RegisterClipboardFormat(string name);
    [DllImport("user32.dll", SetLastError = true)] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool OpenClipboard(IntPtr owner);
    [DllImport("user32.dll")] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool CloseClipboard();
    [DllImport("user32.dll", SetLastError = true)] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool EmptyClipboard();
    [DllImport("user32.dll", SetLastError = true)] private static extern IntPtr SetClipboardData(uint format, IntPtr memory);
    [DllImport("user32.dll", SetLastError = true)] [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool AddClipboardFormatListener(IntPtr hwnd);
    [DllImport("user32.dll", SetLastError = true)] [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool RemoveClipboardFormatListener(IntPtr hwnd);
    [DllImport("user32.dll")] private static extern uint GetClipboardSequenceNumber();
    [DllImport("user32.dll")] private static extern IntPtr GetClipboardOwner();
    [DllImport("user32.dll")] private static extern IntPtr GetClipboardData(uint format);
    [DllImport("kernel32.dll")] private static extern nuint GlobalSize(IntPtr memory);
}
