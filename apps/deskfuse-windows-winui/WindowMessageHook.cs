using System.ComponentModel;
using System.Runtime.InteropServices;

namespace DeskFuse.Windows;

/// <summary>
/// Minimal Win32 window-procedure chaining for WinUI 3, which has no HwndSource
/// hook. One thunk per window forwards to every attached handler and then falls
/// through to the previous procedure. Used for WM_CLIPBOARDUPDATE and (later)
/// WM_HOTKEY.
/// </summary>
internal static class WindowMessageHook
{
    private const int GwlWndProc = -4;
    private delegate IntPtr WindowProcedureDelegate(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam);

    private sealed class Hook
    {
        // Kept alive for the lifetime of the subclass: the native window stores a
        // raw function pointer to this thunk.
        public WindowProcedureDelegate Procedure = null!;
        public IntPtr Previous;
        public readonly List<Func<uint, IntPtr, IntPtr, bool>> Handlers = [];
    }

    private static readonly Dictionary<IntPtr, Hook> Hooks = [];

    public static void Attach(IntPtr hwnd, Func<uint, IntPtr, IntPtr, bool> handler)
    {
        if (hwnd == IntPtr.Zero) throw new ArgumentException("window handle is required", nameof(hwnd));
        if (Hooks.TryGetValue(hwnd, out var attached))
        {
            attached.Handlers.Add(handler);
            return;
        }
        var hook = new Hook();
        hook.Handlers.Add(handler);
        hook.Procedure = (window, message, wParam, lParam) =>
        {
            if (!Hooks.TryGetValue(window, out var current))
                return DefWindowProcW(window, message, wParam, lParam);
            foreach (var candidate in current.Handlers.ToArray())
            {
                bool handled;
                try { handled = candidate(message, wParam, lParam); }
                catch (Exception) { handled = false; } // a handler fault must not kill the window loop
                if (handled) return IntPtr.Zero;
            }
            return CallWindowProcW(current.Previous, window, message, wParam, lParam);
        };
        Hooks[hwnd] = hook;
        Marshal.GetLastWin32Error(); // clear stale error state before the call
        hook.Previous = SetWindowLongPtrW(hwnd, GwlWndProc, Marshal.GetFunctionPointerForDelegate(hook.Procedure));
        if (hook.Previous == IntPtr.Zero)
        {
            // Zero is also a valid previous value, so only fail on a real error.
            int error = Marshal.GetLastWin32Error();
            if (error != 0)
            {
                Hooks.Remove(hwnd);
                throw new Win32Exception(error, "cannot subclass the window procedure");
            }
        }
    }

    public static void Detach(IntPtr hwnd)
    {
        if (Hooks.Remove(hwnd, out var hook) && hook.Previous != IntPtr.Zero)
            SetWindowLongPtrW(hwnd, GwlWndProc, hook.Previous);
    }

    [DllImport("user32.dll", EntryPoint = "SetWindowLongPtrW", SetLastError = true)]
    private static extern IntPtr SetWindowLongPtrW(IntPtr hwnd, int index, IntPtr value);

    [DllImport("user32.dll", EntryPoint = "CallWindowProcW")]
    private static extern IntPtr CallWindowProcW(IntPtr previous, IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam);

    [DllImport("user32.dll", EntryPoint = "DefWindowProcW")]
    private static extern IntPtr DefWindowProcW(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam);
}
