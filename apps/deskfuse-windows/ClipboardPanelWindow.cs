using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Windows.System;
using Colors = Microsoft.UI.Colors;
using PointInt32 = global::Windows.Graphics.PointInt32;
using RectInt32 = global::Windows.Graphics.RectInt32;

namespace DeskFuse.Windows;

/// <summary>
/// 剪贴板快捷面板。选择不会改写系统剪贴板；“仅复制”和“粘贴到原应用”
/// 是两个独立动作，自动粘贴前会复核目标焦点、修饰键和原生剪贴板代次。
/// </summary>
public sealed class ClipboardPanelWindow : Window
{
    private readonly DesktopClipboardController clipboard;
    private readonly ClipboardLibrary library;
    private readonly IntPtr pasteTarget;
    private readonly Action<string> onNotice;
    private readonly Action onShowAll;
    private readonly Func<IReadOnlyList<(string Id, string Name)>> sharePeers;
    private readonly Func<ClipboardEntry, string?, string, Task> shareEntry;
    private readonly Func<ClipboardEntry, (IReadOnlyList<(string Id, string Name)> Items, string Hint)> workflowOptions;
    private readonly Func<ClipboardEntry, string, Task> runWorkflow;
    private readonly ListView list;
    private readonly TextBox search;
    private readonly TextBlock emptyText;
    private readonly TextBlock status;
    private readonly Button copyButton;
    private readonly Button pasteButton;
    private readonly CancellationTokenSource session = new();
    private string selectedSource = "所有设备";
    private ClipboardLibrary.Selection librarySelection = ClipboardLibrary.Selection.All;
    private ClipboardEntry[] lastEntries = [];
    private int refreshVersion;
    private bool searchComposing;
    private bool busy;
    private bool isClosed;
    private bool wasActivated;
    private bool closingForPaste;
    private int openFlyouts;

