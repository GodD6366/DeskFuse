using System.Runtime.InteropServices;
using System.Runtime.InteropServices.ComTypes;
using System.Text;
using ComDataObject = System.Runtime.InteropServices.ComTypes.IDataObject;

namespace DeskFuse.Windows;

/// Relative tree metadata used by the OLE object. Lindex is assigned by the
/// sender and is stable across descriptor and FileContents requests.
public sealed class ExplorerOleFileEntry
{
    public ExplorerOleFileEntry(string relativePath, bool isDirectory, long size, int lindex, string? localPath = null)
    {
        RelativePath = relativePath;
        IsDirectory = isDirectory;
        Size = size;
        Lindex = lindex;
        LocalPath = localPath;
    }

    public string RelativePath { get; }
    public bool IsDirectory { get; }
    public long Size { get; }
    public int Lindex { get; }
    public string? LocalPath { get; }

    public ExplorerOleFileEntry WithLocalPath(string path) => new(RelativePath, IsDirectory, Size, Lindex, path);
}

/// OLE data object for Explorer paste. Metadata enumeration is side-effect free;
/// the first GetData(FileGroupDescriptorW) materializes the verified service
/// staging files. Explorer may therefore download while previewing formats.
[ComVisible(true)]
public sealed class ExplorerOleFileDataObject : ComDataObject
{
    private const int DVASPECT_CONTENT = 1;
    private const int TYMED_HGLOBAL = 1;
    private const int S_OK = 0;
    private const int DV_E_FORMATETC = unchecked((int)0x80040064);
    private const int FD_ATTRIBUTES = 0x00000004;
    private const int FD_FILESIZE = 0x00000040;
    private const int FILE_ATTRIBUTE_DIRECTORY = 0x00000010;
    private const int FILE_ATTRIBUTE_NORMAL = 0x00000080;
    private readonly Func<IReadOnlyList<ExplorerOleFileEntry>> materialize;
    private readonly object gate = new();
    private readonly ExplorerOleFileEntry[] metadata;
    private ExplorerOleFileEntry[]? files;
    private readonly short descriptorFormat = Register("FileGroupDescriptorW");
    private readonly short contentsFormat = Register("FileContents");

    public ExplorerOleFileDataObject(
        IReadOnlyList<ExplorerOleFileEntry> metadata,
        Func<IReadOnlyList<ExplorerOleFileEntry>> materialize)
    {
        this.metadata = Normalize(metadata);
        this.materialize = materialize;
    }

    /// Compatibility constructor for callers that only know a flat path list.
    /// It keeps the old deferred callback contract while exposing one stable
    /// file index per path.
    public ExplorerOleFileDataObject(Func<IReadOnlyList<string>> materialize)
    {
        metadata = Array.Empty<ExplorerOleFileEntry>();
        this.materialize = () => materialize().Select((path, index) =>
            new ExplorerOleFileEntry(Path.GetFileName(path), false, new FileInfo(path).Length, index, path)).ToArray();
    }

    private static ExplorerOleFileEntry[] Normalize(IEnumerable<ExplorerOleFileEntry> entries)
    {
        var result = entries.OrderBy(entry => entry.Lindex).ToArray();
        if (result.Length == 0) return result;
        if (result.Select((entry, index) => entry.Lindex == index).Any(valid => !valid))
            throw new ArgumentException("OLE file indexes must be contiguous and stable", nameof(entries));
        return result;
    }

    private ExplorerOleFileEntry[] Materialized()
    {
        lock (gate)
        {
            if (files is not null) return files;
            var resolved = Normalize(materialize());
            if (resolved.Any(entry => !entry.IsDirectory && (entry.LocalPath is null || !File.Exists(entry.LocalPath))))
                throw new IOException("远端文件内容不完整");
            files = resolved;
            if (files.Length == 0) throw new IOException("远端文件内容为空");
            return files;
        }
    }

    private IReadOnlyList<ExplorerOleFileEntry> Descriptors => metadata;

    public int QueryGetData(ref FORMATETC format)
    {
        if (format.dwAspect != DVASPECT_CONTENT || (format.cfFormat != descriptorFormat && format.cfFormat != contentsFormat)
            || (format.cfFormat == descriptorFormat && format.lindex != -1)
            || (format.cfFormat == contentsFormat && (format.lindex < 0 ||
                (metadata.Length > 0 && !Descriptors.Any(entry => entry.Lindex == format.lindex && !entry.IsDirectory))))
            || (format.tymed & TYMED_HGLOBAL) == 0)
            return DV_E_FORMATETC;
        return S_OK;
    }

