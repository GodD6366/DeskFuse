using System.Runtime.InteropServices;
using System.Text.Json;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Windows.ApplicationModel.DataTransfer;
using Windows.Storage;
using Windows.Storage.FileProperties;
using Colors = Microsoft.UI.Colors;

namespace DeskFuse.Windows;

/// File-only floating shelf, matching ShelfController.swift. Removing a reference
/// never deletes its source. Drag/drop uses WinRT deferrals, never blocks the UI.
public sealed class ShelfWindow : IShelfHost
{
    private readonly DesktopClipboardController controller;
    private readonly ServiceClient service;
    private readonly Action<string> report;
    private readonly Window window = new();
    private readonly List<string> files = [];
    private readonly Grid root;
    private readonly Grid body = new();
    private readonly TextBlock status = DfUi.Detail("");
    private readonly Button send;
    private readonly Button expand;
    private readonly SemaphoreSlim thumbnailSlots = new(4, 4);
    private CancellationTokenSource visuals = new();
    private bool exiting;
    private bool expanded;
    private bool preparing;
    private bool draggingFromShelf;
    private bool droppedBackOnShelf;
    private string? transferId;
    private int operation;
    private double dpiScale = 1;

    public ShelfWindow(DesktopClipboardController controller, ServiceClient service, Action<string> report)
    {
        this.controller = controller;
        this.service = service;
        this.report = report;
        window.Title = "DeskFuse 暂存区";
        AppIcon.Apply(window);
        window.ExtendsContentIntoTitleBar = true;
        if (window.AppWindow.Presenter is OverlappedPresenter presenter)
        {
            presenter.SetBorderAndTitleBar(false, false);
            presenter.IsAlwaysOnTop = true;
            presenter.IsResizable = false;
            presenter.IsMaximizable = false;
            presenter.IsMinimizable = false;
        }
        try { window.SystemBackdrop = new DesktopAcrylicBackdrop(); } catch (Exception) { }
        root = new Grid
        {
            Background = DfUi.Surface,
            BorderBrush = DfUi.Line,
            BorderThickness = new Thickness(1),
            CornerRadius = new CornerRadius(20),
            AllowDrop = true,
        };
        root.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        root.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        root.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        var header = new Grid { Margin = new Thickness(12, 12, 12, 4), ColumnSpacing = 8, Height = 28 };
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var close = DfUi.IconButton("\uE711", async (_, _) => await ClearAndCloseAsync(), "清空并关闭", 28, 12);
        header.Children.Add(close);
        var dragHandle = new Grid { Background = new SolidColorBrush(Colors.Transparent) };
        Grid.SetColumn(dragHandle, 1); header.Children.Add(dragHandle);
        send = DfUi.IconButton("\uE724", (_, _) => ShowSendMenu(), "发送到设备", 28, 12);
        Grid.SetColumn(send, 2); header.Children.Add(send);
        expand = DfUi.IconButton("\uE740", (_, _) => ToggleExpanded(), "展开暂存区", 28, 12);
        Grid.SetColumn(expand, 3); header.Children.Add(expand);
        root.Children.Add(header);
        Grid.SetRow(body, 1); root.Children.Add(body);
        status.TextWrapping = TextWrapping.Wrap;
        status.Margin = new Thickness(14, 0, 14, 10);
        status.Visibility = Visibility.Collapsed;
        Grid.SetRow(status, 2); root.Children.Add(status);
        root.DragOver += (_, args) =>
        {
            args.AcceptedOperation = !preparing && args.DataView.Contains(StandardDataFormats.StorageItems)
                ? DataPackageOperation.Copy : DataPackageOperation.None;
            args.Handled = true;
        };
        root.Drop += OnDrop;
        window.Content = root;
        window.SetTitleBar(dragHandle);
        window.AppWindow.Closing += async (_, args) =>
        {
            if (exiting) return;
            args.Cancel = true;
            await ClearAndCloseAsync();
        };
        service.SnapshotChanged += ObserveTransfer;
        service.ConnectionChanged += ConnectionChanged;
        Resize();
        Render();
    }

    public void Add(ClipboardEntry entry)
    {
        if (entry.Content.Files is not { Count: > 0 } paths)
        {
            ShowStatus("暂存区只接收文件或文件夹；文字和图片请使用剪贴板面板。");
            return;
        }
        AddPaths(paths);
    }

