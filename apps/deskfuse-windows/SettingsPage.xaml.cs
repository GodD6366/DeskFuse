using System.Text.Json;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;

namespace DeskFuse.Windows;

/// <summary>Settings inlined in the main window: horizontal category bar and a
/// content host, mirroring the macOS settings layout. Content is built in code
/// so the existing controls (retention editor, update checker, diagnostics)
/// move over unchanged.</summary>
public sealed partial class SettingsPage : UserControl
{
    private readonly Dictionary<string, UIElement> pages = [];
    private readonly ListBoxItem clipboardHistoryTab = new() { Content = "历史" };
    private readonly ListBoxItem clipboardWorkflowsTab = new() { Content = "工作流" };
    private readonly ListBox clipboardMode = new()
    {
        Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent),
        BorderThickness = new Thickness(0),
    };
    private readonly UIElement clipboardHistoryPanel;
    private readonly UIElement workflows;
    private readonly Action loadWorkflows;
    private readonly ServiceClient service;
    private readonly DesktopClipboardController clipboard;
    private readonly PowerStateController power;
    private readonly TextBlock serviceStatus = Description("");
    private readonly TextBlock generalStatus = Description("");
    private readonly TextBlock historyStatus = Description("");
    private readonly TextBlock advancedStatus = Description("");
    private readonly CheckBox sync;
    private bool updatingAwake;

    public SettingsPage(DesktopClipboardController clipboard, ServiceClient service, PowerStateController power, UIElement workflows, Action loadWorkflows)
    {
        this.clipboard = clipboard;
        this.service = service;
        this.power = power;
        this.workflows = workflows;
        this.loadWorkflows = loadWorkflows;
        InitializeComponent();

        sync = new CheckBox { Content = "自动同步剪贴板", IsChecked = clipboard.SyncEnabled };
        sync.Checked += (_, _) => clipboard.SyncEnabled = true;
        sync.Unchecked += (_, _) => clipboard.SyncEnabled = false;
        var awake = new CheckBox { Content = "保持电脑唤醒", IsChecked = clipboard.KeepAwake };
        awake.Checked += (_, _) => { if (!updatingAwake) SetKeepAwake(awake, true); };
        awake.Unchecked += (_, _) => { if (!updatingAwake) SetKeepAwake(awake, false); };
        AddPage("general", "通用", Form(
            Section("剪贴板同步", new StackPanel { Spacing = 12, Children = { sync, Description("同步之后的新复制内容。开启时不会重新发送历史。") } }),
            Section("后台运行", new StackPanel { Spacing = 12, Children = { awake, Description("阻止系统自动休眠和显示器休眠；退出 DeskFuse 时自动恢复系统默认行为。"), generalStatus } }),
            Section("本机状态", serviceStatus)));

        var capture = new CheckBox { Content = "记录新的剪贴板历史", IsChecked = clipboard.CaptureEnabled };
        capture.Checked += (_, _) => clipboard.CaptureEnabled = true;
        capture.Unchecked += (_, _) => clipboard.CaptureEnabled = false;
        clipboardHistoryPanel = Section("本机历史", new StackPanel
        {
            Spacing = 12,
            Children = { capture, Description("停止记录不关闭已开启的同步。历史只属于当前用户。"), HistoryRetentionSettings() },
        });
        clipboardMode.Items.Add(clipboardHistoryTab);
        clipboardMode.Items.Add(clipboardWorkflowsTab);
        clipboardMode.SelectionChanged += (_, _) =>
        {
            bool history = ReferenceEquals(clipboardMode.SelectedItem, clipboardHistoryTab);
            clipboardHistoryPanel.Visibility = history ? Visibility.Visible : Visibility.Collapsed;
            workflows.Visibility = history ? Visibility.Collapsed : Visibility.Visible;
            if (!history) loadWorkflows();
        };
        var clipboardRoot = new StackPanel { Spacing = 16 };
        clipboardMode.Visibility = Visibility.Collapsed; // shown by Show() when workflows are enabled
        clipboardRoot.Children.Add(clipboardMode);
        clipboardRoot.Children.Add(clipboardHistoryPanel);
        clipboardRoot.Children.Add(workflows);
        AddPage("clipboard", "剪贴板", new ScrollViewer { Content = clipboardRoot, Padding = new Thickness(24, 0, 24, 24), VerticalScrollBarVisibility = ScrollBarVisibility.Auto });

        AddPage("shortcuts", "快捷键", Form(
            Section("键鼠控制", new StackPanel
            {
                Spacing = 12,
                Children =
                {
                    Description("方向与锁定快捷键在「键鼠共享」页面编辑，保存后由输入运行时校验并持久化。"),
                    Description("紧急收回：Ctrl + Alt + Win + Esc；文件暂存区快捷键在设置 → 剪贴板 → 工作流中配置。"),
                },
            }),
            Section("工作流快捷键", new StackPanel
            {
                Spacing = 12,
                Children = { Description("在设置 → 剪贴板 → 工作流中为每个工作流分配快捷键；快捷键通过受认证 IPC 调度到 Windows 服务执行。") },
            })));

        AddPage("files", "文件", Form(
            Section("文件传输", Description("默认在目标设备使用文件时传输。在「局域网传输」查看任务、接收文件和打开暂存区。")),
            Section("接收位置", Description("接收的文件经过完整校验后写入本地暂存区，并放入本机剪贴板。"))));

        AddPage("permissions", "权限", Form(
            Section("输入权限", new StackPanel
            {
                Spacing = 12,
                Children =
                {
                    Description("控制其他应用的提升窗口需要签名并安装到受信任位置的 UIAccess 版本；当前运行的构建使用普通权限。"),
                    Description("普通桌面输入（本机与已授权对端）在当前权限下可用；登录、UAC 与锁屏桌面始终不可用。"),
                },
            })));

        var export = new Button { Content = "导出诊断文件…", HorizontalAlignment = HorizontalAlignment.Left };
        export.Click += async (_, _) =>
        {
            export.IsEnabled = false;
            try
            {
                await System.Threading.Tasks.Task.Run(() =>
                {
                    string path = DiagnosticsExport.Export(service.LatestSnapshot, clipboard.Entries.Count, clipboard.HistoryRetentionDays, clipboard.StorageError);
                    var start = new System.Diagnostics.ProcessStartInfo("explorer.exe") { UseShellExecute = false };
                    start.ArgumentList.Add("/select,");
                    start.ArgumentList.Add(path);
                    System.Diagnostics.Process.Start(start);
                });
                generalStatus.Text = "诊断文件已写入本地诊断目录；其中不含剪贴板正文、设备信任材料或文件内容。";
            }
            catch (Exception) { generalStatus.Text = "诊断导出失败，请检查用户存储目录权限。"; }
            finally { export.IsEnabled = true; }
        };
        var updateStatus = Description("检查更新会先读取 GitHub Releases（canonical），仅在其失败或为空时回退到独立域名镜像。");
        var openRelease = new HyperlinkButton { Content = "打开发布页", Visibility = Visibility.Collapsed, HorizontalAlignment = HorizontalAlignment.Left };
        var checkUpdate = new Button { Content = "检查更新", HorizontalAlignment = HorizontalAlignment.Left };
        checkUpdate.Click += async (_, _) =>
        {
            checkUpdate.IsEnabled = false;
            updateStatus.Text = "正在检查更新…";
            openRelease.Visibility = Visibility.Collapsed;
            try
            {
                Version? current = Version.TryParse(AppVersion.Display.Split(' ')[0], out var parsed) ? parsed : null;
                if (current is null)
                {
                    updateStatus.Text = "无法读取当前版本号。";
                    return;
                }
                var result = await UpdateChecker.CheckAsync(current);
                updateStatus.Text = result.Summary;
                if (result.ReleaseUrl is string url && url.Length > 0)
                {
                    openRelease.NavigateUri = new Uri(url);
                    openRelease.Visibility = Visibility.Visible;
                }
            }
            catch (Exception) { updateStatus.Text = "更新检查未完成，请检查网络后重试。"; }
            finally { checkUpdate.IsEnabled = true; }
        };
        var feedback = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 10 };
        var report = new Button { Content = "反馈问题…" };
        report.Click += (_, _) => SupportLinks.ReportProblem(service.LatestSnapshot);
        var known = new Button { Content = "已报告的问题" };
        known.Click += (_, _) => SupportLinks.Open(SupportLinks.IssuesUrl);
        feedback.Children.Add(report);
        feedback.Children.Add(known);
        AddPage("about", "关于与更新", Form(
            Section("DeskFuse", new StackPanel
            {
                Spacing = 12,
                Children =
                {
                    new TextBlock { Text = "在所有设备间，自由流转", FontSize = 12, Foreground = DFRes("DFMutedBrush"), TextWrapping = TextWrapping.Wrap },
                    new TextBlock { Text = $"DeskFuse {AppVersion.Display}" },
                },
            }),
            Section("更新", new StackPanel
            {
                Spacing = 12,
                Children =
                {
                    checkUpdate,
                    updateStatus,
                    openRelease,
                    Description("Windows 安装包与自动安装更新尚未随发行提供；检查更新只报告状态，不会下载或安装任何内容。"),
                },
            }),
            Section("反馈与诊断", new StackPanel
            {
                Spacing = 12,
                Children =
                {
                    Description("遇到问题时打开反馈页，版本、系统和服务状态已经填好，只需描述发生了什么。"),
                    feedback,
                    export,
                    Description("收集服务状态、客户端内存与历史统计到一个 JSON 文件，便于上报问题。"),
                },
            })));

        var clear = new Button { Content = "清空剪贴板历史…", HorizontalAlignment = HorizontalAlignment.Left };
        clear.Click += async (_, _) =>
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
        };
        AddPage("advanced", "高级", Form(
            Section("数据", new StackPanel { Spacing = 12, Children = { clear, advancedStatus } }),
            Section("工作流", Description("工作流为进阶功能，在设置 → 剪贴板 → 工作流中管理。"))));

    }

    private void NavigationChanged(object sender, SelectionChangedEventArgs args)
    {
        if (Navigation.SelectedItem is not ListBoxItem { Tag: string key }) return;
        Page.Content = pages[key];
        clipboardMode.Visibility = key == "clipboard" ? Visibility.Visible : Visibility.Collapsed;
        if (key == "clipboard") SelectClipboardMode(clipboardHistoryTab);
    }

    private void SelectClipboardMode(ListBoxItem tab)
    {
        clipboardMode.SelectedItem = tab;
    }

    private UIElement HistoryRetentionSettings()
    {
        var days = new NumberBox
        {
            Header = "保留天数", Value = clipboard.HistoryRetentionDays, Minimum = 1, Maximum = int.MaxValue,
            SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Compact,
            ValidationMode = NumberBoxValidationMode.Disabled, HorizontalAlignment = HorizontalAlignment.Left, Width = 200,
        };
        var message = Description("");
        var save = new Button { Content = "保存", IsEnabled = false };
        var revert = new Button { Content = "撤销修改", IsEnabled = false };
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
        return new StackPanel
        {
            Spacing = 12,
            Children =
            {
                days,
                Description("默认保留 30 天。到期记录会在后台清理，关闭面板不影响清理。历史不设条数或累计大小上限；单次跨设备传输仍遵守传输通道限制。"),
                new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, Children = { save, revert } },
                message,
            },
        };
    }

    private void SetKeepAwake(CheckBox toggle, bool enabled)
    {
        if (!clipboard.SetKeepAwake(enabled))
        {
            updatingAwake = true;
            try { toggle.IsChecked = clipboard.KeepAwake; } finally { updatingAwake = false; }
            generalStatus.Text = "保持唤醒设置无法保存，请检查当前用户目录权限。";
            return;
        }
        if (!power.SetEnabled(enabled))
        {
            clipboard.SetKeepAwake(!enabled);
            updatingAwake = true;
            try { toggle.IsChecked = clipboard.KeepAwake; } finally { updatingAwake = false; }
            generalStatus.Text = "Windows 未接受保持唤醒请求，设置未启用。";
            return;
        }
        generalStatus.Text = enabled ? "已阻止系统自动休眠和显示器休眠。" : "已恢复系统默认休眠行为。";
    }

    private void AddPage(string key, string label, UIElement content)
    {
        pages.Add(key, content);
        Navigation.Items.Add(new ListBoxItem { Tag = key, Content = label });
    }

    private static UIElement Form(params UIElement[] sections)
    {
        var content = new StackPanel { Spacing = 24, MaxWidth = 720 };
        foreach (var section in sections) content.Children.Add(section);
        return new ScrollViewer { Content = content, Padding = new Thickness(24, 0, 24, 24), VerticalScrollBarVisibility = ScrollBarVisibility.Auto };
    }

    // Styles are applied as direct properties: styles fetched from
    // Application.Current.Resources carry ThemeResource setters that do not
    // resolve on code-created elements, leaving them blank at render time.
    private static Brush DFRes(string key) => (Brush)Application.Current.Resources[key];

    private static TextBlock Description(string text) => new()
    {
        Text = text, TextWrapping = TextWrapping.Wrap,
        FontSize = 12, Foreground = DFRes("DFMutedBrush"),
    };

    private static UIElement Section(string heading, UIElement body) => new StackPanel
    {
        Spacing = 10,
        Children =
        {
            new TextBlock { Text = heading, FontSize = 13, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold },
            new Border
            {
                Background = DFRes("DFSurfaceBrush"),
                BorderBrush = DFRes("DFLineBrush"),
                BorderThickness = new Thickness(1),
                CornerRadius = new CornerRadius(12),
                Padding = new Thickness(16),
                Child = body,
            },
        },
    };

    public void RefreshService(JsonElement snapshot) => serviceStatus.Text = $"服务版本 {snapshot.String("package_version")} · {(snapshot.Bool("network_running") ? "网络运行中" : "网络未就绪")}";

    public void Show(string category = "general")
    {
        if (service.LatestSnapshot is JsonElement snapshot) RefreshService(snapshot);
        sync.IsChecked = clipboard.SyncEnabled;
        if (category == "workflows")
        {
            // Workflows live in 设置 → 剪贴板 as a segment, matching macOS.
            Navigation.SelectedItem = Navigation.Items.OfType<ListBoxItem>().Single(x => x.Tag as string == "clipboard");
            SelectClipboardMode(clipboardWorkflowsTab);
            loadWorkflows();
            return;
        }
        Navigation.SelectedItem = Navigation.Items.OfType<ListBoxItem>().Single(x => x.Tag as string == category);
    }
}