    public ClipboardPanelWindow(
        DesktopClipboardController clipboard,
        ClipboardLibrary library,
        IntPtr pasteTarget,
        Action<string> onNotice,
        Action onShowAll,
        Func<IReadOnlyList<(string Id, string Name)>> sharePeers,
        Func<ClipboardEntry, string?, string, Task> shareEntry,
        Func<ClipboardEntry, (IReadOnlyList<(string Id, string Name)> Items, string Hint)> workflowOptions,
        Func<ClipboardEntry, string, Task> runWorkflow)
    {
        this.clipboard = clipboard;
        this.library = library;
        this.pasteTarget = pasteTarget;
        this.onNotice = onNotice;
        this.onShowAll = onShowAll;
        this.sharePeers = sharePeers;
        this.shareEntry = shareEntry;
        this.workflowOptions = workflowOptions;
        this.runWorkflow = runWorkflow;

        Title = "DeskFuse 剪贴板";
        AppIcon.Apply(this);
        ExtendsContentIntoTitleBar = true;
        if (AppWindow.Presenter is global::Microsoft.UI.Windowing.OverlappedPresenter presenter)
        {
            presenter.SetBorderAndTitleBar(false, false);
            presenter.IsAlwaysOnTop = true;
            presenter.IsResizable = false;
            presenter.IsMinimizable = false;
            presenter.IsMaximizable = false;
        }
        try
        {
            AppWindow.TitleBar.ExtendsContentIntoTitleBar = true;
            AppWindow.TitleBar.ButtonBackgroundColor = Colors.Transparent;
            AppWindow.TitleBar.ButtonInactiveBackgroundColor = Colors.Transparent;
        }
        catch (Exception) { /* Older shells keep their default caption settings. */ }
        MoveNearPointer();

        var root = new Grid
        {
            Background = DfUi.Canvas,
            CornerRadius = new CornerRadius(12),
            BorderBrush = DfUi.Line,
            BorderThickness = new Thickness(1),
        };
        root.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        root.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        root.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });

        var header = new StackPanel { Spacing = 10, Margin = new Thickness(16, 16, 16, 12) };
        var titleRow = new Grid();
        titleRow.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        titleRow.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        titleRow.Children.Add(DfUi.Text("最近剪贴板", 15, true));
        var showAllButton = DfUi.Plain("查看全部", (_, _) => { Dismiss(); this.onShowAll(); });
        showAllButton.VerticalAlignment = VerticalAlignment.Center;
        Grid.SetColumn(showAllButton, 1);
        titleRow.Children.Add(showAllButton);
        header.Children.Add(titleRow);

        var searchSurface = DfUi.SearchSurface(
            search = DfUi.SearchField("搜索剪贴板", (_, _) => RefreshList()),
            height: 36);
        search.TextCompositionStarted += (_, _) => searchComposing = true;
        search.TextCompositionEnded += (_, _) => searchComposing = false;
        header.Children.Add(searchSurface);

        var filterRow = new Grid();
        filterRow.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        filterRow.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var sourceButton = new Button
        {
            Background = new SolidColorBrush(Colors.Transparent),
            BorderThickness = new Thickness(0),
            CornerRadius = new CornerRadius(6),
            Padding = new Thickness(8, 0, 8, 0),
            MinHeight = 28,
            Template = (ControlTemplate)Application.Current.Resources["DFFlatButtonTemplate"],
        };
        var sourceContent = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 };
        sourceContent.Children.Add(new FontIcon { Glyph = "\uE7F8", FontSize = 12, Foreground = DfUi.Muted });
        var sourceLabel = DfUi.Detail("所有设备");
        sourceContent.Children.Add(sourceLabel);
        sourceContent.Children.Add(new FontIcon { Glyph = "\uE70D", FontSize = 10, Foreground = DfUi.Muted });
        sourceButton.Content = sourceContent;
        sourceButton.Click += (_, _) =>
        {
            var flyout = new MenuFlyout();
            var sources = new[] { "所有设备" }.Concat(SourceNames()).ToArray();
            foreach (var source in sources)
            {
                var item = new MenuFlyoutItem { Text = source };
                var captured = source;
                item.Click += (_, _) =>
                {
                    selectedSource = captured;
                    sourceLabel.Text = captured;
                    RefreshList();
                };
                flyout.Items.Add(item);
            }
            ShowTrackedFlyout(flyout, sourceButton);
        };
        filterRow.Children.Add(sourceButton);
        header.Children.Add(filterRow);
        Grid.SetRow(header, 0);
        root.Children.Add(header);

        list = new ListView
        {
            Background = new SolidColorBrush(Colors.Transparent),
            BorderThickness = new Thickness(0),
            Padding = new Thickness(8, 0, 8, 8),
            SelectionMode = ListViewSelectionMode.Single,
            IsItemClickEnabled = true,
        };
        list.ItemClick += (_, args) =>
        {
            if (args.ClickedItem is not ClipboardEntry entry) return;
            list.SelectedItem = entry;
            list.Focus(FocusState.Programmatic);
        };
        list.DoubleTapped += async (_, args) =>
        {
            if (EntryFrom(args.OriginalSource) is not ClipboardEntry entry || busy) return;
            args.Handled = true;
            list.SelectedItem = entry;
            await PasteEntryAsync(entry);
        };
        list.RightTapped += (_, args) =>
        {
            if (EntryFrom(args.OriginalSource) is not ClipboardEntry entry || busy) return;
            args.Handled = true;
            list.SelectedItem = entry;
            var flyout = new MenuFlyout();
            var pasteItem = new MenuFlyoutItem { Text = "粘贴到原应用", Icon = new FontIcon { Glyph = "\uE8B5" } };
            pasteItem.Click += async (_, _) => await PasteEntryAsync(entry);
            flyout.Items.Add(pasteItem);
            var copyItem = new MenuFlyoutItem { Text = "仅复制", Icon = new FontIcon { Glyph = "\uE8C8" } };
            copyItem.Click += async (_, _) => await CopyEntryAsync(entry);
            flyout.Items.Add(copyItem);
            if (HasPlainText(entry))
            {
                var plainItem = new MenuFlyoutItem { Text = "粘贴为纯文本" };
                plainItem.Click += async (_, _) => await PasteEntryAsync(entry, plainText: true);
                flyout.Items.Add(plainItem);
            }
            flyout.Items.Add(new MenuFlyoutSeparator());
            flyout.Items.Add(BuildShareMenu(entry));
            flyout.Items.Add(BuildWorkflowMenu(entry));
            ShowTrackedFlyout(flyout, list, args.GetPosition(list));
        };
        list.ItemTemplate = (DataTemplate)Microsoft.UI.Xaml.Markup.XamlReader.Load(
            """
            <DataTemplate xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation">
              <Grid Padding="10,6">
                <Grid.ColumnDefinitions>
                  <ColumnDefinition Width="Auto" />
                  <ColumnDefinition Width="*" />
                  <ColumnDefinition Width="Auto" />
                </Grid.ColumnDefinitions>
                <Border Width="36" Height="36" CornerRadius="6" Background="{ThemeResource DFControlBrush}">
                  <FontIcon Glyph="{Binding KindGlyph}" FontSize="14" Foreground="{ThemeResource DFMutedBrush}" />
                </Border>
                <StackPanel Grid.Column="1" Margin="10,0,8,0" VerticalAlignment="Center" Spacing="2">
                  <TextBlock Text="{Binding Title}" FontSize="13" FontWeight="Medium"
                             TextTrimming="CharacterEllipsis" TextWrapping="NoWrap" />
                  <TextBlock Text="{Binding SourceLabel}" FontSize="11"
                             Foreground="{ThemeResource DFMutedBrush}" TextTrimming="CharacterEllipsis"
                             TextWrapping="NoWrap" />
                </StackPanel>
                <TextBlock Grid.Column="2" Text="{Binding TimeLabel}" FontSize="11"
                           Foreground="{ThemeResource DFMutedBrush}" VerticalAlignment="Center" />
              </Grid>
            </DataTemplate>
            """);
        Grid.SetRow(list, 1);
        root.Children.Add(list);

        emptyText = DfUi.Detail("没有剪贴板记录");
        emptyText.HorizontalAlignment = HorizontalAlignment.Center;
        emptyText.VerticalAlignment = VerticalAlignment.Center;
        emptyText.Visibility = Visibility.Collapsed;
        Grid.SetRow(emptyText, 1);
        root.Children.Add(emptyText);

        var footer = new Border
        {
            Background = DfUi.Surface,
            BorderBrush = DfUi.Line,
            BorderThickness = new Thickness(0, 1, 0, 0),
            Padding = new Thickness(12, 10, 12, 10),
        };
        var footerContent = new StackPanel { Spacing = 8 };
        status = DfUi.Detail("", 11);
        status.Foreground = DfUi.Danger;
        status.Visibility = Visibility.Collapsed;
        footerContent.Children.Add(status);
        var actionRow = new Grid { ColumnSpacing = 8 };
        actionRow.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        actionRow.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        actionRow.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var hint = DfUi.Detail("Enter 粘贴 · Ctrl+C 复制", 11);
        hint.VerticalAlignment = VerticalAlignment.Center;
        actionRow.Children.Add(hint);
        copyButton = DfUi.Secondary("复制", async (_, _) => { if (list.SelectedItem is ClipboardEntry entry) await CopyEntryAsync(entry); });
        Grid.SetColumn(copyButton, 1);
        actionRow.Children.Add(copyButton);
        pasteButton = DfUi.Primary("粘贴", async (_, _) => { if (list.SelectedItem is ClipboardEntry entry) await PasteEntryAsync(entry); });
        Grid.SetColumn(pasteButton, 2);
        actionRow.Children.Add(pasteButton);
        footerContent.Children.Add(actionRow);
        footer.Child = footerContent;
        Grid.SetRow(footer, 2);
        root.Children.Add(footer);

        Content = root;
        RefreshList();
        root.AddHandler(UIElement.KeyDownEvent, new KeyEventHandler(OnKeyDown), true);
        root.Loaded += (_, _) => search.Focus(FocusState.Programmatic);
        Activated += (_, args) =>
        {
            if (args.WindowActivationState != WindowActivationState.Deactivated)
            {
                wasActivated = true;
                return;
            }
            // WinUI activation events can briefly lag the actual Win32 foreground
            // window when a panel is reopened immediately after focus loss.
            var handle = WinRT.Interop.WindowNative.GetWindowHandle(this);
            if (wasActivated && !closingForPaste && openFlyouts == 0
                && !ClipboardPanelNative.IsForeground(handle)) Dismiss();
        };
        Closed += (_, _) =>
        {
            isClosed = true;
            if (!closingForPaste) session.Cancel();
        };
    }

    /// <summary>Capture before activating the panel; DeskFuse itself is never a paste target.</summary>
    internal static IntPtr CaptureExternalTarget() => ClipboardPanelNative.CaptureExternalTarget();

    public void Dismiss()
    {
        if (isClosed) return;
        isClosed = true;
        closingForPaste = false;
        session.Cancel();
        Close();
    }

    public void RefreshList()
    {
        if (isClosed) return;
        if (clipboard.CanonicalConnected)
        {
            _ = RefreshCanonicalAsync();
            return;
        }
        refreshVersion++;
        RenderLocal();
    }

    private void RenderLocal()
    {
        var searchText = search.Text.Trim();
        var visible = library.Filter(librarySelection, clipboard.Entries)
            .Where(entry =>
                (selectedSource == "所有设备" || entry.SourceName == selectedSource)
                && (searchText.Length == 0 || $"{entry.Title} {entry.Preview} {entry.SourceName}".Contains(searchText, StringComparison.CurrentCultureIgnoreCase)))
            .Take(50)
            .ToArray();
        ShowEntries(visible, "没有剪贴板记录");
    }

    private async Task RefreshCanonicalAsync()
    {
        int version = ++refreshVersion;
        string searchText = search.Text.Trim();
        string? source = selectedSource == "所有设备" ? null : selectedSource;
        try
        {
            var page = await clipboard.QueryCanonicalAsync(searchText, null, "all", source, 0, 50);
            if (version != refreshVersion || isClosed) return;
            ClearStatus();
            ShowEntries(page.Items.Select(CanonicalRow.ToEntry).ToArray(),
                page.Total == 0 ? "还没有剪贴板记录" : "没有匹配的内容");
        }
        catch (Exception ex) when (ex is IOException or InvalidOperationException or OperationCanceledException or TimeoutException)
        {
            if (version != refreshVersion || isClosed) return;
            ShowError($"内容库暂时不可用：{ex.Message}");
            RenderLocal();
        }
    }

    private void ShowEntries(ClipboardEntry[] visible, string emptyLabel)
    {
        lastEntries = visible;
        list.ItemsSource = visible;
        emptyText.Text = emptyLabel;
        emptyText.Visibility = visible.Length == 0 ? Visibility.Visible : Visibility.Collapsed;
        list.SelectedIndex = visible.Length == 0 ? -1 : 0;
    }

    private string[] SourceNames() =>
        clipboard.Entries.Select(entry => entry.SourceName)
            .Concat(lastEntries.Select(entry => entry.SourceName))
            .Where(name => name.Length > 0)
            .Distinct()
            .ToArray();

    private async void OnKeyDown(object sender, KeyRoutedEventArgs args)
    {
        if (busy || isClosed || searchComposing) return;
        bool control = ClipboardPanelNative.IsControlDown();
        if (control && args.Key == VirtualKey.F)
        {
            search.Focus(FocusState.Programmatic);
            search.SelectAll();
            args.Handled = true;
            return;
        }
        if (control && args.Key == VirtualKey.C)
        {
            // Preserve the text box's normal copy command when it has a selection.
            if (search.FocusState != FocusState.Unfocused && search.SelectedText.Length > 0) return;
            if (list.SelectedItem is ClipboardEntry entry) await CopyEntryAsync(entry);
            args.Handled = true;
            return;
        }
        int quickIndex = QuickIndex(args.Key);
        if (control && quickIndex >= 0)
        {
            if (quickIndex < list.Items.Count && list.Items[quickIndex] is ClipboardEntry entry)
            {
                list.SelectedIndex = quickIndex;
                await PasteEntryAsync(entry);
            }
            args.Handled = true;
            return;
        }
        switch (args.Key)
        {
            case VirtualKey.Escape:
                Dismiss();
                args.Handled = true;
                break;
            case VirtualKey.Enter when list.SelectedItem is ClipboardEntry entry:
                await PasteEntryAsync(entry);
                args.Handled = true;
                break;
            case VirtualKey.Down:
                MoveSelection(1);
                args.Handled = true;
                break;
            case VirtualKey.Up:
                MoveSelection(-1);
                args.Handled = true;
                break;
            case VirtualKey.Delete when search.FocusState == FocusState.Unfocused:
                if (list.SelectedItem is ClipboardEntry selectedEntry)
                {
                    if (selectedEntry.IsCanonical)
                    {
                        SetBusy(true);
                        try { await clipboard.DeleteCanonicalAsync([selectedEntry]); }
                        catch (Exception ex) when (ex is IOException or InvalidOperationException) { ShowError(ex.Message); }
                        finally { SetBusy(false); }
                        RefreshList();
                    }
                    else
                    {
                        clipboard.Remove(selectedEntry);
                        RefreshList();
                    }
                }
                args.Handled = true;
                break;
        }
    }

    private static int QuickIndex(VirtualKey key) => key switch
    {
        VirtualKey.Number1 or VirtualKey.NumberPad1 => 0,
        VirtualKey.Number2 or VirtualKey.NumberPad2 => 1,
        VirtualKey.Number3 or VirtualKey.NumberPad3 => 2,
        VirtualKey.Number4 or VirtualKey.NumberPad4 => 3,
        VirtualKey.Number5 or VirtualKey.NumberPad5 => 4,
        VirtualKey.Number6 or VirtualKey.NumberPad6 => 5,
        VirtualKey.Number7 or VirtualKey.NumberPad7 => 6,
        VirtualKey.Number8 or VirtualKey.NumberPad8 => 7,
        VirtualKey.Number9 or VirtualKey.NumberPad9 => 8,
        _ => -1,
    };

    private void MoveSelection(int delta)
    {
        if (list.Items.Count == 0) return;
        int index = list.SelectedIndex < 0 ? 0 : Math.Clamp(list.SelectedIndex + delta, 0, list.Items.Count - 1);
        list.SelectedIndex = index;
        list.ScrollIntoView(list.SelectedItem);
    }

    private async Task CopyEntryAsync(ClipboardEntry entry)
    {
        if (busy || isClosed) return;
        SetBusy(true);
        ClearStatus();
        try
        {
            entry = await ResolveAsync(entry);
            clipboard.Restore(entry);
            Dismiss();
        }
        catch (Exception ex)
        {
            SetBusy(false);
            ShowError(RestoreError(ex));
        }
    }

    private async Task<ClipboardEntry> ResolveAsync(ClipboardEntry entry)
    {
        if (!entry.IsCanonical || entry.ContentLoaded) return entry;
        return await clipboard.ResolveCanonicalAsync(entry, session.Token);
    }

    private async Task PasteEntryAsync(ClipboardEntry entry, bool plainText = false)
    {
        if (busy || isClosed) return;
        SetBusy(true);
        ClearStatus();
        uint expectedSequence;
        try
        {
            entry = await ResolveAsync(entry);
            if (plainText)
            {
                var text = PlainText(entry);
                if (text is null)
                {
                    SetBusy(false);
                    ShowError("这条记录没有可用的纯文本。");
                    return;
                }
                entry = entry with { Content = new ClipboardContent("text/plain", text) };
            }
            expectedSequence = clipboard.Restore(entry);
        }
        catch (Exception ex)
        {
            SetBusy(false);
            ShowError(RestoreError(ex));
            return;
        }
        if (!ClipboardPanelNative.IsTargetAvailable(pasteTarget))
        {
            SetBusy(false);
            ShowError("原目标应用不可用；内容已复制，可切换到目标应用后手动粘贴。");
            return;
        }

        closingForPaste = true;
        isClosed = true;
        Close();
        try
        {
            await ClipboardPanelNative.PasteWhenReadyAsync(pasteTarget, expectedSequence, clipboard.Native, session.Token);
            onNotice("已向原应用发送粘贴。");
        }
        catch (OperationCanceledException) { }
        catch (ClipboardPanelNative.Failure ex)
        {
            onNotice(ex.ContentPreserved ? $"{ex.Message}；内容已复制，可手动粘贴。" : ex.Message);
        }
        catch (Exception)
        {
            onNotice("自动粘贴未完成；内容已复制，可手动粘贴。");
        }
        finally { closingForPaste = false; }
    }

    private static string RestoreError(Exception ex) => ex is InvalidOperationException or IOException or NotSupportedException
        ? ex.Message
        : "无法恢复剪贴板，请检查当前会话和文件是否可用后重试。";

    /// <summary>Mac parity: paste only the plain-text representation.</summary>
    private static bool HasPlainText(ClipboardEntry entry) => entry.Kind == "text"
        || entry.Content.Representations?.Any(item => item.Mime == "text/plain") == true;

    private static byte[]? PlainText(ClipboardEntry entry) =>
        entry.Content.Representations?.FirstOrDefault(item => item.Mime == "text/plain")?.Data
        ?? (entry.Kind == "text" ? entry.Content.Data : null);

    private static ClipboardEntry? EntryFrom(object source) => source is FrameworkElement element
        ? element.DataContext as ClipboardEntry
        : null;

    private MenuFlyoutSubItem BuildShareMenu(ClipboardEntry entry)
    {
        var menu = new MenuFlyoutSubItem { Text = "发送到设备" };
        if (entry.CanonicalId is not ulong id)
        {
            menu.IsEnabled = false;
            menu.Items.Add(new MenuFlyoutItem { Text = "仅内容库条目可直接发送", IsEnabled = false });
            return menu;
        }
        var peers = sharePeers();
        if (peers.Count == 0)
        {
            menu.Items.Add(new MenuFlyoutItem { Text = "没有已连接的设备", IsEnabled = false });
            return menu;
        }
        var workflows = workflowOptions(entry);
        foreach (var peer in peers)
        {
            var peerMenu = new MenuFlyoutSubItem { Text = peer.Name };
            var direct = new MenuFlyoutItem { Text = "直接发送" };
            direct.Click += async (_, _) =>
            {
                await shareEntry(entry, null, peer.Id);
                Dismiss();
            };
            peerMenu.Items.Add(direct);
            if (workflows.Items.Count > 0)
            {
                peerMenu.Items.Add(new MenuFlyoutSeparator());
                foreach (var (workflowId, name) in workflows.Items)
                {
                    var item = new MenuFlyoutItem { Text = $"使用「{name}」发送" };
                    item.Click += async (_, _) =>
                    {
                        await shareEntry(entry, workflowId, peer.Id);
                        Dismiss();
                    };
                    peerMenu.Items.Add(item);
                }
            }
            menu.Items.Add(peerMenu);
        }
        return menu;
    }

    private MenuFlyoutSubItem BuildWorkflowMenu(ClipboardEntry entry)
    {
        var menu = new MenuFlyoutSubItem { Text = "使用工作流" };
        var options = workflowOptions(entry);
        if (options.Items.Count == 0)
        {
            menu.IsEnabled = false;
            menu.Items.Add(new MenuFlyoutItem { Text = options.Hint, IsEnabled = false });
            return menu;
        }
        foreach (var (id, name) in options.Items)
        {
            var item = new MenuFlyoutItem { Text = name };
            item.Click += async (_, _) =>
            {
                // History-entry runs only produce a copy; paste stays an explicit action.
                await runWorkflow(entry, id);
                Dismiss();
            };
            menu.Items.Add(item);
        }
        return menu;
    }

    private void SetBusy(bool value)
    {
        busy = value;
        list.IsEnabled = !value;
        copyButton.IsEnabled = !value;
        pasteButton.IsEnabled = !value;
    }

    private void ClearStatus() => status.Visibility = Visibility.Collapsed;

    private void ShowError(string message)
    {
        status.Text = message;
        status.Visibility = Visibility.Visible;
    }

    private void ShowTrackedFlyout(MenuFlyout flyout, FrameworkElement target, global::Windows.Foundation.Point? point = null)
    {
        openFlyouts++;
        flyout.Closed += (_, _) =>
        {
            openFlyouts--;
            if (!isClosed && !closingForPaste && openFlyouts == 0) Activate();
        };
        try
        {
            if (point is { } position) flyout.ShowAt(target, position);
            else flyout.ShowAt(target);
        }
        catch (Exception)
        {
            openFlyouts--;
            throw;
        }
    }

    private void MoveNearPointer()
    {
        ClipboardPanelNative.GetCursorPos(out var cursor);
        var display = global::Microsoft.UI.Windowing.DisplayArea.GetFromPoint(
            new PointInt32(cursor.X, cursor.Y), global::Microsoft.UI.Windowing.DisplayAreaFallback.Nearest);
        var work = display.WorkArea;
        int left = work.X + 8, top = work.Y + 8;
        int right = work.X + work.Width - 8, bottom = work.Y + work.Height - 8;
        int width = Math.Min(440, Math.Max(1, right - left));
        int height = Math.Min(780, Math.Max(1, bottom - top - 40));
        int preferredX = cursor.X + 12 + width <= right ? cursor.X + 12 : cursor.X - width - 12;
        int x = Math.Clamp(preferredX, left, Math.Max(left, right - width));
        int y = Math.Clamp(cursor.Y - 16, top, Math.Max(top, bottom - height));
        AppWindow.MoveAndResize(new RectInt32(x, y, width, height));
    }
}

