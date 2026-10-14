using System.Text.Json;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;

namespace DeskFuse.Windows;

/// <summary>Persistent settings navigation with the existing workflow editor hosted in place.</summary>
public sealed class SettingsWindow
{
    private readonly Window window = new();
    private readonly ListBox navigation = new() { Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent), BorderThickness = new Thickness(0) };
    private readonly ContentControl page = new() { HorizontalContentAlignment = HorizontalAlignment.Stretch, VerticalContentAlignment = VerticalAlignment.Stretch };
    private readonly TextBlock title = new() { FontSize = 28, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold, Margin = new Thickness(24, 24, 24, 16) };
    private readonly TextBlock serviceStatus = Description("");
    private readonly TextBlock generalStatus = Description("");
    private readonly TextBlock historyStatus = Description("");
    private readonly Dictionary<string, UIElement> pages = [];
    private readonly Action loadWorkflows;
    private readonly ServiceClient service;
    private readonly DesktopClipboardController clipboard;
    private readonly PowerStateController power;
    private readonly CheckBox sync;
    private bool updatingAwake;

    public SettingsWindow(DesktopClipboardController clipboard, ServiceClient service, PowerStateController power, UIElement workflows, Action loadWorkflows)
    {
        this.service = service;
        this.clipboard = clipboard;
        this.power = power;
        this.loadWorkflows = loadWorkflows;
        AppIcon.Apply(window);
        window.AppWindow.Title = "DeskFuse 设置";
        var id = Microsoft.UI.Win32Interop.GetWindowIdFromWindow(WinRT.Interop.WindowNative.GetWindowHandle(window));
        var work = Microsoft.UI.Windowing.DisplayArea.GetFromWindowId(id, Microsoft.UI.Windowing.DisplayAreaFallback.Nearest).WorkArea;
        window.AppWindow.Resize(new global::Windows.Graphics.SizeInt32(Math.Min(1120, work.Width), Math.Min(800, work.Height)));
        window.AppWindow.Closing += (_, args) => { args.Cancel = true; window.AppWindow.Hide(); };

        var root = new Grid();
        root.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(196) });
        root.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        navigation.Margin = new Thickness(12, 24, 12, 12);
        root.Children.Add(navigation);
        var body = new Grid();
        body.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        body.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        body.Children.Add(title);
        Grid.SetRow(page, 1);
        body.Children.Add(page);
        Grid.SetColumn(body, 1);
        root.Children.Add(body);
        window.Content = root;

        sync = new CheckBox { Content = "自动同步剪贴板", IsChecked = clipboard.SyncEnabled };
        sync.Checked += (_, _) => clipboard.SyncEnabled = true;
        sync.Unchecked += (_, _) => clipboard.SyncEnabled = false;
        var awake = new CheckBox { Content = "保持电脑唤醒", IsChecked = clipboard.KeepAwake };
        awake.Checked += (_, _) => { if (!updatingAwake) SetKeepAwake(awake, true); };
        awake.Unchecked += (_, _) => { if (!updatingAwake) SetKeepAwake(awake, false); };
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
        AddPage("general", "通用", Form(
            Section("剪贴板同步", new StackPanel { Spacing = 12, Children = { sync, Description("同步之后的新复制内容。开启时不会重新发送历史。") } }),
            Section("后台运行", new StackPanel { Spacing = 12, Children = { awake, Description("阻止系统自动休眠和显示器休眠；退出 DeskFuse 时自动恢复系统默认行为。"), generalStatus } }),
            Section("诊断", new StackPanel { Spacing = 12, Children = { export, Description("收集服务状态、客户端内存与历史统计到一个 JSON 文件，便于上报问题。") } }),
            Section("本机状态", serviceStatus)));

        var capture = new CheckBox { Content = "记录新的剪贴板历史", IsChecked = clipboard.CaptureEnabled };
        capture.Checked += (_, _) => clipboard.CaptureEnabled = true;
        capture.Unchecked += (_, _) => clipboard.CaptureEnabled = false;
        AddPage("clipboard", "剪贴板", Form(Section("本机历史", new StackPanel
        {
            Spacing = 12,
            Children = { capture, Description("停止记录不关闭已开启的同步。历史只属于当前用户。"), HistoryRetentionSettings(root) },
        })));

        var clear = new Button { Content = "清空剪贴板历史…", HorizontalAlignment = HorizontalAlignment.Left };
        clear.Click += async (_, _) =>
        {
            var dialog = new ContentDialog
            {
                XamlRoot = root.XamlRoot, Title = "清空所有本机剪贴板历史？",
                Content = "此操作无法撤销。用户源文件和已粘贴文件会保留。",
                PrimaryButtonText = "清空历史", CloseButtonText = "取消", DefaultButton = ContentDialogButton.Close,
            };
            if (await dialog.ShowAsync() != ContentDialogResult.Primary) return;
            try { clipboard.Clear(); historyStatus.Text = "本机历史已清空。"; }
            catch (Exception) { historyStatus.Text = "历史清理未完成，请检查用户存储目录权限。"; }
        };
        generalStatus.Text = "";
        AddPage("storage", "存储", Form(
            Section("文件传输", Description("默认在目标设备使用文件时传输。在“分享与传输”查看任务、接收文件和打开暂存区。")),
            Section("本地存储", new StackPanel { Spacing = 12, Children = { clear, historyStatus } })));
        AddPage("workflows", "工作流", workflows);
        var feedback = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 10 };
        var report = new Button { Content = "反馈问题…" };
        report.Click += (_, _) => SupportLinks.ReportProblem(service.LatestSnapshot);
        var known = new Button { Content = "已报告的问题" };
        known.Click += (_, _) => SupportLinks.Open(SupportLinks.IssuesUrl);
        feedback.Children.Add(report);
        feedback.Children.Add(known);
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
        AddPage("updates", "更新", Form(
            Section("版本与更新", new StackPanel
            {
                Spacing = 12,
                Children =
                {
                    new TextBlock { Text = $"DeskFuse {AppVersion.Display}" },
                    checkUpdate,
                    updateStatus,
                    openRelease,
                    Description("Windows 安装包与自动安装更新尚未随发行提供；检查更新只报告状态，不会下载或安装任何内容。"),
                },
            }),
            Section("帮助与反馈", new StackPanel
            {
                Spacing = 12,
                Children =
                {
                    Description("遇到问题时打开反馈页，版本、系统和服务状态已经填好，只需描述发生了什么。"),
                    feedback,
                },
            })));
        navigation.SelectionChanged += (_, _) =>
        {
            if (navigation.SelectedItem is not ListBoxItem { Tag: string key } item) return;
            title.Text = item.Content as string ?? "设置";
            page.Content = pages[key];
            if (key == "workflows") loadWorkflows();
        };
    }

    private UIElement HistoryRetentionSettings(FrameworkElement root)
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
                        XamlRoot = root.XamlRoot, Title = "缩短历史保留时间？",
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
        navigation.Items.Add(new ListBoxItem { Tag = key, Content = label, Padding = new Thickness(12, 10, 12, 10), Margin = new Thickness(0, 0, 0, 4), CornerRadius = new CornerRadius(6) });
    }

    private static UIElement Form(params UIElement[] sections)
    {
        var content = new StackPanel { Spacing = 24 };
        foreach (var section in sections) content.Children.Add(section);
        return new ScrollViewer { Content = content, Padding = new Thickness(24, 0, 24, 24), VerticalScrollBarVisibility = ScrollBarVisibility.Auto };
    }

    private static TextBlock Description(string text) => new()
    {
        Text = text, TextWrapping = TextWrapping.Wrap,
        Foreground = (Brush)Application.Current.Resources["TextFillColorSecondaryBrush"],
    };

    private static UIElement Section(string heading, UIElement body) => new StackPanel
    {
        Spacing = 10,
        Children =
        {
            new TextBlock { Text = heading, FontSize = 16, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold },
            new Border
            {
                Background = (Brush)Application.Current.Resources["CardBackgroundFillColorDefaultBrush"],
                BorderBrush = (Brush)Application.Current.Resources["CardStrokeColorDefaultBrush"],
                BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(8), Padding = new Thickness(20), Child = body,
            },
        },
    };

    public void RefreshService(JsonElement snapshot) => serviceStatus.Text = $"服务版本 {snapshot.String("package_version")} · {(snapshot.Bool("network_running") ? "网络运行中" : "网络未就绪")}";

    public void Show(string category = "general")
    {
        if (service.LatestSnapshot is JsonElement snapshot) RefreshService(snapshot);
        sync.IsChecked = clipboard.SyncEnabled;
        var item = navigation.Items.OfType<ListBoxItem>().Single(x => x.Tag as string == category);
        if (ReferenceEquals(navigation.SelectedItem, item) && category == "workflows") loadWorkflows();
        navigation.SelectedItem = item;
        window.AppWindow.Show();
        window.Activate();
    }
}

internal static class FluentExtensions
{
    public static T Also<T>(this T value, Action<T> action) { action(value); return value; }
}
