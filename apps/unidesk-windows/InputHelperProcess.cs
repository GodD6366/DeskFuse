using System.Diagnostics;
using System.IO;
using System.Windows.Threading;

namespace UniDesk.Windows;

/// <summary>Supervises the fixed ordinary-desktop Rust input helper.</summary>
public sealed class InputHelperProcess : IDisposable
{
    private readonly Dispatcher dispatcher;
    private readonly DispatcherTimer restart;
    private Process? process;
    private bool disposed;

    public InputHelperProcess(Dispatcher dispatcher)
    {
        this.dispatcher = dispatcher;
        restart = new DispatcherTimer(TimeSpan.FromSeconds(2), DispatcherPriority.Background, (_, _) =>
        {
            restart.Stop();
            Start();
        }, dispatcher);
        Start();
    }

    private void Start()
    {
        if (disposed || process is { HasExited: false }) return;
        string executable = Path.Combine(AppContext.BaseDirectory, "unidesk-input-helper.exe");
        if (!File.Exists(executable)) return;
        try
        {
            var start = new ProcessStartInfo(executable)
            {
                UseShellExecute = false,
                CreateNoWindow = true,
                WorkingDirectory = AppContext.BaseDirectory
            };
            start.ArgumentList.Add("--parent");
            start.ArgumentList.Add(Environment.ProcessId.ToString(System.Globalization.CultureInfo.InvariantCulture));
            var next = Process.Start(start);
            if (next is null) return;
            next.EnableRaisingEvents = true;
            next.Exited += HelperExited;
            process?.Dispose();
            process = next;
        }
        catch (Exception) { restart.Start(); }
    }

    private void HelperExited(object? sender, EventArgs args) => dispatcher.BeginInvoke(() =>
    {
        if (sender is Process exited) exited.Exited -= HelperExited;
        if (!disposed) restart.Start();
    });

    public void Dispose()
    {
        disposed = true;
        restart.Stop();
        if (process is { } current)
        {
            current.Exited -= HelperExited;
            try { if (!current.HasExited) current.Kill(entireProcessTree: true); }
            catch (Exception) { }
            current.Dispose();
        }
        process = null;
    }
}
