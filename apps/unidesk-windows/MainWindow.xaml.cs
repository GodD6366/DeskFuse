using System.ComponentModel;
using System.Collections.Specialized;
using System.Diagnostics;
using System.IO;
using System.Net;
using System.Net.Sockets;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using Button = System.Windows.Controls.Button;
using CheckBox = System.Windows.Controls.CheckBox;
using ComboBox = System.Windows.Controls.ComboBox;
using Orientation = System.Windows.Controls.Orientation;
using TextBox = System.Windows.Controls.TextBox;
using Application = System.Windows.Application;

namespace UniDesk.Windows;

public partial class MainWindow : Window
{
    private readonly ServiceClient service;
    private readonly DesktopClipboardController clipboard;
    private SettingsWindow? settings;
    private List<DeviceRow> devices = [];
    private bool updatingHistory;
    private bool updatingDevices;
    private bool updatingSync;
    private bool wasConnected;
    private string lastDeviceSnapshot = "";
    private JsonElement? latest;
    private bool busy;
    public MainWindow(ServiceClient service, DesktopClipboardController clipboard)
    {
        this.service = service;
        this.clipboard = clipboard;
        InitializeComponent();
        clipboard.Entries.CollectionChanged += HistoryChanged;
        clipboard.Error += ShowNotice;
        service.SnapshotChanged += SnapshotChanged;
        service.ConnectionChanged += ConnectionChanged;
        Closing += OnClosing;
        PreviewKeyDown += KeyboardShortcut;
        DeviceDetails.LostKeyboardFocus += (_, _) => Dispatcher.BeginInvoke(() => { if (!DeviceDetails.IsKeyboardFocusWithin) RenderDeviceDetails(); });
        RefreshHistory();
        ConnectionChanged(false, "正在连接 UniDeskService…");
        DrawDisplays();
    }
    private void NavigationChanged(object sender, SelectionChangedEventArgs args)
    {
        if (ClipboardPage is null) return;
        ClipboardPage.Visibility = Navigation.SelectedIndex == 0 ? Visibility.Visible : Visibility.Collapsed;
        InputPage.Visibility = Navigation.SelectedIndex == 1 ? Visibility.Visible : Visibility.Collapsed;
        DevicesPage.Visibility = Navigation.SelectedIndex == 2 ? Visibility.Visible : Visibility.Collapsed;
        TransfersPage.Visibility = Navigation.SelectedIndex == 3 ? Visibility.Visible : Visibility.Collapsed;
        if (Navigation.SelectedIndex == 1) DrawDisplays();
    }
    private void HistoryChanged(object? sender, NotifyCollectionChangedEventArgs args) => RefreshHistory();
    private void HistoryFilterChanged(object sender, RoutedEventArgs args)
    {
        if (!updatingHistory && HistoryList is not null && SourceFilter is not null) RefreshHistory();
    }
    private void RefreshHistory()
    {
        if (HistoryList is null) return;
        updatingHistory = true;
        try
        {
            string? selected = (HistoryList.SelectedItem as ClipboardEntry)?.Id;
            string source = SourceFilter.SelectedItem as string ?? "全部来源";
            var sources = new[] { "全部来源" }.Concat(clipboard.Entries.Select(e => e.SourceName).Distinct()).ToArray();
            SourceFilter.ItemsSource = sources;
            SourceFilter.SelectedItem = sources.Contains(source) ? source : "全部来源";
            string search = HistorySearch.Text.Trim();
            string kind = KindFilter.SelectedIndex switch { 1 => "text", 2 => "image", 3 => "files", _ => "" };
            source = SourceFilter.SelectedItem as string ?? "全部来源";
            var visible = clipboard.Entries.Where(e => (kind.Length == 0 || e.Kind == kind) && (source == "全部来源" || e.SourceName == source) && (search.Length == 0 || $"{e.Title} {e.Preview} {e.SourceName}".Contains(search, StringComparison.CurrentCultureIgnoreCase))).ToArray();
            HistoryList.ItemsSource = visible;
            HistoryList.SelectedItem = visible.FirstOrDefault(e => e.Id == selected) ?? visible.FirstOrDefault();
            HistoryEmpty.Visibility = visible.Length == 0 ? Visibility.Visible : Visibility.Collapsed;
            HistoryEmpty.Text = clipboard.Entries.Count == 0 ? "还没有剪贴板记录。\n复制文字、图片或本机文件开始使用。" : "没有匹配记录。\n修改搜索文字或清除筛选。";
        }
        finally { updatingHistory = false; }
        RefreshPreview();
    }
    private void HistorySelectionChanged(object sender, SelectionChangedEventArgs args) { if (!updatingHistory) RefreshPreview(); }
    private void RefreshPreview()
    {
        var entry = HistoryList.SelectedItem as ClipboardEntry;
        CopyButton.IsEnabled = entry is not null;
        AddShelfButton.IsEnabled = entry is not null;
        ImagePreview.Source = null;
        ImagePreview.Visibility = Visibility.Collapsed;
        TextPreview.Visibility = Visibility.Visible;
        RevealButton.Visibility = Visibility.Collapsed;
        FitButton.Visibility = Visibility.Collapsed;
        if (entry is null)
        {
            PreviewTitle.Text = "选择一条剪贴板记录";
            PreviewMetadata.Text = "选择仅预览；点击复制才恢复系统剪贴板。";
            TextPreview.Text = "内容将在这里显示";
            FormatMetadata.Text = "";
            return;
        }
        PreviewTitle.Text = entry.Title.Replace('\r', ' ').Replace('\n', ' ');
        PreviewMetadata.Text = $"来自 {entry.SourceName} · {entry.CreatedAt.LocalDateTime:g}";
        FormatMetadata.Text = entry.Kind == "files" ? $"{entry.Content.Files?.Count ?? 0} 个本地文件引用 · 不读取文件正文" : $"{entry.Content.Mime} · {FormatBytes(entry.SizeBytes)} · 保留原始格式";
        TextPreview.Text = entry.Preview;
        if (entry.Kind == "image")
        {
            try
            {
                ImagePreview.Source = NativeClipboard.DecodeImage(entry.Content.Data);
                ImagePreview.Visibility = Visibility.Visible;
                TextPreview.Visibility = Visibility.Collapsed;
                FitButton.Visibility = Visibility.Visible;
            }
            catch (Exception) { TextPreview.Text = "图片无法预览，可能已损坏或超过解码限制。"; CopyButton.IsEnabled = false; }
        }
        if (entry.Kind == "files")
        {
            bool valid = entry.Content.Files is { Count: > 0 } files && files.All(p => File.Exists(p) || Directory.Exists(p));
            CopyButton.IsEnabled = valid;
            RevealButton.Visibility = valid ? Visibility.Visible : Visibility.Collapsed;
            PreviewMetadata.Text += valid ? " · 本地可用" : " · 源文件已移动或不可用";
        }
    }
    private void HistoryDoubleClick(object sender, MouseButtonEventArgs args) { /* Selection is preview only. */ }
    private void CopyClick(object sender, RoutedEventArgs args)
    {
        if (HistoryList.SelectedItem is not ClipboardEntry entry) return;
        try { ShowNotice(clipboard.SyncEnabled ? "已恢复本机剪贴板；跨设备结果取决于真实连接与服务回执。" : "已恢复本机系统剪贴板。"); clipboard.Restore(entry); }
        catch (Exception) { ShowNotice("无法恢复剪贴板。请检查文件是否仍存在、当前会话是否可用或剪贴板是否被占用。"); }
    }
    private void AddShelfClick(object sender, RoutedEventArgs args) { if (HistoryList.SelectedItem is ClipboardEntry entry) clipboard.ShowShelf(entry); }
    private void ShelfClick(object sender, RoutedEventArgs args) => clipboard.ShowShelf();
    private void RevealClick(object sender, RoutedEventArgs args)
    {
        if (HistoryList.SelectedItem is not ClipboardEntry entry || entry.Content.Files?.FirstOrDefault() is not string path) return;
        try { var start = new ProcessStartInfo("explorer.exe") { UseShellExecute = false }; start.ArgumentList.Add("/select,"); start.ArgumentList.Add(path); Process.Start(start); }
        catch (Exception) { ShowNotice("资源管理器无法打开此文件位置。"); }
    }
    private void FitClick(object sender, RoutedEventArgs args) => ImagePreview.Stretch = ImagePreview.Stretch == Stretch.Uniform ? Stretch.UniformToFill : Stretch.Uniform;
    private void SyncChanged(object sender, RoutedEventArgs args)
    {
        if (updatingSync || clipboard is null) return;
        clipboard.SyncEnabled = SyncCheck.IsChecked == true;
        ShowNotice(clipboard.SyncEnabled ? "已开启文字与图片同步；只发送之后的新复制，不重放历史。" : "已关闭跨设备剪贴板同步；本机历史继续记录。");
    }
    private void SnapshotChanged(JsonElement snapshot)
    {
        latest = snapshot;
        LocalName.Text = snapshot.String("local_name", "这台 Windows");
        LocalStatus.Text = snapshot.Bool("network_running") ? "服务运行中" : "网络未就绪";
        bool supportsClipboard = snapshot.TryGetProperty("capabilities", out var caps) && caps.Bool("clipboard");
        updatingSync = true;
        SyncCheck.IsEnabled = supportsClipboard || clipboard.SyncEnabled;
        SyncCheck.IsChecked = clipboard.SyncEnabled;
        updatingSync = false;
        SyncStatus.Text = supportsClipboard ? (clipboard.SyncEnabled ? "文字 / PNG / HTML / RTF · 最多 1 MiB" : "同步已关闭；远端新内容会明确拒收") : "桌面同步尚未就绪；本机历史可用";
        PairIpButton.IsEnabled = !busy && snapshot.Bool("network_running");
        string raw = snapshot.GetRawText();
        if (raw != lastDeviceSnapshot)
        {
            lastDeviceSnapshot = raw;
            RebuildDevices();
        }
        string error = snapshot.String("last_error");
        if (error.Length > 0) ShowNotice(error);
        settings?.RefreshService(snapshot);
    }
    private void ConnectionChanged(bool connected, string message)
    {
        if (!connected)
        {
            latest = null;
            LocalStatus.Text = "服务不可用 · 本机历史仍可用";
            updatingSync = true;
            SyncCheck.IsChecked = clipboard.SyncEnabled;
            SyncCheck.IsEnabled = clipboard.SyncEnabled;
            updatingSync = false;
            SyncStatus.Text = "同步暂停；重新连接不会重发旧历史";
            PairIpButton.IsEnabled = false;
            lastDeviceSnapshot = "";
            RebuildDevices();
            ShowNotice(message);
        }
        else if (!wasConnected) ShowNotice("uniDesk 已就绪。关闭窗口后仍在托盘运行；从托盘菜单可以退出。");
        wasConnected = connected;
    }
    private void DeviceSearchChanged(object sender, TextChangedEventArgs args) { if (service is not null && DevicesList is not null) RebuildDevices(); }
    private void RebuildDevices()
    {
        if (DevicesList is null) return;
        string? selected = (DevicesList.SelectedItem as DeviceRow)?.Id;
        devices = [new("local", latest?.String("local_name", "这台 Windows") ?? "这台 Windows", "本机", latest is null ? "服务未连接" : "Windows · 后台已连接", true, default)];
        if (latest is JsonElement snapshot)
        {
            devices.AddRange(snapshot.Array("peers").Select(peer => new DeviceRow(peer.String("peer_id"), peer.String("display_name", "未命名设备"), peer.String("trust") == "trusted" ? "已配对" : "附近设备", $"{ConnectionLabel(peer.String("connection"))} · {TrustLabel(peer.String("trust"))}", false, peer)));
        }
        string search = DeviceSearch.Text.Trim();
        var visible = devices.Where(d => search.Length == 0 || $"{d.Name} {d.Id}".Contains(search, StringComparison.CurrentCultureIgnoreCase)).ToArray();
        updatingDevices = true;
        DevicesList.ItemsSource = visible;
        DevicesList.SelectedItem = visible.FirstOrDefault(d => d.Id == selected) ?? visible.FirstOrDefault();
        updatingDevices = false;
        // Do not destroy a focused name field while periodic snapshots arrive.
        if (!DeviceDetails.IsKeyboardFocusWithin) RenderDeviceDetails();
    }
    private void DeviceSelectionChanged(object sender, SelectionChangedEventArgs args) { if (!updatingDevices) RenderDeviceDetails(); }
    private void RenderDeviceDetails()
    {
        if (DeviceDetails is null) return;
        DeviceDetails.Children.Clear();
        if (DevicesList.SelectedItem is not DeviceRow device) { DeviceDetails.Children.Add(Label("没有匹配设备", 24, true)); return; }
        DeviceDetails.Children.Add(Label(device.Name, 26, true));
        DeviceDetails.Children.Add(Label(device.Detail, 14, false, true));
        if (latest is null)
        {
            DeviceDetails.Children.Add(Label("正在连接 uniDesk 后台组件，程序会自动启动并重连，无需另外打开服务。具体状态见窗口底部。", 15));
            return;
        }
        foreach (var invite in latest.Value.Array("pending"))
        {
            var invitation = new StackPanel { Margin = new Thickness(0, 14, 0, 14) };
            invitation.Children.Add(Label($"{invite.String("display_name", "附近设备")} · {(invite.Bool("incoming") ? "邀请你配对" : "等待对方确认")}", 16, true));
            string owner = invite.String("owner_name");
            if (owner.Length > 0) invitation.Children.Add(Label($"工作空间所有者：{owner}", 13, false, true));
            var members = invite.Array("member_names").Where(v => v.ValueKind == JsonValueKind.String).Select(v => v.GetString()).ToArray();
            if (members.Length > 0) invitation.Children.Add(Label($"成员：{string.Join("、", members)}", 13, false, true));
            if (invite.Bool("incoming"))
            {
                string id = invite.String("request_id");
                var actions = new WrapPanel();
                actions.Children.Add(ActionButton("接受配对", "AcceptPairing", new { request_id = id }));
                actions.Children.Add(ActionButton("拒绝", "RejectPairing", new { request_id = id }));
                invitation.Children.Add(actions);
            }
            DeviceDetails.Children.Add(new Border { Background = new SolidColorBrush(Color.FromRgb(232, 243, 255)), Padding = new Thickness(16), CornerRadius = new CornerRadius(8), Child = invitation, Margin = new Thickness(0, 14, 0, 10) });
        }
        string pairing = latest.Value.String("pairing_status");
        if (pairing.Length > 0) DeviceDetails.Children.Add(Label(pairing, 13, false, true));
        if (device.Local)
        {
            DeviceDetails.Children.Add(Label($"设备 ID：{latest.Value.String("local_device_id")}", 13));
            DeviceDetails.Children.Add(Label($"服务版本：{latest.Value.String("package_version")}\n控制端口：{latest.Value.Number("control_port")}", 14));
            DeviceDetails.Children.Add(Label("本机名称", 17, true));
            var name = new TextBox { Text = latest.Value.String("local_name"), MaxLength = 128, Margin = new Thickness(0, 8, 0, 10) };
            DeviceDetails.Children.Add(name);
            var rename = new Button { Content = "保存本机名称", HorizontalAlignment = HorizontalAlignment.Left };
            rename.Click += async (_, _) => { if (!string.IsNullOrWhiteSpace(name.Text)) await RunCommand("Rename", new { name = name.Text.Trim() }); };
            DeviceDetails.Children.Add(rename);
            DeviceDetails.Children.Add(Label("身份和信任由服务保存；不会因客户端重启或改名重建身份。", 13, false, true));
            return;
        }
        var peer = device.Peer;
        DeviceDetails.Children.Add(Label($"设备 ID：{device.Id}", 13));
        DeviceDetails.Children.Add(Label("连接候选", 18, true));
        bool connected = peer.String("connection") == "connected";
        string active = "";
        if (connected && peer.TryGetProperty("selected_route", out var selected) && selected.ValueKind == JsonValueKind.String)
        {
            var route = peer.Array("routes").FirstOrDefault(r => r.String("id") == selected.GetString());
            if (route.ValueKind == JsonValueKind.Object && route.TryGetProperty("endpoint", out var endpoint)) active = EndpointLabel(endpoint);
        }
        DeviceDetails.Children.Add(Label(active.Length > 0 ? $"当前活动路径：{active}" : "当前没有已确认的活动地址", 13, false, true));
        var routes = peer.Array("routes").ToArray();
        if (routes.Length == 0) DeviceDetails.Children.Add(Label("还没有连接候选。等待自动发现，或在左下角通过 IPv4 定位。", 14));
        foreach (var route in routes)
        {
            string endpoint = route.TryGetProperty("endpoint", out var endpointValue) ? EndpointLabel(endpointValue) : "地址未知";
            string source = route.TryGetProperty("source", out var sourceValue) ? JsonLabel(sourceValue) : "来源未知";
            DeviceDetails.Children.Add(Label($"{endpoint}\n来源：{source} · 健康：{route.String("health", "unknown")} · 接口：{route.String("path_kind", "未识别")}", 13));
        }
        DeviceDetails.Children.Add(Label("候选地址不代表已连通；不会根据 IP 前缀猜测 Wi-Fi 或雷电。", 12, false, true));
        var routePreference = new ComboBox { IsEnabled = false, ItemsSource = new[] { "自动路径（服务管理）" }, SelectedIndex = 0, Margin = new Thickness(0, 12, 0, 16) };
        routePreference.ToolTip = "当前桌面 IPC 尚未开放路径偏好命令。";
        DeviceDetails.Children.Add(routePreference);
        var buttons = new WrapPanel();
        if (peer.String("trust") != "trusted") buttons.Children.Add(ActionButton("邀请配对", "RequestPairing", new { peer_id = device.Id }));
        if (connected) buttons.Children.Add(ActionButton("断开连接", "Disconnect", new { peer_id = device.Id }));
        if (peer.String("trust") == "trusted") buttons.Children.Add(ActionButton("撤销信任", "Revoke", new { peer_id = device.Id }));
        DeviceDetails.Children.Add(buttons);
    }
    private Button ActionButton(string label, string command, object args)
    {
        var button = new Button { Content = label, IsEnabled = service.IsConnected && !busy };
        button.Click += async (_, _) => await RunCommand(command, args);
        return button;
    }
    private async Task RunCommand(string command, object args)
    {
        if (busy) return;
        busy = true;
        PairIpButton.IsEnabled = false;
        try
        {
            var reply = await service.SendAsync(ServiceClient.Command(command, args));
            ShowNotice(reply.TryGetProperty("Queued", out _) ? "操作已提交；等待服务和对端的真实状态更新。" : "服务已响应；以设备列表中的实际状态为准。");
        }
        catch (Exception ex) { ShowNotice(ex is IOException ? ex.Message : "操作未完成，服务可能已断开或请求超时。"); }
        finally { busy = false; PairIpButton.IsEnabled = service.IsConnected; }
    }
    private async void PairIpClick(object sender, RoutedEventArgs args)
    {
        string ip = ManualIp.Text.Trim();
        if (ip.Split('.').Length != 4 || !IPAddress.TryParse(ip, out var address) || address.AddressFamily != AddressFamily.InterNetwork || address.ToString() != ip)
        { ShowNotice("请输入完整 IPv4 地址，例如 192.168.1.23。IP 仅用于定位，不授予信任。"); return; }
        await RunCommand("RequestPairingAt", new { ipv4 = ip });
    }
    private void DisplayCanvasSizeChanged(object sender, SizeChangedEventArgs args) => DrawDisplays();
    private void DrawDisplays()
    {
        if (DisplayCanvas is null || DisplayCanvas.ActualWidth < 10 || DisplayCanvas.ActualHeight < 10) return;
        DisplayCanvas.Children.Clear();
        var screens = System.Windows.Forms.Screen.AllScreens;
        if (screens.Length == 0) return;
        int left = screens.Min(s => s.Bounds.Left), top = screens.Min(s => s.Bounds.Top);
        int right = screens.Max(s => s.Bounds.Right), bottom = screens.Max(s => s.Bounds.Bottom);
        double scale = Math.Min((DisplayCanvas.ActualWidth - 50) / Math.Max(1, right - left), (DisplayCanvas.ActualHeight - 110) / Math.Max(1, bottom - top));
        scale = Math.Max(0.01, scale);
        double offsetX = (DisplayCanvas.ActualWidth - (right - left) * scale) / 2;
        double offsetY = Math.Max(20, (DisplayCanvas.ActualHeight - 70 - (bottom - top) * scale) / 2);
        for (int i = 0; i < screens.Length; i++)
        {
            var screen = screens[i];
            var content = new StackPanel { VerticalAlignment = VerticalAlignment.Center, HorizontalAlignment = HorizontalAlignment.Center, Margin = new Thickness(8) };
            content.Children.Add(new TextBlock { Text = (i + 1).ToString(), FontSize = 28, FontWeight = FontWeights.SemiBold, TextAlignment = TextAlignment.Center });
            content.Children.Add(new TextBlock { Text = screen.DeviceName, FontSize = 12, TextAlignment = TextAlignment.Center });
            content.Children.Add(new TextBlock { Text = $"{screen.Bounds.Width} × {screen.Bounds.Height} px{(screen.Primary ? " · 主屏" : "")}", FontSize = 12, TextAlignment = TextAlignment.Center });
            var rectangle = new Border { Width = screen.Bounds.Width * scale, Height = screen.Bounds.Height * scale, BorderBrush = new SolidColorBrush(Color.FromRgb(40, 114, 193)), BorderThickness = new Thickness(2), Background = new SolidColorBrush(Color.FromRgb(221, 236, 250)), CornerRadius = new CornerRadius(6), Child = content };
            Canvas.SetLeft(rectangle, offsetX + (screen.Bounds.Left - left) * scale);
            Canvas.SetTop(rectangle, offsetY + (screen.Bounds.Top - top) * scale);
            DisplayCanvas.Children.Add(rectangle);
        }
    }
    private static TextBlock Label(string text, double size = 14, bool bold = false, bool muted = false) => new() { Text = text, FontSize = size, FontWeight = bold ? FontWeights.SemiBold : FontWeights.Normal, Foreground = muted ? new SolidColorBrush(Color.FromRgb(101, 118, 140)) : new SolidColorBrush(Color.FromRgb(24, 36, 60)), Margin = new Thickness(0, 8, 0, 10), TextWrapping = TextWrapping.Wrap };
    private static string EndpointLabel(JsonElement value)
    {
        if (value.ValueKind != JsonValueKind.Object) return JsonLabel(value);
        string host = value.String("host", "地址未知");
        if (host.Contains(':')) host = $"[{host}]";
        return $"{host}:{value.Number("port")}";
    }
    private static string JsonLabel(JsonElement value) => value.ValueKind == JsonValueKind.String ? value.GetString() ?? "" : value.GetRawText();
    private static string ConnectionLabel(string value) => value switch { "connected" => "已连接", "connecting" => "连接中", "disconnected" => "未连接", "backoff" => "等待重连", "failed" => "连接失败", _ => value.Length == 0 ? "未连接" : value };
    private static string TrustLabel(string value) => value switch { "trusted" => "已信任", "untrusted" or "unknown" => "未配对", "pairing" => "配对中", "pending" => "等待确认", "blocked" => "已阻止", _ => value.Length == 0 ? "未配对" : value };
    internal static string FormatBytes(long value) => value >= 1024 * 1024 ? $"{value / (1024.0 * 1024):N1} MiB" : value >= 1024 ? $"{value / 1024.0:N1} KiB" : $"{value} B";
    private void ShowNotice(string message) => Notice.Text = message;
    private void SettingsClick(object sender, RoutedEventArgs args) => ShowSettings();
    public void ShowSettings()
    {
        if (settings is null) { settings = new SettingsWindow(clipboard, service) { Owner = this }; settings.Closed += (_, _) => settings = null; }
        settings.Show(); settings.Activate();
    }
    private void KeyboardShortcut(object sender, System.Windows.Input.KeyEventArgs args)
    {
        if (Keyboard.Modifiers == ModifierKeys.Control && args.Key == Key.F) { if (Navigation.SelectedIndex == 2) DeviceSearch.Focus(); else { Navigation.SelectedIndex = 0; HistorySearch.Focus(); } args.Handled = true; }
        else if (Keyboard.Modifiers == ModifierKeys.Control && args.Key == Key.OemComma) { ShowSettings(); args.Handled = true; }
        else if (args.Key == Key.Enter && HistoryList.IsKeyboardFocusWithin) { CopyClick(this, args); args.Handled = true; }
    }
    private void OnClosing(object? sender, CancelEventArgs args)
    {
        if (Application.Current is App { Exiting: false }) { args.Cancel = true; Hide(); return; }
        clipboard.Entries.CollectionChanged -= HistoryChanged;
        clipboard.Error -= ShowNotice;
        service.SnapshotChanged -= SnapshotChanged;
        service.ConnectionChanged -= ConnectionChanged;
    }
    private sealed record DeviceRow(string Id, string Name, string Group, string Detail, bool Local, JsonElement Peer);
}
