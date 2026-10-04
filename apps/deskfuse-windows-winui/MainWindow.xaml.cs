using System.Text.Json;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace DeskFuse.Windows;

/// <summary>
/// Migration shell: navigation skeleton plus the authenticated service IPC.
/// Page-level behaviour is ported from the WPF client incrementally; this window
/// already proves the WinUI 3 host, the named-pipe client and the snapshot
/// pipeline work under Windows App SDK.
/// </summary>
public sealed partial class MainWindow : Window
{
    private readonly ServiceClient service;
    private bool wasConnected;

    public MainWindow()
    {
        InitializeComponent();
        var appWindow = AppWindow;
        appWindow.Title = "DeskFuse";
        appWindow.Resize(new global::Windows.Graphics.SizeInt32(1200, 780));
        service = new ServiceClient(DispatcherQueue.GetForCurrentThread());
        service.SnapshotChanged += SnapshotChanged;
        service.ConnectionChanged += ConnectionChanged;
        Closed += (_, _) => service.Dispose();
        ConnectionChanged(false, "正在连接 DeskFuseService…");
        service.Start();
    }

    private void NavigationChanged(object sender, SelectionChangedEventArgs args)
    {
        if (ClipboardPage is null) return;
        ClipboardPage.Visibility = Navigation.SelectedIndex == 0 ? Visibility.Visible : Visibility.Collapsed;
        InputPage.Visibility = Navigation.SelectedIndex == 1 ? Visibility.Visible : Visibility.Collapsed;
        DevicesPage.Visibility = Navigation.SelectedIndex == 2 ? Visibility.Visible : Visibility.Collapsed;
        TransfersPage.Visibility = Navigation.SelectedIndex == 3 ? Visibility.Visible : Visibility.Collapsed;
        WorkflowsPage.Visibility = Navigation.SelectedIndex == 4 ? Visibility.Visible : Visibility.Collapsed;
    }

    private void SnapshotChanged(JsonElement snapshot)
    {
        LocalName.Text = snapshot.String("local_name", "这台 Windows");
        LocalStatus.Text = snapshot.Bool("network_running") ? "服务运行中" : "网络未就绪";
        bool clipboard = snapshot.TryGetProperty("capabilities", out var capabilities) && capabilities.Bool("clipboard");
        PageDetail.Text = clipboard
            ? "服务已连接，剪贴板能力可用。迁移中：历史列表、预览与恢复操作即将接入。"
            : "桌面同步尚未就绪；本机历史功能将在迁移完成后接入。";
        string error = snapshot.String("last_error");
        if (error.Length > 0) ShowNotice(error);
    }

    private void ConnectionChanged(bool connected, string message)
    {
        if (!connected)
        {
            LocalStatus.Text = "服务不可用 · 本机历史仍可用";
            PageDetail.Text = "正在等待 DeskFuseService；不会接受新的远端内容。";
            ShowNotice(message);
        }
        else if (!wasConnected)
        {
            ShowNotice("WinUI 3 客户端已连接服务（迁移预览：仅导航与状态，页面功能待接入）。");
        }
        wasConnected = connected;
    }

    private void ShowNotice(string message) => Notice.Text = message;
}
