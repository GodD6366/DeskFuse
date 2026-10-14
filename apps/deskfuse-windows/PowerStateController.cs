using System.Runtime.InteropServices;

namespace DeskFuse.Windows;

/// <summary>Windows equivalent of macOS CaffeineController.</summary>
public sealed class PowerStateController : IDisposable
{
    private const uint EsContinuous = 0x80000000;
    private const uint EsSystemRequired = 0x00000001;
    private const uint EsDisplayRequired = 0x00000002;
    private bool enabled;
    private bool disposed;

    public bool IsEnabled => enabled;
    public bool StartupFailed { get; private set; }

    public PowerStateController(bool initiallyEnabled)
    {
        if (initiallyEnabled && !SetEnabled(true)) StartupFailed = true;
    }

    public bool SetEnabled(bool value)
    {
        ObjectDisposedException.ThrowIf(disposed, this);
        if (enabled == value) return true;
        uint flags = EsContinuous | (value ? EsSystemRequired | EsDisplayRequired : 0);
        if (SetThreadExecutionState(flags) == 0) return false;
        enabled = value;
        return true;
    }

    public void Dispose()
    {
        if (disposed) return;
        disposed = true;
        if (enabled) SetThreadExecutionState(EsContinuous);
        enabled = false;
    }

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern uint SetThreadExecutionState(uint executionState);
}
