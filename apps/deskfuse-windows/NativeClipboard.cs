using System.Diagnostics;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Runtime.InteropServices.ComTypes;
using System.Text;
using Microsoft.UI.Dispatching;

namespace DeskFuse.Windows;

/// <summary>
/// Win32 clipboard listener on the UI thread. Replaces the WPF client's
/// System.Windows.Clipboard/WIC usage with raw Win32 + GDI+: the sequence and
/// owner checks, format set and size limits are unchanged, and no HTML is
/// evaluated and no file body is read.
/// </summary>
public sealed class NativeClipboard : IDisposable
{
    private const uint WmClipboardUpdate = 0x031D;
    private const uint CfText = 13;
    private const uint CfHdrop = 15;
    private const uint CfDib = 8;
    private const uint CfDibV5 = 17;
    private const string OriginFormat = "com.deskfuse.clipboard-origin";
    public const int MaxContentBytes = 16 * 1024 * 1024;
    private readonly DispatcherQueueTimer debounce;
    private readonly string origin = Guid.NewGuid().ToString("N");
    private IntPtr window;
    private uint sequence;
    private IntPtr ownClipboardOwner;
    private uint lastInvalidatedSequence;
    /// <summary>Tick count of the last clipboard change this process did not make.</summary>
    private long lastForeignChangeTicks;
    private int retries;
    private bool paused;
    private bool disposed;
    public event Action<ClipboardContent>? Changed;
    public event Action? LocalInvalidated;
    public event Action<string>? Error;
    public uint SequenceNumber => GetClipboardSequenceNumber();
    /// <summary>
    /// Milliseconds since another application last wrote the clipboard. Remote
    /// clipboard application uses this to leave a copy the user just made
    /// (a screenshot, for example) intact instead of replacing its formats.
    /// </summary>
    public long MillisecondsSinceForeignChange =>
        lastForeignChangeTicks == 0 ? long.MaxValue : Environment.TickCount64 - lastForeignChangeTicks;

    public NativeClipboard(DispatcherQueue dispatcher)
    {
        debounce = dispatcher.CreateTimer();
        debounce.Interval = TimeSpan.FromMilliseconds(100);
        debounce.IsRepeating = false;
        debounce.Tick += (_, _) => ReadChanged();
    }

    public void Start(IntPtr hwnd)
    {
        ObjectDisposedException.ThrowIf(disposed, this);
        if (window != IntPtr.Zero) return;
        WindowMessageHook.Attach(hwnd, WindowProcedure);
        if (!AddClipboardFormatListener(hwnd))
        {
            WindowMessageHook.Detach(hwnd);
            throw new InvalidOperationException("无法注册 Windows 剪贴板监听");
        }
        window = hwnd;
        // Existing clipboard is a baseline, not a new copy to replay on connection.
        sequence = GetClipboardSequenceNumber();
        lastInvalidatedSequence = sequence;
    }

    private bool WindowProcedure(uint message, IntPtr wParam, IntPtr lParam)
    {
        if (message != WmClipboardUpdate || paused || disposed) return false;
        var ownership = TryAcknowledgeOwnWrite(out _);
        if (ownership is true) { debounce.Stop(); return true; }
        if (ownership is null) { debounce.Start(); return true; }
        var current = GetClipboardSequenceNumber();
        if (current != sequence && current != lastInvalidatedSequence)
        {
            lastInvalidatedSequence = current;
            // Every local change retires older remote work, even formats we
            // cannot decode, oversized data, or temporary clipboard lock errors.
            LocalInvalidated?.Invoke();
        }
        retries = 0; debounce.Stop(); debounce.Start();
        return true;
    }

    private void ReadChanged()
    {
        debounce.Stop();
        if (paused || disposed) return;
        var ownership = TryAcknowledgeOwnWrite(out _);
        if (ownership is true) return;
        if (ownership is null) { if (++retries < 5) debounce.Start(); else Error?.Invoke("剪贴板正被占用，暂未确认变更来源"); return; }
        var now = GetClipboardSequenceNumber();
        if (now == sequence) return;
        try
        {
            // Owner + marker, not sequence alone, identifies our completed native write.
            // A foreign writer copying our marker must still be captured.
            var content = Read();
            var afterRead = TryAcknowledgeOwnWrite(out uint checkedSequence);
            if (afterRead is true) return;
            if (afterRead is null || now != checkedSequence || now != GetClipboardSequenceNumber()) { debounce.Start(); return; }
            sequence = now;
            lastForeignChangeTicks = Environment.TickCount64;
            if (content is not null) Changed?.Invoke(content);
        }
        catch (Exception ex) when (ex is InvalidOperationException or IOException or ArgumentException or NotSupportedException or ExternalException)
        { sequence = now; Error?.Invoke(ex is IOException or InvalidOperationException ? "剪贴板格式无法读取或超出当前容量限制" : "剪贴板正被其他程序占用，本次复制未读取"); }
    }