    public void GetData(ref FORMATETC format, out STGMEDIUM medium)
    {
        if (QueryGetData(ref format) != S_OK) throw new COMException("Unsupported Explorer clipboard format", DV_E_FORMATETC);
        var entries = Materialized();
        byte[] bytes = format.cfFormat == descriptorFormat
            ? Descriptor(entries)
            : File.ReadAllBytes(entries.Single(entry => entry.Lindex == format.lindex).LocalPath!);
        IntPtr memory = Marshal.AllocHGlobal(Math.Max(1, bytes.Length));
        Marshal.Copy(bytes, 0, memory, bytes.Length);
        medium = new STGMEDIUM { tymed = TYMED_HGLOBAL, unionmember = memory, pUnkForRelease = null };
    }

    private static byte[] Descriptor(IReadOnlyList<ExplorerOleFileEntry> entries)
    {
        // FILEDESCRIPTORW is 592 bytes. Include directories and every subtree
        // entry; FileContents later uses the same descriptor lindex.
        const int size = 592;
        var result = new byte[checked(4 + size * entries.Count)];
        BitConverter.GetBytes(entries.Count).CopyTo(result, 0);
        foreach (var entry in entries)
        {
            var descriptor = 4 + size * entry.Lindex;
            BitConverter.GetBytes(FD_ATTRIBUTES | FD_FILESIZE).CopyTo(result, descriptor);
            BitConverter.GetBytes(entry.IsDirectory ? FILE_ATTRIBUTE_DIRECTORY : FILE_ATTRIBUTE_NORMAL)
                .CopyTo(result, descriptor + 36);
            var length = entry.IsDirectory ? 0L : Math.Max(0L, entry.Size);
            BitConverter.GetBytes((uint)(length >> 32)).CopyTo(result, descriptor + 64);
            BitConverter.GetBytes((uint)length).CopyTo(result, descriptor + 68);
            var name = entry.RelativePath.Replace('/', '\\');
            Encoding.Unicode.GetBytes(name[..Math.Min(name.Length, 259)] + "\0")
                .CopyTo(result, descriptor + 76);
        }
        return result;
    }

    public void GetDataHere(ref FORMATETC format, ref STGMEDIUM medium) => throw new COMException("GetDataHere is not supported", DV_E_FORMATETC);
    public int GetCanonicalFormatEtc(ref FORMATETC formatIn, out FORMATETC formatOut) { formatOut = formatIn; return -1; }
    public void SetData(ref FORMATETC formatIn, ref STGMEDIUM medium, bool release) => throw new COMException("SetData is not supported", DV_E_FORMATETC);
    public IEnumFORMATETC EnumFormatEtc(DATADIR direction)
    {
        if (direction != DATADIR.DATADIR_GET) throw new COMException("Only read enumeration is supported", DV_E_FORMATETC);
        var formats = new List<FORMATETC>
        {
            new() { cfFormat = descriptorFormat, dwAspect = DVASPECT_CONTENT, tymed = TYMED_HGLOBAL, lindex = -1 },
        };
        IReadOnlyList<ExplorerOleFileEntry> contentEntries = metadata.Length == 0
            ? [new ExplorerOleFileEntry("file", false, 0, 0)]
            : metadata.Where(entry => !entry.IsDirectory).ToArray();
        foreach (var entry in contentEntries)
            formats.Add(new FORMATETC { cfFormat = contentsFormat, dwAspect = DVASPECT_CONTENT, tymed = TYMED_HGLOBAL, lindex = entry.Lindex });
        return new FormatEnumerator(formats.ToArray());
    }
    public int DAdvise(ref FORMATETC format, ADVF flags, IAdviseSink sink, out int connection) { connection = 0; return unchecked((int)0x80040003); }
    public void DUnadvise(int connection) { }
    public int EnumDAdvise(out IEnumSTATDATA enumAdvise) { enumAdvise = null!; return unchecked((int)0x80040003); }

    private static short Register(string name) => unchecked((short)RegisterClipboardFormat(name));
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern uint RegisterClipboardFormat(string name);

    [ComVisible(true)]
    private sealed class FormatEnumerator : IEnumFORMATETC
    {
        private readonly FORMATETC[] formats;
        private int index;
        public FormatEnumerator(FORMATETC[] formats) => this.formats = formats;
        public int Next(int count, FORMATETC[] output, int[] fetched)
        {
            var copied = 0;
            while (copied < count && index < formats.Length) output[copied++] = formats[index++];
            if (fetched.Length > 0) fetched[0] = copied;
            return copied == count ? 0 : 1;
        }
        public int Skip(int count) { index = Math.Min(formats.Length, index + count); return index < formats.Length ? 0 : 1; }
        public void Reset() => index = 0;
        public void Clone(out IEnumFORMATETC clone) { clone = new FormatEnumerator(formats) { index = index }; }
    }
}
