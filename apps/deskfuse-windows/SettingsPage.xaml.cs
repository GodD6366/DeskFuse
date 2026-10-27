using System.Text.Json;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;

namespace DeskFuse.Windows;

/// <summary>Settings pages mirrored from the macOS SettingsView: one scrolling
/// 720 pt form per category, section label + card + footer rows. Category
/// selection lives in the main sidebar.</summary>
public sealed partial class SettingsPage : UserControl
{
    private readonly Dictionary<string, UIElement> pages = [];
    private readonly DesktopClipboardController clipboard;
    private readonly ServiceClient service;
    private readonly PowerStateController power;
    private readonly UIElement workflows;
    private readonly Action loadWorkflows;
    private readonly Action openInputPage;
    private readonly MainWindow owner;
    private readonly TextBlock serviceStatus = DfUi.Detail("");
    private readonly TextBlock cacheStatus = DfUi.Detail("正在读取缓存…");
    private readonly TextBlock advancedStatus = DfUi.Detail("");
    private readonly TextBlock generalStatus = DfUi.Detail("");
    private readonly TextBox excludedApps = new() { PlaceholderText = "notepad.exe, Code.exe", AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MinHeight = 64 };
    private readonly TextBlock canonicalStatus = DfUi.Detail("");
    private bool canonicalSettingsLoaded;
    private readonly TextBlock updateStatus = DfUi.Detail("检查更新会先读取 GitHub Releases，失败时回退到独立域名镜像。");
    private readonly HyperlinkButton openRelease = new() { Content = "打开发布页", Visibility = Visibility.Collapsed, HorizontalAlignment = HorizontalAlignment.Left };
    private readonly DfToggle sync;
    private readonly DfToggle awake;
    private readonly DfToggle capture;
    private readonly DfToggle startup;
    private readonly DfToggle lockedSession;
    private readonly DfToggle remoteSession;
    private readonly TextBlock sessionPolicyStatus = DfUi.Detail("");
    private readonly StackPanel permissionRows = new() { Spacing = 0 };
    private readonly ComboBox syncDevicePicker = new() { MinWidth = 210, PlaceholderText = "选择设备" };
    private readonly Button syncDeviceButton;
    private readonly TextBlock syncDeviceStatus = DfUi.Detail("");
    private string syncDeviceSignature = "";
    private bool syncingDevice;
    private Grid clipboardModeHost = null!;
    private Grid clipboardHistoryHost = null!;
    private Grid clipboardWorkflowsHost = null!;
    private Button historySegment = null!;
    private Button workflowsSegment = null!;
    private bool workflowsMode;
    private bool updatingAwake;
    private bool updatingStartup;
    private bool updatingSessionPolicy;

    public SettingsPage(DesktopClipboardController clipboard, ServiceClient service, PowerStateController power,
        UIElement workflows, Action loadWorkflows, Action openInputPage, MainWindow owner)
    {
        this.clipboard = clipboard;
        this.service = service;
        this.power = power;
        this.workflows = workflows;
        this.loadWorkflows = loadWorkflows;
        this.openInputPage = openInputPage;
        this.owner = owner;
        InitializeComponent();

        sync = DfUi.Switch(clipboard.SyncEnabled, (_, _) => clipboard.SyncEnabled = sync!.IsOn);
        awake = DfUi.Switch(clipboard.KeepAwake, (_, _) => { if (!updatingAwake) SetKeepAwake(awake!.IsOn); });
        capture = DfUi.Switch(clipboard.CaptureEnabled, (_, _) => clipboard.CaptureEnabled = capture!.IsOn);
        startup = DfUi.Switch(StartupRegistration.IsEnabled, (_, _) => SetStartup());
        lockedSession = DfUi.Switch(false, (_, _) => SetSessionPolicy());
        remoteSession = DfUi.Switch(false, (_, _) => SetSessionPolicy());
        syncDeviceButton = DfUi.Secondary("同步并替换…", async (_, _) => await SyncClipboardFromDeviceAsync());

        pages["general"] = BuildGeneral();
        pages["clipboard"] = BuildClipboard();
        pages["shortcuts"] = BuildShortcuts();
        pages["files"] = BuildFiles();
        pages["permissions"] = BuildPermissions();
        pages["about"] = BuildAbout();
        pages["advanced"] = BuildAdvanced();
        Show("general");
        RefreshSyncDevices();
        if (UpdateChecker.LastResult is { } update) ApplyUpdateResult(update);
    }

    // MARK: - Pages

    private UIElement BuildGeneral()
    {
        var startupContent = new StackPanel
        {
            Children =
            {
                DfUi.SettingRow("登录时自动启动", "登录后在后台启动 DeskFuse；不会请求管理员权限。", startup),
                DfUi.Divider(8, 8),
                DfUi.SettingRow("关闭窗口后继续在托盘运行", "关闭主窗口不会退出后台组件；从托盘菜单可以退出。",
                    DfUi.StatusPill("始终开启", "DFSuccessBrush")),
            },
        };
        var available = new StackPanel
        {
            Children =
            {
                DfUi.SettingRow("保持电脑唤醒", "阻止系统和显示器自动休眠，作为被控端时保持在线；退出 DeskFuse 后自动恢复。", awake),
                generalStatus,
            },
        };
        var state = new StackPanel
        {
            Children =
            {
                DfUi.SettingRow("后台服务", "DeskFuseService 负责网络、剪贴板与文件传输。", serviceStatus),
            },
        };
        var relaxed = new StackPanel
        {
            Children =
            {
                DfUi.SettingRow("锁屏时保持可用", "锁屏期间配对设备仍可同步剪贴板、传输文件和查看状态；键鼠控制在解锁后自动恢复。", lockedSession),
                DfUi.Divider(8, 8),
                DfUi.SettingRow("允许远程桌面会话", "允许在远程桌面（RDP）中使用 DeskFuse；配对、键鼠控制与本页开关仍仅限本机已解锁桌面。", remoteSession),
                sessionPolicyStatus,
            },
        };
        return Form(
            DfUi.Section("启动", startupContent),
            DfUi.Section("保持可用", available),
            DfUi.Section("远程与锁屏", relaxed, "开关保存在本机服务，修改时需要在本机已解锁桌面操作。"),
            DfUi.Section("本机状态", state));
    }