    private void AddPaths(IEnumerable<string> paths)
    {
        if (preparing) return;
        try
        {
            var combined = files.Concat(paths.Select(Path.GetFullPath)).Distinct(StringComparer.OrdinalIgnoreCase).ToArray();
            NativeClipboard.ValidateLocalFiles(combined);
            if (combined.SequenceEqual(files, StringComparer.OrdinalIgnoreCase)) return;
            files.Clear(); files.AddRange(combined);
            status.Visibility = Visibility.Collapsed;
            Render();
        }
        catch (Exception ex) { ShowStatus($"无法暂存文件：{ex.Message}"); }
    }

    private void Render()
    {
        visuals.Cancel(); visuals.Dispose(); visuals = new();
        body.Children.Clear();
        send.IsEnabled = files.Count > 0 && !preparing;
        ToolTipService.SetToolTip(expand, expanded ? "收起暂存区" : "展开暂存区");
        if (expand.Content is FontIcon glyph) glyph.Glyph = expanded ? "\uE73F" : "\uE740";
        if (preparing)
        {
            var progress = new StackPanel { Spacing = 12, VerticalAlignment = VerticalAlignment.Center, HorizontalAlignment = HorizontalAlignment.Center };
            progress.Children.Add(new ProgressRing { IsActive = true, Width = 28, Height = 28 });
            progress.Children.Add(DfUi.Detail(transferId is null ? "正在准备发送…" : "等待对方接收…"));
            body.Children.Add(progress);
            return;
        }
        if (files.Count == 0)
        {
            var empty = new StackPanel { Spacing = 10, VerticalAlignment = VerticalAlignment.Center, HorizontalAlignment = HorizontalAlignment.Center, Margin = new Thickness(14) };
            empty.Children.Add(DfUi.Glyph("\uE896", 40, DfUi.Muted));
            empty.Children.Add(DfUi.Text("拖入文件", 16, true));
            empty.Children.Add(DfUi.Detail("文件会暂存在这里，拖出即可取用", 11));
            body.Children.Add(empty);
        }
        else if (expanded)
        {
            var grid = new Grid { Margin = new Thickness(14), ColumnSpacing = 12, RowSpacing = 16 };
            for (int i = 0; i < 4; i++) grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            int index = 0;
            foreach (var path in files.AsEnumerable().Reverse())
            {
                if (index % 4 == 0) grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
                var tile = new StackPanel { Spacing = 5 };
                var icon = FileIcon(path, 60);
                AttachDrag(icon, [path]);
                tile.Children.Add(icon);
                tile.Children.Add(new TextBlock { Text = Path.GetFileName(path.TrimEnd(Path.DirectorySeparatorChar)), FontSize = 11, MaxLines = 2, TextWrapping = TextWrapping.Wrap, TextAlignment = TextAlignment.Center });
                var remove = DfUi.IconButton("\uE711", (_, _) => RemovePaths([path]), "移出暂存区", 22, 10);
                remove.HorizontalAlignment = HorizontalAlignment.Center;
                tile.Children.Add(remove);
                Grid.SetColumn(tile, index % 4); Grid.SetRow(tile, index / 4); grid.Children.Add(tile); index++;
            }
            body.Children.Add(new ScrollViewer { Content = grid, VerticalScrollBarVisibility = ScrollBarVisibility.Auto, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled });
        }
        else
        {
            var compact = new Grid();
            compact.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
            compact.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
            var stack = new Grid { Width = 150, Height = 126, VerticalAlignment = VerticalAlignment.Center, HorizontalAlignment = HorizontalAlignment.Center, Background = new SolidColorBrush(Colors.Transparent) };
            var top = files.TakeLast(3).ToArray();
            for (int i = 0; i < top.Length; i++)
            {
                var icon = FileIcon(top[i], 100);
                icon.RenderTransformOrigin = new global::Windows.Foundation.Point(.5, .5);
                icon.RenderTransform = new CompositeTransform { Rotation = (i - top.Length + 1) * -8, TranslateX = (i - top.Length + 1) * -7 };
                stack.Children.Add(icon);
            }
            AttachDrag(stack, files.ToArray());
            stack.Tapped += (_, _) => ToggleExpanded();
            compact.Children.Add(stack);
            var footer = new Grid { Margin = new Thickness(14, 0, 14, 12), ColumnSpacing = 8 };
            footer.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            footer.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            var summary = new StackPanel { Spacing = 3 };
            summary.Children.Add(DfUi.Text($"{files.Count} 个项目", 16, true));
            summary.Children.Add(DfUi.Detail("拖出全部文件", 11));
            footer.Children.Add(summary);
            var all = DfUi.Secondary("查看全部", (_, _) => ToggleExpanded());
            all.Padding = new Thickness(8, 4, 8, 4); all.MinHeight = 28;
            Grid.SetColumn(all, 1); footer.Children.Add(all);
            Grid.SetRow(footer, 1); compact.Children.Add(footer); body.Children.Add(compact);
        }
    }

