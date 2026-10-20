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
        StepBody.Children.Clear();
        if (first) BuildRequirementsStep(); else BuildFeaturesStep();
    }

    private void BuildRequirementsStep()
    {
        StepBody.Children.Add(Heading("运行环境", "DeskFuse 在本机运行服务与输入辅助进程；完成引导后即可发现附近设备。"));
        StepBody.Children.Add(Card(
            new TextBlock { Text = "网络与发现", Style = (Style)Application.Current.Resources["DFSectionTextStyle"] },
            Detail("发现、配对与传输通过本地网络完成。首次使用请允许 DeskFuse 访问本地网络（Windows 会在需要时提示）。")));
        StepBody.Children.Add(Card(
            new TextBlock { Text = "输入权限", Style = (Style)Application.Current.Resources["DFSectionTextStyle"] },
            Detail("普通桌面的键鼠共享开箱可用；控制其他应用的提升窗口需要签名并安装到受信任位置的版本。登录、UAC 与锁屏桌面始终不可用。")));
        StepBody.Children.Add(Card(
            new TextBlock { Text = "隐私", Style = (Style)Application.Current.Resources["DFSectionTextStyle"] },
            Detail("所有数据只在你的设备之间传输；诊断导出不包含剪贴板正文、设备信任材料或文件内容。")));
    }

    private void BuildFeaturesStep()
    {
        StepBody.Children.Add(Heading("功能确认", "默认开启剪贴板、键鼠共享与远程应用；进阶功能可按需开启，之后随时在设置中调整。"));
        StepBody.Children.Add(FeatureCard("\uE8C8", "剪贴板", "跨设备复制粘贴，历史只属于当前用户。", syncToggle));
        StepBody.Children.Add(FeatureCard("\uE962", "键鼠共享", "一套键鼠跨屏操作；由服务端设置管理，Beta。", ManagedPlaceholder()));
        StepBody.Children.Add(FeatureCard("\uE8A7", "远程应用", "在本机运行远端应用窗口；由服务端设置管理，Beta。", ManagedPlaceholder()));
        StepBody.Children.Add(FeatureCard("\uE720", "麦克风共享", "跨设备共享麦克风；Beta，即将在 Windows 提供。", new ToggleSwitch { IsOn = false, IsEnabled = false }));
        StepBody.Children.Add(FeatureCard("\uE8B1", "鼠标按键", "侧键自定义；Beta，即将在 Windows 提供。", new ToggleSwitch { IsOn = false, IsEnabled = false }));
        StepBody.Children.Add(FeatureCard("\uE8C8", "记录剪贴板历史", "保留本机历史与保留天数设置；停止记录不影响同步。", captureToggle));
    }

    private static ToggleSwitch ManagedPlaceholder() => new() { IsOn = true, IsEnabled = false };

    private static StackPanel Heading(string title, string detail) => new()
    {
        Spacing = 8,
        Margin = new Thickness(0, 0, 0, 8),
        Children =
        {
            new TextBlock { Text = title, Style = (Style)Application.Current.Resources["DFPageTitleTextStyle"] },
            new TextBlock { Text = detail, Style = (Style)Application.Current.Resources["DFDetailTextStyle"] },
        },
    };

    private static Border Card(params UIElement[] children)
    {
        var panel = new StackPanel { Spacing = 8 };
        foreach (var child in children) panel.Children.Add(child);
        return new Border { Style = (Style)Application.Current.Resources["CardBorder"], Child = panel };
    }

    private static TextBlock Detail(string text) => new()
    {
        Text = text,
        Style = (Style)Application.Current.Resources["DFDetailTextStyle"],
    };

    private static Border FeatureCard(string glyph, string title, string detail, ToggleSwitch toggle)
    {
        var icon = new FontIcon { FontSize = 20, Foreground = (Brush)Application.Current.Resources["DFAccentBrush"], Glyph = glyph };
        var header = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 10, Children = { icon, new TextBlock { Text = title, Style = (Style)Application.Current.Resources["DFSectionTextStyle"], VerticalAlignment = VerticalAlignment.Center } } };
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
