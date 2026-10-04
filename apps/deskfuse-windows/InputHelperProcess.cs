using System.Diagnostics;
using System.IO;
using Microsoft.UI.Dispatching;

namespace DeskFuse.Windows;

/// <summary>Supervises the fixed signed UIAccess Rust input helper.</summary>
public sealed class InputHelperProcess : IDisposable
{
    private readonly DispatcherQueue dispatcher;
    private readonly DispatcherQueueTimer restart;
    private Process? process;
    private bool disposed;

    public int? ProcessId => process is { HasExited: false } current ? current.Id : null;

    public InputHelperProcess(DispatcherQueue dispatcher)
    {
        this.dispatcher = dispatcher;
        restart = dispatcher.CreateTimer();
        restart.Interval = TimeSpan.FromSeconds(2);
        restart.IsRepeating = false;
        restart.Tick += (_, _) => Start();
        Start();
    }

    /// Stop and drain the fixed helper before a machine update takes its
    /// admission lock. This runs on the UI thread so restart cannot race the
    /// update helper's process identity snapshot.
    public bool StopForMachineUpdate()
    {
        VerifyAccess();
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
        VerifyAccess();
        if (!disposed) Start();
    }

    private void VerifyAccess()
    {
        if (!dispatcher.HasThreadAccess) throw new InvalidOperationException("input helper must be controlled from the UI thread");
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

    private void HelperExited(object? sender, EventArgs args) => dispatcher.TryEnqueue(() =>
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
