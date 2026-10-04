using System.Diagnostics;
using System.Security.Principal;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;

namespace DeskFuse.Windows;

public partial class App : Application
{
    private Mutex? singleton;
    private EventWaitHandle? activate;
    private CancellationTokenSource? activationStop;
    private TrayIcon? tray;
    private ServiceClient? service;
    private DesktopClipboardController? clipboard;
    private InputHelperProcess? inputHelper;
    private MainWindow? window;
    internal bool Exiting { get; private set; }

    public App() => InitializeComponent();

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        base.OnLaunched(args);
        string user = WindowsIdentity.GetCurrent().User?.Value ?? Environment.UserName;
        string instance = $"Local\\DeskFuse.Windows.{user}.{Process.GetCurrentProcess().SessionId}";
        singleton = new Mutex(true, instance, out bool first);
        activate = new EventWaitHandle(false, EventResetMode.AutoReset, instance + ".Activate");
        if (!first)
        {
            activate.Set();
            Exit();
            return;
        }

        var dispatcher = DispatcherQueue.GetForCurrentThread();
        service = new ServiceClient(dispatcher);
        clipboard = new DesktopClipboardController(service, dispatcher);
        window = new MainWindow(service, clipboard);
        window.Closed += (_, _) => Cleanup();
        clipboard.ShelfFactory = () => new ShelfWindow(clipboard.Native, clipboard.History, clipboard);
        clipboard.Start(window.Handle);

        tray = new TrayIcon(window.Handle, "DeskFuse")
        {
            Activated = () => window?.RestoreFromTray(),
            MenuProvider = () => BuildTrayMenu(),
        };

        activationStop = new CancellationTokenSource();
        var token = activationStop.Token;
        _ = Task.Run(() =>
        {
            while (WaitHandle.WaitAny([activate, token.WaitHandle]) == 0) dispatcher.TryEnqueue(() => window?.RestoreFromTray());
        });

        service.Start();
        inputHelper = new InputHelperProcess(dispatcher);
        window.RestoreFromTray();
        if (Environment.GetCommandLineArgs().Any(argument => argument.Equals("--minimized", StringComparison.OrdinalIgnoreCase))) window.HideToTray();
    }

    private IReadOnlyList<TrayMenuEntry> BuildTrayMenu()
    {
        var entries = clipboard?.Entries.Take(20).ToArray() ?? [];
        var recent = new List<TrayMenuEntry>();
        if (entries.Length == 0) recent.Add(new TrayMenuEntry("还没有剪贴板记录", null, null, false));
        foreach (var entry in entries)
        {
            string title = entry.Title.Replace('\r', ' ').Replace('\n', ' ');
            if (title.Length > 42) title = title[..42] + "…";
            string label = $"{title}  ·  {entry.SourceName}";
            recent.Add(new TrayMenuEntry(label, () =>
            {
                try { window?.RestoreEntry(entry); }
                catch (Exception) { }
            }));
        }
        return
        [
            new TrayMenuEntry("打开 DeskFuse", () => window?.RestoreFromTray()),
            new TrayMenuEntry("最近剪贴板", null, recent),
            new TrayMenuEntry("暂存区", () => clipboard?.ShowShelf()),
            new TrayMenuEntry("设置", () => window?.ShowSettings()),
            new TrayMenuEntry("-"),
            new TrayMenuEntry("退出 DeskFuse", ExitApp),
        ];
    }

    private bool cleaned;

    internal void ExitApp()
    {
        Exiting = true;
        if (window is not null) window.AllowClose = true;
        window?.Close(); // Closed drives Cleanup; WinUI 3 has no Application.OnExit override.
        Exit();
    }

    private void Cleanup()
    {
        if (cleaned) return;
        cleaned = true;
        activationStop?.Cancel();
        inputHelper?.Dispose();
        clipboard?.Dispose();
        service?.Dispose();
        tray?.Dispose();
        activate?.Dispose();
        singleton?.Dispose();
    }
}