    private void SetStartup()
    {
        if (updatingStartup) return;
        try
        {
            StartupRegistration.SetEnabled(startup.IsOn);
            generalStatus.Text = startup.IsOn ? "已设置为登录时在后台启动。" : "已取消登录时自动启动。";
        }
        catch (Exception ex) when (ex is UnauthorizedAccessException or System.Security.SecurityException or IOException)
        {
            updatingStartup = true;
            try { startup.IsOn = StartupRegistration.IsEnabled; } finally { updatingStartup = false; }
            generalStatus.Text = $"登录启动项保存失败：{ex.Message}";
        }
    }

    private UIElement BuildClipboard()
    {
        clipboardModeHost = new Grid { Margin = new Thickness(24, 0, 24, 12), HorizontalAlignment = HorizontalAlignment.Left };
        var segmentBorder = new Border
        {
            Background = DfUi.Control,
            CornerRadius = new CornerRadius(8),
            Padding = new Thickness(2),
            Child = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 2 },
        };
        historySegment = SegmentButton("历史", true);
        workflowsSegment = SegmentButton("工作流", false);
        ((StackPanel)segmentBorder.Child).Children.Add(historySegment);
        ((StackPanel)segmentBorder.Child).Children.Add(workflowsSegment);
        clipboardModeHost.Children.Add(segmentBorder);
        historySegment.Click += (_, _) => SetClipboardMode(false);
        workflowsSegment.Click += (_, _) => SetClipboardMode(true);

        clipboardHistoryHost = new Grid();
        clipboardHistoryHost.Children.Add(BuildClipboardHistory());
        clipboardWorkflowsHost = new Grid { Visibility = Visibility.Collapsed };
        clipboardWorkflowsHost.Children.Add(workflows);