    private Grid FileIcon(string path, int size)
    {
        var host = new Grid { Width = size, Height = size, HorizontalAlignment = HorizontalAlignment.Center, Background = new SolidColorBrush(Colors.Transparent) };
        host.Children.Add(DfUi.Glyph(Directory.Exists(path) ? "\uE8B7" : "\uE7C3", size * .65, DfUi.Muted));
        ToolTipService.SetToolTip(host, Path.GetFileName(path));
        _ = LoadIconAsync(host, path, size, visuals.Token);
        return host;
    }

    private async Task LoadIconAsync(Grid host, string path, int size, CancellationToken token)
    {
        bool entered = false;
        try
        {
            await thumbnailSlots.WaitAsync(token); entered = true;
            using StorageItemThumbnail? thumbnail = Directory.Exists(path)
                ? await (await StorageFolder.GetFolderFromPathAsync(path).AsTask(token)).GetThumbnailAsync(ThumbnailMode.SingleItem, (uint)size).AsTask(token)
                : await (await StorageFile.GetFileFromPathAsync(path).AsTask(token)).GetThumbnailAsync(ThumbnailMode.SingleItem, (uint)size).AsTask(token);
            if (thumbnail is null || token.IsCancellationRequested) return;
            var image = new BitmapImage();
            await image.SetSourceAsync(thumbnail).AsTask(token);
            if (token.IsCancellationRequested) return;
            host.Children.Clear(); host.Children.Add(new Image { Source = image, Stretch = Stretch.Uniform });
        }
        catch (Exception) { /* Generic file/folder icon remains available. */ }
        finally { if (entered) thumbnailSlots.Release(); }
    }

    private async void OnDrop(object sender, DragEventArgs args)
    {
        if (preparing || !args.DataView.Contains(StandardDataFormats.StorageItems)) return;
        var deferral = args.GetDeferral();
        try
        {
            var items = await args.DataView.GetStorageItemsAsync();
            if (draggingFromShelf) droppedBackOnShelf = true;
            AddPaths(items.Select(item => item.Path).Where(path => path.Length > 0));
            args.AcceptedOperation = DataPackageOperation.Copy;
        }
        catch (Exception ex) { ShowStatus($"拖入未完成：{ex.Message}"); }
        finally { args.Handled = true; deferral.Complete(); }
    }

    private void AttachDrag(UIElement target, string[] paths)
    {
        target.CanDrag = true;
        target.DragStarting += async (_, args) =>
        {
            if (preparing) { args.Cancel = true; return; }
            var deferral = args.GetDeferral();
            draggingFromShelf = true;
            droppedBackOnShelf = false;
            try
            {
                NativeClipboard.ValidateLocalFiles(paths);
                var items = new List<IStorageItem>();
                foreach (var path in paths)
                    items.Add(Directory.Exists(path) ? await StorageFolder.GetFolderFromPathAsync(path) : await StorageFile.GetFileFromPathAsync(path));
                args.Data.RequestedOperation = DataPackageOperation.Copy;
                args.Data.SetStorageItems(items, true);
            }
            catch (Exception ex) { args.Cancel = true; ShowStatus($"拖出未完成：{ex.Message}"); }
            finally { deferral.Complete(); }
        };
        target.DropCompleted += (_, args) =>
        {
            if (args.DropResult != DataPackageOperation.None && !droppedBackOnShelf) RemovePaths(paths);
            draggingFromShelf = false;
            droppedBackOnShelf = false;
        };
    }

    private void RemovePaths(IEnumerable<string> paths)
    {
        if (preparing) return;
        var removed = paths.ToHashSet(StringComparer.OrdinalIgnoreCase);
        files.RemoveAll(removed.Contains);
        Render();
    }

    private void ShowSendMenu()
    {
        if (preparing || files.Count == 0) return;
        var menu = new MenuFlyout();
        var peers = service.LatestSnapshot?.Array("peers").Where(peer => peer.String("trust") == "trusted" && peer.String("connection") == "connected").ToArray() ?? [];
        if (peers.Length == 0) menu.Items.Add(new MenuFlyoutItem { Text = "没有可发送的在线设备", IsEnabled = false });
        foreach (var peer in peers)
        {
            string id = peer.String("peer_id");
            var item = new MenuFlyoutItem { Text = peer.String("display_name", "未命名设备") };
            item.Click += async (_, _) => await SendAsync(id);
            menu.Items.Add(item);
        }
        menu.ShowAt(send);
    }

