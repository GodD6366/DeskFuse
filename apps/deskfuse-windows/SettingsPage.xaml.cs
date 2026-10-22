using System.Text.Json;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;

namespace DeskFuse.Windows;

/// <summary>Settings pages mirrored from the macOS SettingsView: one scrolling
/// 720 pt form per category, section label + card + footer rows. Category
/// selection lives in the main sidebar.</summary>
public sealed partial class SettingsPage : UserControl
{
    private readonly Dictionary<string, UIElement> pages = [];
    private readonly DesktopClipboardController clipboard;
    private readonly ServiceClient service;
    private readonly PowerStateController power;
    private readonly UIElement workflows;
    private readonly Action loadWorkflows;
    private readonly Action openInputPage;
    private readonly TextBlock serviceStatus = DfUi.Detail("");
    private readonly TextBlock cacheStatus = DfUi.Detail("正在读取缓存…");
    private readonly TextBlock advancedStatus = DfUi.Detail("");
    private readonly TextBlock generalStatus = DfUi.Detail("");
    private readonly TextBlock updateStatus = DfUi.Detail("检查更新会先读取 GitHub Releases，失败时回退到独立域名镜像。");
    private readonly HyperlinkButton openRelease = new() { Content = "打开发布页", Visibility = Visibility.Collapsed, HorizontalAlignment = HorizontalAlignment.Left };
    private readonly DfToggle sync;
    private readonly DfToggle awake;
    private readonly DfToggle capture;
    private Grid clipboardModeHost = null!;
    private Grid clipboardHistoryHost = null!;
    private Grid clipboardWorkflowsHost = null!;
    private Button historySegment = null!;
    private Button workflowsSegment = null!;
    private bool workflowsMode;
    private bool updatingAwake;

    public SettingsPage(DesktopClipboardController clipboard, ServiceClient service, PowerStateController power,
        UIElement workflows, Action loadWorkflows, Action openInputPage)
    {
        this.clipboard = clipboard;
        this.service = service;
        this.power = power;
        this.workflows = workflows;
        this.loadWorkflows = loadWorkflows;
        this.openInputPage = openInputPage;
        InitializeComponent();

        sync = DfUi.Switch(clipboard.SyncEnabled, (_, _) => clipboard.SyncEnabled = sync!.IsOn);
        awake = DfUi.Switch(clipboard.KeepAwake, (_, _) => { if (!updatingAwake) SetKeepAwake(awake!.IsOn); });
        capture = DfUi.Switch(clipboard.CaptureEnabled, (_, _) => clipboard.CaptureEnabled = capture!.IsOn);

        pages["general"] = BuildGeneral();
        pages["clipboard"] = BuildClipboard();
        pages["shortcuts"] = BuildShortcuts();
        pages["files"] = BuildFiles();
        pages["permissions"] = BuildPermissions();
        pages["about"] = BuildAbout();
        pages["advanced"] = BuildAdvanced();
        Show("general");
    }

    // MARK: - Pages

    private UIElement BuildGeneral()
    {
        var startup = new StackPanel
        {
            Children =
            {
                DfUi.SettingRow("登录时自动启动", "Windows 安装包尚未提供登录项；从开始菜单打开 DeskFuse 即可。",
                    DfUi.StatusPill("暂不可用", "DFMutedBrush")),
                DfUi.Divider(8, 8),
                DfUi.SettingRow("关闭窗口后继续在托盘运行", "关闭主窗口不会退出后台组件；从托盘菜单可以退出。",
                    DfUi.StatusPill("始终开启", "DFSuccessBrush")),
            },
        };
        var available = new StackPanel
        {
            Children =
            {
                DfUi.SettingRow("保持电脑唤醒", "阻止系统和显示器自动休眠，作为被控端时保持在线；退出 DeskFuse 后自动恢复。", awake),
                generalStatus,
            },
        };
        var state = new StackPanel
        {
            Children =
            {
                DfUi.SettingRow("后台服务", "DeskFuseService 负责网络、剪贴板与文件传输。", serviceStatus),
            },
        };
        return Form(
            DfUi.Section("启动", startup, "登录项由 Windows 管理；当前构建只提供托盘后台运行。"),
            DfUi.Section("保持可用", available),
            DfUi.Section("本机状态", state));
    }