    public ClipboardContent? Read()
    {
        // File references always win over the shell's accompanying path-as-text.
        var files = ReadFileDropList();
        if (files.Length > 0)
        {
            ValidateLocalFiles(files);
            return new(ClipboardContent.LocalFilesMime, [], files);
        }
        var formats = new List<ClipboardRepresentation>();
        var text = ReadUnicodeText();
        if (text is not null) formats.Add(new("text/plain", Bounded(Encoding.UTF8.GetBytes(text))));
        var png = ReadPng();
        if (png is not null) formats.Add(new("image/png", png));
        var html = ReadFormatBytes(RegisterClipboardFormat("HTML Format"));
        if (html is not null) formats.Add(new("text/html", Bounded(Encoding.UTF8.GetBytes(ExtractHtml(DecodeHtml(html))))));
        var rtf = ReadFormatBytes(RegisterClipboardFormat("Rich Text Format"));
        if (rtf is not null) formats.Add(new("text/rtf", Bounded(TrimTerminators(rtf))));
        if (formats.Count == 0) return null;
        if (formats.Sum(f => (long)f.Data.Length) > MaxContentBytes) throw new InvalidOperationException("剪贴板过大");
        var primary = formats.FirstOrDefault(f => f.Mime == "image/png")
                      ?? formats.FirstOrDefault(f => f.Mime == "text/plain") ?? formats[0];
        return new(primary.Mime, primary.Data, Representations: formats);
    }

    private string[] ReadFileDropList()
    {
        if (!IsClipboardFormatAvailable(CfHdrop) || !OpenClipboard(window)) return [];
        try
        {
            var handle = GetClipboardData(CfHdrop);
            if (handle == IntPtr.Zero) return [];
            var count = DragQueryFile(handle, uint.MaxValue, null, 0);
            if (count is 0 or > 512) return [];
            var paths = new string[count];
            for (uint index = 0; index < count; index++)
            {
                var length = DragQueryFile(handle, index, null, 0);
                if (length == 0) return [];
                var buffer = new StringBuilder((int)length + 1);
                if (DragQueryFile(handle, index, buffer, (uint)buffer.Capacity) == 0) return [];
                paths[index] = buffer.ToString();
            }
            return paths;
        }
        finally { CloseClipboard(); }
    }

    private string? ReadUnicodeText()
    {
        var bytes = ReadFormatBytes(CfText);
        if (bytes is null || bytes.Length is 0 or > MaxContentBytes * 2) return null;
        var text = Encoding.Unicode.GetString(bytes).TrimEnd('\0');
        return text.Length == 0 ? null : text;
    }

    private byte[]? ReadPng()
    {
        var png = ReadFormatBytes(RegisterClipboardFormat("PNG"));
        if (png is not null)
        {
            if (png.Length > MaxContentBytes) throw new InvalidOperationException("图片过大");
            return Bounded(png);
        }
        var dib = ReadFormatBytes(CfDibV5) ?? ReadFormatBytes(CfDib);
        return dib is null ? null : PngFromDib(dib);
    }

    /// <summary>Wrap a raw clipboard DIB in a minimal BMP container so GDI+ can decode it.</summary>
    private static byte[] PngFromDib(byte[] dib)
    {
        if (dib.Length < 40) throw new InvalidOperationException("位图数据无效");
        int headerSize = BitConverter.ToInt32(dib, 0);
        int width = BitConverter.ToInt32(dib, 4);
        int height = Math.Abs(BitConverter.ToInt32(dib, 8));
        short bitCount = BitConverter.ToInt16(dib, 14);
        int compression = BitConverter.ToInt32(dib, 16);
        if (headerSize < 40 || width <= 0 || height <= 0 || (long)width * height > 24_000_000 || dib.Length < headerSize)
            throw new InvalidOperationException("图片过大或格式无效");
        int offset = 14 + headerSize;
        if (bitCount <= 8) offset += (1 << bitCount) * 4;
        else if (compression == 3 && headerSize == 40) offset += 12; // BI_BITFIELDS masks
        var file = new byte[14 + dib.Length];
        file[0] = (byte)'B'; file[1] = (byte)'M';
        BitConverter.GetBytes(file.Length).CopyTo(file, 2);
        BitConverter.GetBytes(offset).CopyTo(file, 10);
        dib.CopyTo(file, 14);
        using var bitmap = new Bitmap(new MemoryStream(file, writable: false));
        using var output = new MemoryStream();
        bitmap.Save(output, ImageFormat.Png);
        return Bounded(output.ToArray());
    }