    private async Task SendAsync(string peer)
    {
        if (preparing || files.Count == 0) return;
        int request = ++operation;
        preparing = true; transferId = null; status.Visibility = Visibility.Collapsed; Render();
        try
        {
            string id = await controller.BeginFileTransferAsync(files.ToArray(), peer);
            if (request != operation) { await controller.CancelFileAsync(id); return; }
            transferId = id;
            Render();
            if (service.LatestSnapshot is JsonElement snapshot) ObserveTransfer(snapshot);
        }
        catch (Exception ex)
        {
            if (request != operation) { report(ex.Message); return; }
            preparing = false; transferId = null; Render(); ShowStatus(ex.Message);
        }
    }

    private void ObserveTransfer(JsonElement snapshot)
    {
        if (!preparing || transferId is null) return;
        var row = snapshot.Array("file_transfers").FirstOrDefault(t => t.String("transfer_id") == transferId);
        switch (row.String("state"))
        {
            case "sending": case "completed":
                preparing = false; transferId = null; files.Clear(); Render(); Hide(); break;
            case "failed": case "cancelled":
                preparing = false; transferId = null; Render(); ShowStatus(row.String("detail", "发送未完成；文件保留在暂存区。")); break;
        }
    }

    private void ConnectionChanged(bool connected, string message)
    {
        if (connected || !preparing) return;
        ++operation; preparing = false; transferId = null; Render(); ShowStatus("连接已断开；文件保留在暂存区，请重试。");
    }

    private async Task ClearAndCloseAsync()
    {
        ++operation;
        string? pending = transferId;
        transferId = null; preparing = false; files.Clear(); Render(); Hide();
        if (pending is not null)
        {
            try { await controller.CancelFileAsync(pending); }
            catch (Exception ex) { report($"取消发送未完成：{ex.Message}"); }
        }
    }

    private void ShowStatus(string message) { status.Text = message; status.Visibility = Visibility.Visible; }
    private void ToggleExpanded() { expanded = !expanded; Resize(); Render(); }
    private void Resize()
    {
        window.AppWindow.Resize(new global::Windows.Graphics.SizeInt32((int)((expanded ? 480 : 240) * dpiScale), (int)((expanded ? 420 : 240) * dpiScale)));
        Constrain();
    }
    private void Constrain()
    {
        var area = DisplayArea.GetFromWindowId(window.AppWindow.Id, DisplayAreaFallback.Nearest).WorkArea;
        var size = window.AppWindow.Size; var position = window.AppWindow.Position;
        window.AppWindow.Move(new global::Windows.Graphics.PointInt32(Math.Clamp(position.X, area.X + 12, Math.Max(area.X + 12, area.X + area.Width - size.Width - 12)), Math.Clamp(position.Y, area.Y + 12, Math.Max(area.Y + 12, area.Y + area.Height - size.Height - 12))));
    }
    public bool IsVisible => window.AppWindow.IsVisible;
    public void Show(bool nearPointer = false)
    {
        if (nearPointer || !IsVisible) MoveNearPointer();
        window.AppWindow.Show(false);
    }
    private void MoveNearPointer()
    {
        GetCursorPos(out var point);
        var area = DisplayArea.GetFromPoint(point, DisplayAreaFallback.Nearest).WorkArea;
        var monitor = MonitorFromPoint(point, 2);
        if (GetDpiForMonitor(monitor, 0, out uint dpi, out _) == 0 && dpi > 0) dpiScale = dpi / 96.0;
        Resize();
        var size = window.AppWindow.Size;
        int gap = (int)(32 * dpiScale);
        int x = point.X + gap + size.Width <= area.X + area.Width - 12 ? point.X + gap : point.X - gap - size.Width;
        int y = point.Y - gap - size.Height >= area.Y + 12 ? point.Y - gap - size.Height : point.Y + gap;
        window.AppWindow.Move(new global::Windows.Graphics.PointInt32(x, y));
        Constrain();
    }
    public void Hide() => window.AppWindow.Hide();
    public void Shutdown()
    {
        exiting = true; ++operation;
        service.SnapshotChanged -= ObserveTransfer; service.ConnectionChanged -= ConnectionChanged;
        visuals.Cancel(); visuals.Dispose(); window.Close();
    }
    [DllImport("user32.dll")] private static extern bool GetCursorPos(out global::Windows.Graphics.PointInt32 point);
    [DllImport("user32.dll")] private static extern IntPtr MonitorFromPoint(global::Windows.Graphics.PointInt32 point, uint flags);
    [DllImport("shcore.dll")] private static extern int GetDpiForMonitor(IntPtr monitor, int kind, out uint x, out uint y);
}
