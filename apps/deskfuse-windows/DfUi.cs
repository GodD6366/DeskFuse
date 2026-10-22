using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;

namespace DeskFuse.Windows;

/// <summary>
/// Builders for the macOS DFComponents vocabulary. Every value is assigned as a
/// direct property: styles from Application.Resources carry ThemeResource
/// setters and render blank on code-created elements in this unpackaged app.
/// </summary>
internal static class DfUi
{
    internal static Brush Brush(string key) => (Brush)Application.Current.Resources[key];
    internal static Brush Accent => Brush("DFAccentBrush");
    internal static Brush Muted => Brush("DFMutedBrush");
    internal static Brush Surface => Brush("DFSurfaceBrush");
    internal static Brush Canvas => Brush("DFCanvasBrush");
    internal static Brush Control => Brush("DFControlBrush");
    internal static Brush Line => Brush("DFLineBrush");
    internal static Brush PrimaryText => Brush("TextFillColorPrimaryBrush");
    internal static Brush Success => Brush("DFSuccessBrush");
    internal static Brush Warning => Brush("DFWarningBrush");
    internal static Brush Danger => Brush("DFDangerBrush");

    internal static TextBlock Text(string text, double size = 13, bool semibold = false,
        Brush? foreground = null, bool wrap = true, double lineHeight = 0)
    {
        var block = new TextBlock
        {
            Text = text,
            FontSize = size,
            FontWeight = semibold ? FontWeights.SemiBold : FontWeights.Normal,
            Foreground = foreground ?? PrimaryText,
            TextWrapping = wrap ? TextWrapping.Wrap : TextWrapping.NoWrap,
        };
        if (lineHeight > 0) block.LineHeight = lineHeight;
        return block;
    }

    internal static TextBlock Detail(string text, double size = 12) =>
        Text(text, size, foreground: Muted);

    internal static TextBlock SectionLabel(string text)
    {
        var block = Text(text, 13, true, Muted);
        block.Margin = new Thickness(4, 0, 0, 0);
        return block;
    }

    internal static Border Divider(double top = 12, double bottom = 12) => new()
    {
        Height = 1,
        Background = Line,
        Margin = new Thickness(0, top, 0, bottom),
        HorizontalAlignment = HorizontalAlignment.Stretch,
    };

    internal static Border Card(UIElement child, double padding = 16, double radius = 12,
        Brush? background = null, Brush? border = null)
    {
        return new Border
        {
            Background = background ?? Surface,
            BorderBrush = border ?? Line,
            BorderThickness = new Thickness(1),
            CornerRadius = new CornerRadius(radius),
            Padding = new Thickness(padding),
            Child = child,
        };
    }

    internal static StackPanel Stack(double spacing = 8, params UIElement[] children)
    {
        var panel = new StackPanel { Spacing = spacing };
        foreach (var child in children) panel.Children.Add(child);
        return panel;
    }

    internal static StackPanel Section(string? title, UIElement body, string? footer = null)
    {
        var panel = new StackPanel { Spacing = 8 };
        if (!string.IsNullOrEmpty(title)) panel.Children.Add(SectionLabel(title));
        panel.Children.Add(Card(body));
        if (!string.IsNullOrEmpty(footer)) panel.Children.Add(Detail(footer));
        return panel;
    }

    internal static Grid SettingRow(string title, string? detail, FrameworkElement control)
    {
        var grid = new Grid { MinHeight = 32, ColumnSpacing = 16, Padding = new Thickness(0, 2, 0, 2) };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var left = new StackPanel { Spacing = 2, VerticalAlignment = VerticalAlignment.Center };
        left.Children.Add(Text(title, 13));
        if (!string.IsNullOrEmpty(detail)) left.Children.Add(Detail(detail));
        grid.Children.Add(left);
        control.VerticalAlignment = VerticalAlignment.Center;
        control.HorizontalAlignment = HorizontalAlignment.Right;
        Grid.SetColumn(control, 1);
        grid.Children.Add(control);
        return grid;
    }

    internal static Border Badge(string text, string tone = "DFWarningBrush")
    {
        var brush = Brush(tone);
        var label = Text(text, 11, true, brush);
        return new Border
        {
            Background = tone == "DFWarningBrush" ? Brush("DFWarningSubtleBrush") : Brush("DFControlBrush"),
            CornerRadius = new CornerRadius(4),
            Padding = new Thickness(6, 1, 6, 1),
            Child = label,
            VerticalAlignment = VerticalAlignment.Center,
        };
    }

