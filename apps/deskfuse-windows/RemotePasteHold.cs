using System.Runtime.InteropServices;
using Microsoft.UI.Dispatching;

namespace DeskFuse.Windows;

/// <summary>
/// Holds Ctrl+V while remote clipboard content is still on its way and replays
/// it once that content is on the clipboard, like the macOS paste hold. The
/// low-level keyboard hook exists only while something is in flight; the
/// native callback only flips flags, all window work runs on the UI thread.
/// </summary>
internal sealed class RemotePasteHold : IDisposable
{
    private const int WhKeyboardLl = 13;
    private const uint WmKeyDown = 0x0100, WmKeyUp = 0x0101, WmSysKeyDown = 0x0104, WmSysKeyUp = 0x0105;
    private const uint WmQuit = 0x0012, WmSync = 0x8001;
    private const int VkControl = 0x11, VkMenu = 0x12, VkLeftWin = 0x5B, VkRightWin = 0x5C, VkV = 0x56, VkEscape = 0x1B;
    private const uint LlkhfInjected = 0x10;

    private readonly DispatcherQueue dispatcher;
    private readonly Action<IntPtr> held;
    private readonly Action cancelled;
    private readonly HookProc callback;
    private readonly Thread thread;
    private int nativeThreadId;
    private IntPtr hook;
    private int armed;
    private int pending;
    // Hook-thread only: the key-up halves of keys we swallowed.
    private bool swallowV;
    private bool swallowEscape;
    private bool disposed;

    public RemotePasteHold(DispatcherQueue dispatcher, Action<IntPtr> held, Action cancelled)
    {
        this.dispatcher = dispatcher;
        this.held = held;
        this.cancelled = cancelled;
        callback = HookCallback;
        thread = new Thread(Run) { IsBackground = true, Name = "DeskFuse remote paste hold" };
        thread.Start();
    }

    /// <summary>A Ctrl+V is being held for remote content.</summary>
    public bool Pending => Volatile.Read(ref pending) != 0;

    /// <summary>Forget the held paste after it was replayed or cancelled.</summary>
    public void Reset() => Interlocked.Exchange(ref pending, 0);

    /// <summary>Hold pastes while remote content is in flight.</summary>
    public void SetArmed(bool value)
    {
        if (Interlocked.Exchange(ref armed, value ? 1 : 0) == (value ? 1 : 0)) return;
        int id = Volatile.Read(ref nativeThreadId);
        if (id != 0) PostThreadMessage(id, WmSync, IntPtr.Zero, IntPtr.Zero);
    }

    private void Run()
    {
        // Create the thread queue before publishing its ID so arm/quit posts land.
        PeekMessage(out _, IntPtr.Zero, 0, 0, 0);
        Interlocked.Exchange(ref nativeThreadId, GetCurrentThreadId());
        Sync(); // An arm raised before the ID was published.
        try
        {
            while (GetMessage(out var message, IntPtr.Zero, 0, 0) > 0)
            {
                if (message.Hwnd == IntPtr.Zero && message.Message == WmSync) { Sync(); continue; }
                TranslateMessage(ref message);
                DispatchMessage(ref message);
            }
        }
        finally
        {
            if (hook != IntPtr.Zero) UnhookWindowsHookEx(hook);
            hook = IntPtr.Zero;
        }
    }

    private void Sync()
    {
        bool wanted = Volatile.Read(ref armed) != 0;
        if (wanted && hook == IntPtr.Zero)
        {
            // Without the hook the paste simply goes through unheld.
            hook = SetWindowsHookEx(WhKeyboardLl, callback, GetModuleHandle(null), 0);
        }
        else if (!wanted && hook != IntPtr.Zero)
        {
            UnhookWindowsHookEx(hook);
            hook = IntPtr.Zero;
            swallowV = swallowEscape = false;
        }
    }

    private IntPtr HookCallback(int code, IntPtr wParam, IntPtr lParam)
    {
        if (code >= 0)
        {
            var data = Marshal.PtrToStructure<KbdLlHookStruct>(lParam);
            uint message = unchecked((uint)wParam.ToInt64());
            bool down = message is WmKeyDown or WmSysKeyDown;
            bool up = message is WmKeyUp or WmSysKeyUp;
            // Our own replay is injected; it must reach the target untouched.
            if ((data.Flags & LlkhfInjected) == 0)
            {
                if (data.VkCode == VkV)
                {
                    if (down && Volatile.Read(ref armed) != 0 && IsDown(VkControl)
                        && !IsDown(VkMenu) && !IsDown(VkLeftWin) && !IsDown(VkRightWin))
                    {
                        swallowV = true;
                        if (Interlocked.Exchange(ref pending, 1) == 0)
                        {
                            IntPtr target = GetForegroundWindow();
                            dispatcher.TryEnqueue(() => { if (!disposed) held(target); });
                        }
                        return 1;
                    }
                    if (up && swallowV) { swallowV = false; return 1; }
                }
                else if (data.VkCode == VkEscape)
                {
                    if (down && Pending)
                    {
                        swallowEscape = true;
                        dispatcher.TryEnqueue(() => { if (!disposed) cancelled(); });
                        return 1;
                    }
                    if (up && swallowEscape) { swallowEscape = false; return 1; }
                }
            }
        }
        return CallNextHookEx(hook, code, wParam, lParam);
    }

