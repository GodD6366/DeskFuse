using System.Diagnostics;
using System.Net;
using System.Net.Sockets;
using System.Text.Json;
using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Windows.Storage;

namespace DeskFuse.Windows;

public sealed partial class MainWindow : Window
{
    private readonly ServiceClient service;
    private readonly DesktopClipboardController clipboard;
    private readonly WorkflowExecutionPort workflowExecution;
    private SettingsWindow? settings;
    private RemoteAppWindow? remoteApps;
    private List<DeviceRow> devices = [];
    private bool updatingHistory;
    private bool updatingDevices;
    private bool updatingSync;
    private bool wasConnected;
    private bool loaded;
    private string lastDeviceSnapshot = "";
    private JsonElement? latest;
    private bool busy;
    private bool updatingInputSettings;
    private readonly List<WorkflowRecord> workflows = [];
    private WorkflowRecord? editingWorkflow;
    private Border? draggingDisplay;
    private global::Windows.Foundation.Point dragOrigin;
    private readonly Dictionary<int, WorkflowRecord> hotkeys = [];
    private const uint WmHotKey = 0x0312;
    private const uint ModAlt = 0x0001;
    private const uint ModControl = 0x0002;
    private const uint ModShift = 0x0004;
    private const uint ModWin = 0x0008;
    public IntPtr Handle { get; }
    /// <summary>Set by the app before a real shutdown; otherwise close hides to tray.</summary>
    public bool AllowClose { get; set; }

    public MainWindow(ServiceClient service, DesktopClipboardController clipboard)
    {
        this.service = service;
        this.clipboard = clipboard;
        InitializeComponent();
        Handle = WinRT.Interop.WindowNative.GetWindowHandle(this);
        AppIcon.Apply(this);
        AppWindow.Title = "DeskFuse";
        AppWindow.Resize(new global::Windows.Graphics.SizeInt32(1200, 780));
        AppWindow.Closing += (_, args) =>
        {
            if (AllowClose) { RemoveHotkeys(); return; }
            args.Cancel = true;
            HideToTray();
        };
        workflowExecution = new WindowsWorkflowExecutionPort(service, clipboard);
        clipboard.Entries.CollectionChanged += (_, _) => RefreshHistory();
        clipboard.Error += Notify;
        clipboard.FileOfferReceived += FileOfferArrived;
        clipboard.FileTransferSettled += FileTransferDone;
        service.SnapshotChanged += SnapshotChanged;
        service.ConnectionChanged += ConnectionChanged;
        Root.KeyDown += KeyboardShortcut;
        DeviceDetails.LostFocus += (_, _) => RenderDeviceDetails();
        RefreshHistory();
        ConnectionChanged(false, "正在连接 DeskFuseService…");
        DrawInputLayout(default);
        Root.Loaded += (_, _) => { loaded = true; InstallWorkflowHotkeys(); };
    }

    public void HideToTray() => AppWindow.Hide();
    public void RestoreFromTray()
    {
        AppWindow.Show();
        Activate();
    }

    private void NavigationChanged(object sender, SelectionChangedEventArgs args)
    {
        if (ClipboardPage is null) return;
        ClipboardPage.Visibility = Navigation.SelectedIndex == 0 ? Visibility.Visible : Visibility.Collapsed;
        InputPage.Visibility = Navigation.SelectedIndex == 1 ? Visibility.Visible : Visibility.Collapsed;
        DevicesPage.Visibility = Navigation.SelectedIndex == 2 ? Visibility.Visible : Visibility.Collapsed;
        TransfersPage.Visibility = Navigation.SelectedIndex == 3 ? Visibility.Visible : Visibility.Collapsed;
        WorkflowsPage.Visibility = Navigation.SelectedIndex == 4 ? Visibility.Visible : Visibility.Collapsed;
        if (Navigation.SelectedIndex == 1) DrawInputLayout(latest.HasValue && latest.Value.TryGetProperty("input", out var layoutInput) ? layoutInput : default);
        if (Navigation.SelectedIndex == 4) _ = LoadWorkflowsAsync();
    }

    private void HistoryFilterChanged(object sender, RoutedEventArgs args) => RefreshHistory();