    private byte[]? ReadFormatBytes(uint format)
    {
        if (format == 0 || !IsClipboardFormatAvailable(format) || !OpenClipboard(window)) return null;
        try
        {
            var handle = GetClipboardData(format);
            if (handle == IntPtr.Zero) return null;
            var size = (int)GlobalSize(handle);
            if (size <= 0) return null;
            var pointer = GlobalLock(handle);
            if (pointer == IntPtr.Zero) throw new InvalidOperationException("剪贴板内存不可用");
            try
            {
                var bytes = new byte[size];
                Marshal.Copy(pointer, bytes, 0, size);
                return bytes;
            }
            finally { GlobalUnlock(handle); }
        }
        finally { CloseClipboard(); }
    }

    private static string DecodeHtml(byte[] bytes)
    {
        // CF_HTML is a UTF-8 byte stream whose offsets are byte offsets.
        var length = bytes.Length;
        while (length > 0 && bytes[length - 1] == 0) length--;
        return Encoding.UTF8.GetString(bytes, 0, length);
    }

    private static byte[] TrimTerminators(byte[] bytes)
    {
        var length = bytes.Length;
        while (length > 0 && bytes[length - 1] == 0) length--;
        return bytes[..length];
    }

    private static byte[] Bounded(byte[] bytes) => bytes.Length <= MaxContentBytes
        ? bytes : throw new InvalidOperationException("剪贴板过大");

    public void WriteOle(object dataObject)
    {
        ObjectDisposedException.ThrowIf(disposed, this);
        if (paused) throw new InvalidOperationException("当前会话不能写入剪贴板");
        if (dataObject is not IDataObject ole) throw new InvalidOperationException("文件数据对象无效");
        int result = OleSetClipboard(ole);
        if (result != 0) Marshal.ThrowExceptionForHR(result);
        result = OleFlushClipboard();
        if (result != 0) Marshal.ThrowExceptionForHR(result);
        sequence = GetClipboardSequenceNumber();
        lastInvalidatedSequence = sequence;
        ownClipboardOwner = window;
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
        if (paused || disposed || window == IntPtr.Zero) return null;
        if (ownClipboardOwner == IntPtr.Zero) return false;
        if (!OpenClipboard(window)) return null;
        try
        {
            current = GetClipboardSequenceNumber();
            if (GetClipboardOwner() != ownClipboardOwner) return false;
            var marker = HasOwnMarker();
            if (marker is not true) return marker;
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
            var actual = new byte[expected.Length];
            Marshal.Copy(pointer, actual, 0, actual.Length);
            return actual.AsSpan().SequenceEqual(expected);
        }
        finally { GlobalUnlock(memory); }
    }

    /// <summary>Prepare first, then compare and commit under the Windows clipboard lock.</summary>
    public bool TryWrite(ClipboardContent content, uint expectedSequence) =>
        WriteCore(content, expectedSequence, requireProcessed: true, allowLocalFiles: false);

