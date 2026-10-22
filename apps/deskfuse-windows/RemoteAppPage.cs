using System.Text.Json;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;

namespace DeskFuse.Windows;

/// Local-only RemoteApp registry editor. The LocalSystem service performs all
/// HKLM/TSAppAllowList writes; this page never elevates or edits the registry.
public sealed class RemoteAppPage : UserControl
{
    private readonly ServiceClient service;
    private readonly ListBox registered = new()
    {
        Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent),
        BorderThickness = new Thickness(0),
        SelectionMode = SelectionMode.Single,
    };
    private readonly ListBox installed = new()
    {
        Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent),
        BorderThickness = new Thickness(0),
        SelectionMode = SelectionMode.Single,
    };
    private readonly TextBlock status = DfUi.Detail("正在读取远程应用…");
    private readonly TextBlock registeredEmpty = DfUi.Detail("还没有注册应用。\n注册后，Mac 端会在远程应用列表中看到它。");
    private readonly TextBlock installedEmpty = DfUi.Detail("没有可选的已安装程序。");
    private readonly TextBox name = new();
    private readonly TextBox displayName = new();
    private readonly TextBox path = new();
    private readonly StackPanel preflightBody = new() { Spacing = 8 };
    private bool preflightRetryPending;

    public RemoteAppPage(ServiceClient service)
    {
        this.service = service;
        var root = new StackPanel { Spacing = 24, MaxWidth = 720, Margin = new Thickness(24, 4, 24, 24) };

        var header = new Grid { ColumnSpacing = 14 };
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        header.Children.Add(new Border
        {
            Width = 42, Height = 42, CornerRadius = new CornerRadius(10),
            Background = DfUi.Brush("DFAccentSubtleBrush"),
            Child = DfUi.Glyph("\uE8A7", 20, DfUi.Accent),
        });
        var title = new StackPanel { Spacing = 4, VerticalAlignment = VerticalAlignment.Center };
        title.Children.Add(DfUi.Text("共享的远程应用", 15, true));
        title.Children.Add(DfUi.Detail("管理这台 Windows 共享的应用，其他已配对设备可以连接使用。连接时需要 Windows 登录凭据，DeskFuse 不保存密码。"));
        Grid.SetColumn(title, 1);
        header.Children.Add(title);
        var refresh = DfUi.IconButton("\uE72C", async (_, _) => await Refresh(), "刷新应用列表");
        Grid.SetColumn(refresh, 2);
        header.Children.Add(refresh);
        root.Children.Add(header);
        root.Children.Add(DfUi.Section("服务端预检", preflightBody, "Mac 连接前需通过全部检查；端口与证书指纹来自本机 RDP 监听，Mac 自动固定证书。"));

        var listBody = new StackPanel { Spacing = 8 };
        listBody.Children.Add(StyledList(registered, 180, registeredEmpty));
        root.Children.Add(DfUi.Section("已注册 RemoteApp", listBody, "选择应用后注册；Mac 端将在列表中看到它。"));

        var form = new StackPanel { Spacing = 10 };
        form.Children.Add(Field("注册名（英文）", name));
        form.Children.Add(Field("显示名称", displayName));
        form.Children.Add(Field("可执行文件完整路径", path));
        var choose = DfUi.Secondary("从已安装程序选择", (_, _) =>
        {
            if (installed.SelectedItem is InstalledEntry app)
            {
                name.Text = app.Name;
                displayName.Text = app.Name;
                path.Text = app.Path;
            }
        });
        choose.HorizontalAlignment = HorizontalAlignment.Left;
        form.Children.Add(choose);
        form.Children.Add(StyledList(installed, 160, installedEmpty));
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        actions.Children.Add(DfUi.Primary("注册 / 更新", async (_, _) => await Register()));
        var remove = DfUi.Secondary("移除选中", async (_, _) => await Remove());
        remove.Foreground = DfUi.Danger;
        actions.Children.Add(remove);
        form.Children.Add(actions);
        form.Children.Add(status);
        root.Children.Add(DfUi.Section("注册应用", form));

        Content = new ScrollViewer
        {
            Content = root,
            VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
            HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
        };
        Loaded += async (_, _) => await Refresh();
    }

    private static UIElement StyledList(ListBox list, double height, TextBlock placeholder)
    {
        list.ItemContainerStyle = new Style(typeof(ListBoxItem))
        {
            Setters =
            {
                new Setter(ListBoxItem.PaddingProperty, new Thickness(10, 7, 10, 7)),
                new Setter(ListBoxItem.MarginProperty, new Thickness(0, 1, 0, 1)),
                new Setter(ListBoxItem.CornerRadiusProperty, new CornerRadius(8)),
                new Setter(ListBoxItem.HorizontalContentAlignmentProperty, HorizontalAlignment.Stretch),
            },
        };
        var grid = new Grid();
        grid.Children.Add(list);
        placeholder.HorizontalAlignment = HorizontalAlignment.Center;
        placeholder.VerticalAlignment = VerticalAlignment.Center;
        placeholder.TextAlignment = TextAlignment.Center;
        placeholder.IsHitTestVisible = false;
        grid.Children.Add(placeholder);
        return new Border
        {
            Background = DfUi.Control,
            CornerRadius = new CornerRadius(8),
            Padding = new Thickness(4),
            Height = height,
            Child = new ScrollViewer
            {
                Content = grid,
                VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
                HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
            },
        };
    }

    private static UIElement Field(string label, TextBox box)
    {
        var panel = new StackPanel { Spacing = 4 };
        panel.Children.Add(DfUi.Detail(label));
        box.Background = DfUi.Surface;
        box.BorderBrush = DfUi.Line;
        box.BorderThickness = new Thickness(1);
        box.CornerRadius = new CornerRadius(8);
        box.Height = 32;
        box.Padding = new Thickness(10, 0, 10, 0);
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
            registeredEmpty.Visibility = registered.Items.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
            installedEmpty.Visibility = installed.Items.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
            if (remote.ValueKind == JsonValueKind.Object) RenderPreflight(remote);
            status.Text = remote.ValueKind == JsonValueKind.Object && !remote.Bool("available") ? remote.String("reason") : "选择应用后注册；Mac 端将在列表中看到它。";
        }
        catch (Exception ex) { status.Text = ex.Message; }
    }

    private void RenderPreflight(JsonElement remote)
    {
        preflightBody.Children.Clear();
        if (!remote.TryGetProperty("host", out var host) || host.ValueKind != JsonValueKind.Object)
        {
            preflightBody.Children.Add(DfUi.Detail("服务正在完成首次预检…"));
            SchedulePreflightRetry();
            return;
        }
        var allPassed = true;
        if (host.TryGetProperty("checks", out var checks) && checks.ValueKind == JsonValueKind.Array)
        {
            foreach (var check in checks.EnumerateArray())
            {
                var passed = check.Bool("passed");
                allPassed &= passed;
                preflightBody.Children.Add(CheckRow(passed, check.String("message")));
            }
        }
        var port = host.TryGetProperty("port", out var portValue) && portValue.ValueKind == JsonValueKind.Number
            ? portValue.GetUInt16()
            : (ushort)0;
        var fingerprint = host.String("certificate_sha256");
        if (port > 0)
        {
            preflightBody.Children.Add(DfUi.Detail(fingerprint.Length > 0
                ? $"RDP 端口 {port} · 监听证书 {Shorten(fingerprint)}"
                : $"RDP 端口 {port}"));
        }
        if (allPassed && port > 0 && fingerprint.Length > 0)
        {
            preflightBody.Children.Add(DfUi.Detail("预检通过，已配对的 Mac 可以直接连接。"));
        }
    }

    private void SchedulePreflightRetry()
    {
        if (preflightRetryPending) return;
        preflightRetryPending = true;
        DispatcherQueue.TryEnqueue(async () =>
        {
            await Task.Delay(1500);
            preflightRetryPending = false;
            await Refresh();
        });
    }

    private static string Shorten(string fingerprint) =>
        fingerprint.Length <= 23 ? fingerprint : fingerprint[..23] + "…";

    private static UIElement CheckRow(bool passed, string message)
    {
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        row.Children.Add(DfUi.Glyph(passed ? "\uE73E" : "\uE7BA", 12, passed ? DfUi.Success : DfUi.Warning));
        row.Children.Add(DfUi.Text(message, 12, foreground: passed ? DfUi.Muted : DfUi.PrimaryText));
        return row;
    }

    private async Task Register()
    {
        if (name.Text.Trim() is not { Length: > 0 } app || path.Text.Trim() is not { Length: > 0 } executable)
        {
            status.Text = "注册名和可执行文件路径不能为空。";
            return;
        }
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

    private sealed record RegisteredEntry(string Name, string DisplayName, string Path)
    {
        public string Label => $"{DisplayName}  ·  {Name}\n{Path}";
    }

    private sealed record InstalledEntry(string Name, string Path)
    {
        public string Label => $"{Name}\n{Path}";
    }
}