    internal static Border StatusPill(string text, string tone = "DFMutedBrush", bool subtle = true)
    {
        var brush = Brush(tone);
        var panel = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6, VerticalAlignment = VerticalAlignment.Center };
        panel.Children.Add(new Microsoft.UI.Xaml.Shapes.Ellipse { Width = 7, Height = 7, Fill = brush, VerticalAlignment = VerticalAlignment.Center });
        panel.Children.Add(Text(text, 12, true, brush));
        return new Border
        {
            Background = subtle ? Brush("DFControlBrush") : brush,
            CornerRadius = new CornerRadius(12),
            Padding = new Thickness(10, 0, 10, 0),
            Height = 24,
            Child = panel,
            VerticalAlignment = VerticalAlignment.Center,
        };
    }

    internal static FontIcon Glyph(string glyph, double size = 14, Brush? foreground = null) => new()
    {
        Glyph = glyph,
        FontSize = size,
        Foreground = foreground ?? PrimaryText,
        HorizontalAlignment = HorizontalAlignment.Center,
        VerticalAlignment = VerticalAlignment.Center,
    };

    private static ControlTemplate FlatTemplate => (ControlTemplate)Application.Current.Resources["DFFlatButtonTemplate"];

    internal static Button ButtonBase(string? text, FontIcon? icon, Brush background, Brush foreground,
        Brush border, double height, double padding, RoutedEventHandler? click)
    {
        var button = new Button
        {
            Background = background,
            Foreground = foreground,
            BorderBrush = border,
            BorderThickness = new Thickness(1),
            CornerRadius = new CornerRadius(8),
            Padding = new Thickness(padding, 0, padding, 0),
            MinHeight = height,
            MinWidth = 0,
            FontSize = 13,
            FontWeight = FontWeights.SemiBold,
            HorizontalContentAlignment = HorizontalAlignment.Center,
            VerticalContentAlignment = VerticalAlignment.Center,
            Template = FlatTemplate,
        };
        if (icon is not null)
        {
            icon.Foreground = foreground;
            if (text is null) button.Content = icon;
            else
            {
                var content = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
                content.Children.Add(icon);
                content.Children.Add(new TextBlock { Text = text, FontSize = 13, FontWeight = FontWeights.SemiBold, Foreground = foreground });
                button.Content = content;
            }
        }
        else button.Content = text;
        if (text is not null) AutomationProperties.SetName(button, text);
        if (click is not null) button.Click += click;
        return button;
    }

    internal static Button Primary(string text, RoutedEventHandler? click = null) =>
        ButtonBase(text, null, Accent, Brush("DFAccentForegroundBrush"), Brush("DFAccentForegroundBrush"), 32, 12, click);

    internal static Button Secondary(string text, RoutedEventHandler? click = null) =>
        ButtonBase(text, null, Surface, PrimaryText, Line, 32, 12, click);

    internal static Button Plain(string text, RoutedEventHandler? click = null)
    {
        var button = ButtonBase(text, null, new SolidColorBrush(Microsoft.UI.Colors.Transparent), Accent,
            new SolidColorBrush(Microsoft.UI.Colors.Transparent), 28, 4, click);
        button.BorderThickness = new Thickness(0);
        return button;
    }

    internal static Button IconButton(string glyph, RoutedEventHandler? click = null, string? tooltip = null,
        double size = 28, double glyphSize = 14)
    {
        var button = new Button
        {
            Content = Glyph(glyph, glyphSize),
            Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent),
            BorderThickness = new Thickness(0),
            Padding = new Thickness(0),
            Width = size,
            Height = size,
            MinWidth = 0,
            MinHeight = 0,
            CornerRadius = new CornerRadius(8),
            HorizontalContentAlignment = HorizontalAlignment.Center,
            VerticalContentAlignment = VerticalAlignment.Center,
            Template = FlatTemplate,
        };
        if (click is not null) button.Click += click;
        if (tooltip is not null)
        {
            ToolTipService.SetToolTip(button, tooltip);
            AutomationProperties.SetName(button, tooltip);
        }
        button.PointerEntered += (_, _) => button.Background = Control;
        button.PointerExited += (_, _) => button.Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        return button;
    }

    internal static DfToggle Switch(bool isOn, RoutedEventHandler? toggled = null, bool enabled = true)
    {
        var toggle = new DfToggle(isOn, enabled);
        if (toggled is not null) toggle.Toggled += toggled;
        return toggle;
    }

    internal static ScrollViewer Page(UIElement content, double maxWidth = 720,
        double horizontal = 24, double top = 4, double bottom = 24)
    {
        var panel = (FrameworkElement)content;
        panel.MaxWidth = maxWidth;
        panel.Width = maxWidth;
        panel.HorizontalAlignment = HorizontalAlignment.Left;
        panel.Margin = new Thickness(horizontal, top, horizontal, bottom);
        return new ScrollViewer
        {
            Content = panel,
            VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
            HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
        };
    }

    internal static StackPanel EmptyState(string glyph, string title, string message,
        string? actionTitle = null, RoutedEventHandler? action = null)
    {
        var panel = new StackPanel
        {
            Spacing = 12,
            MaxWidth = 360,
            HorizontalAlignment = HorizontalAlignment.Center,
            VerticalAlignment = VerticalAlignment.Center,
        };
        panel.Children.Add(new FontIcon
        {
            Glyph = glyph, FontSize = 40, Foreground = Muted,
            HorizontalAlignment = HorizontalAlignment.Center,
        });
        var heading = Text(title, 15, true, wrap: false);
        heading.HorizontalAlignment = HorizontalAlignment.Center;
        panel.Children.Add(heading);
        var detail = Detail(message);
        detail.TextAlignment = TextAlignment.Center;
        panel.Children.Add(detail);
        if (actionTitle is not null && action is not null)
        {
            var button = Primary(actionTitle, action);
            button.HorizontalAlignment = HorizontalAlignment.Center;
            panel.Children.Add(button);
        }
        return panel;
    }

    internal static TextBox SearchField(string placeholder, TextChangedEventHandler? changed = null)
    {
        var box = new TextBox
        {
            PlaceholderText = placeholder,
            Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent),
            BorderThickness = new Thickness(0),
            Padding = new Thickness(0, 0, 8, 0),
            VerticalAlignment = VerticalAlignment.Center,
        };
        if (changed is not null) box.TextChanged += changed;
        return box;
    }

    internal static Border SearchSurface(FrameworkElement field, double height = 32, string glyph = "\uE721")
    {
        var grid = new Grid();
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        var icon = Glyph(glyph, 13);
        icon.Foreground = Muted;
        icon.Margin = new Thickness(10, 0, 8, 0);
        grid.Children.Add(icon);
        Grid.SetColumn(field, 1);
        grid.Children.Add(field);
        return new Border
        {
            Height = height,
            CornerRadius = new CornerRadius(8),
            Background = Control,
            Child = grid,
        };
    }

    internal static StackPanel ValueRow(string title, string value)
    {
        var grid = new Grid { MinHeight = 24, ColumnSpacing = 12 };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.Children.Add(Detail(title));
        var text = Text(value, 13);
        text.TextAlignment = TextAlignment.Right;
        text.HorizontalAlignment = HorizontalAlignment.Stretch;
        text.IsTextSelectionEnabled = true;
        Grid.SetColumn(text, 1);
        grid.Children.Add(text);
        return new StackPanel { Children = { grid } };
    }
}

