using System.Text.Json;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;

namespace DeskFuse.Windows;

/// Local-only RemoteApp registry editor. The LocalSystem service performs all
/// HKLM/TSAppAllowList writes; this window never elevates or edits the registry.
public sealed class RemoteAppWindow
{
    private readonly ServiceClient service;
    private readonly Window window = new();
    private readonly ListBox registered = new();
    private readonly ListBox installed = new();
    private readonly TextBlock status = new() { Foreground = new SolidColorBrush(Microsoft.UI.ColorHelper.FromArgb(255, 112, 128, 144)), TextWrapping = TextWrapping.Wrap };
    private readonly TextBox name = new();
    private readonly TextBox displayName = new();
    private readonly TextBox path = new();

    public RemoteAppWindow(ServiceClient service)
    {
        this.service = service;
        AppIcon.Apply(window);
        window.AppWindow.Title = "Windows RemoteApp";
        window.AppWindow.Resize(new global::Windows.Graphics.SizeInt32(760, 560));

        var root = new Grid { Margin = new Thickness(22) };
        root.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        root.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        root.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });

        var header = new StackPanel { Orientation = Orientation.Horizontal };
        header.Children.Add(new TextBlock { Text = "Windows RemoteApp", FontSize = 25, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold, VerticalAlignment = VerticalAlignment.Center });
        var refresh = new Button { Content = "刷新", Margin = new Thickness(22, 0, 0, 0), Padding = new Thickness(14, 5, 14, 5) };
        refresh.Click += async (_, _) => await Refresh();
        header.Children.Add(refresh);
        Grid.SetRow(header, 0); root.Children.Add(header);

        var notice = new TextBlock { Text = "RemoteApp 使用 Windows 原生 RDP/NLA；macOS 端会单独建立 RDP 连接。这里仅管理服务端 TSAppAllowList，不读取或保存 Windows 密码。", Margin = new Thickness(0, 10, 0, 14), Foreground = new SolidColorBrush(Microsoft.UI.ColorHelper.FromArgb(255, 112, 128, 144)), TextWrapping = TextWrapping.Wrap };
        Grid.SetRow(notice, 1); root.Children.Add(notice);

        var columns = new Grid();
        columns.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        columns.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1.25, GridUnitType.Star) });
        Grid.SetRow(columns, 2); root.Children.Add(columns);

        var left = new StackPanel { Margin = new Thickness(0, 0, 12, 0) };
        left.Children.Add(new TextBlock { Text = "已注册 RemoteApp", FontWeight = Microsoft.UI.Text.FontWeights.SemiBold, Margin = new Thickness(0, 0, 0, 8) });
        registered.SelectionMode = SelectionMode.Single;
        left.Children.Add(registered);
        Grid.SetColumn(left, 0); columns.Children.Add(left);

        var right = new StackPanel { Margin = new Thickness(12, 0, 0, 0) };
        right.Children.Add(new TextBlock { Text = "注册应用", FontWeight = Microsoft.UI.Text.FontWeights.SemiBold });
        right.Children.Add(Field("注册名（英文）", name));
        right.Children.Add(Field("显示名称", displayName));
        right.Children.Add(Field("可执行文件完整路径", path));
        var choose = new Button { Content = "从已安装程序选择", Margin = new Thickness(0, 6, 0, 8) };
        choose.Click += (_, _) => { if (installed.SelectedItem is InstalledEntry app) { name.Text = app.Name; displayName.Text = app.Name; path.Text = app.Path; } };
        right.Children.Add(choose);
        installed.SelectionMode = SelectionMode.Single;
        right.Children.Add(installed);
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Margin = new Thickness(0, 12, 0, 0) };
        var add = new Button { Content = "注册 / 更新", Padding = new Thickness(14, 5, 14, 5) };
        add.Click += async (_, _) => await Register();
        var remove = new Button { Content = "移除选中", Margin = new Thickness(8, 0, 0, 0), Padding = new Thickness(14, 5, 14, 5) };
        remove.Click += async (_, _) => await Remove();
        actions.Children.Add(add); actions.Children.Add(remove);
        right.Children.Add(actions);
        right.Children.Add(status);
        Grid.SetColumn(right, 1); columns.Children.Add(right);

        _ = Refresh();
    }

    private static UIElement Field(string label, TextBox box)
    {
        var panel = new StackPanel { Margin = new Thickness(0, 10, 0, 0) };
        panel.Children.Add(new TextBlock { Text = label, FontSize = 12, Foreground = new SolidColorBrush(Microsoft.UI.ColorHelper.FromArgb(255, 112, 128, 144)) });
        panel.Children.Add(box);
        return panel;
    }

    private async Task Refresh()
    {
        try
        {
            var apps = await service.SendAsync("ListRemoteApps");
            registered.ItemsSource = apps.TryGetProperty("RemoteApps", out var remote)
                ? remote.GetProperty("apps").EnumerateArray().Select(x => new RegisteredEntry(x.String("name"), x.String("display_name"), x.String("path"))).ToArray()
                : Array.Empty<RegisteredEntry>();
            registered.DisplayMemberPath = nameof(RegisteredEntry.Label);
            var installedReply = await service.SendAsync("ListInstalledApps");
            installed.ItemsSource = installedReply.TryGetProperty("InstalledApps", out var available)
                ? available.GetProperty("apps").EnumerateArray().Select(x => new InstalledEntry(x.String("name"), x.String("path"))).ToArray()
                : Array.Empty<InstalledEntry>();
            installed.DisplayMemberPath = nameof(InstalledEntry.Label);
            status.Text = remote.ValueKind == JsonValueKind.Object && !remote.Bool("available") ? remote.String("reason") : "选择应用后注册；Mac 端将在列表中看到它。";
        }
        catch (Exception ex) { status.Text = ex.Message; }
    }

    private async Task Register()
    {
        if (name.Text.Trim() is not { Length: > 0 } app || path.Text.Trim() is not { Length: > 0 } executable) { status.Text = "注册名和可执行文件路径不能为空。"; return; }
        try
        {
            await service.SendAsync(ServiceClient.Command("RegisterRemoteApp", new { name = app, path = executable, display_name = string.IsNullOrWhiteSpace(displayName.Text) ? app : displayName.Text.Trim() }));
            status.Text = "已提交注册；以列表刷新后的实际状态为准。";
            await Refresh();
        }
        catch (Exception ex) { status.Text = ex.Message; }
    }

    private async Task Remove()
    {
        if (registered.SelectedItem is not RegisteredEntry app) return;
        try
        {
            await service.SendAsync(ServiceClient.Command("UnregisterRemoteApp", new { name = app.Name }));
            status.Text = "已提交移除。";
            await Refresh();
        }
        catch (Exception ex) { status.Text = ex.Message; }
    }

    public void Show() { window.AppWindow.Show(); window.Activate(); }

    private sealed record RegisteredEntry(string Name, string DisplayName, string Path)
    {
        public string Label => $"{DisplayName}  ·  {Name}\n{Path}";
    }

    private sealed record InstalledEntry(string Name, string Path)
    {
        public string Label => $"{Name}\n{Path}";
    }
}