/// <summary>Win32 focus and input boundary for the clipboard panel.</summary>
internal static class ClipboardPanelNative
{
    internal sealed class Failure : InvalidOperationException
    {
        internal Failure(string message, bool contentPreserved = true) : base(message) => ContentPreserved = contentPreserved;
        internal bool ContentPreserved { get; }
    }

    internal static IntPtr CaptureExternalTarget()
    {
        IntPtr window = GetForegroundWindow();
        if (window == IntPtr.Zero) return IntPtr.Zero;
        window = GetAncestor(window, GaRoot);
        GetWindowThreadProcessId(window, out uint processId);
        return processId == Environment.ProcessId || !IsWindowVisible(window) ? IntPtr.Zero : window;
    }

    internal static bool IsTargetAvailable(IntPtr window) => window != IntPtr.Zero && IsWindow(window) && IsWindowVisible(window);

    internal static bool IsControlDown() => IsKeyDown(VkControl);

    internal static bool IsForeground(IntPtr window) =>
        window != IntPtr.Zero && GetForegroundWindow() == window;

    internal static async Task PasteWhenReadyAsync(
        IntPtr target, uint expectedSequence, NativeClipboard clipboard, CancellationToken token)
    {
        if (!IsTargetAvailable(target)) throw new Failure("原目标应用不可用");
        if (IsIconic(target)) ShowWindowAsync(target, SwRestore);
        SetForegroundWindow(target);
        for (var attempt = 0; attempt < 100; attempt++)
        {
            token.ThrowIfCancellationRequested();
            if (!IsWindow(target)) throw new Failure("原目标应用已关闭");
            if (GetForegroundWindow() == target && !ModifiersDown())
            {
                if (clipboard.SequenceNumber != expectedSequence)
                    throw new Failure("剪贴板内容已变化，未发送粘贴；请重新选择。", contentPreserved: false);
                SendPaste();
                return;
            }
            await Task.Delay(20, token);
        }
        throw new Failure("目标应用未获得焦点或修饰键尚未释放，已取消自动粘贴");
    }

