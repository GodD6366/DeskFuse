using System.Collections.ObjectModel;
using System.ComponentModel;
using System.IO;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Interop;
using System.Windows.Media;
using Button = System.Windows.Controls.Button;
using DataObject = System.Windows.DataObject;
using DataFormats = System.Windows.DataFormats;
using ListBox = System.Windows.Controls.ListBox;
using Orientation = System.Windows.Controls.Orientation;
using Point = System.Windows.Point;
using Brushes = System.Windows.Media.Brushes;
using DragDropEffects = System.Windows.DragDropEffects;

namespace DeskFuse.Windows;

/// <summary>Local temporary shelf. It holds references; removing items never removes user files.</summary>
public sealed class ShelfWindow : Window
{
    private readonly NativeClipboard clipboard;
    private readonly ClipboardHistory history;
    private readonly ObservableCollection<ClipboardEntry> entries = new();
    private readonly ListBox list = new() { DisplayMemberPath = nameof(ClipboardEntry.Title), BorderThickness = new Thickness(0) };
    private readonly TextBlock status = new() { Text = "拖入本机文件，或从历史加入", TextWrapping = TextWrapping.Wrap, Foreground = Brushes.SlateGray, FontSize = 11 };
    private Point? dragStart;
    private bool exiting;
    public ShelfWindow(NativeClipboard clipboard, ClipboardHistory history)
    {
        this.clipboard = clipboard; this.history = history;
        Title = "DeskFuse 暂存区"; Width = 240; Height = 240; MinWidth = 240; MinHeight = 240;
        ShowInTaskbar = false; ShowActivated = false; ResizeMode = ResizeMode.CanResizeWithGrip;
        Background = Brushes.White; AllowDrop = true; WindowStartupLocation = WindowStartupLocation.CenterOwner;
        var layout = new DockPanel { Margin = new Thickness(10) };
        var header = new DockPanel { Margin = new Thickness(0, 0, 0, 8) };
        var expand = new Button { Content = "展开", Padding = new Thickness(5, 2, 5, 2) };
        expand.Click += (_, _) => { var large = Width > 300; Width = large ? 240 : 480; Height = large ? 240 : 420; expand.Content = large ? "展开" : "收起"; };
        DockPanel.SetDock(expand, Dock.Right); header.Children.Add(expand);
        header.Children.Add(new TextBlock { Text = "暂存区", FontWeight = FontWeights.SemiBold, FontSize = 16 });
        DockPanel.SetDock(header, Dock.Top); layout.Children.Add(header);
        DockPanel.SetDock(status, Dock.Bottom); layout.Children.Add(status);
        list.ItemsSource = entries; layout.Children.Add(list); Content = layout;
        var menu = new ContextMenu();
        var copy = new MenuItem { Header = "复制" }; copy.Click += (_, _) => CopySelection(); menu.Items.Add(copy);
        var send = new MenuItem { Header = "发送到设备", IsEnabled = false, ToolTip = "Windows 文件传输链路尚未接入" }; menu.Items.Add(send);
        var remove = new MenuItem { Header = "移出暂存区" }; remove.Click += (_, _) => { if (list.SelectedItem is ClipboardEntry e) entries.Remove(e); }; menu.Items.Add(remove);
        list.ContextMenu = menu;
        list.PreviewMouseLeftButtonDown += (_, e) => dragStart = e.GetPosition(list);
        list.PreviewMouseMove += BeginDrag;
        list.MouseDoubleClick += (_, _) => CopySelection();
        list.KeyDown += (_, e) => { if (e.Key == Key.Enter) { CopySelection(); e.Handled = true; } };
        DragOver += (_, e) => { e.Effects = e.Data.GetDataPresent(DataFormats.FileDrop) ? DragDropEffects.Copy : DragDropEffects.None; e.Handled = true; };
        Drop += OnDrop;
        Closing += OnClosing;
    }
    public void Add(ClipboardEntry entry)
    {
        if (entries.Any(e => e.Id == entry.Id)) return;
        if (entries.Count >= 100) { status.Text = "暂存区已满（100 项），请先移出部分项目"; return; }
        entries.Insert(0, entry); list.SelectedItem = entry;
        status.Text = $"{entries.Count} 项 · 移出不会删除原文件";
    }
    private void OnDrop(object sender, System.Windows.DragEventArgs args)
    {
        try
        {
            if (args.Data.GetData(DataFormats.FileDrop) is not string[] paths) return;
            NativeClipboard.ValidateLocalFiles(paths);
            // Staging must not silently replace the system clipboard or transmit file bodies.
            var content = new ClipboardContent(ClipboardContent.LocalFilesMime, [], paths);
            Add(history.Add(content, "local", "本机")); args.Effects = DragDropEffects.Copy;
        }
        catch (Exception ex) when (ex is IOException or InvalidOperationException or UnauthorizedAccessException) { status.Text = "无法暂存：文件不可用或数量超过限制"; }
        args.Handled = true;
    }
    private void CopySelection()
    {
        if (list.SelectedItem is not ClipboardEntry entry) return;
        try { clipboard.Write(entry.Content); status.Text = "已复制到本机剪贴板"; }
        catch (Exception ex) when (ex is IOException or InvalidOperationException or System.Runtime.InteropServices.COMException or NotSupportedException or ArgumentException)
        { status.Text = "复制失败，原文件保持不变"; }
    }
    private void BeginDrag(object sender, System.Windows.Input.MouseEventArgs args)
    {
        if (dragStart is not Point start || args.LeftButton != MouseButtonState.Pressed || list.SelectedItem is not ClipboardEntry entry) return;
        var now = args.GetPosition(list);
        if (Math.Abs(now.X - start.X) < SystemParameters.MinimumHorizontalDragDistance && Math.Abs(now.Y - start.Y) < SystemParameters.MinimumVerticalDragDistance) return;
        dragStart = null;
        try
        {
            var data = new DataObject();
            if (entry.Content.Files is { Count: > 0 } paths)
            { NativeClipboard.ValidateLocalFiles(paths); data.SetData(DataFormats.FileDrop, paths.ToArray()); }
            else if (entry.Content.Mime == "image/png") data.SetImage(NativeClipboard.DecodeImage(entry.Content.Data));
            else if (entry.Content.Mime == "text/plain") data.SetText(System.Text.Encoding.UTF8.GetString(entry.Content.Data));
            else { status.Text = "此格式请使用复制"; return; }
            // Copy only: Explorer must never move/delete the source as a shelf side effect.
            DragDrop.DoDragDrop(list, data, DragDropEffects.Copy);
        }
        catch (Exception ex) when (ex is IOException or InvalidOperationException or System.Runtime.InteropServices.COMException or ArgumentException)
        { status.Text = "拖出失败：请确认原文件仍可用"; }
    }
    private void OnClosing(object? sender, CancelEventArgs args) { if (!exiting) { args.Cancel = true; Hide(); } }
    public void Shutdown() { exiting = true; Close(); }
}
