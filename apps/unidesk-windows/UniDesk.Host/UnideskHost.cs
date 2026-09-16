namespace UniDesk.Host;

public sealed class UnideskHost : IDisposable
{
    public WindowsClipboardCapture ClipboardCapture { get; } = new();
    public WindowsClipboardApply ClipboardApply { get; } = new();
    public WindowsFileAuthorization FileAuthorization { get; } = new();

    public Task StartAsync(CancellationToken cancellationToken = default)
    {
        ClipboardCapture.StartMonitoring();
        return Task.CompletedTask;
    }

    public void Dispose()
    {
        ClipboardCapture.StopMonitoring();
    }
}
