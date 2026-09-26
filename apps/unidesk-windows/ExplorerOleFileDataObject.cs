using System.Runtime.InteropServices;
using System.Runtime.InteropServices.ComTypes;
using System.Text;
using ComDataObject = System.Runtime.InteropServices.ComTypes.IDataObject;

namespace UniDesk.Windows;

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
    private readonly Func<IReadOnlyList<string>> materialize;
    private readonly object gate = new();
    private string[]? files;
    private readonly short descriptorFormat = Register("FileGroupDescriptorW");
    private readonly short contentsFormat = Register("FileContents");

    public ExplorerOleFileDataObject(Func<IReadOnlyList<string>> materialize) => this.materialize = materialize;

    private string[] Materialized()
    {
        lock (gate)
        {
            files ??= materialize().Where(File.Exists).ToArray();
            if (files.Length == 0) throw new IOException("远端文件内容为空");
            return files;
        }
    }

    public int QueryGetData(ref FORMATETC format)
    {
        if (format.dwAspect != DVASPECT_CONTENT || (format.cfFormat != descriptorFormat && format.cfFormat != contentsFormat)
            || (format.cfFormat == contentsFormat && format.lindex < 0) || (format.tymed & TYMED_HGLOBAL) == 0)
            return DV_E_FORMATETC;
        return S_OK;
    }

    public void GetData(ref FORMATETC format, out STGMEDIUM medium)
    {
        if (QueryGetData(ref format) != S_OK) throw new COMException("Unsupported Explorer clipboard format", DV_E_FORMATETC);
        var paths = Materialized();
        byte[] bytes = format.cfFormat == descriptorFormat ? Descriptor(paths) : File.ReadAllBytes(paths[format.lindex]);
        IntPtr memory = Marshal.AllocHGlobal(Math.Max(1, bytes.Length));
        Marshal.Copy(bytes, 0, memory, bytes.Length);
        medium = new STGMEDIUM { tymed = TYMED_HGLOBAL, unionmember = memory, pUnkForRelease = null };
    }

    private static byte[] Descriptor(IReadOnlyList<string> paths)
    {
        // FILEDESCRIPTORW is 592 bytes. Explorer only needs flags and cFileName;
        // size is filled to let it show useful details before FileContents.
        const int size = 592;
        var result = new byte[checked(4 + size * paths.Count)];
        BitConverter.GetBytes(paths.Count).CopyTo(result, 0);
        for (var i = 0; i < paths.Count; i++)
        {
            var descriptor = 4 + size * i;
            BitConverter.GetBytes(0x00000004 | 0x00000040).CopyTo(result, descriptor); // FD_ATTRIBUTES | FD_FILESIZE
            var length = new FileInfo(paths[i]).Length;
            BitConverter.GetBytes(length).CopyTo(result, descriptor + 72);
            var name = Path.GetFileName(paths[i]);
            Encoding.Unicode.GetBytes(name[..Math.Min(name.Length, 259)] + "\0").CopyTo(result, descriptor + 76);
        }
        return result;
    }

    public void GetDataHere(ref FORMATETC format, ref STGMEDIUM medium) => throw new COMException("GetDataHere is not supported", DV_E_FORMATETC);
    public int GetCanonicalFormatEtc(ref FORMATETC formatIn, out FORMATETC formatOut) { formatOut = formatIn; return -1; }
    public void SetData(ref FORMATETC formatIn, ref STGMEDIUM medium, bool release) => throw new COMException("SetData is not supported", DV_E_FORMATETC);
    public IEnumFORMATETC EnumFormatEtc(DATADIR direction)
    {
        if (direction != DATADIR.DATADIR_GET) throw new COMException("Only read enumeration is supported", DV_E_FORMATETC);
        return new FormatEnumerator(descriptorFormat, contentsFormat);
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
        public FormatEnumerator(short descriptor, short contents) => formats = [
            new FORMATETC { cfFormat = descriptor, dwAspect = DVASPECT_CONTENT, tymed = TYMED_HGLOBAL, lindex = -1 },
            new FORMATETC { cfFormat = contents, dwAspect = DVASPECT_CONTENT, tymed = TYMED_HGLOBAL, lindex = 0 },
        ];
        public int Next(int count, FORMATETC[] output, int[] fetched)
        {
            var copied = 0;
            while (copied < count && index < formats.Length) output[copied++] = formats[index++];
            if (fetched.Length > 0) fetched[0] = copied;
            return copied == count ? 0 : 1;
        }
        public int Skip(int count) { index = Math.Min(formats.Length, index + count); return index < formats.Length ? 0 : 1; }
        public void Reset() => index = 0;
        public void Clone(out IEnumFORMATETC clone) { clone = new FormatEnumerator(formats[0].cfFormat, formats[1].cfFormat) { index = index }; }
    }
}
