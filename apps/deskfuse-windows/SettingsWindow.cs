using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using Button = System.Windows.Controls.Button;
using CheckBox = System.Windows.Controls.CheckBox;

namespace DeskFuse.Windows;

public sealed class SettingsWindow : Window
{
    private readonly TextBlock serviceStatus = new() { Text = "等待服务连接…", Margin = new Thickness(0, 12, 0, 20) };
    private readonly TextBlock historyStatus = new() { Margin = new Thickness(0, 10, 0, 24), Foreground = Brushes.SlateGray };
    public SettingsWindow(DesktopClipboardController clipboard, ServiceClient service)
    {
        Title = "DeskFuse 设置"; Width = 560; Height = 620; MinWidth = 460; MinHeight = 420; WindowStartupLocation = WindowStartupLocation.CenterOwner;
        var content = new StackPanel { Margin = new Thickness(28) };
        Content = new ScrollViewer { Content = content, VerticalScrollBarVisibility = ScrollBarVisibility.Auto };
        content.Children.Add(Heading("设置"));
        content.Children.Add(serviceStatus);
        content.Children.Add(Heading("本机剪贴板"));
        var capture = new CheckBox { Content = "记录新的剪贴板历史", IsChecked = clipboard.CaptureEnabled, Margin = new Thickness(0, 12, 0, 12) };
        capture.Checked += (_, _) => clipboard.CaptureEnabled = true;
        capture.Unchecked += (_, _) => clipboard.CaptureEnabled = false;
        content.Children.Add(capture);
        content.Children.Add(new TextBlock { Text = "停止记录不关闭已单独开启的同步。只保存当前用户的历史。启动与重连不会自动恢复或重新发送历史内容。跨设备同步在主窗口单独开启。", Foreground = Brushes.SlateGray });
        var clear = new Button { Content = "清空本机历史", HorizontalAlignment = HorizontalAlignment.Left, Margin = new Thickness(0, 18, 0, 0) };
        clear.Click += (_, _) =>
        {
            try { clipboard.Clear(); historyStatus.Text = "本机历史已清空；用户源文件和已粘贴文件未删除。"; }
            catch (Exception) { historyStatus.Text = "历史清理未完成，请检查用户存储目录权限。"; }
        };
        content.Children.Add(clear); content.Children.Add(historyStatus);
        content.Children.Add(Heading("文件与缓存"));
        content.Children.Add(new TextBlock { Text = "本地暂存只保存引用，不预读文件正文。跨设备文件按需传输已接入：远端文件完整下载并逐文件校验后落地，缓存位于服务保护目录；接收的文件在传输页选择目标位置。Explorer OLE 按完整目录 manifest 延迟下载，首次粘贴请求时才读取并校验文件正文。", Margin = new Thickness(0, 10, 0, 24), Foreground = Brushes.SlateGray });
        content.Children.Add(Heading("版本与更新"));
        content.Children.Add(new TextBlock { Text = $"DeskFuse {SupportLinks.VersionLabel}", Margin = new Thickness(0, 10, 0, 0) });
        content.Children.Add(new TextBlock { Text = "Windows 自动更新安装通道尚未接入。请使用匹配版本的 Windows 安装包更新客户端与服务。", Margin = new Thickness(0, 10, 0, 24), Foreground = Brushes.SlateGray });
        content.Children.Add(Heading("帮助与反馈"));
        content.Children.Add(new TextBlock { Text = "遇到问题时打开反馈页，版本、系统和服务状态已经填好，只需描述发生了什么。", Margin = new Thickness(0, 10, 0, 12), Foreground = Brushes.SlateGray, TextWrapping = TextWrapping.Wrap });
        var feedback = new StackPanel { Orientation = Orientation.Horizontal, Margin = new Thickness(0, 0, 0, 24) };
        var report = new Button { Content = "反馈问题…", Padding = new Thickness(12, 4, 12, 4) };
        report.Click += (_, _) => SupportLinks.ReportProblem(service.LatestSnapshot);
        var known = new Button { Content = "已报告的问题", Padding = new Thickness(12, 4, 12, 4), Margin = new Thickness(10, 0, 0, 0) };
        known.Click += (_, _) => SupportLinks.Open(SupportLinks.IssuesUrl);
        feedback.Children.Add(report); feedback.Children.Add(known);
        content.Children.Add(feedback);
        content.Children.Add(new TextBlock { Text = "关闭主窗口保留系统托盘；从托盘选择“退出 DeskFuse”才结束客户端。Windows 服务由系统独立管理。", FontSize = 12, Foreground = Brushes.SlateGray });
        if (service.LatestSnapshot is JsonElement snapshot) RefreshService(snapshot);
    }
    private static TextBlock Heading(string text) => new() { Text = text, FontSize = 21, FontWeight = FontWeights.SemiBold };
    public void RefreshService(JsonElement snapshot) => serviceStatus.Text = $"服务版本 {snapshot.String("package_version")} · {(snapshot.Bool("network_running") ? "网络运行中" : "网络未就绪")}";
}
