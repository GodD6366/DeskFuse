using System.Runtime.InteropServices;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace DeskFuse.Windows;

/// <summary>
/// Progress for a held paste, next to the caret, like the macOS RemotePasteHUD.
/// The window never takes focus, so the held paste lands where it was typed.
/// </summary>
internal sealed class RemotePasteHud : IRemotePasteHud
{
    private const int Width = 300, Height = 64;
    private readonly Window window = new();
    private readonly TextBlock title = DfUi.Text("", 13, true, wrap: false);
    private readonly ProgressBar bar = new() { Minimum = 0, Maximum = 1, Height = 4 };
    private readonly DispatcherQueueTimer hideTimer;
    private bool visible;

    public RemotePasteHud(DispatcherQueue dispatcher, Action cancel)
    {
        window.Title = "DeskFuse 粘贴";
        window.ExtendsContentIntoTitleBar = true;
        if (window.AppWindow.Presenter is OverlappedPresenter presenter)
        {
            presenter.SetBorderAndTitleBar(false, false);
            presenter.IsAlwaysOnTop = true;
            presenter.IsResizable = false;
            presenter.IsMaximizable = false;
            presenter.IsMinimizable = false;
        }
        window.AppWindow.IsShownInSwitchers = false;
        IntPtr handle = WinRT.Interop.WindowNative.GetWindowHandle(window);
        // WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW: clicks and showing never steal focus.
        SetWindowLongPtr(handle, -20, (IntPtr)(GetWindowLongPtr(handle, -20).ToInt64() | 0x08000000 | 0x00000080));
        title.TextTrimming = TextTrimming.CharacterEllipsis;
        var text = new StackPanel { Spacing = 8, VerticalAlignment = VerticalAlignment.Center };
        text.Children.Add(title);
        text.Children.Add(bar);
        var close = DfUi.IconButton("\uE711", (_, _) => cancel(), "取消粘贴（Esc）", 24, 10);
        close.VerticalAlignment = VerticalAlignment.Center;
        Grid.SetColumn(close, 1);
        var row = new Grid { ColumnSpacing = 10 };
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        row.Children.Add(text);
        row.Children.Add(close);
        window.Content = new Border
        {
            Background = DfUi.Surface,
            BorderBrush = DfUi.Line,
            BorderThickness = new Thickness(1),
            CornerRadius = new CornerRadius(12),
            Padding = new Thickness(14, 10, 8, 10),
            Child = row,
        };
        hideTimer = dispatcher.CreateTimer();
        hideTimer.Interval = TimeSpan.FromSeconds(2.5);
        hideTimer.IsRepeating = false;
        hideTimer.Tick += (_, _) => Dismiss();
    }

    public void Present(string peer, long received, long total, int x, int y)
    {
        hideTimer.Stop();
        if (total <= 0)
        {
            title.Text = $"等待 {peer} 发送…";
            bar.IsIndeterminate = true;
        }
        else
        {
            double fraction = Math.Clamp((double)received / total, 0, 1);
            title.Text = $"正在从 {peer} 接收 · {fraction:P0}";
            bar.IsIndeterminate = false;
            bar.Value = fraction;
        }
        bar.Visibility = Visibility.Visible;
        if (!visible) Place(x, y);
        visible = true;
        window.AppWindow.Show(false);
    }

    public void Fail(string message)
    {
        if (!visible) return;
        title.Text = message;
        bar.Visibility = Visibility.Collapsed;
        hideTimer.Stop();
        hideTimer.Start();
    }

    public void Dismiss()
    {
        hideTimer.Stop();
        visible = false;
        window.AppWindow.Hide();
    }

    public void Shutdown()
    {
        hideTimer.Stop();
        window.Close();
    }

    private void Place(int x, int y)
    {
        var point = new global::Windows.Graphics.PointInt32(x, y);
        double scale = GetDpiForMonitor(MonitorFromPoint(point, 2), 0, out uint dpi, out _) == 0 && dpi > 0 ? dpi / 96.0 : 1;
        int width = (int)(Width * scale), height = (int)(Height * scale), gap = (int)(8 * scale);
        var area = DisplayArea.GetFromPoint(point, DisplayAreaFallback.Nearest).WorkArea;
        int left = Math.Clamp(x, area.X + gap, Math.Max(area.X + gap, area.X + area.Width - width - gap));
        // Below the caret line; above it when the caret sits near the bottom edge.
        int top = y + gap + height <= area.Y + area.Height ? y + gap : y - gap * 4 - height;
        top = Math.Clamp(top, area.Y + gap, Math.Max(area.Y + gap, area.Y + area.Height - height - gap));
        window.AppWindow.MoveAndResize(new global::Windows.Graphics.RectInt32(left, top, width, height));
    }

    [DllImport("user32.dll", EntryPoint = "GetWindowLongPtrW")] private static extern IntPtr GetWindowLongPtr(IntPtr window, int index);
    [DllImport("user32.dll", EntryPoint = "SetWindowLongPtrW")] private static extern IntPtr SetWindowLongPtr(IntPtr window, int index, IntPtr value);
    [DllImport("user32.dll")] private static extern IntPtr MonitorFromPoint(global::Windows.Graphics.PointInt32 point, uint flags);
    [DllImport("shcore.dll")] private static extern int GetDpiForMonitor(IntPtr monitor, int kind, out uint x, out uint y);
}
