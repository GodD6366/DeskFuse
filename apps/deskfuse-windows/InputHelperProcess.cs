using System.Diagnostics;
using System.IO;
using System.Windows.Threading;

namespace DeskFuse.Windows;

/// <summary>Supervises the fixed signed UIAccess Rust input helper.</summary>
public sealed class InputHelperProcess : IDisposable
{
    private readonly Dispatcher dispatcher;
    private readonly DispatcherTimer restart;
    private Process? process;
    private bool disposed;

    public int? ProcessId => process is { HasExited: false } current ? current.Id : null;

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

    /// Stop and drain the fixed helper before a machine update takes its
    /// admission lock. This runs on the WPF dispatcher so restart cannot race
    /// the update helper's process identity snapshot.
    public bool StopForMachineUpdate()
    {
        dispatcher.VerifyAccess();
        restart.Stop();
        if (process is not { } current) return true;
        try
        {
            if (!current.HasExited) current.Kill(entireProcessTree: true);
            return current.WaitForExit(5000);
        }
        catch (Exception) { return false; }
        finally
        {
            current.Exited -= HelperExited;
            current.Dispose();
            process = null;
        }
    }

    public void ResumeAfterMachineUpdate()
    {
        dispatcher.VerifyAccess();
        if (!disposed) Start();
    }

    private void Start()
    {
        if (disposed || process is { HasExited: false }) return;
        string executable = Path.Combine(AppContext.BaseDirectory, "deskfuse-input-helper.exe");
        if (!File.Exists(executable)) return;
        try
        {
            var start = new ProcessStartInfo(executable)
            {
                // A medium-integrity process cannot create a uiAccess child with
                // CreateProcess (Win32 740). Launch through the shell, which
                // performs the checked UIAccess launch; Hidden keeps the
                // console-subsystem helper invisible.
                UseShellExecute = true,
                WindowStyle = ProcessWindowStyle.Hidden,
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
