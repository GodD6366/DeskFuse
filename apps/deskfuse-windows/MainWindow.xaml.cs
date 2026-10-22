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
    private string historyKind = "";
    private readonly ClipboardLibrary library;
    private Button KindAllButton = null!;
    private Button KindTextButton = null!;
    private Button KindImageButton = null!;
    private Button KindFilesButton = null!;
    private string localName = "这台 Windows";
    private string lastSettingsCategory = "general";
    private string? selectedDeviceId;
    private string? remoteSelectedDeviceId;
    private string lastDeviceSnapshot = "";
    private JsonElement? latest;
    private bool busy;
    private bool updatingInputSettings;
    private readonly List<WorkflowRecord> workflows = [];
    private readonly List<string> stagedFiles = [];
    private string sendTargetSignature = "";
    private WorkflowRecord? editingWorkflow;
    private Border? draggingDisplay;
    private global::Windows.Foundation.Point dragOrigin;
    private readonly Dictionary<string, ClipboardContentEditorWindow> contentEditors = [];
    private readonly Dictionary<int, WorkflowRecord> hotkeys = [];
    private readonly TextBox manualIp = new() { PlaceholderText = "192.168.1.10" };
    private Button? pairIpButton;
    private DfToggle? featureToggle;
    private DfToggle? inputControllerToggle;
    private List<(string Tone, string Title, string Detail)> healthIssues = [];
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
        KindAllButton = KindChip("全部", "");
        KindTextButton = KindChip("文字", "text");
        KindImageButton = KindChip("图片", "image");
        KindFilesButton = KindChip("文件", "files");
        var kindChips = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 2, Margin = new Thickness(10, 0, 10, 4) };
        kindChips.Children.Add(KindAllButton);
        kindChips.Children.Add(KindTextButton);
        kindChips.Children.Add(KindImageButton);
        kindChips.Children.Add(KindFilesButton);
        LibraryListPanel.Children.Add(kindChips);
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
                RemoveHotkeys();
                return;
            }
            args.Cancel = true;
            HideToTray();
        };
        workflowExecution = new WindowsWorkflowExecutionPort(service, clipboard);
        clipboard.Entries.CollectionChanged += (_, _) => RefreshHistory();
        clipboard.Error += Notify;
        clipboard.FileOfferReceived += FileOfferArrived;
        clipboard.FileTransferSettled += FileTransferDone;
        service.SnapshotChanged += SnapshotChanged;
        service.ConnectionChanged += ConnectionChanged;
        Root.KeyDown += KeyboardShortcut;
        UpdateKindChips();
        RefreshHistory();
        BuildMouseBindings();
        inputControllerToggle = DfUi.Switch(false, InputControllerToggled, false);
        InputControllerToggleHost.Children.Add(inputControllerToggle);
        LanAccessToggleHost.Children.Add(DfUi.Switch(false, null, false));
        ConnectionChanged(false, "正在连接 DeskFuseService…");
        DrawInputLayout(default);
        Root.Loaded += (_, _) =>
        {
            loaded = true;
            ExtendsContentIntoTitleBar = true;
            SetTitleBar(TitleBarDragRegion);
            UpdateCaptionButtons();
            Root.ActualThemeChanged += (_, _) => UpdateCaptionButtons();
            InstallWorkflowHotkeys();
        };
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
        WorkspacePage.MouseButtons => "鼠标按键",
        _ => "DeskFuse",
    };

    private static string SectionFor(WorkspacePage page) => page == WorkspacePage.Devices ? "配置" : "功能";

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
        NavPanel.Children.Add(SectionHeader("功能"));
        AddNavRow(WorkspacePage.Clipboard, "剪贴板", "\uE8C8", false, null);
        AddNavRow(WorkspacePage.Input, "键鼠共享", "\uE962", false, null);
        AddNavRow(WorkspacePage.ShareAndTransfer, "局域网传输", "\uE8AB", false, null);
        AddNavRow(WorkspacePage.RemoteApplications, "远程应用", "\uE8A7", true, null);
        AddNavRow(WorkspacePage.Microphone, "麦克风共享", "\uE720", true, null);
        AddNavRow(WorkspacePage.MouseButtons, "鼠标按键", "\uE8B1", true, null);
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
            var badge = DfUi.Badge("Beta");
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

    private void ShowSettings(string category)
    {
        if (settings is null)
        {
            PageHost.Children.Remove(WorkflowsPage);
            WorkflowsPage.Visibility = Visibility.Collapsed;
            settings = new SettingsPage(clipboard, service, power, WorkflowsPage, () => _ = LoadWorkflowsAsync(),
                () => ShowPage(WorkspacePage.Input));
            SettingsHost.Children.Add(settings);
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
        DfToggle? toggle = selectedPage switch
        {
            WorkspacePage.Clipboard => DfUi.Switch(clipboard.SyncEnabled, ClipboardFeatureToggled),
            WorkspacePage.Input => DfUi.Switch(InputFeatureEnabled(), InputFeatureToggled, latest.HasValue),
            WorkspacePage.RemoteApplications => DfUi.Switch(remoteAppsEnabled, RemoteAppsToggled),
            WorkspacePage.ShareAndTransfer => DfUi.Switch(true, null, false),
            WorkspacePage.Microphone or WorkspacePage.MouseButtons => DfUi.Switch(false, null, false),
            _ => null,
        };
        if (toggle is null) return;
        string tooltip = selectedPage switch
        {
            WorkspacePage.Clipboard => clipboard.SyncEnabled ? "关闭剪贴板同步并停止相关后台任务" : "开启剪贴板同步",
            WorkspacePage.Input => InputFeatureEnabled() ? "关闭键鼠共享并停止相关后台任务" : "开启键鼠共享",
            WorkspacePage.RemoteApplications => remoteAppsEnabled ? "关闭远程应用" : "开启远程应用",
            WorkspacePage.ShareAndTransfer => "文件传输由服务自动管理",
            WorkspacePage.Microphone => "Windows 端暂不支持麦克风共享",
            WorkspacePage.MouseButtons => "Windows 端暂不支持鼠标按键自定义",
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
                }
            }
            if (inputControllerToggle is not null)
            {
                inputControllerToggle.IsOn = InputControllerEnabled();
                inputControllerToggle.IsEnabled = latest.HasValue && InputFeatureEnabled();
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
            node["enabled"] = toggle.IsOn;
            node["role"] = toggle.IsOn ? "Controller" : "Disabled";
            await service.SendAsync(ServiceClient.Command("SetInputSharingSettings", new { settings = node }));
            Notify(toggle.IsOn ? "已开启主控；把指针移到屏幕边缘即可控制已添加设备。" : "已停止主控；本机不再控制其他设备。");
        }
        catch (FormatException) { Notify("输入设置格式无效。"); }
        catch (Exception ex) { Notify(ex is IOException ? ex.Message : "主控设置未保存，服务可能已断开。"); }
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

    private void UpdateRemoteApplicationsHost()
    {
        if (RemoteApplicationsHost is null) return;
        RemoteApplicationsHost.Children.Clear();
        if (!remoteAppsEnabled)
        {
            RemoteApplicationsHost.Children.Add(DfUi.EmptyState("\uE8A7", "远程应用已关闭",
                "相关后台任务已停止。打开右上角总开关即可继续使用。"));
            return;
        }
        var page = new RemoteAppPage(service);
        RemoteApplicationsHost.Children.Add(page);
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

    private void UpdateHealth(bool connected)
    {
        var issues = new List<(string Tone, string Title, string Detail)>();
        if (!connected) issues.Add(("DFDangerBrush", "服务未连接", "无法连接 DeskFuseService；本机剪贴板历史仍可用。"));
        if (!string.IsNullOrEmpty(clipboard.StorageError)) issues.Add(("DFWarningBrush", "剪贴板存储异常", clipboard.StorageError));
        if (connected && latest is JsonElement snapshot
            && !snapshot.Array("peers").Any(peer => peer.String("connection") == "connected"))
            issues.Add(("DFAccentBrush", "还没有连接其他电脑", "添加另一台电脑后，就能同步剪贴板、共享键鼠和传输文件。"));
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

    private void HistoryFilterChanged(object sender, RoutedEventArgs args) => RefreshHistory();

    private Button KindChip(string label, string kind)
    {
        var chip = new Button
        {
            Content = label,
            Tag = kind,
            FontSize = 11,
            Padding = new Thickness(8, 0, 8, 0),
            MinHeight = 26,
            CornerRadius = new CornerRadius(8),
            Background = new SolidColorBrush(Colors.Transparent),
            BorderThickness = new Thickness(1),
        };
        chip.Click += KindFilterClick;
        return chip;
    }

    private void KindFilterClick(object sender, RoutedEventArgs args)
    {
        historyKind = (sender as Button)?.Tag as string ?? "";
        UpdateKindChips();
        RefreshHistory();
    }

    private void UpdateKindChips()
    {
        foreach (var (button, kind) in new[] { (KindAllButton, ""), (KindTextButton, "text"), (KindImageButton, "image"), (KindFilesButton, "files") })
        {
            bool selected = historyKind == kind;
            button.Background = selected ? DfUi.Surface : new SolidColorBrush(Colors.Transparent);
            button.BorderBrush = selected ? DfUi.Line : new SolidColorBrush(Colors.Transparent);
            button.BorderThickness = new Thickness(1);
            button.Foreground = selected ? DfUi.PrimaryText : DfUi.Muted;
            button.FontWeight = selected ? Microsoft.UI.Text.FontWeights.SemiBold : Microsoft.UI.Text.FontWeights.Normal;
        }
    }

    private void RefreshHistory()
    {
        if (HistoryList is null || updatingHistory) return;
        updatingHistory = true;
        try
        {
            string? selected = (HistoryList.SelectedItem as ClipboardEntry)?.Id;
            string source = SourceFilter.SelectedItem as string ?? "所有设备";
            var sources = new[] { "所有设备" }.Concat(clipboard.Entries.Select(e => e.SourceName).Distinct()).ToArray();
            SourceFilter.ItemsSource = sources;
            SourceFilter.SelectedItem = sources.Contains(source) ? source : "所有设备";
            string search = HistorySearch.Text.Trim();
            source = SourceFilter.SelectedItem as string ?? "所有设备";
            var visible = clipboard.Entries.Where(e =>
                (historyKind.Length == 0 || e.Kind == historyKind)
                && (source == "所有设备" || e.SourceName == source)
                && (search.Length == 0 || $"{e.Title} {e.Preview} {e.SourceName}".Contains(search, StringComparison.CurrentCultureIgnoreCase)))
                .ToArray();
            var groups = new List<HistoryGroup>();
            foreach (var entry in visible)
            {
                string key = HistoryGroupKey(entry.CreatedAt.LocalDateTime);
                if (groups.Count == 0 || groups[^1].Key != key) groups.Add(new HistoryGroup(key));
                groups[^1].Add(entry);
            }
            var grouped = new CollectionViewSource { IsSourceGrouped = true, Source = groups };
            HistoryList.ItemsSource = grouped.View;
            HistoryList.SelectedItem = visible.FirstOrDefault(e => e.Id == selected) ?? visible.FirstOrDefault();
            HistoryEmpty.Visibility = visible.Length == 0 ? Visibility.Visible : Visibility.Collapsed;
            HistoryEmptyTitle.Text = clipboard.Entries.Count == 0 ? "还没有剪贴板历史" : "没有匹配的内容";
            HistoryEmptyDetail.Text = clipboard.Entries.Count == 0
                ? "复制文字、图片或文件后\n会显示在这里。"
                : "试试其他关键词或筛选条件。";
            HistoryCount.Text = $"{visible.Length} 条";
        }
        finally { updatingHistory = false; }
        RefreshPreview();
    }

    private static string HistoryGroupKey(DateTime value)
    {
        var today = DateTime.Today;
        if (value.Date == today) return "今天";
        if (value.Date == today.AddDays(-1)) return "昨天";
        return value.ToString("M月d日");
    }

    private sealed class HistoryGroup(string key) : List<ClipboardEntry>
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

    private void DeleteSelectedClick(object sender, RoutedEventArgs args)
    {
        var selected = HistoryList.SelectedItems?.OfType<ClipboardEntry>().ToArray() ?? [];
        if (selected.Length == 0) return;
        foreach (var entry in selected) clipboard.Remove(entry);
        Notify($"已删除 {selected.Length} 条本机历史；系统剪贴板、用户源文件和已粘贴文件保持不变。");
    }

    private async void SaveImageClick(object sender, RoutedEventArgs args) => await SaveEntryAsync("image");
    private async void SaveTextClick(object sender, RoutedEventArgs args) => await SaveEntryAsync("text");

    private async Task SaveEntryAsync(string kind)
    {
        if (HistoryList.SelectedItem is not ClipboardEntry entry || entry.Kind != kind)
        {
            Notify(kind == "image" ? "请选择一条图片记录。" : "请选择一条文字记录。");
            return;
        }
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

    private void OpenFileClick(object sender, RoutedEventArgs args)
    {
        if (HistoryList.SelectedItem is not ClipboardEntry { Kind: "files" } entry) { Notify("请选择一条文件记录。"); return; }
        string? path = entry.Content.Files?.FirstOrDefault(p => File.Exists(p) || Directory.Exists(p));
        if (path is null) { Notify("源文件已移动或不可用。"); return; }
        try { Process.Start(new ProcessStartInfo(path) { UseShellExecute = true }); }
        catch (Exception) { Notify("无法打开该文件。"); }
    }

    private void HistoryDoubleClick(object sender, DoubleTappedRoutedEventArgs args) { /* Selection is preview only. */ }

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
            historyKind = "";
            SourceFilter.SelectedItem = "所有设备";
            UpdateKindChips();
            RefreshHistory();
        };
        flyout.Items.Add(clear);
        flyout.ShowAt((FrameworkElement)sender);
    }

    private void RefreshPreview()
    {
        var entry = HistoryList.SelectedItem as ClipboardEntry;
        CopyButton.IsEnabled = entry is not null;
        EditTitleButton.IsEnabled = entry is not null;
        EditContentButton.IsEnabled = entry?.Kind == "text";
        AddShelfButton.IsEnabled = entry is not null;
        ImagePreview.Source = null;
        ImagePreview.Visibility = Visibility.Collapsed;
        TextPreview.Visibility = Visibility.Visible;
        RevealButton.Visibility = Visibility.Collapsed;
        FitButton.Visibility = Visibility.Collapsed;
        if (entry is null)
        {
            PreviewTitle.Text = "复制内容，从这里接着用";
            PreviewMetadata.Text = "在任意应用里复制，内容会在可信设备之间同步。";
            TextPreview.Visibility = Visibility.Collapsed;
            PreviewEmpty.Visibility = Visibility.Visible;
            FormatMetadata.Text = "";
            return;
        }
        PreviewEmpty.Visibility = Visibility.Collapsed;
        TextPreview.Visibility = Visibility.Visible;
        PreviewTitle.Text = entry.Title.Replace('\r', ' ').Replace('\n', ' ');
        PreviewMetadata.Text = $"{entry.SourceLabel} · {entry.CreatedAt.LocalDateTime:g}";
        FormatMetadata.Text = entry.Kind switch
        {
            "files" => $"种类：文件 · {entry.Content.Files?.Count ?? 0} 项 · {FormatBytes(entry.SizeBytes)}",
            "image" => $"种类：图片 · {FormatBytes(entry.SizeBytes)}",
            _ => $"种类：文字 · {FormatBytes(entry.SizeBytes)}",
        };
        TextPreview.Text = entry.Preview;
        if (entry.Kind == "image")
        {
            var image = ClipboardImages.Create(entry.Content.Data);
            if (image is null) { TextPreview.Text = "图片无法预览，可能已损坏或超过解码限制。"; CopyButton.IsEnabled = false; }
            else
            {
                ImagePreview.Source = image;
                ImagePreview.Visibility = Visibility.Visible;
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

    private void EditHistoryContentClick(object sender, RoutedEventArgs args)
    {
        if (HistoryList.SelectedItem is not ClipboardEntry { Kind: "text" } entry) return;
        if (!contentEditors.TryGetValue(entry.Id, out var editor))
        {
            editor = new ClipboardContentEditorWindow(entry, content =>
            {
                clipboard.SaveEditedContent(content);
                Notify("编辑内容已复制；开启历史记录时将保存为新记录。");
            });
            editor.Closed += () => contentEditors.Remove(entry.Id);
            contentEditors.Add(entry.Id, editor);
        }
        editor.Show();
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
            clipboard.History.SetTitle(entry.Id, editor.Text);
            Notify("标题已更新。");
        }
        catch (Exception ex) when (ex is IOException or InvalidOperationException) { Notify(ex.Message); }
    }

    private void DeleteHistoryClick(object sender, RoutedEventArgs args)
    {
        if (HistoryList.SelectedItem is not ClipboardEntry entry) return;
        clipboard.Remove(entry);
        Notify("已删除这条本机历史；系统剪贴板、用户源文件和已粘贴文件保持不变。");
    }

    private void CopyClick(object sender, RoutedEventArgs args) => CopySelection();

    /// <summary>Tray "recent clipboard" entry point.</summary>
    public void RestoreEntry(ClipboardEntry entry)
    {
        try { Notify(clipboard.SyncEnabled ? "已恢复本机剪贴板；跨设备结果取决于真实连接与服务回执。" : "已恢复本机系统剪贴板。"); clipboard.Restore(entry); }
        catch (Exception) { Notify("无法恢复剪贴板。请检查文件是否仍存在、当前会话是否可用或剪贴板是否被占用。"); }
    }

    private void CopySelection()
    {
        if (HistoryList.SelectedItem is ClipboardEntry entry) RestoreEntry(entry);
    }

    private void AddShelfClick(object sender, RoutedEventArgs args) { if (HistoryList.SelectedItem is ClipboardEntry entry) clipboard.ShowShelf(entry); }

    private void SendClipboardFilesClick(object sender, RoutedEventArgs args)
    {
        if (HistoryList.SelectedItem is not ClipboardEntry { Kind: "files" } entry || entry.Content.Files is not { Count: > 0 } files)
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
        SendStagedButton.IsEnabled = any && !busy && SelectedSendTarget() is not null;
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
        SendStagedButton.IsEnabled = stagedFiles.Count > 0 && !busy && SelectedSendTarget() is not null;
    }

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
        if (busy || stagedFiles.Count == 0) return;
        string? target = SelectedSendTarget();
        if (target is null)
        {
            Notify("还没有已连接的电脑；先到设备页添加。");
            return;
        }
        busy = true;
        RenderStagedFiles();
        try
        {
            bool sent = await clipboard.PublishFilesAsync(stagedFiles.ToArray(), target);
            if (sent)
            {
                stagedFiles.Clear();
                Notify("已发送文件引用；对方在传输任务中选择接收。");
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

    private void RevealClick(object sender, RoutedEventArgs args)
    {
        if (HistoryList.SelectedItem is not ClipboardEntry entry || entry.Content.Files?.FirstOrDefault() is not string path) return;
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
        if (error.Length > 0) Notify(error);
        settings?.RefreshService(snapshot);
    }

    private void ConnectionChanged(bool connected, string message)
    {
        if (!connected)
        {
            latest = null;
            updatingSync = true;
            SyncCheck.IsChecked = clipboard.SyncEnabled;
            SyncCheck.IsEnabled = clipboard.SyncEnabled;
            updatingSync = false;
            SyncStatus.Text = "同步暂停；重新连接不会重发旧历史";
            if (pairIpButton is not null) pairIpButton.IsEnabled = false;
            InputAvailability.Text = "普通桌面输入 helper 正在等待服务重连；不会接受新的远端输入。";
            InputReceiverReady.IsChecked = false;
            ReclaimInputButton.IsEnabled = false;
            ClearInputQuality();
            lastDeviceSnapshot = "";
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
            InputAvailability.Text = "当前服务未提供 Windows 输入 helper 状态。";
            InputReceiverReady.IsChecked = false;
            ReclaimInputButton.IsEnabled = false;
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
        InputReceiverReady.IsChecked = ready;
        ReclaimInputButton.IsEnabled = managed;
        InputAvailability.Text = !capable
            ? "普通桌面输入 helper 尚未就绪；远端输入保持关闭。"
            : managed
                ? $"{(ready ? "正在被控" : "被控准备中")} · 主控：{(controller.Length > 0 ? controller : "已授权设备")}{(reason.Length > 0 ? $"\n{reason}" : "")}"
                : $"输入 helper 已连接 · {input.Number("display_count")} 个本机显示器 · 等待设备网络中的主控。";
        inputSnapshot = input;
        inputSnapshotValid = true;
        var settings = input.TryGetProperty("settings", out var sharing) && sharing.ValueKind == JsonValueKind.Object ? sharing : default;
        bool controllerEnabled = settings.ValueKind == JsonValueKind.Object && settings.Bool("enabled") && settings.String("role") == "Controller";
        if (controllerEnabled)
        {
            string controllingId = input.TryGetProperty("controlling", out var controlling) && controlling.ValueKind == JsonValueKind.Object ? controlling.String("peer_id") : "";
            string target = input.TryGetProperty("controlling", out var controlling2) && controlling2.ValueKind == JsonValueKind.Object ? controlling2.String("target_display") : "";
            string peer = snapshot.Array("peers").FirstOrDefault(p => p.String("peer_id") == controllingId).String("display_name", controllingId);
            InputAvailability.Text += $"\n本机已开启主控{(peer.Length > 0 && controllingId.Length > 0 ? $"：正在控制 {peer}{(target.Length > 0 ? $"（{target}）" : "")}" : "；把指针移到屏幕边缘即可控制对端")}";
        }
        RenderInputParticipants(settings, snapshot, input);
        RenderShortcutBindings(input);
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
        if (SaveEnvironmentLayoutButton is not null)
            SaveEnvironmentLayoutButton.IsEnabled = service.IsConnected && effective.Length > 0 && !busy;
        if (LayoutScenesPanel is null) return;
        LayoutScenesPanel.Children.Clear();
        var scenes = input.TryGetProperty("scenes", out var list) && list.ValueKind == JsonValueKind.Array ? list : default;
        if (scenes.ValueKind != JsonValueKind.Array || scenes.GetArrayLength() == 0)
        {
            LayoutScenesPanel.Children.Add(DfUi.Text("还没有保存的布局；拖动显示器排布后保存到当前 Wi-Fi，回到该网络会自动恢复。", 12, false, DfUi.Muted));
            return;
        }
        foreach (var scene in scenes.EnumerateArray())
        {
            string sceneId = scene.String("id");
            string sceneSsid = scene.String("ssid");
            var row = new Grid { Margin = new Thickness(0, 2, 0, 2), ColumnSpacing = 8 };
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            row.Children.Add(DfUi.Text($"{scene.String("name", "未命名布局")}{(sceneSsid.Length > 0 ? " · Wi-Fi" : "")}{(sceneSsid.Length > 0 && sceneSsid == effective ? "（当前网络）" : "")}", 12));
            var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
            string captured = sceneId;
            var apply = DfUi.Secondary("应用", async (_, _) =>
            {
                if (busy) return;
                busy = true;
                try { await service.ApplySceneAsync(captured); Notify("已请求应用布局；以屏幕上的实际排布为准。"); }
                catch (Exception ex) { Notify(ex is IOException ? $"{ex.Message}（应用布局需要匹配版本的服务）" : "操作未完成，服务可能已断开。"); }
                finally { busy = false; }
            });
            apply.IsEnabled = service.IsConnected && !busy;
            var delete = DfUi.Secondary("删除", async (_, _) =>
            {
                if (busy) return;
                if (!Dialogs.Confirm(Handle, "删除该已保存的布局？删除后回到对应 Wi-Fi 不再自动恢复。", "删除布局")) return;
                busy = true;
                try { await service.DeleteSceneAsync(captured); Notify("已请求删除布局。"); }
                catch (Exception ex) { Notify(ex is IOException ? $"{ex.Message}（删除布局需要匹配版本的服务）" : "操作未完成，服务可能已断开。"); }
                finally { busy = false; }
            });
            delete.IsEnabled = service.IsConnected && !busy;
            actions.Children.Add(apply);
            actions.Children.Add(delete);
            Grid.SetColumn(actions, 1);
            row.Children.Add(actions);
            LayoutScenesPanel.Children.Add(row);
        }
    }

    /// <summary>Wi-Fi 标识详情：本机读取、场景判定、首选主控与各对端上报。</summary>
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

    private async void SaveEnvironmentLayoutClick(object sender, RoutedEventArgs args)
    {
        if (busy) return;
        busy = true;
        try
        {
            await service.SaveEnvironmentLayoutAsync(null);
            Notify("已请求把当前布局绑定到当前 Wi-Fi；回到该网络会自动恢复排布。");
        }
        catch (Exception ex) { Notify(ex is IOException ? $"{ex.Message}（绑定布局需要匹配版本的服务）" : "操作未完成，服务可能已断开。"); }
        finally { busy = false; }
    }

    private void RenderShortcutBindings(JsonElement input)
    {
        if (ShortcutBindings is null || updatingInputSettings || !input.TryGetProperty("settings", out var settings) || settings.ValueKind != JsonValueKind.Object) return;
        if (!settings.TryGetProperty("shortcuts", out var shortcuts) || shortcuts.ValueKind != JsonValueKind.Object) return;
        string[] names = ["left", "right", "up", "down", "lock", "emergency"];
        var lines = names.Select(name =>
        {
            var shortcut = shortcuts.TryGetProperty(name, out var value) ? value : default;
            return $"{shortcut.Number("key_code")}, {shortcut.Number("modifiers")}";
        });
        updatingInputSettings = true;
        ShortcutBindings.Text = string.Join(Environment.NewLine, lines);
        updatingInputSettings = false;
    }

    /// <summary>Draws the shared screen layout: local displays plus remote
    /// participant topologies, positioned like the runtime layout resolver.</summary>
    private void DrawInputLayout(JsonElement input)
    {
        if (DisplayCanvas is null || DisplayCanvas.ActualWidth < 10 || DisplayCanvas.ActualHeight < 10) return;
        DisplayCanvas.Children.Clear();
        var monitors = MonitorLayout.All();
        if (monitors.Count == 0) return;
        int left = monitors.Min(m => m.Left), top = monitors.Min(m => m.Top);
        int right = monitors.Max(m => m.Left + m.Width), bottom = monitors.Max(m => m.Top + m.Height);
        double scale = Math.Min((DisplayCanvas.ActualWidth - 50) / Math.Max(1, right - left), (DisplayCanvas.ActualHeight - 110) / Math.Max(1, bottom - top));
        scale = Math.Max(0.01, scale);
        double offsetX = (DisplayCanvas.ActualWidth - (right - left) * scale) / 2;
        double offsetY = Math.Max(20, (DisplayCanvas.ActualHeight - 70 - (bottom - top) * scale) / 2);
        for (int i = 0; i < monitors.Count; i++)
        {
            var monitor = monitors[i];
            var content = new StackPanel { VerticalAlignment = VerticalAlignment.Center, HorizontalAlignment = HorizontalAlignment.Center, Margin = new Thickness(8) };
            content.Children.Add(new TextBlock { Text = (i + 1).ToString(), FontSize = 28, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold, TextAlignment = TextAlignment.Center });
            content.Children.Add(new TextBlock { Text = monitor.DeviceName, FontSize = 12, TextAlignment = TextAlignment.Center });
            content.Children.Add(new TextBlock { Text = $"{monitor.Width} × {monitor.Height} px{(monitor.Primary ? " · 主屏" : "")}", FontSize = 12, TextAlignment = TextAlignment.Center });
            var rectangle = new Border
            {
                Width = monitor.Width * scale,
                Height = monitor.Height * scale,
                BorderBrush = DfUi.Brush("DFAccentBrush"),
                BorderThickness = new Thickness(2),
                Background = DfUi.Brush("DFAccentSubtleBrush"),
                CornerRadius = new CornerRadius(4),
                Child = content,
            };
            Canvas.SetLeft(rectangle, offsetX + (monitor.Left - left) * scale);
            Canvas.SetTop(rectangle, offsetY + (monitor.Top - top) * scale);
            DisplayCanvas.Children.Add(rectangle);
        }
        int screenCount = monitors.Count;
        double remoteOffset = offsetX + (right - left) * scale + 24;
        double remoteY = offsetY;
        var seen = new HashSet<string>();
        if (input.ValueKind == JsonValueKind.Object)
        {
            foreach (var remote in input.Array("remote_topologies"))
            {
                string peer = remote.String("peer_id");
                if (!seen.Add(peer)) continue;
                string name = service.LatestSnapshot?.Array("peers").FirstOrDefault(p => p.String("peer_id") == peer).String("display_name", "远端") ?? "远端";
                var topology = remote.GetProperty("topology");
                int minX = int.MaxValue, minY = int.MaxValue, maxX = 0, maxY = 0;
                foreach (var display in topology.Array("displays"))
                {
                    int x = (int)display.Number("frame_x");
                    int y = (int)display.Number("frame_y");
                    int width = (int)display.Number("width");
                    int height = (int)display.Number("height");
                    minX = Math.Min(minX, x); minY = Math.Min(minY, y);
                    maxX = Math.Max(maxX, x + width); maxY = Math.Max(maxY, y + height);
                }
                if (minX == int.MaxValue) continue;
                foreach (var display in topology.Array("displays"))
                {
                    int x = (int)display.Number("frame_x");
                    int y = (int)display.Number("frame_y");
                    int width = (int)display.Number("width");
                    int height = (int)display.Number("height");
                    var content = new StackPanel { VerticalAlignment = VerticalAlignment.Center, HorizontalAlignment = HorizontalAlignment.Center, Margin = new Thickness(6) };
                    content.Children.Add(new TextBlock { Text = name, FontSize = 13, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold, TextAlignment = TextAlignment.Center });
                    content.Children.Add(new TextBlock { Text = $"{width} × {height} px", FontSize = 11, TextAlignment = TextAlignment.Center });
                    var rectangle = new Border
                    {
                        Width = width * scale,
                        Height = height * scale,
                        BorderBrush = DfUi.Brush("DFSuccessBrush"),
                        BorderThickness = new Thickness(2),
                        Background = DfUi.Brush("DFSuccessSubtleBrush"),
                        CornerRadius = new CornerRadius(4),
                        Child = content,
                        Tag = new RemoteDisplayDrag(peer, display.String("id"), x, y, scale),
                    };
                    rectangle.PointerPressed += DisplayDragStarted;
                    rectangle.PointerMoved += DisplayDragMoved;
                    rectangle.PointerReleased += DisplayDragFinished;
                    ToolTipService.SetToolTip(rectangle, $"{name} · {width} × {height}");
                    var displayMenu = new MenuFlyout();
                    var reconnect = new MenuFlyoutItem { Text = "重新连接" };
                    reconnect.Click += async (_, _) =>
                    {
                        try
                        {
                            await service.SendAsync(ServiceClient.Command("RestartInputControl", new { peer_id = peer }));
                            Notify($"已请求重新连接 {name}。");
                        }
                        catch (Exception ex) { Notify(ex is IOException ? ex.Message : "重新连接请求未完成。"); }
                    };
                    displayMenu.Items.Add(reconnect);
                    var removeDisplay = new MenuFlyoutItem { Text = "移除此设备的屏幕" };
                    removeDisplay.Click += async (_, _) => await RemoveInputParticipantAsync(peer);
                    displayMenu.Items.Add(removeDisplay);
                    rectangle.ContextFlyout = displayMenu;
                    Canvas.SetLeft(rectangle, remoteOffset + (x - minX) * scale);
                    Canvas.SetTop(rectangle, remoteY + (y - minY) * scale);
                    DisplayCanvas.Children.Add(rectangle);
                    screenCount++;
                }
                remoteY += (maxY - minY) * scale + 24;
            }
        }
        InputLayoutSummary.Text = $"{screenCount} 个屏幕";
    }

    private async void SaveShortcutBindingsClick(object sender, RoutedEventArgs args)
    {
        if (updatingInputSettings || latest is not JsonElement snapshot || !snapshot.TryGetProperty("input", out var input) || !input.TryGetProperty("settings", out var settings)) return;
        var lines = ShortcutBindings.Text.Split(["\r\n", "\n"], StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries);
        if (lines.Length != 6) { Notify("需要按左、右、上、下、锁定、紧急释放顺序填写六行快捷键。 "); return; }
        try
        {
            var node = JsonNode.Parse(settings.GetRawText())?.AsObject() ?? throw new FormatException();
            var shortcuts = node["shortcuts"]?.AsObject() ?? throw new FormatException();
            string[] names = ["left", "right", "up", "down", "lock", "emergency"];
            for (int i = 0; i < names.Length; i++)
            {
                var values = lines[i].Split(',', StringSplitOptions.TrimEntries);
                if (values.Length != 2 || !ushort.TryParse(values[0], out var key) || !ulong.TryParse(values[1], out var modifiers)) throw new FormatException();
                shortcuts[names[i]] = new JsonObject { ["key_code"] = key, ["modifiers"] = modifiers };
            }
            await service.SendAsync(ServiceClient.Command("SetInputSharingSettings", new { settings = node }));
            Notify("快捷键已提交；以输入状态和服务持久化结果为准。");
        }
        catch (FormatException) { Notify("快捷键格式无效，请使用 key code, modifiers，每行一组。"); }
        catch (Exception ex) { Notify(ex is IOException ? ex.Message : "快捷键未保存，服务可能已断开。"); }
    }

    private sealed record RemoteDisplayDrag(string DeviceId, string DisplayId, double X, double Y, double Scale);

    private void DisplayDragStarted(object sender, PointerRoutedEventArgs args)
    {
        if (sender is not Border border || border.Tag is not RemoteDisplayDrag || latest is not JsonElement snapshot || !snapshot.TryGetProperty("input", out var input) || !input.TryGetProperty("settings", out var settings) || settings.ValueKind != JsonValueKind.Object) return;
        if (!settings.TryGetProperty("role", out var role) || role.GetString() != "Controller" || !settings.Bool("enabled")) { Notify("只有已启用的本机主控可以编辑远端布局。"); return; }
        draggingDisplay = border;
        dragOrigin = args.GetCurrentPoint(DisplayCanvas).Position;
        border.CapturePointer(args.Pointer);
        args.Handled = true;
    }

    private void DisplayDragMoved(object sender, PointerRoutedEventArgs args)
    {
        if (draggingDisplay is not Border border || border.Tag is not RemoteDisplayDrag drag || !ReferenceEquals(border, sender)) return;
        var position = args.GetCurrentPoint(DisplayCanvas).Position;
        double deltaX = position.X - dragOrigin.X;
        double deltaY = position.Y - dragOrigin.Y;
        Canvas.SetLeft(border, Canvas.GetLeft(border) + deltaX);
        Canvas.SetTop(border, Canvas.GetTop(border) + deltaY);
        dragOrigin = position;
        border.Tag = drag with { X = drag.X + deltaX / Math.Max(0.01, drag.Scale), Y = drag.Y + deltaY / Math.Max(0.01, drag.Scale) };
    }

    private async void DisplayDragFinished(object sender, PointerRoutedEventArgs args)
    {
        if (sender is not Border border || border.Tag is not RemoteDisplayDrag drag) return;
        border.ReleasePointerCapture(args.Pointer);
        draggingDisplay = null;
        try
        {
            await service.SendAsync(ServiceClient.Command("MoveControlDevice", new { device_id = drag.DeviceId, display_id = drag.DisplayId, x = drag.X, y = drag.Y }));
            Notify("布局位置已提交；运行时会进行吸附并持久化。");
        }
        catch (Exception ex) { Notify(ex is IOException ? ex.Message : "布局未保存，服务可能已断开。"); }
        args.Handled = true;
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
        if (!TryInputSettings(out var settings) || latest is not JsonElement snapshot)
        {
            Notify("输入运行时尚未就绪，无法添加设备。");
            return;
        }
        var participants = settings.Array("participants")
            .Where(value => value.ValueKind == JsonValueKind.String)
            .Select(value => value.GetString() ?? "")
            .Where(value => value.Length > 0)
            .ToHashSet(StringComparer.Ordinal);
        var candidates = snapshot.Array("peers")
            .Where(peer => peer.String("trust") == "trusted" && !participants.Contains(peer.String("peer_id")))
            .ToArray();
        if (candidates.Length == 0)
        {
            Notify("没有可添加的已配对设备。请先在设备页完成配对。");
            return;
        }
        var panel = new StackPanel { Spacing = 8 };
        var choices = new List<(string Id, CheckBox Box)>();
        foreach (var peer in candidates)
        {
            string id = peer.String("peer_id");
            string name = peer.String("display_name", "未命名设备");
            string connection = peer.String("connection") == "connected" ? "在线" : "离线";
            var box = new CheckBox { Content = $"{name} · {connection}", Tag = id, IsChecked = true };
            choices.Add((id, box));
            panel.Children.Add(box);
        }
        var dialog = new ContentDialog
        {
            XamlRoot = Root.XamlRoot,
            Title = "添加设备屏幕",
            Content = new ScrollViewer { Content = panel, MaxHeight = 360 },
            PrimaryButtonText = "添加",
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Primary,
        };
        if (await dialog.ShowAsync() != ContentDialogResult.Primary) return;
        var selected = choices.Where(choice => choice.Box.IsChecked == true).Select(choice => choice.Id).ToArray();
        if (selected.Length == 0) return;
        await SetInputParticipants(participants.Concat(selected));
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

    private async Task SetInputParticipants(IEnumerable<string> ids)
    {
        if (!TryInputSettings(out var settings)) return;
        try
        {
            var node = JsonNode.Parse(settings.GetRawText())?.AsObject() ?? throw new FormatException();
            var array = new JsonArray();
            foreach (string id in ids.Distinct(StringComparer.Ordinal)) array.Add(id);
            node["participants"] = array;
            await service.SendAsync(ServiceClient.Command("SetInputSharingSettings", new { settings = node }));
            Notify("参与者已更新。");
        }
        catch (FormatException) { Notify("输入设置格式无效。"); }
        catch (Exception ex) { Notify(ex is IOException ? ex.Message : "参与者未保存，服务可能已断开。"); }
    }

    private void RenderInputParticipants(JsonElement settings, JsonElement snapshot, JsonElement input)
    {
        if (InputParticipantsPanel is null) return;
        InputParticipantsPanel.Children.Clear();
        var participants = settings.Array("participants")
            .Where(value => value.ValueKind == JsonValueKind.String)
            .Select(value => value.GetString() ?? "")
            .Where(value => value.Length > 0)
            .ToArray();
        InputParticipantsEmpty.Visibility = participants.Length == 0 ? Visibility.Visible : Visibility.Collapsed;
        var topologyPeers = input.Array("remote_topologies")
            .Select(item => item.String("peer_id"))
            .Where(id => id.Length > 0)
            .ToHashSet(StringComparer.Ordinal);
        for (int index = 0; index < participants.Length; index++)
        {
            string id = participants[index];
            var peer = snapshot.Array("peers").FirstOrDefault(item => item.String("peer_id") == id);
            string name = peer.ValueKind == JsonValueKind.Object ? peer.String("display_name", id) : id;
            bool online = peer.ValueKind == JsonValueKind.Object && peer.String("connection") == "connected";
            bool ready = topologyPeers.Contains(id);
            string status = ready ? "已就绪" : online ? "等待对端准备" : "离线";
            var row = new Grid { Padding = new Thickness(0, 8, 0, 8), ColumnSpacing = 10 };
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            row.Children.Add(new Border
            {
                Width = 32, Height = 32, CornerRadius = new CornerRadius(8),
                Background = DfUi.Control,
                Child = DfUi.Glyph("\uE7F4", 15, DfUi.Muted),
            });
            var text = new StackPanel { Spacing = 2, VerticalAlignment = VerticalAlignment.Center };
            text.Children.Add(DfUi.Text(name, 13, true, DfUi.PrimaryText, wrap: false));
            var statusRow = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
            statusRow.Children.Add(new Microsoft.UI.Xaml.Shapes.Ellipse
            {
                Width = 6, Height = 6,
                Fill = ready ? DfUi.Success : online ? DfUi.Warning : new SolidColorBrush(Colors.Gray),
                VerticalAlignment = VerticalAlignment.Center,
            });
            statusRow.Children.Add(DfUi.Detail(status));
            text.Children.Add(statusRow);
            Grid.SetColumn(text, 1);
            row.Children.Add(text);
            string captured = id;
            var remove = DfUi.Secondary("移除", async (_, _) => await SetInputParticipants(participants.Where(item => item != captured)));
            remove.MinHeight = 28;
            remove.FontSize = 12;
            Grid.SetColumn(remove, 2);
            row.Children.Add(remove);
            InputParticipantsPanel.Children.Add(row);
            if (index < participants.Length - 1) InputParticipantsPanel.Children.Add(DfUi.Divider(0, 0));
        }
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
    }

    private void InstallWorkflowHotkeys()
    {
        if (!loaded) return;
        RemoveHotkeys();
        WindowMessageHook.Attach(Handle, HotkeyWindowProc);
        int hotkeyId = 0x5540;
        foreach (var workflow in workflows.Where(w => w.Enabled && w.Shortcut is not null))
        {
            var shortcut = workflow.Shortcut!;
            uint modifiers = shortcut.Modifiers switch
            {
                var value when (value & 0x80000) != 0 => ModAlt,
                _ => 0,
            };
            if ((shortcut.Modifiers & 0x40000) != 0) modifiers |= ModControl;
            if ((shortcut.Modifiers & 0x20000) != 0) modifiers |= ModShift;
            if ((shortcut.Modifiers & 0x100000) != 0) modifiers |= ModWin;
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
        if (message == WmHotKey && hotkeys.TryGetValue(wParam.ToInt32(), out var workflow))
        {
            _ = ExecuteWorkflowAsync(workflow);
            return true;
        }
        return false;
    }

    private async Task ExecuteWorkflowAsync(WorkflowRecord workflow)
    {
        if (!service.IsConnected || latest is not JsonElement snapshot || !snapshot.TryGetProperty("capabilities", out var caps) || !caps.Bool("workflow_execution"))
        {
            Notify("工作流执行能力尚未就绪；没有向服务发送剪贴板内容。");
            return;
        }
        try
        {
            long epoch = snapshot.Number("session_epoch");
            await workflowExecution.ExecuteAsync(workflow.Id, epoch, DateTime.UtcNow.Ticks, CancellationToken.None);
            Notify($"已执行工作流：{workflow.Name}");
        }
        catch (Exception ex) { Notify(ex is IOException ? ex.Message : "工作流未执行，剪贴板保持不变。"); }
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

    private void RenderWorkflowEditor()
    {
        var workflow = editingWorkflow;
        bool enabled = workflow is not null;
        WorkflowName.IsEnabled = enabled; WorkflowEnabled.IsEnabled = enabled; WorkflowShortcut.IsEnabled = enabled;
        SaveWorkflowButton.IsEnabled = enabled;
        DeleteWorkflowButton.IsEnabled = workflow is not null && workflows.Contains(workflow);
        if (workflow is null) { WorkflowName.Text = ""; WorkflowEnabled.IsChecked = false; WorkflowShortcut.Text = ""; return; }
        WorkflowName.Text = workflow.Name; WorkflowEnabled.IsChecked = workflow.Enabled;
        WorkflowShortcut.Text = workflow.Shortcut is null ? "" : $"{workflow.Shortcut.KeyCode}, {workflow.Shortcut.Modifiers}";
        WorkflowNotice.Text = "";
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
            editingWorkflow.Name = WorkflowName.Text.Trim(); editingWorkflow.Enabled = WorkflowEnabled.IsChecked == true; editingWorkflow.Shortcut = shortcut;
            if (editingWorkflow.Steps.Count == 0) editingWorkflow.Steps = [WorkflowStepRecord.Copy()];
            await service.SendAsync(ServiceClient.Command("SaveWorkflow", new { workflow = editingWorkflow }));
            if (!workflows.Contains(editingWorkflow)) workflows.Add(editingWorkflow);
            InstallWorkflowHotkeys();
            WorkflowList.ItemsSource = null; WorkflowList.ItemsSource = workflows;
            WorkflowList.SelectedItem = editingWorkflow;
            WorkflowNotice.Text = "已提交；服务端 Rust 模型校验并持久化。";
        }
        catch (FormatException) { WorkflowNotice.Text = "快捷键格式无效，请使用 key code, modifiers。"; }
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
            var reply = await service.SendAsync("Snapshot");
            if (reply.TryGetProperty("Snapshot", out var snapshot))
            {
                SnapshotChanged(snapshot);
                Notify("设备状态已刷新。");
            }
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
            bool capable = latest is JsonElement snapshot && snapshot.TryGetProperty("capabilities", out var caps) && caps.Bool("remote_app");
            var launch = DfUi.IconButton("\uE8A7", (_, _) => { selectedDeviceId = device.Id; RenderDeviceDetails(); ShowPage(WorkspacePage.RemoteApplications); }, "打开远程应用", 26, 12);
            launch.IsEnabled = capable;
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
        bool canRemoteApp = latest is JsonElement remoteSnapshot
            && remoteSnapshot.TryGetProperty("capabilities", out var remoteCaps) && remoteCaps.Bool("remote_app");
        var more = DfUi.IconButton("\uE712", (sender, _) =>
        {
            var flyout = new MenuFlyout();
            var open = new MenuFlyoutItem { Text = "打开远程应用", IsEnabled = connected && canRemoteApp };
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
            {
                var reconnect = DfUi.Secondary("重新连接");
                reconnect.IsEnabled = false;
                ToolTipService.SetToolTip(reconnect, "网络会在设备恢复后自动重连");
                actions.Children.Add(reconnect);
            }
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
        else if (args.Key == global::Windows.System.VirtualKey.Enter && ReferenceEquals(FocusManager.GetFocusedElement(Root.XamlRoot), HistoryList)) { CopySelection(); args.Handled = true; }
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