        var root = new StackPanel { Spacing = 0 };
        root.Children.Add(clipboardModeHost);
        root.Children.Add(clipboardHistoryHost);
        root.Children.Add(clipboardWorkflowsHost);
        return root;
    }

    private UIElement BuildClipboardHistory()
    {
        var history = new StackPanel
        {
            Children =
            {
                DfUi.SettingRow("记录新的剪贴板历史", "停止记录不关闭已开启的同步。历史只属于当前用户。", capture),
                DfUi.Divider(8, 8),
                HistoryRetentionRow(),
            },
        };
        var syncRow = new StackPanel
        {
            Children =
            {
                DfUi.SettingRow("自动同步剪贴板", "同步之后的新复制内容；开启时不会重新发送历史。", sync),
            },
        };
        var search = new StackPanel
        {
            Children =
            {
                DfUi.SettingRow("同步标题与识别文字", "跨设备只同步标题与元数据；图片和文件正文按需获取。",
                    DfUi.StatusPill("始终开启", "DFSuccessBrush")),
                DfUi.Divider(8, 8),
                DfUi.SettingRow("已同步的远端索引", "只删除远端搜索记录，不删除本机剪贴板历史。",
                    DfUi.Secondary("清除…", async (_, _) => await ClearRemoteIndexAsync())),
                DfUi.Divider(8, 8),
                DfUi.SettingRow("排除应用", "每行一个应用进程名；这些应用的新复制不会写入历史或搜索索引。",
                    DfUi.Secondary("保存", async (_, _) => await SaveExcludedAppsAsync())),
                excludedApps,
                canonicalStatus,
            },
        };
        var deviceControls = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        deviceControls.Children.Add(syncDevicePicker);
        deviceControls.Children.Add(syncDeviceButton);
        var deviceSync = new StackPanel
        {
            Children =
            {
                DfUi.SettingRow("从设备同步剪贴板", "选择一台已连接的已配对设备，以它的历史替换本机剪贴板。", deviceControls),
                syncDeviceStatus,
            },
        };
        return Form(
            DfUi.Section("普通历史", history, "默认保留 30 天，不限制条数；到期记录会在后台清理。"),
            DfUi.Section("跨设备搜索与排除", search),
            DfUi.Section("同步", syncRow),
            DfUi.Section("从设备同步", deviceSync,
                "先完整拉取并校验所选设备的历史，成功后才原子替换本机历史，并把最新一条写入系统剪贴板。失败时本机现有历史保持不变。文件记录只保留在内容库，不导入本机历史。"),
            DfUi.Detail("本页修改立即生效；剪贴板历史、同步标题与排除应用属于本机配置。"));
    }

    private void RefreshSyncDevices()
    {
        if (syncDevicePicker is null) return;
        var peers = (service.LatestSnapshot?.Array("peers") ?? Enumerable.Empty<JsonElement>())
            .Where(peer => peer.String("trust") == "trusted" && peer.String("connection") == "connected")
            .Select(peer => (Id: peer.String("peer_id"), Name: peer.String("display_name", "未命名设备")))
            .Where(peer => peer.Id.Length > 0)
            .ToArray();
        string signature = string.Join("\n", peers.Select(peer => $"{peer.Id} {peer.Name}"));
        string? selected = (syncDevicePicker.SelectedItem as ComboBoxItem)?.Tag as string;
        if (signature != syncDeviceSignature)
        {
            syncDeviceSignature = signature;
            syncDevicePicker.Items.Clear();
            foreach (var peer in peers)
                syncDevicePicker.Items.Add(new ComboBoxItem { Content = peer.Name, Tag = peer.Id });
            int index = selected is null ? -1 : Array.FindIndex(peers, peer => peer.Id == selected);
            syncDevicePicker.SelectedIndex = index >= 0 ? index : peers.Length > 0 ? 0 : -1;
        }
        syncDevicePicker.IsEnabled = peers.Length > 0 && !syncingDevice;
        syncDeviceButton.IsEnabled = peers.Length > 0 && !syncingDevice;
        if (peers.Length == 0 && !syncingDevice) syncDeviceStatus.Text = "没有已连接的已配对设备。";
        else if (!syncingDevice && syncDeviceStatus.Text == "没有已连接的已配对设备。") syncDeviceStatus.Text = "";
    }

    private async Task SyncClipboardFromDeviceAsync()
    {
        if (syncingDevice) return;
        if (syncDevicePicker.SelectedItem is not ComboBoxItem { Tag: string deviceId } item
            || string.IsNullOrEmpty(deviceId))
        {
            syncDeviceStatus.Text = "请先选择一台已连接的设备。";
            return;
        }
        string deviceName = item.Content?.ToString() ?? "设备";
        if (!Dialogs.Confirm(owner.Handle,
                $"用「{deviceName}」的剪贴板历史替换本机剪贴板？\n\n"
                + "本机现有剪贴板历史会被删除，当前系统剪贴板也会被这台设备的最新一条内容覆盖。\n"
                + "文件记录不会导入本机历史。内容库和其他设备的数据不受影响。",
                "同步并替换剪贴板")) return;
        syncingDevice = true;
        syncDevicePicker.IsEnabled = false;
        syncDeviceButton.IsEnabled = false;
        syncDeviceStatus.Text = $"正在读取「{deviceName}」的历史索引…";
        try
        {
            var items = await LoadDeviceItemsAsync(deviceId, deviceName);
            if (items.Count == 0) throw new InvalidOperationException("这台设备还没有可导入的剪贴板历史。");
            syncDeviceStatus.Text = $"已读取 {items.Count} 条索引，正在校验内容…";
            var adopted = new List<ClipboardEntry>(items.Count);
            ClipboardEntry? newest = null;
            int skipped = 0;
            long bytes = 0;
            const long byteBudget = 256L * 1024 * 1024;
            for (int index = 0; index < items.Count; index++)
            {
                var source = items[index];
                if (!source.Available || source.FileCount > 0)
                {
                    skipped++;
                    continue;
                }
                // A payload failure aborts the whole import: replacing history
                // with a silently truncated subset would be worse than retrying.
                var content = await clipboard.Canonical.ContentAsync(source.Id);
                if (content.Files is { Count: > 0 }) { skipped++; continue; }
                long size = content.Data.LongLength + (content.Representations?.Sum(rep => (long)rep.Data.Length) ?? 0);
                if (bytes + size > byteBudget)
                {
                    skipped += items.Count - index;
                    syncDeviceStatus.Text = "历史内容超过 256 MiB，已导入较新的记录。";
                    break;
                }
                bytes += size;
                var mapped = CanonicalRow.ToEntry(source);
                var entry = new ClipboardEntry($"adopted-{source.Id}", content, deviceId, deviceName,
                    mapped.CopyTimeKnown ? mapped.CreatedAt : DateTimeOffset.Now)
                {
                    CustomTitle = string.IsNullOrWhiteSpace(source.Title) ? null : source.Title.Trim(),
                    SourceApp = string.IsNullOrWhiteSpace(source.SourceAppName) ? null : source.SourceAppName,
                    CopyTimeKnown = mapped.CopyTimeKnown,
                };
                adopted.Add(entry);
                newest ??= entry;
                if (index % 20 == 19)
                    syncDeviceStatus.Text = $"正在校验内容：{index + 1}/{items.Count}…";
            }
            if (adopted.Count == 0) throw new InvalidOperationException("这台设备的历史正文暂时不可用，本机历史未改变。");
            syncDeviceStatus.Text = $"正在原子替换本机历史（{adopted.Count} 条）…";
            var replaced = await clipboard.History.ReplaceAllAsync(adopted);
            string restoreError = "";
            if (newest is not null)
            {
                try { clipboard.Restore(newest); }
                catch (Exception ex) { restoreError = $"；写入系统剪贴板失败：{ex.Message}"; }
            }
            owner.Notify(skipped > 0
                ? $"已用「{deviceName}」的 {replaced.Count} 条历史替换本机剪贴板，跳过 {skipped} 条不可用记录{restoreError}。"
                : $"已用「{deviceName}」的 {replaced.Count} 条历史替换本机剪贴板{restoreError}。");
            owner.ShowDeviceHistory(deviceId, deviceName);
        }
        catch (Exception ex) when (ex is IOException or InvalidOperationException or TimeoutException
            or OperationCanceledException or JsonException)
        {
            syncDeviceStatus.Text = $"同步失败，本机历史未改变：{ex.Message}";
        }
        finally
        {
            syncingDevice = false;
            RefreshSyncDevices();
        }
    }

    private async Task<List<CanonicalItem>> LoadDeviceItemsAsync(string deviceId, string deviceName)
    {
        var items = new List<CanonicalItem>();
        foreach (string source in new[] { deviceId, deviceName }.Distinct(StringComparer.Ordinal))
        {
            items.Clear();
            long offset = 0;
            while (true)
            {
                var page = await clipboard.QueryCanonicalAsync("", null, "all", source, (int)Math.Min(offset, int.MaxValue), 100);
                items.AddRange(page.Items);
                if (page.NextOffset is not long next || next <= offset || items.Count >= 2000) break;
                offset = next;
            }
            if (items.Count > 0) return items;
        }
        return items;
    }

    private async Task LoadCanonicalSettingsAsync()
    {
        if (canonicalSettingsLoaded || !service.IsConnected) return;
        try
        {
            string json = await clipboard.Canonical.QueryAsync(JsonSerializer.Serialize(new { op = "state" }));
            using var state = JsonDocument.Parse(json);
            var apps = state.RootElement.GetProperty("settings").GetProperty("excluded_apps").EnumerateArray()
                .Select(app => app.GetProperty("bundle_id").GetString() ?? "").Where(id => id.Length > 0);
            excludedApps.Text = string.Join(Environment.NewLine, apps);
            canonicalSettingsLoaded = true;
            canonicalStatus.Text = "";
        }
        catch (Exception ex) when (ex is IOException or JsonException) { canonicalStatus.Text = $"内容库设置读取失败：{ex.Message}"; }
    }

    private async Task SaveExcludedAppsAsync()
    {
        try
        {
            var apps = excludedApps.Text.Split([',', '\r', '\n'], StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries)
                .Where(name => name.Length <= 200).Distinct(StringComparer.OrdinalIgnoreCase).Take(128)
                .Select(name => new { bundle_id = name, name }).ToArray();
            await clipboard.Canonical.MutateAsync(JsonSerializer.Serialize(new { op = "settings", settings = new { excluded_apps = apps } }));
            canonicalStatus.Text = "排除应用已保存；仅影响之后的新复制。";
        }
        catch (Exception ex) when (ex is IOException or JsonException) { canonicalStatus.Text = $"排除应用保存失败：{ex.Message}"; }
    }

    private async Task ClearRemoteIndexAsync()
    {
        try
        {
            await clipboard.Canonical.MutateAsync(JsonSerializer.Serialize(new { op = "clear_remote_index" }));
            canonicalStatus.Text = "已清除远端索引；本机历史未改变。";
        }
        catch (Exception ex) when (ex is IOException or JsonException) { canonicalStatus.Text = $"远端索引清除失败：{ex.Message}"; }
    }

    private UIElement BuildShortcuts()
    {
        var draft = owner.CurrentClipboardShortcut;
        var chord = DfUi.Text(draft.Label, 14, true);
        var enabled = new CheckBox { Content = "启用全局快捷键", IsChecked = draft.Enabled };
        var feedback = DfUi.Detail("默认 Ctrl+Shift+V。双击、回车可粘贴回打开前的应用。");
        var record = DfUi.Secondary("录制…", (_, _) =>
        {
            feedback.Text = "请按新的组合键；Esc 取消。";
            owner.BeginClipboardShortcutRecording(value =>
            {
                if (value is null) { feedback.Text = "已取消录制，原快捷键未改变。"; return; }
                if (!value.IsValid) { feedback.Text = "请使用包含 Ctrl、Alt 或 Win 的组合键。"; return; }
                draft = value;
                enabled.IsChecked = true;
                chord.Text = value.Label;
                feedback.Text = "已录制；点击保存后生效。";
            });
        });
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(record, "clipboard-shortcut-record");
        var save = DfUi.Secondary("保存", (_, _) =>
        {
            owner.StopClipboardShortcutRecording();
            try
            {
                var next = draft with { Enabled = enabled.IsChecked == true };
                owner.SaveClipboardShortcut(next);
                draft = next; chord.Text = next.Label;
                feedback.Text = "已保存，立即生效；重启后保留。";
            }
            catch (Exception ex) { feedback.Text = ex.Message; }
        });
        var reset = DfUi.Plain("恢复默认", (_, _) =>
        {
            owner.StopClipboardShortcutRecording();
            draft = ClipboardHotkey.Default;
            enabled.IsChecked = true; chord.Text = draft.Label;
            feedback.Text = "默认组合为 Ctrl+Shift+V；点击保存后生效。";
        });
        var controls = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        controls.Children.Add(record); controls.Children.Add(save); controls.Children.Add(reset);
        var clipboardShortcut = new StackPanel { Spacing = 12 };
        clipboardShortcut.Children.Add(chord); clipboardShortcut.Children.Add(enabled);
        clipboardShortcut.Children.Add(controls); clipboardShortcut.Children.Add(feedback);
        var input = new StackPanel
        {
            Children =
            {
                DfUi.SettingRow("鼠标共享", "查看当前主控角色与屏幕排布。", DfUi.Secondary("打开键鼠共享", (_, _) => openInputPage())),
                DfUi.Divider(8, 8),
                DfUi.SettingRow("紧急收回", "自定义按键之外，此组合始终可用。", DfUi.StatusPill("Ctrl + Alt + Win + Esc", "DFMutedBrush")),
            },
        };
        var workflowsInfo = new StackPanel
        {
            Children =
            {
                DfUi.SettingRow("工作流快捷键", "在设置 → 剪贴板 → 工作流中为每个工作流分配快捷键。",
                    DfUi.Secondary("管理工作流", (_, _) => Show("workflows"))),
            },
        };
        var reserved = new StackPanel
        {
            Children =
            {
                DfUi.SettingRow("文件暂存区", "打开或收起暂存区。", DfUi.StatusPill("Ctrl + Shift + D", "DFMutedBrush")),
            },
        };
        return Form(
            DfUi.Section("剪贴板面板", clipboardShortcut, "只影响这台 Windows 的唤起按键，不修改 Mac 的快捷键。"),
            DfUi.Section("键鼠控制", input, "方向切换与锁定遵循键鼠共享状态。"),
            DfUi.Section("工作流", workflowsInfo, "停用的流程不会响应快捷键。"),
            DfUi.Section("固定快捷键", reserved, "这些组合键由 DeskFuse 保留，不能分配给工作流。"));
    }

    private UIElement BuildFiles()
    {
        var receive = new StackPanel
        {
            Children =
            {
                DfUi.SettingRow("接收位置", MainWindow.DefaultReceiveFolder(),
                    DfUi.Secondary("打开文件夹", (_, _) => OpenFolder(MainWindow.DefaultReceiveFolder()))),
            },
        };
        var cache = new StackPanel
        {
            Children =
            {
                DfUi.SettingRow("已使用", "服务为正在接收、粘贴和导出的内容保留缓存。", cacheStatus),
                DfUi.Divider(8, 8),
                DfUi.SettingRow("历史传输文件", "保护正在接收、粘贴和导出的内容，不删除用户源文件或已粘贴副本。",
                    DfUi.Secondary("清除…", (_, _) => advancedStatus.Text = "Windows 端缓存由服务自动回收。")),
            },
        };
        return Form(
            DfUi.Section("接收位置", receive, "从暂存区发送到此设备的文件会直接保存到该位置。"),
            DfUi.Section("缓存", cache));
    }

    private UIElement BuildPermissions()
    {
        RenderPermissions(service.LatestSnapshot);
        return Form(
            DfUi.Section("系统权限", permissionRows, "状态来自当前服务与实际 Windows 会话；安全桌面始终不可注入。"));
    }

    private void RenderPermissions(JsonElement? snapshot)
    {
        var capabilities = snapshot is JsonElement value && value.TryGetProperty("capabilities", out var current) ? current : default;
        bool ordinary = capabilities.ValueKind == JsonValueKind.Object && capabilities.Bool("input");
        bool elevated = capabilities.ValueKind == JsonValueKind.Object && capabilities.Bool("elevated_input");
        permissionRows.Children.Clear();
        permissionRows.Children.Add(PermissionRow("\uE7E8", "普通桌面输入", ordinary ? "当前用户桌面与输入 helper 已就绪。" : "等待本机已解锁桌面与输入 helper 就绪。", ordinary ? "可用" : "未就绪", ordinary ? "DFSuccessBrush" : "DFMutedBrush"));
        permissionRows.Children.Add(DfUi.Divider(0, 0));
        permissionRows.Children.Add(PermissionRow("\uE72E", "提升窗口控制", elevated ? "UIAccess helper 的签名、路径和令牌已通过服务校验。" : "需要签名并安装到受信任位置的 UIAccess 版本。", elevated ? "可用" : "未授权", elevated ? "DFSuccessBrush" : "DFMutedBrush"));
        permissionRows.Children.Add(DfUi.Divider(0, 0));
        permissionRows.Children.Add(PermissionRow("\uE8D7", "登录、UAC 与锁屏桌面", "这些安全桌面始终不可注入。", "不可用", "DFDangerBrush"));
    }

    private UIElement BuildAbout()
    {
        var brand = new Grid { ColumnSpacing = 14, Padding = new Thickness(20) };
        brand.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        brand.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        var brandIcon = new Grid { Width = 48, Height = 48 };
        brandIcon.Children.Add(DfUi.Glyph("\uE8A7", 24, DfUi.Accent));
        var brandImage = new Image { Stretch = Stretch.Uniform };
        BrandMark.Apply(brandImage, this);
        brandIcon.Children.Add(brandImage);
        brand.Children.Add(new Border
        {
            Width = 48, Height = 48, CornerRadius = new CornerRadius(12),
            Background = DfUi.Brush("DFAccentSubtleBrush"),
            Child = brandIcon,
        });
        var brandText = new StackPanel { Spacing = 4, VerticalAlignment = VerticalAlignment.Center };
        brandText.Children.Add(DfUi.Text("DeskFuse", 20, true));
        brandText.Children.Add(DfUi.Detail("一套键鼠、一个剪贴板，在多台电脑之间无缝工作。"));
        Grid.SetColumn(brandText, 1);
        brand.Children.Add(brandText);

        var check = DfUi.Secondary("检查更新", async (_, _) => await CheckUpdateAsync());
        var update = new StackPanel
        {
            Children =
            {
                DfUi.SettingRow($"DeskFuse {AppVersion.Display}", "Windows 安装包与自动安装更新尚未随发行提供。", check),
                updateStatus,
                openRelease,
            },
        };
        var export = DfUi.Secondary("导出…", async (_, _) => await ExportDiagnosticsAsync());
        var feedback = new StackPanel
        {
            Children =
            {
                DfUi.SettingRow("导出诊断信息", "遇到连接、同步或键鼠问题时，导出给开发者分析；不包含剪贴板正文。", export),
            },
        };
        var report = DfUi.Secondary("反馈问题…", (_, _) =>
            SupportLinks.ReportProblem(service.LatestSnapshot, clipboard.Entries.Count,
                clipboard.HistoryRetentionDays, clipboard.StorageError));
        var known = DfUi.Secondary("已报告的问题", (_, _) => SupportLinks.Open(SupportLinks.IssuesUrl));
        var feedbackActions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, Children = { report, known } };
        feedback.Children.Add(DfUi.Divider(8, 8));
        feedback.Children.Add(DfUi.SettingRow("联系开发者", "反馈页会预填版本、系统和服务状态，并自动导出脱敏诊断；在资源管理器中把选中的文件拖入 issue 即可。", feedbackActions));
        return Form(
            new Border { Child = brand },
            DfUi.Section("更新", update, "DeskFuse 每 12 小时自动检查一次更新。"),
            DfUi.Section("反馈问题", feedback));
    }

    private UIElement BuildAdvanced()
    {
        var features = new StackPanel
        {
            Children =
            {
                DfUi.SettingRow("启用工作流", "在剪贴板里串联多个动作并绑定全局快捷键；由服务端管理。",
                    DfUi.StatusPill("由服务管理", "DFMutedBrush")),
            },
        };
        var clear = DfUi.Secondary("清空…", async (_, _) => await ClearHistoryAsync());
        clear.Foreground = DfUi.Danger;
        var clipboardSettings = new StackPanel
        {
            Children =
            {
                DfUi.SettingRow("跨设备索引与排除 App", "同步标题与识别文字、管理不保存历史的应用。",
                    DfUi.Secondary("打开剪贴板设置", (_, _) => Show("clipboard"))),
                DfUi.Divider(8, 8),
                DfUi.SettingRow("清空剪贴板历史", "删除本机保存的全部剪贴板历史。", clear),
            },
        };
        var export = DfUi.Secondary("导出…", async (_, _) => await ExportDiagnosticsAsync());
        var diagnostics = new StackPanel
        {
            Children =
            {
                DfUi.SettingRow("导出诊断信息", "附带服务状态、客户端内存与历史统计；不包含剪贴板内容。", export),
                DfUi.Divider(8, 8),
                DfUi.SettingRow("更新通道", "Windows 桌面安装", DfUi.StatusPill("手动安装", "DFMutedBrush")),
                DfUi.Divider(8, 8),
                DfUi.SettingRow("构建号", "客户端版本", DfUi.StatusPill(AppVersion.Display, "DFMutedBrush")),
                advancedStatus,
            },
        };
        // 与 macOS 设置 › 高级一致：重走初次设置 + 本机数据文件夹入口（Windows 端无日志文件）。
        var debug = new StackPanel
        {
            Children =
            {
                DfUi.SettingRow("重走初次设置", "重新显示权限说明与功能选择引导。",
                    DfUi.Secondary("开始", (_, _) => (Application.Current as App)?.RestartOnboarding())),
                DfUi.Divider(8, 8),
                DfUi.SettingRow("数据文件夹", "剪贴板历史与客户端设置保存在此。",
                    DfUi.Secondary("在资源管理器中显示", (_, _) => OpenDataFolder())),
            },
        };
        return Form(
            DfUi.Section("功能", features, "工作流属于进阶功能；关闭后所有入口隐藏，已注册的全局快捷键也会停用。"),
            DfUi.Section("剪贴板", clipboardSettings),
            DfUi.Section("诊断", diagnostics),
            DfUi.Section("调试", debug, "重走初次设置会沿用当前的功能开关与权限，不会清除任何数据。"));
    }

    private static void OpenDataFolder()
    {
        string directory = System.IO.Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "DeskFuse");
        System.IO.Directory.CreateDirectory(directory);
        System.Diagnostics.Process.Start(new System.Diagnostics.ProcessStartInfo("explorer.exe", directory) { UseShellExecute = false });
    }

    private Button SegmentButton(string text, bool selected)
    {
        return new Button
        {
            Content = new TextBlock { Text = text, FontSize = 13, FontWeight = selected ? Microsoft.UI.Text.FontWeights.SemiBold : Microsoft.UI.Text.FontWeights.Normal },
            Background = selected ? DfUi.Surface : new SolidColorBrush(Microsoft.UI.Colors.Transparent),
            Foreground = DfUi.PrimaryText,
            BorderThickness = new Thickness(0),
            CornerRadius = new CornerRadius(6),
            Padding = new Thickness(18, 0, 18, 0),
            MinHeight = 26,
            MinWidth = 0,
            Template = (ControlTemplate)Application.Current.Resources["DFFlatButtonTemplate"],
        };
    }

    private void SetClipboardMode(bool showWorkflows)
    {
        workflowsMode = showWorkflows;
        clipboardHistoryHost.Visibility = showWorkflows ? Visibility.Collapsed : Visibility.Visible;
        clipboardWorkflowsHost.Visibility = showWorkflows ? Visibility.Visible : Visibility.Collapsed;
        StyleSegment(historySegment, !showWorkflows);
        StyleSegment(workflowsSegment, showWorkflows);
        if (showWorkflows) loadWorkflows();
    }

    private static void StyleSegment(Button button, bool selected)
    {
        button.Background = selected ? DfUi.Surface : new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        if (button.Content is TextBlock text)
            text.FontWeight = selected ? Microsoft.UI.Text.FontWeights.SemiBold : Microsoft.UI.Text.FontWeights.Normal;
    }

    private UIElement HistoryRetentionRow()
    {
        var days = new NumberBox
        {
            Value = clipboard.HistoryRetentionDays, Minimum = 1, Maximum = int.MaxValue,
            SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Compact,
            ValidationMode = NumberBoxValidationMode.Disabled, Width = 96,
        };
        var message = DfUi.Detail("");
        var save = DfUi.Primary("保存");
        var revert = DfUi.Secondary("撤销修改");
        save.IsEnabled = false;
        revert.IsEnabled = false;
        bool saving = false;
        void RefreshActions()
        {
            bool dirty = days.Value != clipboard.HistoryRetentionDays;
            save.IsEnabled = !saving && dirty && double.IsFinite(days.Value) && days.Value >= 1
                && days.Value <= int.MaxValue && days.Value == Math.Truncate(days.Value);
            revert.IsEnabled = !saving && dirty;
            days.IsEnabled = !saving;
        }
        days.ValueChanged += (_, _) => { message.Text = ""; RefreshActions(); };
        revert.Click += (_, _) => { days.Value = clipboard.HistoryRetentionDays; message.Text = ""; RefreshActions(); };
        save.Click += async (_, _) =>
        {
            if (!save.IsEnabled) return;
            int submitted = (int)days.Value;
            saving = true;
            RefreshActions();
            try
            {
                if (submitted < clipboard.HistoryRetentionDays)
                {
                    var confirm = new ContentDialog
                    {
                        XamlRoot = XamlRoot, Title = "缩短历史保留时间？",
                        Content = $"保存后将清理超过 {submitted} 天的历史。用户源文件和已粘贴文件会保留。",
                        PrimaryButtonText = "保存并清理", CloseButtonText = "取消", DefaultButton = ContentDialogButton.Close,
                    };
                    if (await confirm.ShowAsync() != ContentDialogResult.Primary) return;
                }
                await clipboard.SetHistoryRetentionDaysAsync(submitted);
                message.Text = "已保存";
            }
            catch (IOException ex) { message.Text = ex.Message; }
            finally { saving = false; RefreshActions(); }
        };
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        actions.Children.Add(revert);
        actions.Children.Add(save);
        return new StackPanel
        {
            Spacing = 8,
            Children =
            {
                DfUi.SettingRow("保留天数", "默认保留 30 天；手动列表成员长期保留。", days),
                message,
                actions,
            },
        };
    }

    private static UIElement PermissionRow(string glyph, string title, string detail, string state, string tone)
    {
        var grid = new Grid { ColumnSpacing = 12, Padding = new Thickness(14, 10, 14, 10) };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(28) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        grid.Children.Add(DfUi.Glyph(glyph, 16, DfUi.Muted));
        var text = new StackPanel { Spacing = 2 };
        text.Children.Add(DfUi.Text(title, 13, true));
        text.Children.Add(DfUi.Detail(detail));
        Grid.SetColumn(text, 1);
        grid.Children.Add(text);
        var pill = DfUi.StatusPill(state, tone);
        Grid.SetColumn(pill, 2);
        grid.Children.Add(pill);
        return grid;
    }

    private static UIElement Form(params UIElement[] sections)
    {
        var content = new StackPanel { Spacing = 24 };
        foreach (var section in sections) content.Children.Add(section);
        return DfUi.Page(content);
    }

    // MARK: - Actions

    public void Show(string category = "general")
    {
        owner.StopClipboardShortcutRecording();
        _ = LoadCanonicalSettingsAsync();
        if (service.LatestSnapshot is JsonElement snapshot) RefreshService(snapshot);
        sync.IsOn = clipboard.SyncEnabled;
        capture.IsOn = clipboard.CaptureEnabled;
        if (category == "workflows")
        {
            category = "clipboard";
            workflowsMode = true;
        }
        if (!pages.TryGetValue(category, out var page)) page = pages["general"];
        PageHost.Children.Clear();
        PageHost.Children.Add(page);
        if (category == "clipboard") SetClipboardMode(workflowsMode);
        else workflowsMode = false;
    }

    public void RefreshService(JsonElement snapshot)
    {
        serviceStatus.Text = $"服务版本 {snapshot.String("package_version")} · {(snapshot.Bool("network_running") ? "网络运行中" : "网络未就绪")}";
        if (snapshot.TryGetProperty("file_cache", out var cache))
            cacheStatus.Text = $"{MainWindow.FormatBytes(cache.Number("used_bytes"))} / {MainWindow.FormatBytes(cache.Number("max_bytes"))}";
        sync.IsOn = clipboard.SyncEnabled;
        capture.IsOn = clipboard.CaptureEnabled;
        RestoreSessionPolicySwitches(snapshot);
        RenderPermissions(snapshot);
        RefreshSyncDevices();
    }

    private void RestoreSessionPolicySwitches(JsonElement? snapshot)
    {
        bool allowLocked = false, allowRemote = false, fullPresence = true;
        if (snapshot is JsonElement value)
        {
            fullPresence = value.Bool("session_full_presence");
            if (value.TryGetProperty("session_policy", out var policy))
            {
                allowLocked = policy.Bool("allow_locked");
                allowRemote = policy.Bool("allow_remote");
            }
        }
        updatingSessionPolicy = true;
        try
        {
            lockedSession.IsOn = allowLocked;
            remoteSession.IsOn = allowRemote;
            lockedSession.IsEnabled = fullPresence;
            remoteSession.IsEnabled = fullPresence;
        }
        finally { updatingSessionPolicy = false; }
        sessionPolicyStatus.Text = fullPresence
            ? ""
            : "当前处于锁屏或远程桌面会话：剪贴板、文件与状态可用；修改开关请回到本机已解锁桌面。";
    }

    private async void SetSessionPolicy()
    {
        if (updatingSessionPolicy) return;
        try
        {
            await service.SetSessionPolicyAsync(lockedSession.IsOn, remoteSession.IsOn);
            sessionPolicyStatus.Text = "";
        }
        catch (Exception ex)
        {
            RestoreSessionPolicySwitches(service.LatestSnapshot);
            sessionPolicyStatus.Text = ex.Message;
        }
    }

    private void SetKeepAwake(bool enabled)
    {
        if (!clipboard.SetKeepAwake(enabled))
        {
            updatingAwake = true;
            try { awake.IsOn = clipboard.KeepAwake; } finally { updatingAwake = false; }
            generalStatus.Text = "保持唤醒设置无法保存，请检查当前用户目录权限。";
            return;
        }
        if (!power.SetEnabled(enabled))
        {
            clipboard.SetKeepAwake(!enabled);
            updatingAwake = true;
            try { awake.IsOn = clipboard.KeepAwake; } finally { updatingAwake = false; }
            generalStatus.Text = "Windows 未接受保持唤醒请求，设置未启用。";
            return;
        }
        generalStatus.Text = enabled ? "已阻止系统自动休眠和显示器休眠。" : "已恢复系统默认休眠行为。";
    }

    private async Task CheckUpdateAsync()
    {
        updateStatus.Text = "正在检查更新…";
        openRelease.Visibility = Visibility.Collapsed;
        try
        {
            Version? current = Version.TryParse(AppVersion.Display.Split(' ')[0], out var parsed) ? parsed : null;
            if (current is null) { updateStatus.Text = "无法读取当前版本号。"; return; }
            var result = await UpdateChecker.CheckAsync(current);
            ApplyUpdateResult(result);
        }
        catch (Exception) { updateStatus.Text = "更新检查未完成，请检查网络后重试。"; }
    }

    internal void ApplyUpdateResult(UpdateChecker.Result result)
    {
        updateStatus.Text = result.Summary;
        openRelease.Visibility = Visibility.Collapsed;
        if (result.ReleaseUrl is string url && url.Length > 0)
        {
            openRelease.NavigateUri = new Uri(url);
            openRelease.Visibility = Visibility.Visible;
        }
    }

    private async Task ExportDiagnosticsAsync()
    {
        try
        {
            await Task.Run(() =>
            {
                string path = DiagnosticsExport.Export(service.LatestSnapshot, clipboard.Entries.Count, clipboard.HistoryRetentionDays, clipboard.StorageError);
                var start = new System.Diagnostics.ProcessStartInfo("explorer.exe") { UseShellExecute = false };
                start.ArgumentList.Add("/select,");
                start.ArgumentList.Add(path);
                System.Diagnostics.Process.Start(start);
            });
            updateStatus.Text = "诊断文件已写入本地诊断目录；其中不含剪贴板正文、设备信任材料或文件内容。";
        }
        catch (Exception) { updateStatus.Text = "诊断导出失败，请检查用户存储目录权限。"; }
    }

    private void OpenFolder(string path)
    {
        try
        {
            Directory.CreateDirectory(path);
            System.Diagnostics.Process.Start(new System.Diagnostics.ProcessStartInfo("explorer.exe") { UseShellExecute = true, Arguments = path });
        }
        catch (Exception) { advancedStatus.Text = "无法打开文件夹。"; }
    }

    private async Task ClearHistoryAsync()
    {
        var dialog = new ContentDialog
        {
            XamlRoot = XamlRoot, Title = "清空所有本机剪贴板历史？",
            Content = "本机与内容库中的历史记录都会被删除，手动列表成员也会清空。此操作无法撤销，用户源文件和已粘贴文件会保留。",
            PrimaryButtonText = "清空历史", CloseButtonText = "取消", DefaultButton = ContentDialogButton.Close,
        };
        if (await dialog.ShowAsync() != ContentDialogResult.Primary) return;
        try
        {
            clipboard.Clear();
            if (clipboard.CanonicalConnected) await clipboard.ClearCanonicalHistoryAsync();
            advancedStatus.Text = "剪贴板历史已清空。";
        }
        catch (Exception ex) when (ex is IOException or InvalidOperationException or OperationCanceledException)
        { advancedStatus.Text = $"内容库历史未清空：{ex.Message}"; }
        catch (Exception) { advancedStatus.Text = "历史清理未完成，请检查用户存储目录权限。"; }
    }
}