    private void RefreshHistory()
    {
        if (HistoryList is null || updatingHistory) return;
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

    private void HistoryDoubleClick(object sender, DoubleTappedRoutedEventArgs args) { /* Selection is preview only. */ }

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
            var image = ClipboardImages.Create(entry.Content.Data);
            if (image is null) { TextPreview.Text = "图片无法预览，可能已损坏或超过解码限制。"; CopyButton.IsEnabled = false; }
            else
            {
                ImagePreview.Source = image;
                ImagePreview.Visibility = Visibility.Visible;
                TextPreview.Visibility = Visibility.Collapsed;
                FitButton.Visibility = Visibility.Visible;
            }
        }
        if (entry.Kind == "files")
        {
            bool valid = entry.Content.Files is { Count: > 0 } files && files.All(p => File.Exists(p) || Directory.Exists(p));
            CopyButton.IsEnabled = valid;
            RevealButton.Visibility = valid ? Visibility.Visible : Visibility.Collapsed;
            PreviewMetadata.Text += valid ? " · 本地可用" : " · 源文件已移动或不可用";
        }
    }

    private void DeleteHistoryClick(object sender, RoutedEventArgs args)
    {
        if (HistoryList.SelectedItem is not ClipboardEntry entry) return;
        clipboard.Remove(entry);
        Notify("已删除这条本机历史；系统剪贴板、用户源文件和已粘贴文件保持不变。");
    }

    private void CopyClick(object sender, RoutedEventArgs args) => CopySelection();

    /// <summary>Tray "recent clipboard" entry point.</summary>
    public void RestoreEntry(ClipboardEntry entry)
    {
        try { Notify(clipboard.SyncEnabled ? "已恢复本机剪贴板；跨设备结果取决于真实连接与服务回执。" : "已恢复本机系统剪贴板。"); clipboard.Restore(entry); }
        catch (Exception) { Notify("无法恢复剪贴板。请检查文件是否仍存在、当前会话是否可用或剪贴板是否被占用。"); }
    }

    private void CopySelection()
    {
        if (HistoryList.SelectedItem is ClipboardEntry entry) RestoreEntry(entry);
    }

    private void AddShelfClick(object sender, RoutedEventArgs args) { if (HistoryList.SelectedItem is ClipboardEntry entry) clipboard.ShowShelf(entry); }

    private void ShelfClick(object sender, RoutedEventArgs args) => clipboard.ShowShelf();

    private void RemoteAppsClick(object sender, RoutedEventArgs args) => ShowRemoteApps();

    public void ShowRemoteApps()
    {
        if (remoteApps is null) remoteApps = new RemoteAppWindow(service);
        remoteApps.Show();
    }

    private void RevealClick(object sender, RoutedEventArgs args)
    {
        if (HistoryList.SelectedItem is not ClipboardEntry entry || entry.Content.Files?.FirstOrDefault() is not string path) return;
        try { var start = new ProcessStartInfo("explorer.exe") { UseShellExecute = false }; start.ArgumentList.Add("/select,"); start.ArgumentList.Add(path); Process.Start(start); }
        catch (Exception) { Notify("资源管理器无法打开此文件位置。"); }
    }

    private void FitClick(object sender, RoutedEventArgs args) => ImagePreview.Stretch = ImagePreview.Stretch == Stretch.Uniform ? Stretch.UniformToFill : Stretch.Uniform;

    private void SyncChanged(object sender, RoutedEventArgs args)
    {
        if (updatingSync) return;
        clipboard.SyncEnabled = SyncCheck.IsChecked == true;
        Notify(clipboard.SyncEnabled ? "已开启文字与图片同步；只发送之后的新复制，不重放历史。" : "已关闭跨设备剪贴板同步；本机历史继续记录。");
    }

    private void SnapshotChanged(JsonElement snapshot)
    {
        latest = snapshot;
        LocalName.Text = snapshot.String("local_name", "这台 Windows");
        LocalStatus.Text = snapshot.Bool("network_running") ? "服务运行中" : "网络未就绪";
        bool supportsClipboard = snapshot.TryGetProperty("capabilities", out var caps) && caps.Bool("clipboard");
        bool supportsFiles = caps.ValueKind == JsonValueKind.Object && caps.Bool("files");
        updatingSync = true;
        SyncCheck.IsEnabled = supportsClipboard || clipboard.SyncEnabled;
        SyncCheck.IsChecked = clipboard.SyncEnabled;
        updatingSync = false;
        SyncStatus.Text = supportsClipboard
            ? (clipboard.SyncEnabled
                ? (supportsFiles ? "文字 / 图片 / 富文本 / 文件 · 文本最多 16 MiB" : "文字 / PNG / HTML / RTF · 最多 16 MiB")
                : "同步已关闭；远端新内容会明确拒收")
            : "桌面同步尚未就绪；本机历史可用";
        PairIpButton.IsEnabled = !busy && snapshot.Bool("network_running");
        RefreshInputState(snapshot);
        RenderTransfers(snapshot);
        string raw = snapshot.GetRawText();
        if (raw != lastDeviceSnapshot)
        {
            lastDeviceSnapshot = raw;
            RebuildDevices();
        }
        string error = snapshot.String("last_error");
        if (error.Length > 0) Notify(error);
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
            InputAvailability.Text = "普通桌面输入 helper 正在等待服务重连；不会接受新的远端输入。";
            InputReceiverReady.IsChecked = false;
            ReclaimInputButton.IsEnabled = false;
            lastDeviceSnapshot = "";
            RebuildDevices();
            Notify(message);
        }
        else if (!wasConnected)
        {
            Notify("DeskFuse 已就绪。关闭窗口后仍在托盘运行；从托盘菜单可以退出。");
            _ = LoadWorkflowsAsync();
        }
        wasConnected = connected;
    }

    private void RefreshInputState(JsonElement snapshot)
    {
        bool capable = snapshot.TryGetProperty("capabilities", out var capabilities) && capabilities.Bool("input");
        if (!snapshot.TryGetProperty("input", out var input) || input.ValueKind != JsonValueKind.Object)
        {
            InputAvailability.Text = "当前服务未提供 Windows 输入 helper 状态。";
            InputReceiverReady.IsChecked = false;
            ReclaimInputButton.IsEnabled = false;
            DrawInputLayout(default);
            return;
        }
        bool managed = input.Bool("managed");
        string status = input.String("status", "standby");
        string reason = input.String("reason");
        string controllerId = input.String("controller_id");
        string controller = snapshot.Array("peers").FirstOrDefault(peer => peer.String("peer_id") == controllerId).String("display_name", controllerId);
        bool ready = capable && status.Equals("Ready", StringComparison.OrdinalIgnoreCase);
        InputReceiverReady.IsChecked = ready;
        ReclaimInputButton.IsEnabled = managed;
        InputAvailability.Text = !capable
            ? "普通桌面输入 helper 尚未就绪；远端输入保持关闭。"
            : managed
                ? $"{(ready ? "正在被控" : "被控准备中")} · 主控：{(controller.Length > 0 ? controller : "已授权设备")}{(reason.Length > 0 ? $"\n{reason}" : "")}"
                : $"输入 helper 已连接 · {input.Number("display_count")} 个本机显示器 · 等待设备网络中的主控。";
        // 主控状态：设置角色 + 当前持有的控制会话。
        var settings = input.TryGetProperty("settings", out var sharing) && sharing.ValueKind == JsonValueKind.Object ? sharing : default;
        bool controllerEnabled = settings.ValueKind == JsonValueKind.Object && settings.Bool("enabled") && settings.String("role") == "Controller";
        if (controllerEnabled)
        {
            string controllingId = input.TryGetProperty("controlling", out var controlling) && controlling.ValueKind == JsonValueKind.Object ? controlling.String("peer_id") : "";
            string target = input.TryGetProperty("controlling", out var controlling2) && controlling2.ValueKind == JsonValueKind.Object ? controlling2.String("target_display") : "";
            string peer = snapshot.Array("peers").FirstOrDefault(p => p.String("peer_id") == controllingId).String("display_name", controllingId);
            InputAvailability.Text += $"\n本机已开启主控{(peer.Length > 0 && controllingId.Length > 0 ? $"：正在控制 {peer}{(target.Length > 0 ? $"（{target}）" : "")}" : "；把指针移到屏幕边缘即可控制对端")}";
        }
        RenderShortcutBindings(input);
        DrawInputLayout(input);
    }

    private void RenderShortcutBindings(JsonElement input)
    {
        if (ShortcutBindings is null || updatingInputSettings || !input.TryGetProperty("settings", out var settings) || settings.ValueKind != JsonValueKind.Object) return;
        if (!settings.TryGetProperty("shortcuts", out var shortcuts) || shortcuts.ValueKind != JsonValueKind.Object) return;
        string[] names = ["left", "right", "up", "down", "lock"];
        var lines = names.Select(name =>
        {
            var shortcut = shortcuts.TryGetProperty(name, out var value) ? value : default;
            return $"{shortcut.Number("key_code")}, {shortcut.Number("modifiers")}";
        });
        updatingInputSettings = true;
        ShortcutBindings.Text = string.Join(Environment.NewLine, lines);
        updatingInputSettings = false;
    }

    /// <summary>Draws the shared screen layout: local displays plus remote
    /// participant topologies, positioned like the runtime layout resolver.</summary>
    private void DrawInputLayout(JsonElement input)
    {
        if (DisplayCanvas is null || DisplayCanvas.ActualWidth < 10 || DisplayCanvas.ActualHeight < 10) return;
        DisplayCanvas.Children.Clear();
        var monitors = MonitorLayout.All();
        if (monitors.Count == 0) return;
        int left = monitors.Min(m => m.Left), top = monitors.Min(m => m.Top);
        int right = monitors.Max(m => m.Left + m.Width), bottom = monitors.Max(m => m.Top + m.Height);
        double scale = Math.Min((DisplayCanvas.ActualWidth - 50) / Math.Max(1, right - left), (DisplayCanvas.ActualHeight - 110) / Math.Max(1, bottom - top));
        scale = Math.Max(0.01, scale);
        double offsetX = (DisplayCanvas.ActualWidth - (right - left) * scale) / 2;
        double offsetY = Math.Max(20, (DisplayCanvas.ActualHeight - 70 - (bottom - top) * scale) / 2);
        for (int i = 0; i < monitors.Count; i++)
        {
            var monitor = monitors[i];
            var content = new StackPanel { VerticalAlignment = VerticalAlignment.Center, HorizontalAlignment = HorizontalAlignment.Center, Margin = new Thickness(8) };
            content.Children.Add(new TextBlock { Text = (i + 1).ToString(), FontSize = 28, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold, TextAlignment = TextAlignment.Center });
            content.Children.Add(new TextBlock { Text = monitor.DeviceName, FontSize = 12, TextAlignment = TextAlignment.Center });
            content.Children.Add(new TextBlock { Text = $"{monitor.Width} × {monitor.Height} px{(monitor.Primary ? " · 主屏" : "")}", FontSize = 12, TextAlignment = TextAlignment.Center });
            var rectangle = new Border
            {
                Width = monitor.Width * scale,
                Height = monitor.Height * scale,
                BorderBrush = new SolidColorBrush(Microsoft.UI.ColorHelper.FromArgb(255, 40, 114, 193)),
                BorderThickness = new Thickness(2),
                Background = new SolidColorBrush(Microsoft.UI.ColorHelper.FromArgb(255, 221, 236, 250)),
                CornerRadius = new CornerRadius(6),
                Child = content,
            };
            Canvas.SetLeft(rectangle, offsetX + (monitor.Left - left) * scale);
            Canvas.SetTop(rectangle, offsetY + (monitor.Top - top) * scale);
            DisplayCanvas.Children.Add(rectangle);
        }
        // Remote groups render after the local group with their own layout.
        double remoteOffset = offsetX + (right - left) * scale + 24;
        double remoteY = offsetY;
        var seen = new HashSet<string>();
        if (input.ValueKind == JsonValueKind.Object)
        {
            foreach (var remote in input.Array("remote_topologies"))
            {
                string peer = remote.String("peer_id");
                if (!seen.Add(peer)) continue;
                string name = service.LatestSnapshot?.Array("peers").FirstOrDefault(p => p.String("peer_id") == peer).String("display_name", "远端") ?? "远端";
                var topology = remote.GetProperty("topology");
                int minX = int.MaxValue, minY = int.MaxValue, maxX = 0, maxY = 0;
                foreach (var display in topology.Array("displays"))
                {
                    int x = (int)display.Number("frame_x");
                    int y = (int)display.Number("frame_y");
                    int width = (int)display.Number("width");
                    int height = (int)display.Number("height");
                    minX = Math.Min(minX, x); minY = Math.Min(minY, y);
                    maxX = Math.Max(maxX, x + width); maxY = Math.Max(maxY, y + height);
                }
                if (minX == int.MaxValue) continue;
                foreach (var display in topology.Array("displays"))
                {
                    int x = (int)display.Number("frame_x");
                    int y = (int)display.Number("frame_y");
                    int width = (int)display.Number("width");
                    int height = (int)display.Number("height");
                    var content = new StackPanel { VerticalAlignment = VerticalAlignment.Center, HorizontalAlignment = HorizontalAlignment.Center, Margin = new Thickness(6) };
                    content.Children.Add(new TextBlock { Text = name, FontSize = 13, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold, TextAlignment = TextAlignment.Center });
                    content.Children.Add(new TextBlock { Text = $"{width} × {height} px", FontSize = 11, TextAlignment = TextAlignment.Center });
                    var rectangle = new Border
                    {
                        Width = width * scale,
                        Height = height * scale,
                        BorderBrush = new SolidColorBrush(Microsoft.UI.ColorHelper.FromArgb(255, 22, 163, 74)),
                        BorderThickness = new Thickness(2),
                        Background = new SolidColorBrush(Microsoft.UI.ColorHelper.FromArgb(255, 220, 245, 228)),
                        CornerRadius = new CornerRadius(6),
                        Child = content,
                        Tag = new RemoteDisplayDrag(peer, display.String("id"), x, y, scale),
                    };
                    rectangle.PointerPressed += DisplayDragStarted;
                    rectangle.PointerMoved += DisplayDragMoved;
                    rectangle.PointerReleased += DisplayDragFinished;
                    Canvas.SetLeft(rectangle, remoteOffset + (x - minX) * scale);
                    Canvas.SetTop(rectangle, remoteY + (y - minY) * scale);
                    DisplayCanvas.Children.Add(rectangle);
                }
                remoteY += (maxY - minY) * scale + 24;
            }
        }
    }

    private async void SaveShortcutBindingsClick(object sender, RoutedEventArgs args)
    {
        if (updatingInputSettings || latest is not JsonElement snapshot || !snapshot.TryGetProperty("input", out var input) || !input.TryGetProperty("settings", out var settings)) return;
        var lines = ShortcutBindings.Text.Split(["\r\n", "\n"], StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries);
        if (lines.Length != 5) { Notify("需要按左、右、上、下、锁定顺序填写五行快捷键。 "); return; }
        try
        {
            var node = JsonNode.Parse(settings.GetRawText())?.AsObject() ?? throw new FormatException();
            var shortcuts = node["shortcuts"]?.AsObject() ?? throw new FormatException();
            string[] names = ["left", "right", "up", "down", "lock"];
            for (int i = 0; i < names.Length; i++)
            {
                var values = lines[i].Split(',', StringSplitOptions.TrimEntries);
                if (values.Length != 2 || !ushort.TryParse(values[0], out var key) || !ulong.TryParse(values[1], out var modifiers)) throw new FormatException();
                shortcuts[names[i]] = new JsonObject { ["key_code"] = key, ["modifiers"] = modifiers };
            }
            await service.SendAsync(ServiceClient.Command("SetInputSharingSettings", new { settings = node }));
            Notify("快捷键已提交；以输入状态和服务持久化结果为准。");
        }
        catch (FormatException) { Notify("快捷键格式无效，请使用 key code, modifiers，每行一组。"); }
        catch (Exception ex) { Notify(ex is IOException ? ex.Message : "快捷键未保存，服务可能已断开。"); }
    }

    private sealed record RemoteDisplayDrag(string DeviceId, string DisplayId, double X, double Y, double Scale);

    private void DisplayDragStarted(object sender, PointerRoutedEventArgs args)
    {
        if (sender is not Border border || border.Tag is not RemoteDisplayDrag || latest is not JsonElement snapshot || !snapshot.TryGetProperty("input", out var input) || !input.TryGetProperty("settings", out var settings) || settings.ValueKind != JsonValueKind.Object) return;
        if (!settings.TryGetProperty("role", out var role) || role.GetString() != "Controller" || !settings.Bool("enabled")) { Notify("只有已启用的本机主控可以编辑远端布局。"); return; }
        draggingDisplay = border;
        dragOrigin = args.GetCurrentPoint(DisplayCanvas).Position;
        border.CapturePointer(args.Pointer);
        args.Handled = true;
    }

    private void DisplayDragMoved(object sender, PointerRoutedEventArgs args)
    {
        if (draggingDisplay is not Border border || border.Tag is not RemoteDisplayDrag drag || !ReferenceEquals(border, sender)) return;
        var position = args.GetCurrentPoint(DisplayCanvas).Position;
        double deltaX = position.X - dragOrigin.X;
        double deltaY = position.Y - dragOrigin.Y;
        Canvas.SetLeft(border, Canvas.GetLeft(border) + deltaX);
        Canvas.SetTop(border, Canvas.GetTop(border) + deltaY);
        dragOrigin = position;
        border.Tag = drag with { X = drag.X + deltaX / Math.Max(0.01, drag.Scale), Y = drag.Y + deltaY / Math.Max(0.01, drag.Scale) };
    }

    private async void DisplayDragFinished(object sender, PointerRoutedEventArgs args)
    {
        if (sender is not Border border || border.Tag is not RemoteDisplayDrag drag) return;
        border.ReleasePointerCapture(args.Pointer);
        draggingDisplay = null;
        try
        {
            await service.SendAsync(ServiceClient.Command("MoveControlDevice", new { device_id = drag.DeviceId, display_id = drag.DisplayId, x = drag.X, y = drag.Y }));
            Notify("布局位置已提交；运行时会进行吸附并持久化。");
        }
        catch (Exception ex) { Notify(ex is IOException ? ex.Message : "布局未保存，服务可能已断开。"); }
        args.Handled = true;
    }

    private async void ReclaimInputClick(object sender, RoutedEventArgs args)
    {
        try
        {
            await service.SendAsync("ReclaimInput");
            Notify("已请求停止被控并释放远端按键、鼠标与控制租约。");
        }
        catch (Exception ex) { Notify(ex is IOException ? ex.Message : "停止被控请求未完成。"); }
    }

    private sealed record TransferRow(string Id, string Title, string Detail, string State, double Progress, bool CanReceive, bool CanCancel);

    private void RenderTransfers(JsonElement snapshot)
    {
        if (TransferList is null || TransferSummary is null) return;
        TransferList.Children.Clear();
        var rows = new List<TransferRow>();
        foreach (var transfer in snapshot.Array("file_transfers"))
        {
            string id = transfer.String("transfer_id");
            if (id.Length == 0) continue;
            string direction = transfer.String("direction");
            string state = transfer.String("state");
            var names = transfer.Array("names").Where(v => v.ValueKind == JsonValueKind.String).Select(v => v.GetString() ?? "").ToArray();
            string title = names.Length == 1 ? names[0] : names.Length > 1 ? $"{names.Length} 个项目" : "文件传输";
            string peer = transfer.String("peer_id");
            string peerName = snapshot.Array("peers").FirstOrDefault(p => p.String("peer_id") == peer).String("display_name", peer);
            long total = transfer.Number("total_bytes");
            long sent = transfer.Number("transferred_bytes");
            string detail = transfer.String("detail");
            string summary = $"{(direction == "outgoing" ? "发送到" : "接收自")} {peerName} · {FormatBytes(total)}";
            if (detail.Length > 0) summary += $" · {detail}";
            double progress = total > 0 ? Math.Clamp((double)sent / total, 0, 1) : (state == "completed" ? 1 : 0);
            rows.Add(new TransferRow(id, title, summary, StateLabel(state), progress, state is "offered", state is "offered" or "requested" or "receiving"));
        }
        var cache = snapshot.TryGetProperty("file_cache", out var fileCache) ? fileCache : default;
        TransferSummary.Text = cache.ValueKind == JsonValueKind.Object
            ? $"共 {rows.Count} 项 · 服务缓存 {FormatBytes(cache.Number("used_bytes"))} / {FormatBytes(cache.Number("max_bytes"))}"
            : $"共 {rows.Count} 项";
        foreach (var row in rows) TransferList.Children.Add(BuildTransferRow(row));
    }

    private UIElement BuildTransferRow(TransferRow row)
    {
        var panel = new StackPanel();
        panel.Children.Add(new TextBlock { Text = row.Title, FontSize = 15, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold });
        panel.Children.Add(new TextBlock { Text = row.Detail, FontSize = 12, Foreground = new SolidColorBrush(Microsoft.UI.ColorHelper.FromArgb(255, 101, 118, 140)) });
        panel.Children.Add(new ProgressBar { Value = row.Progress * 100, Height = 4, Margin = new Thickness(0, 8, 0, 8) });
        var actions = new StackPanel { Orientation = Orientation.Horizontal };
        if (row.CanReceive)
        {
            var receive = new Button { Content = "选择位置并接收…", Padding = new Thickness(10, 3, 10, 3), Margin = new Thickness(0, 0, 8, 0) };
            receive.Click += async (_, _) => await ReceiveAsync(row.Id);
            actions.Children.Add(receive);
        }
        if (row.CanCancel)
        {
            var cancel = new Button { Content = "取消", Padding = new Thickness(10, 3, 10, 3) };
            cancel.Click += async (_, _) => { await clipboard.CancelFileAsync(row.Id); Notify("已请求取消该文件传输。"); };
            actions.Children.Add(cancel);
        }
        panel.Children.Add(actions);
        return new Border
        {
            CornerRadius = new CornerRadius(8),
            BorderBrush = new SolidColorBrush(Microsoft.UI.ColorHelper.FromArgb(255, 203, 213, 225)),
            BorderThickness = new Thickness(1),
            Padding = new Thickness(12),
            Margin = new Thickness(0, 6, 0, 0),
            Child = panel,
        };
    }

    private async Task ReceiveAsync(string transferId)
    {
        if (!clipboard.FilesEnabled) { Notify("跨设备文件传输尚未就绪；请确认对端也是新版本。"); return; }
        var folder = Dialogs.PickFolder(Handle, "选择接收文件夹", DefaultReceiveFolder());
        if (folder is null) return;
        bool requested = await clipboard.ReceiveFileAsync(transferId, folder);
        Notify(requested
            ? "已请求文件内容；传输完成后会自动落盘并放入剪贴板。"
            : "接收请求未完成。");
    }

    private static string StateLabel(string state) => state switch
    {
        "offered" => "等待接收",
        "requested" => "已请求内容",
        "preparing" => "正在准备",
        "receiving" => "正在接收",
        "verifying" => "正在校验",
        "staged" => "校验通过，等待落地",
        "sending" => "正在发送",
        "completed" => "已完成",
        "failed" => "失败",
        "cancelled" => "已取消",
        _ => state,
    };

    internal static string DefaultReceiveFolder()
    {
        string profile = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);
        string downloads = Path.Combine(profile, "Downloads");
        return Path.Combine(Directory.Exists(downloads) ? downloads : profile, "DeskFuse");
    }

    private async Task LoadWorkflowsAsync()
    {
        if (!service.IsConnected || WorkflowList is null) return;
        try
        {
            var result = await service.SendAsync("GetWorkflows");
            if (!result.TryGetProperty("Workflows", out var catalog) || !catalog.TryGetProperty("workflows", out var values) || values.ValueKind != JsonValueKind.Array) return;
            var loaded = JsonSerializer.Deserialize<List<WorkflowRecord>>(values.GetRawText()) ?? [];
            workflows.Clear(); workflows.AddRange(loaded);
            InstallWorkflowHotkeys();
            WorkflowList.ItemsSource = null; WorkflowList.ItemsSource = workflows;
            if (editingWorkflow is not null) WorkflowList.SelectedItem = workflows.FirstOrDefault(w => w.Id == editingWorkflow.Id);
        }
        catch (Exception ex) { WorkflowNotice.Text = ex is IOException ? ex.Message : "工作流目录暂时不可用。"; }
    }

    private void InstallWorkflowHotkeys()
    {
        if (!loaded) return;
        RemoveHotkeys();
        WindowMessageHook.Attach(Handle, HotkeyWindowProc);
        int hotkeyId = 0x5540;
        foreach (var workflow in workflows.Where(w => w.Enabled && w.Shortcut is not null))
        {
            var shortcut = workflow.Shortcut!;
            uint modifiers = shortcut.Modifiers switch
            {
                var value when (value & 0x80000) != 0 => ModAlt,
                _ => 0,
            };
            if ((shortcut.Modifiers & 0x40000) != 0) modifiers |= ModControl;
            if ((shortcut.Modifiers & 0x20000) != 0) modifiers |= ModShift;
            if ((shortcut.Modifiers & 0x100000) != 0) modifiers |= ModWin;
            if (modifiers == 0 || !RegisterHotKey(Handle, hotkeyId, modifiers, shortcut.KeyCode)) continue;
            hotkeys[hotkeyId++] = workflow;
        }
    }

    private void RemoveHotkeys()
    {
        foreach (var id in hotkeys.Keys.ToArray()) UnregisterHotKey(Handle, id);
        hotkeys.Clear();
    }

    private bool HotkeyWindowProc(uint message, IntPtr wParam, IntPtr lParam)
    {
        if (message == WmHotKey && hotkeys.TryGetValue(wParam.ToInt32(), out var workflow))
        {
            _ = ExecuteWorkflowAsync(workflow);
            return true;
        }
        return false;
    }

    private async Task ExecuteWorkflowAsync(WorkflowRecord workflow)
    {
        if (!service.IsConnected || latest is not JsonElement snapshot || !snapshot.TryGetProperty("capabilities", out var caps) || !caps.Bool("workflow_execution"))
        {
            Notify("工作流执行能力尚未就绪；没有向服务发送剪贴板内容。");
            return;
        }
        try
        {
            long epoch = snapshot.Number("session_epoch");
            await workflowExecution.ExecuteAsync(workflow.Id, epoch, DateTime.UtcNow.Ticks, CancellationToken.None);
            Notify($"已执行工作流：{workflow.Name}");
        }
        catch (Exception ex) { Notify(ex is IOException ? ex.Message : "工作流未执行，剪贴板保持不变。"); }
    }

    private void NewWorkflowClick(object sender, RoutedEventArgs args)
    {
        editingWorkflow = new WorkflowRecord { Name = "新工作流", Id = Guid.NewGuid().ToString() };
        WorkflowList.SelectedItem = null;
        RenderWorkflowEditor();
    }

    private void WorkflowSelectionChanged(object sender, SelectionChangedEventArgs args)
    {
        editingWorkflow = WorkflowList.SelectedItem as WorkflowRecord;
        RenderWorkflowEditor();
    }

    private void RenderWorkflowEditor()
    {
        var workflow = editingWorkflow;
        bool enabled = workflow is not null;
        WorkflowName.IsEnabled = enabled; WorkflowEnabled.IsEnabled = enabled; WorkflowShortcut.IsEnabled = enabled;
        SaveWorkflowButton.IsEnabled = enabled;
        DeleteWorkflowButton.IsEnabled = workflow is not null && workflows.Contains(workflow);
        if (workflow is null) { WorkflowName.Text = ""; WorkflowEnabled.IsChecked = false; WorkflowShortcut.Text = ""; return; }
        WorkflowName.Text = workflow.Name; WorkflowEnabled.IsChecked = workflow.Enabled;
        WorkflowShortcut.Text = workflow.Shortcut is null ? "" : $"{workflow.Shortcut.KeyCode}, {workflow.Shortcut.Modifiers}";
        WorkflowNotice.Text = "";
    }

    private async void SaveWorkflowClick(object sender, RoutedEventArgs args)
    {
        if (editingWorkflow is null) return;
        if (string.IsNullOrWhiteSpace(WorkflowName.Text)) { WorkflowNotice.Text = "工作流名称不能为空。"; return; }
        try
        {
            ShortcutRecord? shortcut = null;
            if (!string.IsNullOrWhiteSpace(WorkflowShortcut.Text))
            {
                var values = WorkflowShortcut.Text.Split(',', StringSplitOptions.TrimEntries);
                if (values.Length != 2 || !ushort.TryParse(values[0], out var key) || !ulong.TryParse(values[1], out var modifiers)) throw new FormatException();
                shortcut = new ShortcutRecord { KeyCode = key, Modifiers = modifiers };
            }
            editingWorkflow.Name = WorkflowName.Text.Trim(); editingWorkflow.Enabled = WorkflowEnabled.IsChecked == true; editingWorkflow.Shortcut = shortcut;
            if (editingWorkflow.Steps.Count == 0) editingWorkflow.Steps = [WorkflowStepRecord.Copy()];
            await service.SendAsync(ServiceClient.Command("SaveWorkflow", new { workflow = editingWorkflow }));
            if (!workflows.Contains(editingWorkflow)) workflows.Add(editingWorkflow);
            InstallWorkflowHotkeys();
            WorkflowList.ItemsSource = null; WorkflowList.ItemsSource = workflows;
            WorkflowList.SelectedItem = editingWorkflow;
            WorkflowNotice.Text = "已提交；服务端 Rust 模型校验并持久化。";
        }
        catch (FormatException) { WorkflowNotice.Text = "快捷键格式无效，请使用 key code, modifiers。"; }
        catch (Exception ex) { WorkflowNotice.Text = ex is IOException ? ex.Message : "工作流保存失败。"; }
    }

    private async void DeleteWorkflowClick(object sender, RoutedEventArgs args)
    {
        if (editingWorkflow is null || !workflows.Contains(editingWorkflow)) return;
        try
        {
            await service.SendAsync(ServiceClient.Command("DeleteWorkflow", new { id = editingWorkflow.Id }));
            workflows.Remove(editingWorkflow); editingWorkflow = null; InstallWorkflowHotkeys(); WorkflowList.ItemsSource = null; WorkflowList.ItemsSource = workflows; RenderWorkflowEditor();
        }
        catch (Exception ex) { WorkflowNotice.Text = ex is IOException ? ex.Message : "工作流删除失败。"; }
    }

    private void DeviceSearchChanged(object sender, TextChangedEventArgs args) => RebuildDevices();

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
        RenderDeviceDetails();
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
            DeviceDetails.Children.Add(Label("正在连接 DeskFuse 后台组件，程序会自动启动并重连，无需另外打开服务。具体状态见窗口底部。", 15));
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
                var actions = new StackPanel { Orientation = Orientation.Horizontal };
                actions.Children.Add(ActionButton("接受配对", "AcceptPairing", new { request_id = id }));
                actions.Children.Add(ActionButton("拒绝", "RejectPairing", new { request_id = id }));
                invitation.Children.Add(actions);
            }
            DeviceDetails.Children.Add(new Border { Background = new SolidColorBrush(Microsoft.UI.ColorHelper.FromArgb(255, 232, 243, 255)), Padding = new Thickness(16), CornerRadius = new CornerRadius(8), Child = invitation, Margin = new Thickness(0, 14, 0, 10) });
        }
        string pairing = latest.Value.String("pairing_status");
        if (pairing.Length > 0) DeviceDetails.Children.Add(Label(pairing, 13, false, true));
        if (device.Local)
        {
            DeviceDetails.Children.Add(Label($"设备 ID：{latest.Value.String("local_device_id")}", 13));
            DeviceDetails.Children.Add(Label($"服务版本：{latest.Value.String("package_version")}\n控制端口：{latest.Value.Number("control_port")}", 14));
            DeviceDetails.Children.Add(Label($"本机名称：{latest.Value.String("local_name")}", 17, true));
            DeviceDetails.Children.Add(Label("本机名称与 Windows 计算机名一致；修改计算机名后重启服务即会更新。", 13, false, true));
            return;
        }
        var peer = device.Peer;
        DeviceDetails.Children.Add(Label($"设备 ID：{device.Id}", 13));
        DeviceDetails.Children.Add(Label("连接候选", 18, true));
        bool connected = peer.String("connection") == "connected";
        string active = "";
        if (connected && peer.TryGetProperty("selected_route", out var selectedRoute) && selectedRoute.ValueKind == JsonValueKind.String)
        {
            var route = peer.Array("routes").FirstOrDefault(r => r.String("id") == selectedRoute.GetString());
            if (route.ValueKind == JsonValueKind.Object && route.TryGetProperty("endpoint", out var endpoint)) active = EndpointLabel(endpoint);
        }
        DeviceDetails.Children.Add(Label(active.Length > 0 ? $"当前活动路径：{active}" : "当前没有已确认的活动地址", 13, false, true));
        var routes = peer.Array("routes").ToArray();
        if (routes.Length == 0) DeviceDetails.Children.Add(Label("还没有连接候选。等待自动发现，或在左下角通过 IPv4 定位。", 14));
        foreach (var route in routes)
        {
            string endpoint = route.TryGetProperty("endpoint", out var endpointValue) ? EndpointLabel(endpointValue) : "地址未知";
            string routeSource = route.TryGetProperty("source", out var sourceValue) ? JsonLabel(sourceValue) : "来源未知";
            DeviceDetails.Children.Add(Label($"{endpoint}\n来源：{routeSource} · 健康：{route.String("health", "unknown")} · 接口：{route.String("path_kind", "未识别")}", 13));
        }
        DeviceDetails.Children.Add(Label("候选地址不代表已连通；不会根据 IP 前缀猜测 Wi-Fi 或雷电。", 12, false, true));
        var routePreference = new ComboBox { IsEnabled = false, ItemsSource = new[] { "自动路径（服务管理）" }, SelectedIndex = 0, Margin = new Thickness(0, 12, 0, 16) };
        ToolTipService.SetToolTip(routePreference, "当前桌面 IPC 尚未开放路径偏好命令。");
        DeviceDetails.Children.Add(routePreference);
        var buttons = new StackPanel { Orientation = Orientation.Horizontal };
        if (peer.String("trust") == "trusted" && latest.Value.TryGetProperty("capabilities", out var localCaps)
            && localCaps.Bool("remote_app"))
        {
            var remoteAppButton = new Button { Content = "管理 Windows RemoteApp", IsEnabled = service.IsConnected && !busy, Margin = new Thickness(0, 0, 8, 0) };
            remoteAppButton.Click += (_, _) => ShowRemoteApps();
            buttons.Children.Add(remoteAppButton);
        }
        if (peer.String("trust") != "trusted") buttons.Children.Add(ActionButton("邀请配对", "RequestPairing", new { peer_id = device.Id }));
        if (connected) buttons.Children.Add(ActionButton("断开连接", "Disconnect", new { peer_id = device.Id }));
        if (peer.String("trust") == "trusted") buttons.Children.Add(ActionButton("撤销信任", "Revoke", new { peer_id = device.Id }));
        DeviceDetails.Children.Add(buttons);

        // 键鼠主控：仅对已信任设备开放，服务会持久化角色与参与者。
        var input = latest.Value.TryGetProperty("input", out var inputState) ? inputState : default;
        var settings = input.TryGetProperty("settings", out var sharing) ? sharing : default;
        bool inputEnabled = settings.ValueKind == JsonValueKind.Object && settings.Bool("enabled");
        bool isController = settings.ValueKind == JsonValueKind.Object && settings.String("role") == "Controller";
        bool isParticipant = settings.ValueKind == JsonValueKind.Object
            && settings.Array("participants").Any(p => p.ValueKind == JsonValueKind.String && p.GetString() == device.Id);
        if (peer.String("trust") == "trusted")
        {
            DeviceDetails.Children.Add(Label("键鼠共享", 18, true));
            var inputButtons = new StackPanel { Orientation = Orientation.Horizontal, Margin = new Thickness(0, 14, 0, 0) };
            if (inputEnabled && isController && isParticipant)
            {
                inputButtons.Children.Add(ActionButton("停止主控此设备", "DisableInputControl", new { }));
                var activation = input.Array("activations").FirstOrDefault(a => a.String("peer_id") == device.Id);
                string status = activation.ValueKind == JsonValueKind.Object ? activation.String("status") : "";
                string reason = activation.ValueKind == JsonValueKind.Object ? activation.String("reason") : "";
                DeviceDetails.Children.Add(Label(
                    status.Length == 0
                        ? "正在等待对端确认设备激活…把本机指针移到屏幕边缘即可开始主控。"
                        : $"激活状态：{InputStatusLabel(status)}{(reason.Length > 0 ? $"\n{reason}" : "")}",
                    13, false, true));
            }
            else
            {
                inputButtons.Children.Add(ActionButton("主控此设备", "EnableInputControl", new { peer_id = device.Id }));
                DeviceDetails.Children.Add(Label("开启后本机指针移到屏幕边缘即可控制对端；对端仍需在设备激活中确认。", 13, false, true));
            }
            DeviceDetails.Children.Add(inputButtons);
        }
    }

    private static string InputStatusLabel(string status) => status switch
    {
        "Preparing" => "准备中",
        "Ready" => "对端已就绪",
        "PausedByUser" => "对端已暂停",
        "Unavailable" => "不可用",
        _ => status,
    };

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
            Notify(reply.TryGetProperty("Queued", out _) ? "操作已提交；等待服务和对端的真实状态更新。" : "服务已响应；以设备列表中的实际状态为准。");
        }
        catch (Exception ex) { Notify(ex is IOException ? ex.Message : "操作未完成，服务可能已断开或请求超时。"); }
        finally { busy = false; PairIpButton.IsEnabled = service.IsConnected; }
    }

    private async void PairIpClick(object sender, RoutedEventArgs args)
    {
        string ip = ManualIp.Text.Trim();
        if (ip.Split('.').Length != 4 || !IPAddress.TryParse(ip, out var address) || address.AddressFamily != AddressFamily.InterNetwork || address.ToString() != ip)
        { Notify("请输入完整 IPv4 地址，例如 192.168.1.23。IP 仅用于定位，不授予信任。"); return; }
        await RunCommand("RequestPairingAt", new { ipv4 = ip });
    }

    private void DisplayCanvasSizeChanged(object sender, SizeChangedEventArgs args)
    {
        // WinUI 3 has no ClipToBounds: clip the canvas geometry to its own size.
        DisplayCanvas.Clip = new RectangleGeometry { Rect = new global::Windows.Foundation.Rect(0, 0, args.NewSize.Width, args.NewSize.Height) };
        DrawInputLayout(latest.HasValue && latest.Value.TryGetProperty("input", out var layoutInput) && layoutInput.ValueKind == JsonValueKind.Object ? layoutInput : default);
    }

    private static TextBlock Label(string text, double size = 14, bool bold = false, bool muted = false) => new()
    {
        Text = text,
        FontSize = size,
        FontWeight = bold ? Microsoft.UI.Text.FontWeights.SemiBold : Microsoft.UI.Text.FontWeights.Normal,
        Foreground = new SolidColorBrush(muted ? Microsoft.UI.ColorHelper.FromArgb(255, 101, 118, 140) : Microsoft.UI.ColorHelper.FromArgb(255, 24, 36, 60)),
        Margin = new Thickness(0, 8, 0, 10),
        TextWrapping = TextWrapping.Wrap,
    };

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

    internal void Notify(string message) => Notice.Text = message;

    private void FileOfferArrived(string peer, string names) =>
        Notify($"{peer} 发来文件：{names}。在“传输”页选择位置接收。");

    private void FileTransferDone(string id, string detail, bool success) =>
        Notify(success ? detail : $"文件传输未完成：{detail}");

    private void SettingsClick(object sender, RoutedEventArgs args) => ShowSettings();

    public void ShowSettings()
    {
        if (settings is null) settings = new SettingsWindow(clipboard, service);
        settings.Show();
    }

    private void KeyboardShortcut(object sender, KeyRoutedEventArgs args)
    {
        bool control = IsDown(global::Windows.System.VirtualKey.Control);
        if (control && args.Key == global::Windows.System.VirtualKey.F)
        {
            if (Navigation.SelectedIndex == 2) DeviceSearch.Focus(FocusState.Programmatic);
            else { Navigation.SelectedIndex = 0; HistorySearch.Focus(FocusState.Programmatic); }
            args.Handled = true;
        }
        else if (control && args.Key == (global::Windows.System.VirtualKey)0xBC /* OEM_COMMA */) { ShowSettings(); args.Handled = true; }
        else if (args.Key == global::Windows.System.VirtualKey.Enter && ReferenceEquals(FocusManager.GetFocusedElement(Root.XamlRoot), HistoryList)) { CopySelection(); args.Handled = true; }
        else if (args.Key is global::Windows.System.VirtualKey.Delete or global::Windows.System.VirtualKey.Back && ReferenceEquals(FocusManager.GetFocusedElement(Root.XamlRoot), HistoryList))
        {
            if (HistoryList.SelectedItem is ClipboardEntry entry) { clipboard.Remove(entry); Notify("已删除这条本机历史；系统剪贴板、用户源文件和已粘贴文件保持不变。"); }
            args.Handled = true;
        }
    }

    private static bool IsDown(global::Windows.System.VirtualKey key) =>
        (global::Microsoft.UI.Input.InputKeyboardSource.GetKeyStateForCurrentThread(key) & global::Windows.UI.Core.CoreVirtualKeyStates.Down) != 0;

    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool RegisterHotKey(IntPtr hWnd, int id, uint modifiers, uint virtualKey);
    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool UnregisterHotKey(IntPtr hWnd, int id);

    private sealed record DeviceRow(string Id, string Name, string Group, string Detail, bool Local, JsonElement Peer);
}