/// <summary>38 x 22 mac-style switch. Built from direct properties so it never
/// depends on a ThemeResource style resolved on a code-created element.</summary>
internal sealed class DfToggle : ToggleButton
{
    private readonly Border track;
    private readonly Ellipse thumb;
    public event RoutedEventHandler? Toggled;

    public bool IsOn
    {
        get => IsChecked == true;
        set { if (IsChecked != value) IsChecked = value; }
    }

    public DfToggle(bool isOn = false, bool enabled = true)
    {
        Width = 38;
        Height = 22;
        MinWidth = 0;
        MinHeight = 0;
        Padding = new Thickness(0);
        BorderThickness = new Thickness(0);
        Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        HorizontalContentAlignment = HorizontalAlignment.Left;
        VerticalContentAlignment = VerticalAlignment.Center;
        VerticalAlignment = VerticalAlignment.Center;
        IsEnabled = enabled;
        Template = (ControlTemplate)Application.Current.Resources["DFContentOnlyTemplate"];
        track = new Border
        {
            Width = 38,
            Height = 22,
            CornerRadius = new CornerRadius(11),
            HorizontalAlignment = HorizontalAlignment.Center,
            VerticalAlignment = VerticalAlignment.Center,
        };
        thumb = new Ellipse
        {
            Width = 18,
            Height = 18,
            Fill = new SolidColorBrush(Microsoft.UI.Colors.White),
            VerticalAlignment = VerticalAlignment.Center,
        };
        var grid = new Grid { Width = 38, Height = 22 };
        grid.Children.Add(track);
        grid.Children.Add(thumb);
        Content = grid;
        IsChecked = isOn;
        UpdateVisual();
        Checked += (_, e) => { UpdateVisual(); Toggled?.Invoke(this, e); };
        Unchecked += (_, e) => { UpdateVisual(); Toggled?.Invoke(this, e); };
    }

    private void UpdateVisual()
    {
        bool on = IsOn;
        track.Background = on ? DfUi.Accent : DfUi.Brush("DFControlStrongBrush");
        track.BorderBrush = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        track.BorderThickness = new Thickness(0);
        thumb.HorizontalAlignment = on ? HorizontalAlignment.Right : HorizontalAlignment.Left;
        thumb.Margin = on ? new Thickness(0, 0, 2, 0) : new Thickness(2, 0, 0, 0);
        Opacity = IsEnabled ? 1 : 0.45;
    }
}
