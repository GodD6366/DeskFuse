using System.Diagnostics;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;

namespace DeskFuse.Windows;

public sealed record OnboardingResult(bool Clipboard, bool Input, bool RemoteApplications);

/// <summary>Two-step first-run flow mirrored from the macOS OnboardingFlow:
/// access requirements first, then feature defaults. Choices are applied once
/// when the user finishes.</summary>
public sealed partial class OnboardingPage : UserControl
{
    private readonly DesktopClipboardController clipboard;
    private readonly Action<OnboardingResult> onCompleted;
    private readonly Dictionary<string, bool> enabled = new()
    {
        ["clipboard"] = true,
        ["input"] = true,
        ["remote"] = true,
        ["mouse"] = false,
        ["microphone"] = false,
    };
    private readonly List<Action<double>> demoTicks = [];
    private readonly Stopwatch demoClock = new();
    private bool demoRunning;
    private int step = 1;

    public OnboardingPage(DesktopClipboardController clipboard, Action<OnboardingResult> onCompleted)
    {
        this.clipboard = clipboard;
        this.onCompleted = onCompleted;
        InitializeComponent();
        UpdateChrome();
        Unloaded += (_, _) => StopDemos();
    }

    private void UpdateChrome()
    {
        bool first = step == 1;
        StepTitle.Text = first ? "系统权限" : "功能开关";
        StepCount.Text = $"第 {step} 步，共 2 步";
        FooterHint.Text = first ? "可以稍后授权，功能页会再次提示"
            : EnabledCount == 0 ? "所有功能保持关闭，随时可以再打开" : $"将开启 {EnabledCount} 项功能，随时可以调整";
        BackButton.Visibility = first ? Visibility.Collapsed : Visibility.Visible;
        NextButton.Content = first ? "下一步：选择功能" : "开始使用";
        BuildStepList();
        StepContent.Children.Clear();
        if (first)
        {
            StopDemos();
            BuildPermissionsStep();
        }
        else
        {
            BuildFeaturesStep();
        }
    }

    private int EnabledCount => new[] { "clipboard", "input", "remote", "mouse", "microphone" }.Count(kind => enabled[kind]);

    // MARK: - Step rail

    private void BuildStepList()
    {
        StepList.Children.Clear();
        StepList.Children.Add(StepRow(1, "系统权限", "允许必要的系统访问"));
        StepList.Children.Add(StepRow(2, "功能开关", "确认默认开启的功能"));
    }

    private UIElement StepRow(int index, string title, string detail)
    {
        bool active = step == index;
        bool completed = step > index;
        var grid = new Grid { ColumnSpacing = 12 };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        var circle = new Border
        {
            Width = 24, Height = 24, CornerRadius = new CornerRadius(12),
            Background = active ? DfUi.Accent : completed ? DfUi.Brush("DFSuccessSubtleBrush") : DfUi.Control,
            VerticalAlignment = VerticalAlignment.Top,
            Child = completed
                ? DfUi.Glyph("\uE73E", 11, DfUi.Success)
                : new TextBlock
                {
                    Text = index.ToString(), FontSize = 11, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold,
                    Foreground = active ? DfUi.Brush("DFAccentForegroundBrush") : DfUi.Muted,
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                },
        };
        grid.Children.Add(circle);
        var text = new StackPanel { Spacing = 2 };
        text.Children.Add(DfUi.Text(title, 13, true, active || completed ? DfUi.PrimaryText : DfUi.Muted));
        text.Children.Add(DfUi.Detail(detail));
        Grid.SetColumn(text, 1);
        grid.Children.Add(text);
        return grid;
    }

    // MARK: - Step 1

