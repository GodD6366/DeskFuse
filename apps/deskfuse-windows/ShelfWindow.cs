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
    private readonly ListBox list = new() { BorderThickness = new Thickness(0), Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent) };
    private readonly TextBlock status = new()
    {
        Text = "拖入本机文件，或从历史加入",
        TextWrapping = TextWrapping.Wrap,
        Foreground = (Brush)Application.Current.Resources["DFMutedBrush"],
        FontSize = 12,
    };
    private readonly Button expand = new() { Content = "展开", Padding = new Thickness(10, 4, 10, 4), MinHeight = 0, Margin = new Thickness(0, 0, 0, 0) };
    private bool exiting;
    private bool expanded;

    public ShelfWindow(NativeClipboard clipboard, ClipboardHistory history, DesktopClipboardController controller)
    {
        this.clipboard = clipboard;
        this.history = history;
        this.controller = controller;
        AppIcon.Apply(window);
        window.AppWindow.Title = "DeskFuse 暂存区";
        window.AppWindow.Resize(new global::Windows.Graphics.SizeInt32(320, 480));
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
        list.ItemContainerStyle = new Style(typeof(ListBoxItem))
        {
            Setters =
            {
                new Setter(ListBoxItem.PaddingProperty, new Thickness(12, 8, 12, 8)),
                new Setter(ListBoxItem.MarginProperty, new Thickness(2)),
                new Setter(ListBoxItem.CornerRadiusProperty, new CornerRadius(6)),
            },
        };

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
            window.AppWindow.Resize(expanded ? new global::Windows.Graphics.SizeInt32(480, 640) : new global::Windows.Graphics.SizeInt32(320, 480));
            expand.Content = expanded ? "收起" : "展开";
        };

        var header = new Grid { Margin = new Thickness(0, 0, 0, 12) };
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var title = new TextBlock { Text = "暂存区", FontWeight = Microsoft.UI.Text.FontWeights.SemiBold, FontSize = 18, VerticalAlignment = VerticalAlignment.Center };
        Grid.SetColumn(title, 0); header.Children.Add(title);
        Grid.SetColumn(expand, 1); header.Children.Add(expand);

        var card = new Border
        {
            Background = (Brush)Application.Current.Resources["CardBackgroundFillColorDefaultBrush"],
            BorderBrush = (Brush)Application.Current.Resources["CardStrokeColorDefaultBrush"],
            BorderThickness = new Thickness(1),
            CornerRadius = new CornerRadius(8),
            Padding = new Thickness(4),
            Child = list,
        };

        var layout = new Grid { Margin = new Thickness(16) };
        layout.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        layout.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        layout.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        Grid.SetRow(header, 0); Grid.SetRow(card, 1); Grid.SetRow(status, 2);
        layout.Children.Add(header); layout.Children.Add(card); layout.Children.Add(status);
        status.Margin = new Thickness(0, 10, 0, 0);
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