    private UIElement BuildClipboard()
    {
        clipboardModeHost = new Grid { Margin = new Thickness(24, 0, 24, 12), HorizontalAlignment = HorizontalAlignment.Left };
        var segmentBorder = new Border
        {
            Background = DfUi.Control,
            CornerRadius = new CornerRadius(8),
            Padding = new Thickness(2),
            Child = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 2 },
        };
        historySegment = SegmentButton("历史", true);
        workflowsSegment = SegmentButton("工作流", false);
        ((StackPanel)segmentBorder.Child).Children.Add(historySegment);
        ((StackPanel)segmentBorder.Child).Children.Add(workflowsSegment);
        clipboardModeHost.Children.Add(segmentBorder);
        historySegment.Click += (_, _) => SetClipboardMode(false);
        workflowsSegment.Click += (_, _) => SetClipboardMode(true);

        clipboardHistoryHost = new Grid();
        clipboardHistoryHost.Children.Add(BuildClipboardHistory());
        clipboardWorkflowsHost = new Grid { Visibility = Visibility.Collapsed };
        clipboardWorkflowsHost.Children.Add(workflows);

        var root = new StackPanel { Spacing = 0 };
        root.Children.Add(clipboardModeHost);
        root.Children.Add(clipboardHistoryHost);
        root.Children.Add(clipboardWorkflowsHost);
        return root;
    }

    private UIElement BuildClipboardHistory()
    {
        var history = new StackPanel
        {
            Children =
            {
                DfUi.SettingRow("记录新的剪贴板历史", "停止记录不关闭已开启的同步。历史只属于当前用户。", capture),
                DfUi.Divider(8, 8),
                HistoryRetentionRow(),
            },
        };
        var syncRow = new StackPanel
        {
            Children =
            {
                DfUi.SettingRow("自动同步剪贴板", "同步之后的新复制内容；开启时不会重新发送历史。", sync),
            },
        };
        var search = new StackPanel
        {
            Children =
            {
                DfUi.SettingRow("同步标题与识别文字", "跨设备只同步标题与元数据；图片和文件正文按需获取。",
                    DfUi.StatusPill("始终开启", "DFSuccessBrush")),
            },
        };
        return Form(
            DfUi.Section("普通历史", history, "默认保留 30 天，不限制条数；到期记录会在后台清理。"),
            DfUi.Section("跨设备搜索", search),
            DfUi.Section("同步", syncRow),
            DfUi.Detail("本页修改立即生效；剪贴板历史、同步标题与排除应用属于本机配置。"));
    }

    private UIElement BuildShortcuts()
    {
        var input = new StackPanel
        {
            Children =
            {
                DfUi.SettingRow("方向切换与锁定快捷键", "在「键鼠共享」页面编辑；保存后由输入运行时校验并持久化。", DfUi.Secondary("打开键鼠共享", (_, _) => openInputPage())),
                DfUi.Divider(8, 8),
                DfUi.SettingRow("紧急收回", "自定义按键之外，此组合始终可用。", DfUi.StatusPill("Ctrl + Alt + Win + Esc", "DFMutedBrush")),
            },
        };
        var workflowsInfo = new StackPanel
        {
            Children =
            {
                DfUi.SettingRow("工作流快捷键", "在设置 → 剪贴板 → 工作流中为每个工作流分配快捷键。",
                    DfUi.Secondary("管理工作流", (_, _) => Show("workflows"))),
            },
        };
        var reserved = new StackPanel
        {
            Children =
            {
                DfUi.SettingRow("文件暂存区", "打开或收起暂存区。", DfUi.StatusPill("Ctrl + Shift + D", "DFMutedBrush")),
            },
        };
        return Form(
            DfUi.Section("键鼠控制", input, "方向切换与锁定遵循键鼠共享状态。"),
            DfUi.Section("工作流", workflowsInfo, "停用的流程不会响应快捷键。"),
            DfUi.Section("固定快捷键", reserved, "这些组合键由 DeskFuse 保留，不能分配给工作流。"));
    }

    private UIElement BuildFiles()
    {
        var receive = new StackPanel
        {
            Children =
            {
                DfUi.SettingRow("接收位置", MainWindow.DefaultReceiveFolder(),
                    DfUi.Secondary("打开文件夹", (_, _) => OpenFolder(MainWindow.DefaultReceiveFolder()))),
            },
        };
        var cache = new StackPanel
        {
            Children =
            {
                DfUi.SettingRow("已使用", "服务为正在接收、粘贴和导出的内容保留缓存。", cacheStatus),
                DfUi.Divider(8, 8),
                DfUi.SettingRow("历史传输文件", "保护正在接收、粘贴和导出的内容，不删除用户源文件或已粘贴副本。",
                    DfUi.Secondary("清除…", (_, _) => advancedStatus.Text = "Windows 端缓存由服务自动回收。")),
            },
        };
        return Form(
            DfUi.Section("接收位置", receive, "从暂存区发送到此设备的文件会直接保存到该位置。"),
            DfUi.Section("缓存", cache));
    }

    private UIElement BuildPermissions()
    {
        var permissionRows = new StackPanel { Spacing = 0 };
        permissionRows.Children.Add(PermissionRow("\uE7E8", "普通桌面输入", "在本机与被控端注入键鼠事件。", "可用", "DFSuccessBrush"));
        permissionRows.Children.Add(DfUi.Divider(0, 0));
        permissionRows.Children.Add(PermissionRow("\uE72E", "提升窗口控制", "需要签名并安装到受信任位置的 UIAccess 版本。", "未授权", "DFMutedBrush"));
        permissionRows.Children.Add(DfUi.Divider(0, 0));
        permissionRows.Children.Add(PermissionRow("\uE8D7", "登录、UAC 与锁屏桌面", "这些安全桌面始终不可注入。", "不可用", "DFDangerBrush"));
        return Form(
            DfUi.Section("系统权限", permissionRows, "状态来自 Windows；仅展示 DeskFuse 实际使用的能力。"));
    }

    private UIElement BuildAbout()
    {
        var brand = new Grid { ColumnSpacing = 14, Padding = new Thickness(20) };
        brand.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        brand.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        var brandIcon = new Grid { Width = 48, Height = 48 };
        brandIcon.Children.Add(DfUi.Glyph("\uE8A7", 24, DfUi.Accent));
        brandIcon.Children.Add(new Image
        {
            Source = new Microsoft.UI.Xaml.Media.Imaging.BitmapImage(new Uri("ms-appx:///Assets/Square44x44Logo.png")),
            Stretch = Stretch.Uniform,
        });
        brand.Children.Add(new Border
        {
            Width = 48, Height = 48, CornerRadius = new CornerRadius(12),
            Background = DfUi.Brush("DFAccentSubtleBrush"),
            Child = brandIcon,
        });
        var brandText = new StackPanel { Spacing = 4, VerticalAlignment = VerticalAlignment.Center };
        brandText.Children.Add(DfUi.Text("DeskFuse", 20, true));
        brandText.Children.Add(DfUi.Detail("一套键鼠、一个剪贴板，在多台电脑之间无缝工作。"));
        Grid.SetColumn(brandText, 1);
        brand.Children.Add(brandText);

        var check = DfUi.Secondary("检查更新", async (_, _) => await CheckUpdateAsync());
        var update = new StackPanel
        {
            Children =
            {
                DfUi.SettingRow($"DeskFuse {AppVersion.Display}", "Windows 安装包与自动安装更新尚未随发行提供。", check),
                updateStatus,
                openRelease,
            },
        };
        var export = DfUi.Secondary("导出…", async (_, _) => await ExportDiagnosticsAsync());
        var feedback = new StackPanel
        {
            Children =
            {
                DfUi.SettingRow("导出诊断信息", "遇到连接、同步或键鼠问题时，导出给开发者分析；不包含剪贴板正文。", export),
            },
        };
        var report = DfUi.Secondary("反馈问题…", (_, _) => SupportLinks.ReportProblem(service.LatestSnapshot));
        var known = DfUi.Secondary("已报告的问题", (_, _) => SupportLinks.Open(SupportLinks.IssuesUrl));
        var feedbackActions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, Children = { report, known } };
        feedback.Children.Add(DfUi.Divider(8, 8));
        feedback.Children.Add(DfUi.SettingRow("联系开发者", "反馈页会预填版本、系统和服务状态，只需描述发生了什么。", feedbackActions));
        return Form(
            new Border { Child = brand },
            DfUi.Section("更新", update, "DeskFuse 每 12 小时自动检查一次更新。"),
            DfUi.Section("反馈问题", feedback));
    }

    private UIElement BuildAdvanced()
    {
        var features = new StackPanel
        {
            Children =
            {
                DfUi.SettingRow("启用工作流", "在剪贴板里串联多个动作并绑定全局快捷键；由服务端管理。",
                    DfUi.StatusPill("由服务管理", "DFMutedBrush")),
            },
        };
        var clear = DfUi.Secondary("清空…", async (_, _) => await ClearHistoryAsync());
        clear.Foreground = DfUi.Danger;
        var clipboardSettings = new StackPanel
        {
            Children =
            {
                DfUi.SettingRow("跨设备索引与排除 App", "同步标题与识别文字、管理不保存历史的应用。",
                    DfUi.Secondary("打开剪贴板设置", (_, _) => Show("clipboard"))),
                DfUi.Divider(8, 8),
                DfUi.SettingRow("清空剪贴板历史", "删除本机保存的全部剪贴板历史。", clear),
            },
        };
        var export = DfUi.Secondary("导出…", async (_, _) => await ExportDiagnosticsAsync());
        var diagnostics = new StackPanel
        {
            Children =
            {
                DfUi.SettingRow("导出诊断信息", "附带服务状态、客户端内存与历史统计；不包含剪贴板内容。", export),
                DfUi.Divider(8, 8),
                DfUi.SettingRow("更新通道", "Windows 桌面安装", DfUi.StatusPill("手动安装", "DFMutedBrush")),
                DfUi.Divider(8, 8),
                DfUi.SettingRow("构建号", "客户端版本", DfUi.StatusPill(AppVersion.Display, "DFMutedBrush")),
                advancedStatus,
            },
        };
        return Form(
            DfUi.Section("功能", features, "工作流属于进阶功能；关闭后所有入口隐藏，已注册的全局快捷键也会停用。"),
            DfUi.Section("剪贴板", clipboardSettings),
            DfUi.Section("诊断", diagnostics));
    }

    private Button SegmentButton(string text, bool selected)
    {
        return new Button
        {
            Content = new TextBlock { Text = text, FontSize = 13, FontWeight = selected ? Microsoft.UI.Text.FontWeights.SemiBold : Microsoft.UI.Text.FontWeights.Normal },
            Background = selected ? DfUi.Surface : new SolidColorBrush(Microsoft.UI.Colors.Transparent),
            Foreground = DfUi.PrimaryText,
            BorderThickness = new Thickness(0),
            CornerRadius = new CornerRadius(6),
            Padding = new Thickness(18, 0, 18, 0),
            MinHeight = 26,
            MinWidth = 0,
            Template = (ControlTemplate)Application.Current.Resources["DFFlatButtonTemplate"],
        };
    }

    private void SetClipboardMode(bool showWorkflows)
    {
        workflowsMode = showWorkflows;
        clipboardHistoryHost.Visibility = showWorkflows ? Visibility.Collapsed : Visibility.Visible;
        clipboardWorkflowsHost.Visibility = showWorkflows ? Visibility.Visible : Visibility.Collapsed;
        StyleSegment(historySegment, !showWorkflows);
        StyleSegment(workflowsSegment, showWorkflows);
        if (showWorkflows) loadWorkflows();
    }

    private static void StyleSegment(Button button, bool selected)
    {
        button.Background = selected ? DfUi.Surface : new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        if (button.Content is TextBlock text)
            text.FontWeight = selected ? Microsoft.UI.Text.FontWeights.SemiBold : Microsoft.UI.Text.FontWeights.Normal;
    }

    private UIElement HistoryRetentionRow()
    {
        var days = new NumberBox
        {
            Value = clipboard.HistoryRetentionDays, Minimum = 1, Maximum = int.MaxValue,
            SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Compact,
            ValidationMode = NumberBoxValidationMode.Disabled, Width = 96,
        };
        var message = DfUi.Detail("");
        var save = DfUi.Primary("保存");
        var revert = DfUi.Secondary("撤销修改");
        save.IsEnabled = false;
        revert.IsEnabled = false;
        bool saving = false;
        void RefreshActions()
        {
            bool dirty = days.Value != clipboard.HistoryRetentionDays;
            save.IsEnabled = !saving && dirty && double.IsFinite(days.Value) && days.Value >= 1
                && days.Value <= int.MaxValue && days.Value == Math.Truncate(days.Value);
            revert.IsEnabled = !saving && dirty;
            days.IsEnabled = !saving;
        }
        days.ValueChanged += (_, _) => { message.Text = ""; RefreshActions(); };
        revert.Click += (_, _) => { days.Value = clipboard.HistoryRetentionDays; message.Text = ""; RefreshActions(); };
        save.Click += async (_, _) =>
        {
            if (!save.IsEnabled) return;
            int submitted = (int)days.Value;
            saving = true;
            RefreshActions();
            try
            {
                if (submitted < clipboard.HistoryRetentionDays)
                {
                    var confirm = new ContentDialog
                    {
                        XamlRoot = XamlRoot, Title = "缩短历史保留时间？",
                        Content = $"保存后将清理超过 {submitted} 天的历史。用户源文件和已粘贴文件会保留。",
                        PrimaryButtonText = "保存并清理", CloseButtonText = "取消", DefaultButton = ContentDialogButton.Close,
                    };
                    if (await confirm.ShowAsync() != ContentDialogResult.Primary) return;
                }
                clipboard.SetHistoryRetentionDays(submitted);
                message.Text = "已保存";
            }
            catch (IOException ex) { message.Text = ex.Message; }
            finally { saving = false; RefreshActions(); }
        };
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        actions.Children.Add(revert);
        actions.Children.Add(save);
        return new StackPanel
        {
            Spacing = 8,
            Children =
            {
                DfUi.SettingRow("保留天数", "默认保留 30 天；手动列表成员长期保留。", days),
                message,
                actions,
            },
        };
    }

    private static UIElement PermissionRow(string glyph, string title, string detail, string state, string tone)
    {
        var grid = new Grid { ColumnSpacing = 12, Padding = new Thickness(14, 10, 14, 10) };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(28) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        grid.Children.Add(DfUi.Glyph(glyph, 16, DfUi.Muted));
        var text = new StackPanel { Spacing = 2 };
        text.Children.Add(DfUi.Text(title, 13, true));
        text.Children.Add(DfUi.Detail(detail));
        Grid.SetColumn(text, 1);
        grid.Children.Add(text);
        var pill = DfUi.StatusPill(state, tone);
        Grid.SetColumn(pill, 2);
        grid.Children.Add(pill);
        return grid;
    }

    private static UIElement Form(params UIElement[] sections)
    {
        var content = new StackPanel { Spacing = 24 };
        foreach (var section in sections) content.Children.Add(section);
        return DfUi.Page(content);
    }

    // MARK: - Actions

    public void Show(string category = "general")
    {
        if (service.LatestSnapshot is JsonElement snapshot) RefreshService(snapshot);
        sync.IsOn = clipboard.SyncEnabled;
        capture.IsOn = clipboard.CaptureEnabled;
        if (category == "workflows")
        {
            category = "clipboard";
            workflowsMode = true;
        }
        if (!pages.TryGetValue(category, out var page)) page = pages["general"];
        PageHost.Children.Clear();
        PageHost.Children.Add(page);
        if (category == "clipboard") SetClipboardMode(workflowsMode);
        else workflowsMode = false;
    }

    public void RefreshService(JsonElement snapshot)
    {
        serviceStatus.Text = $"服务版本 {snapshot.String("package_version")} · {(snapshot.Bool("network_running") ? "网络运行中" : "网络未就绪")}";
        if (snapshot.TryGetProperty("file_cache", out var cache))
            cacheStatus.Text = $"{MainWindow.FormatBytes(cache.Number("used_bytes"))} / {MainWindow.FormatBytes(cache.Number("max_bytes"))}";
        sync.IsOn = clipboard.SyncEnabled;
        capture.IsOn = clipboard.CaptureEnabled;
    }

    private void SetKeepAwake(bool enabled)
    {
        if (!clipboard.SetKeepAwake(enabled))
        {
            updatingAwake = true;
            try { awake.IsOn = clipboard.KeepAwake; } finally { updatingAwake = false; }
            generalStatus.Text = "保持唤醒设置无法保存，请检查当前用户目录权限。";
            return;
        }
        if (!power.SetEnabled(enabled))
        {
            clipboard.SetKeepAwake(!enabled);
            updatingAwake = true;
            try { awake.IsOn = clipboard.KeepAwake; } finally { updatingAwake = false; }
            generalStatus.Text = "Windows 未接受保持唤醒请求，设置未启用。";
            return;
        }
        generalStatus.Text = enabled ? "已阻止系统自动休眠和显示器休眠。" : "已恢复系统默认休眠行为。";
    }

    private async Task CheckUpdateAsync()
    {
        updateStatus.Text = "正在检查更新…";
        openRelease.Visibility = Visibility.Collapsed;
        try
        {
            Version? current = Version.TryParse(AppVersion.Display.Split(' ')[0], out var parsed) ? parsed : null;
            if (current is null) { updateStatus.Text = "无法读取当前版本号。"; return; }
            var result = await UpdateChecker.CheckAsync(current);
            updateStatus.Text = result.Summary;
            if (result.ReleaseUrl is string url && url.Length > 0)
            {
                openRelease.NavigateUri = new Uri(url);
                openRelease.Visibility = Visibility.Visible;
            }
        }
        catch (Exception) { updateStatus.Text = "更新检查未完成，请检查网络后重试。"; }
    }

    private async Task ExportDiagnosticsAsync()
    {
        try
        {
            await Task.Run(() =>
            {
                string path = DiagnosticsExport.Export(service.LatestSnapshot, clipboard.Entries.Count, clipboard.HistoryRetentionDays, clipboard.StorageError);
                var start = new System.Diagnostics.ProcessStartInfo("explorer.exe") { UseShellExecute = false };
                start.ArgumentList.Add("/select,");
                start.ArgumentList.Add(path);
                System.Diagnostics.Process.Start(start);
            });
            updateStatus.Text = "诊断文件已写入本地诊断目录；其中不含剪贴板正文、设备信任材料或文件内容。";
        }
        catch (Exception) { updateStatus.Text = "诊断导出失败，请检查用户存储目录权限。"; }
    }

    private void OpenFolder(string path)
    {
        try
        {
            Directory.CreateDirectory(path);
            System.Diagnostics.Process.Start(new System.Diagnostics.ProcessStartInfo("explorer.exe") { UseShellExecute = true, Arguments = path });
        }
        catch (Exception) { advancedStatus.Text = "无法打开文件夹。"; }
    }

    private async Task ClearHistoryAsync()
    {
        var dialog = new ContentDialog
        {
            XamlRoot = XamlRoot, Title = "清空所有本机剪贴板历史？",
            Content = "此操作无法撤销。用户源文件和已粘贴文件会保留。",
            PrimaryButtonText = "清空历史", CloseButtonText = "取消", DefaultButton = ContentDialogButton.Close,
        };
        if (await dialog.ShowAsync() != ContentDialogResult.Primary) return;
        try { clipboard.Clear(); advancedStatus.Text = "本机历史已清空。"; }
        catch (Exception) { advancedStatus.Text = "历史清理未完成，请检查用户存储目录权限。"; }
    }
}
