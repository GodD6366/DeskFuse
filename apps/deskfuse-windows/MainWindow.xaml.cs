using System.Diagnostics;
using System.Net;
using System.Net.Sockets;
using System.Text.Json;
using System.Text.Json.Nodes;
using Microsoft.UI;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Data;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Windows.Storage;

namespace DeskFuse.Windows;

public sealed partial class MainWindow : Window
{
    private readonly ServiceClient service;
    private readonly DesktopClipboardController clipboard;
    private readonly PowerStateController power;
    private readonly WorkflowExecutionPort workflowExecution;
    private SettingsPage? settings;
    private WorkspacePage selectedPage = WorkspacePage.Devices;
    private enum WorkspacePage { Clipboard, Input, RemoteApplications, Devices, ShareAndTransfer, Microphone, MouseButtons }
    private List<DeviceRow> devices = [];
    private bool updatingHistory;
    private bool updatingSync;
    private bool updatingFeature;
    private bool wasConnected;
    private bool loaded;
    private bool settingsVisible;
    private bool remoteAppsEnabled = true;
    private readonly ClipboardLibrary library;
    private ClipboardLibrary.Selection librarySelection = ClipboardLibrary.Selection.All;
    private ClipboardEntry[] canonicalRows = [];
    private ClipboardEntry[] visibleEntries = [];
    private long lastLibraryRevision = -1;
    private DateTime lastLibraryDay;
    private bool historyRefreshRunning;
    private bool historyRefreshPending;
    private string? lastHistoryPresentation;
    private string? lastHistoryFilter;
    private readonly System.Collections.ObjectModel.ObservableCollection<HistoryGroup> historyGroups = [];
    private CollectionViewSource? historyView;
    private ClipboardEntry? previewEntry;
    private bool previewInitialized;
    private CanonicalState? canonicalState;
    private Dictionary<string, long> canonicalCounts = [];
    private int historyVersion;
    private bool usingCanonical;
    private int canonicalOffset;
    private bool canonicalHasMore;
    private bool canonicalLoadingMore;
    private string localName = "这台 Windows";
    private string lastSettingsCategory = "general";
    private string? selectedDeviceId;
    private string? remoteSelectedDeviceId;
    private string lastDeviceSnapshot = "";
    private string lastErrorNotice = "";
    private string lastAutomaticUpdateTag = "";
    private JsonElement? latest;
    private bool busy;
    private readonly List<WorkflowRecord> workflows = [];
    private readonly List<string> stagedFiles = [];
    private string sendTargetSignature = "";
    private WorkflowRecord? editingWorkflow;
    private Border? draggingDisplay;
    private global::Windows.Foundation.Point dragOrigin;
    /// <summary>Canvas transform of the last layout render, needed to translate a
    /// dragged tile back into shared display coordinates.</summary>
    private double layoutScale = 1, layoutOffsetX, layoutOffsetY, layoutOriginX, layoutOriginY;
    private readonly List<InputLayoutRect> layoutRects = [];
    private (double X, double Y) dragTranslation;
    private sealed record PendingLayoutMove(string DeviceId, InputLayoutRect[] Before, double X, double Y);
    private PendingLayoutMove? pendingLayoutMove;
    private readonly Dictionary<string, ClipboardContentEditorWindow> contentEditors = [];
    private readonly Dictionary<int, WorkflowRecord> hotkeys = [];
    private readonly TextBox manualIp = new() { PlaceholderText = "192.168.1.10" };
    private Button? pairIpButton;
    private DfToggle? featureToggle;
    private DfToggle? inputControllerToggle;
    private bool? pendingInputController;
    private DateTime inputControllerDeadline;
    private DfToggle? lanAccessToggle;
    private UIElement? lanQrElement;
    private string lanQrSignature = "";
    private bool updatingLanWeb;
    private List<(string Tone, string Title, string Detail)> healthIssues = [];
    private ClipboardPanelWindow? clipboardPanel;
    private IntPtr clipboardPasteTarget;
    private ClipboardHotkey clipboardShortcut = ClipboardHotkey.Default;
    private int clipboardHotkeyId = 0x5600;
    private bool clipboardHotkeyRegistered;
    private Action<ClipboardHotkey?>? clipboardShortcutRecorder;
    private static string ClipboardShortcutPath => Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "DeskFuse", "desktop", "clipboard-shortcut-v1.json");
    internal ClipboardHotkey CurrentClipboardShortcut => clipboardShortcut;
    private const uint WmHotKey = 0x0312;
    private const uint ModAlt = 0x0001;
    private const uint ModControl = 0x0002;
    private const uint ModShift = 0x0004;
    private const uint ModWin = 0x0008;
    public IntPtr Handle { get; }
    /// <summary>Set by the app before a real shutdown; otherwise close hides to tray.</summary>
    public bool AllowClose { get; set; }

    public MainWindow(ServiceClient service, DesktopClipboardController clipboard, PowerStateController power, ClipboardLibrary library)
    {
        this.service = service;
        this.clipboard = clipboard;
        this.power = power;
        this.library = library;
        InitializeComponent();
        BrandMark.Apply(BrandMarkImage, Root);
        thumbnailCache = new ClipboardThumbnailCache(clipboard);
        library.Changed += () => DispatcherQueue.TryEnqueue(RebuildLibrarySidebar);
        RebuildLibrarySidebar();
        ExtendsContentIntoTitleBar = true;
        SetTitleBar(TitleBarDragRegion);
        try
        {
            AppWindow.TitleBar.ButtonBackgroundColor = Colors.Transparent;
            AppWindow.TitleBar.ButtonInactiveBackgroundColor = Colors.Transparent;
        }
        catch (Exception) { /* Older shells fall back to the system caption. */ }
        Navigate(WorkspacePage.Devices);
        Handle = WinRT.Interop.WindowNative.GetWindowHandle(this);
        AppIcon.Apply(this);
        AppWindow.Title = "DeskFuse";
        var windowId = Microsoft.UI.Win32Interop.GetWindowIdFromWindow(Handle);
        var workArea = global::Microsoft.UI.Windowing.DisplayArea.GetFromWindowId(windowId, global::Microsoft.UI.Windowing.DisplayAreaFallback.Nearest).WorkArea;
        int windowWidth = Math.Clamp((int)(workArea.Width * 0.78), Math.Min(1280, workArea.Width), workArea.Width);
        int windowHeight = Math.Clamp((int)(workArea.Height * 0.82), Math.Min(800, workArea.Height), workArea.Height);
        if (AppWindow.Presenter is global::Microsoft.UI.Windowing.OverlappedPresenter presenter)
        {
            presenter.PreferredMinimumWidth = Math.Min(1080, workArea.Width);
            presenter.PreferredMinimumHeight = Math.Min(700, workArea.Height);
            // The mac shell draws its own traffic lights; removing the system
            // titlebar avoids a second set of caption buttons on the right.
            presenter.SetBorderAndTitleBar(true, false);
        }
        AppWindow.Move(new global::Windows.Graphics.PointInt32(workArea.X + (workArea.Width - windowWidth) / 2, workArea.Y + (workArea.Height - windowHeight) / 2));
        AppWindow.Resize(new global::Windows.Graphics.SizeInt32(windowWidth, windowHeight));
        AppWindow.TitleBar.ExtendsContentIntoTitleBar = true;
        AppWindow.Closing += (_, args) =>
        {
            if (AllowClose)
            {
                foreach (var editor in contentEditors.Values.ToArray()) editor.Close();
                var panel = clipboardPanel;
                clipboardPanel = null;
                panel?.Dismiss();
                RemoveHotkeys();
                UnregisterHotKey(Handle, clipboardHotkeyId);
                UnregisterHotKey(Handle, 0x4400);
                return;
            }
            args.Cancel = true;
            HideToTray();
        };
        workflowExecution = new WindowsWorkflowExecutionPort(service, clipboard);
        clipboard.Entries.CollectionChanged += (_, _) => { if (!clipboard.History.IsReplacing) RefreshHistory(); };
        clipboard.Error += Notify;
        clipboard.FileOfferReceived += FileOfferArrived;
        clipboard.FileTransferSettled += FileTransferDone;
        service.SnapshotChanged += SnapshotChanged;
        service.ConnectionChanged += ConnectionChanged;
        Root.KeyDown += KeyboardShortcut;
        RefreshHistory();
        BuildMouseBindings();
        inputControllerToggle = DfUi.Switch(false, InputControllerToggled, false);
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(inputControllerToggle, "此电脑作为主控");
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(inputControllerToggle, "input-controller-toggle");
        InputControllerToggleHost.Children.Add(inputControllerToggle);
        lanAccessToggle = DfUi.Switch(false, LanAccessToggled, false);
        LanAccessToggleHost.Children.Add(lanAccessToggle);
        ConnectionChanged(false, "正在连接 DeskFuseService…");
        RenderLanWeb(default);
        DrawInputLayout(default);
        Root.Loaded += (_, _) =>
        {
            loaded = true;
            ExtendsContentIntoTitleBar = true;
            SetTitleBar(TitleBarDragRegion);
            UpdateCaptionButtons();
            Root.ActualThemeChanged += (_, _) => UpdateCaptionButtons();
            InstallWorkflowHotkeys();
            InstallClipboardPanelHotkey();
            InstallHistoryPaging();
            WindowMessageHook.Attach(Handle, ShortcutRecorderProc);
        };
    }

    private bool historyPagingInstalled;

    private void InstallHistoryPaging()
    {
        if (historyPagingInstalled) return;
        var viewer = FindDescendant<ScrollViewer>(HistoryList);
        if (viewer is null) return;
        historyPagingInstalled = true;
        viewer.ViewChanged += (_, _) =>
        {
            if (canonicalLoadingMore || !canonicalHasMore) return;
            if (viewer.ExtentHeight - viewer.VerticalOffset - viewer.ViewportHeight < 240)
                _ = LoadMoreCanonicalAsync();
        };
    }

    private static T? FindDescendant<T>(DependencyObject root) where T : DependencyObject
    {
        int count = VisualTreeHelper.GetChildrenCount(root);
        for (int index = 0; index < count; index++)
        {
            var child = VisualTreeHelper.GetChild(root, index);
            if (child is T match) return match;
            if (FindDescendant<T>(child) is { } nested) return nested;
        }
        return null;
    }

    private async Task LoadMoreCanonicalAsync()
    {
        if (canonicalLoadingMore || !canonicalHasMore || !usingCanonical) return;
        canonicalLoadingMore = true;
        int version = historyVersion;
        string query = HistorySearch.Text.Trim();
        string? listId = librarySelection.ListId;
        string kind = librarySelection.Kind ?? "all";
        string? source = selectedSource == "所有设备" ? null : selectedSource;
        try
        {
            var page = await clipboard.QueryCanonicalAsync(query, listId, kind, source, canonicalOffset, 100, SelectedRules());
            if (version != historyVersion) return;
            canonicalRows = canonicalRows.Concat(page.Items.Select(CanonicalRow.ToEntry)).ToArray();
            canonicalOffset = (int)(page.NextOffset ?? canonicalRows.Length);
            canonicalHasMore = page.NextOffset is not null;
            updatingHistory = true;
            try { RenderVisibleEntries(canonicalRows, page.Total, (int)page.Total); }
            finally { updatingHistory = false; }
            _ = LoadCanonicalThumbnailsAsync(version);
        }
        catch (Exception ex) when (ex is IOException or InvalidOperationException or OperationCanceledException or TimeoutException)
        {
            Notify($"加载更多失败：{ex.Message}");
        }
        finally { canonicalLoadingMore = false; }
    }

    private void InstallClipboardPanelHotkey()
    {
        WindowMessageHook.Attach(Handle, ClipboardPanelHotkeyProc);
        try { clipboardShortcut = ClipboardHotkey.Load(ClipboardShortcutPath); }
        catch (Exception ex) { Notify($"快捷键配置读取失败，暂用 Ctrl+Shift+V：{ex.Message}"); }
        clipboardHotkeyRegistered = clipboardShortcut.Enabled && RegisterHotKey(Handle, clipboardHotkeyId, clipboardShortcut.Modifiers, clipboardShortcut.Key);
        if (clipboardShortcut.Enabled && !clipboardHotkeyRegistered)
            Notify($"无法注册剪贴板快捷键 {clipboardShortcut.Label}；请在设置 → 快捷键更改。");
        // Ctrl+Shift+D = 0x44 (D key)
        if (!RegisterHotKey(Handle, 0x4400, ModControl | ModShift, 0x44))
        {
            Notify("无法注册暂存区快捷键 Ctrl+Shift+D；可能已被其他应用占用。");
        }
    }

    private bool ClipboardPanelHotkeyProc(uint message, IntPtr wParam, IntPtr lParam)
    {
        if (clipboardShortcutRecorder is not null && message is 0x0100 or 0x0104 or WmHotKey)
        {
            uint key = message == WmHotKey ? (uint)((lParam.ToInt64() >> 16) & 0xffff) : (uint)wParam.ToInt32();
            if (key is 16 or 17 or 18 or 91 or 92) return true;
            uint modifiers = message == WmHotKey ? (uint)(lParam.ToInt64() & 15) :
                (IsDown(global::Windows.System.VirtualKey.Control) ? ModControl : 0)
                | (IsDown(global::Windows.System.VirtualKey.Menu) ? ModAlt : 0)
                | (IsDown(global::Windows.System.VirtualKey.Shift) ? ModShift : 0)
                | (IsDown(global::Windows.System.VirtualKey.LeftWindows) || IsDown(global::Windows.System.VirtualKey.RightWindows) ? ModWin : 0);
            var callback = clipboardShortcutRecorder;
            clipboardShortcutRecorder = null;
            callback(key == 27 && modifiers == 0 ? null : new ClipboardHotkey(key, modifiers));
            return true;
        }
        if (message != WmHotKey) return false;
        if (wParam.ToInt32() == clipboardHotkeyId)
        {
            ToggleClipboardPanel();
            return true;
        }
        if (wParam.ToInt32() == 0x4400)
        {
            clipboard.ToggleShelf();
            return true;
        }
        return false;
    }

    internal void BeginClipboardShortcutRecording(Action<ClipboardHotkey?> callback) => clipboardShortcutRecorder = callback;

    internal void StopClipboardShortcutRecording()
    {
        var callback = clipboardShortcutRecorder;
        clipboardShortcutRecorder = null;
        callback?.Invoke(null);
    }

    internal void SaveClipboardShortcut(ClipboardHotkey value)
    {
        if (!value.IsValid) throw new InvalidOperationException("快捷键必须包含 Ctrl、Alt 或 Win。");
        if (value.Enabled && value.Key == 0x44 && value.Modifiers == (ModControl | ModShift))
            throw new InvalidOperationException("Ctrl+Shift+D 已用于文件暂存区，请选择其他组合。");
        if (value.Enabled && hotkeys.Values.Any(workflow => workflow.Shortcut is { } shortcut
            && shortcut.KeyCode == value.Key && WorkflowModifiers(shortcut.Modifiers) == value.Modifiers))
            throw new InvalidOperationException("此快捷键已用于工作流；原快捷键保持不变。");
        if (value == clipboardShortcut && (!value.Enabled || clipboardHotkeyRegistered)) return;
        int candidateId = clipboardHotkeyId == 0x5600 ? 0x5601 : 0x5600;
        if (value.Enabled && !RegisterHotKey(Handle, candidateId, value.Modifiers, value.Key))
            throw new InvalidOperationException("此快捷键已被占用；原快捷键保持不变。");
        try { value.Save(ClipboardShortcutPath); }
        catch { if (value.Enabled) UnregisterHotKey(Handle, candidateId); throw; }
        UnregisterHotKey(Handle, clipboardHotkeyId);
        clipboardHotkeyId = candidateId;
        clipboardHotkeyRegistered = value.Enabled;
        clipboardShortcut = value;
    }

    private void ToggleClipboardPanel()
    {
        if (clipboardPanel is not null)
        {
            var current = clipboardPanel;
            clipboardPanel = null;
            current.Dismiss();
            return;
        }
        // Capture before the panel takes focus. If the hotkey was pressed inside
        // DeskFuse, retain the most recent external destination instead.
        var foreground = ClipboardPanelWindow.CaptureExternalTarget();
        if (foreground != IntPtr.Zero) clipboardPasteTarget = foreground;
        var panel = new ClipboardPanelWindow(
            clipboard,
            library,
            clipboardPasteTarget,
            Notify,
            () => { Navigate(WorkspacePage.Clipboard); Activate(); },
            SharePeers,
            ShareEntryAsync,
            HistoryWorkflows,
            RunHistoryWorkflow);
        clipboardPanel = panel;
        panel.Closed += (_, _) => { if (ReferenceEquals(clipboardPanel, panel)) clipboardPanel = null; };
        panel.Activate();
    }

    private void UpdateCaptionButtons()
    {
        try
        {
            bool dark = Root.ActualTheme == ElementTheme.Dark;
            var bar = AppWindow.TitleBar;
            // The mac shell owns the titlebar; the system caption buttons would
            // duplicate the traffic lights on the left.
            bar.PreferredHeightOption = global::Microsoft.UI.Windowing.TitleBarHeightOption.Collapsed;
            bar.ButtonBackgroundColor = dark ? global::Windows.UI.Color.FromArgb(255, 26, 26, 30) : global::Windows.UI.Color.FromArgb(255, 244, 244, 245);
            bar.ButtonInactiveBackgroundColor = bar.ButtonBackgroundColor;
            bar.ButtonForegroundColor = dark ? Colors.White : global::Windows.UI.Color.FromArgb(255, 26, 26, 30);
            bar.ButtonInactiveForegroundColor = dark ? global::Windows.UI.Color.FromArgb(140, 255, 255, 255) : global::Windows.UI.Color.FromArgb(140, 26, 26, 30);
            bar.ButtonHoverBackgroundColor = dark ? global::Windows.UI.Color.FromArgb(36, 255, 255, 255) : global::Windows.UI.Color.FromArgb(18, 0, 0, 0);
            bar.ButtonHoverForegroundColor = bar.ButtonForegroundColor;
            bar.ButtonPressedBackgroundColor = dark ? global::Windows.UI.Color.FromArgb(52, 255, 255, 255) : global::Windows.UI.Color.FromArgb(26, 0, 0, 0);
            bar.ButtonPressedForegroundColor = bar.ButtonForegroundColor;
        }
        catch (Exception) { /* Older shells keep the system caption. */ }
    }

    private void CloseLightClick(object sender, RoutedEventArgs args) => HideToTray();

    private void MinimizeLightClick(object sender, RoutedEventArgs args) => MinimizeFromTitleBar();

    private void MaximizeLightClick(object sender, RoutedEventArgs args) => ToggleMaximizeFromTitleBar();

    internal void MinimizeFromTitleBar()
    {
        if (AppWindow.Presenter is global::Microsoft.UI.Windowing.OverlappedPresenter presenter)
            presenter.Minimize();
    }

    internal void ToggleMaximizeFromTitleBar()
    {
        if (AppWindow.Presenter is not global::Microsoft.UI.Windowing.OverlappedPresenter presenter) return;
        if (presenter.State == global::Microsoft.UI.Windowing.OverlappedPresenterState.Maximized) presenter.Restore();
        else presenter.Maximize();
    }

    private void TrafficLightEntered(object sender, PointerRoutedEventArgs args)
    {
        if (sender is Button { Content: Grid grid } && grid.Children.Count > 1 && grid.Children[1] is TextBlock glyph)
            glyph.Visibility = Visibility.Visible;
    }

    private void TrafficLightExited(object sender, PointerRoutedEventArgs args)
    {
        if (sender is Button { Content: Grid grid } && grid.Children.Count > 1 && grid.Children[1] is TextBlock glyph)
            glyph.Visibility = Visibility.Collapsed;
    }

    public void HideToTray() => AppWindow.Hide();
    public void RestoreFromTray()
    {
        AppWindow.Show();
        Activate();
    }

    // MARK: - Navigation

    private static readonly (string Key, string Title)[] SettingsCategoryList =
    [
        ("general", "通用"), ("clipboard", "剪贴板"), ("shortcuts", "快捷键"), ("files", "文件"),
        ("permissions", "权限"), ("about", "关于与更新"), ("advanced", "高级"),
    ];

    private static string PageTitleFor(WorkspacePage page) => page switch
    {
        WorkspacePage.Devices => "设备",
        WorkspacePage.Clipboard => "剪贴板",
        WorkspacePage.Input => "键鼠共享",
        WorkspacePage.ShareAndTransfer => "局域网传输",
        WorkspacePage.RemoteApplications => "远程应用",
        WorkspacePage.Microphone => "麦克风共享",
        WorkspacePage.MouseButtons => "鼠标自定义",
        _ => "DeskFuse",
    };

    private static string SectionFor(WorkspacePage page) => page switch
    {
        WorkspacePage.Devices => "配置",
        WorkspacePage.Clipboard or WorkspacePage.Input or WorkspacePage.ShareAndTransfer => "基础",
        _ => "高级",
    };

    private static bool IsBeta(WorkspacePage page) =>
        page is WorkspacePage.RemoteApplications or WorkspacePage.Microphone or WorkspacePage.MouseButtons;

    private static string CategoryTitle(string key) => key switch
    {
        "workflows" => "剪贴板",
        _ => SettingsCategoryList.FirstOrDefault(item => item.Key == key).Title ?? "通用",
    };

    private void BuildNavigation()
    {
        NavPanel.Children.Clear();
        NavPanel.Children.Add(SectionHeader("基础"));
        AddNavRow(WorkspacePage.Clipboard, "剪贴板", "\uE8C8", false, null);
        AddNavRow(WorkspacePage.Input, "键鼠共享", "\uE962", false, null);
        AddNavRow(WorkspacePage.ShareAndTransfer, "局域网传输", "\uE8AB", false, null);
        NavPanel.Children.Add(SectionHeader("高级"));
        AddNavRow(WorkspacePage.RemoteApplications, "远程应用", "\uE8A7", true, null);
        AddNavRow(WorkspacePage.Microphone, "麦克风共享", "\uE720", true, null);
        AddNavRow(WorkspacePage.MouseButtons, "鼠标自定义", "\uE8B1", true, null);
        NavPanel.Children.Add(SectionHeader("配置"));
        int connected = latest is JsonElement snapshot
            ? snapshot.Array("peers").Count(peer => peer.String("connection") == "connected")
            : 0;
        AddNavRow(WorkspacePage.Devices, "设备", "\uE772", false, $"{connected} 台已连接");
    }

    private static TextBlock SectionHeader(string title)
    {
        var block = DfUi.Text(title, 11, true, DfUi.Muted);
        block.Margin = new Thickness(10, 12, 0, 4);
        return block;
    }

    private void AddNavRow(WorkspacePage page, string title, string glyph, bool beta, string? trailing)
    {
        bool selected = !settingsVisible && selectedPage == page;
        var row = new Button
        {
            Background = selected ? DfUi.Surface : new SolidColorBrush(Colors.Transparent),
            BorderBrush = selected ? DfUi.Line : new SolidColorBrush(Colors.Transparent),
            BorderThickness = new Thickness(1),
            CornerRadius = new CornerRadius(8),
            Padding = new Thickness(10, 0, 10, 0),
            MinHeight = 32,
            MinWidth = 0,
            HorizontalAlignment = HorizontalAlignment.Stretch,
            HorizontalContentAlignment = HorizontalAlignment.Stretch,
            VerticalContentAlignment = VerticalAlignment.Center,
            Template = (ControlTemplate)Application.Current.Resources["DFFlatButtonTemplate"],
        };
        var grid = new Grid();
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(18) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var icon = DfUi.Glyph(glyph, 14);
        icon.Foreground = selected ? DfUi.Accent : DfUi.Muted;
        icon.HorizontalAlignment = HorizontalAlignment.Left;
        grid.Children.Add(icon);
        var label = DfUi.Text(title, 13, false, DfUi.PrimaryText, wrap: false);
        label.Opacity = selected ? 1 : 0.82;
        label.VerticalAlignment = VerticalAlignment.Center;
        label.Margin = new Thickness(10, 0, 0, 0);
        Grid.SetColumn(label, 1);
        grid.Children.Add(label);
        if (beta)
        {
            var badge = DfUi.Badge("即将支持");
            badge.Margin = new Thickness(6, 0, 0, 0);
            Grid.SetColumn(badge, 2);
            grid.Children.Add(badge);
        }
        if (trailing is not null)
        {
            var tail = DfUi.Text(trailing, 11, false, DfUi.Muted, wrap: false);
            tail.VerticalAlignment = VerticalAlignment.Center;
            tail.Margin = new Thickness(8, 0, 0, 0);
            Grid.SetColumn(tail, 3);
            grid.Children.Add(tail);
        }
        row.Content = grid;
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(row, title);
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(row, $"nav-{page}");
        row.Click += (_, _) => ShowPage(page);
        if (!selected)
        {
            row.PointerEntered += (_, _) => row.Background = DfUi.Control;
            row.PointerExited += (_, _) => row.Background = new SolidColorBrush(Colors.Transparent);
        }
        NavPanel.Children.Add(row);
    }

    private void BuildSettingsCategories(string selectedKey)
    {
        SettingsCategories.Children.Clear();
        var grid = new Grid();
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        var line = new Border
        {
            Width = 1,
            Background = DfUi.Line,
            HorizontalAlignment = HorizontalAlignment.Left,
            Margin = new Thickness(8, 4, 0, 4),
        };
        grid.Children.Add(line);
        var panel = new StackPanel { Spacing = 2, Margin = new Thickness(10, 0, 0, 0) };
        foreach (var (key, title) in SettingsCategoryList)
        {
            bool selected = key == selectedKey;
            var row = new Button
            {
                Background = selected ? DfUi.Surface : new SolidColorBrush(Colors.Transparent),
                BorderBrush = selected ? DfUi.Line : new SolidColorBrush(Colors.Transparent),
                BorderThickness = new Thickness(1),
                CornerRadius = new CornerRadius(8),
                Padding = new Thickness(10, 0, 10, 0),
                MinHeight = 30,
                MinWidth = 0,
                HorizontalAlignment = HorizontalAlignment.Stretch,
                HorizontalContentAlignment = HorizontalAlignment.Stretch,
                VerticalContentAlignment = VerticalAlignment.Center,
                Template = (ControlTemplate)Application.Current.Resources["DFFlatButtonTemplate"],
                Content = DfUi.Text(title, 13, false, DfUi.PrimaryText, wrap: false),
            };
            var captured = key;
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(row, title);
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(row, $"settings-{captured}");
            row.Click += (_, _) => ShowSettings(captured);
            if (!selected)
            {
                row.PointerEntered += (_, _) => row.Background = DfUi.Control;
                row.PointerExited += (_, _) => row.Background = new SolidColorBrush(Colors.Transparent);
            }
            panel.Children.Add(row);
        }
        Grid.SetColumn(panel, 1);
        grid.Children.Add(panel);
        SettingsCategories.Children.Add(grid);
    }

    private void UpdateSettingsEntry()
    {
        SettingsButton.Background = settingsVisible ? DfUi.Surface : new SolidColorBrush(Colors.Transparent);
        SettingsButton.BorderBrush = settingsVisible ? DfUi.Line : new SolidColorBrush(Colors.Transparent);
        SettingsIcon.Foreground = settingsVisible ? DfUi.Accent : DfUi.Muted;
        SettingsCategories.Visibility = settingsVisible ? Visibility.Visible : Visibility.Collapsed;
    }

    private void Navigate(WorkspacePage page) => ShowPage(page);

    private void ShowPage(WorkspacePage page)
    {
        StopClipboardShortcutRecording();
        if (selectedPage == WorkspacePage.Input && page != WorkspacePage.Input) displayWallpapers.Clear();
        selectedPage = page;
        settingsVisible = false;
        SettingsHost.Visibility = Visibility.Collapsed;
        SettingsCategories.Visibility = Visibility.Collapsed;
        ClipboardPage.Visibility = page == WorkspacePage.Clipboard ? Visibility.Visible : Visibility.Collapsed;
        InputPage.Visibility = page == WorkspacePage.Input ? Visibility.Visible : Visibility.Collapsed;
        DevicesPage.Visibility = page == WorkspacePage.Devices ? Visibility.Visible : Visibility.Collapsed;
        TransfersPage.Visibility = page == WorkspacePage.ShareAndTransfer ? Visibility.Visible : Visibility.Collapsed;
        RemoteApplicationsPage.Visibility = page == WorkspacePage.RemoteApplications ? Visibility.Visible : Visibility.Collapsed;
        MicrophonePage.Visibility = page == WorkspacePage.Microphone ? Visibility.Visible : Visibility.Collapsed;
        MouseButtonsPage.Visibility = page == WorkspacePage.MouseButtons ? Visibility.Visible : Visibility.Collapsed;
        PageTitle.Text = PageTitleFor(page);
        SectionLabel.Text = SectionFor(page);
        PageBadge.Visibility = IsBeta(page) ? Visibility.Visible : Visibility.Collapsed;
        UpdateSettingsEntry();
        BuildNavigation();
        BuildFeatureToggle();
        if (page == WorkspacePage.RemoteApplications) UpdateRemoteApplicationsHost();
        if (page == WorkspacePage.Input)
            DrawInputLayout(latest.HasValue && latest.Value.TryGetProperty("input", out var layoutInput) ? layoutInput : default);
        if (page == WorkspacePage.Devices) RebuildDevices();
    }

    private void SettingsClick(object sender, RoutedEventArgs args) => ShowSettings(lastSettingsCategory);

    private void WorkflowsClick(object sender, RoutedEventArgs args) => ShowSettings("workflows");

    public void ShowSettings() => ShowSettings(lastSettingsCategory);

    /// <summary>After adopting one device's history, show exactly that device's
    /// rows so the result of the settings action is visible immediately. The
    /// canonical source token may be the stable device id while the label is
    /// the current display name.</summary>
    internal void ShowDeviceHistory(string sourceToken, string sourceLabel)
    {
        selectedSource = sourceToken;
        SourceFilterLabel.Text = sourceLabel;
        ShowPage(WorkspacePage.Clipboard);
        RefreshHistory();
    }

    internal void ApplyAutomaticUpdateResult(UpdateChecker.Result result)
    {
        settings?.ApplyUpdateResult(result);
        if (result.UpdateAvailable && result.LatestTag is string tag && tag != lastAutomaticUpdateTag)
        {
            lastAutomaticUpdateTag = tag;
            Notify(result.Summary);
        }
    }

    private void ShowSettings(string category)
    {
        if (settings is null)
        {
            PageHost.Children.Remove(WorkflowsPage);
            WorkflowsPage.Visibility = Visibility.Collapsed;
            settings = new SettingsPage(clipboard, service, power, WorkflowsPage, () => _ = LoadWorkflowsAsync(),
                () => ShowPage(WorkspacePage.Input), this);
            SettingsHost.Children.Add(settings);
            if (UpdateChecker.LastResult is { } update) settings.ApplyUpdateResult(update);
        }
        if (category == "workflows")
        {
            lastSettingsCategory = "clipboard";
            settings.Show("workflows");
        }
        else
        {
            lastSettingsCategory = category;
            settings.Show(category);
        }
        settingsVisible = true;
        ClipboardPage.Visibility = Visibility.Collapsed;
        InputPage.Visibility = Visibility.Collapsed;
        DevicesPage.Visibility = Visibility.Collapsed;
        TransfersPage.Visibility = Visibility.Collapsed;
        RemoteApplicationsPage.Visibility = Visibility.Collapsed;
        MicrophonePage.Visibility = Visibility.Collapsed;
        MouseButtonsPage.Visibility = Visibility.Collapsed;
        SettingsHost.Visibility = Visibility.Visible;
        FeatureControlsHost.Children.Clear();
        featureToggle = null;
        SectionLabel.Text = "设置";
        PageTitle.Text = CategoryTitle(lastSettingsCategory);
        PageBadge.Visibility = Visibility.Collapsed;
        UpdateSettingsEntry();
        BuildSettingsCategories(lastSettingsCategory);
        BuildNavigation();
    }

    // MARK: - Feature controls

    private void BuildFeatureToggle()
    {
        FeatureControlsHost.Children.Clear();
        featureToggle = null;
        if (IsBeta(selectedPage) || selectedPage == WorkspacePage.Input) return;
        DfToggle? toggle = selectedPage switch
        {
            WorkspacePage.Clipboard => DfUi.Switch(clipboard.SyncEnabled, ClipboardFeatureToggled),
            WorkspacePage.Input => DfUi.Switch(InputFeatureEnabled(), InputFeatureToggled, latest.HasValue),
            WorkspacePage.RemoteApplications => DfUi.Switch(remoteAppsEnabled, RemoteAppsToggled),
            WorkspacePage.ShareAndTransfer => DfUi.Switch(FileTransferEnabled(), FileTransferToggled, FileTransferAvailable()),
            WorkspacePage.Microphone or WorkspacePage.MouseButtons => DfUi.Switch(false, null, false),
            _ => null,
        };
        if (toggle is null) return;
        string tooltip = selectedPage switch
        {
            WorkspacePage.Clipboard => clipboard.SyncEnabled ? "关闭剪贴板同步并停止相关后台任务" : "开启剪贴板同步",
            WorkspacePage.Input => InputFeatureEnabled() ? "关闭键鼠共享并停止相关后台任务" : "开启键鼠共享",
            WorkspacePage.RemoteApplications => remoteAppsEnabled ? "关闭远程应用" : "开启远程应用",
            WorkspacePage.ShareAndTransfer => FileTransferEnabled() ? "关闭文件传输与局域网分享" : "开启文件传输",
            WorkspacePage.Microphone => "Windows 端暂不支持麦克风共享",
            WorkspacePage.MouseButtons => "Windows 端暂不支持鼠标自定义",
            _ => "总开关",
        };
        ToolTipService.SetToolTip(toggle, tooltip);
        featureToggle = toggle;
        FeatureControlsHost.Children.Add(toggle);
    }

    private void UpdateFeatureToggleState()
    {
        updatingFeature = true;
        try
        {
            if (featureToggle is not null)
            {
                switch (selectedPage)
                {
                    case WorkspacePage.Clipboard: featureToggle.IsOn = clipboard.SyncEnabled; break;
                    case WorkspacePage.Input:
                        featureToggle.IsOn = InputFeatureEnabled();
                        featureToggle.IsEnabled = latest.HasValue;
                        break;
                    case WorkspacePage.RemoteApplications: featureToggle.IsOn = remoteAppsEnabled; break;
                    case WorkspacePage.ShareAndTransfer:
                        featureToggle.IsOn = FileTransferEnabled();
                        featureToggle.IsEnabled = FileTransferAvailable();
                        break;
                }
            }
            if (inputControllerToggle is not null)
            {
                bool currentRole = InputControllerEnabled();
                if (pendingInputController is bool expected)
                {
                    if (currentRole == expected) pendingInputController = null;
                    else if (DateTime.UtcNow >= inputControllerDeadline)
                    {
                        pendingInputController = null;
                        Notify("主控切换尚未生效，已恢复当前状态。请检查输入组件与设备连接后重试。");
                    }
                }
                inputControllerToggle.IsOn = pendingInputController ?? currentRole;
                bool hasSettings = TryInputSettings(out _);
                bool managed = latest is JsonElement current && current.TryGetProperty("input", out var input) && input.Bool("managed");
                inputControllerToggle.IsEnabled = hasSettings && !managed && pendingInputController is null;
                ToolTipService.SetToolTip(InputControllerToggleHost, !hasSettings
                    ? "正在连接后台服务，暂时无法更改主控。"
                    : managed ? "当前由另一台设备控制；先停止被控，再切换主控。"
                    : pendingInputController is not null ? "正在保存主控设置…"
                    : "使用本机键鼠控制已添加的设备；不需要连接 Wi-Fi。");
            }
        }
        finally { updatingFeature = false; }
    }

    private bool InputFeatureEnabled()
    {
        if (latest is not JsonElement snapshot || !snapshot.TryGetProperty("input", out var input)
            || !input.TryGetProperty("settings", out var settings) || settings.ValueKind != JsonValueKind.Object)
            return false;
        return settings.Bool("feature_enabled");
    }

    private bool InputControllerEnabled()
    {
        if (latest is not JsonElement snapshot || !snapshot.TryGetProperty("input", out var input)
            || !input.TryGetProperty("settings", out var settings) || settings.ValueKind != JsonValueKind.Object)
            return false;
        return settings.Bool("feature_enabled")
            && settings.Bool("enabled")
            && settings.String("role") == "Controller";
    }

    private void ClipboardFeatureToggled(object sender, RoutedEventArgs args)
    {
        if (updatingFeature || sender is not DfToggle toggle) return;
        clipboard.SyncEnabled = toggle.IsOn;
        clipboard.CaptureEnabled = toggle.IsOn;
        SyncCheck.IsChecked = toggle.IsOn;
        if (featureToggle is not null)
            ToolTipService.SetToolTip(featureToggle, toggle.IsOn ? "关闭剪贴板同步并停止相关后台任务" : "开启剪贴板同步");
        Notify(toggle.IsOn ? "已开启剪贴板同步与历史记录。" : "已关闭剪贴板同步；相关后台任务已停止。");
    }

    private async void FileTransferToggled(object sender, RoutedEventArgs args)
    {
        if (updatingFeature || sender is not DfToggle toggle) return;
        if (!TryFileTransferSettings(out _))
        {
            Notify("文件传输设置尚未就绪，无法修改总开关。");
            return;
        }
        try
        {
            updatingFeature = true;
            await service.SendAsync(ServiceClient.Command("SetFileTransferEnabled", new { enabled = toggle.IsOn }));
            Notify(toggle.IsOn ? "已开启文件传输。" : "已关闭文件传输与局域网分享，活动任务已停止。");
        }
        catch (Exception ex) { Notify(ex is IOException ? ex.Message : "文件传输设置未保存，服务可能已断开。"); }
        finally
        {
            updatingFeature = false;
            UpdateFeatureToggleState();
        }
    }

    private async void LanAccessToggled(object sender, RoutedEventArgs args)
    {
        if (updatingLanWeb || sender is not DfToggle toggle) return;
        if (toggle.IsOn && !FileTransferEnabled())
        {
            Notify("文件传输已关闭，请先开启文件传输总开关。");
            updatingLanWeb = true;
            toggle.IsOn = false;
            updatingLanWeb = false;
            return;
        }
        try
        {
            updatingLanWeb = true;
            await service.SendAsync(ServiceClient.Command("ConfigureLanWeb", new { enabled = toggle.IsOn }));
            Notify(toggle.IsOn ? "已请求开启局域网访问；二维码将在监听成功后显示。" : "已关闭局域网访问。");
        }
        catch (Exception ex) { Notify(ex is IOException ? ex.Message : "局域网设置未保存，服务可能已断开。"); }
        finally
        {
            updatingLanWeb = false;
            if (latest is JsonElement snapshot) RenderLanWeb(snapshot);
        }
    }

    private void RemoteAppsToggled(object sender, RoutedEventArgs args)
    {
        if (updatingFeature || sender is not DfToggle toggle) return;
        remoteAppsEnabled = toggle.IsOn;
        UpdateRemoteApplicationsHost();
        Notify(remoteAppsEnabled ? "已开启远程应用。" : "远程应用已关闭。");
    }

    private async void InputFeatureToggled(object sender, RoutedEventArgs args)
    {
        if (updatingFeature || sender is not DfToggle toggle) return;
        if (!TryInputSettings(out var settings))
        {
            Notify("输入运行时尚未就绪，无法修改总开关。");
            return;
        }
        try
        {
            updatingFeature = true;
            var node = JsonNode.Parse(settings.GetRawText())?.AsObject() ?? throw new FormatException();
            node["feature_enabled"] = toggle.IsOn;
            await service.SendAsync(ServiceClient.Command("SetInputSharingSettings", new { settings = node }));
            Notify(toggle.IsOn ? "已开启键鼠共享功能。" : "已关闭键鼠共享功能，相关后台任务已停止。");
        }
        catch (FormatException) { Notify("输入设置格式无效。"); }
        catch (Exception ex) { Notify(ex is IOException ? ex.Message : "键鼠共享设置未保存，服务可能已断开。"); }
        finally
        {
            updatingFeature = false;
            UpdateFeatureToggleState();
        }
    }

    private async void InputControllerToggled(object sender, RoutedEventArgs args)
    {
        if (updatingFeature || sender is not DfToggle toggle) return;
        if (!TryInputSettings(out var settings))
        {
            Notify("输入运行时尚未就绪，无法切换主控角色。");
            return;
        }
        try
        {
            updatingFeature = true;
            var node = JsonNode.Parse(settings.GetRawText())?.AsObject() ?? throw new FormatException();
            pendingInputController = toggle.IsOn;
            inputControllerDeadline = DateTime.UtcNow.AddSeconds(5);
            inputControllerToggle!.IsEnabled = false;
            if (toggle.IsOn) node["feature_enabled"] = true;
            node["enabled"] = toggle.IsOn;
            node["role"] = toggle.IsOn ? "Controller" : "Disabled";
            await service.SendAsync(ServiceClient.Command("SetInputSharingSettings", new { settings = node }));
            Notify(toggle.IsOn ? "已提交主控设置；能否跨屏控制以连接与输入状态为准。" : "已请求停止主控。");
        }
        catch (FormatException) { pendingInputController = null; Notify("输入设置格式无效。"); }
        catch (Exception ex) { pendingInputController = null; Notify(ex is IOException ? ex.Message : "主控设置未保存，服务可能已断开。"); }
        finally
        {
            updatingFeature = false;
            UpdateFeatureToggleState();
        }
    }

    private bool TryInputSettings(out JsonElement settings)
    {
        settings = default;
        return latest is JsonElement snapshot
            && snapshot.TryGetProperty("input", out var input)
            && input.TryGetProperty("settings", out settings)
            && settings.ValueKind == JsonValueKind.Object;
    }

    private bool TryFileTransferSettings(out JsonElement settings)
    {
        settings = default;
        return latest is JsonElement snapshot
            && snapshot.TryGetProperty("file_settings", out settings)
            && settings.ValueKind == JsonValueKind.Object;
    }

    private bool FileTransferAvailable() => TryFileTransferSettings(out _);

    private bool FileTransferEnabled() => TryFileTransferSettings(out var settings) && settings.Bool("enabled");

    private void UpdateRemoteApplicationsHost()
    {
        if (RemoteApplicationsHost is null) return;
        RemoteApplicationsHost.Children.Clear();
        RemoteApplicationsHost.Children.Add(DfUi.EmptyState("\uE8A7", "即将支持",
            "Windows 端远程应用尚未开放。鼠标跨屏共享请前往键鼠共享页面。"));
    }

    // MARK: - Health

    private void HealthClick(object sender, RoutedEventArgs args)
    {
        var panel = new StackPanel { Spacing = 12, Width = 288 };
        var header = new Grid();
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        header.Children.Add(DfUi.Text("运行状态", 15, true, wrap: false));
        var refresh = DfUi.IconButton("\uE72C", async (_, _) => await RefreshDevicesAsync(), "刷新设备");
        Grid.SetColumn(refresh, 1);
        header.Children.Add(refresh);
        panel.Children.Add(header);
        if (healthIssues.Count == 0)
        {
            panel.Children.Add(DfUi.Detail("DeskFuse 正在正常工作。"));
        }
        else
        {
            foreach (var (tone, title, detail) in healthIssues)
            {
                var row = new StackPanel { Spacing = 3 };
                var heading = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
                heading.Children.Add(new Microsoft.UI.Xaml.Shapes.Ellipse
                {
                    Width = 6, Height = 6, Fill = DfUi.Brush(tone), VerticalAlignment = VerticalAlignment.Center,
                });
                heading.Children.Add(DfUi.Text(title, 13, true));
                row.Children.Add(heading);
                row.Children.Add(DfUi.Detail(detail));
                panel.Children.Add(row);
            }
        }
        panel.Children.Add(DfUi.Divider(4, 4));
        panel.Children.Add(DfUi.ValueRow("本机", localName));
        panel.Children.Add(DfUi.ValueRow("版本", $"v{AppVersion.Display}"));
        var flyout = new Flyout
        {
            Content = new Border
            {
                Background = DfUi.Surface,
                Padding = new Thickness(16),
                CornerRadius = new CornerRadius(12),
                BorderBrush = DfUi.Line,
                BorderThickness = new Thickness(1),
                Child = panel,
            },
        };
        flyout.ShowAt(HealthButton);
    }

    private string? inputHelperFailure;

    internal void ReportInputHelperStatus(string? message)
    {
        inputHelperFailure = message;
        if (latest is JsonElement snapshot) RefreshInputState(snapshot);
        UpdateHealth(service.IsConnected);
    }

    private void UpdateHealth(bool connected)
    {
        var issues = new List<(string Tone, string Title, string Detail)>();
        if (!connected) issues.Add(("DFDangerBrush", "服务未连接", "无法连接 DeskFuseService；本机剪贴板历史仍可用。"));
        if (!string.IsNullOrEmpty(clipboard.StorageError)) issues.Add(("DFWarningBrush", "剪贴板存储异常", clipboard.StorageError));
        if (connected && latest is JsonElement snapshot
            && !snapshot.Array("peers").Any(peer => peer.String("connection") == "connected"))
            issues.Add(("DFAccentBrush", "还没有连接其他电脑", "添加另一台电脑后，就能同步剪贴板、共享键鼠和传输文件。"));
        if (connected && InputFeatureEnabled() && latest is JsonElement inputState
            && !JsonProperty(inputState, "capabilities").Bool("input"))
            issues.Add(("DFWarningBrush", "鼠标共享未就绪", inputHelperFailure ?? "本机输入组件未就绪，请在设置 → 权限检查安装与授权状态。"));
        string tone = issues.Any(issue => issue.Tone == "DFDangerBrush") ? "DFDangerBrush"
            : issues.Any(issue => issue.Tone == "DFWarningBrush") ? "DFWarningBrush"
            : issues.Any(issue => issue.Tone == "DFAccentBrush") ? "DFAccentBrush"
            : "DFSuccessBrush";
        HealthDot.Fill = DfUi.Brush(tone);
        HealthSummary.Text = issues.Count switch
        {
            0 => "一切正常",
            1 => issues[0].Title,
            _ => $"{issues.Count} 项需要留意",
        };
        HealthChevron.Visibility = issues.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
        ToolTipService.SetToolTip(HealthButton, "查看运行状态与需要处理的问题");
        healthIssues = issues;
    }

    // MARK: - History

    private string selectedSource = "所有设备";
    private string displayMode = "list";
    private readonly ClipboardThumbnailCache thumbnailCache;


    private void SourceFilterClick(object sender, RoutedEventArgs args)
    {
        var flyout = new MenuFlyout();
        var sources = usingCanonical ? CanonicalSources() : new[] { "所有设备" }.Concat(clipboard.Entries.Select(e => e.SourceName).Distinct()).ToArray();
        foreach (var source in sources)
        {
            var item = new MenuFlyoutItem { Text = source };
            var captured = source;
            // Query by the stable device id when the display name belongs to a
            // known device; names can change without rewriting older rows.
            string token = source == "所有设备"
                ? source
                : canonicalState?.Devices.FirstOrDefault(device => device.Name == source)?.Id ?? source;
            item.Click += (_, _) =>
            {
                selectedSource = token;
                SourceFilterLabel.Text = captured;
                RefreshHistory();
            };
            flyout.Items.Add(item);
        }
        flyout.ShowAt(SourceFilterButton);
    }

    private string[] CanonicalSources()
    {
        var names = new SortedSet<string>(StringComparer.CurrentCulture)
        {
            "所有设备",
        };
        foreach (var entry in canonicalRows) names.Add(entry.SourceName);
        if (canonicalState is not null)
            foreach (var device in canonicalState.Devices)
                if (!string.IsNullOrWhiteSpace(device.Name)) names.Add(device.Name);
        return names.ToArray();
    }

    private void HistoryFilterChanged(object sender, RoutedEventArgs args) => RefreshHistory();

    private void DisplayModeListClick(object sender, RoutedEventArgs args)
    {
        displayMode = "list";
        DisplayModeListButton.Background = DfUi.Control;
        DisplayModeGridButton.Background = new SolidColorBrush(Colors.Transparent);
        HistoryList.Visibility = Visibility.Visible;
        HistoryGrid.Visibility = Visibility.Collapsed;
    }

    private void DisplayModeGridClick(object sender, RoutedEventArgs args)
    {
        displayMode = "grid";
        DisplayModeGridButton.Background = DfUi.Control;
        DisplayModeListButton.Background = new SolidColorBrush(Colors.Transparent);
        HistoryList.Visibility = Visibility.Collapsed;
        HistoryGrid.Visibility = Visibility.Visible;
        RenderGrid(visibleEntries);
    }

    // MARK: - Library sidebar

    private void RebuildLibrarySidebar()
    {
        if (LibraryListPanel is null) return;
        LibraryListPanel.Children.Clear();
        bool online = usingCanonical && canonicalState is not null;
        LibraryListPanel.Children.Add(LibraryRow("全部", "\uE8A5", ClipboardLibrary.Selection.All,
            online ? CountFor("all") : clipboard.Entries.Count));
        LibraryListPanel.Children.Add(DfUi.SectionLabel("类型"));
        LibraryListPanel.Children.Add(LibraryRow("纯文本", "\uE8E4", ClipboardLibrary.Selection.OfKind("text"),
            online ? -1 : library.Count(ClipboardLibrary.Selection.OfKind("text"), clipboard.Entries)));
        LibraryListPanel.Children.Add(LibraryRow("图片", "\uE91B", ClipboardLibrary.Selection.OfKind("image"),
            online ? -1 : library.Count(ClipboardLibrary.Selection.OfKind("image"), clipboard.Entries)));
        LibraryListPanel.Children.Add(LibraryRow("文件", "\uE8B7", ClipboardLibrary.Selection.OfKind("files"),
            online ? -1 : library.Count(ClipboardLibrary.Selection.OfKind("files"), clipboard.Entries)));
        if (online)
        {
            var lists = canonicalState!.Lists.OrderBy(list => list.Position).ToArray();
            if (lists.Length > 0)
            {
                LibraryListPanel.Children.Add(DfUi.Divider(8, 8));
                LibraryListPanel.Children.Add(DfUi.SectionLabel("我的列表"));
                foreach (var list in lists)
                {
                    var row = LibraryRow(list.Name, list.IsSmart ? "\uE71C" : "\uE8FD",
                        ClipboardLibrary.Selection.OfList(list.Id), CountFor(list.Id));
                    row.ContextFlyout = LibraryListFlyout(list);
                    if (list.IsSmart) ToolTipService.SetToolTip(row, SmartListDescription(list));
                    LibraryListPanel.Children.Add(row);
                }
            }
            LibraryListPanel.Children.Add(DfUi.Divider(8, 8));
            LibraryListPanel.Children.Add(SidebarAction("\uE710", "新建列表", NewListClick));
            LibraryListPanel.Children.Add(SidebarAction("\uE710", "新建智能列表", NewSmartListClick));
            return;
        }
        if (library.Lists.Count > 0)
        {
            LibraryListPanel.Children.Add(DfUi.Divider(8, 8));
            LibraryListPanel.Children.Add(DfUi.SectionLabel("我的列表"));
            foreach (var list in library.Lists)
            {
                var row = LibraryRow(list.Name, "\uE8FD", ClipboardLibrary.Selection.OfList(list.Id), list.Members.Count);
                row.ContextFlyout = LibraryListFlyout(list);
                LibraryListPanel.Children.Add(row);
            }
        }
        LibraryListPanel.Children.Add(DfUi.Divider(8, 8));
        LibraryListPanel.Children.Add(SidebarAction("\uE710", "新建列表", NewListClick));
    }

    private int CountFor(string key) => canonicalCounts.TryGetValue(key, out long count) ? (int)count : -1;

    private static string SmartListDescription(CanonicalList list)
    {
        if (list.Rules.Count == 0) return $"{list.Name} · 没有筛选条件";
        var parts = list.Rules.Select(rule => rule.Field switch
        {
            "app" => $"应用{(rule.Op == "is_not" ? "不是" : "是")} {rule.Value}",
            "device" => $"设备{(rule.Op == "is_not" ? "不是" : "是")} {rule.Value}",
            "kind" => $"类型{(rule.Op == "is_not" ? "不是" : "是")} {rule.Value}",
            "text" => rule.Op == "not_contains" ? $"不包含「{rule.Value}」" : $"包含「{rule.Value}」",
            "date" => rule.Op switch
            {
                "today" => "今天",
                "yesterday" => "昨天",
                "last_days" => $"最近 {rule.Value} 天",
                _ => "日期范围",
            },
            _ => rule.Field,
        });
        return $"{list.Name} · {string.Join(list.MatchMode == "any" ? " 或 " : " 且 ", parts)}";
    }

    private Button SidebarAction(string glyph, string label, RoutedEventHandler click)
    {
        var button = new Button
        {
            Background = new SolidColorBrush(Colors.Transparent),
            BorderThickness = new Thickness(0),
            CornerRadius = new CornerRadius(8),
            Padding = new Thickness(10, 0, 10, 0),
            MinHeight = 32,
            HorizontalAlignment = HorizontalAlignment.Stretch,
            HorizontalContentAlignment = HorizontalAlignment.Left,
            Template = (ControlTemplate)Application.Current.Resources["DFFlatButtonTemplate"],
        };
        var content = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        content.Children.Add(new FontIcon { Glyph = glyph, FontSize = 12, Foreground = DfUi.Accent, VerticalAlignment = VerticalAlignment.Center });
        content.Children.Add(DfUi.Text(label, 13, false, DfUi.Accent));
        button.Content = content;
        button.Click += click;
        return button;
    }

    private Button LibraryRow(string name, string glyph, ClipboardLibrary.Selection selection, int count)
    {
        bool selected = librarySelection.Equals(selection);
        var button = new Button
        {
            Background = selected ? DfUi.Surface : new SolidColorBrush(Colors.Transparent),
            BorderBrush = selected ? DfUi.Line : new SolidColorBrush(Colors.Transparent),
            BorderThickness = new Thickness(1),
            CornerRadius = new CornerRadius(8),
            Padding = new Thickness(10, 0, 10, 0),
            MinHeight = 32,
            MinWidth = 0,
            HorizontalAlignment = HorizontalAlignment.Stretch,
            HorizontalContentAlignment = HorizontalAlignment.Stretch,
            VerticalContentAlignment = VerticalAlignment.Center,
            Template = (ControlTemplate)Application.Current.Resources["DFFlatButtonTemplate"],
        };
        var grid = new Grid();
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(18) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        grid.Children.Add(new FontIcon
        {
            Glyph = glyph,
            FontSize = 13,
            Foreground = selected ? DfUi.PrimaryText : DfUi.Muted,
            VerticalAlignment = VerticalAlignment.Center,
        });
        var title = DfUi.Text(name, 13, false, selected ? DfUi.PrimaryText : DfUi.Muted);
        title.Margin = new Thickness(9, 0, 0, 0);
        title.VerticalAlignment = VerticalAlignment.Center;
        title.TextTrimming = TextTrimming.CharacterEllipsis;
        Grid.SetColumn(title, 1);
        grid.Children.Add(title);
        if (count >= 0)
        {
            var counter = DfUi.Text(count.ToString(), 11, false, selected ? DfUi.Muted : DfUi.Muted);
            counter.VerticalAlignment = VerticalAlignment.Center;
            Grid.SetColumn(counter, 2);
            grid.Children.Add(counter);
        }
        button.Content = grid;
        button.Click += (_, _) =>
        {
            librarySelection = selection;
            RebuildLibrarySidebar();
            RefreshHistory();
        };
        return button;
    }

    private MenuFlyout LibraryListFlyout(CanonicalList list)
    {
        var flyout = new MenuFlyout();
        if (list.IsSmart)
        {
            var edit = new MenuFlyoutItem { Text = "编辑条件…" };
            edit.Click += async (_, _) => await EditSmartListAsync(list);
            flyout.Items.Add(edit);
            flyout.Items.Add(new MenuFlyoutSeparator());
        }
        var rename = new MenuFlyoutItem { Text = "重命名…" };
        rename.Click += async (_, _) => await RenameListAsync(list);
        flyout.Items.Add(rename);
        var moveUp = new MenuFlyoutItem { Text = "向前移动" };
        moveUp.Click += async (_, _) => await MoveListAsync(list, -1);
        flyout.Items.Add(moveUp);
        var moveDown = new MenuFlyoutItem { Text = "向后移动" };
        moveDown.Click += async (_, _) => await MoveListAsync(list, 1);
        flyout.Items.Add(moveDown);
        flyout.Items.Add(new MenuFlyoutSeparator());
        var delete = new MenuFlyoutItem { Text = "删除列表" };
        delete.Click += async (_, _) =>
        {
            try
            {
                if (librarySelection.ListId == list.Id) librarySelection = ClipboardLibrary.Selection.All;
                await clipboard.DeleteCanonicalListAsync(list.Id);
                await ReloadCanonicalStateAsync();
            }
            catch (Exception ex) when (ex is IOException or InvalidOperationException) { Notify(ex.Message); }
        };
        flyout.Items.Add(delete);
        return flyout;
    }

    private MenuFlyout LibraryListFlyout(ClipboardLibrary.ManualList list)
    {
        var flyout = new MenuFlyout();
        var rename = new MenuFlyoutItem { Text = "重命名…" };
        rename.Click += async (_, _) => await RenameListAsync(list);
        flyout.Items.Add(rename);
        var moveUp = new MenuFlyoutItem { Text = "向前移动" };
        moveUp.Click += (_, _) => { library.Move(list.Id, -1); };
        flyout.Items.Add(moveUp);
        var moveDown = new MenuFlyoutItem { Text = "向后移动" };
        moveDown.Click += (_, _) => { library.Move(list.Id, 1); };
        flyout.Items.Add(moveDown);
        flyout.Items.Add(new MenuFlyoutSeparator());
        var delete = new MenuFlyoutItem { Text = "删除列表" };
        delete.Click += (_, _) =>
        {
            if (librarySelection.ListId == list.Id) librarySelection = ClipboardLibrary.Selection.All;
            library.Delete(list.Id);
            RebuildLibrarySidebar();
            RefreshHistory();
        };
        flyout.Items.Add(delete);
        return flyout;
    }

    private async void NewListClick(object sender, RoutedEventArgs args)
    {
        var editor = new TextBox { PlaceholderText = "列表名称", MaxLength = 60 };
        var dialog = new ContentDialog
        {
            XamlRoot = Root.XamlRoot,
            Title = "新建列表",
            Content = editor,
            PrimaryButtonText = "创建",
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Primary,
        };
        if (await dialog.ShowAsync() != ContentDialogResult.Primary) return;
        try
        {
            if (usingCanonical && canonicalState is not null)
            {
                var list = new CanonicalList
                {
                    Id = Guid.NewGuid().ToString("N"),
                    Name = editor.Text.Trim(),
                    Kind = "manual",
                    Position = canonicalState.Lists.Count,
                };
                if (list.Name.Length == 0) throw new InvalidOperationException("列表名称不能为空");
                await clipboard.SaveCanonicalListAsync(list);
                librarySelection = ClipboardLibrary.Selection.OfList(list.Id);
                await ReloadCanonicalStateAsync();
                return;
            }
            var created = library.Create(editor.Text);
            librarySelection = ClipboardLibrary.Selection.OfList(created.Id);
            RebuildLibrarySidebar();
            RefreshHistory();
        }
        catch (InvalidOperationException ex) { Notify(ex.Message); }
        catch (Exception ex) when (ex is IOException) { Notify(ex.Message); }
    }

    private async void NewSmartListClick(object sender, RoutedEventArgs args)
    {
        var draft = await ShowSmartListDialogAsync(null);
        if (draft is null) return;
        try
        {
            await clipboard.SaveCanonicalListAsync(draft);
            librarySelection = ClipboardLibrary.Selection.OfList(draft.Id);
            await ReloadCanonicalStateAsync();
        }
        catch (Exception ex) when (ex is IOException or InvalidOperationException) { Notify(ex.Message); }
    }

    private async Task<CanonicalList?> ShowSmartListDialogAsync(CanonicalList? existing)
    {
        var panel = new StackPanel { Spacing = 10, MinWidth = 460 };
        var name = new TextBox { Text = existing?.Name ?? "", PlaceholderText = "列表名称", MaxLength = 60 };
        panel.Children.Add(name);
        var mode = new ComboBox { HorizontalAlignment = HorizontalAlignment.Stretch, SelectedIndex = existing?.MatchMode == "any" ? 1 : 0 };
        mode.Items.Add("满足全部条件");
        mode.Items.Add("满足任一条件");
        panel.Children.Add(mode);
        var rowsHost = new StackPanel { Spacing = 10 };
        panel.Children.Add(new ScrollViewer
        {
            Content = rowsHost,
            MaxHeight = 340,
            VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
            HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
        });
        var rows = new List<SmartRuleRow>();
        void AddRow(CanonicalRule? rule)
        {
            var row = new SmartRuleRow();
            row.Build(rule, () =>
            {
                if (rows.Count <= 1) return;
                rowsHost.Children.Remove(row.Host);
                rows.Remove(row);
            });
            rows.Add(row);
            rowsHost.Children.Add(row.Host);
        }
        foreach (var rule in existing?.Rules ?? []) AddRow(rule);
        if (rows.Count == 0) AddRow(null);
        panel.Children.Add(DfUi.Secondary("增加条件", (_, _) => AddRow(null)));
        var dialog = new ContentDialog
        {
            XamlRoot = Root.XamlRoot,
            Title = existing is null ? "新建智能列表" : "编辑智能列表",
            Content = panel,
            PrimaryButtonText = "保存",
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Primary,
        };
        if (await dialog.ShowAsync() != ContentDialogResult.Primary) return null;
        string listName = name.Text.Trim();
        if (listName.Length == 0) { Notify("列表名称不能为空"); return null; }
        var rules = new List<CanonicalRule>();
        foreach (var row in rows)
        {
            if (!row.TryBuild(out var rule, out string? error))
            {
                Notify(error ?? "智能条件无效");
                return null;
            }
            rules.Add(rule);
        }
        if (rules.Count == 0) { Notify("请至少添加一个条件"); return null; }
        return new CanonicalList
        {
            Id = existing?.Id ?? Guid.NewGuid().ToString("N"),
            Name = listName,
            Kind = "smart",
            MatchMode = mode.SelectedIndex == 1 ? "any" : "all",
            Rules = rules,
            Position = existing?.Position ?? (canonicalState?.Lists.Count ?? 0),
        };
    }

    /// <summary>One editable smart-list condition: field, operator and value controls.</summary>
    private sealed class SmartRuleRow
    {
        private static readonly string[] Fields = ["text", "app", "device", "kind", "date"];
        private static readonly string[] FieldLabels = ["内容包含文字", "来自应用", "来自设备", "类型为", "创建日期"];
        private static readonly string[][] Operators =
        [
            ["contains", "not_contains"],
            ["is", "is_not"],
            ["is", "is_not"],
            ["is", "is_not"],
            ["today", "yesterday", "last_days", "between"],
        ];
        private static readonly string[][] OperatorLabels =
        [
            ["包含", "不包含"],
            ["是", "不是"],
            ["是", "不是"],
            ["是", "不是"],
            ["今天", "昨天", "最近天数", "日期范围"],
        ];

        private readonly ComboBox field = new() { MinWidth = 148, HorizontalAlignment = HorizontalAlignment.Stretch };
        private readonly ComboBox op = new() { MinWidth = 112 };
        private readonly TextBox value = new() { PlaceholderText = "值" };
        private readonly TextBox value2 = new() { PlaceholderText = "结束日期 (yyyy-MM-dd)" };
        private readonly ComboBox kind = new() { HorizontalAlignment = HorizontalAlignment.Stretch };
        private bool ready;

        public StackPanel Host { get; } = new() { Spacing = 6 };

        public void Build(CanonicalRule? initial, Action remove)
        {
            foreach (var label in FieldLabels) field.Items.Add(label);
            kind.Items.Add("文字");
            kind.Items.Add("图片");
            kind.Items.Add("文件");
            kind.Items.Add("其他格式");
            var header = new Grid { ColumnSpacing = 8 };
            header.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            header.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            header.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            header.Children.Add(field);
            Grid.SetColumn(op, 1);
            header.Children.Add(op);
            var delete = DfUi.IconButton("\uE711", (_, _) => remove(), "删除条件");
            delete.HorizontalAlignment = HorizontalAlignment.Right;
            Grid.SetColumn(delete, 2);
            header.Children.Add(delete);
            Host.Children.Add(header);
            var values = new Grid { ColumnSpacing = 8 };
            values.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            values.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            values.Children.Add(value);
            Grid.SetColumn(value2, 1);
            values.Children.Add(value2);
            Host.Children.Add(values);
            Host.Children.Add(kind);

            field.SelectionChanged += (_, _) => { if (ready) PopulateOperators(); };
            op.SelectionChanged += (_, _) => { if (ready) RefreshValueControls(); };

            int fieldIndex = initial is null ? 0 : Math.Max(0, Array.IndexOf(Fields, initial.Field));
            field.SelectedIndex = fieldIndex;
            PopulateOperators(initial?.Op);
            value.Text = initial?.Value ?? "";
            value2.Text = initial?.Value2 ?? "";
            kind.SelectedIndex = initial?.Value switch { "image" => 1, "files" => 2, "other" => 3, _ => 0 };
            ready = true;
            RefreshValueControls();
        }

        private void PopulateOperators(string? selected = null)
        {
            int index = Math.Max(0, field.SelectedIndex);
            op.Items.Clear();
            foreach (var label in OperatorLabels[index]) op.Items.Add(label);
            int wanted = selected is null ? 0 : Array.IndexOf(Operators[index], selected);
            op.SelectedIndex = wanted < 0 ? 0 : wanted;
            RefreshValueControls();
        }

        private void RefreshValueControls()
        {
            string name = Fields[Math.Max(0, field.SelectedIndex)];
            string currentOp = CurrentOp;
            bool dateRange = name == "date";
            kind.Visibility = name == "kind" ? Visibility.Visible : Visibility.Collapsed;
            value.Visibility = name == "kind" || (dateRange && currentOp is "today" or "yesterday")
                ? Visibility.Collapsed
                : Visibility.Visible;
            value2.Visibility = dateRange && currentOp == "between" ? Visibility.Visible : Visibility.Collapsed;
            value.PlaceholderText = name switch
            {
                "app" => "应用标识，例如 chrome.exe",
                "device" => "来源设备标识或名称",
                "date" => "天数，例如 7",
                _ => "要包含的文字",
            };
            if (dateRange && currentOp == "between")
            {
                value.PlaceholderText = "开始日期 (yyyy-MM-dd)";
                value2.PlaceholderText = "结束日期 (yyyy-MM-dd)";
            }
        }

        private string CurrentOp => Operators[Math.Max(0, field.SelectedIndex)][Math.Max(0, op.SelectedIndex)];

        public bool TryBuild(out CanonicalRule rule, out string? error)
        {
            rule = new CanonicalRule();
            error = null;
            string name = Fields[Math.Max(0, field.SelectedIndex)];
            string currentOp = CurrentOp;
            string text = value.Text.Trim();
            switch (name)
            {
                case "kind":
                    if (kind.SelectedIndex < 0) { error = "请选择内容类型"; return false; }
                    rule = new CanonicalRule { Field = "kind", Op = currentOp, Value = kind.SelectedIndex switch { 1 => "image", 2 => "files", 3 => "other", _ => "text" } };
                    return true;
                case "date" when currentOp is "today" or "yesterday":
                    rule = new CanonicalRule { Field = "date", Op = currentOp, Value = "" };
                    return true;
                case "date" when currentOp == "last_days":
                    if (!int.TryParse(text, out int days) || days < 1) { error = "最近天数必须大于零"; return false; }
                    rule = new CanonicalRule { Field = "date", Op = currentOp, Value = days.ToString() };
                    return true;
                case "date":
                    if (!DateTime.TryParse(text, out var start) || !DateTime.TryParse(value2.Text.Trim(), out var end) || end < start)
                    {
                        error = "日期范围无效，请使用 yyyy-MM-dd";
                        return false;
                    }
                    rule = new CanonicalRule
                    {
                        Field = "date", Op = "between",
                        Value = start.ToString("yyyy-MM-dd"), Value2 = end.ToString("yyyy-MM-dd"),
                    };
                    return true;
                default:
                    if (text.Length == 0) { error = "条件内容不能为空"; return false; }
                    rule = new CanonicalRule { Field = name, Op = currentOp, Value = text };
                    return true;
            }
        }
    }

    private async Task EditSmartListAsync(CanonicalList list)
    {
        var draft = await ShowSmartListDialogAsync(list);
        if (draft is null) return;
        try
        {
            await clipboard.SaveCanonicalListAsync(draft);
            await ReloadCanonicalStateAsync();
        }
        catch (Exception ex) when (ex is IOException or InvalidOperationException) { Notify(ex.Message); }
    }

    private async Task MoveListAsync(CanonicalList list, int delta)
    {
        if (canonicalState is null) return;
        var ids = canonicalState.Lists.OrderBy(item => item.Position).Select(item => item.Id).ToList();
        int index = ids.IndexOf(list.Id);
        int target = index + delta;
        if (index < 0 || target < 0 || target >= ids.Count) return;
        (ids[index], ids[target]) = (ids[target], ids[index]);
        try
        {
            await clipboard.ReorderCanonicalListsAsync(ids);
            await ReloadCanonicalStateAsync();
        }
        catch (Exception ex) when (ex is IOException or InvalidOperationException) { Notify(ex.Message); }
    }

    private async Task RenameListAsync(CanonicalList list)
    {
        var editor = new TextBox { Text = list.Name, MaxLength = 60 };
        var dialog = new ContentDialog
        {
            XamlRoot = Root.XamlRoot,
            Title = "重命名列表",
            Content = editor,
            PrimaryButtonText = "保存",
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Primary,
        };
        if (await dialog.ShowAsync() != ContentDialogResult.Primary) return;
        string name = editor.Text.Trim();
        if (name.Length == 0) { Notify("列表名称不能为空"); return; }
        try
        {
            list.Name = name;
            await clipboard.SaveCanonicalListAsync(list);
            await ReloadCanonicalStateAsync();
        }
        catch (Exception ex) when (ex is IOException or InvalidOperationException) { Notify(ex.Message); }
    }

    private async Task ReloadCanonicalStateAsync()
    {
        try
        {
            canonicalState = await clipboard.CanonicalStateAsync();
            usingCanonical = true;
            RebuildLibrarySidebar();
            RefreshHistory();
        }
        catch (Exception ex) when (ex is IOException or InvalidOperationException or OperationCanceledException)
        {
            usingCanonical = false;
            RebuildLibrarySidebar();
            Notify(ex.Message);
        }
    }

    private async Task RenameListAsync(ClipboardLibrary.ManualList list)
    {
        var editor = new TextBox { Text = list.Name, MaxLength = 60 };
        var dialog = new ContentDialog
        {
            XamlRoot = Root.XamlRoot,
            Title = "重命名列表",
            Content = editor,
            PrimaryButtonText = "保存",
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Primary,
        };
        if (await dialog.ShowAsync() != ContentDialogResult.Primary) return;
        try { library.Rename(list.Id, editor.Text); }
        catch (InvalidOperationException ex) { Notify(ex.Message); }
    }

    private void RefreshHistory()
    {
        if (HistoryList is null || updatingHistory) return;
        ++historyVersion;
        if (clipboard.CanonicalConnected)
        {
            historyRefreshPending = true;
            if (!historyRefreshRunning) _ = DrainHistoryRefreshAsync();
            return;
        }
        if (usingCanonical)
        {
            usingCanonical = false;
            canonicalState = null;
            canonicalCounts.Clear();
            canonicalOffset = 0;
            canonicalHasMore = false;
            RebuildLibrarySidebar();
        }
        updatingHistory = true;
        try
        {
            var visible = library.Filter(librarySelection, clipboard.Entries)
                .Where(MatchesLocalFilter)
                .ToArray();
            RenderVisibleEntries(visible, visible.Length, clipboard.Entries.Count);
        }
        finally { updatingHistory = false; }
        RefreshPreview();
    }

    private bool MatchesLocalFilter(ClipboardEntry entry) =>
        (selectedSource == "所有设备" || entry.SourceName == selectedSource || entry.SourceId == selectedSource)
        && (HistorySearch.Text.Trim().Length == 0
            || $"{entry.Title} {entry.Preview} {entry.SourceName}".Contains(HistorySearch.Text.Trim(), StringComparison.CurrentCultureIgnoreCase));

    private IReadOnlyList<CanonicalRule>? SelectedRules() =>
        canonicalState?.Lists.FirstOrDefault(list => list.Id == librarySelection.ListId)?.Rules;

    private async Task DrainHistoryRefreshAsync()
    {
        historyRefreshRunning = true;
        try
        {
            while (historyRefreshPending && clipboard.CanonicalConnected)
            {
                historyRefreshPending = false;
                await RefreshCanonicalHistoryAsync();
            }
        }
        finally { historyRefreshRunning = false; }
    }

    private async Task RefreshCanonicalHistoryAsync()
    {
        int version = historyVersion;
        string query = HistorySearch.Text.Trim();
        string? listId = librarySelection.ListId;
        string kind = librarySelection.Kind ?? "all";
        string? source = selectedSource == "所有设备" ? null : selectedSource;
        try
        {
            var state = await clipboard.CanonicalStateAsync();
            if (version != historyVersion) return;
            usingCanonical = true;
            canonicalState = state;
            var page = await clipboard.QueryCanonicalAsync(query, listId, kind, source, 0, 100, SelectedRules());
            if (version != historyVersion) return;
            canonicalRows = page.Items.Select(CanonicalRow.ToEntry).ToArray();
            canonicalOffset = (int)(page.NextOffset ?? canonicalRows.Length);
            canonicalHasMore = page.NextOffset is not null;
            updatingHistory = true;
            try { RenderVisibleEntries(canonicalRows, page.Total, (int)page.Total); }
            finally { updatingHistory = false; }
            RebuildLibrarySidebar();
            RefreshPreview();
            _ = LoadCanonicalThumbnailsAsync(version);
            _ = LoadCanonicalCountsAsync(version, state.Lists);
        }
        catch (Exception ex) when (ex is IOException or InvalidOperationException or OperationCanceledException or TimeoutException)
        {
            if (version != historyVersion) return;
            lastLibraryRevision = -1;
            if (usingCanonical && canonicalRows.Length > 0)
            {
                SyncStatus.Text = "内容库刷新暂未完成；保留当前内容并稍后重试。";
                return;
            }
            usingCanonical = false;
            canonicalState = null;
            canonicalCounts.Clear();
            canonicalOffset = 0;
            canonicalHasMore = false;
            HistoryEmptyTitle.Text = "内容库暂时不可用";
            HistoryEmptyDetail.Text = $"{ex.Message}\n本机历史仍可继续使用。";
            RebuildLibrarySidebar();
            updatingHistory = true;
            try
            {
                var visible = library.Filter(librarySelection, clipboard.Entries).Where(MatchesLocalFilter).ToArray();
                RenderVisibleEntries(visible, visible.Length, clipboard.Entries.Count);
                if (visible.Length == 0) HistoryEmptyTitle.Text = "内容库暂时不可用";
            }
            finally { updatingHistory = false; }
            RefreshPreview();
        }
    }

    private async Task LoadCanonicalCountsAsync(int version, IReadOnlyList<CanonicalList> lists)
    {
        try
        {
            var counts = await clipboard.CanonicalCountsAsync(lists);
            if (version != historyVersion) return;
            canonicalCounts = counts;
            RebuildLibrarySidebar();
        }
        catch (Exception ex) when (ex is IOException or InvalidOperationException or OperationCanceledException) { }
    }

    private Task LoadCanonicalThumbnailsAsync(int version) => Task.CompletedTask;

    private void RenderVisibleEntries(ClipboardEntry[] visible, long total, int emptyTotal)
    {
        string filter = $"{librarySelection}|{selectedSource}|{HistorySearch.Text.Trim()}";
        string presentation = JsonSerializer.Serialize(visible.Select(HistoryRowSignature));
        HistoryCount.Text = $"{total} 条";
        if (lastHistoryPresentation == presentation && lastHistoryFilter == filter) return;
        var viewer = FindDescendant<ScrollViewer>(HistoryList);
        double? scroll = lastHistoryFilter == filter ? viewer?.VerticalOffset : null;
        lastHistoryPresentation = presentation;
        lastHistoryFilter = filter;
        var selectedIds = HistoryList.SelectedItems.OfType<ClipboardEntry>().Select(entry => entry.Id).ToHashSet();
        var groups = new List<HistoryGroup>();
        foreach (var entry in visible)
        {
            string key = entry.CopyTimeKnown ? HistoryGroupKey(entry.CreatedAt.LocalDateTime) : "复制时间未知";
            var group = groups.FirstOrDefault(value => value.Key == key);
            if (group is null) { group = new HistoryGroup(key); groups.Add(group); }
            group.Add(entry);
        }
        StableCollection.Apply(historyGroups, groups, group => group.Key, (_, _) => true);
        for (int i = 0; i < groups.Count; i++)
            StableCollection.Apply(historyGroups[i], groups[i], entry => entry.Id,
                (before, after) => HistoryRowSignature(before) == HistoryRowSignature(after));
        visibleEntries = historyGroups.SelectMany(group => group).ToArray();
        if (historyView is null)
        {
            historyView = new CollectionViewSource { IsSourceGrouped = true, Source = historyGroups };
            HistoryList.ItemsSource = historyView.View;
        }
        foreach (var entry in visibleEntries.Where(entry => selectedIds.Contains(entry.Id)))
            if (!HistoryList.SelectedItems.Contains(entry)) HistoryList.SelectedItems.Add(entry);
        if (HistoryList.SelectedItems.Count == 0 && visibleEntries.Length > 0)
            HistoryList.SelectedItem = visibleEntries[0];
        if (scroll is double y && viewer is not null)
            DispatcherQueue.TryEnqueue(() => viewer.ChangeView(null, y, null, true));
        HistoryEmpty.Visibility = visible.Length == 0 ? Visibility.Visible : Visibility.Collapsed;
        HistoryEmptyTitle.Text = emptyTotal == 0 ? "还没有剪贴板历史" : "没有匹配的内容";
        HistoryEmptyDetail.Text = emptyTotal == 0
            ? "复制文字、图片或文件后\n会显示在这里。"
            : "试试其他关键词或筛选条件。";
        HistoryCount.Text = $"{total} 条";
        if (displayMode == "grid") RenderGrid(visible);
    }

    private static string HistoryRowSignature(ClipboardEntry entry) => JsonSerializer.Serialize(new
    {
        entry.Id, entry.Title, entry.Preview, entry.Kind, entry.SourceId, entry.SourceName, entry.SourceApp,
        entry.CreatedLabel, entry.CopyTimeKnown, entry.Available, entry.Remote, entry.OcrText, entry.SizeBytes,
        Memberships = entry.MembershipIds.Order(StringComparer.Ordinal).ToArray(),
    });

    private void HistoryThumbnailLoaded(object sender, RoutedEventArgs args)
    {
        if (sender is ClipboardThumbnail thumbnail)
        {
            thumbnail.Cache = thumbnailCache;
            thumbnail.Refresh();
        }
    }

    private readonly System.Collections.ObjectModel.ObservableCollection<ClipboardEntry> cardRows = [];
    private readonly HashSet<Border> realizedCards = [];
    private void RenderGrid(ClipboardEntry[] items)
    {
        if (HistoryCards.ItemsSource is null) HistoryCards.ItemsSource = cardRows;
        StableCollection.Apply(cardRows, items, entry => entry.Id,
            (a, b) => HistoryRowSignature(a) == HistoryRowSignature(b));
        RefreshGridSelection();
    }
    private void HistoryContainerChanging(ListViewBase sender, ContainerContentChangingEventArgs args) { }
    private void HistoryGridViewChanged(object? sender, ScrollViewerViewChangedEventArgs args)
    {
        if (displayMode == "grid" && HistoryGrid.ScrollableHeight - HistoryGrid.VerticalOffset < 240)
            _ = LoadMoreCanonicalAsync();
    }
    private void HistoryCardPrepared(ItemsRepeater sender, ItemsRepeaterElementPreparedEventArgs args)
    {
        if (args.Element is Border border) { realizedCards.Add(border); StyleHistoryCard(border); }
    }
    private void HistoryCardClearing(ItemsRepeater sender, ItemsRepeaterElementClearingEventArgs args)
    {
        if (args.Element is Border border) realizedCards.Remove(border);
    }
    private void HistoryCardLoaded(object sender, RoutedEventArgs args)
    {
        if (sender is Border border) StyleHistoryCard(border);
    }
    private void StyleHistoryCard(Border border)
    {
        if (border.DataContext is not ClipboardEntry entry) return;
        bool selected = (HistoryList.SelectedItem as ClipboardEntry)?.Id == entry.Id;
        border.Background = selected ? DfUi.Surface : DfUi.Canvas;
        border.BorderBrush = selected ? DfUi.Accent : DfUi.Line;
        if (FindDescendant<ClipboardThumbnail>(border) is { } thumbnail)
            thumbnail.Visibility = entry.Kind is "image" or "files" ? Visibility.Visible : Visibility.Collapsed;
    }
    private void HistoryCardTapped(object sender, TappedRoutedEventArgs args)
    {
        if (sender is Border { DataContext: ClipboardEntry entry }) HistoryList.SelectedItem = entry;
        RefreshGridSelection();
    }
    private void RefreshGridSelection()
    {
        foreach (var border in realizedCards) StyleHistoryCard(border);
    }

    private static string HistoryGroupKey(DateTime value)
    {
        var today = DateTime.Today;
        if (value.Date == today) return "今天";
        if (value.Date == today.AddDays(-1)) return "昨天";
        return value.ToString("M月d日");
    }

    private sealed class HistoryGroup(string key) : System.Collections.ObjectModel.ObservableCollection<ClipboardEntry>
    {
        public string Key { get; } = key;
    }

    private void HistorySelectionChanged(object sender, SelectionChangedEventArgs args)
    {
        int selected = HistoryList.SelectedItems?.Count ?? 0;
        BulkDeleteButton.Visibility = selected > 1 ? Visibility.Visible : Visibility.Collapsed;
        BulkDeleteButton.Content = $"删除所选（{selected}）";
        if (!updatingHistory) RefreshPreview();
    }

    private async void DeleteSelectedClick(object sender, RoutedEventArgs args)
    {
        var selected = HistoryList.SelectedItems?.OfType<ClipboardEntry>().ToArray() ?? [];
        if (selected.Length == 0) return;
        if (usingCanonical && selected.Any(entry => entry.IsCanonical))
        {
            try
            {
                await clipboard.DeleteCanonicalAsync(selected);
                Notify($"已删除 {selected.Length} 条内容库记录；系统剪贴板、用户源文件和已粘贴文件保持不变。");
                RefreshHistory();
            }
            catch (Exception ex) when (ex is IOException or InvalidOperationException) { Notify(ex.Message); }
            return;
        }
        foreach (var entry in selected) clipboard.Remove(entry);
        Notify($"已删除 {selected.Length} 条本机历史；系统剪贴板、用户源文件和已粘贴文件保持不变。");
    }

    private async void SaveImageClick(object sender, RoutedEventArgs args) => await SaveEntryAsync("image");
    private async void SaveTextClick(object sender, RoutedEventArgs args) => await SaveEntryAsync("text");

    private async Task SaveEntryAsync(string kind)
    {
        if (HistoryList.SelectedItem is not ClipboardEntry row || row.Kind != kind)
        {
            Notify(kind == "image" ? "请选择一条图片记录。" : "请选择一条文字记录。");
            return;
        }
        var entry = await ResolveSelectedAsync(row);
        if (entry is null) return;
        try
        {
            var picker = new global::Windows.Storage.Pickers.FileSavePicker();
            WinRT.Interop.InitializeWithWindow.Initialize(picker, Handle);
            if (kind == "image")
            {
                picker.SuggestedFileName = "剪贴板图片";
                picker.FileTypeChoices.Add("PNG 图片", new List<string> { ".png" });
            }
            else
            {
                picker.SuggestedFileName = "剪贴板文本";
                picker.FileTypeChoices.Add("文本文件", new List<string> { ".txt" });
            }
            var file = await picker.PickSaveFileAsync();
            if (file is null) return;
            byte[] data = kind == "image"
                ? entry.Content.Data
                : entry.Content.Representations?.FirstOrDefault(r => r.Mime == "text/plain")?.Data ?? entry.Content.Data;
            await File.WriteAllBytesAsync(file.Path, data);
            Notify($"已保存到 {file.Path}");
        }
        catch (Exception ex) { Notify(ex is IOException ? ex.Message : "保存未完成。"); }
    }

    /// <summary>Copies the referenced files or folders into a user-chosen folder.</summary>
    private async void SaveFilesClick(object sender, RoutedEventArgs args)
    {
        if (HistoryList.SelectedItem is not ClipboardEntry { Kind: "files" } row)
        {
            Notify("请选择一条文件记录。");
            return;
        }
        var entry = await ResolveSelectedAsync(row);
        if (entry?.Content.Files is not { Count: > 0 } files) return;
        if (files.Any(path => !File.Exists(path) && !Directory.Exists(path)))
        {
            Notify("部分源文件已移动或不可用，未开始保存。");
            return;
        }
        try
        {
            var picker = new global::Windows.Storage.Pickers.FolderPicker();
            WinRT.Interop.InitializeWithWindow.Initialize(picker, Handle);
            picker.FileTypeFilter.Add("*");
            var folder = await picker.PickSingleFolderAsync();
            if (folder is null) return;
            int saved = 0;
            foreach (var path in files)
            {
                string name = Path.GetFileName(path.TrimEnd(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar));
                CopyPath(path, UniquePath(Path.Combine(folder.Path, name.Length == 0 ? "未命名" : name)));
                saved++;
            }
            Notify($"已保存 {saved} 项到 {folder.Path}");
        }
        catch (Exception ex)
        {
            Notify(ex is IOException or UnauthorizedAccessException ? $"保存未完成：{ex.Message}" : "保存未完成。");
        }
    }

    private static string UniquePath(string path)
    {
        if (!File.Exists(path) && !Directory.Exists(path)) return path;
        string? directory = Path.GetDirectoryName(path);
        string stem = Path.GetFileNameWithoutExtension(path);
        string extension = Path.GetExtension(path);
        for (int index = 2; ; index++)
        {
            string candidate = Path.Combine(directory ?? "", $"{stem} {index}{extension}");
            if (!File.Exists(candidate) && !Directory.Exists(candidate)) return candidate;
        }
    }

    private static void CopyPath(string source, string destination)
    {
        if (Directory.Exists(source))
        {
            Directory.CreateDirectory(destination);
            foreach (var child in Directory.EnumerateFileSystemEntries(source))
                CopyPath(child, Path.Combine(destination, Path.GetFileName(child)));
            return;
        }
        Directory.CreateDirectory(Path.GetDirectoryName(destination) ?? ".");
        File.Copy(source, destination);
    }

    private async void OpenFileClick(object sender, RoutedEventArgs args)
    {
        if (HistoryList.SelectedItem is not ClipboardEntry { Kind: "files" } row) { Notify("请选择一条文件记录。"); return; }
        var entry = await ResolveSelectedAsync(row);
        if (entry is null) return;
        string? path = entry.Content.Files?.FirstOrDefault(p => File.Exists(p) || Directory.Exists(p));
        if (path is null) { Notify("源文件已移动或不可用。"); return; }
        try { Process.Start(new ProcessStartInfo(path) { UseShellExecute = true }); }
        catch (Exception) { Notify("无法打开该文件。"); }
    }

    private void HistoryDoubleClick(object sender, DoubleTappedRoutedEventArgs args) { /* Selection is preview only. */ }

    private void HistoryContextOpening(object sender, object args)
    {
        var entry = HistoryList.SelectedItem as ClipboardEntry;
        UpdateHistorySaveItems(entry);
        PopulateAddToList(AddToListMenu, entry);
        PopulateAddToList(AddToListContextMenu, entry);
        PopulateShareToDevice(entry);
        PopulateWorkflowHistory(WorkflowHistoryMenu, entry);
        PopulateWorkflowHistory(WorkflowHistoryMenuCompact, entry);
    }

    /// <summary>Only the save action matching the selected entry kind is offered.</summary>
    private void UpdateHistorySaveItems(ClipboardEntry? entry)
    {
        string? kind = entry?.Kind;
        SaveImageContextMenuItem.Visibility = kind == "image" ? Visibility.Visible : Visibility.Collapsed;
        SaveTextContextMenuItem.Visibility = kind == "text" ? Visibility.Visible : Visibility.Collapsed;
        SaveFilesContextMenuItem.Visibility = kind == "files" ? Visibility.Visible : Visibility.Collapsed;
        SaveImageMenuItem.Visibility = SaveImageContextMenuItem.Visibility;
        SaveTextMenuItem.Visibility = SaveTextContextMenuItem.Visibility;
        SaveFilesMenuItem.Visibility = SaveFilesContextMenuItem.Visibility;
        var files = kind == "files" ? Visibility.Visible : Visibility.Collapsed;
        OpenFileContextMenuItem.Visibility = files;
        RevealContextMenuItem.Visibility = files;
        OpenFileMenuItem.Visibility = files;
    }

    /// <summary>Entries can run any enabled workflow that accepts their kind; output is copy-only.</summary>
    private void PopulateWorkflowHistory(MenuFlyoutSubItem menu, ClipboardEntry? entry)
    {
        menu.Items.Clear();
        if (entry is null)
        {
            menu.IsEnabled = false;
            return;
        }
        var options = HistoryWorkflows(entry);
        menu.IsEnabled = options.Items.Count > 0;
        if (options.Items.Count == 0)
        {
            menu.Items.Add(new MenuFlyoutItem { Text = options.Hint, IsEnabled = false });
            return;
        }
        foreach (var (id, name) in options.Items)
        {
            var item = new MenuFlyoutItem { Text = name };
            item.Click += async (_, _) => await RunHistoryWorkflow(entry, id);
            menu.Items.Add(item);
        }
    }

    private (IReadOnlyList<(string Id, string Name)> Items, string Hint) HistoryWorkflows(ClipboardEntry entry)
    {
        if (!WorkflowsReady) return ([], "工作流执行尚未就绪");
        if (entry.Kind == "files") return ([], "Windows 工作流暂不支持文件内容");
        var items = workflows
            .Where(workflow => workflow.Enabled && WorkflowAccepts(workflow, entry.Kind))
            .Select(workflow => (workflow.Id, workflow.Name))
            .ToArray();
        return (items, items.Length == 0 ? "没有适合此内容的已启用工作流" : "");
    }

    private async Task RunHistoryWorkflow(ClipboardEntry entry, string workflowId)
    {
        var workflow = workflows.FirstOrDefault(item => item.Id == workflowId);
        if (workflow is null)
        {
            Notify("工作流已不存在；请刷新目录后重试。");
            return;
        }
        var resolved = await ResolveSelectedAsync(entry);
        if (resolved is not null) await ExecuteWorkflowAsync(workflow, resolved.Content);
    }

    private bool WorkflowsReady =>
        service.IsConnected && latest is JsonElement snapshot
        && snapshot.TryGetProperty("capabilities", out var caps)
        && caps.Bool("workflow_execution");

    private static bool WorkflowAccepts(WorkflowRecord workflow, string kind)
    {
        if (WorkflowCompatibilityIssue(workflow) is not null) return false;
        if (workflow.InputType == "Text" && kind != "text") return false;
        if (workflow.InputType == "Image" && kind != "image") return false;
        foreach (var step in workflow.Steps)
        {
            if (step.Kind is "TextReplace" or "TextTrim" && kind != "text") return false;
            if (step.Kind is "Compress" or "Resize" or "Convert" && kind != "image") return false;
        }
        return true;
    }

    private void PopulateShareToDevice(ClipboardEntry? entry)
    {
        ShareToDeviceMenu.Items.Clear();
        if (entry is not { CanonicalId: ulong id })
        {
            ShareToDeviceMenu.IsEnabled = false;
            ShareToDeviceMenu.Items.Add(new MenuFlyoutItem
            {
                Text = "仅内容库条目可直接发送",
                IsEnabled = false,
            });
            return;
        }
        ShareToDeviceMenu.IsEnabled = true;
        var peers = SharePeers();
        if (peers.Count == 0)
        {
            ShareToDeviceMenu.Items.Add(new MenuFlyoutItem { Text = "没有已连接的设备", IsEnabled = false });
            return;
        }
        var options = HistoryWorkflows(entry);
        foreach (var peer in peers)
        {
            var peerMenu = new MenuFlyoutSubItem { Text = peer.Name };
            var direct = new MenuFlyoutItem { Text = "直接发送" };
            direct.Click += async (_, _) => await ShareEntryToPeer(entry, peer.Id);
            peerMenu.Items.Add(direct);
            if (options.Items.Count > 0)
            {
                peerMenu.Items.Add(new MenuFlyoutSeparator());
                foreach (var (workflowId, name) in options.Items)
                {
                    var item = new MenuFlyoutItem { Text = $"使用「{name}」发送" };
                    item.Click += async (_, _) => await ShareEntryWithWorkflow(entry, workflowId, peer.Id);
                    peerMenu.Items.Add(item);
                }
            }
            ShareToDeviceMenu.Items.Add(peerMenu);
        }
    }

    private IReadOnlyList<(string Id, string Name)> SharePeers()
    {
        if (latest is not JsonElement snapshot) return [];
        return snapshot.Array("peers")
            .Where(peer => peer.String("trust") == "trusted" && peer.String("connection") == "connected")
            .Select(peer => (Id: peer.String("peer_id"), Name: peer.String("display_name", "未命名设备")))
            .Where(peer => peer.Id.Length > 0)
            .ToArray();
    }

    private async Task ShareEntryToPeer(ClipboardEntry entry, string peerId)
    {
        if (entry.CanonicalId is not ulong id)
        {
            Notify("仅内容库条目可直接发送。");
            return;
        }
        try
        {
            var reply = await service.SendAsync(ServiceClient.Command("ShareLibraryEntry",
                new { id, peer_id = peerId }));
            Notify(reply.TryGetProperty("Queued", out var queued)
                ? queued.String("message", "已发送；等待目标设备应用。")
                : "已发送；以目标设备的实际状态为准。");
        }
        catch (Exception ex) when (ex is IOException or InvalidOperationException) { Notify(ex.Message); }
    }

    /// <summary>Runs the workflow locally and sends only the result; the clipboard is untouched.</summary>
    private async Task ShareEntryWithWorkflow(ClipboardEntry entry, string workflowId, string peerId)
    {
        if (!WorkflowsReady || latest is not JsonElement snapshot)
        {
            Notify("工作流执行尚未就绪。");
            return;
        }
        var workflow = workflows.FirstOrDefault(item => item.Id == workflowId);
        if (workflow is null)
        {
            Notify("工作流已不存在；请刷新目录后重试。");
            return;
        }
        var resolved = await ResolveSelectedAsync(entry);
        if (resolved is null) return;
        try
        {
            var output = await workflowExecution.ExecuteToContentAsync(
                workflow.Id, resolved.Content, snapshot.Number("session_epoch"), DateTime.UtcNow.Ticks, CancellationToken.None);
            var formats = (output.Representations ?? [new ClipboardRepresentation(output.Mime, output.Data)])
                .Select(item => new { mime = item.Mime, data_base64 = Convert.ToBase64String(item.Data) })
                .ToArray();
            var reply = await service.SendAsync(ServiceClient.Command("ShareContent",
                new { peer_id = peerId, formats }));
            Notify(reply.TryGetProperty("Queued", out var queued)
                ? queued.String("message", "已发送处理结果；等待目标设备应用。")
                : "已发送；以目标设备的实际状态为准。");
        }
        catch (Exception ex) when (ex is IOException or InvalidOperationException) { Notify(ex.Message); }
    }

    private Task ShareEntryAsync(ClipboardEntry entry, string? workflowId, string peerId) =>
        workflowId is null ? ShareEntryToPeer(entry, peerId) : ShareEntryWithWorkflow(entry, workflowId, peerId);

    private void PopulateAddToList(MenuFlyoutSubItem menu, ClipboardEntry? entry)
    {
        menu.Items.Clear();
        menu.IsEnabled = entry is { Available: true };
        if (entry is null || !entry.Available) return;
        if (usingCanonical && canonicalState is not null)
        {
            var lists = canonicalState.Lists.Where(list => !list.IsSmart).ToArray();
            if (lists.Length == 0)
            {
                menu.Items.Add(new MenuFlyoutItem { Text = "请先新建手动列表", IsEnabled = false });
                return;
            }
            foreach (var list in lists)
            {
                bool present = entry.MembershipIds.Contains(list.Id);
                var item = new ToggleMenuFlyoutItem { Text = list.Name, IsChecked = present };
                item.Click += async (_, _) =>
                {
                    try
                    {
                        await clipboard.SetCanonicalMembershipAsync(list.Id, entry, !present);
                        RefreshHistory();
                    }
                    catch (Exception ex) when (ex is IOException or InvalidOperationException) { Notify(ex.Message); }
                };
                menu.Items.Add(item);
            }
            return;
        }
        if (library.Lists.Count == 0)
        {
            menu.Items.Add(new MenuFlyoutItem { Text = "请先新建手动列表", IsEnabled = false });
            return;
        }
        var memberships = library.Memberships(entry.Id);
        foreach (var list in library.Lists.ToArray())
        {
            bool present = memberships.Contains(list.Name);
            var item = new ToggleMenuFlyoutItem { Text = list.Name, IsChecked = present };
            item.Click += (_, _) =>
            {
                if (present) library.Remove(list.Id, [entry.Id]);
                else library.Add(list.Id, [entry.Id]);
                RefreshHistory();
            };
            menu.Items.Add(item);
        }
    }

    private void HistoryMenuClick(object sender, RoutedEventArgs args)
    {
        var flyout = new MenuFlyout();
        var delete = new MenuFlyoutItem { Text = "删除所选记录", IsEnabled = HistoryList.SelectedItem is ClipboardEntry };
        delete.Click += (_, _) => DeleteHistoryClick(sender, args);
        flyout.Items.Add(delete);
        var clear = new MenuFlyoutItem { Text = "清除搜索与筛选" };
        clear.Click += (_, _) =>
        {
            HistorySearch.Text = "";
            librarySelection = ClipboardLibrary.Selection.All;
            selectedSource = "所有设备";
            SourceFilterLabel.Text = "所有设备";
            RebuildLibrarySidebar();
            RefreshHistory();
        };
        flyout.Items.Add(clear);
        flyout.ShowAt((FrameworkElement)sender);
    }

    private void RefreshPreview()
    {
        var entry = HistoryList.SelectedItem as ClipboardEntry;
        if (previewInitialized && ReferenceEquals(previewEntry, entry)) return;
        previewInitialized = true;
        previewEntry = entry;
        CopyButton.IsEnabled = entry is not null;
        AddShelfButton.IsEnabled = entry is { Kind: "files", Available: true };
        MoreActionsButton.IsEnabled = entry is not null;
        ImagePreview.Source = null;
        ImagePreviewHost.Visibility = Visibility.Collapsed;
        TextPreview.Visibility = Visibility.Visible;
        RevealButton.Visibility = Visibility.Collapsed;
        FitButton.Visibility = Visibility.Collapsed;
        MetaOcrRow.Visibility = Visibility.Collapsed;
        if (entry is null)
        {
            PreviewTitle.Text = "";
            PreviewTitle.Visibility = Visibility.Collapsed;
            PreviewMetadata.Text = "在任意应用里复制，内容会在可信设备之间同步。";
            TextPreview.Visibility = Visibility.Collapsed;
            PreviewEmpty.Visibility = Visibility.Visible;
            MetadataCard.Visibility = Visibility.Collapsed;
            return;
        }
        PreviewEmpty.Visibility = Visibility.Collapsed;
        if (entry.IsCanonical && !entry.ContentLoaded)
        {
            ShowCanonicalPreviewPlaceholder(entry);
            if (entry.Available) _ = ResolvePreviewAsync(entry);
            return;
        }
        RenderPreview(entry);
    }

    private void ShowCanonicalPreviewPlaceholder(ClipboardEntry entry)
    {
        PreviewTitle.Text = entry.DetailTitle ?? "";
        PreviewTitle.Visibility = entry.DetailTitle is null ? Visibility.Collapsed : Visibility.Visible;
        PreviewMetadata.Text = entry.Kind == "files"
            ? $"来自 {entry.SourceName} · {entry.CanonicalFileCount} 项"
            : $"来自 {entry.SourceName} · {FormatBytes(entry.SizeBytes)}";
        PreviewMetadata.Text += entry.Remote ? " · 来自其他设备" : "";
        MetadataCard.Visibility = Visibility.Visible;
        MetaKind.Text = entry.Kind switch { "files" => $"{entry.CanonicalFileCount} 个文件", "image" => "图片", _ => "文字" };
        MetaSize.Text = entry.Kind == "files" ? $"{entry.CanonicalFileCount} 项" : FormatBytes(entry.SizeBytes);
        MetaSource.Text = entry.SourceName;
        MetaTime.Text = entry.CopyTimeKnown ? entry.CreatedAt.LocalDateTime.ToString("yyyy/M/d HH:mm") : "复制时间未知";
        if (!string.IsNullOrWhiteSpace(entry.CustomTitle)) PreviewTitle.Text = entry.CustomTitle;
        EditTitleMenuItem.IsEnabled = true;
        EditContentMenuItem.IsEnabled = entry.Kind == "text" && entry.Available;
        SaveImageMenuItem.IsEnabled = false;
        SaveTextMenuItem.IsEnabled = false;
        OpenFileMenuItem.IsEnabled = entry.Kind == "files" && entry.Available;
        if (!entry.Available)
        {
            TextPreview.Text = "内容正在从来源设备获取，请稍后重试。查询索引仍然可用，正文不会伪造。";
            CopyButton.IsEnabled = false;
            AddShelfButton.IsEnabled = false;
            return;
        }
        if (entry.Kind == "image")
        {
            TextPreview.Text = "正在读取图片…";
        }
        else
        {
            TextPreview.Text = entry.Preview.Length > 0 ? entry.Preview : "正在读取内容…";
        }
    }

    private async Task ResolvePreviewAsync(ClipboardEntry entry)
    {
        try
        {
            var loaded = await clipboard.ResolveCanonicalAsync(entry);
            if ((HistoryList.SelectedItem as ClipboardEntry)?.Id == entry.Id) RenderPreview(loaded);
        }
        catch (Exception ex) when (ex is IOException or InvalidOperationException or OperationCanceledException)
        {
            if ((HistoryList.SelectedItem as ClipboardEntry)?.Id != entry.Id) return;
            previewEntry = null;
            TextPreview.Text = ex.Message;
            CopyButton.IsEnabled = false;
            AddShelfButton.IsEnabled = false;
        }
    }

    private async void RenderPreview(ClipboardEntry entry)
    {
        TextPreview.Visibility = Visibility.Visible;
        PreviewTitle.Text = entry.DetailTitle ?? "";
        PreviewTitle.Visibility = entry.DetailTitle is null ? Visibility.Collapsed : Visibility.Visible;
        PreviewMetadata.Text = entry.Kind == "files"
            ? $"来自 {entry.SourceName} · {entry.Content.Files?.Count ?? entry.CanonicalFileCount} 项"
            : $"来自 {entry.SourceName} · {FormatBytes(entry.SizeBytes)}";
        PreviewMetadata.Text += entry.Remote ? " · 来自其他设备" : "";
        TextPreview.Text = entry.Preview;

        MetadataCard.Visibility = Visibility.Visible;
        MetaKind.Text = entry.Kind switch
        {
            "files" => $"{entry.Content.Files?.Count ?? entry.CanonicalFileCount} 个文件",
            "image" => entry.OcrText.Length > 0 ? "图片 · 已识别文字" : "图片",
            _ => "文字",
        };
        MetaSize.Text = entry.Kind == "files"
            ? $"{entry.Content.Files?.Count ?? entry.CanonicalFileCount} 项"
            : FormatBytes(entry.SizeBytes);
        MetaSource.Text = entry.SourceName;
        MetaTime.Text = entry.CopyTimeKnown ? entry.CreatedAt.LocalDateTime.ToString("yyyy/M/d HH:mm") : "复制时间未知";
        bool hasOcr = !string.IsNullOrWhiteSpace(entry.OcrText);
        MetaOcrRow.Visibility = hasOcr ? Visibility.Visible : Visibility.Collapsed;
        MetaOcr.Text = entry.OcrText;

        EditTitleMenuItem.IsEnabled = true;
        EditContentMenuItem.IsEnabled = entry.Kind == "text";
        SaveImageMenuItem.IsEnabled = entry.Kind == "image";
        SaveTextMenuItem.IsEnabled = entry.Kind == "text";
        OpenFileMenuItem.IsEnabled = entry.Kind == "files";

        if (entry.Kind == "image")
        {
            var image = await ClipboardImages.CreateAsync(entry.Content.Data, 1400);
            if ((HistoryList.SelectedItem as ClipboardEntry)?.Id != entry.Id) return;
            if (image is null) { TextPreview.Text = "图片无法预览，可能已损坏或超过解码限制。"; CopyButton.IsEnabled = false; }
            else
            {
                ImagePreview.Source = image;
                ImagePreviewHost.Visibility = Visibility.Visible;
                TextPreview.Visibility = Visibility.Collapsed;
                FitButton.Visibility = Visibility.Visible;
            }
        }
        if (entry.Kind == "files")
        {
            bool valid = entry.Content.Files is { Count: > 0 } files && files.All(p => File.Exists(p) || Directory.Exists(p));
            CopyButton.IsEnabled = valid;
            RevealButton.Visibility = valid ? Visibility.Visible : Visibility.Collapsed;
            PreviewMetadata.Text += valid ? " · 本地可用" : " · 源文件已移动或不可用";
        }
    }

    private async void EditHistoryContentClick(object sender, RoutedEventArgs args)
    {
        if (HistoryList.SelectedItem is not ClipboardEntry { Kind: "text" } row) return;
        var entry = await ResolveSelectedAsync(row);
        if (entry is null) return;
        if (!contentEditors.TryGetValue(entry.Id, out var editor))
        {
            editor = new ClipboardContentEditorWindow(entry, content =>
            {
                _ = SaveEditedContentAsync(entry, content);
            });
            editor.Closed += () => contentEditors.Remove(entry.Id);
            contentEditors.Add(entry.Id, editor);
        }
        editor.Show();
    }

    private async Task SaveEditedContentAsync(ClipboardEntry entry, ClipboardContent content)
    {
        try
        {
            if (entry.IsCanonical) await clipboard.SaveEditedContentAsync(entry, content);
            else clipboard.SaveEditedContent(content);
            Notify("编辑内容已复制；开启历史记录时将保存为新记录。");
            if (entry.IsCanonical) RefreshHistory();
        }
        catch (Exception ex) when (ex is IOException or InvalidOperationException)
        {
            Notify(ex.Message);
        }
    }

    private async void EditHistoryTitleClick(object sender, RoutedEventArgs args)
    {
        if (HistoryList.SelectedItem is not ClipboardEntry entry) return;
        var editor = new TextBox { Text = entry.CustomTitle ?? "", PlaceholderText = "留空恢复默认标题", AcceptsReturn = false };
        var dialog = new ContentDialog
        {
            XamlRoot = Root.XamlRoot, Title = "编辑标题", Content = editor,
            PrimaryButtonText = "保存", CloseButtonText = "取消", DefaultButton = ContentDialogButton.Primary,
        };
        dialog.Opened += (_, _) => { editor.Focus(FocusState.Programmatic); editor.SelectAll(); };
        if (await dialog.ShowAsync() != ContentDialogResult.Primary) return;
        try
        {
            if (entry.IsCanonical) await clipboard.SetCanonicalTitleAsync(entry, editor.Text);
            else clipboard.History.SetTitle(entry.Id, editor.Text);
            Notify("标题已更新。");
            if (entry.IsCanonical) RefreshHistory();
        }
        catch (Exception ex) when (ex is IOException or InvalidOperationException) { Notify(ex.Message); }
    }

    private async void DeleteHistoryClick(object sender, RoutedEventArgs args)
    {
        if (HistoryList.SelectedItem is not ClipboardEntry entry) return;
        if (entry.IsCanonical)
        {
            try
            {
                await clipboard.DeleteCanonicalAsync([entry]);
                Notify("已删除这条内容库记录；系统剪贴板、用户源文件和已粘贴文件保持不变。");
                RefreshHistory();
            }
            catch (Exception ex) when (ex is IOException or InvalidOperationException) { Notify(ex.Message); }
            return;
        }
        clipboard.Remove(entry);
        Notify("已删除这条本机历史；系统剪贴板、用户源文件和已粘贴文件保持不变。");
    }

    private async void CopyClick(object sender, RoutedEventArgs args) => await CopySelectionAsync();

    /// <summary>Tray "recent clipboard" entry point.</summary>
    public void RestoreEntry(ClipboardEntry entry)
    {
        if (entry.IsCanonical) { _ = RestoreEntryAsync(entry); return; }
        try { Notify(clipboard.SyncEnabled ? "已恢复本机剪贴板；跨设备结果取决于真实连接与服务回执。" : "已恢复本机系统剪贴板。"); clipboard.Restore(entry); }
        catch (Exception) { Notify("无法恢复剪贴板。请检查文件是否仍存在、当前会话是否可用或剪贴板是否被占用。"); }
    }

    private async Task RestoreEntryAsync(ClipboardEntry entry)
    {
        try
        {
            var resolved = entry.IsCanonical ? await ResolveSelectedAsync(entry) : entry;
            if (resolved is null) return;
            clipboard.Restore(resolved);
            Notify(clipboard.SyncEnabled ? "已恢复本机剪贴板；跨设备结果取决于真实连接与服务回执。" : "已恢复本机系统剪贴板。");
        }
        catch (Exception) { Notify("无法恢复剪贴板。请检查文件是否仍存在、当前会话是否可用或剪贴板是否被占用。"); }
    }

    private async Task CopySelectionAsync()
    {
        if (HistoryList.SelectedItem is ClipboardEntry entry) await RestoreEntryAsync(entry);
    }

    private async Task<ClipboardEntry?> ResolveSelectedAsync(ClipboardEntry entry)
    {
        if (!entry.IsCanonical) return entry;
        try { return await clipboard.ResolveCanonicalAsync(entry); }
        catch (Exception ex) when (ex is IOException or InvalidOperationException or OperationCanceledException)
        {
            Notify(ex is InvalidOperationException ? ex.Message : "内容暂时不可用，请稍后重试。");
            return null;
        }
    }

    private async void AddShelfClick(object sender, RoutedEventArgs args)
    {
        if (HistoryList.SelectedItem is not ClipboardEntry entry) return;
        var resolved = await ResolveSelectedAsync(entry);
        if (resolved is not null) clipboard.ShowShelf(resolved);
    }

    private async void SendClipboardFilesClick(object sender, RoutedEventArgs args)
    {
        if (HistoryList.SelectedItem is not ClipboardEntry { Kind: "files" } row)
        {
            Notify("请先在剪贴板历史中选择一条本地文件记录。");
            return;
        }
        var entry = await ResolveSelectedAsync(row);
        if (entry?.Content.Files is not { Count: > 0 } files)
        {
            Notify("请先在剪贴板历史中选择一条本地文件记录。");
            return;
        }
        AddStagedFiles(files);
        Notify($"已加入 {files.Count} 项到发送列表。");
    }

    private async void PickFilesClick(object sender, RoutedEventArgs args)
    {
        if (busy) return;
        try
        {
            var picker = new global::Windows.Storage.Pickers.FileOpenPicker();
            WinRT.Interop.InitializeWithWindow.Initialize(picker, Handle);
            picker.FileTypeFilter.Add("*");
            var files = await picker.PickMultipleFilesAsync();
            if (files.Count == 0) return;
            AddStagedFiles(files.Select(file => file.Path));
        }
        catch (Exception ex) { Notify(ex is IOException ? ex.Message : "文件选择未完成。"); }
    }

    private async void PickFolderClick(object sender, RoutedEventArgs args)
    {
        if (busy) return;
        try
        {
            var picker = new global::Windows.Storage.Pickers.FolderPicker();
            WinRT.Interop.InitializeWithWindow.Initialize(picker, Handle);
            picker.FileTypeFilter.Add("*");
            var folder = await picker.PickSingleFolderAsync();
            if (folder is not null) AddStagedFiles([folder.Path]);
        }
        catch (Exception ex) { Notify(ex is IOException ? ex.Message : "文件夹选择未完成。"); }
    }

    private static bool CanStage(string path) =>
        !string.IsNullOrWhiteSpace(path) && (File.Exists(path) || Directory.Exists(path));

    private void AddStagedFiles(IEnumerable<string> paths)
    {
        foreach (string path in paths)
        {
            if (!CanStage(path) || stagedFiles.Contains(path, StringComparer.OrdinalIgnoreCase)) continue;
            stagedFiles.Add(path);
        }
        RenderStagedFiles();
    }

    private void RenderStagedFiles()
    {
        if (StagedFilesPanel is null) return;
        StagedFilesPanel.Children.Clear();
        long totalBytes = 0;
        for (int index = 0; index < stagedFiles.Count; index++)
        {
            string path = stagedFiles[index];
            bool directory = Directory.Exists(path);
            string name = Path.GetFileName(path.TrimEnd(Path.DirectorySeparatorChar));
            long size = 0;
            try
            {
                if (!directory) { size = new FileInfo(path).Length; totalBytes += size; }
            }
            catch (Exception) { }
            var row = new Grid { Padding = new Thickness(0, 6, 0, 6), ColumnSpacing = 10 };
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            var icon = new Border
            {
                Width = 24, Height = 24,
                Child = DfUi.Glyph(directory ? "\uE8B7" : "\uE7C3", 15, DfUi.Muted),
            };
            row.Children.Add(icon);
            var fileName = DfUi.Text(name, 13, false, DfUi.PrimaryText, wrap: false);
            fileName.TextTrimming = TextTrimming.CharacterEllipsis;
            fileName.VerticalAlignment = VerticalAlignment.Center;
            Grid.SetColumn(fileName, 1);
            row.Children.Add(fileName);
            var sizeText = DfUi.Detail(directory ? "文件夹" : FormatBytes(size));
            sizeText.FontFamily = new FontFamily("Cascadia Mono");
            sizeText.VerticalAlignment = VerticalAlignment.Center;
            Grid.SetColumn(sizeText, 2);
            row.Children.Add(sizeText);
            string captured = path;
            var remove = DfUi.IconButton("\uE711", (_, _) => { stagedFiles.Remove(captured); RenderStagedFiles(); }, "移除", 22, 10);
            remove.Foreground = DfUi.Muted;
            Grid.SetColumn(remove, 3);
            row.Children.Add(remove);
            StagedFilesPanel.Children.Add(row);
            if (index < stagedFiles.Count - 1) StagedFilesPanel.Children.Add(DfUi.Divider(0, 0));
        }
        bool any = stagedFiles.Count > 0;
        DropZoneEmpty.Visibility = any ? Visibility.Collapsed : Visibility.Visible;
        StagedListHost.Visibility = any ? Visibility.Visible : Visibility.Collapsed;
        AddToShelfButton.Visibility = any ? Visibility.Visible : Visibility.Collapsed;
        StagedHeaderText.Text = any ? $"{stagedFiles.Count} 个项目 · {FormatBytes(totalBytes)}" : "";
        SendStagedButton.Content = any ? $"发送 {stagedFiles.Count} 个项目" : "发送";
        SendStagedButton.IsEnabled = any && CanSendStagedFiles();
    }

    private void SetDropTargeted(bool targeted)
    {
        if (DropZoneRect is null) return;
        DropZoneRect.Stroke = targeted ? DfUi.Accent : DfUi.Line;
        DropZoneRect.StrokeThickness = targeted ? 1.5 : 1;
        DropZoneTitle.Text = targeted ? "松开即可添加" : "拖入文件或文件夹";
        var accent = (DfUi.Accent as SolidColorBrush)?.Color ?? Colors.DodgerBlue;
        StagedListHost.Background = targeted
            ? new SolidColorBrush(WindowsColor(accent, 0.12))
            : new SolidColorBrush(Colors.Transparent);
    }

    /// <summary>目标电脑：已配对且当前在线；选择变化在快照刷新间保持。</summary>
    private void RenderSendTargets(JsonElement snapshot)
    {
        if (SendTargetPicker is null) return;
        string? selected = SelectedSendTarget();
        var peers = snapshot.Array("peers")
            .Where(peer => peer.String("trust") == "trusted" && peer.String("connection") == "connected")
            .Select(peer => (Id: peer.String("peer_id"), Name: peer.String("display_name", "未命名设备")))
            .Where(peer => peer.Id.Length > 0)
            .ToArray();
        string signature = string.Join("\n", peers.Select(peer => $"{peer.Id} {peer.Name}"));
        if (signature != sendTargetSignature)
        {
            sendTargetSignature = signature;
            SendTargetPicker.Items.Clear();
            foreach (var peer in peers)
                SendTargetPicker.Items.Add(new ComboBoxItem { Content = peer.Name, Tag = peer.Id });
            int index = selected is null ? -1 : Array.FindIndex(peers, peer => peer.Id == selected);
            SendTargetPicker.SelectedIndex = index >= 0 ? index : peers.Length > 0 ? 0 : -1;
        }
        bool hasPeers = peers.Length > 0;
        SendTargetLabel.Visibility = hasPeers ? Visibility.Visible : Visibility.Collapsed;
        SendTargetPicker.Visibility = hasPeers ? Visibility.Visible : Visibility.Collapsed;
        SendNoPeersText.Visibility = hasPeers ? Visibility.Collapsed : Visibility.Visible;
        SendGoAddButton.Visibility = hasPeers ? Visibility.Collapsed : Visibility.Visible;
        SendStagedButton.IsEnabled = stagedFiles.Count > 0 && CanSendStagedFiles();
    }

    private bool CanSendStagedFiles() => !busy && latest.HasValue && service.IsConnected
        && FileTransferEnabled() && clipboard.FilesEnabled && SelectedSendTarget() is not null;

    private string? SelectedSendTarget() => (SendTargetPicker?.SelectedItem as ComboBoxItem)?.Tag as string;

    private void SendGoAddClick(object sender, RoutedEventArgs args) => ShowPage(WorkspacePage.Devices);

    private void AddStagedToShelfClick(object sender, RoutedEventArgs args)
    {
        if (stagedFiles.Count == 0) return;
        try
        {
            NativeClipboard.ValidateLocalFiles(stagedFiles);
            var content = new ClipboardContent(ClipboardContent.LocalFilesMime, [], stagedFiles.ToArray());
            clipboard.ShowShelf(clipboard.History.Add(content, "local", "本机"));
            stagedFiles.Clear();
            RenderStagedFiles();
        }
        catch (Exception ex) when (ex is IOException or InvalidOperationException or UnauthorizedAccessException)
        {
            Notify("加入暂存区失败：文件不可用或数量超过限制。");
        }
    }

    private void ClearStagedClick(object sender, RoutedEventArgs args)
    {
        stagedFiles.Clear();
        RenderStagedFiles();
    }

    private async void SendStagedClick(object sender, RoutedEventArgs args)
    {
        if (stagedFiles.Count == 0 || !CanSendStagedFiles()) return;
        string? target = SelectedSendTarget();
        if (target is null)
        {
            Notify("还没有已连接的电脑；先到设备页添加。");
            return;
        }
        var submitted = stagedFiles.ToArray();
        busy = true;
        RenderStagedFiles();
        try
        {
            bool sent = await clipboard.PublishFilesAsync(submitted, target);
            if (sent)
            {
                foreach (var path in submitted) stagedFiles.Remove(path);
                Notify("已提交文件发送请求；接收状态请查看传输任务。");
            }
            else Notify("文件发送未完成。");
        }
        catch (Exception ex) { Notify(ex is IOException ? ex.Message : "文件发送未完成。"); }
        finally { busy = false; RenderStagedFiles(); }
    }

    private void FileDropDragOver(object sender, DragEventArgs args)
    {
        args.AcceptedOperation = global::Windows.ApplicationModel.DataTransfer.DataPackageOperation.Copy;
        args.DragUIOverride.Caption = "发送文件";
        args.Handled = true;
        SetDropTargeted(true);
    }

    private void FileDropDragLeave(object sender, DragEventArgs args) => SetDropTargeted(false);

    private async void FileDropDrop(object sender, DragEventArgs args)
    {
        SetDropTargeted(false);
        if (!args.DataView.Contains(global::Windows.ApplicationModel.DataTransfer.StandardDataFormats.StorageItems)) return;
        try
        {
            var items = await args.DataView.GetStorageItemsAsync();
            AddStagedFiles(items.Select(item => item.Path));
        }
        catch (Exception ex) { Notify(ex is IOException ? ex.Message : "拖入的文件不可用。"); }
        args.Handled = true;
    }

    private void TransfersPageSizeChanged(object sender, SizeChangedEventArgs args)
    {
        if (TransfersSideColumn is null) return;
        double width = Math.Min(380, args.NewSize.Width * 0.38);
        if (Math.Abs(TransfersSideColumn.Width - width) > 0.5) TransfersSideColumn.Width = width;
    }

    private void TransfersLeftSizeChanged(object sender, SizeChangedEventArgs args)
    {
        double height = Math.Max(0, TransfersLeftColumn.ViewportHeight - 32);
        if (Math.Abs(TransfersLeftContent.MinHeight - height) > 0.5) TransfersLeftContent.MinHeight = height;
    }

    private void RemoteAppsClick(object sender, RoutedEventArgs args) => ShowRemoteApps();

    public void ShowRemoteApps()
    {
        Navigate(WorkspacePage.RemoteApplications);
        RestoreFromTray();
    }

    private async void RevealClick(object sender, RoutedEventArgs args)
    {
        if (HistoryList.SelectedItem is not ClipboardEntry row) return;
        var entry = await ResolveSelectedAsync(row);
        if (entry?.Content.Files?.FirstOrDefault() is not string path) return;
        try { var start = new ProcessStartInfo("explorer.exe") { UseShellExecute = false }; start.ArgumentList.Add("/select,"); start.ArgumentList.Add(path); Process.Start(start); }
        catch (Exception) { Notify("资源管理器无法打开此文件位置。"); }
    }

    private void FitClick(object sender, RoutedEventArgs args) => ImagePreview.Stretch = ImagePreview.Stretch == Stretch.Uniform ? Stretch.UniformToFill : Stretch.Uniform;

    private void SyncChanged(object sender, RoutedEventArgs args)
    {
        if (updatingSync) return;
        clipboard.SyncEnabled = SyncCheck.IsChecked == true;
        Notify(clipboard.SyncEnabled ? "已开启文字与图片同步；只发送之后的新复制，不重放历史。" : "已关闭跨设备剪贴板同步；本机历史继续记录。");
        UpdateFeatureToggleState();
    }

    // MARK: - Snapshot

    private void SnapshotChanged(JsonElement snapshot)
    {
        latest = snapshot;
        localName = snapshot.String("local_name", "这台 Windows");
        bool supportsClipboard = snapshot.TryGetProperty("capabilities", out var caps) && caps.Bool("clipboard");
        bool supportsFiles = caps.ValueKind == JsonValueKind.Object && caps.Bool("files");
        updatingSync = true;
        SyncCheck.IsEnabled = supportsClipboard || clipboard.SyncEnabled;
        SyncCheck.IsChecked = clipboard.SyncEnabled;
        updatingSync = false;
        SyncStatus.Text = supportsClipboard
            ? (clipboard.SyncEnabled
                ? (supportsFiles ? "文字 / 图片 / 富文本 / 文件 · 文本最多 16 MiB" : "文字 / PNG / HTML / RTF · 最多 16 MiB")
                : "同步已关闭；远端新内容会明确拒收")
            : "桌面同步尚未就绪；本机历史可用";
        if (pairIpButton is not null) pairIpButton.IsEnabled = !busy && snapshot.Bool("network_running");
        RefreshInputState(snapshot);
        RenderLanWeb(snapshot);
        RenderTransfers(snapshot);
        RenderSendTargets(snapshot);
        UpdateHealth(true);
        UpdateFeatureToggleState();
        string raw = snapshot.GetRawText();
        if (raw != lastDeviceSnapshot)
        {
            lastDeviceSnapshot = raw;
            RebuildDevices();
            BuildNavigation();
        }
        string error = snapshot.String("last_error");
        if (error.Length > 0 && error != lastErrorNotice) Notify(error);
        lastErrorNotice = error;
        settings?.RefreshService(snapshot);
        long revision = snapshot.Number("library_revision", -1);
        if (revision != lastLibraryRevision || lastLibraryDay != DateTime.Today)
        {
            lastLibraryRevision = revision;
            lastLibraryDay = DateTime.Today;
            RefreshHistory();
        }
    }

    private void ConnectionChanged(bool connected, string message)
    {
        if (!connected)
        {
            latest = null;
            pendingInputController = null;
            lastLibraryRevision = -1;
            updatingSync = true;
            SyncCheck.IsChecked = clipboard.SyncEnabled;
            SyncCheck.IsEnabled = clipboard.SyncEnabled;
            updatingSync = false;
            SyncStatus.Text = "同步暂停；重新连接不会重发旧历史";
            if (pairIpButton is not null) pairIpButton.IsEnabled = false;
            InputAvailability.Text = "后台服务已断开，正在重新连接；鼠标共享暂不可用。";
            InputAvailability.Visibility = Visibility.Visible;
            ReclaimInputButton.Visibility = Visibility.Collapsed;
            ClearInputQuality();
            MouseSharingPill.Visibility = Visibility.Collapsed;
            DrawInputLayout(default);
            RenderSendTargets(default);
            lastDeviceSnapshot = "";
            lastErrorNotice = "";
            RebuildDevices();
            if (wasConnected) Notify(message);
        }
        else if (!wasConnected)
        {
            _ = LoadWorkflowsAsync();
        }
        wasConnected = connected;
        UpdateHealth(connected);
        UpdateFeatureToggleState();
        RefreshHistory();
    }

    internal void ApplyOnboarding(OnboardingResult result)
    {
        clipboard.SyncEnabled = result.Clipboard;
        clipboard.CaptureEnabled = result.Clipboard;
        remoteAppsEnabled = result.RemoteApplications;
        ShowPage(WorkspacePage.Devices);
        if (latest is JsonElement snapshot && snapshot.TryGetProperty("input", out var input)
            && input.TryGetProperty("settings", out var settings) && settings.ValueKind == JsonValueKind.Object)
        {
            _ = Task.Run(async () =>
            {
                try
                {
                    var node = JsonNode.Parse(settings.GetRawText())?.AsObject();
                    if (node is null) return;
                    node["feature_enabled"] = result.Input;
                    node["enabled"] = result.Input;
                    if (result.Input) node["role"] = "Controller";
                    await service.SendAsync(ServiceClient.Command("SetInputSharingSettings", new { settings = node }));
                }
                catch (Exception) { /* The feature page surfaces the live state. */ }
            });
        }
    }

    public void ShowOnboarding(UIElement view)
    {
        OnboardingHost.Children.Clear();
        OnboardingHost.Children.Add(view);
        OnboardingOverlay.Visibility = Visibility.Visible;
    }

    public void HideOnboarding()
    {
        OnboardingOverlay.Visibility = Visibility.Collapsed;
        OnboardingHost.Children.Clear();
    }

    private void DismissNoticeClick(object sender, RoutedEventArgs args) => NoticeHost.Visibility = Visibility.Collapsed;

    internal void Notify(string message)
    {
        Notice.Text = message;
        NoticeHost.Visibility = Visibility.Visible;
    }

    // MARK: - Input page

    /// <summary>最近一次输入状态快照，供流畅度详情 Flyout 在打开时读取。</summary>
    private JsonElement inputSnapshot;
    private bool inputSnapshotValid;

    private void ClearInputQuality()
    {
        inputSnapshotValid = false;
        if (QualityBadge is null) return;
        QualityBadge.Visibility = Visibility.Collapsed;
        InputLayoutSummary.Visibility = Visibility.Visible;
        QualityBadgeCanvas.Children.Clear();
        QualityBadgeLatency.Text = "—";
        QualityBadgeDot.Fill = DfUi.Muted;
        QualityBadgeStatus.Text = "空闲";
        QualityBadgeStatus.Foreground = DfUi.Muted;
    }

    private sealed record LatencySample(double Second, double RttMs);

    /// <summary>最近 60 秒的逐秒往返延迟；与 mac 一致只取毫秒非负的样本。</summary>
    private static List<LatencySample> LatencySamples(JsonElement quality)
    {
        var points = new List<LatencySample>();
        if (quality.ValueKind != JsonValueKind.Object) return points;
        int index = 0;
        foreach (var sample in quality.Array("samples"))
        {
            index++;
            if (sample.ValueKind != JsonValueKind.Object) continue;
            double rtt = sample.Double("rtt_ms");
            double second = sample.TryGetProperty("second", out var s) && s.ValueKind == JsonValueKind.Number
                ? s.GetDouble() : index - 1;
            if (!double.IsFinite(rtt) || rtt < 0) continue;
            points.Add(new LatencySample(Math.Clamp(second, 0, 60), rtt));
        }
        return points;
    }

    private static string Milliseconds(double? value)
    {
        if (value is not double number) return "—";
        if (number > 0 && number < 0.1) return $"{number:0.00} ms";
        return $"{number:0.0} ms";
    }

    /// <summary>P95 超过 50 ms 标红、超过 20 ms 标黄，否则与 mac 一致用强调色。</summary>
    private static Brush LatencySeriesColor(double? p95Ms) =>
        p95Ms is > 50 ? DfUi.Danger : p95Ms is > 20 ? DfUi.Warning : DfUi.Accent;

    private static (string Title, Brush Color) QualityStatus(JsonElement quality)
    {
        bool hasSignal = quality.ValueKind == JsonValueKind.Object
            && (quality.Number("rtt_p50_micros") > 0 || quality.Number("rtt_p95_micros") > 0 || quality.Number("hitches") > 0);
        return quality.String("grade", "idle") switch
        {
            "smooth" => ("流畅", DfUi.Success),
            "fair" => ("一般", DfUi.Warning),
            "poor" => ("较差", DfUi.Danger),
            _ => hasSignal ? ("采样较少", DfUi.Muted) : ("空闲", DfUi.Muted),
        };
    }

    /// <summary>迷你趋势线：固定 0–100 ms 刻度，采样间隔超过 2 秒断开。</summary>
    private static void DrawLatencySparkline(Canvas canvas, IReadOnlyList<LatencySample> points, Brush series)
    {
        canvas.Children.Clear();
        double width = canvas.Width, height = canvas.Height;
        if (width < 4 || height < 4 || points.Count == 0) return;
        double MapX(double second) => second / 60.0 * width;
        double MapY(double rtt) => height - Math.Min(rtt, 100) / (100 * 1.15) * height;
        var run = new PointCollection();
        double previous = double.NaN;
        void Flush()
        {
            if (run.Count >= 2)
            {
                var polyline = new Microsoft.UI.Xaml.Shapes.Polyline
                {
                    Stroke = series,
                    StrokeThickness = 1.5,
                    StrokeLineJoin = PenLineJoin.Round,
                    StrokeStartLineCap = PenLineCap.Round,
                    StrokeEndLineCap = PenLineCap.Round,
                };
                foreach (var point in run) polyline.Points.Add(point);
                canvas.Children.Add(polyline);
            }
            run.Clear();
        }
        foreach (var point in points)
        {
            if (!double.IsNaN(previous) && point.Second - previous > 2) Flush();
            previous = point.Second;
            run.Add(new global::Windows.Foundation.Point(MapX(point.Second), MapY(point.RttMs)));
        }
        Flush();
        if (points.Count == 1)
        {
            // 只有一个样本时画竖线，与 mac 的 RuleMark 一致。
            canvas.Children.Add(new Microsoft.UI.Xaml.Shapes.Line
            {
                X1 = MapX(points[0].Second), X2 = MapX(points[0].Second),
                Y1 = height, Y2 = MapY(points[0].RttMs),
                Stroke = series, StrokeThickness = 1.5, StrokeStartLineCap = PenLineCap.Round,
            });
        }
    }

    private void RenderQualityBadge(JsonElement input)
    {
        if (QualityBadge is null) return;
        var settings = input.TryGetProperty("settings", out var sharing) && sharing.ValueKind == JsonValueKind.Object ? sharing : default;
        bool shows = settings.ValueKind == JsonValueKind.Object
            && settings.Bool("enabled") && settings.String("role") == "Controller"
            && !input.Bool("managed");
        QualityBadge.Visibility = shows ? Visibility.Visible : Visibility.Collapsed;
        InputLayoutSummary.Visibility = shows ? Visibility.Collapsed : Visibility.Visible;
        if (!shows) return;
        inputSnapshot = input;
        inputSnapshotValid = true;
        var quality = input.TryGetProperty("quality", out var report) && report.ValueKind == JsonValueKind.Object ? report : default;
        var samples = LatencySamples(quality);
        double? p50 = quality.ValueKind == JsonValueKind.Object && quality.Number("rtt_p50_micros") is > 0 ? quality.Number("rtt_p50_micros") / 1000.0 : null;
        double? p95 = quality.ValueKind == JsonValueKind.Object && quality.Number("rtt_p95_micros") is > 0 ? quality.Number("rtt_p95_micros") / 1000.0 : null;
        double? typical = p50 ?? p95 ?? (samples.Count > 0 ? samples[^1].RttMs : null);
        QualityBadgeLatency.Text = Milliseconds(typical);
        var (title, color) = QualityStatus(quality);
        QualityBadgeDot.Fill = color;
        QualityBadgeStatus.Text = title;
        QualityBadgeStatus.Foreground = color;
        DrawLatencySparkline(QualityBadgeCanvas, samples, LatencySeriesColor(p95 ?? typical));
    }

    private void QualityBadgeClick(object sender, RoutedEventArgs args)
    {
        if (!inputSnapshotValid) return;
        var panel = BuildQualityDetailPanel(inputSnapshot);
        var flyout = new Flyout
        {
            Content = new Border
            {
                Background = DfUi.Surface,
                Padding = new Thickness(18),
                CornerRadius = new CornerRadius(12),
                BorderBrush = DfUi.Line,
                BorderThickness = new Thickness(1),
                Width = 520,
                Child = panel,
            },
            Placement = FlyoutPlacementMode.BottomEdgeAlignedLeft,
        };
        flyout.ShowAt(QualityBadge);
    }

    /// <summary>流畅度详情面板，结构与 mac qualityChart 一致。</summary>
    private UIElement BuildQualityDetailPanel(JsonElement input)
    {
        var quality = input.TryGetProperty("quality", out var report) && report.ValueKind == JsonValueKind.Object ? report : default;
        var samples = LatencySamples(quality);
        double? p50 = quality.Number("rtt_p50_micros") is > 0 ? quality.Number("rtt_p50_micros") / 1000.0 : null;
        double? p95 = quality.Number("rtt_p95_micros") is > 0 ? quality.Number("rtt_p95_micros") / 1000.0 : null;
        double? typical = p50 ?? p95 ?? (samples.Count > 0 ? samples[^1].RttMs : null);
        double peak = samples.Count > 0 ? samples.Max(point => point.RttMs) : 0;
        bool showPeak = peak > 0 && (typical is not double t || (peak > t + 0.2 && peak > t * 1.25));
        var (statusTitle, statusColor) = QualityStatus(quality);
        var series = LatencySeriesColor(p95 ?? typical);

        var panel = new StackPanel { Spacing = 16 };

        // 头部：图标 + 标题 + 状态胶囊
        var header = new Grid { ColumnSpacing = 10 };
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var titleRow = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6, VerticalAlignment = VerticalAlignment.Center };
        titleRow.Children.Add(DfUi.Glyph("\uE9D2", 13));
        titleRow.Children.Add(DfUi.Text("流畅度", 13, true, wrap: false));
        header.Children.Add(titleRow);
        var statusPill = new Border
        {
            CornerRadius = new CornerRadius(11),
            Padding = new Thickness(9, 4, 9, 4),
            Background = SubtleTint(statusColor),
            VerticalAlignment = VerticalAlignment.Center,
            Child = new StackPanel
            {
                Orientation = Orientation.Horizontal,
                Spacing = 5,
                Children =
                {
                    new Microsoft.UI.Xaml.Shapes.Ellipse { Width = 6, Height = 6, Fill = statusColor, VerticalAlignment = VerticalAlignment.Center },
                    DfUi.Text(statusTitle, 11, true, statusColor, wrap: false),
                },
            },
        };
        Grid.SetColumn(statusPill, 2);
        header.Children.Add(statusPill);
        panel.Children.Add(header);

        var chartSection = new StackPanel { Spacing = 10 };
        if (typical is double typicalValue)
        {
            var hero = new Grid { ColumnSpacing = 10 };
            hero.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            hero.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            hero.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            var value = DfUi.Text(Milliseconds(typicalValue), 24, true, wrap: false);
            value.FontFamily = new FontFamily("Cascadia Mono");
            hero.Children.Add(value);
            var label = DfUi.Detail("典型往返延迟", 11);
            label.VerticalAlignment = VerticalAlignment.Bottom;
            label.Margin = new Thickness(0, 0, 0, 3);
            Grid.SetColumn(label, 1);
            hero.Children.Add(label);
            if (showPeak)
            {
                var peakText = DfUi.Detail($"峰值 {Milliseconds(peak)}", 11);
                peakText.FontFamily = new FontFamily("Cascadia Mono");
                peakText.VerticalAlignment = VerticalAlignment.Bottom;
                peakText.Margin = new Thickness(0, 0, 0, 3);
                Grid.SetColumn(peakText, 2);
                hero.Children.Add(peakText);
            }
            chartSection.Children.Add(hero);
        }
        var chartHost = new Grid { Height = 168 };
        chartHost.SizeChanged += (_, _) => DrawLatencyChart(chartHost, samples, series);
        DrawLatencyChart(chartHost, samples, series);
        chartSection.Children.Add(chartHost);
        chartSection.Children.Add(DfUi.Detail("固定刻度 0–100 ms；超过上限的点显示在上限处，数值仍为实际延迟。", 11));
        panel.Children.Add(chartSection);
        panel.Children.Add(DfUi.Divider(0, 0));

        // 指标行：P50 / P95 / 网络卡顿
        var metrics = new Grid { ColumnSpacing = 16 };
        string[] metricTitles = ["P50", "P95", "网络卡顿"];
        string[] metricValues = [Milliseconds(p50), Milliseconds(p95), HitchRate(quality)];
        for (int i = 0; i < metricTitles.Length; i++)
        {
            metrics.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            var cell = new StackPanel { Spacing = 3 };
            cell.Children.Add(DfUi.Detail(metricTitles[i], 11));
            var metricValue = DfUi.Text(metricValues[i], 15, true, wrap: false);
            metricValue.FontFamily = new FontFamily("Cascadia Mono");
            cell.Children.Add(metricValue);
            Grid.SetColumn(cell, i);
            metrics.Children.Add(cell);
        }
        panel.Children.Add(metrics);

        BuildQualityAdvice(panel, quality);
        BuildQualityDiagnostics(panel, input, quality);
        var footer = DfUi.Detail("最近 60 秒 · 3 秒平滑趋势 · 指标与峰值为原始采样", 11);
        footer.Opacity = 0.7;
        panel.Children.Add(footer);
        return panel;
    }

    private static string HitchRate(JsonElement quality)
    {
        long hitches = quality.ValueKind == JsonValueKind.Object ? quality.Number("hitches") : 0;
        return hitches == 0 ? "0" : $"{hitches} 次/分钟";
    }

    private static SolidColorBrush SubtleTint(Brush source)
    {
        var color = (source as SolidColorBrush)?.Color ?? Colors.Gray;
        color.A = (byte)Math.Round(color.A * 0.12 + 0.5);
        return new SolidColorBrush(color);
    }

    private void BuildQualityAdvice(StackPanel panel, JsonElement quality)
    {
        string fallback = quality.ValueKind == JsonValueKind.Object ? quality.String("fallback") : "";
        if (fallback.Length > 0)
        {
            var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
            row.Children.Add(DfUi.Glyph("\uE946", 11, DfUi.Muted));
            row.Children.Add(DfUi.Detail(fallback, 11));
            panel.Children.Add(row);
        }
        var suggestions = quality.ValueKind == JsonValueKind.Object ? quality.Array("suggestions").ToList() : new List<JsonElement>();
        if (suggestions.Count == 0) return;
        var group = new StackPanel { Spacing = 8 };
        group.Children.Add(DfUi.Text("优化建议", 11, true));
        foreach (var suggestion in suggestions)
        {
            string trigger = suggestion.String("trigger");
            if (trigger.Length == 0) continue;
            string priority = suggestion.String("priority");
            var (glyph, tint) = priority switch
            {
                "info" => ("\uE946", DfUi.Muted),
                "ok" => ("\uE73E", DfUi.Success),
                _ => ("\uEA80", DfUi.Warning),
            };
            var row = new Grid { ColumnSpacing = 6 };
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            row.Children.Add(DfUi.Glyph(glyph, 11, tint));
            var triggerText = DfUi.Detail(trigger, 11);
            Grid.SetColumn(triggerText, 1);
            row.Children.Add(triggerText);
            var info = DfUi.IconButton("\uE946", null, "查看建议详情", 20, 11);
            Grid.SetColumn(info, 2);
            row.Children.Add(info);
            var button = new Button
            {
                Content = row,
                Background = new SolidColorBrush(Colors.Transparent),
                BorderThickness = new Thickness(0),
                Padding = new Thickness(0),
                MinHeight = 0,
                HorizontalAlignment = HorizontalAlignment.Stretch,
                HorizontalContentAlignment = HorizontalAlignment.Stretch,
                Template = (ControlTemplate)Application.Current.Resources["DFFlatButtonTemplate"],
            };
            void ShowSuggestionDetail()
            {
                var detail = new StackPanel { Spacing = 8, Width = 320 };
                detail.Children.Add(DfUi.Text(suggestion.String("trigger"), 11, true));
                foreach (var (label, key) in new[] { ("做法", "action"), ("预期", "expected"), ("复测", "retest") })
                {
                    var cell = new StackPanel { Spacing = 2 };
                    var caption = DfUi.Detail(label, 11);
                    caption.Opacity = 0.7;
                    cell.Children.Add(caption);
                    cell.Children.Add(DfUi.Detail(suggestion.String(key), 11));
                    detail.Children.Add(cell);
                }
                new Flyout
                {
                    Content = new Border
                    {
                        Background = DfUi.Surface,
                        Padding = new Thickness(12),
                        CornerRadius = new CornerRadius(12),
                        BorderBrush = DfUi.Line,
                        BorderThickness = new Thickness(1),
                        Child = detail,
                    },
                    Placement = FlyoutPlacementMode.Bottom,
                }.ShowAt(button);
            }
            info.Click += (_, _) => ShowSuggestionDetail();
            button.Click += (_, _) => ShowSuggestionDetail();
            group.Children.Add(button);
        }
        panel.Children.Add(group);
    }

    private void BuildQualityDiagnostics(StackPanel panel, JsonElement input, JsonElement quality)
    {
        string mode = quality.String("mode");
        foreach (var link in input.Array("links"))
        {
            if (link.String("control_session").Length == 0) continue;
            string linkMode = link.String("motion_mode");
            if (linkMode.Length > 0) mode = linkMode;
            break;
        }
        string modeLabel = mode switch
        {
            "relative" => "相对移动",
            "hybrid" => "绝对坐标",
            _ => mode.Length == 0 ? "等待会话" : "可靠路径",
        };
        var lines = new StackPanel { Spacing = 6 };
        lines.Children.Add(DfUi.Detail("每秒更新；空闲时显示设备连接延迟，没有可用延迟时显示 0。", 11));
        lines.Children.Add(DfUi.Detail($"当前模式：{modeLabel}", 11));
        lines.Children.Add(DfUi.Detail("数据报通道：尚无采样", 11));
        lines.Children.Add(DfUi.Detail("平台没有光标呈现时间。端到端时间是采集排队、发送、往返和注入的估计，不是两台电脑的时钟同步。", 11));
        var expander = new Expander
        {
            Header = "查看诊断详情",
            Content = lines,
            FontSize = 11,
            HorizontalAlignment = HorizontalAlignment.Stretch,
            HorizontalContentAlignment = HorizontalAlignment.Stretch,
        };
        panel.Children.Add(expander);
    }

    /// <summary>完整延迟图：固定 0–100 ms 刻度、渐变底色、坐标轴标签。</summary>
    private static void DrawLatencyChart(Grid host, IReadOnlyList<LatencySample> points, Brush series)
    {
        host.Children.Clear();
        double width = host.ActualWidth > 8 ? host.ActualWidth : 484;
        double height = host.Height;
        const double labelWidth = 40, axisHeight = 16;
        double plotWidth = Math.Max(40, width - labelWidth);
        double plotHeight = Math.Max(40, height - axisHeight);
        double MapX(double second) => labelWidth + (second + 1) / 62.0 * plotWidth;
        double MapY(double rtt) => Math.Min(rtt, 100) / (100 * 1.28) * plotHeight;
        double Y(double rtt) => plotHeight - MapY(rtt);

        // 网格线与 Y 轴标签：0 / 50 / 100 ms
        foreach (double tick in new[] { 0.0, 50.0, 100.0 })
        {
            var gridLine = new Microsoft.UI.Xaml.Shapes.Line
            {
                X1 = labelWidth, X2 = width, Y1 = Y(tick), Y2 = Y(tick),
                Stroke = DfUi.Line, StrokeThickness = 1, Opacity = 0.4,
            };
            host.Children.Add(gridLine);
            string text = tick == 100 ? "100 ms" : $"{tick:0}";
            var label = DfUi.Detail(text, 11);
            label.FontFamily = new FontFamily("Cascadia Mono");
            label.TextAlignment = TextAlignment.Right;
            label.Width = labelWidth - 6;
            label.Margin = new Thickness(0, Y(tick) - 8, 0, 0);
            label.VerticalAlignment = VerticalAlignment.Top;
            host.Children.Add(label);
        }
        // X 轴标签：60秒前 / 30秒前 / 现在
        foreach (var (second, text) in new[] { (0.0, "60秒前"), (30.0, "30秒前"), (60.0, "现在") })
        {
            var label = DfUi.Detail(text, 11);
            label.Margin = new Thickness(second == 0 ? labelWidth : second == 60 ? width - 34 : labelWidth + plotWidth / 2 - 17, plotHeight + 2, 0, 0);
            label.VerticalAlignment = VerticalAlignment.Top;
            host.Children.Add(label);
        }
        if (points.Count == 0) return;

        // 分段：采样间隔超过 2 秒断开。
        var runs = new List<List<LatencySample>>();
        List<LatencySample>? current = null;
        double previous = double.NaN;
        foreach (var point in points)
        {
            if (current is null || point.Second - previous > 2) { current = []; runs.Add(current); }
            previous = point.Second;
            current.Add(point);
        }
        var accent = (series as SolidColorBrush)?.Color ?? Colors.DodgerBlue;
        foreach (var run in runs)
        {
            if (run.Count >= 2)
            {
                var areaPoints = new PointCollection { new global::Windows.Foundation.Point(MapX(run[0].Second), plotHeight) };
                foreach (var point in run) areaPoints.Add(new global::Windows.Foundation.Point(MapX(point.Second), Y(point.RttMs)));
                areaPoints.Add(new global::Windows.Foundation.Point(MapX(run[^1].Second), plotHeight));
                host.Children.Add(new Microsoft.UI.Xaml.Shapes.Polygon
                {
                    Points = areaPoints,
                    Fill = new LinearGradientBrush(
                        new GradientStopCollection
                        {
                            new GradientStop { Color = WindowsColor(accent, 0.30), Offset = 0 },
                            new GradientStop { Color = WindowsColor(accent, 0.03), Offset = 1 },
                        }, 90),
                });
                var line = new Microsoft.UI.Xaml.Shapes.Polyline
                {
                    Stroke = series,
                    StrokeThickness = 2,
                    StrokeLineJoin = PenLineJoin.Round,
                    StrokeStartLineCap = PenLineCap.Round,
                    StrokeEndLineCap = PenLineCap.Round,
                };
                foreach (var point in run) line.Points.Add(new global::Windows.Foundation.Point(MapX(point.Second), Y(point.RttMs)));
                host.Children.Add(line);
            }
            else if (points.Count == 1)
            {
                host.Children.Add(new Microsoft.UI.Xaml.Shapes.Line
                {
                    X1 = MapX(run[0].Second), X2 = MapX(run[0].Second), Y1 = plotHeight, Y2 = Y(run[0].RttMs),
                    Stroke = series, StrokeThickness = 2, StrokeStartLineCap = PenLineCap.Round,
                });
            }
        }
        // 数据点：少于 8 个全部标出，否则只标最后一个。
        IReadOnlyList<LatencySample> marked = points.Count <= 8 ? points : new List<LatencySample> { points[^1] };
        foreach (var point in marked)
        {
            var dot = new Microsoft.UI.Xaml.Shapes.Ellipse { Width = 5, Height = 5, Fill = series };
            dot.Margin = new Thickness(MapX(point.Second) - 2.5, Y(point.RttMs) - 2.5, 0, 0);
            dot.VerticalAlignment = VerticalAlignment.Top;
            host.Children.Add(dot);
        }
    }

    private static global::Windows.UI.Color WindowsColor(global::Windows.UI.Color color, double opacity)
    {
        color.A = (byte)Math.Round(255 * opacity);
        return color;
    }

    private void InputPageSizeChanged(object sender, SizeChangedEventArgs args)
    {
        double height = Math.Max(0, InputPage.ViewportHeight - 28);
        if (Math.Abs(InputContent.MinHeight - height) > 0.5) InputContent.MinHeight = height;
    }

    private void WifiIdentityClick(object sender, RoutedEventArgs args) => ShowWifiIdentityFlyout();

    private void RefreshInputState(JsonElement snapshot)
    {
        bool capable = snapshot.TryGetProperty("capabilities", out var capabilities) && capabilities.Bool("input");
        if (!snapshot.TryGetProperty("input", out var input) || input.ValueKind != JsonValueKind.Object)
        {
            InputAvailability.Text = "正在等待后台服务提供鼠标共享状态。";
            InputAvailability.Visibility = Visibility.Visible;
            ReclaimInputButton.Visibility = Visibility.Collapsed;
            MouseSharingPill.Visibility = Visibility.Collapsed;
            ClearInputQuality();
            if (WifiIdentityButton is not null)
            {
                WifiIdentitySummary.Text = "未连接 Wi-Fi";
                WifiIdentityWarning.Visibility = Visibility.Collapsed;
                WifiIdentityGlyph.Foreground = DfUi.Muted;
            }
            DrawInputLayout(default);
            return;
        }
        bool managed = input.Bool("managed");
        string status = input.String("status", "standby");
        string reason = input.String("reason");
        string controllerId = input.String("controller_id");
        string controller = snapshot.Array("peers").FirstOrDefault(peer => peer.String("peer_id") == controllerId).String("display_name", controllerId);
        bool ready = capable && status.Equals("Ready", StringComparison.OrdinalIgnoreCase);
        ReclaimInputButton.IsEnabled = managed;
        ReclaimInputButton.Visibility = managed ? Visibility.Visible : Visibility.Collapsed;
        InputAvailability.Text = !capable
            ? inputHelperFailure ?? "本机输入组件尚未就绪，暂时无法跨屏控制。请在设置 → 权限查看安装和授权状态。"
            : managed
                ? $"{(ready ? "正在被控" : "被控准备中")} · 主控：{(controller.Length > 0 ? controller : "已授权设备")}{(reason.Length > 0 ? $"\n{reason}" : "")}"
                : $"输入 helper 已连接 · {input.Number("display_count")} 个本机显示器 · 等待设备网络中的主控。";
        inputSnapshot = input;
        inputSnapshotValid = true;
        var settings = input.TryGetProperty("settings", out var sharing) && sharing.ValueKind == JsonValueKind.Object ? sharing : default;
        bool controllerEnabled = settings.ValueKind == JsonValueKind.Object && settings.Bool("enabled") && settings.String("role") == "Controller";
        // 与 macOS 鼠标自定义页一致：键鼠共享开启且本机作为主控时展示生效状态。
        bool sharingActive = controllerEnabled && settings.Bool("feature_enabled") && !managed;
        MouseSharingPill.Visibility = sharingActive ? Visibility.Visible : Visibility.Collapsed;
        InputAvailability.Visibility = !capable || managed ? Visibility.Visible : Visibility.Collapsed;
        if (controllerEnabled && capable)
        {
            string controllingId = input.TryGetProperty("controlling", out var controlling) && controlling.ValueKind == JsonValueKind.Object ? controlling.String("peer_id") : "";
            string target = input.TryGetProperty("controlling", out var controlling2) && controlling2.ValueKind == JsonValueKind.Object ? controlling2.String("target_display") : "";
            string peer = snapshot.Array("peers").FirstOrDefault(p => p.String("peer_id") == controllingId).String("display_name", controllingId);
            InputAvailability.Text += $"\n本机已开启主控{(peer.Length > 0 && controllingId.Length > 0 ? $"：正在控制 {peer}{(target.Length > 0 ? $"（{target}）" : "")}" : "；把指针移到屏幕边缘即可控制对端")}";
        }
        RenderQualityBadge(input);
        RenderWifiIdentity(input, snapshot);
        DrawInputLayout(input);
    }

    /// <summary>页头 Wi-Fi 胶囊：SSID 摘要 + 对端不一致时给出警示。</summary>
    private void RenderWifiIdentity(JsonElement input, JsonElement snapshot)
    {
        if (WifiIdentityButton is null) return;
        var environment = input.TryGetProperty("environment", out var env) && env.ValueKind == JsonValueKind.Object ? env : default;
        string effective = environment.String("ssid");
        string observed = environment.String("observed_ssid");
        string reference = effective.Length > 0 ? effective : observed;
        bool attention = reference.Length > 0 && environment.Array("peer_ssids")
            .Any(peer => peer.String("ssid") is { Length: > 0 } peerSsid && peerSsid != reference);
        WifiIdentitySummary.Text = reference.Length > 0 ? reference : "未连接 Wi-Fi";
        WifiIdentityWarning.Visibility = attention ? Visibility.Visible : Visibility.Collapsed;
        WifiIdentityGlyph.Foreground = attention ? DfUi.Warning : DfUi.Muted;
    }

    private void ShowWifiIdentityFlyout()
    {
        if (!inputSnapshotValid || latest is not JsonElement snapshot) return;
        var input = inputSnapshot;
        var environment = input.TryGetProperty("environment", out var env) && env.ValueKind == JsonValueKind.Object ? env : default;
        string observed = environment.String("observed_ssid");
        string effective = environment.String("ssid");
        string message = environment.String("message");
        bool adopted = effective.Length > 0 && effective != observed;
        string reference = effective.Length > 0 ? effective : observed;
        string localId = input.String("local_device_id");
        string NameOf(string id) => id.Length > 0 && id == localId ? "这台电脑"
            : snapshot.Array("peers").FirstOrDefault(p => p.String("peer_id") == id) is { ValueKind: JsonValueKind.Object } peer
                ? peer.String("display_name", id.Length > 8 ? id[..8] : id)
                : (id.Length > 8 ? id[..8] : id);
        string preferred = "";
        if (effective.Length > 0)
        {
            foreach (var rule in environment.Array("rules"))
            {
                if (rule.String("ssid") == effective) { preferred = NameOf(rule.String("controller")); break; }
            }
        }
        var peers = environment.Array("peer_ssids")
            .Where(peer => peer.String("ssid").Length > 0)
            .Select(peer => (Id: peer.String("device_id"), Ssid: peer.String("ssid")))
            .OrderBy(peer => NameOf(peer.Id), StringComparer.CurrentCultureIgnoreCase)
            .ThenBy(peer => peer.Id, StringComparer.Ordinal)
            .ToArray();

        TextBlock Mono(string text) => new()
        {
            Text = text, FontSize = 12, FontFamily = new FontFamily("Cascadia Mono"),
            Foreground = DfUi.PrimaryText, IsTextSelectionEnabled = true,
        };
        TextBlock Label(string text) => DfUi.Detail(text, 13);

        var panel = new StackPanel { Spacing = 12 };
        var heading = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
        heading.Children.Add(DfUi.Glyph("\uE701", 13, DfUi.PrimaryText));
        heading.Children.Add(DfUi.Text("Wi-Fi 标识", 13, true, wrap: false));
        panel.Children.Add(heading);

        var grid = new Grid { ColumnSpacing = 16, RowSpacing = 8 };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        int rowIndex = 0;
        void AddRow(string label, FrameworkElement value)
        {
            grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
            var caption = Label(label);
            Grid.SetRow(caption, rowIndex);
            grid.Children.Add(caption);
            Grid.SetRow(value, rowIndex);
            Grid.SetColumn(value, 1);
            grid.Children.Add(value);
            rowIndex++;
        }

        AddRow("本机读取", observed.Length > 0
            ? Mono(observed)
            : DfUi.Detail("未连接 Wi-Fi；有线网络会采用对端上报的标识", 13));

        if (effective.Length > 0)
        {
            var value = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
            value.Children.Add(Mono(effective));
            if (adopted) value.Children.Add(DfUi.Detail("采用对端上报", 11));
            AddRow("场景判定", value);
        }
        else AddRow("场景判定", DfUi.Detail("未确定", 13));

        AddRow("首选主控", DfUi.Text(preferred.Length > 0 ? preferred : "该网络未保存主控", 13));

        if (peers.Length == 0)
        {
            AddRow("对端上报", DfUi.Detail("暂无；对端需与本机建立输入链路后才会上报", 13));
        }
        else
        {
            var list = new StackPanel { Spacing = 4 };
            foreach (var peer in peers)
            {
                var line = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
                line.Children.Add(DfUi.Text(NameOf(peer.Id), 13, wrap: false));
                line.Children.Add(Mono(peer.Ssid));
                if (reference.Length > 0)
                {
                    bool matches = peer.Ssid == reference;
                    var tag = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4, VerticalAlignment = VerticalAlignment.Center };
                    tag.Children.Add(DfUi.Glyph(matches ? "\uE73E" : "\uE7BA", 11, matches ? DfUi.Success : DfUi.Warning));
                    tag.Children.Add(DfUi.Text(matches ? "一致" : "不一致", 11, false, matches ? DfUi.Success : DfUi.Warning, wrap: false));
                    line.Children.Add(tag);
                }
                list.Children.Add(line);
            }
            AddRow("对端上报", list);
        }
        panel.Children.Add(grid);
        if (message.Length > 0) panel.Children.Add(DfUi.Detail(message, 11));

        new Flyout
        {
            Content = new Border
            {
                Background = DfUi.Surface,
                Padding = new Thickness(16),
                CornerRadius = new CornerRadius(12),
                BorderBrush = DfUi.Line,
                BorderThickness = new Thickness(1),
                Width = 380,
                Child = panel,
            },
            Placement = FlyoutPlacementMode.BottomEdgeAlignedLeft,
        }.ShowAt(WifiIdentityButton);
    }

    /// <summary>Draws the shared screen layout: local displays plus remote
    /// participant topologies, positioned like the runtime layout resolver.</summary>
    private readonly DisplayWallpapers displayWallpapers = new();

    private void DrawInputLayout(JsonElement input)
    {
        if (DisplayCanvas is null || DisplayCanvas.ActualWidth < 10 || DisplayCanvas.ActualHeight < 10 || draggingDisplay is not null) return;
        if (pendingLayoutMove is { } pending)
        {
            // An IPC acknowledgement only queues the command. Ignore heartbeat
            // snapshots until every screen in the moved desktop is confirmed.
            if (!InputLayout.Confirms(JsonProperty(input, "settings"), pending.Before, pending.DeviceId, pending.X, pending.Y)) return;
            pendingLayoutMove = null;
            Notify("屏幕布局已保存。");
        }
        DisplayCanvas.Children.Clear();
        var monitors = MonitorLayout.All();
        if (monitors.Count == 0) return;
        var tiles = new List<(string Peer, string Id, string Name, double X, double Y, double Width, double Height, BitmapImage? Wallpaper)>();
        // 画布统一用逻辑点：远端屏幕的 frame_x 和尺寸本来就来自服务端的逻辑坐标。
        // 本机显示器是物理像素枚举的，150% 的 4K 屏会被画成 1.5 倍大，
        // 旁边 HiDPI 的 1080p Mac 就显得巨小。这里按各自缩放比换算。
        var local = monitors
            .Select(m => new
            {
                Monitor = m,
                X = m.Left / m.Scale,
                Y = m.Top / m.Scale,
                Width = Math.Ceiling(m.Width / m.Scale),
                Height = Math.Ceiling(m.Height / m.Scale),
            })
            .ToList();
        double originX = local.Min(m => m.X), originY = local.Min(m => m.Y);
        foreach (var monitor in local)
            tiles.Add(("", monitor.Monitor.DeviceName, latest?.String("local_name", "本机") ?? "本机",
                monitor.X - originX, monitor.Y - originY, monitor.Width, monitor.Height,
                selectedPage == WorkspacePage.Input ? displayWallpapers.Local(monitor.Monitor) : null));
        // 未摆放过的远端屏幕紧贴本机组摆放；留 96 的空隙会让它看起来是浮着的。
        double fallbackX = tiles.Max(t => t.X + t.Width);
        var layoutSettings = JsonProperty(input, "settings");
        var wallpapers = JsonProperty(input, "remote_wallpapers");
        foreach (var remote in input.Array("remote_topologies"))
        {
            string peer = remote.String("peer_id");
            string name = latest?.Array("peers").FirstOrDefault(p => p.String("peer_id") == peer).String("display_name", "远端") ?? "远端";
            var displays = JsonProperty(remote, "topology").Array("displays").ToArray();
            double minX = displays.Length == 0 ? 0 : displays.Min(d => LayoutNumber(d, "frame_x"));
            double minY = displays.Length == 0 ? 0 : displays.Min(d => LayoutNumber(d, "frame_y"));
            foreach (var display in displays)
            {
                string id = display.String("id");
                var placement = InputLayout.Placement(layoutSettings, peer, id);
                double x = placement?.X ?? fallbackX + LayoutNumber(display, "frame_x") - minX;
                double y = placement?.Y ?? LayoutNumber(display, "frame_y") - minY;
                double width = display.Number("width"), height = display.Number("height");
                if (width <= 0 || height <= 0) continue;
                string encoded = JsonProperty(wallpapers, peer).String(id);
                tiles.Add((peer, id, name, x, y, width, height,
                    selectedPage == WorkspacePage.Input ? displayWallpapers.Remote(peer, id, encoded) : null));
            }
            fallbackX = tiles.Max(t => t.X + t.Width);
        }
        double left = tiles.Min(t => t.X), top = tiles.Min(t => t.Y);
        double contentWidth = tiles.Max(t => t.X + t.Width) - left, contentHeight = tiles.Max(t => t.Y + t.Height) - top;
        // 四周留出 30% 余量：画面正好铺满画布时，最左/最上的屏幕贴着边缘，
        // 用户根本没有空间把它拖到别的屏幕另一侧。
        const double DragMargin = 0.3;
        double totalWidth = contentWidth * (1 + 2 * DragMargin), totalHeight = contentHeight * (1 + 2 * DragMargin);
        double scale = Math.Max(0.001, Math.Min((DisplayCanvas.ActualWidth - 48) / totalWidth, (DisplayCanvas.ActualHeight - 48) / totalHeight));
        double offsetX = (DisplayCanvas.ActualWidth - totalWidth * scale) / 2 + contentWidth * DragMargin * scale;
        double offsetY = (DisplayCanvas.ActualHeight - totalHeight * scale) / 2 + contentHeight * DragMargin * scale;
        layoutScale = scale; layoutOffsetX = offsetX; layoutOffsetY = offsetY;
        layoutOriginX = left; layoutOriginY = top;
        layoutRects.Clear();
        foreach (var tile in tiles)
        {
            var surface = new Grid { Background = DfUi.Brush("DFControlStrongBrush") };
            if (tile.Wallpaper is not null) surface.Children.Add(new Image { Source = tile.Wallpaper, Stretch = Stretch.UniformToFill });
            var caption = new Border
            {
                Background = new SolidColorBrush(global::Windows.UI.Color.FromArgb(165, 0, 0, 0)),
                Padding = new Thickness(10, 5, 10, 5), VerticalAlignment = VerticalAlignment.Bottom,
                Child = new TextBlock { Text = tile.Name + (tile.Peer.Length == 0 ? " · 本机" : ""), FontSize = 12,
                    Foreground = new SolidColorBrush(Colors.White), TextTrimming = TextTrimming.CharacterEllipsis },
            };
            surface.Children.Add(caption);
            var rectangle = new Border
            {
                Width = tile.Width * scale, Height = tile.Height * scale,
                BorderBrush = DfUi.Line, BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(6),
                Child = surface,
            };
            ToolTipService.SetToolTip(rectangle, $"{tile.Name} · {tile.Width} × {tile.Height} px");
            if (tile.Peer.Length > 0)
            {
                rectangle.Tag = new RemoteDisplayDrag(tile.Peer, tile.Id, tile.X, tile.Y, scale, tile.Width, tile.Height);
                rectangle.PointerPressed += DisplayDragStarted;
                rectangle.PointerMoved += DisplayDragMoved;
                rectangle.PointerReleased += DisplayDragFinished;
                rectangle.PointerCanceled += DisplayDragCanceled;
                rectangle.PointerCaptureLost += DisplayDragCanceled;
                var menu = new MenuFlyout();
                var reconnect = new MenuFlyoutItem { Text = "重新连接" };
                reconnect.Click += async (_, _) =>
                {
                    try { await service.SendAsync(ServiceClient.Command("RestartInputControl", new { peer_id = tile.Peer })); }
                    catch (Exception ex) { Notify(ex is IOException ? ex.Message : "重新连接请求未完成。"); }
                };
                var remove = new MenuFlyoutItem { Text = "移除此设备的屏幕" };
                remove.Click += async (_, _) => await RemoveInputParticipantAsync(tile.Peer);
                menu.Items.Add(reconnect); menu.Items.Add(remove);
                rectangle.ContextFlyout = menu;
            }
            double canvasLeft = offsetX + (tile.X - left) * scale;
            double canvasTop = offsetY + (tile.Y - top) * scale;
            layoutRects.Add(new(tile.Peer, tile.Id, tile.X, tile.Y, tile.Width, tile.Height));
            Canvas.SetLeft(rectangle, canvasLeft);
            Canvas.SetTop(rectangle, canvasTop);
            DisplayCanvas.Children.Add(rectangle);
        }
        InputLayoutSummary.Text = $"{tiles.Count} 个屏幕";
    }

    private static double LayoutNumber(JsonElement value, string name)
    {
        var field = JsonProperty(value, name);
        return field.ValueKind == JsonValueKind.Number && field.TryGetDouble(out double number) && double.IsFinite(number) ? number : 0;
    }

    private static JsonElement JsonProperty(JsonElement value, string name) =>
        value.ValueKind == JsonValueKind.Object && value.TryGetProperty(name, out var property) ? property : default;

    private sealed record RemoteDisplayDrag(
        string DeviceId, string DisplayId, double X, double Y, double Scale, double Width, double Height);

    private void DisplayDragStarted(object sender, PointerRoutedEventArgs args)
    {
        if (sender is not Border border || border.Tag is not RemoteDisplayDrag || pendingLayoutMove is not null) return;
        if (!args.GetCurrentPoint(DisplayCanvas).Properties.IsLeftButtonPressed) return;
        if (!TryInputSettings(out var settings) || settings.String("role") != "Controller" || !settings.Bool("enabled"))
        {
            Notify("请先开启「此电脑作为主控」，再调整屏幕布局。");
            return;
        }
        if (!border.CapturePointer(args.Pointer)) return;
        draggingDisplay = border;
        dragOrigin = args.GetCurrentPoint(DisplayCanvas).Position;
        dragTranslation = (0, 0);
        args.Handled = true;
    }

    private void PreviewDisplayDrag(Border border, global::Windows.Foundation.Point position)
    {
        if (border.Tag is not RemoteDisplayDrag drag) return;
        // Absolute displacement from pointer-down, never from the snapped tile.
        dragTranslation = InputLayout.SnapMove(layoutRects, drag.DeviceId,
            (position.X - dragOrigin.X) / layoutScale, (position.Y - dragOrigin.Y) / layoutScale);
        foreach (var tile in DisplayCanvas.Children.OfType<Border>())
        {
            if (tile.Tag is not RemoteDisplayDrag member || member.DeviceId != drag.DeviceId) continue;
            Canvas.SetLeft(tile, layoutOffsetX + (member.X + dragTranslation.X - layoutOriginX) * layoutScale);
            Canvas.SetTop(tile, layoutOffsetY + (member.Y + dragTranslation.Y - layoutOriginY) * layoutScale);
        }
    }

    private void DisplayDragMoved(object sender, PointerRoutedEventArgs args)
    {
        if (draggingDisplay is not Border border || !ReferenceEquals(border, sender)) return;
        PreviewDisplayDrag(border, args.GetCurrentPoint(DisplayCanvas).Position);
        args.Handled = true;
    }

    private void DisplayDragCanceled(object sender, PointerRoutedEventArgs args)
    {
        if (!ReferenceEquals(draggingDisplay, sender)) return;
        draggingDisplay = null;
        DrawInputLayout(inputSnapshotValid ? inputSnapshot : default);
    }

    private async void DisplayDragFinished(object sender, PointerRoutedEventArgs args)
    {
        if (sender is not Border border || border.Tag is not RemoteDisplayDrag drag || !ReferenceEquals(draggingDisplay, border)) return;
        PreviewDisplayDrag(border, args.GetCurrentPoint(DisplayCanvas).Position);
        draggingDisplay = null;
        border.ReleasePointerCapture(args.Pointer);
        args.Handled = true;
        if (Math.Abs(dragTranslation.X) < 0.001 && Math.Abs(dragTranslation.Y) < 0.001)
        {
            DrawInputLayout(inputSnapshotValid ? inputSnapshot : default);
            return;
        }
        var pending = new PendingLayoutMove(drag.DeviceId, layoutRects.ToArray(), dragTranslation.X, dragTranslation.Y);
        pendingLayoutMove = pending;
        double x = drag.X + pending.X, y = drag.Y + pending.Y;
        Notify("正在保存屏幕布局…");
        try
        {
            await service.SendAsync(ServiceClient.Command("MoveControlDevice", new { device_id = drag.DeviceId, display_id = drag.DisplayId, x, y }));
            // The command response means the runtime accepted and persisted the
            // placement. Keep the preview only until the next heartbeat confirms
            // it; this short watchdog is a fallback, not a UI save delay.
            DrawInputLayout(inputSnapshotValid ? inputSnapshot : default);
            Notify("布局已保存，正在同步显示状态。");
            await Task.Delay(TimeSpan.FromMilliseconds(1500));
            if (!ReferenceEquals(pendingLayoutMove, pending)) return;
            pendingLayoutMove = null;
            DrawInputLayout(inputSnapshotValid ? inputSnapshot : default);
        }
        catch (Exception ex)
        {
            if (!ReferenceEquals(pendingLayoutMove, pending)) return;
            pendingLayoutMove = null;
            Notify(ex is IOException ? ex.Message : "布局未保存，服务可能已断开。");
            DrawInputLayout(inputSnapshotValid ? inputSnapshot : default);
        }
    }

    private async void ReclaimInputClick(object sender, RoutedEventArgs args)
    {
        try
        {
            await service.SendAsync("ReclaimInput");
            Notify("已请求停止被控并释放远端按键、鼠标与控制租约。");
        }
        catch (Exception ex) { Notify(ex is IOException ? ex.Message : "停止被控请求未完成。"); }
    }

    private async void AddInputDeviceClick(object sender, RoutedEventArgs args)
    {
        if (!TryInputSettings(out _) || latest is not JsonElement snapshot)
        {
            Notify("输入运行时尚未就绪，无法添加设备。");
            return;
        }
        var panel = new StackPanel { Spacing = 12 };
        panel.Children.Add(DfUi.Detail("选择已配对的电脑，添加它的全部屏幕。屏幕尺寸由对端提供，连接就绪后才能跨屏操作。"));
        var rows = new StackPanel { Spacing = 8 };
        panel.Children.Add(new ScrollViewer { Content = rows, MaxHeight = 320 });
        var hint = DfUi.Detail("");
        panel.Children.Add(hint);
        var pending = new Dictionary<string, (bool Included, DateTime Deadline)>();
        var controls = new Dictionary<string, (TextBlock Status, Button Action)>();
        void Refresh(JsonElement state)
        {
            var settings = JsonProperty(JsonProperty(state, "input"), "settings");
            var included = settings.Array("participants").Select(p => p.GetString() ?? "").ToHashSet();
            var activations = JsonProperty(state, "input").Array("activations").ToArray();
            foreach (var peer in state.Array("peers").Where(p => p.String("trust") == "trusted"))
            {
                string id = peer.String("peer_id");
                if (!controls.TryGetValue(id, out var control))
                {
                    var row = new Grid { ColumnSpacing = 12, Padding = new Thickness(0, 8, 0, 8) };
                    row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
                    row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
                    var body = new StackPanel { Spacing = 4 };
                    body.Children.Add(DfUi.Text(peer.String("display_name", "未命名设备"), 13, true));
                    var status = DfUi.Detail(""); body.Children.Add(status); row.Children.Add(body);
                    var action = DfUi.Secondary("添加", async (_, _) =>
                    {
                        if (!TryInputSettings(out var current) || pending.ContainsKey(id)) return;
                        var members = current.Array("participants").Select(p => p.GetString() ?? "").Where(p => p.Length > 0).ToHashSet();
                        bool add = !members.Contains(id);
                        if (add) members.Add(id); else members.Remove(id);
                        pending[id] = (add, DateTime.UtcNow.AddSeconds(5));
                        if (latest is JsonElement before) Refresh(before);
                        if (!await SetInputParticipants(members)) pending.Remove(id);
                        if (latest is JsonElement after) Refresh(after);
                    });
                    Grid.SetColumn(action, 1); row.Children.Add(action); rows.Children.Add(row);
                    control = (status, action); controls[id] = control;
                }
                bool member = included.Contains(id);
                if (pending.TryGetValue(id, out var change) && (change.Included == member || change.Deadline <= DateTime.UtcNow))
                {
                    if (change.Included != member) hint.Text = "设备设置尚未生效，请稍后重试。";
                    pending.Remove(id);
                }
                string reason = activations.FirstOrDefault(a => a.String("peer_id") == id).String("reason");
                bool screensReady = JsonProperty(state, "input").Array("remote_topologies").Any(t => t.String("peer_id") == id);
                string progress = !member ? "在线" : !JsonProperty(state, "capabilities").Bool("input")
                    ? "本机输入组件未就绪" : screensReady ? "屏幕已连接" : "等待对端提供屏幕";
                control.Status.Text = pending.ContainsKey(id) ? "正在保存…" : (member ? "已添加 · " : "")
                    + (peer.String("connection") == "connected" ? progress : "离线")
                    + (reason.Length > 0 ? " · " + reason : "");
                control.Action.Content = member ? "移除" : "添加";
                control.Action.IsEnabled = !pending.ContainsKey(id);
            }
            var trusted = state.Array("peers").Where(p => p.String("trust") == "trusted").Select(p => p.String("peer_id")).ToHashSet();
            foreach (var (id, control) in controls.Where(pair => !trusted.Contains(pair.Key)))
            {
                control.Status.Text = "设备已不在已配对列表中";
                control.Action.IsEnabled = false;
            }
            if (controls.Count == 0) hint.Text = "先到设备页配对另一台电脑，再回来添加屏幕。";
        }
        Refresh(snapshot);
        var dialog = new ContentDialog
        {
            XamlRoot = Root.XamlRoot, Title = "添加设备屏幕", Content = panel,
            PrimaryButtonText = "完成", SecondaryButtonText = "前往设备页配对", DefaultButton = ContentDialogButton.Primary,
        };
        service.SnapshotChanged += Refresh;
        try
        {
            if (await dialog.ShowAsync() == ContentDialogResult.Secondary) ShowPage(WorkspacePage.Devices);
        }
        finally { service.SnapshotChanged -= Refresh; }
    }

    private async Task RemoveInputParticipantAsync(string peerId)
    {
        if (!TryInputSettings(out var settings)) return;
        var remaining = settings.Array("participants")
            .Where(value => value.ValueKind == JsonValueKind.String)
            .Select(value => value.GetString() ?? "")
            .Where(value => value.Length > 0 && value != peerId)
            .ToArray();
        await SetInputParticipants(remaining);
    }

    private async Task<bool> SetInputParticipants(IEnumerable<string> ids)
    {
        if (!TryInputSettings(out var settings)) return false;
        try
        {
            var node = JsonNode.Parse(settings.GetRawText())?.AsObject() ?? throw new FormatException();
            var array = new JsonArray();
            foreach (string id in ids.Distinct(StringComparer.Ordinal)) array.Add(id);
            node["participants"] = array;
            await service.SendAsync(ServiceClient.Command("SetInputSharingSettings", new { settings = node }));
            Notify("设备设置请求已提交；屏幕与激活状态以对端实际就绪情况为准。");
            return true;
        }
        catch (FormatException) { Notify("输入设置格式无效。"); }
        catch (Exception ex) { Notify(ex is IOException ? ex.Message : "参与者未保存，服务可能已断开。"); }
        return false;
    }

    private void BuildMouseBindings()
    {
        if (MouseBindingList is null) return;
        MouseBindingList.Children.Clear();
        var rows = new[]
        {
            ("\uE962", "左键"), ("\uE962", "右键"), ("\uE962", "中键 滚轮按下"),
            ("\uE962", "后退侧键（按钮 4）"), ("\uE962", "前进侧键（按钮 5）"),
        };
        for (int i = 0; i < rows.Length; i++)
        {
            var (glyph, title) = rows[i];
            var grid = new Grid { Padding = new Thickness(0, 8, 0, 8), ColumnSpacing = 10 };
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(24) });
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            grid.Children.Add(DfUi.Glyph(glyph, 15, DfUi.Muted));
            var label = DfUi.Text(title, 13);
            Grid.SetColumn(label, 1);
            grid.Children.Add(label);
            var action = DfUi.Text("保持原始功能", 12, false, DfUi.Muted, wrap: false);
            action.VerticalAlignment = VerticalAlignment.Center;
            Grid.SetColumn(action, 2);
            grid.Children.Add(action);
            MouseBindingList.Children.Add(grid);
            if (i < rows.Length - 1) MouseBindingList.Children.Add(DfUi.Divider(0, 0));
        }
    }

    // MARK: - Transfers

    private sealed record TransferRow(string Id, string Title, string Detail, string State, double Progress,
        bool CanReceive, bool CanCancel, bool Outgoing, string Percent, bool CanOpen);

    private void RenderLanWeb(JsonElement snapshot)
    {
        if (LanWebContent is null || lanAccessToggle is null) return;
        var lan = snapshot.ValueKind == JsonValueKind.Object && snapshot.TryGetProperty("lan_web", out var value)
            ? value
            : default;
        bool available = lan.ValueKind == JsonValueKind.Object;
        bool enabled = available && lan.Bool("enabled");
        bool running = available && lan.Bool("running");
        updatingLanWeb = true;
        lanAccessToggle.IsOn = enabled;
        lanAccessToggle.IsEnabled = available && (FileTransferEnabled() || enabled);
        updatingLanWeb = false;
        LanWebContent.Children.Clear();
        if (!available)
        {
            LanWebContent.Children.Add(DfUi.Glyph("\uE8AB", 36, DfUi.Muted));
            LanWebContent.Children.Add(DfUi.Text("局域网服务状态不可用", 15, true, wrap: false));
            LanWebContent.Children.Add(DfUi.Detail("请更新服务和客户端到同一版本。"));
            return;
        }
        if (!enabled)
        {
            lanQrSignature = "";
            lanQrElement = null;
            LanWebContent.Children.Add(DfUi.Glyph("\uE8AB", 36, DfUi.Muted));
            LanWebContent.Children.Add(DfUi.Text("开启后扫码传输", 15, true, wrap: false));
            LanWebContent.Children.Add(DfUi.Detail("仅当前电脑所在局域网可访问，不会创建配对或信任。"));
            return;
        }
        if (!running)
        {
            lanQrSignature = "";
            lanQrElement = null;
            LanWebContent.Children.Add(DfUi.Glyph("\uE8AB", 36, DfUi.Muted));
            LanWebContent.Children.Add(DfUi.Text("局域网服务未运行", 15, true, wrap: false));
            string message = lan.String("error");
            if (message.Length == 0)
                message = lan.Bool("session_available") ? "正在监听局域网地址…" : "等待本机桌面解锁与服务连接。";
            LanWebContent.Children.Add(DfUi.Detail(message));
            return;
        }
        string address = lan.String("address");
        if (address.Length == 0)
        {
            LanWebContent.Children.Add(DfUi.Detail("监听已建立，但没有可供手机访问的局域网地址。"));
            return;
        }
        if (BuildLanQr(lan, address) is UIElement qr) LanWebContent.Children.Add(qr);
        var addressText = DfUi.Text(address, 14, true, DfUi.Accent, wrap: false);
        addressText.IsTextSelectionEnabled = true;
        LanWebContent.Children.Add(addressText);
        LanWebContent.Children.Add(DfUi.Detail("手机与这台电脑连接到同一局域网后扫码，即可发送文字或照片。"));
        var task = lan.Array("tasks").FirstOrDefault();
        if (task.ValueKind == JsonValueKind.Object && task.String("message") is string taskMessage && taskMessage.Length > 0)
            LanWebContent.Children.Add(DfUi.Detail($"最近上传：{taskMessage}"));
    }

    private UIElement? BuildLanQr(JsonElement lan, string address)
    {
        int size = (int)lan.Number("qr_size");
        string signature = $"{address}:{size}";
        if (lanQrElement is not null && signature == lanQrSignature) return lanQrElement;
        var modules = lan.Array("qr_modules").ToArray();
        if (size <= 0 || modules.Length != checked(size * size)) return null;
        var grid = new Grid { Width = 168, Height = 168, Background = new SolidColorBrush(Colors.White) };
        for (int row = 0; row < size; row++) grid.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        for (int column = 0; column < size; column++) grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        for (int row = 0; row < size; row++)
        {
            for (int column = 0; column < size; column++)
            {
                if (!modules[row * size + column].GetBoolean()) continue;
                var cell = new Border { Background = new SolidColorBrush(Colors.Black) };
                Grid.SetRow(cell, row);
                Grid.SetColumn(cell, column);
                grid.Children.Add(cell);
            }
        }
        var border = new Border
        {
            Background = new SolidColorBrush(Colors.White),
            BorderBrush = DfUi.Line,
            BorderThickness = new Thickness(1),
            CornerRadius = new CornerRadius(8),
            Padding = new Thickness(8),
            Child = grid,
        };
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(border, "局域网传输二维码");
        lanQrSignature = signature;
        lanQrElement = border;
        return border;
    }

    private void RenderTransfers(JsonElement snapshot)
    {
        if (TransferList is null || TransferSummary is null) return;
        TransferList.Children.Clear();
        var rows = new List<TransferRow>();
        foreach (var transfer in snapshot.Array("file_transfers"))
        {
            string id = transfer.String("transfer_id");
            if (id.Length == 0) continue;
            string direction = transfer.String("direction");
            string state = transfer.String("state");
            var names = transfer.Array("names").Where(v => v.ValueKind == JsonValueKind.String).Select(v => v.GetString() ?? "").ToArray();
            string title = names.Length == 1 ? names[0] : names.Length > 1 ? $"{names.Length} 个项目" : "文件传输";
            string peer = transfer.String("peer_id");
            string peerName = snapshot.Array("peers").FirstOrDefault(p => p.String("peer_id") == peer).String("display_name", peer);
            long total = transfer.Number("total_bytes");
            long sent = transfer.Number("transferred_bytes");
            string detail = transfer.String("detail");
            string summary = $"{(direction == "outgoing" ? "发送到" : "接收自")} {peerName} · {FormatBytes(total)}";
            if (detail.Length > 0) summary += $" · {detail}";
            double progress = total > 0 ? Math.Clamp((double)sent / total, 0, 1) : (state == "completed" ? 1 : 0);
            bool outgoing = direction == "outgoing";
            bool canOpen = state == "completed" && !outgoing && clipboard.TryGetReceivedPath(id, out _);
            rows.Add(new TransferRow(id, title, summary, StateLabel(state, outgoing), progress,
                !outgoing && state is "offered", !outgoing && state is "offered" or "requested" or "receiving",
                outgoing, total > 0 ? $"{progress * 100:0}% · {FormatBytes(sent)} / {FormatBytes(total)}" : StateLabel(state, outgoing),
                canOpen));
        }
        bool empty = rows.Count == 0;
        TransferEmptyState.Visibility = empty ? Visibility.Visible : Visibility.Collapsed;
        TransferList.Visibility = empty ? Visibility.Collapsed : Visibility.Visible;
        var cache = snapshot.TryGetProperty("file_cache", out var fileCache) ? fileCache : default;
        TransferSummary.Text = empty
            ? "在另一台电脑粘贴文件时会自动开始传输，也可以从暂存区立即发送。"
            : cache.ValueKind == JsonValueKind.Object
                ? $"共 {rows.Count} 项 · 服务缓存 {FormatBytes(cache.Number("used_bytes"))} / {FormatBytes(cache.Number("max_bytes"))}"
                : $"共 {rows.Count} 项";
        if (empty) return;
        for (int index = 0; index < rows.Count; index++)
        {
            TransferList.Children.Add(BuildTransferRow(rows[index]));
            if (index < rows.Count - 1) TransferList.Children.Add(DfUi.Divider(0, 0));
        }
    }

    private UIElement BuildTransferRow(TransferRow row)
    {
        var grid = new Grid { ColumnSpacing = 12, Padding = new Thickness(0, 12, 0, 12) };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        var iconColor = row.State is "失败" ? DfUi.Warning : row.Outgoing ? DfUi.Accent : DfUi.Success;
        var icon = new Border
        {
            Width = 40, Height = 40, CornerRadius = new CornerRadius(8),
            Background = SubtleTint(iconColor),
            Child = DfUi.Glyph(row.Outgoing ? "\uE898" : "\uE896", 18, iconColor),
        };
        grid.Children.Add(icon);
        var body = new StackPanel { Spacing = 3, VerticalAlignment = VerticalAlignment.Center };
        body.Children.Add(DfUi.Text(row.Title, 13, true, DfUi.PrimaryText, wrap: false));
        body.Children.Add(DfUi.Detail(row.Detail));
        var progressRow = new Grid { ColumnSpacing = 8, Margin = new Thickness(0, 5, 0, 0) };
        progressRow.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        progressRow.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        progressRow.Children.Add(new ProgressBar { Value = row.Progress * 100, Height = 3, VerticalAlignment = VerticalAlignment.Center });
        var percent = DfUi.Detail(row.Percent);
        percent.VerticalAlignment = VerticalAlignment.Center;
        Grid.SetColumn(percent, 1);
        progressRow.Children.Add(percent);
        body.Children.Add(progressRow);
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4, Margin = new Thickness(0, 2, 0, 0) };
        actions.Children.Add(DfUi.Text(row.State, 11, true, row.State is "已完成" ? DfUi.Success : DfUi.Muted, wrap: false));
        if (row.CanReceive)
            actions.Children.Add(DfUi.Plain("接收", async (_, _) => await ReceiveAsync(row.Id)));
        if (row.CanCancel)
            actions.Children.Add(DfUi.Plain("取消", async (_, _) => { await clipboard.CancelFileAsync(row.Id); Notify("已请求取消该文件传输。"); }));
        if (row.CanOpen)
            actions.Children.Add(DfUi.Plain("打开", (_, _) => OpenReceived(row.Id)));
        body.Children.Add(actions);
        Grid.SetColumn(body, 1);
        grid.Children.Add(body);
        return grid;
    }

    private async Task ReceiveAsync(string transferId)
    {
        if (!clipboard.FilesEnabled) { Notify("跨设备文件传输尚未就绪；请确认对端也是新版本。"); return; }
        string folder = string.IsNullOrWhiteSpace(clipboard.DownloadDirectory)
            ? DefaultReceiveFolder()
            : clipboard.DownloadDirectory;
        bool requested = await clipboard.ReceiveFileAsync(transferId, folder);
        Notify(requested
            ? $"已请求文件内容；完成后会保存到 {folder} 并放入剪贴板。"
            : "接收请求未完成。");
    }

    private void OpenReceived(string transferId)
    {
        if (!clipboard.TryGetReceivedPath(transferId, out string path))
        {
            Notify("还没记录到接收文件的位置。");
            return;
        }
        try { Process.Start(new ProcessStartInfo(path) { UseShellExecute = true }); }
        catch (Exception) { Notify("无法打开接收的文件。"); }
    }

    private static string StateLabel(string state, bool outgoing = false) => state switch
    {
        "offered" => outgoing ? "等待对方接收" : "等待接收",
        "requested" => "已请求内容",
        "preparing" => outgoing ? "准备发送" : "准备接收",
        "receiving" => "接收中",
        "verifying" => "校验中",
        "staged" => "等待保存",
        "sending" => "发送中",
        "completed" => "已完成",
        "failed" => "失败",
        "cancelled" => "已取消",
        _ => state,
    };

    internal static string DefaultReceiveFolder()
    {
        string profile = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);
        string downloads = Path.Combine(profile, "Downloads");
        return Path.Combine(Directory.Exists(downloads) ? downloads : profile, "DeskFuse");
    }

    // MARK: - Workflows

    private async Task LoadWorkflowsAsync()
    {
        if (!service.IsConnected || WorkflowList is null) return;
        try
        {
            var result = await service.SendAsync("GetWorkflows");
            if (!result.TryGetProperty("Workflows", out var catalog) || !catalog.TryGetProperty("workflows", out var values) || values.ValueKind != JsonValueKind.Array) return;
            var loaded = JsonSerializer.Deserialize<List<WorkflowRecord>>(values.GetRawText()) ?? [];
            workflows.Clear(); workflows.AddRange(loaded);
            InstallWorkflowHotkeys();
            WorkflowList.ItemsSource = null; WorkflowList.ItemsSource = workflows;
            if (editingWorkflow is not null) WorkflowList.SelectedItem = workflows.FirstOrDefault(w => w.Id == editingWorkflow.Id);
        }
        catch (Exception ex) { WorkflowNotice.Text = ex is IOException ? ex.Message : "工作流目录暂时不可用。"; }
        await RefreshWorkflowCatalogAsync();
    }

    private async void RefreshWorkflowCatalogClick(object sender, RoutedEventArgs args) => await RefreshWorkflowCatalogAsync();

    private async Task RefreshWorkflowCatalogAsync()
    {
        if (!service.IsConnected)
        {
            WorkflowSyncStatus.Text = "服务未连接，工作流同步暂不可用。";
            WorkflowConflictList.Children.Clear();
            return;
        }
        try
        {
            var reply = await service.SendAsync("GetWorkflowCatalog");
            if (!reply.TryGetProperty("WorkflowCatalog", out var envelope)
                || !envelope.TryGetProperty("json", out var jsonValue)
                || jsonValue.ValueKind != JsonValueKind.String)
            {
                WorkflowSyncStatus.Text = "同步状态不可用。";
                return;
            }
            using var document = JsonDocument.Parse(jsonValue.GetString() ?? "{}");
            var root = document.RootElement;
            string status = root.TryGetProperty("sync_status", out var statusValue) ? statusValue.GetString() ?? "pending" : "pending";
            WorkflowSyncStatus.Text = status switch
            {
                "synced" => "工作流已在已配对设备间同步。",
                "pending" => "等待同步：设备连接后会自动交换工作流定义。",
                "waiting_for_devices" => "等待已配对设备连接。",
                "error" => "同步失败，已保留本地修改。",
                _ => $"同步状态：{status}",
            };
            WorkflowConflictList.Children.Clear();
            if (!root.TryGetProperty("conflicts", out var conflicts) || conflicts.ValueKind != JsonValueKind.Array) return;
            foreach (var conflict in conflicts.EnumerateArray())
            {
                string id = conflict.TryGetProperty("id", out var idValue) ? idValue.GetString() ?? "" : "";
                if (id.Length == 0) continue;
                string name = "工作流";
                if (conflict.TryGetProperty("record", out var record)
                    && record.TryGetProperty("definition", out var definition)
                    && definition.TryGetProperty("value", out var workflow)
                    && workflow.TryGetProperty("name", out var nameValue))
                    name = nameValue.GetString() ?? name;
                var body = new StackPanel { Spacing = 4 };
                body.Children.Add(DfUi.Text(name, 12, true));
                body.Children.Add(DfUi.Detail("其他设备有并发修改；恢复会覆盖当前版本。"));
                var restore = DfUi.Secondary("恢复保留版本");
                string conflictId = id;
                restore.Click += async (_, _) =>
                {
                    try
                    {
                        await service.SendAsync(ServiceClient.Command("RestoreWorkflowConflict", new { conflict_id = conflictId }));
                        await LoadWorkflowsAsync();
                    }
                    catch (Exception ex) { WorkflowNotice.Text = ex is IOException ? ex.Message : "恢复失败。"; }
                };
                body.Children.Add(restore);
                WorkflowConflictList.Children.Add(DfUi.Card(body, padding: 10, radius: 8, background: DfUi.Control));
            }
        }
        catch (Exception ex) { WorkflowSyncStatus.Text = ex is IOException ? ex.Message : "同步状态暂时不可用。"; }
    }

    private bool hotkeyHookInstalled;

    private static uint WorkflowModifiers(ulong value) =>
        ((value & 0x80000) != 0 ? ModAlt : 0) | ((value & 0x40000) != 0 ? ModControl : 0)
        | ((value & 0x20000) != 0 ? ModShift : 0) | ((value & 0x100000) != 0 ? ModWin : 0);

    private void InstallWorkflowHotkeys()
    {
        if (!loaded) return;
        RemoveHotkeys();
        if (!hotkeyHookInstalled)
        {
            WindowMessageHook.Attach(Handle, HotkeyWindowProc);
            hotkeyHookInstalled = true;
        }
        int hotkeyId = 0x5540;
        foreach (var workflow in workflows.Where(w => w.Enabled && w.Shortcut is not null))
        {
            var shortcut = workflow.Shortcut!;
            uint modifiers = WorkflowModifiers(shortcut.Modifiers);
            if (clipboardShortcut.Enabled && shortcut.KeyCode == clipboardShortcut.Key && modifiers == clipboardShortcut.Modifiers) continue;
            if (shortcut.KeyCode == 0x44 && modifiers == (ModControl | ModShift)) continue;
            if (modifiers == 0 || !RegisterHotKey(Handle, hotkeyId, modifiers, shortcut.KeyCode)) continue;
            hotkeys[hotkeyId++] = workflow;
        }
    }

    private void RemoveHotkeys()
    {
        foreach (var id in hotkeys.Keys.ToArray()) UnregisterHotKey(Handle, id);
        hotkeys.Clear();
    }

    private bool HotkeyWindowProc(uint message, IntPtr wParam, IntPtr lParam)
    {
        if (clipboardShortcutRecorder is not null) return false;
        if (message == WmHotKey && hotkeys.TryGetValue(wParam.ToInt32(), out var workflow))
        {
            _ = ExecuteWorkflowAsync(workflow);
            return true;
        }
        return false;
    }

    private sealed class WorkflowRunEntry
    {
        public required string Id { get; init; }
        public required string Name { get; init; }
        public string Phase { get; set; } = "running";
        public string Message { get; set; } = "正在处理剪贴板内容…";
        public string OutputMime { get; set; } = "";
        public long OutputBytes { get; set; }
    }

    private readonly List<WorkflowRunEntry> workflowRuns = [];

    private async void RunWorkflowClick(object sender, RoutedEventArgs args)
    {
        if (editingWorkflow is not null && workflows.Contains(editingWorkflow))
            await ExecuteWorkflowAsync(editingWorkflow);
    }

    private async Task ExecuteWorkflowAsync(WorkflowRecord workflow, ClipboardContent? source = null)
    {
        if (!service.IsConnected || latest is not JsonElement snapshot || !snapshot.TryGetProperty("capabilities", out var caps) || !caps.Bool("workflow_execution"))
        {
            Notify("工作流执行能力尚未就绪；没有向服务发送剪贴板内容。");
            return;
        }
        var run = new WorkflowRunEntry { Id = Guid.NewGuid().ToString(), Name = workflow.Name };
        EnterWorkflowRun(run);
        try
        {
            long epoch = snapshot.Number("session_epoch");
            // History-entry runs only produce a copy; the quick hotkey path still
            // honours a trailing paste step.
            bool paste = source is null && workflow.Steps.Count > 0 && workflow.Steps[^1].Kind == "Paste";
            // Capture the external foreground window before any DeskFuse UI can
            // take focus. A missing target is reported after the result is copied.
            IntPtr target = paste ? ClipboardPanelNative.CaptureExternalTarget() : IntPtr.Zero;
            var output = await workflowExecution.ExecuteAsync(workflow.Id, paste, target, epoch, DateTime.UtcNow.Ticks, CancellationToken.None, source);
            run.Phase = "completed";
            run.OutputMime = output.Mime;
            run.OutputBytes = output.SizeBytes;
            run.Message = paste ? "结果已写入剪贴板并发送粘贴。" : "结果已写入剪贴板。";
            Notify(paste ? $"已执行工作流并发送粘贴：{workflow.Name}" : $"已执行工作流：{workflow.Name}");
        }
        catch (OperationCanceledException)
        {
            run.Phase = "cancelled";
            run.Message = "执行已取消，剪贴板保持不变。";
        }
        catch (Exception ex)
        {
            run.Phase = "failed";
            run.Message = ex is IOException or InvalidOperationException ? ex.Message : "工作流未执行，剪贴板保持不变。";
            Notify(run.Message);
        }
        finally { RenderWorkflowRuns(); }
    }

    private void EnterWorkflowRun(WorkflowRunEntry run)
    {
        // Keep running jobs visible and the five most recent terminal results,
        // matching the macOS run list's bounded history.
        workflowRuns.Insert(0, run);
        var running = workflowRuns.Where(entry => entry.Phase == "running").ToList();
        var recent = workflowRuns.Where(entry => entry.Phase != "running").Take(5).ToList();
        workflowRuns.Clear();
        workflowRuns.AddRange(running.Concat(recent));
        RenderWorkflowRuns();
    }

    private void RenderWorkflowRuns()
    {
        WorkflowRunsHeader.Visibility = workflowRuns.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
        WorkflowRunsPanel.Children.Clear();
        foreach (var run in workflowRuns.Take(3))
        {
            var body = new StackPanel { Spacing = 3 };
            var header = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
            header.Children.Add(new FontIcon
            {
                Glyph = run.Phase switch
                {
                    "completed" => "\uE73E",
                    "cancelled" => "\uE711",
                    "running" => "\uE895",
                    _ => "\uE7BA",
                },
                FontSize = 11,
                Foreground = run.Phase == "failed" ? DfUi.Warning : DfUi.Muted,
                VerticalAlignment = VerticalAlignment.Center,
            });
            header.Children.Add(DfUi.Text(run.Phase switch
            {
                "running" => $"{run.Name} · 执行中",
                "completed" => $"{run.Name} · 已完成",
                "cancelled" => $"{run.Name} · 已取消",
                _ => $"{run.Name} · 失败",
            }, 12, true));
            body.Children.Add(header);
            if (run.Message.Length > 0)
                body.Children.Add(DfUi.Text(run.Message, 11, foreground: DfUi.Muted));
            if (run.Phase == "completed" && run.OutputBytes > 0)
                body.Children.Add(DfUi.Text(
                    $"{(run.OutputMime.StartsWith("image/", StringComparison.Ordinal) ? "图片结果" : "文字结果")} · {FormatBytes(run.OutputBytes)}",
                    11, foreground: DfUi.Muted));
            WorkflowRunsPanel.Children.Add(DfUi.Card(body, padding: 10, radius: 8, background: DfUi.Control));
        }
    }

    private void NewWorkflowClick(object sender, RoutedEventArgs args)
    {
        editingWorkflow = new WorkflowRecord { Name = "新工作流", Id = Guid.NewGuid().ToString() };
        WorkflowList.SelectedItem = null;
        RenderWorkflowEditor();
    }

    private void WorkflowSelectionChanged(object sender, SelectionChangedEventArgs args)
    {
        editingWorkflow = WorkflowList.SelectedItem as WorkflowRecord;
        RenderWorkflowEditor();
    }

    private bool updatingWorkflowEditor;
    private bool recordingShortcut;

    private void WorkflowInputTypeChanged(object sender, SelectionChangedEventArgs args)
    {
        if (updatingWorkflowEditor || editingWorkflow is null) return;
        if (WorkflowInputType.SelectedItem is ComboBoxItem { Tag: string kind })
            editingWorkflow.InputType = kind;
        RefreshWorkflowCompatibility();
    }

    private void RenderWorkflowEditor()
    {
        StopShortcutRecording();
        var workflow = editingWorkflow;
        bool enabled = workflow is not null;
        updatingWorkflowEditor = true;
        try
        {
            WorkflowName.IsEnabled = enabled; WorkflowEnabled.IsEnabled = enabled;
            WorkflowShortcut.IsEnabled = enabled; WorkflowShortcutRecord.IsEnabled = enabled;
            WorkflowInputType.IsEnabled = enabled;
            AddWorkflowStepButton.IsEnabled = enabled && workflow!.Steps.Count < 32;
            SaveWorkflowButton.IsEnabled = enabled;
            RunWorkflowButton.IsEnabled = workflow is not null && workflows.Contains(workflow);
            DeleteWorkflowButton.IsEnabled = workflow is not null && workflows.Contains(workflow);
            WorkflowStepList.Children.Clear();
            if (workflow is null)
            {
                WorkflowName.Text = ""; WorkflowEnabled.IsChecked = false; WorkflowShortcut.Text = "";
                WorkflowInputType.SelectedIndex = 0;
                WorkflowNotice.Text = "";
                WorkflowCompatibility.Visibility = Visibility.Collapsed;
                return;
            }
            WorkflowName.Text = workflow.Name; WorkflowEnabled.IsChecked = workflow.Enabled;
            WorkflowShortcut.Text = workflow.Shortcut is null ? "" : $"{workflow.Shortcut.KeyCode}, {workflow.Shortcut.Modifiers}";
            int selected = 0;
            for (int index = 0; index < WorkflowInputType.Items.Count; index++)
                if (WorkflowInputType.Items[index] is ComboBoxItem { Tag: string tag } && tag == workflow.InputType) selected = index;
            WorkflowInputType.SelectedIndex = selected;
            if (WorkflowInputType.SelectedItem is ComboBoxItem { Tag: string current }) workflow.InputType = current;
            for (int index = 0; index < workflow.Steps.Count; index++)
                WorkflowStepList.Children.Add(BuildWorkflowStepCard(workflow, index));
            WorkflowNotice.Text = "";
        }
        finally { updatingWorkflowEditor = false; }
        RefreshWorkflowCompatibility();
    }

    private static string WorkflowStepTitle(string kind) => kind switch
    {
        "TextReplace" => "文字替换",
        "TextTrim" => "去除首尾空白",
        "Compress" => "压缩图片",
        "Resize" => "调整尺寸",
        "Convert" => "转换格式",
        "Rename" => "重命名文件",
        "Zip" => "打包 ZIP",
        "SystemShortcut" => "系统快捷指令",
        "Paste" => "粘贴到当前应用",
        _ => "复制到剪贴板",
    };

    private static bool IsWorkflowOutputStep(WorkflowStepRecord step) => step.Kind is "Copy" or "Paste";

    private static string? WorkflowCompatibilityIssue(WorkflowRecord workflow)
    {
        if (workflow.Steps.Count is 0 or > 32) return "流程需要 1–32 个步骤。";
        if (!IsWorkflowOutputStep(workflow.Steps[^1])) return "流程必须以复制或粘贴步骤结束。";
        if (workflow.Steps.Take(workflow.Steps.Count - 1).Any(IsWorkflowOutputStep)) return "复制或粘贴只能作为最后一步。";
        bool text = workflow.Steps.Any(step => step.Kind is "TextReplace" or "TextTrim");
        bool image = workflow.Steps.Any(step => step.Kind is "Compress" or "Resize" or "Convert");
        if (text && image) return "文字步骤与图片步骤不能混用。";
        if (workflow.InputType == "Text" && image) return "接受文字时不能包含图片步骤。";
        if (workflow.InputType == "Image" && text) return "接受图片时不能包含文字步骤。";
        foreach (var step in workflow.Steps)
        {
            if (step.Kind is "Rename" or "Zip" or "SystemShortcut")
                return "Windows 不支持重命名、ZIP 或 macOS 快捷指令步骤；请移除后再保存。";
            if (step.Kind == "TextReplace" && step.Find.Length == 0) return "文字替换需要填写查找内容。";
            if (step.Kind == "Resize" && (step.MaxEdge is null) == (step.ScalePercent is null))
                return "尺寸步骤请选择最长边或比例中的一种。";
            if (step.Kind == "Convert" && step.Format == "Heic") return "Windows 不支持 HEIC 编码；请选择 PNG、JPEG 或 WebP。";
            if (step.Quality is < 1 or > 100) return "图片质量必须为 1–100。";
        }
        return null;
    }

    private void RefreshWorkflowCompatibility()
    {
        if (editingWorkflow is null) return;
        string? issue = WorkflowCompatibilityIssue(editingWorkflow);
        WorkflowCompatibility.Text = issue ?? "";
        WorkflowCompatibility.Visibility = issue is null ? Visibility.Collapsed : Visibility.Visible;
        SaveWorkflowButton.IsEnabled = issue is null;
    }

    private Border BuildWorkflowStepCard(WorkflowRecord workflow, int index)
    {
        var step = workflow.Steps[index];
        bool terminal = index == workflow.Steps.Count - 1;
        var body = new StackPanel { Spacing = 10 };
        var header = new Grid { ColumnSpacing = 8 };
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        header.Children.Add(new TextBlock
        {
            Text = $"{index + 1}",
            Foreground = DfUi.Muted,
            VerticalAlignment = VerticalAlignment.Center,
            MinWidth = 16,
        });
        var title = new TextBlock
        {
            Text = WorkflowStepTitle(step.Kind),
            FontWeight = Microsoft.UI.Text.FontWeights.SemiBold,
            VerticalAlignment = VerticalAlignment.Center,
            TextTrimming = TextTrimming.CharacterEllipsis,
        };
        Grid.SetColumn(title, 1); header.Children.Add(title);
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4, VerticalAlignment = VerticalAlignment.Center };
        if (!terminal)
        {
            var up = DfUi.IconButton("\uE70E", (_, _) => MoveWorkflowStep(index, -1), "上移");
            up.IsEnabled = index > 0;
            var down = DfUi.IconButton("\uE70D", (_, _) => MoveWorkflowStep(index, 1), "下移");
            down.IsEnabled = index + 1 < workflow.Steps.Count - 1;
            var remove = DfUi.IconButton("\uE738", (_, _) => RemoveWorkflowStep(index), "移除步骤");
            actions.Children.Add(up); actions.Children.Add(down); actions.Children.Add(remove);
        }
        Grid.SetColumn(actions, 2); header.Children.Add(actions);
        body.Children.Add(header);
        if (step.Kind is "Rename" or "Zip" or "SystemShortcut")
        {
            body.Children.Add(DfUi.Detail("Windows 不支持此步骤，保存前请移除。"));
        }
        else switch (step.Kind)
        {
            case "TextReplace":
                body.Children.Add(WorkflowField("查找", step.Find, value => step.Find = value));
                body.Children.Add(WorkflowField("替换为", step.Replacement, value => step.Replacement = value));
                body.Children.Add(DfUi.Detail("区分大小写，按原文匹配，不使用正则表达式。"));
                break;
            case "TextTrim":
                body.Children.Add(DfUi.Detail("仅去除整段文字首尾的空格、制表符和换行，保留段落内部格式。"));
                break;
            case "Compress":
            {
                body.Children.Add(WorkflowCombo("模式", [("质量", "quality"), ("目标体积", "target")],
                    step.TargetBytes is null ? "quality" : "target", value =>
                    {
                        step.TargetBytes = value == "target" ? 512_000 : null;
                        RenderWorkflowEditor();
                    }));
                if (step.TargetBytes is { } bytes)
                {
                    body.Children.Add(WorkflowNumber(bytes / 1024.0, 1, 131_072, value => step.TargetBytes = (ulong)Math.Round(value) * 1024, "最终输出上限（KiB）"));
                    body.Children.Add(DfUi.Detail("在后续转换完成后检查；无法达标时不会复制结果。多个上限取最严格值。"));
                }
                else
                {
                    body.Children.Add(WorkflowNumber(step.Quality, 1, 100, value => step.Quality = (byte)Math.Round(value), "质量"));
                }
                break;
            }
            case "Resize":
            {
                body.Children.Add(WorkflowCombo("方式", [("最长边", "edge"), ("比例", "scale")],
                    step.MaxEdge is null ? "scale" : "edge", value =>
                    {
                        step.MaxEdge = value == "edge" ? 2048 : null;
                        step.ScalePercent = value == "edge" ? null : 75;
                        RenderWorkflowEditor();
                    }));
                if (step.MaxEdge is { } edge)
                    body.Children.Add(WorkflowNumber(edge, 1, 16_384, value => step.MaxEdge = (uint)Math.Round(value), "最长边（px）"));
                else
                    body.Children.Add(WorkflowNumber(step.ScalePercent ?? 75, 1, 100, value => step.ScalePercent = (uint)Math.Round(value), "缩放比例（%）"));
                body.Children.Add(DfUi.Detail("保持比例，不放大图片。"));
                break;
            }
            case "Convert":
            {
                body.Children.Add(WorkflowCombo("格式", [("PNG", "Png"), ("JPEG", "Jpeg"), ("WebP", "Webp"), ("HEIC（Windows 不支持）", "Heic")],
                    step.Format, value => { step.Format = value; RenderWorkflowEditor(); }));
                if (step.Format != "Png")
                    body.Children.Add(WorkflowNumber(step.Quality, 1, 100, value => step.Quality = (byte)Math.Round(value), "质量"));
                body.Children.Add(DfUi.Detail("保持当前尺寸；透明区域在 JPEG 输出中不保留。"));
                break;
            }
            default:
                body.Children.Add(WorkflowCombo("最后执行", [("复制到剪贴板", "Copy"), ("粘贴到当前应用", "Paste")],
                    step.Kind, value => { step.Kind = value; RefreshWorkflowCompatibility(); }));
                body.Children.Add(DfUi.Detail("粘贴会等待快捷键释放后发送到原应用；目标不可用时保留复制结果。"));
                break;
        }
        return DfUi.Card(body, padding: 12, radius: 10, background: DfUi.Control);
    }

    private TextBox WorkflowField(string header, string value, Action<string> apply)
    {
        var box = new TextBox { Header = header, Text = value };
        box.TextChanged += (_, _) => { apply(box.Text); RefreshWorkflowCompatibility(); };
        return box;
    }

    private ComboBox WorkflowCombo(string header, (string Label, string Value)[] items, string selected, Action<string> apply)
    {
        var combo = new ComboBox { Header = header, MinWidth = 160, HorizontalAlignment = HorizontalAlignment.Left };
        foreach (var (label, value) in items)
            combo.Items.Add(new ComboBoxItem { Content = label, Tag = value });
        int index = Array.FindIndex(items, item => item.Value == selected);
        combo.SelectedIndex = index < 0 ? 0 : index;
        combo.SelectionChanged += (_, _) =>
        {
            if (updatingWorkflowEditor) return;
            if (combo.SelectedItem is ComboBoxItem { Tag: string value })
            {
                apply(value);
                RefreshWorkflowCompatibility();
            }
        };
        return combo;
    }

    private NumberBox WorkflowNumber(double value, double minimum, double maximum, Action<double> apply, string header)
    {
        var box = new NumberBox
        {
            Header = header,
            Value = value,
            Minimum = minimum,
            Maximum = maximum,
            SmallChange = 1,
            SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Compact,
            ValidationMode = NumberBoxValidationMode.Disabled,
            Width = 160,
            HorizontalAlignment = HorizontalAlignment.Left,
        };
        box.ValueChanged += (_, args) =>
        {
            if (double.IsNaN(args.NewValue)) return;
            apply(Math.Clamp(args.NewValue, minimum, maximum));
            RefreshWorkflowCompatibility();
        };
        return box;
    }

    private void AddWorkflowStepClick(object sender, RoutedEventArgs args)
    {
        if (editingWorkflow is null || sender is not MenuFlyoutItem { Tag: string kind }) return;
        var steps = editingWorkflow.Steps;
        if (steps.Count >= 32) { WorkflowNotice.Text = "流程最多 32 个步骤。"; return; }
        if (steps.Count == 0 || !IsWorkflowOutputStep(steps[^1])) steps.Add(WorkflowStepRecord.Copy());
        var step = kind switch
        {
            "Compress" => new WorkflowStepRecord { Kind = kind, Quality = 85 },
            "Resize" => new WorkflowStepRecord { Kind = kind, MaxEdge = 2048, ScalePercent = null },
            "Convert" => new WorkflowStepRecord { Kind = kind, Format = "Jpeg" },
            _ => new WorkflowStepRecord { Kind = kind },
        };
        steps.Insert(steps.Count - 1, step);
        RenderWorkflowEditor();
    }

    private void MoveWorkflowStep(int index, int offset)
    {
        if (editingWorkflow is null) return;
        var steps = editingWorkflow.Steps;
        int target = index + offset;
        if (index < 0 || index >= steps.Count - 1 || target < 0 || target >= steps.Count - 1) return;
        (steps[index], steps[target]) = (steps[target], steps[index]);
        RenderWorkflowEditor();
    }

    private void RemoveWorkflowStep(int index)
    {
        if (editingWorkflow is null) return;
        var steps = editingWorkflow.Steps;
        if (index < 0 || index >= steps.Count - 1) return;
        steps.RemoveAt(index);
        RenderWorkflowEditor();
    }

    private void RecordWorkflowShortcutClick(object sender, RoutedEventArgs args)
    {
        if (editingWorkflow is null) return;
        recordingShortcut = !recordingShortcut;
        WorkflowShortcutRecord.Content = recordingShortcut ? "请按快捷键…（Esc 取消）" : "录制…";
        if (recordingShortcut) WorkflowNotice.Text = "按下包含 Ctrl、Alt 或 Win 的组合键。";
    }

    private void StopShortcutRecording()
    {
        recordingShortcut = false;
        if (WorkflowShortcutRecord is not null) WorkflowShortcutRecord.Content = "录制…";
    }

    private bool ShortcutRecorderProc(uint message, IntPtr wParam, IntPtr lParam)
    {
        if (!recordingShortcut || message is not (0x0100 or 0x0104)) return false;
        int key = wParam.ToInt32();
        if (key is 0x10 or 0x11 or 0x12 or 0x5B or 0x5C) return true;
        if (key == 0x1B)
        {
            StopShortcutRecording();
            WorkflowNotice.Text = "已取消快捷键录制。";
            return true;
        }
        ulong modifiers = 0;
        if (IsDown(global::Windows.System.VirtualKey.Control)) modifiers |= 0x40000;
        if (IsDown(global::Windows.System.VirtualKey.Menu)) modifiers |= 0x80000;
        if (IsDown(global::Windows.System.VirtualKey.Shift)) modifiers |= 0x20000;
        if (IsDown(global::Windows.System.VirtualKey.LeftWindows) || IsDown(global::Windows.System.VirtualKey.RightWindows)) modifiers |= 0x100000;
        if ((modifiers & 0x1c0000) == 0)
        {
            WorkflowNotice.Text = "快捷键必须包含 Ctrl、Alt 或 Win。";
            return true;
        }
        if (key > 127)
        {
            WorkflowNotice.Text = "该按键不受支持，请使用字母、数字或 F1–F12。";
            return true;
        }
        WorkflowShortcut.Text = $"{key}, {modifiers}";
        StopShortcutRecording();
        WorkflowNotice.Text = "快捷键已记录；保存后生效。";
        return true;
    }

    private async void SaveWorkflowClick(object sender, RoutedEventArgs args)
    {
        if (editingWorkflow is null) return;
        if (string.IsNullOrWhiteSpace(WorkflowName.Text)) { WorkflowNotice.Text = "工作流名称不能为空。"; return; }
        try
        {
            ShortcutRecord? shortcut = null;
            if (!string.IsNullOrWhiteSpace(WorkflowShortcut.Text))
            {
                var values = WorkflowShortcut.Text.Split(',', StringSplitOptions.TrimEntries);
                if (values.Length != 2 || !ushort.TryParse(values[0], out var key) || !ulong.TryParse(values[1], out var modifiers)) throw new FormatException();
                shortcut = new ShortcutRecord { KeyCode = key, Modifiers = modifiers };
            }
            editingWorkflow.Name = WorkflowName.Text.Trim();
            editingWorkflow.Enabled = WorkflowEnabled.IsChecked == true;
            editingWorkflow.Shortcut = shortcut;
            if (WorkflowInputType.SelectedItem is ComboBoxItem { Tag: string inputType }) editingWorkflow.InputType = inputType;
            if (editingWorkflow.Steps.Count == 0) editingWorkflow.Steps = [WorkflowStepRecord.Copy()];
            if (!IsWorkflowOutputStep(editingWorkflow.Steps[^1])) editingWorkflow.Steps.Add(WorkflowStepRecord.Copy());
            if (WorkflowCompatibilityIssue(editingWorkflow) is string issue) { WorkflowNotice.Text = issue; return; }
            await service.SendAsync(ServiceClient.Command("SaveWorkflow", new { workflow = editingWorkflow }));
            if (!workflows.Contains(editingWorkflow)) workflows.Add(editingWorkflow);
            InstallWorkflowHotkeys();
            WorkflowList.ItemsSource = null; WorkflowList.ItemsSource = workflows;
            WorkflowList.SelectedItem = editingWorkflow;
            WorkflowNotice.Text = "已保存；服务端 Rust 模型校验并持久化。";
        }
        catch (FormatException) { WorkflowNotice.Text = "快捷键格式无效，请使用录制按钮或 key code, modifiers。"; }
        catch (Exception ex) { WorkflowNotice.Text = ex is IOException ? ex.Message : "工作流保存失败。"; }
    }

    private async void DeleteWorkflowClick(object sender, RoutedEventArgs args)
    {
        if (editingWorkflow is null || !workflows.Contains(editingWorkflow)) return;
        try
        {
            await service.SendAsync(ServiceClient.Command("DeleteWorkflow", new { id = editingWorkflow.Id }));
            workflows.Remove(editingWorkflow); editingWorkflow = null; InstallWorkflowHotkeys(); WorkflowList.ItemsSource = null; WorkflowList.ItemsSource = workflows; RenderWorkflowEditor();
        }
        catch (Exception ex) { WorkflowNotice.Text = ex is IOException ? ex.Message : "工作流删除失败。"; }
    }

    // MARK: - Devices

    private void DeviceSearchChanged(object sender, TextChangedEventArgs args) => RebuildDevices();

    private void RemoteDeviceSearchChanged(object sender, TextChangedEventArgs args) => RebuildRemoteDeviceList();

    private void RefreshDevicesClick(object sender, RoutedEventArgs args) => _ = RefreshDevicesAsync();

    private async Task RefreshDevicesAsync()
    {
        try
        {
            await service.SendAsync("RefreshPeers");
            Notify("已请求重新发现设备并刷新连接状态。");
        }
        catch (Exception ex) { Notify(ex is IOException ? ex.Message : "刷新设备失败，服务可能已断开。"); }
    }

    private void ManualPairClick(object sender, RoutedEventArgs args)
    {
        var panel = new StackPanel { Spacing = 10, Width = 280 };
        panel.Children.Add(DfUi.Text("手动输入 IPv4 地址", 13, true));
        panel.Children.Add(DfUi.Detail("配对仍需对方确认。"));
        panel.Children.Add(manualIp);
        pairIpButton = DfUi.Primary("配对", PairIpClick);
        pairIpButton.HorizontalAlignment = HorizontalAlignment.Right;
        panel.Children.Add(pairIpButton);
        var flyout = new Flyout
        {
            Content = new Border
            {
                Background = DfUi.Surface,
                Padding = new Thickness(16),
                CornerRadius = new CornerRadius(12),
                BorderBrush = DfUi.Line,
                BorderThickness = new Thickness(1),
                Child = panel,
            },
        };
        flyout.ShowAt(ManualPairButton);
    }

    private void RebuildDevices()
    {
        if (DevicesList is null) return;
        devices = [new("local", localName, "本机", latest is null ? "服务未连接" : "Windows · 后台已连接", true, default)];
        if (latest is JsonElement snapshot)
        {
            devices.AddRange(snapshot.Array("peers").Select(peer =>
            {
                string trust = peer.String("trust");
                string group = trust == "trusted" ? "已配对" : trust == "blocked" ? "已阻止" : "附近";
                string status = peer.String("connection") == "connected" ? "在线" : trust == "blocked" ? "已阻止" : "离线";
                string endpoint = EndpointForPeer(peer);
                string detail = endpoint.Length > 0 ? $"{status} · {endpoint}" : status;
                return new DeviceRow(peer.String("peer_id"), peer.String("display_name", "未命名设备"), group, detail, false, peer);
            }).OrderBy(row => row.Group == "已配对" ? 0 : row.Group == "附近" ? 1 : 2)
                .ThenBy(row => row.Name, StringComparer.CurrentCultureIgnoreCase));
        }
        string search = DeviceSearch.Text.Trim();
        var visible = devices.Where(d => search.Length == 0 || $"{d.Name} {d.Id} {d.Detail}".Contains(search, StringComparison.CurrentCultureIgnoreCase)).ToArray();
        if (selectedDeviceId is null || visible.All(d => d.Id != selectedDeviceId))
        {
            selectedDeviceId = visible.FirstOrDefault(d => d.Group == "已配对")?.Id
                ?? visible.FirstOrDefault(d => d.Group == "附近")?.Id
                ?? visible.FirstOrDefault(d => d.Group == "已阻止")?.Id
                ?? visible.FirstOrDefault(d => d.Local)?.Id;
        }
        DevicesList.Children.Clear();
        var local = visible.FirstOrDefault(d => d.Local);
        if (local is not null)
        {
            DevicesList.Children.Add(DeviceSectionHeader("本机"));
            DevicesList.Children.Add(BuildDeviceRow(local, false));
        }
        foreach (var group in new[] { "已配对", "附近", "已阻止" })
        {
            var rows = visible.Where(d => d.Group == group).ToArray();
            if (rows.Length == 0) continue;
            DevicesList.Children.Add(DeviceSectionHeader($"{group}（{rows.Length}）"));
            foreach (var row in rows) DevicesList.Children.Add(BuildDeviceRow(row, true));
        }
        if (visible.Length == 0)
        {
            var empty = DfUi.Detail(search.Length == 0 ? "尚未发现其他设备" : "没有匹配的设备");
            empty.Margin = new Thickness(12, 20, 12, 20);
            DevicesList.Children.Add(empty);
        }
        RebuildPairingHost();
        RebuildRemoteDeviceList();
        RenderDeviceDetails();
    }

    private void RebuildRemoteDeviceList()
    {
        if (RemoteDevicesList is null) return;
        string search = RemoteDeviceSearch.Text.Trim();
        var visible = devices.Where(d => !d.Local
            && (search.Length == 0 || $"{d.Name} {d.Id} {d.Detail}".Contains(search, StringComparison.CurrentCultureIgnoreCase)))
            .ToArray();
        if (remoteSelectedDeviceId is null || visible.All(d => d.Id != remoteSelectedDeviceId))
            remoteSelectedDeviceId = visible.FirstOrDefault(d => d.Group == "已配对")?.Id ?? visible.FirstOrDefault()?.Id;
        RemoteDevicesList.Children.Clear();
        foreach (var group in new[] { "已配对", "附近", "已阻止" })
        {
            var rows = visible.Where(d => d.Group == group).ToArray();
            if (rows.Length == 0) continue;
            RemoteDevicesList.Children.Add(DeviceSectionHeader($"{group}（{rows.Length}）"));
            foreach (var row in rows) RemoteDevicesList.Children.Add(BuildDeviceRow(row, false, remote: true));
        }
        if (visible.Length == 0)
        {
            var empty = DfUi.Detail(search.Length == 0 ? "尚未发现其他设备" : "没有匹配的设备");
            empty.Margin = new Thickness(12, 20, 12, 20);
            RemoteDevicesList.Children.Add(empty);
        }
    }

    private static TextBlock DeviceSectionHeader(string title)
    {
        var block = DfUi.Text(title, 13, true, DfUi.Muted, wrap: false);
        block.Margin = new Thickness(10, 10, 0, 4);
        return block;
    }

    private Border BuildDeviceRow(DeviceRow device, bool allowRemoteApp, bool remote = false)
    {
        bool selected = (remote ? remoteSelectedDeviceId : selectedDeviceId) == device.Id;
        var border = new Border
        {
            Padding = new Thickness(10, 6, 10, 6),
            CornerRadius = new CornerRadius(8),
            Background = selected ? DfUi.Brush("DFAccentSubtleBrush") : new SolidColorBrush(Colors.Transparent),
            BorderBrush = selected ? DfUi.Line : new SolidColorBrush(Colors.Transparent),
            BorderThickness = new Thickness(1),
        };
        var grid = new Grid { ColumnSpacing = 12 };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var icon = new Border
        {
            Width = 42, Height = 42, CornerRadius = new CornerRadius(10),
            Background = DfUi.Brush("DFControlBrush"),
            Child = DfUi.Glyph(DeviceGlyph(device.Name), 20, DfUi.Muted),
        };
        grid.Children.Add(icon);
        var body = new StackPanel { Spacing = 4, VerticalAlignment = VerticalAlignment.Center };
        body.Children.Add(DfUi.Text(device.Name, 14, true, DfUi.PrimaryText, wrap: false));
        if (device.Local)
        {
            body.Children.Add(DfUi.Text(device.Detail, 11, false, DfUi.Muted, wrap: false));
        }
        else
        {
            var statusRow = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
            bool online = device.Peer.String("connection") == "connected";
            statusRow.Children.Add(new Microsoft.UI.Xaml.Shapes.Ellipse
            {
                Width = 7, Height = 7,
                Fill = online ? DfUi.Success : new SolidColorBrush(Colors.Gray),
                VerticalAlignment = VerticalAlignment.Center,
            });
            statusRow.Children.Add(DfUi.Text(device.Detail, 11, false, DfUi.Muted, wrap: false));
            body.Children.Add(statusRow);
        }
        Grid.SetColumn(body, 1);
        grid.Children.Add(body);
        if (allowRemoteApp && device.Peer.ValueKind == JsonValueKind.Object
            && device.Peer.String("trust") == "trusted" && device.Peer.String("connection") == "connected")
        {
            var launch = DfUi.IconButton("\uE8A7", (_, _) => { selectedDeviceId = device.Id; RenderDeviceDetails(); ShowPage(WorkspacePage.RemoteApplications); }, "打开远程应用", 26, 12);
            launch.IsEnabled = false;
            ToolTipService.SetToolTip(launch, "远程应用 · 即将支持");
            Grid.SetColumn(launch, 2);
            grid.Children.Add(launch);
        }
        border.Child = grid;
        border.Tapped += (_, _) =>
        {
            if (remote) remoteSelectedDeviceId = device.Id; else selectedDeviceId = device.Id;
            RebuildDevices();
        };
        border.PointerEntered += (_, _) => { if (!selected) border.Background = DfUi.Control; };
        border.PointerExited += (_, _) => { if (!selected) border.Background = new SolidColorBrush(Colors.Transparent); };
        return border;
    }

    private void RebuildPairingHost()
    {
        if (PairingHost is null) return;
        PairingHost.Children.Clear();
        if (latest is not JsonElement snapshot) return;
        foreach (var invite in snapshot.Array("pending"))
        {
            string requestId = invite.String("request_id");
            bool incoming = invite.Bool("incoming");
            var card = new StackPanel { Spacing = 8 };
            var head = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 10 };
            head.Children.Add(new Border
            {
                Width = 40, Height = 40, CornerRadius = new CornerRadius(20),
                Background = DfUi.Brush("DFWarningSubtleBrush"),
                Child = DfUi.Glyph("\uE8FA", 18, DfUi.Warning),
            });
            var names = new StackPanel { Spacing = 3, VerticalAlignment = VerticalAlignment.Center };
            names.Children.Add(DfUi.Text(invite.String("display_name", "附近设备"), 14, true));
            var status = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 5 };
            status.Children.Add(new Microsoft.UI.Xaml.Shapes.Ellipse { Width = 5, Height = 5, Fill = DfUi.Warning, VerticalAlignment = VerticalAlignment.Center });
            status.Children.Add(DfUi.Text(incoming ? "请求加入设备网络" : "等待对端确认", 12, false, DfUi.Muted));
            names.Children.Add(status);
            head.Children.Add(names);
            card.Children.Add(head);
            if (incoming)
            {
                var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6, HorizontalAlignment = HorizontalAlignment.Right };
                actions.Children.Add(DfUi.Secondary("拒绝", async (_, _) => await RunCommand("RejectPairing", new { request_id = requestId })));
                actions.Children.Add(DfUi.Primary("允许", async (_, _) => await RunCommand("AcceptPairing", new { request_id = requestId })));
                card.Children.Add(actions);
            }
            PairingHost.Children.Add(new Border
            {
                Background = DfUi.Brush("DFWarningSubtleBrush"),
                BorderBrush = DfUi.Warning,
                BorderThickness = new Thickness(1),
                CornerRadius = new CornerRadius(12),
                Padding = new Thickness(12),
                Child = card,
            });
        }
    }

    private void RenderDeviceDetails()
    {
        if (DeviceDetails is null) return;
        DeviceDetails.Children.Clear();
        var device = devices.FirstOrDefault(d => d.Id == selectedDeviceId);
        if (device is null)
        {
            DeviceDetails.Children.Add(DfUi.EmptyState("\uE772", "选择设备", "从左侧列表选择设备查看连接与传输状态。"));
            return;
        }
        if (latest is JsonElement snapshot && snapshot.TryGetProperty("pairing_status", out var pairing) && pairing.ValueKind == JsonValueKind.String && pairing.GetString() is { Length: > 0 } pairingText)
            DeviceDetails.Children.Add(DfUi.Detail(pairingText));
        if (device.Local)
        {
            RenderLocalDetails(device);
            return;
        }
        RenderRemoteDetails(device);
    }

    private void RenderLocalDetails(DeviceRow device)
    {
        var header = new Grid { ColumnSpacing = 16 };
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        header.Children.Add(new Border
        {
            Width = 58, Height = 58, CornerRadius = new CornerRadius(14),
            Background = DfUi.Brush("DFControlBrush"),
            Child = DfUi.Glyph(DeviceGlyph(device.Name), 28, DfUi.Accent),
        });
        var info = new StackPanel { Spacing = 6, VerticalAlignment = VerticalAlignment.Center };
        info.Children.Add(DfUi.Text(device.Name, 15, true));
        var line = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        line.Children.Add(new Microsoft.UI.Xaml.Shapes.Ellipse
        {
            Width = 10, Height = 10,
            Fill = latest?.Bool("network_running") == true ? DfUi.Success : DfUi.Warning,
            VerticalAlignment = VerticalAlignment.Center,
        });
        line.Children.Add(DfUi.Text(latest?.Bool("network_running") == true ? "服务运行中" : "网络未就绪", 12, false, DfUi.Muted));
        info.Children.Add(line);
        info.Children.Add(DfUi.Detail("Windows · 本机"));
        Grid.SetColumn(info, 1);
        header.Children.Add(info);
        DeviceDetails.Children.Add(DfUi.Card(header, 20));

        var infoBody = new StackPanel { Spacing = 0 };
        infoBody.Children.Add(InfoRow("\uE7E8", "运行状态", latest?.Bool("network_running") == true ? "服务运行中" : "网络未就绪"));
        infoBody.Children.Add(DfUi.Divider(0, 0));
        infoBody.Children.Add(InfoRow("\uE962", "键鼠", InputFeatureEnabled() ? "键鼠共享已开启" : "键鼠共享未开启"));
        DeviceDetails.Children.Add(DfUi.Card(infoBody, 0));

        if (latest is JsonElement snapshot)
        {
            var facts = new StackPanel { Spacing = 0 };
            facts.Children.Add(DfUi.ValueRow("设备 ID", snapshot.String("local_device_id")));
            facts.Children.Add(DfUi.Divider(0, 0));
            facts.Children.Add(DfUi.ValueRow("服务版本", snapshot.String("package_version")));
            facts.Children.Add(DfUi.Divider(0, 0));
            facts.Children.Add(DfUi.ValueRow("控制端口", snapshot.Number("control_port").ToString()));
            DeviceDetails.Children.Add(DfUi.Card(facts, 16));
        }
        var note = DfUi.Detail("本机名称与 Windows 计算机名一致；修改计算机名后重启服务即会更新。");
        note.Margin = new Thickness(4, 0, 0, 0);
        DeviceDetails.Children.Add(note);
    }

    private static Grid InfoRow(string glyph, string title, string value)
    {
        var grid = new Grid { ColumnSpacing = 12, Padding = new Thickness(18, 14, 18, 14) };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(24) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(62) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.Children.Add(DfUi.Glyph(glyph, 15, DfUi.Muted));
        var label = DfUi.Text(title, 12);
        Grid.SetColumn(label, 1);
        grid.Children.Add(label);
        var text = DfUi.Text(string.IsNullOrEmpty(value) ? "暂无信息" : value, 12, false, DfUi.Muted);
        text.IsTextSelectionEnabled = true;
        Grid.SetColumn(text, 2);
        grid.Children.Add(text);
        return grid;
    }

    private void RenderRemoteDetails(DeviceRow device)
    {
        var peer = device.Peer;
        bool connected = peer.String("connection") == "connected";
        string trust = peer.String("trust");
        string active = "";
        if (connected && peer.TryGetProperty("selected_route", out var selectedRoute) && selectedRoute.ValueKind == JsonValueKind.String)
        {
            var route = peer.Array("routes").FirstOrDefault(r => r.String("id") == selectedRoute.GetString());
            if (route.ValueKind == JsonValueKind.Object && route.TryGetProperty("endpoint", out var endpoint)) active = EndpointLabel(endpoint);
        }
        var header = new Grid { ColumnSpacing = 16 };
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        header.Children.Add(new Border
        {
            Width = 58, Height = 58, CornerRadius = new CornerRadius(14),
            Background = DfUi.Brush("DFControlBrush"),
            Child = DfUi.Glyph(DeviceGlyph(device.Name), 28, DfUi.Accent),
        });
        var info = new StackPanel { Spacing = 6, VerticalAlignment = VerticalAlignment.Center };
        info.Children.Add(DfUi.Text(device.Name, 15, true, DfUi.PrimaryText, wrap: false));
        var status = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        status.Children.Add(new Microsoft.UI.Xaml.Shapes.Ellipse
        {
            Width = 10, Height = 10,
            Fill = connected ? DfUi.Success : new SolidColorBrush(Colors.Gray),
            VerticalAlignment = VerticalAlignment.Center,
        });
        status.Children.Add(DfUi.Text(connected ? "在线" : trust == "blocked" ? "已阻止" : "离线", 12, false, DfUi.Muted));
        if (active.Length > 0) status.Children.Add(DfUi.Text($"· {active}", 12, false, DfUi.Muted, wrap: false));
        info.Children.Add(status);
        info.Children.Add(DfUi.Detail(device.Detail));
        Grid.SetColumn(info, 1);
        header.Children.Add(info);
        var more = DfUi.IconButton("\uE712", (sender, _) =>
        {
            var flyout = new MenuFlyout();
            var open = new MenuFlyoutItem { Text = "远程应用 · 即将支持", IsEnabled = false };
            open.Click += (_, _) => ShowPage(WorkspacePage.RemoteApplications);
            flyout.Items.Add(open);
            flyout.Items.Add(new MenuFlyoutSeparator());
            var refresh = new MenuFlyoutItem { Text = "刷新连接" };
            refresh.Click += async (_, _) => await RefreshDevicesAsync();
            flyout.Items.Add(refresh);
            flyout.ShowAt((FrameworkElement)sender);
        }, "设备操作", 32);
        Grid.SetColumn(more, 2);
        header.Children.Add(more);
        DeviceDetails.Children.Add(DfUi.Card(header, 20));

        DeviceDetails.Children.Add(BuildRoutesCard(device));

        DeviceDetails.Children.Add(BuildTransferActivity());
        DeviceDetails.Children.Add(BuildOperationsCard(device));
        DeviceDetails.Children.Add(BuildInputCard(device));
    }

    private UIElement BuildRoutesCard(DeviceRow device)
    {
        var peer = device.Peer;
        var routes = peer.Array("routes").ToArray();
        string selectedRouteId = peer.String("selected_route");
        var body = new StackPanel { Spacing = 0 };
        var header = new Grid { Margin = new Thickness(16, 14, 16, 8) };
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var titleRow = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        titleRow.Children.Add(DfUi.Text("连接与路由", 13, true, DfUi.PrimaryText, wrap: false));
        titleRow.Children.Add(DfUi.Glyph("\uE946", 12, DfUi.Muted));
        titleRow.Children.Add(DfUi.Text($"{routes.Length} 条", 11, false, DfUi.Muted, wrap: false));
        header.Children.Add(titleRow);
        var refresh = DfUi.Plain("刷新", async (_, _) => await RefreshDevicesAsync());
        refresh.FontSize = 12;
        Grid.SetColumn(refresh, 1);
        header.Children.Add(refresh);
        body.Children.Add(header);
        if (routes.Length == 0)
        {
            var empty = DfUi.Detail("暂未发现连接地址。请确认对方应用正在运行后刷新，或通过 IP 添加设备。");
            empty.Margin = new Thickness(16, 8, 16, 16);
            body.Children.Add(empty);
        }
        else
        {
            var tableHeader = new Grid { Padding = new Thickness(16, 6, 16, 6), ColumnSpacing = 8 };
            tableHeader.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(24) });
            tableHeader.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            tableHeader.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(128) });
            tableHeader.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(94) });
            var addressHeader = DfUi.Text("地址", 11, true, DfUi.Muted, wrap: false);
            Grid.SetColumn(addressHeader, 1); tableHeader.Children.Add(addressHeader);
            var sourceHeader = DfUi.Text("发现来源", 11, true, DfUi.Muted, wrap: false);
            Grid.SetColumn(sourceHeader, 2); tableHeader.Children.Add(sourceHeader);
            var healthHeader = DfUi.Text("状态", 11, true, DfUi.Muted, wrap: false);
            Grid.SetColumn(healthHeader, 3); tableHeader.Children.Add(healthHeader);
            body.Children.Add(tableHeader);
            foreach (var route in routes)
            {
                string id = route.String("id");
                bool activeRoute = id == selectedRouteId;
                string endpoint = route.TryGetProperty("endpoint", out var endpointValue) ? EndpointLabel(endpointValue) : "地址未知";
                string routeSource = route.TryGetProperty("source", out var sourceValue) ? JsonLabel(sourceValue) : "来源未知";
                string path = route.String("path_kind", "未识别");
                string routeHealth = route.String("health", "unknown");
                var row = new Grid { Padding = new Thickness(16, 10, 16, 10), ColumnSpacing = 8 };
                row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(24) });
                row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
                row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(128) });
                row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(94) });
                row.Children.Add(new Border
                {
                    Width = 18, Height = 18, CornerRadius = new CornerRadius(9),
                    Background = activeRoute ? DfUi.Accent : DfUi.Brush("DFControlBrush"),
                    Child = activeRoute ? DfUi.Glyph("\uE73E", 9, DfUi.Brush("DFAccentForegroundBrush")) : null,
                    VerticalAlignment = VerticalAlignment.Center,
                });
                var address = DfUi.Text(endpoint, 12, false, DfUi.PrimaryText, wrap: false);
                address.FontFamily = new FontFamily("Cascadia Mono");
                address.IsTextSelectionEnabled = true;
                Grid.SetColumn(address, 1); row.Children.Add(address);
                var origin = DfUi.Text($"{routeSource}\n{path}", 11, false, DfUi.Muted, wrap: false);
                Grid.SetColumn(origin, 2); row.Children.Add(origin);
                var state = DfUi.Text(activeRoute ? "使用中" : HealthLabel(routeHealth), 11, true,
                    activeRoute ? DfUi.Success : DfUi.Muted, wrap: false);
                Grid.SetColumn(state, 3); row.Children.Add(state);
                body.Children.Add(row);
                body.Children.Add(DfUi.Divider(0, 0));
            }
        }
        var note = DfUi.Detail("自动模式按实测握手延迟选择最快可达线路；刷新会立即重测候选地址，不会中断当前连接。");
        note.Margin = new Thickness(16, 10, 16, 14);
        body.Children.Add(note);
        return DfUi.Card(body, 0);
    }

    private UIElement BuildTransferActivity()
    {
        var body = new StackPanel { Spacing = 8, Padding = new Thickness(16) };
        var header = new Grid();
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        header.Children.Add(DfUi.Text("传输活动", 13, true, DfUi.PrimaryText, wrap: false));
        var all = DfUi.Plain("查看全部", (_, _) => ShowPage(WorkspacePage.ShareAndTransfer));
        all.FontSize = 12;
        Grid.SetColumn(all, 1);
        header.Children.Add(all);
        body.Children.Add(header);
        var transfers = latest is JsonElement snapshot ? snapshot.Array("file_transfers").Take(3).ToArray() : [];
        if (transfers.Length == 0)
        {
            var empty = DfUi.Detail("暂无传输任务");
            empty.Margin = new Thickness(0, 6, 0, 6);
            body.Children.Add(empty);
        }
        else
        {
            foreach (var transfer in transfers)
            {
                var transferRow = new Grid { Padding = new Thickness(0, 8, 0, 8), ColumnSpacing = 10 };
                transferRow.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
                transferRow.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
                transferRow.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
                bool outgoing = transfer.String("direction") == "outgoing";
                transferRow.Children.Add(DfUi.Glyph(outgoing ? "\uE898" : "\uE896", 18, outgoing ? DfUi.Accent : DfUi.Success));
                var names = transfer.Array("names").Where(v => v.ValueKind == JsonValueKind.String).Select(v => v.GetString() ?? "").ToArray();
                var info = new StackPanel { Spacing = 2, VerticalAlignment = VerticalAlignment.Center };
                info.Children.Add(DfUi.Text(names.Length == 1 ? names[0] : names.Length > 1 ? $"{names.Length} 个项目" : "文件传输", 12, true, DfUi.PrimaryText, wrap: false));
                info.Children.Add(DfUi.Detail(StateLabel(transfer.String("state"))));
                Grid.SetColumn(info, 1);
                transferRow.Children.Add(info);
                var state = DfUi.Text(StateLabel(transfer.String("state")), 11, false, DfUi.Muted, wrap: false);
                state.VerticalAlignment = VerticalAlignment.Center;
                Grid.SetColumn(state, 2);
                transferRow.Children.Add(state);
                body.Children.Add(transferRow);
            }
        }
        return DfUi.Card(body, 0);
    }

    private UIElement BuildOperationsCard(DeviceRow device)
    {
        var peer = device.Peer;
        bool connected = peer.String("connection") == "connected";
        string trust = peer.String("trust");
        var body = new StackPanel { Spacing = 12, Padding = new Thickness(16) };
        body.Children.Add(DfUi.Text("操作", 13, true, DfUi.PrimaryText, wrap: false));
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        if (trust == "blocked")
        {
            actions.Children.Add(DfUi.Secondary("允许重新配对", async (_, _) =>
            {
                if (busy) return;
                busy = true;
                try { await service.AllowPairingAsync(device.Id); Notify("已请求解除阻止；以设备列表中的实际状态为准。"); }
                catch (Exception ex) { Notify(ex is IOException ? $"{ex.Message}（解除阻止需要匹配版本的服务）" : "操作未完成，服务可能已断开。"); }
                finally { busy = false; }
            }));
        }
        else if (trust != "trusted")
        {
            actions.Children.Add(DfUi.Primary("邀请配对", async (_, _) => await RunCommand("RequestPairing", new { peer_id = device.Id })));
        }
        else
        {
            if (connected)
                actions.Children.Add(DfUi.Secondary("断开连接", async (_, _) => await RunCommand("Disconnect", new { peer_id = device.Id })));
            else
                actions.Children.Add(DfUi.Secondary("重新连接", async (_, _) => await RunCommand("Connect", new { peer_id = device.Id })));
            actions.Children.Add(DfUi.Secondary("忘记设备", async (_, _) =>
            {
                if (!Dialogs.Confirm(Handle, $"忘记「{device.Name}」？\n\n该设备会从列表中移除；重新发现后可再次配对。", "忘记设备")) return;
                if (busy) return;
                busy = true;
                try { await service.ForgetAsync(device.Id); Notify("已请求忘记设备；以设备列表中的实际状态为准。"); }
                catch (Exception ex) { Notify(ex is IOException ? $"{ex.Message}（忘记设备需要匹配版本的服务）" : "操作未完成，服务可能已断开。"); }
                finally { busy = false; }
            }));
        }
        if (trust != "blocked")
        {
            var block = DfUi.Secondary("拉黑设备", async (_, _) =>
            {
                if (!Dialogs.Confirm(Handle, $"拉黑「{device.Name}」？\n\n这会断开连接并阻止该设备配对，直到你允许重新配对。", "拉黑设备")) return;
                await RunCommand("Revoke", new { peer_id = device.Id });
            });
            block.Foreground = DfUi.Danger;
            actions.Children.Add(block);
        }
        body.Children.Add(actions);
        return DfUi.Card(body, 0);
    }

    private UIElement BuildInputCard(DeviceRow device)
    {
        var peer = device.Peer;
        var input = latest is JsonElement snapshot && snapshot.TryGetProperty("input", out var value) ? value : default;
        var settings = input.ValueKind == JsonValueKind.Object && input.TryGetProperty("settings", out var sharing) ? sharing : default;
        bool inputEnabled = settings.ValueKind == JsonValueKind.Object && settings.Bool("enabled");
        bool isController = settings.ValueKind == JsonValueKind.Object && settings.String("role") == "Controller";
        bool isParticipant = settings.ValueKind == JsonValueKind.Object
            && settings.Array("participants").Any(p => p.ValueKind == JsonValueKind.String && p.GetString() == device.Id);
        var body = new StackPanel { Spacing = 10, Padding = new Thickness(16) };
        body.Children.Add(DfUi.Text("键鼠共享", 13, true, DfUi.PrimaryText, wrap: false));
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        if (inputEnabled && isController && isParticipant)
        {
            actions.Children.Add(DfUi.Secondary("停止主控此设备", async (_, _) => await RunCommand("DisableInputControl", new { })));
            var activation = input.Array("activations").FirstOrDefault(a => a.String("peer_id") == device.Id);
            string status = activation.ValueKind == JsonValueKind.Object ? activation.String("status") : "";
            string reason = activation.ValueKind == JsonValueKind.Object ? activation.String("reason") : "";
            body.Children.Add(DfUi.Detail(status.Length == 0
                ? "正在等待对端确认设备激活…把本机指针移到屏幕边缘即可开始主控。"
                : $"激活状态：{InputStatusLabel(status)}{(reason.Length > 0 ? $"\n{reason}" : "")}"));
        }
        else
        {
            actions.Children.Add(DfUi.Secondary("主控此设备", async (_, _) => await RunCommand("EnableInputControl", new { peer_id = device.Id })));
            body.Children.Add(DfUi.Detail("开启后本机指针移到屏幕边缘即可控制对端；对端仍需在设备激活中确认。"));
        }
        body.Children.Add(actions);
        return DfUi.Card(body, 0);
    }

    private static string InputStatusLabel(string status) => status switch
    {
        "Preparing" => "准备中",
        "Ready" => "对端已就绪",
        "PausedByUser" => "对端已暂停",
        "Unavailable" => "不可用",
        _ => status,
    };

    private static string DeviceGlyph(string name)
    {
        string lower = name.ToLowerInvariant();
        if (lower.Contains("iphone") || lower.Contains("ipad") || lower.Contains("phone") || lower.Contains("手机")) return "\uE8EA";
        if (lower.Contains("mac") || lower.Contains("book") || lower.Contains("laptop")) return "\uE7F8";
        return "\uE7F4";
    }

    private static string HealthLabel(string health) => health switch
    {
        "healthy" or "ready" or "ok" => "可用",
        "degraded" => "降级",
        "unreachable" => "不可达",
        _ => "未知",
    };

    private static string EndpointForPeer(JsonElement peer)
    {
        if (peer.TryGetProperty("selected_route", out var selected) && selected.ValueKind == JsonValueKind.String)
        {
            var route = peer.Array("routes").FirstOrDefault(r => r.String("id") == selected.GetString());
            if (route.ValueKind == JsonValueKind.Object && route.TryGetProperty("endpoint", out var endpoint)) return EndpointLabel(endpoint);
        }
        var first = peer.Array("routes").FirstOrDefault();
        return first.ValueKind == JsonValueKind.Object && first.TryGetProperty("endpoint", out var fallback) ? EndpointLabel(fallback) : "";
    }

    private async Task RunCommand(string command, object args)
    {
        if (busy) return;
        busy = true;
        if (pairIpButton is not null) pairIpButton.IsEnabled = false;
        try
        {
            var reply = await service.SendAsync(ServiceClient.Command(command, args));
            Notify(reply.TryGetProperty("Queued", out _) ? "操作已提交；等待服务和对端的真实状态更新。" : "服务已响应；以设备列表中的实际状态为准。");
        }
        catch (Exception ex) { Notify(ex is IOException ? ex.Message : "操作未完成，服务可能已断开或请求超时。"); }
        finally { busy = false; if (pairIpButton is not null) pairIpButton.IsEnabled = service.IsConnected; }
    }

    private async void PairIpClick(object sender, RoutedEventArgs args)
    {
        string ip = manualIp.Text.Trim();
        if (ip.Split('.').Length != 4 || !IPAddress.TryParse(ip, out var address) || address.AddressFamily != AddressFamily.InterNetwork || address.ToString() != ip)
        { Notify("请输入完整 IPv4 地址，例如 192.168.1.23。IP 仅用于定位，不授予信任。"); return; }
        await RunCommand("RequestPairingAt", new { ipv4 = ip });
    }

    private void DisplayCanvasSizeChanged(object sender, SizeChangedEventArgs args)
    {
        DisplayCanvas.Clip = new RectangleGeometry { Rect = new global::Windows.Foundation.Rect(0, 0, args.NewSize.Width, args.NewSize.Height) };
        DrawInputLayout(latest.HasValue && latest.Value.TryGetProperty("input", out var layoutInput) && layoutInput.ValueKind == JsonValueKind.Object ? layoutInput : default);
    }

    private static string EndpointLabel(JsonElement value)
    {
        if (value.ValueKind != JsonValueKind.Object) return JsonLabel(value);
        string host = value.String("host", "地址未知");
        if (host.Contains(':')) host = $"[{host}]";
        return $"{host}:{value.Number("port")}";
    }

    private static string JsonLabel(JsonElement value) => value.ValueKind == JsonValueKind.String ? value.GetString() ?? "" : value.GetRawText();

    private static string ConnectionLabel(string value) => value switch { "connected" => "已连接", "connecting" => "连接中", "disconnected" => "未连接", "backoff" => "等待重连", "failed" => "连接失败", _ => value.Length == 0 ? "未连接" : value };

    private static string TrustLabel(string value) => value switch { "trusted" => "已信任", "untrusted" or "unknown" => "未配对", "pairing" => "配对中", "pending" => "等待确认", "blocked" => "已阻止", _ => value.Length == 0 ? "未配对" : value };

    internal static string FormatBytes(long value) => value >= 1024 * 1024 ? $"{value / (1024.0 * 1024):N1} MiB" : value >= 1024 ? $"{value / 1024.0:N1} KiB" : $"{value} B";

    private void FileOfferArrived(string peer, string names) =>
        Notify($"{peer} 发来文件：{names}。在“局域网传输”页选择位置接收。");

    private void FileTransferDone(string id, string detail, bool success) =>
        Notify(success ? detail : $"文件传输未完成：{detail}");

    private void KeyboardShortcut(object sender, KeyRoutedEventArgs args)
    {
        bool control = IsDown(global::Windows.System.VirtualKey.Control);
        if (control && args.Key == global::Windows.System.VirtualKey.F)
        {
            if (selectedPage == WorkspacePage.Devices) DeviceSearch.Focus(FocusState.Programmatic);
            else { Navigate(WorkspacePage.Clipboard); HistorySearch.Focus(FocusState.Programmatic); }
            args.Handled = true;
        }
        else if (control && args.Key == (global::Windows.System.VirtualKey)0xBC /* OEM_COMMA */) { ShowSettings(); args.Handled = true; }
        else if (control && args.Key == global::Windows.System.VirtualKey.A && selectedPage == WorkspacePage.Clipboard
                 && FocusManager.GetFocusedElement(Root.XamlRoot) is not TextBox)
        {
            HistoryList.SelectAll();
            args.Handled = true;
        }
        else if (args.Key == global::Windows.System.VirtualKey.Enter && ReferenceEquals(FocusManager.GetFocusedElement(Root.XamlRoot), HistoryList)) { _ = CopySelectionAsync(); args.Handled = true; }
        else if (args.Key is global::Windows.System.VirtualKey.Delete or global::Windows.System.VirtualKey.Back && ReferenceEquals(FocusManager.GetFocusedElement(Root.XamlRoot), HistoryList))
        {
            DeleteSelectedClick(sender, args);
            args.Handled = true;
        }
    }

    private static bool IsDown(global::Windows.System.VirtualKey key) =>
        (global::Microsoft.UI.Input.InputKeyboardSource.GetKeyStateForCurrentThread(key) & global::Windows.UI.Core.CoreVirtualKeyStates.Down) != 0;

    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool RegisterHotKey(IntPtr hWnd, int id, uint modifiers, uint virtualKey);
    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool UnregisterHotKey(IntPtr hWnd, int id);

    private sealed record DeviceRow(string Id, string Name, string Group, string Detail, bool Local, JsonElement Peer);
}
