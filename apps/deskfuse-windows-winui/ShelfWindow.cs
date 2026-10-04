using System.Collections.ObjectModel;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Windows.ApplicationModel.DataTransfer;
using Windows.Storage;
using Windows.Storage.Streams;

namespace DeskFuse.Windows;

/// <summary>Local temporary shelf. It holds references; removing items never removes user files.</summary>
public sealed class ShelfWindow : IShelfHost
{
    private readonly NativeClipboard clipboard;
    private readonly ClipboardHistory history;
    private readonly DesktopClipboardController controller;
    private readonly Window window = new();
    private readonly ObservableCollection<ClipboardEntry> entries = [];
    private readonly ListBox list = new() { BorderThickness = new Thickness(0) };
    private readonly TextBlock status = new() { Text = "拖入本机文件，或从历史加入", TextWrapping = TextWrapping.Wrap, Foreground = new SolidColorBrush(Microsoft.UI.ColorHelper.FromArgb(255, 112, 128, 144)), FontSize = 11 };
    private readonly Button expand = new() { Content = "展开", Padding = new Thickness(5, 2, 5, 2) };
    private bool exiting;
    private bool expanded;

    public ShelfWindow(NativeClipboard clipboard, ClipboardHistory history, DesktopClipboardController controller)
    {
        this.clipboard = clipboard;
        this.history = history;
        this.controller = controller;
        AppIcon.Apply(window);
        window.AppWindow.Title = "DeskFuse 暂存区";
        window.AppWindow.Resize(new global::Windows.Graphics.SizeInt32(240, 240));
        window.AppWindow.Closing += (_, args) => { if (!exiting) { args.Cancel = true; window.AppWindow.Hide(); } };

        list.ItemsSource = entries;
        list.DisplayMemberPath = nameof(ClipboardEntry.Title);
        list.SelectionMode = SelectionMode.Single;
        list.CanDrag = true;
        list.AllowDrop = true;
        list.DoubleTapped += (_, _) => CopySelection();
        list.KeyDown += (_, args) => { if (args.Key == global::Windows.System.VirtualKey.Enter) { CopySelection(); args.Handled = true; } };
        list.DragStarting += DragOut;
        list.DragOver += (_, args) => { args.AcceptedOperation = args.DataView.Contains(StandardDataFormats.StorageItems) ? DataPackageOperation.Copy : DataPackageOperation.None; args.Handled = true; };
        list.Drop += OnDrop;

        var copy = new MenuFlyoutItem { Text = "复制" };
        copy.Click += (_, _) => CopySelection();
        var send = new MenuFlyoutItem { Text = "发送到设备" };
        send.Click += async (_, _) => await SendSelectionAsync();
        var remove = new MenuFlyoutItem { Text = "移出暂存区" };
        remove.Click += (_, _) => { if (list.SelectedItem is ClipboardEntry selected) entries.Remove(selected); UpdateStatus(); };
        var menu = new MenuFlyout();
        menu.Items.Add(copy); menu.Items.Add(send); menu.Items.Add(remove);
        list.ContextFlyout = menu;

        expand.Click += (_, _) =>
        {
            expanded = !expanded;
            window.AppWindow.Resize(expanded ? new global::Windows.Graphics.SizeInt32(480, 420) : new global::Windows.Graphics.SizeInt32(240, 240));
            expand.Content = expanded ? "收起" : "展开";
        };
        var header = new StackPanel { Orientation = Orientation.Horizontal, Margin = new Thickness(0, 0, 0, 8) };
        header.Children.Add(new TextBlock { Text = "暂存区", FontWeight = Microsoft.UI.Text.FontWeights.SemiBold, FontSize = 16, VerticalAlignment = VerticalAlignment.Center });
        header.Children.Add(expand);
        var layout = new Grid { Margin = new Thickness(10) };
        layout.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        layout.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        layout.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        Grid.SetRow(header, 0); Grid.SetRow(list, 1); Grid.SetRow(status, 2);
        layout.Children.Add(header); layout.Children.Add(list); layout.Children.Add(status);
        window.Content = layout;
    }

