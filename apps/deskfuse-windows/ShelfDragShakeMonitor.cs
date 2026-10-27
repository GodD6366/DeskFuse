using System.ComponentModel;
using System.Runtime.InteropServices;
using Microsoft.UI.Dispatching;

namespace DeskFuse.Windows;

/// Global low-level mouse monitoring mirrors the macOS drag-shake thresholds.
/// Native callbacks only accumulate coordinates; window work runs on the UI thread.
internal sealed class ShelfDragShakeMonitor : IDisposable
{
    private const int WhMouseLl = 14;
    private const uint WmMouseMove = 0x0200;
    private const uint WmLButtonDown = 0x0201;
    private const uint WmLButtonUp = 0x0202;
    private const uint WmQuit = 0x0012;
    private const int VkLButton = 0x01;

    private readonly DispatcherQueue dispatcher;
    private readonly Action show;
    private readonly Action<string> error;
    private readonly HookProc callback;
    private readonly Thread thread;
    private int nativeThreadId;
    private IntPtr hook;
    private int lastX;
    private int lastDirection;
    private int reversals;
    private long startedAt;
    private long lastShown;
    private bool disposed;

    public ShelfDragShakeMonitor(DispatcherQueue dispatcher, Action show, Action<string> error)
    {
        this.dispatcher = dispatcher;
        this.show = show;
        this.error = error;
        callback = HookCallback;
        thread = new Thread(Run) { IsBackground = true, Name = "DeskFuse shelf drag monitor" };
        thread.Start();
    }

    private void Run()
    {
        // Create the thread queue before publishing its ID so an immediate Dispose
        // can always post WM_QUIT instead of leaving a native message loop behind.
        PeekMessage(out _, IntPtr.Zero, 0, 0, 0);
        nativeThreadId = GetCurrentThreadId();
        hook = SetWindowsHookEx(WhMouseLl, callback, GetModuleHandle(null), 0);
        if (hook == IntPtr.Zero)
        {
            string message = new Win32Exception(Marshal.GetLastWin32Error()).Message;
            dispatcher.TryEnqueue(() => error($"拖动晃动唤起暂存区不可用：{message}"));
            return;
        }
        try
        {
            while (GetMessage(out var message, IntPtr.Zero, 0, 0) > 0)
            {
                TranslateMessage(ref message);
                DispatchMessage(ref message);
            }
        }
        finally
        {
            UnhookWindowsHookEx(hook);
            hook = IntPtr.Zero;
        }
    }

    private IntPtr HookCallback(int code, IntPtr wParam, IntPtr lParam)
    {
        if (code >= 0)
        {
            uint message = unchecked((uint)wParam.ToInt64());
            if (message == WmLButtonDown || message == WmLButtonUp) Reset();
            else if (message == WmMouseMove && (GetAsyncKeyState(VkLButton) & 0x8000) != 0)
            {
                var data = Marshal.PtrToStructure<MsLlHookStruct>(lParam);
                Observe(data.Point.X, data.Point.Y);
            }
        }
        return CallNextHookEx(hook, code, wParam, lParam);
    }

    private void Observe(int x, int y)
    {
        long now = Environment.TickCount64;
        if (now - lastShown <= 2000) return;
        if (now - startedAt > 800)
        {
            startedAt = now;
            reversals = 0;
            lastDirection = 0;
        }
        int movement = x - lastX;
        if (Math.Abs(movement) > 8)
        {
            int direction = movement > 0 ? 1 : -1;
            if (lastDirection != 0 && direction != lastDirection) reversals++;
            lastDirection = direction;
        }
        lastX = x;
        _ = y; // Coordinates are read again on the UI thread when placing the shelf.
        if (reversals < 3) return;
        lastShown = now;
        reversals = 0;
        dispatcher.TryEnqueue(() => { if (!disposed) show(); });
    }

    private void Reset()
    {
        lastX = 0;
        lastDirection = 0;
        reversals = 0;
        startedAt = 0;
    }

    public void Dispose()
    {
        if (disposed) return;
        disposed = true;
        if (nativeThreadId != 0) PostThreadMessage(nativeThreadId, WmQuit, IntPtr.Zero, IntPtr.Zero);
        if (thread.IsAlive && !thread.Join(TimeSpan.FromSeconds(2)) && hook != IntPtr.Zero) UnhookWindowsHookEx(hook);
    }

    private delegate IntPtr HookProc(int code, IntPtr wParam, IntPtr lParam);

    [StructLayout(LayoutKind.Sequential)]
    private struct MsLlHookStruct
    {
        public NativePoint Point;
        public uint MouseData;
        public uint Flags;
        public uint Time;
        public UIntPtr ExtraInfo;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct NativePoint { public int X; public int Y; }

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

    [DllImport("user32.dll", SetLastError = true)] private static extern IntPtr SetWindowsHookEx(int idHook, HookProc callback, IntPtr module, uint threadId);
    [DllImport("user32.dll", SetLastError = true)] private static extern bool UnhookWindowsHookEx(IntPtr hook);
    [DllImport("user32.dll")] private static extern IntPtr CallNextHookEx(IntPtr hook, int code, IntPtr wParam, IntPtr lParam);
    [DllImport("user32.dll")] private static extern short GetAsyncKeyState(int key);
    [DllImport("user32.dll", SetLastError = true)] private static extern int GetMessage(out NativeMessage message, IntPtr hwnd, uint min, uint max);
    [DllImport("user32.dll")] private static extern bool PeekMessage(out NativeMessage message, IntPtr hwnd, uint min, uint max, uint remove);
    [DllImport("user32.dll")] private static extern bool TranslateMessage(ref NativeMessage message);
    [DllImport("user32.dll")] private static extern IntPtr DispatchMessage(ref NativeMessage message);
    [DllImport("user32.dll", SetLastError = true)] private static extern bool PostThreadMessage(int threadId, uint message, IntPtr wParam, IntPtr lParam);
    [DllImport("kernel32.dll")] private static extern int GetCurrentThreadId();
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] private static extern IntPtr GetModuleHandle(string? name);
}
