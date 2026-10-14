using System.Runtime.InteropServices;
using System.Text;

namespace DeskFuse.Windows;

/// <summary>
/// Native dialogs. MessageBoxW keeps the ported synchronous call sites intact
/// (WinUI ContentDialog would force every caller to become async), and
/// IFileOpenDialog replaces System.Windows.Forms.FolderBrowserDialog.
/// </summary>
internal static class Dialogs
{
    private const uint MbOk = 0x00000000;
    private const uint MbYesNo = 0x00000004;
    private const uint MbIconWarning = 0x00000030;
    private const uint MbIconQuestion = 0x00000020;
    private const int IdYes = 6;
    private const uint FosPickFolders = 0x00000020;
    private const uint FosForceFileSystem = 0x00000040;
    private const uint FosPathMustExist = 0x00000800;

    public static void Warning(IntPtr owner, string message, string caption = "DeskFuse") =>
        MessageBoxW(owner, message, caption, MbOk | MbIconWarning);

    public static bool Confirm(IntPtr owner, string message, string caption = "DeskFuse") =>
        MessageBoxW(owner, message, caption, MbYesNo | MbIconQuestion) == IdYes;

    public static string? PickFolder(IntPtr owner, string title, string initialDirectory)
    {
        var dialog = (IFileOpenDialog)new FileOpenDialog();
        try
        {
            dialog.GetOptions(out uint options);
            dialog.SetOptions(options | FosPickFolders | FosForceFileSystem | FosPathMustExist);
            dialog.SetTitle(title);
            if (Directory.Exists(initialDirectory))
            {
                if (SHCreateItemFromParsingName(initialDirectory, IntPtr.Zero, typeof(IShellItem).GUID, out var item) == 0)
                {
                    dialog.SetFolder(item);
                    Marshal.ReleaseComObject(item);
                }
            }
            int result = dialog.Show(owner);
            if (result != 0) return null; // cancelled (ERROR_CANCELLED = 0x800704C7)
            dialog.GetResult(out var selection);
            try
            {
                if (selection.GetDisplayName(0x80058000 /* SIGDN_FILESYSPATH */, out var path) != 0) return null;
                try { return Marshal.PtrToStringUni(path); }
                finally { Marshal.FreeCoTaskMem(path); }
            }
            finally { Marshal.ReleaseComObject(selection); }
        }
        finally { Marshal.ReleaseComObject(dialog); }
    }

    [DllImport("user32.dll", EntryPoint = "MessageBoxW", CharSet = CharSet.Unicode)]
    private static extern int MessageBoxW(IntPtr owner, string text, string caption, uint type);

    [DllImport("shell32.dll", CharSet = CharSet.Unicode, PreserveSig = false)]
    private static extern int SHCreateItemFromParsingName(string path, IntPtr bindContext, [MarshalAs(UnmanagedType.LPStruct)] Guid interfaceId, out IShellItem item);

    [ComImport, Guid("42f85136-db7e-439c-85f1-e4075d135fc8"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IFileOpenDialog
    {
        [PreserveSig] int Show(IntPtr parent);
        void SetFileTypes(uint count, IntPtr filter);
        void SetFileTypeIndex(uint index);
        void GetFileTypeIndex(out uint index);
        void Advise(IntPtr events, out uint cookie);
        void Unadvise(uint cookie);
        void SetOptions(uint options);
        void GetOptions(out uint options);
        void SetDefaultFolder(IShellItem folder);
        void SetFolder(IShellItem folder);
        void GetFolder(out IShellItem folder);
        void GetCurrentSelection(out IShellItem selection);
        void SetFileName([MarshalAs(UnmanagedType.LPWStr)] string name);
        void GetFileName(out IntPtr name);
        void SetTitle([MarshalAs(UnmanagedType.LPWStr)] string title);
        void SetOkButtonLabel([MarshalAs(UnmanagedType.LPWStr)] string text);
        void SetFileNameLabel([MarshalAs(UnmanagedType.LPWStr)] string label);
        void GetResult(out IShellItem item);
    }

    [ComImport, Guid("43826d1e-e718-42ee-bc55-a1e261c37bfe"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IShellItem
    {
        [PreserveSig] int BindToHandler(IntPtr bindContext, ref Guid handler, ref Guid interfaceId, out IntPtr result);
        [PreserveSig] int GetParent(out IShellItem parent);
        [PreserveSig] int GetDisplayName(uint format, out IntPtr name);
        [PreserveSig] int GetAttributes(uint mask, out uint attributes);
        [PreserveSig] int Compare(IShellItem other, uint hint, out int order);
    }

    [ComImport, Guid("DC1C5A9C-E88A-4dde-A5A1-60F82A20AEF7")]
    private class FileOpenDialog { }
}