    private static bool ModifiersDown() => IsKeyDown(VkShift) || IsKeyDown(VkControl)
        || IsKeyDown(VkMenu) || IsKeyDown(VkLeftWin) || IsKeyDown(VkRightWin);

    private static bool IsKeyDown(int key) => (GetAsyncKeyState(key) & 0x8000) != 0;

    private static void SendPaste()
    {
        var inputs = new[] { Key(VkControl, false), Key(VkV, false), Key(VkV, true), Key(VkControl, true) };
        uint sent = SendInput((uint)inputs.Length, inputs, Marshal.SizeOf<Input>());
        if (sent == inputs.Length) return;
        ReleasePasteKeys();
        throw new Failure("Windows 未允许向目标应用发送粘贴");
    }

    private static void ReleasePasteKeys()
    {
        var release = new[] { Key(VkV, true), Key(VkControl, true) };
        SendInput((uint)release.Length, release, Marshal.SizeOf<Input>());
    }

    private static Input Key(int virtualKey, bool keyUp) => new()
    {
        Type = InputKeyboard,
        Data = new InputUnion
        {
            Keyboard = new KeyboardInput
            {
                VirtualKey = (ushort)virtualKey,
                ScanCode = 0,
                Flags = keyUp ? KeyeventKeyup : 0,
                Time = 0,
                ExtraInfo = UIntPtr.Zero,
            },
        },
    };

