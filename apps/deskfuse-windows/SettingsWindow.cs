using System.Text.Json;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace DeskFuse.Windows;

/// <summary>Settings window: sectioned cards, native Fluent metrics.</summary>
public sealed class SettingsWindow
{
    private readonly Window window = new();
    private readonly TextBlock serviceStatus = new() { Style = (Style)Application.Current.Resources["CaptionTextBlockStyle"], Foreground = (Microsoft.UI.Xaml.Media.Brush)Application.Current.Resources["TextFillColorSecondaryBrush"], Margin = new Thickness(0, 6, 0, 0) };
    private readonly TextBlock historyStatus = new() { Style = (Style)Application.Current.Resources["CaptionTextBlockStyle"], Foreground = (Microsoft.UI.Xaml.Media.Brush)Application.Current.Resources["TextFillColorSecondaryBrush"], Margin = new Thickness(0, 10, 0, 0) };

    public SettingsWindow(DesktopClipboardController clipboard, ServiceClient service)
    {
        AppIcon.Apply(window);
        window.AppWindow.Title = "DeskFuse 设置";
        window.AppWindow.Resize(new global::Windows.Graphics.SizeInt32(640, 720));

        var scroll = new ScrollViewer { VerticalScrollBarVisibility = ScrollBarVisibility.Auto, Padding = new Thickness(32, 24, 32, 24) };
        var content = new StackPanel { MaxWidth = 560, Spacing = 16 };
        scroll.Content = content;
        window.Content = scroll;

        // 标题
        content.Children.Add(new TextBlock { Text = "设置", FontSize = 28, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold });
        content.Children.Add(serviceStatus);

        // 本机剪贴板
        content.Children.Add(Section("本机剪贴板", new StackPanel
        {
            Spacing = 12,
            Children =
            {
                CreateCaptureToggle(clipboard),
                new TextBlock
                {
                    Text = "停止记录不关闭已单独开启的同步。只保存当前用户的历史。启动与重连不会自动恢复或重新发送历史内容。跨设备同步在主窗口单独开启。",
                    Foreground = (Microsoft.UI.Xaml.Media.Brush)Application.Current.Resources["TextFillColorSecondaryBrush"],
                    TextWrapping = TextWrapping.Wrap,
                },
                new Button { Content = "清空本机历史", HorizontalAlignment = HorizontalAlignment.Left, Margin = new Thickness(0, 0, 0, 0) }
                    .Also(button => button.Click += (_, _) =>
                    {
                        try { clipboard.Clear(); historyStatus.Text = "本机历史已清空；用户源文件和已粘贴文件未删除。"; }
                        catch (Exception) { historyStatus.Text = "历史清理未完成，请检查用户存储目录权限。"; }
                    }),
                historyStatus,
            },
        }));

        // 文件与缓存
        content.Children.Add(Section("文件与缓存", new StackPanel
        {
            Children =
            {
                new TextBlock
                {
                    Text = "本地暂存只保存引用，不预读文件正文。跨设备文件按需传输已接入：远端文件完整下载并逐文件校验后落地，缓存位于服务保护目录；接收的文件在传输页选择目标位置。Explorer OLE 按完整目录 manifest 延迟下载，首次粘贴请求时才读取并校验文件正文。",
                    Foreground = (Microsoft.UI.Xaml.Media.Brush)Application.Current.Resources["TextFillColorSecondaryBrush"],
                    TextWrapping = TextWrapping.Wrap,
                },
            },
        }));

        // 版本与更新
        content.Children.Add(Section("版本与更新", new StackPanel
        {
            Spacing = 8,
            Children =
            {
                new TextBlock { Text = $"DeskFuse {SupportLinks.VersionLabel}" },
                new TextBlock
                {
                    Text = "Windows 自动更新安装通道尚未接入。请使用匹配版本的 Windows 安装包更新客户端与服务。",
                    Foreground = (Microsoft.UI.Xaml.Media.Brush)Application.Current.Resources["TextFillColorSecondaryBrush"],
                    TextWrapping = TextWrapping.Wrap,
                },
                new TextBlock
                {
                    Text = "关闭主窗口保留系统托盘；从托盘选择“退出 DeskFuse”才结束客户端。Windows 服务由系统独立管理。",
                    Style = (Style)Application.Current.Resources["CaptionTextBlockStyle"],
                    Foreground = (Microsoft.UI.Xaml.Media.Brush)Application.Current.Resources["TextFillColorSecondaryBrush"],
                    TextWrapping = TextWrapping.Wrap,
                },
            },
        }));

        // 帮助与反馈
        var feedback = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 10 };
        var report = new Button { Content = "反馈问题…" };
        report.Click += (_, _) => SupportLinks.ReportProblem(service.LatestSnapshot);
        var known = new Button { Content = "已报告的问题" };
        known.Click += (_, _) => SupportLinks.Open(SupportLinks.IssuesUrl);
        feedback.Children.Add(report);
        feedback.Children.Add(known);
        content.Children.Add(Section("帮助与反馈", new StackPanel
        {
            Spacing = 12,
            Children =
            {
                new TextBlock
                {
                    Text = "遇到问题时打开反馈页，版本、系统和服务状态已经填好，只需描述发生了什么。",
                    Foreground = (Microsoft.UI.Xaml.Media.Brush)Application.Current.Resources["TextFillColorSecondaryBrush"],
                    TextWrapping = TextWrapping.Wrap,
                },
                feedback,
            },
        }));

        if (service.LatestSnapshot is JsonElement snapshot) RefreshService(snapshot);
    }

    private static UIElement Section(string title, UIElement body)
    {
        var panel = new StackPanel { Spacing = 10 };
        panel.Children.Add(new TextBlock { Text = title, FontSize = 16, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold });
        var card = new Border
        {
            Background = (Microsoft.UI.Xaml.Media.Brush)Application.Current.Resources["CardBackgroundFillColorDefaultBrush"],
            BorderBrush = (Microsoft.UI.Xaml.Media.Brush)Application.Current.Resources["CardStrokeColorDefaultBrush"],
            BorderThickness = new Thickness(1),
            CornerRadius = new CornerRadius(8),
            Padding = new Thickness(20),
            Child = body,
        };
        panel.Children.Add(card);
        return panel;
    }

    private static CheckBox CreateCaptureToggle(DesktopClipboardController clipboard)
    {
        var capture = new CheckBox { Content = "记录新的剪贴板历史", IsChecked = clipboard.CaptureEnabled };
        capture.Checked += (_, _) => clipboard.CaptureEnabled = true;
        capture.Unchecked += (_, _) => clipboard.CaptureEnabled = false;
        return capture;
    }

    public void RefreshService(JsonElement snapshot) => serviceStatus.Text = $"服务版本 {snapshot.String("package_version")} · {(snapshot.Bool("network_running") ? "网络运行中" : "网络未就绪")}";

    public void Show() { window.AppWindow.Show(); window.Activate(); }
}

internal static class FluentExtensions
{
    public static T Also<T>(this T value, Action<T> action) { action(value); return value; }
}