    private void BuildPermissionsStep()
    {
        StepContent.Children.Add(PageIntro("\uE72E", "先让 DeskFuse 获得必要的访问权限",
            "这些访问由 Windows 管理，只在本机使用；随时可以在系统设置或应用中更改。"));

        var progress = new Grid { ColumnSpacing = 16, Padding = new Thickness(12) };
        progress.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        progress.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var progressText = new StackPanel { Spacing = 2 };
        progressText.Children.Add(DfUi.Text("必要访问已就绪 2 / 2", 13, true));
        progressText.Children.Add(DfUi.Detail("已经可以开始使用全部核心功能。"));
        progress.Children.Add(progressText);
        var bar = new ProgressBar { Value = 100, Width = 160, VerticalAlignment = VerticalAlignment.Center };
        Grid.SetColumn(bar, 1);
        progress.Children.Add(bar);
        StepContent.Children.Add(new Border
        {
            Background = DfUi.Control,
            CornerRadius = new CornerRadius(8),
            Child = progress,
        });

        var required = new StackPanel { Spacing = 0 };
        required.Children.Add(PermissionRow("\uE774", "本地网络访问",
            "发现、配对与传输通过本地网络完成；Windows 会在首次发现设备时询问。", "按需授权", "DFAccentBrush"));
        required.Children.Add(DfUi.Divider(0, 0));
        required.Children.Add(PermissionRow("\uE962", "普通桌面输入",
            "普通桌面键鼠共享开箱可用；提升窗口需要 UIAccess 版本。", "已就绪", "DFSuccessBrush"));
        StepContent.Children.Add(DfUi.Section("必要权限", required,
            "键鼠共享需要普通桌面输入；本地网络访问会在首次使用时由 Windows 提示。"));

        var advanced = new StackPanel { Spacing = 0 };
        advanced.Children.Add(PermissionRow("\uE72E", "提升窗口控制",
            "控制其他应用的提升窗口需要签名并安装到受信任位置的 UIAccess 版本。", "未授权", "DFMutedBrush"));
        advanced.Children.Add(DfUi.Divider(0, 0));
        advanced.Children.Add(PermissionRow("\uE8D7", "登录、UAC 与锁屏桌面",
            "这些安全桌面始终不可注入。", "不可用", "DFDangerBrush"));
        advanced.Children.Add(DfUi.Divider(0, 0));
        advanced.Children.Add(PermissionRow("\uE720", "麦克风访问",
            "Windows 端尚未提供麦克风共享。", "暂不可用", "DFMutedBrush"));
        StepContent.Children.Add(DfUi.Section("进阶权限", advanced,
            "这些能力只在对应功能开启时才会用到，可以稍后再授权。"));

        var notice = new StackPanel { Spacing = 2 };
        notice.Children.Add(DfUi.Text("逐项授权，随时可改", 13, true));
        notice.Children.Add(DfUi.Detail("缺少访问时，对应功能页会再次提示；未授权的功能不会在后台工作。"));
        StepContent.Children.Add(new Border
        {
            Background = DfUi.Brush("DFAccentHoverBrush"),
            BorderBrush = DfUi.Brush("DFAccentSubtleBrush"),
            BorderThickness = new Thickness(1),
            CornerRadius = new CornerRadius(8),
            Padding = new Thickness(12),
            Child = notice,
        });
    }

