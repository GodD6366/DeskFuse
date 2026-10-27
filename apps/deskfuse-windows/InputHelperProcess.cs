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
    private readonly Action<string?>? statusChanged;
    private string? lastStatus;

    public int? ProcessId => process is { HasExited: false } current ? current.Id : null;

    public InputHelperProcess(DispatcherQueue dispatcher, Action<string?>? statusChanged = null)
    {
        this.dispatcher = dispatcher;
        this.statusChanged = statusChanged;
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

    private void Report(string? message)
    {
        if (lastStatus == message) return;
        lastStatus = message;
        statusChanged?.Invoke(message);
    }

    private void Start()
    {
        if (disposed || process is { HasExited: false }) return;
        string executable = Path.Combine(AppContext.BaseDirectory, "deskfuse-input-helper.exe");
        if (!File.Exists(executable))
        {
            Report("当前目录缺少输入组件。UI 开发版不包含输入组件；鼠标共享需要安装完整的受信任版本。");
            return;
        }
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
            Report(null);
        }
        catch (System.ComponentModel.Win32Exception ex)
        {
            Report(ex.NativeErrorCode == 8235
                ? "Windows 拒绝启动未受信任的 UIAccess 输入组件（8235）。请安装有效签名的输入组件；设备在线不代表本机输入可用。"
                : $"本机输入组件启动失败（Windows {ex.NativeErrorCode}）：{ex.Message}");
            restart.Start();
        }
        catch (Exception ex) { Report($"本机输入组件启动失败：{ex.Message}"); restart.Start(); }
    }

    private void HelperExited(object? sender, EventArgs args) => dispatcher.TryEnqueue(() =>
    {
        if (sender is Process exited) exited.Exited -= HelperExited;
        if (!disposed) { Report("本机输入组件已退出，正在重新连接。请检查安装和授权状态。"); restart.Start(); }
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