    public void Add(ClipboardEntry entry)
    {
        if (entries.Any(existing => existing.Id == entry.Id)) return;
        if (entries.Count >= 100) { status.Text = "暂存区已满（100 项），请先移出部分项目"; return; }
        entries.Insert(0, entry);
        list.SelectedItem = entry;
        UpdateStatus();
    }

    private void UpdateStatus() => status.Text = $"{entries.Count} 项 · 移出不会删除原文件";

    private void OnDrop(object sender, DragEventArgs args)
    {
        try
        {
            if (!args.DataView.Contains(StandardDataFormats.StorageItems)) return;
            var items = args.DataView.GetStorageItemsAsync().AsTask().GetAwaiter().GetResult();
            var paths = items.Select(item => item.Path).Where(path => !string.IsNullOrWhiteSpace(path)).ToArray();
            NativeClipboard.ValidateLocalFiles(paths);
            // Staging must not silently replace the system clipboard or transmit file bodies.
            var content = new ClipboardContent(ClipboardContent.LocalFilesMime, [], paths);
            Add(history.Add(content, "local", "本机"));
            args.AcceptedOperation = DataPackageOperation.Copy;
        }
        catch (Exception ex) when (ex is IOException or InvalidOperationException or UnauthorizedAccessException or AggregateException) { status.Text = "无法暂存：文件不可用或数量超过限制"; }
        args.Handled = true;
    }

    private void DragOut(UIElement sender, DragStartingEventArgs args)
    {
        if (list.SelectedItem is not ClipboardEntry entry) { args.Cancel = true; return; }
        try
        {
            if (entry.Content.Files is { Count: > 0 } paths)
            {
                NativeClipboard.ValidateLocalFiles(paths);
                // Copy only: Explorer must never move/delete the source as a shelf side effect.
                args.Data.RequestedOperation = DataPackageOperation.Copy;
                var items = paths.Select(path => StorageFile.GetFileFromPathAsync(path).AsTask().GetAwaiter().GetResult()).Cast<IStorageItem>().ToList();
                args.Data.SetStorageItems(items, false);
            }
            else if (entry.Content.Mime == "text/plain")
            {
                args.Data.RequestedOperation = DataPackageOperation.Copy;
                args.Data.SetText(System.Text.Encoding.UTF8.GetString(entry.Content.Data));
            }
            else
            {
                // Image drag-out needs a bitmap provider; keep the safe path only.
                args.Cancel = true;
                status.Text = "此格式请使用复制";
            }
        }
        catch (Exception ex) when (ex is IOException or InvalidOperationException or UnauthorizedAccessException or AggregateException or ArgumentException)
        {
            args.Cancel = true;
            status.Text = "拖出失败：请确认原文件仍可用";
        }
    }

    private void CopySelection()
    {
        if (list.SelectedItem is not ClipboardEntry entry) return;
        try { clipboard.Write(entry.Content); status.Text = "已复制到本机剪贴板"; }
        catch (Exception ex) when (ex is IOException or InvalidOperationException or System.Runtime.InteropServices.COMException or NotSupportedException or ArgumentException)
        { status.Text = "复制失败，原文件保持不变"; }
    }

    private async Task SendSelectionAsync()
    {
        if (list.SelectedItem is not ClipboardEntry entry) return;
        if (entry.Content.Files is not { Count: > 0 } paths)
        { status.Text = "只有本机文件或文件夹可以发送到设备"; return; }
        status.Text = "正在发送到已连接设备…";
        bool sent = await controller.PublishFilesAsync(paths);
        status.Text = sent
            ? "已发送文件引用；对方粘贴时按需传输并校验。"
            : "文件未发送；请查看主窗口提示";
    }

    public void Show() { window.AppWindow.Show(); window.Activate(); }
    public void Hide() { if (window.AppWindow.Presenter is OverlappedPresenter presenter && presenter.State == OverlappedPresenterState.Minimized) presenter.Restore(); window.AppWindow.Hide(); }

    public void Shutdown() { exiting = true; window.Close(); }
}
