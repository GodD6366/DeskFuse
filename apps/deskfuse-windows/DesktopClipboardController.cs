using System.Collections.ObjectModel;
using System.IO;
using System.Text.Json;
using System.Windows;
using Microsoft.Win32;

namespace DeskFuse.Windows;

// Local history belongs to the user process; transport and identity belong only
// to the authenticated service. Neither reconnect nor startup republishes history.
public sealed class DesktopClipboardController : IDisposable
{
    private readonly NativeClipboard native = new();
    private readonly ClipboardHistory history = new();
    private readonly ServiceClient service;
    private ShelfWindow? shelf;
    private bool paused;
    private bool disposed;
    private long publishSequence;
    private uint? pollNativeSequence;
    private uint observedNativeSequence;
    private long nativeRevision;
    private long? generation;
    private bool captureEnabled = true;
    private bool syncEnabled;
    private string? preferencesError;
    private readonly string preferencesFile = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "DeskFuse", "desktop", "settings-v1.json");
    public bool SyncEnabled
    {
        get => syncEnabled;
        set { if (syncEnabled == value) return; syncEnabled = value; SavePreferences(); }
    }
    public bool CaptureEnabled
    {
        get => captureEnabled;
        set { if (captureEnabled == value) return; captureEnabled = value; SavePreferences(); }
    }
    public ObservableCollection<ClipboardEntry> Entries => history.Entries;
    public event Action<string>? Error;
    public DesktopClipboardController(ServiceClient service)
    {
        this.service = service;
        LoadPreferences();
        native.Changed += LocalChanged;
        native.LocalInvalidated += ObserveNativeChange;
        native.Error += message => { ObserveNativeChange(); Error?.Invoke(message); };
        service.ClipboardReceived += RemoteReceived;
        service.PollStarting += PollStarting;
        service.SnapshotChanged += Snapshot;
        service.ConnectionChanged += Connection;
        SystemEvents.SessionSwitch += SessionChanged;
    }
    public void Start(Window window)
    {
        try { native.Start(window); observedNativeSequence = native.SequenceNumber; if (history.StorageError is string error) Error?.Invoke(error); if (preferencesError is string warning) Error?.Invoke(warning); }
        catch (Exception) { Error?.Invoke("无法启动 Windows 剪贴板监听。请重新打开客户端。"); }
    }
    private sealed record Preferences(bool CaptureEnabled = true, bool SyncEnabled = false);
    private void LoadPreferences()
    {
        try
        {
            if (!File.Exists(preferencesFile)) return;
            if (new FileInfo(preferencesFile).Length > 8192) throw new InvalidDataException();
            var preferences = JsonSerializer.Deserialize<Preferences>(File.ReadAllText(preferencesFile)) ?? throw new InvalidDataException();
            captureEnabled = preferences.CaptureEnabled;
            syncEnabled = preferences.SyncEnabled;
        }
        catch (Exception) { preferencesError = "无法读取本机偏好；本次使用默认设置，原文件未改变。"; }
    }
    private void SavePreferences()
    {
        string temporary = preferencesFile + ".tmp";
        try
        {
            Directory.CreateDirectory(Path.GetDirectoryName(preferencesFile)!);
            File.WriteAllText(temporary, JsonSerializer.Serialize(new Preferences(captureEnabled, syncEnabled)));
            File.Move(temporary, preferencesFile, overwrite: true);
            preferencesError = null;
        }
        catch (Exception)
        {
            preferencesError = "设置仅在本次会话生效，无法保存到当前用户目录。";
            Error?.Invoke(preferencesError);
            try { if (File.Exists(temporary)) File.Delete(temporary); } catch (Exception) { }
        }
    }
    private void PollStarting()
    {
        // Includes unsupported and over-limit native formats that have no
        // decoded Changed payload. Invalidation is queued before the Poll.
        ObserveNativeChange();
        pollNativeSequence = native.SequenceNumber;
    }
    private void ObserveNativeChange()
    {
        var ownership = native.TryAcknowledgeOwnWrite(out uint ownSequence);
        if (ownership == true) { observedNativeSequence = ownSequence; return; }
        if (ownership is null) return; // Busy is unknown, not proof of a new local copy.
        uint current = native.SequenceNumber;
        if (current == observedNativeSequence || paused || disposed) return;
        observedNativeSequence = current;
        _ = InvalidateAndPublishAsync(null);
    }
    private void Snapshot(JsonElement snapshot)
    {
        long epoch = snapshot.Number("session_epoch");
        if (generation != epoch) { generation = epoch; publishSequence = 0; nativeRevision++; }
    }
    private void Connection(bool connected, string message)
    {
        if (!connected) { generation = null; pollNativeSequence = null; }
    }
    private void LocalChanged(ClipboardContent content)
    {
        if (paused || disposed) return;
        try { if (CaptureEnabled) history.Add(content, "local", "本机"); }
        catch (Exception) { Error?.Invoke("剪贴板历史保存失败。当前系统剪贴板没有被改变。"); }
        observedNativeSequence = native.SequenceNumber;
        _ = InvalidateAndPublishAsync(content);
    }
    private async Task InvalidateAndPublishAsync(ClipboardContent? content)
    {
        long revision = ++nativeRevision;
        var snapshot = service.LatestSnapshot;
        if (paused || !service.IsConnected || snapshot is null || generation is null) return;
        long epoch = generation.Value;
        try
        {
            // Every native mutation first cancels old remote content, even when
            // sync is disabled or the new clipboard cannot be transported.
            await service.SendAsync(ServiceClient.Command("InvalidateClipboard", new { session_epoch = epoch, sequence = ++publishSequence }));
            if (disposed || paused || generation != epoch || nativeRevision != revision || !SyncEnabled || content is null) return;
            if (!snapshot.Value.TryGetProperty("capabilities", out var capabilities) || !capabilities.Bool("clipboard")) return;
            if (content.Files is { Count: > 0 }) { Error?.Invoke("本机文件已记录；Windows 跨设备文件传输尚未接入。"); return; }
            var representations = content.Representations ?? [new ClipboardRepresentation(content.Mime, content.Data)];
            if (representations.Sum(r => (long)r.Data.Length) > NativeClipboard.MaxContentBytes) { Error?.Invoke("内容已保存在本机历史；当前桌面同步通道最多支持 16 MiB。"); return; }
            await service.SendAsync(ServiceClient.Command("PublishClipboard", new { session_epoch = epoch, sequence = ++publishSequence, formats = representations.Select(r => new { mime = r.Mime, data_base64 = Convert.ToBase64String(r.Data) }).ToArray() }));
        }
        catch (Exception) { if (!disposed && generation == epoch) Error?.Invoke("本机剪贴板保留，但服务未接受同步状态。请检查连接；旧内容不会作为新复制重发。"); }
    }
    private async void RemoteReceived(JsonElement item)
    {
        long? epoch = generation;
        if (epoch is null || disposed) return;
        long sequence = item.Number("sequence");
        bool applied = false;
        try
        {
            if (SyncEnabled && !paused && pollNativeSequence is uint expected)
            {
                var formats = item.Array("formats").Select(f => new ClipboardRepresentation(f.String("mime"), Convert.FromBase64String(f.String("data_base64")))).ToArray();
                if (formats.Length is 0 or > 4 || formats.Sum(f => (long)f.Data.Length) > NativeClipboard.MaxContentBytes) throw new InvalidDataException();
                var primary = formats.FirstOrDefault(f => f.Mime == "image/png") ?? formats.FirstOrDefault(f => f.Mime == "text/plain") ?? formats[0];
                var content = new ClipboardContent(primary.Mime, primary.Data, Representations: formats);
                // Check native sequence before committing; a newer user copy wins.
                applied = native.TryWrite(content, expected);
                if (applied)
                {
                    nativeRevision++; // Retire any older local publication awaiting an IPC response.
                    observedNativeSequence = native.SequenceNumber;
                    string peer = item.String("peer_id");
                    string name = service.LatestSnapshot?.Array("peers").FirstOrDefault(p => p.String("peer_id") == peer).String("display_name", peer) ?? peer;
                    if (CaptureEnabled) history.Add(content, peer, name);
                }
            }
        }
        catch (Exception) { Error?.Invoke(applied ? "远端剪贴板已写入，但本机历史保存失败。" : "远端剪贴板未能写入；未向对端确认应用成功。"); }
        try
        {
            if (!disposed && generation == epoch) await service.SendAsync(ServiceClient.Command("CommitClipboard", new { session_epoch = epoch.Value, sequence, applied }));
        }
        catch (Exception) { if (!disposed) Error?.Invoke("远端剪贴板应用回执未送达，请检查连接状态。"); }
    }
    public void Restore(ClipboardEntry entry)
    {
        native.Write(entry.Content);
        observedNativeSequence = native.SequenceNumber;
        _ = InvalidateAndPublishAsync(entry.Content);
    }
    public void Clear() => history.Clear();
    public void Remove(ClipboardEntry entry) => history.Remove(entry);
    public void ShowShelf(ClipboardEntry? entry = null)
    {
        if (shelf is null) { shelf = new ShelfWindow(native, history); shelf.Closed += (_, _) => shelf = null; }
        if (entry is not null) shelf.Add(entry);
        shelf.Show();
    }
    private void SessionChanged(object sender, SessionSwitchEventArgs args) => System.Windows.Application.Current.Dispatcher.BeginInvoke(() =>
    {
        if (disposed) return;
        if (args.Reason is SessionSwitchReason.SessionLock or SessionSwitchReason.SessionLogoff or SessionSwitchReason.ConsoleDisconnect or SessionSwitchReason.RemoteConnect)
        { paused = true; native.Pause(); generation = null; shelf?.Hide(); }
        else if (args.Reason is SessionSwitchReason.SessionUnlock or SessionSwitchReason.ConsoleConnect)
        { paused = false; native.Resume(); }
    });
    public void Dispose()
    {
        disposed = true;
        SystemEvents.SessionSwitch -= SessionChanged;
        service.ClipboardReceived -= RemoteReceived;
        service.PollStarting -= PollStarting;
        service.SnapshotChanged -= Snapshot;
        service.ConnectionChanged -= Connection;
        native.LocalInvalidated -= ObserveNativeChange;
        native.Dispose();
        shelf?.Shutdown();
        history.Dispose();
    }
}