    /// <summary>Screen point (physical pixels) under the caret of
    /// <paramref name="target"/>, or the pointer when it shows no caret.</summary>
    public static (int X, int Y) Anchor(IntPtr target)
    {
        var info = new GuiThreadInfo { Size = Marshal.SizeOf<GuiThreadInfo>() };
        uint thread = GetWindowThreadProcessId(target, out _);
        if (thread != 0 && GetGUIThreadInfo(thread, ref info) && info.Caret != IntPtr.Zero)
        {
            var point = new NativePoint { X = info.CaretRect.Left, Y = info.CaretRect.Bottom };
            if (ClientToScreen(info.Caret, ref point)) return (point.X, point.Y);
        }
        GetCursorPos(out var cursor);
        return (cursor.X, cursor.Y);
    }

    /// <summary>Send the held Ctrl+V to <paramref name="target"/>; false when
    /// the user has moved to another window meanwhile.</summary>
    public static bool Replay(IntPtr target)
    {
        if (target == IntPtr.Zero || GetAncestor(GetForegroundWindow(), 2) != GetAncestor(target, 2)) return false;
        // A Ctrl still held by the user already modifies the V; sending our own
        // Ctrl up would release it under their finger.
        var inputs = IsDown(VkControl)
            ? new[] { Key(VkV, false), Key(VkV, true) }
            : new[] { Key(VkControl, false), Key(VkV, false), Key(VkV, true), Key(VkControl, true) };
        if (SendInput((uint)inputs.Length, inputs, Marshal.SizeOf<Input>()) == inputs.Length) return true;
        var release = new[] { Key(VkV, true) };
        SendInput((uint)release.Length, release, Marshal.SizeOf<Input>());
        return false;
    }

    private static bool IsDown(int key) => (GetAsyncKeyState(key) & 0x8000) != 0;

    private static Input Key(int virtualKey, bool keyUp) => new()
    {
        Type = 1,
        Data = new InputUnion { Keyboard = new KeyboardInput { VirtualKey = (ushort)virtualKey, Flags = keyUp ? 0x0002u : 0 } },
    };

    public void Dispose()
    {
        if (disposed) return;
        disposed = true;
        int id = Volatile.Read(ref nativeThreadId);
        if (id != 0) PostThreadMessage(id, WmQuit, IntPtr.Zero, IntPtr.Zero);
        if (thread.IsAlive && !thread.Join(TimeSpan.FromSeconds(2)) && hook != IntPtr.Zero) UnhookWindowsHookEx(hook);
    }

    private delegate IntPtr HookProc(int code, IntPtr wParam, IntPtr lParam);

    [StructLayout(LayoutKind.Sequential)]
    private struct KbdLlHookStruct { public uint VkCode, ScanCode, Flags, Time; public UIntPtr ExtraInfo; }
    [StructLayout(LayoutKind.Sequential)]
    private struct NativePoint { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)]
    private struct NativeRect { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)]
    private struct GuiThreadInfo
    {
        public int Size;
        public uint Flags;
        public IntPtr Active, Focus, Capture, MenuOwner, MoveSize, Caret;
        public NativeRect CaretRect;
    }
    [StructLayout(LayoutKind.Sequential)]
    private struct NativeMessage
    {
        public IntPtr Hwnd;
        public uint Message;
        public UIntPtr WParam;
        public IntPtr LParam;
        public uint Time;
        public NativePoint Point;
        public uint Private;
    }
    [StructLayout(LayoutKind.Sequential)]
    private struct Input { public uint Type; public InputUnion Data; }
    [StructLayout(LayoutKind.Explicit)]
    private struct InputUnion
    {
        [FieldOffset(0)] public MouseInput Mouse;
        [FieldOffset(0)] public KeyboardInput Keyboard;
    }
    [StructLayout(LayoutKind.Sequential)]
    private struct MouseInput { public int X, Y; public uint MouseData, Flags, Time; public UIntPtr ExtraInfo; }
    [StructLayout(LayoutKind.Sequential)]
    private struct KeyboardInput { public ushort VirtualKey, ScanCode; public uint Flags, Time; public UIntPtr ExtraInfo; }

    [DllImport("user32.dll", SetLastError = true)] private static extern IntPtr SetWindowsHookEx(int idHook, HookProc callback, IntPtr module, uint threadId);
    [DllImport("user32.dll", SetLastError = true)] private static extern bool UnhookWindowsHookEx(IntPtr hook);
    [DllImport("user32.dll")] private static extern IntPtr CallNextHookEx(IntPtr hook, int code, IntPtr wParam, IntPtr lParam);
    [DllImport("user32.dll")] private static extern short GetAsyncKeyState(int key);
    [DllImport("user32.dll")] private static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] private static extern IntPtr GetAncestor(IntPtr window, uint flags);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
    [DllImport("user32.dll")] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool GetGUIThreadInfo(uint thread, ref GuiThreadInfo info);
    [DllImport("user32.dll")] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool ClientToScreen(IntPtr window, ref NativePoint point);
    [DllImport("user32.dll")] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool GetCursorPos(out NativePoint point);
    [DllImport("user32.dll", SetLastError = true)] private static extern uint SendInput(uint count, Input[] inputs, int size);
    [DllImport("user32.dll", SetLastError = true)] private static extern int GetMessage(out NativeMessage message, IntPtr hwnd, uint min, uint max);
    [DllImport("user32.dll")] private static extern bool PeekMessage(out NativeMessage message, IntPtr hwnd, uint min, uint max, uint remove);
    [DllImport("user32.dll")] private static extern bool TranslateMessage(ref NativeMessage message);
    [DllImport("user32.dll")] private static extern IntPtr DispatchMessage(ref NativeMessage message);
    [DllImport("user32.dll", SetLastError = true)] private static extern bool PostThreadMessage(int threadId, uint message, IntPtr wParam, IntPtr lParam);
    [DllImport("kernel32.dll")] private static extern int GetCurrentThreadId();
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] private static extern IntPtr GetModuleHandle(string? name);
}
