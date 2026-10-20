using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;

namespace DeskFuse.Windows;

/// <summary>Two-step first-run onboarding: runtime requirements, then feature
/// confirmation with the same card layout as the macOS onboarding flow.</summary>
public sealed partial class OnboardingPage : UserControl
{
    private readonly DesktopClipboardController clipboard;
    private readonly Action onCompleted;
    private int step = 1;
    private readonly ToggleSwitch syncToggle;
    private readonly ToggleSwitch captureToggle;

    public OnboardingPage(DesktopClipboardController clipboard, Action onCompleted)
    {
        this.clipboard = clipboard;
        this.onCompleted = onCompleted;
        InitializeComponent();
        syncToggle = new ToggleSwitch { IsOn = true, Header = "剪贴板" };
        captureToggle = new ToggleSwitch { IsOn = clipboard.CaptureEnabled, Header = "记录剪贴板历史" };
        ShowStep();
    }

    private void ShowStep()
    {
        bool first = step == 1;
        Step1Dot.Fill = (Brush)Application.Current.Resources[first ? "DFAccentBrush" : "DFControlStrongBrush"];
        Step2Dot.Fill = (Brush)Application.Current.Resources[first ? "DFControlStrongBrush" : "DFAccentBrush"];
        Step1Title.Foreground = (Brush)Application.Current.Resources[first ? "TextFillColorPrimaryBrush" : "DFMutedBrush"];
        Step2Title.Foreground = (Brush)Application.Current.Resources[first ? "DFMutedBrush" : "TextFillColorPrimaryBrush"];
        BackButton.Visibility = first ? Visibility.Collapsed : Visibility.Visible;
        NextButton.Content = first ? "下一步" : "开始使用";
        var panel = new StackPanel { MaxWidth = 640, Spacing = 12 };
        if (first) BuildRequirementsStep(panel); else BuildFeaturesStep(panel);
        StepHost.Content = new ScrollViewer
        {
            VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
            Content = panel,
        };
    }

    private void BuildRequirementsStep(StackPanel panel)
    {
        panel.Children.Add(Heading("运行环境", "DeskFuse 在本机运行服务与输入辅助进程；完成引导后即可发现附近设备。"));
        panel.Children.Add(Card(
            SectionTitle("网络与发现"),
            Detail("发现、配对与传输通过本地网络完成。首次使用请允许 DeskFuse 访问本地网络（Windows 会在需要时提示）。")));
        panel.Children.Add(Card(
            SectionTitle("输入权限"),
            Detail("普通桌面的键鼠共享开箱可用；控制其他应用的提升窗口需要签名并安装到受信任位置的版本。登录、UAC 与锁屏桌面始终不可用。")));
        panel.Children.Add(Card(
            SectionTitle("隐私"),
            Detail("所有数据只在你的设备之间传输；诊断导出不包含剪贴板正文、设备信任材料或文件内容。")));
    }

    private void BuildFeaturesStep(StackPanel panel)
    {
        panel.Children.Add(Heading("功能确认", "默认开启剪贴板、键鼠共享与远程应用；进阶功能可按需开启，之后随时在设置中调整。"));
        panel.Children.Add(FeatureCard("\uE8C8", "剪贴板", "跨设备复制粘贴，历史只属于当前用户。", syncToggle));
        panel.Children.Add(FeatureCard("\uE962", "键鼠共享", "一套键鼠跨屏操作；由服务端设置管理，Beta。", ManagedPlaceholder()));
        panel.Children.Add(FeatureCard("\uE8A7", "远程应用", "在本机运行远端应用窗口；由服务端设置管理，Beta。", ManagedPlaceholder()));
        panel.Children.Add(FeatureCard("\uE720", "麦克风共享", "跨设备共享麦克风；Beta，即将在 Windows 提供。", new ToggleSwitch { IsOn = false, IsEnabled = false }));
        panel.Children.Add(FeatureCard("\uE8B1", "鼠标按键", "侧键自定义；Beta，即将在 Windows 提供。", new ToggleSwitch { IsOn = false, IsEnabled = false }));
        panel.Children.Add(FeatureCard("\uE8C8", "记录剪贴板历史", "保留本机历史与保留天数设置；停止记录不影响同步。", captureToggle));
    }

    private static ToggleSwitch ManagedPlaceholder() => new() { IsOn = true, IsEnabled = false };

    // Styles are applied as direct properties: styles fetched from
    // Application.Current.Resources carry ThemeResource setters that do not
    // resolve on code-created elements, leaving them blank at render time.
    private static Brush DFRes(string key) => (Brush)Application.Current.Resources[key];

    private static StackPanel Heading(string title, string detail) => new()
    {
        Spacing = 8,
        Margin = new Thickness(0, 0, 0, 8),
        Children =
        {
            new TextBlock { Text = title, FontSize = 20, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold },
            new TextBlock { Text = detail, FontSize = 12, Foreground = DFRes("DFMutedBrush"), TextWrapping = TextWrapping.Wrap },
        },
    };

    private static TextBlock SectionTitle(string text) => new()
    {
        Text = text,
        FontSize = 13,
        FontWeight = Microsoft.UI.Text.FontWeights.SemiBold,
    };

    private static Border Card(params UIElement[] children)
    {
        var panel = new StackPanel { Spacing = 8 };
        foreach (var child in children) panel.Children.Add(child);
        return new Border
        {
            Background = DFRes("DFSurfaceBrush"),
            BorderBrush = DFRes("DFLineBrush"),
            BorderThickness = new Thickness(1),
            CornerRadius = new CornerRadius(12),
            Padding = new Thickness(16),
            Child = panel,
        };
    }

    private static TextBlock Detail(string text) => new()
    {
        Text = text,
        FontSize = 12,
        Foreground = DFRes("DFMutedBrush"),
        TextWrapping = TextWrapping.Wrap,
    };

    private static Border FeatureCard(string glyph, string title, string detail, ToggleSwitch toggle)
    {
        var icon = new FontIcon { FontSize = 20, Foreground = DFRes("DFAccentBrush"), Glyph = glyph };
        var header = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 10, Children = { icon, new TextBlock { Text = title, FontSize = 13, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold, VerticalAlignment = VerticalAlignment.Center } } };
        var root = new Grid();
        root.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        root.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var left = new StackPanel { Spacing = 6, Children = { header, Detail(detail) } };
        root.Children.Add(left);
        toggle.VerticalAlignment = VerticalAlignment.Center;
        Grid.SetColumn(toggle, 1);
        root.Children.Add(toggle);
        return Card(root);
    }

    private void BackClick(object sender, RoutedEventArgs args)
    {
        if (step > 1) { step -= 1; ShowStep(); }
    }

    private void NextClick(object sender, RoutedEventArgs args)
    {
        if (step == 1) { step = 2; ShowStep(); return; }
        clipboard.SyncEnabled = syncToggle.IsOn;
        clipboard.CaptureEnabled = captureToggle.IsOn;
        onCompleted();
    }
}
