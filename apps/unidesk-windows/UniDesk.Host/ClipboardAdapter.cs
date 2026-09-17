namespace UniDesk.Host;

public sealed class WindowsClipboardCapture
{
    public void StartMonitoring() { }
    public void StopMonitoring() { }
}

public sealed class WindowsClipboardApply
{
    public bool TryApplyText(string text) => !string.IsNullOrEmpty(text);
}