    private const int GaRoot = 2, SwRestore = 9;
    private const int VkShift = 0x10, VkControl = 0x11, VkMenu = 0x12, VkV = 0x56, VkLeftWin = 0x5B, VkRightWin = 0x5C;
    private const uint InputKeyboard = 1, KeyeventKeyup = 0x0002;

    [StructLayout(LayoutKind.Sequential)]
    private struct Input
    {
        public uint Type;
        public InputUnion Data;
    }

    [StructLayout(LayoutKind.Explicit)]
    private struct InputUnion
    {
        [FieldOffset(0)] public MouseInput Mouse;
        [FieldOffset(0)] public KeyboardInput Keyboard;
        [FieldOffset(0)] public HardwareInput Hardware;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct MouseInput
    {
        public int X, Y;
        public uint MouseData, Flags, Time;
        public UIntPtr ExtraInfo;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct KeyboardInput
    {
        public ushort VirtualKey, ScanCode;
        public uint Flags, Time;
        public UIntPtr ExtraInfo;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct HardwareInput
    {
        public uint Message;
        public ushort ParameterLow, ParameterHigh;
    }

    [DllImport("user32.dll")] private static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] private static extern IntPtr GetAncestor(IntPtr window, uint flags);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
    [DllImport("user32.dll")] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool IsWindow(IntPtr window);
    [DllImport("user32.dll")] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool IsIconic(IntPtr window);
    [DllImport("user32.dll")] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool ShowWindowAsync(IntPtr window, int command);
    [DllImport("user32.dll")] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] private static extern short GetAsyncKeyState(int key);
    [DllImport("user32.dll", SetLastError = true)] private static extern uint SendInput(uint count, Input[] inputs, int size);

    [StructLayout(LayoutKind.Sequential)]
    internal struct Point { internal int X, Y; }

    [DllImport("user32.dll", EntryPoint = "GetCursorPos")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GetCursorPosNative(out Point point);

    internal static void GetCursorPos(out Point point)
    {
        if (!GetCursorPosNative(out point)) point = new Point();
    }
}
