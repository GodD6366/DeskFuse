using System.Diagnostics;
using System.Security.Principal;
using System.Windows;
using Forms = System.Windows.Forms;

namespace UniDesk.Windows;

public partial class App : System.Windows.Application
{
    private Mutex? singleton;
    private EventWaitHandle? activate;
    private CancellationTokenSource? activationStop;
    private Forms.NotifyIcon? tray;
    private System.Drawing.Icon? trayIcon;
    private ServiceClient? service;
    private DesktopClipboardController? clipboard;
    internal bool Exiting { get; private set; }

    protected override void OnStartup(StartupEventArgs e)
    {
        base.OnStartup(e);
        string user = WindowsIdentity.GetCurrent().User?.Value ?? Environment.UserName;
        string instance = $"Local\\UniDesk.Windows.{user}.{Process.GetCurrentProcess().SessionId}";
        singleton = new Mutex(true, instance, out bool first);
        activate = new EventWaitHandle(false, EventResetMode.AutoReset, instance + ".Activate");
        if (!first) { activate.Set(); Shutdown(); return; }
        AppIcon.InstallWindowDefaults();
        service = new ServiceClient(Dispatcher);
        clipboard = new DesktopClipboardController(service);
        var window = new MainWindow(service, clipboard) { Icon = AppIcon.WindowIcon };
        MainWindow = window;
        window.SourceInitialized += (_, _) => clipboard.Start(window);
        window.Show();
        var menu = new Forms.ContextMenuStrip();
        menu.Items.Add("打开 uniDesk", null, (_, _) => Dispatcher.Invoke(ShowMain));
        var recent = new Forms.ToolStripMenuItem("最近剪贴板");
        menu.Items.Add(recent);
        menu.Opening += (_, _) => Dispatcher.Invoke(() =>
        {
            recent.DropDownItems.Clear();
            var entries = clipboard.Entries.Take(20).ToArray();
            if (entries.Length == 0) recent.DropDownItems.Add(new Forms.ToolStripMenuItem("还没有剪贴板记录") { Enabled = false });
            foreach (var entry in entries)
            {
                string title = entry.Title.Replace('\r', ' ').Replace('\n', ' ');
                if (title.Length > 42) title = title[..42] + "…";
                var item = new Forms.ToolStripMenuItem($"{title}  ·  {entry.SourceName}");
                item.Click += (_, _) => Dispatcher.Invoke(() =>
                {
                    try { clipboard.Restore(entry); }
                    catch (Exception) { tray?.ShowBalloonTip(4000, "uniDesk", "无法恢复剪贴板。请检查源文件、当前会话或剪贴板占用。", Forms.ToolTipIcon.Warning); }
                });
                recent.DropDownItems.Add(item);
            }
        });
        menu.Items.Add("暂存区", null, (_, _) => Dispatcher.Invoke(() => clipboard.ShowShelf()));
        menu.Items.Add("设置", null, (_, _) => Dispatcher.Invoke(window.ShowSettings));
        menu.Items.Add(new Forms.ToolStripSeparator());
        menu.Items.Add("退出 uniDesk", null, (_, _) => Dispatcher.Invoke(ExitApp));
        trayIcon = AppIcon.CreateTrayIcon();
        tray = new Forms.NotifyIcon { Text = "uniDesk", Icon = trayIcon, Visible = true, ContextMenuStrip = menu };
        tray.DoubleClick += (_, _) => Dispatcher.Invoke(ShowMain);
        activationStop = new CancellationTokenSource();
        var token = activationStop.Token;
        _ = Task.Run(() => { while (WaitHandle.WaitAny([activate, token.WaitHandle]) == 0) Dispatcher.BeginInvoke(ShowMain); });
        service.Start();
        if (e.Args.Contains("--minimized", StringComparer.OrdinalIgnoreCase)) window.Hide();
    }
    private void ShowMain()
    {
        if (MainWindow is null) return;
        MainWindow.Show();
        if (MainWindow.WindowState == WindowState.Minimized) MainWindow.WindowState = WindowState.Normal;
        MainWindow.Activate();
    }
    internal void ExitApp()
    {
        Exiting = true;
        Shutdown();
    }
    protected override void OnExit(ExitEventArgs e)
    {
        activationStop?.Cancel();
        clipboard?.Dispose();
        service?.Dispose();
        tray?.Dispose();
        trayIcon?.Dispose();
        activate?.Dispose();
        singleton?.Dispose();
        base.OnExit(e);
    }
}
