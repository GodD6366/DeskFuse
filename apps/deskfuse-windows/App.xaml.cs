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
    private PowerStateController? power;
    private ShelfDragShakeMonitor? shelfDragMonitor;
    private MainWindow? window;
    private DispatcherQueueTimer? updateTimer;
    private bool updateChecking;
    internal bool Exiting { get; private set; }
    internal MainWindow? ClientWindow => window;

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
        var library = new ClipboardLibrary();
        clipboard = new DesktopClipboardController(service, dispatcher, library);
        power = new PowerStateController(clipboard.KeepAwake);
        window = new MainWindow(service, clipboard, power, library);
        if (power.StartupFailed)
        {
            clipboard.SetKeepAwake(false);
            window.Notify("Windows 未接受保持唤醒请求，已恢复为系统默认休眠行为。");
        }
        window.Closed += (_, _) => Cleanup();
        clipboard.ShelfFactory = () => new ShelfWindow(clipboard, service, window.Notify);
        clipboard.Start(window.Handle);
        shelfDragMonitor = new ShelfDragShakeMonitor(dispatcher, () => clipboard?.ShowShelfAtPointer(), message => window?.Notify(message));

        tray = null;
        try
        {
            tray = new TrayIcon(window.Handle, "DeskFuse")
            {
                Activated = () => window?.RestoreFromTray(),
                MenuProvider = () => BuildTrayMenu(),
            };
        }
        catch (Exception error)
        {
            // A missing tray icon must be visible, not silent: closing the window
            // would otherwise hide the client with no way back.
            window.Notify($"系统托盘不可用（{error.Message}）；请保持窗口打开，或从任务栏重新打开。");
        }

        activationStop = new CancellationTokenSource();
        var token = activationStop.Token;
        _ = Task.Run(() =>
        {
            while (WaitHandle.WaitAny([activate, token.WaitHandle]) == 0) dispatcher.TryEnqueue(() => window?.RestoreFromTray());
        });

        service.Start();
        inputHelper = new InputHelperProcess(dispatcher, message => window?.ReportInputHelperStatus(message));
        window.RestoreFromTray();
        // Onboarding builds themed visuals; it needs the window shown so the
        // theme resource chain (ActualTheme) resolves.
        ShowOnboardingIfNeeded();
        StartUpdateChecks();
        if (Environment.GetCommandLineArgs().Any(argument => argument.Equals("--minimized", StringComparison.OrdinalIgnoreCase))) window.HideToTray();
    }

    private void StartUpdateChecks()
    {
        updateTimer = DispatcherQueue.GetForCurrentThread().CreateTimer();
        updateTimer.Interval = TimeSpan.FromHours(12);
        updateTimer.Tick += async (_, _) => await CheckForUpdatesAsync();
        updateTimer.Start();
        _ = CheckForUpdatesAsync();
    }

    private async Task CheckForUpdatesAsync()
    {
        if (updateChecking || Exiting) return;
        updateChecking = true;
        try
        {
            if (!Version.TryParse(AppVersion.Display.Split(' ')[0], out var current)) return;
            var result = await UpdateChecker.CheckAsync(current);
            if (!Exiting) window?.ApplyAutomaticUpdateResult(result);
        }
        catch (Exception) { /* Automatic checks stay quiet; manual checks report the error in Settings. */ }
        finally { updateChecking = false; }
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
            new TrayMenuEntry("反馈问题…", () => SupportLinks.ReportProblem(service?.LatestSnapshot,
                clipboard?.Entries.Count, clipboard?.HistoryRetentionDays)),
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
        updateTimer?.Stop();
        activationStop?.Cancel();
        shelfDragMonitor?.Dispose();
        inputHelper?.Dispose();
        clipboard?.Dispose();
        power?.Dispose();
        service?.Dispose();
        tray?.Dispose();
        activate?.Dispose();
        singleton?.Dispose();
    }

    private const int OnboardingVersion = 1;

    private void ShowOnboardingIfNeeded()
    {
        // Unpackaged apps have no package identity, so ApplicationData is
        // unavailable; the registry keeps the per-user onboarding marker.
        using var key = Microsoft.Win32.Registry.CurrentUser.CreateSubKey(@"Software\DeskFuse");
        if (key.GetValue("OnboardingVersion") is int completed && completed >= OnboardingVersion) return;
        ShowOnboarding();
    }

    /// 设置 › 调试入口：与 macOS 的“重走初次设置”一致，沿用当前开关与权限，不清数据。
    internal void RestartOnboarding()
    {
        using var key = Microsoft.Win32.Registry.CurrentUser.CreateSubKey(@"Software\DeskFuse");
        key.DeleteValue("OnboardingVersion", false);
        window?.RestoreFromTray();
        ShowOnboarding();
    }

    private void ShowOnboarding()
    {
        if (window is null || clipboard is null) return;
        window.ShowOnboarding(new OnboardingPage(clipboard, result =>
        {
            window?.ApplyOnboarding(result);
            using var completedKey = Microsoft.Win32.Registry.CurrentUser.CreateSubKey(@"Software\DeskFuse");
            completedKey.SetValue("OnboardingVersion", OnboardingVersion);
            window?.HideOnboarding();
        }));
    }
}