    private static UIElement PermissionRow(string glyph, string title, string detail, string state, string tone)
    {
        var grid = new Grid { ColumnSpacing = 12, Padding = new Thickness(0, 8, 0, 8) };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(28) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var icon = DfUi.Glyph(glyph, 15, DfUi.Muted);
        icon.VerticalAlignment = VerticalAlignment.Top;
        icon.Margin = new Thickness(0, 2, 0, 0);
        grid.Children.Add(icon);
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

    private static UIElement PageIntro(string glyph, string headline, string subhead)
    {
        var grid = new Grid { ColumnSpacing = 16 };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.Children.Add(new Border
        {
            Width = 56, Height = 56, CornerRadius = new CornerRadius(16),
            Background = DfUi.Brush("DFAccentSubtleBrush"),
            Child = DfUi.Glyph(glyph, 24, DfUi.Accent),
        });
        var text = new StackPanel { Spacing = 4, VerticalAlignment = VerticalAlignment.Center };
        text.Children.Add(DfUi.Text(headline, 20, true));
        text.Children.Add(DfUi.Detail(subhead, 13));
        Grid.SetColumn(text, 1);
        grid.Children.Add(text);
        return grid;
    }

    // MARK: - Step 2

    private void BuildFeaturesStep()
    {
        StepContent.Children.Add(PageIntro("\uE703", "选择开机后默认开启的功能",
            "常用功能已经为你打开，进阶功能保持关闭；完成设置后也能随时调整。"));

        StepContent.Children.Add(FeatureSection("默认开启", "建议保留，完成设置后即可跨电脑使用。",
            ("clipboard", "\uE8C8", "剪贴板同步", "在一台电脑复制，另一台直接粘贴。文字、图片和文件都会同步。", false),
            ("input", "\uE962", "键鼠共享", "一套键盘和鼠标穿过屏幕边缘，控制同一网络里的其他电脑。", false),
            ("remote", "\uE8A7", "远程应用", "把另一台电脑上的应用窗口，直接打开在这台电脑上。", true)));
        StepContent.Children.Add(FeatureSection("按需开启", "保持关闭不影响日常使用，需要时再打开。",
            ("mouse", "\uE8B1", "鼠标按键", "把鼠标侧键映射成快捷键、窗口切换或系统操作。", true),
            ("microphone", "\uE720", "麦克风共享", "把另一台电脑的麦克风作为本机的输入设备。", true)));

        StepContent.Children.Add(DfUi.Detail("完成设置后，可在各功能页右上角的总开关或设置中随时修改。"));

        demoClock.Restart();
        if (!demoRunning)
        {
            demoRunning = true;
            CompositionTarget.Rendering += AnimateDemos;
        }
    }

    private UIElement FeatureSection(string title, string caption, params (string Key, string Glyph, string Title, string Detail, bool Beta)[] cards)
    {
        var section = new StackPanel { Spacing = 8 };
        section.Children.Add(DfUi.SectionLabel(title));
        section.Children.Add(DfUi.Detail(caption));
        var grid = new Grid { ColumnSpacing = 12, RowSpacing = 12 };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        for (int i = 0; i < cards.Length; i += 2)
        {
            grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
            for (int column = 0; column < 2 && i + column < cards.Length; column++)
            {
                var card = BuildFeatureCard(cards[i + column]);
                Grid.SetRow(card, i / 2);
                Grid.SetColumn(card, column);
                grid.Children.Add(card);
            }
        }
        section.Children.Add(grid);
        return section;
    }

    private Border BuildFeatureCard((string Key, string Glyph, string Title, string Detail, bool Beta) kind)
    {
        bool available = kind.Key is not ("mouse" or "microphone");
        bool isOn = enabled[kind.Key];
        var toggle = DfUi.Switch(isOn, null, available);
        var demo = BuildDemo(kind.Key);
        demo.Opacity = isOn ? 1 : 0.6;
        var demoSurface = new Border
        {
            Height = 112,
            Margin = new Thickness(12, 12, 12, 0),
            Background = DfUi.Control,
            CornerRadius = new CornerRadius(8),
            Child = demo,
        };
        var footer = new Grid { Padding = new Thickness(16), ColumnSpacing = 8 };
        footer.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        footer.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var text = new StackPanel { Spacing = 4 };
        var heading = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        heading.Children.Add(DfUi.Glyph(kind.Glyph, 14, isOn ? DfUi.Accent : DfUi.Muted));
        heading.Children.Add(DfUi.Text(kind.Title, 15, true, DfUi.PrimaryText, wrap: false));
        if (kind.Beta)
        {
            var badge = DfUi.Badge("Beta");
            badge.Margin = new Thickness(2, 0, 0, 0);
            heading.Children.Add(badge);
        }
        text.Children.Add(heading);
        text.Children.Add(DfUi.Detail(kind.Detail));
        footer.Children.Add(text);
        toggle.VerticalAlignment = VerticalAlignment.Center;
        Grid.SetColumn(toggle, 1);
        footer.Children.Add(toggle);

        var root = new StackPanel();
        root.Children.Add(demoSurface);
        root.Children.Add(footer);
        var card = new Border
        {
            Background = DfUi.Surface,
            BorderBrush = isOn ? DfUi.Brush("DFAccentSubtleBrush") : DfUi.Line,
            BorderThickness = new Thickness(isOn ? 1.5 : 1),
            CornerRadius = new CornerRadius(12),
            Child = root,
        };
        void Apply(bool value)
        {
            enabled[kind.Key] = value;
            demo.Opacity = value ? 1 : 0.6;
            card.BorderBrush = value ? DfUi.Brush("DFAccentSubtleBrush") : DfUi.Line;
            card.BorderThickness = new Thickness(value ? 1.5 : 1);
            FooterHint.Text = EnabledCount == 0 ? "所有功能保持关闭，随时可以再打开" : $"将开启 {EnabledCount} 项功能，随时可以调整";
        }
        toggle.Toggled += (_, _) => Apply(toggle.IsOn);
        return card;
    }

    // MARK: - Demos

    private UIElement BuildDemo(string kind)
    {
        var canvas = new Canvas { Height = 112, IsHitTestVisible = false };
        switch (kind)
        {
            case "clipboard": BuildClipboardDemo(canvas); break;
            case "input": BuildInputDemo(canvas); break;
            case "remote": BuildRemoteDemo(canvas); break;
            case "mouse": BuildMouseDemo(canvas); break;
            default: BuildMicrophoneDemo(canvas); break;
        }
        return canvas;
    }

    private static Border MiniScreen(double width, double height, Brush stroke)
    {
        var inner = new StackPanel { Spacing = 3, VerticalAlignment = VerticalAlignment.Center };
        inner.Children.Add(new Border { Height = 2, Width = width * 0.34, Background = DfUi.Muted, CornerRadius = new CornerRadius(1), HorizontalAlignment = HorizontalAlignment.Center });
        inner.Children.Add(new Border { Height = 2, Width = width * 0.22, Background = DfUi.Muted, CornerRadius = new CornerRadius(1), HorizontalAlignment = HorizontalAlignment.Center });
        return new Border
        {
            Width = width, Height = height, CornerRadius = new CornerRadius(5),
            Background = DfUi.Surface, BorderBrush = stroke, BorderThickness = new Thickness(1.5),
            Child = inner,
        };
    }

    private void BuildClipboardDemo(Canvas canvas)
    {
        var left = MiniScreen(78, 48, DfUi.Muted);
        var right = MiniScreen(78, 48, DfUi.Muted);
        canvas.Children.Add(left);
        canvas.Children.Add(right);
        var dash = new Line
        {
            Stroke = DfUi.Muted, StrokeThickness = 1, StrokeDashArray = [3, 3],
            Y1 = 46, Y2 = 46,
        };
        canvas.Children.Add(dash);
        var pill = new Border
        {
            Background = DfUi.Surface, BorderBrush = DfUi.Brush("DFAccentSubtleBrush"), BorderThickness = new Thickness(1),
            CornerRadius = new CornerRadius(12), Padding = new Thickness(10, 4, 10, 4),
            Child = DfUi.Text("周报草稿", 10, true, DfUi.PrimaryText, wrap: false),
        };
        canvas.Children.Add(pill);
        var check = DfUi.Glyph("\uE73E", 12, DfUi.Success);
        canvas.Children.Add(check);
        demoTicks.Add(time =>
        {
            double width = canvas.ActualWidth > 40 ? canvas.ActualWidth : 300;
            double phase = time % 5.2;
            Canvas.SetLeft(left, width * 0.10); Canvas.SetTop(left, 26);
            Canvas.SetLeft(right, width * 0.72); Canvas.SetTop(right, 26);
            dash.X1 = width * 0.32; dash.X2 = width * 0.68;
            double flight = Math.Clamp((phase - 1.2) / 1.8, 0, 1);
            double eased = 1 - Math.Pow(1 - flight, 3);
            Canvas.SetLeft(pill, width * 0.20 + (width * 0.52) * eased);
            Canvas.SetTop(pill, 16);
            pill.Opacity = phase > 4.8 ? Math.Max(0, (5.2 - phase) / 0.4) : 1;
            Canvas.SetLeft(check, width * 0.72 + 58);
            Canvas.SetTop(check, 18);
            check.Opacity = phase > 3.0 ? 1 : 0;
        });
    }

    private void BuildInputDemo(Canvas canvas)
    {
        var left = MiniScreen(86, 52, DfUi.Muted);
        var right = MiniScreen(86, 52, DfUi.Muted);
        canvas.Children.Add(left);
        canvas.Children.Add(right);
        var cursor = new Ellipse { Width = 9, Height = 9, Fill = DfUi.Accent };
        canvas.Children.Add(cursor);
        var ring = new Ellipse { Width = 18, Height = 18, Stroke = DfUi.Accent, StrokeThickness = 1.5 };
        canvas.Children.Add(ring);
        demoTicks.Add(time =>
        {
            double width = canvas.ActualWidth > 40 ? canvas.ActualWidth : 300;
            Canvas.SetLeft(left, width * 0.10); Canvas.SetTop(left, 28);
            Canvas.SetLeft(right, width * 0.70); Canvas.SetTop(right, 28);
            double phase = time % 6.0;
            double travel = phase < 2 ? 0.25 : phase < 2.8 ? 0.25 + (phase - 2) / 0.8 * 0.5 : phase < 4.4 ? 0.75 : 0.25;
            double x = width * 0.10 + 86 * travel + 4;
            double y = 58;
            Canvas.SetLeft(cursor, x); Canvas.SetTop(cursor, y);
            Canvas.SetLeft(ring, x - 4.5); Canvas.SetTop(ring, y - 4.5);
            ring.Opacity = phase is > 2.6 and < 3.4 ? 0.9 : 0.25;
        });
    }

    private void BuildRemoteDemo(Canvas canvas)
    {
        var local = MiniScreen(90, 54, DfUi.Muted);
        canvas.Children.Add(local);
        var window = new Border
        {
            Width = 76, Height = 50, CornerRadius = new CornerRadius(7),
            Background = DfUi.Surface, BorderBrush = DfUi.Accent, BorderThickness = new Thickness(1.5),
            Child = new StackPanel
            {
                Padding = new Thickness(7),
                Spacing = 3,
                Children =
                {
                    new Border { Height = 2, Width = 28, Background = DfUi.Accent, CornerRadius = new CornerRadius(1), HorizontalAlignment = HorizontalAlignment.Left },
                    new Border { Height = 2, Width = 40, Background = DfUi.Muted, CornerRadius = new CornerRadius(1), HorizontalAlignment = HorizontalAlignment.Left },
                    new Border { Height = 2, Width = 34, Background = DfUi.Muted, CornerRadius = new CornerRadius(1), HorizontalAlignment = HorizontalAlignment.Left },
                },
            },
        };
        canvas.Children.Add(window);
        demoTicks.Add(time =>
        {
            double width = canvas.ActualWidth > 40 ? canvas.ActualWidth : 300;
            Canvas.SetLeft(local, width * 0.14); Canvas.SetTop(local, 28);
            double phase = time % 6.8;
            double flight = Math.Clamp((phase - 1.6) / 2.0, 0, 1);
            double eased = 1 - Math.Pow(1 - flight, 3);
            double start = width * 0.72;
            Canvas.SetLeft(window, start - (start - width * 0.22) * eased);
            Canvas.SetTop(window, 30 - 2 * eased);
            window.Opacity = phase < 1.2 ? 0.4 : 1;
        });
    }

    private void BuildMouseDemo(Canvas canvas)
    {
        var mouse = DfUi.Glyph("\uE962", 30, DfUi.Muted);
        canvas.Children.Add(mouse);
        var ring = new Ellipse { Width = 26, Height = 26, Stroke = DfUi.Accent, StrokeThickness = 2 };
        canvas.Children.Add(ring);
        var pill = new Border
        {
            Background = DfUi.Surface, BorderBrush = DfUi.Line, BorderThickness = new Thickness(1),
            CornerRadius = new CornerRadius(10), Padding = new Thickness(8, 3, 8, 3),
            Child = DfUi.Text("Ctrl C 复制", 10, true, DfUi.PrimaryText, wrap: false),
        };
        canvas.Children.Add(pill);
        demoTicks.Add(time =>
        {
            double width = canvas.ActualWidth > 40 ? canvas.ActualWidth : 300;
            double phase = time % 4.8;
            Canvas.SetLeft(mouse, width * 0.16); Canvas.SetTop(mouse, 30);
            double pulse = (phase % 1.6) / 1.6;
            ring.Width = 20 + pulse * 26; ring.Height = 20 + pulse * 26;
            Canvas.SetLeft(ring, width * 0.16 - (ring.Width - 30) / 2); Canvas.SetTop(ring, 34 - (ring.Height - 30) / 2);
            ring.Opacity = 1 - pulse;
            Canvas.SetLeft(pill, width * 0.50 + 8 * Math.Sin(phase * 2)); Canvas.SetTop(pill, 26);
        });
    }

    private void BuildMicrophoneDemo(Canvas canvas)
    {
        var mic = DfUi.Glyph("\uE720", 30, DfUi.Accent);
        canvas.Children.Add(mic);
        var bars = new List<Border>();
        for (int i = 0; i < 13; i++)
        {
            var bar = new Border { Width = 3, Height = 8, CornerRadius = new CornerRadius(1.5), Background = DfUi.Accent };
            bars.Add(bar);
            canvas.Children.Add(bar);
        }
        var laptop = MiniScreen(64, 40, DfUi.Muted);
        canvas.Children.Add(laptop);
        demoTicks.Add(time =>
        {
            double width = canvas.ActualWidth > 40 ? canvas.ActualWidth : 300;
            Canvas.SetLeft(mic, width * 0.14); Canvas.SetTop(mic, 34);
            Canvas.SetLeft(laptop, width * 0.76); Canvas.SetTop(laptop, 30);
            for (int i = 0; i < bars.Count; i++)
            {
                double height = 8 + Math.Abs(Math.Sin(time * 3.4 + i * 0.55)) * 22;
                bars[i].Height = height;
                Canvas.SetLeft(bars[i], width * 0.30 + i * 7);
                Canvas.SetTop(bars[i], 52 - height / 2);
            }
        });
    }

    private void AnimateDemos(object? sender, object args)
    {
        double time = demoClock.Elapsed.TotalSeconds;
        foreach (var tick in demoTicks) tick(time);
    }

    private void StopDemos()
    {
        if (!demoRunning) return;
        demoRunning = false;
        CompositionTarget.Rendering -= AnimateDemos;
        demoTicks.Clear();
    }

    // MARK: - Navigation

    private static MainWindow? Host => (Application.Current as App)?.ClientWindow;

    private void CloseLightClick(object sender, RoutedEventArgs args) => Host?.HideToTray();

    private void MinimizeLightClick(object sender, RoutedEventArgs args) => Host?.MinimizeFromTitleBar();

    private void MaximizeLightClick(object sender, RoutedEventArgs args) => Host?.ToggleMaximizeFromTitleBar();

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

    private void BackClick(object sender, RoutedEventArgs args)
    {
        if (step <= 1) return;
        step = 1;
        UpdateChrome();
    }

    private void NextClick(object sender, RoutedEventArgs args)
    {
        if (step == 1)
        {
            step = 2;
            UpdateChrome();
            return;
        }
        StopDemos();
        clipboard.SyncEnabled = enabled["clipboard"];
        clipboard.CaptureEnabled = enabled["clipboard"];
        onCompleted(new OnboardingResult(enabled["clipboard"], enabled["input"], enabled["remote"]));
    }
}