    private bool WriteCore(ClipboardContent content, uint expectedSequence, bool requireProcessed, bool allowLocalFiles)
    {
        if (paused || disposed || window == IntPtr.Zero || (!allowLocalFiles && content.Files is not null)) return false;
        var formats = content.Representations ?? [new ClipboardRepresentation(content.Mime, content.Data)];
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
            // Decode and allocation happen before the sequence check, never after
            // it: another application can copy while this work runs.
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
                Add(CfHdrop, drop);
                Add(RegisterClipboardFormat("Preferred DropEffect"), [1, 0, 0, 0]);
            }
            else foreach (var rep in formats)
            {
                switch (rep.Mime)
                {
                    case "text/plain": Add(CfText, Encoding.Unicode.GetBytes(StrictUtf8(rep.Data) + "\0")); break;
                    case "text/html": Add(RegisterClipboardFormat("HTML Format"), Encoding.UTF8.GetBytes(FormatHtml(StrictUtf8(rep.Data)) + "\0")); break;
                    case "text/rtf": Add(RegisterClipboardFormat("Rich Text Format"), Encoding.UTF8.GetBytes(StrictUtf8(rep.Data) + "\0")); break;
                    case "image/png":
                        Add(RegisterClipboardFormat("PNG"), rep.Data);
                        Add(CfDibV5, DibV5(rep.Data)); // CF_DIBV5 fallback for native consumers
                        break;
                    case "image/jpeg":
                    case "image/webp":
                    {
                        // Workflow conversion may return JPEG/WebP. Windows apps
                        // consume the native bitmap formats, so publish a PNG
                        // representation plus CF_DIBV5 from the decoded pixels.
                        using var bitmap = new Bitmap(new MemoryStream(rep.Data, writable: false));
                        using var encoded = new MemoryStream();
                        bitmap.Save(encoded, ImageFormat.Png);
                        var png = encoded.ToArray();
                        Add(RegisterClipboardFormat("PNG"), png);
                        Add(CfDibV5, DibV5(png));
                        break;
                    }
                    default: throw new NotSupportedException("暂不支持该剪贴板格式");
                }
            }
            Add(RegisterClipboardFormat(OriginFormat), Encoding.Unicode.GetBytes(origin + "\0"));
            if (!OpenClipboard(window)) return false;
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
                // Sample while locked, otherwise a subsequent external copy could be
                // mistaken for our own write and never captured by the listener.
                if (mutated) { ownClipboardOwner = window; sequence = GetClipboardSequenceNumber(); }
                CloseClipboard();
            }
            return true;
        }
        finally { foreach (var item in prepared) if (item.Memory != IntPtr.Zero) GlobalFree(item.Memory); }
    }

    private static byte[] DibV5(byte[] png)
    {
        using var bitmap = new Bitmap(new MemoryStream(png, writable: false));
        var width = bitmap.Width;
        var height = bitmap.Height;
        var stride = checked(width * 4);
        var size = checked(stride * height);
        var dib = new byte[checked(124 + size)];
        void I32(int offset, int value) => System.Buffers.Binary.BinaryPrimitives.WriteInt32LittleEndian(dib.AsSpan(offset, 4), value);
        I32(0, 124); I32(4, width); I32(8, -height); // negative height: top-down rows
        dib[12] = 1; dib[14] = 32; // planes=1, bit count=32
        I32(16, 3); I32(20, size); // BI_BITFIELDS, image size
        I32(40, 0x00ff0000); I32(44, 0x0000ff00); I32(48, 0x000000ff); I32(52, unchecked((int)0xff000000));
        I32(56, 0x73524742); // LCS_sRGB
        I32(108, 4); // LCS_GM_IMAGES
        var data = bitmap.LockBits(new Rectangle(0, 0, width, height), ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
        try
        {
            for (var row = 0; row < height; row++)
                Marshal.Copy(data.Scan0 + row * data.Stride, dib, 124 + row * stride, stride);
        }
        finally { bitmap.UnlockBits(data); }
        return dib;
    }

    private static string StrictUtf8(byte[] bytes) => new UTF8Encoding(false, true).GetString(bytes);

    /// <summary>Foreground process at copy time, used as the mac-style source label.</summary>
    internal static string ForegroundProcessName()
    {
        try
        {
            IntPtr hwnd = GetForegroundWindow();
            if (hwnd == IntPtr.Zero) return "";
            GetWindowThreadProcessId(hwnd, out uint pid);
            using var process = Process.GetProcessById((int)pid);
            return process.ProcessName;
        }
        catch (Exception) { return ""; }
    }

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
        if (disposed) return;
        disposed = true; debounce.Stop();
        if (window != IntPtr.Zero)
        {
            RemoveClipboardFormatListener(window);
            WindowMessageHook.Detach(window);
            window = IntPtr.Zero;
        }
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
    [DllImport("user32.dll", SetLastError = true)] private static extern IntPtr GetClipboardData(uint format);
    [DllImport("user32.dll")] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool IsClipboardFormatAvailable(uint format);
    [DllImport("user32.dll", SetLastError = true)] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool AddClipboardFormatListener(IntPtr hwnd);
    [DllImport("user32.dll", SetLastError = true)] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool RemoveClipboardFormatListener(IntPtr hwnd);
    [DllImport("user32.dll")] private static extern uint GetClipboardSequenceNumber();
    [DllImport("user32.dll")] private static extern IntPtr GetClipboardOwner();
    [DllImport("user32.dll")] private static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("kernel32.dll")] private static extern nuint GlobalSize(IntPtr memory);
    [DllImport("shell32.dll", EntryPoint = "DragQueryFileW", CharSet = CharSet.Unicode)] private static extern uint DragQueryFile(IntPtr drop, uint index, StringBuilder? file, uint size);
    [DllImport("ole32.dll")] private static extern int OleSetClipboard(IDataObject dataObject);
    [DllImport("ole32.dll")] private static extern int OleFlushClipboard();
}
