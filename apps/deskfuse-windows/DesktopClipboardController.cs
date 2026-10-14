using System.Collections.ObjectModel;
using System.Text.Json;
using Microsoft.UI.Dispatching;
using Microsoft.Win32;

namespace DeskFuse.Windows;

// Local history belongs to the user process; transport and identity belong only
// to the authenticated service. Neither reconnect nor startup republishes history.
public sealed class DesktopClipboardController : IDisposable, ClipboardCapturePort, ClipboardApplyPort, FileDataObjectPort
{
    private readonly NativeClipboard native;
    private readonly ClipboardHistory history;
    private readonly DispatcherQueueTimer historyMaintenance;
    private readonly ServiceClient service;
    private readonly DispatcherQueue dispatcher;
    private IShelfHost? shelf;
    private bool paused;
    private bool disposed;
    private long publishSequence;
    private uint? pollNativeSequence;
    private uint observedNativeSequence;
    private long nativeRevision;
    private long? generation;
    private bool captureEnabled = true;
    private bool syncEnabled;
    private bool keepAwake;
    private int historyRetentionDays = 30;
    public bool KeepAwake => keepAwake;
    public bool SetKeepAwake(bool enabled)
    {
        if (keepAwake == enabled) return true;
        bool previous = keepAwake;
        keepAwake = enabled;
        if (SavePreferences()) return true;
        keepAwake = previous;
        return false;
    }

    public int HistoryRetentionDays => historyRetentionDays;
    /// <summary>History persistence failure, if any; diagnostics-safe.</summary>
    public string? StorageError => history.StorageError;
    private string? preferencesError;
    private readonly string preferencesFile = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "DeskFuse", "desktop", "settings-v1.json");
    /// <summary>Shelf window factory. Wired by the app so the controller keeps no UI type.</summary>
    public Func<IShelfHost>? ShelfFactory { get; set; }
    /// <summary>Shared with the shelf window; the controller stays the owner.</summary>
    internal NativeClipboard Native => native;
    internal ClipboardHistory History => history;
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
    public DesktopClipboardController(ServiceClient service, DispatcherQueue dispatcher)
    {
        this.service = service;
        this.dispatcher = dispatcher;
        native = new NativeClipboard(dispatcher);
        LoadPreferences();
        history = new ClipboardHistory(historyRetentionDays);
        historyMaintenance = dispatcher.CreateTimer();
        historyMaintenance.Interval = TimeSpan.FromHours(1);
        historyMaintenance.Tick += (_, _) => { if (!disposed) history.Prune(); };
        historyMaintenance.Start();
        native.Changed += LocalChanged;
        native.LocalInvalidated += ObserveNativeChange;
        native.Error += message => { ObserveNativeChange(); Error?.Invoke(message); };
        service.ClipboardReceived += RemoteReceived;
        service.PollStarting += PollStarting;
        service.SnapshotChanged += Snapshot;
        service.ConnectionChanged += Connection;
        SystemEvents.SessionSwitch += SessionChanged;
    }
    public void Start(IntPtr hwnd)
    {
        try { native.Start(hwnd); observedNativeSequence = native.SequenceNumber; if (history.StorageError is string error) Error?.Invoke(error); if (preferencesError is string warning) Error?.Invoke(warning); }
        catch (Exception) { Error?.Invoke("无法启动 Windows 剪贴板监听。请重新打开客户端。"); }
    }

    internal long NativeRevision => native.SequenceNumber;

    async Task<ClipboardContent> ClipboardCapturePort.CaptureAsync(long sessionEpoch, long sequence, CancellationToken token)
    {
        token.ThrowIfCancellationRequested();
        var content = native.Read() ?? throw new InvalidOperationException("当前剪贴板没有可用于工作流的内容。");
        var reply = await service.SendPlatformAsync(sessionEpoch, sequence, PlatformBoundary.ReadClipboardHost(content, native.SequenceNumber), token);
        _ = PlatformBoundary.ReadContent(PlatformBoundary.Event(reply));
        return content;
    }

    async Task<ClipboardContent> ClipboardApplyPort.ApplyAsync(ClipboardContent content, long expectedRevision, long sessionEpoch, long sequence, CancellationToken token)
    {
        token.ThrowIfCancellationRequested();
        if (expectedRevision < 0 || expectedRevision > uint.MaxValue || !native.TryWrite(content, (uint)expectedRevision))
            throw new IOException("剪贴板已变化或被占用，请重试。");
        var reply = await service.SendPlatformAsync(sessionEpoch, sequence, PlatformBoundary.WriteClipboardHost(content, expectedRevision), token);
        var eventValue = PlatformBoundary.Event(reply);
        if (!eventValue.TryGetProperty("ClipboardApplied", out var applied) || !applied.TryGetProperty("Applied", out _))
            throw new IOException("服务未确认 typed 剪贴板写入。");
        observedNativeSequence = native.SequenceNumber;
        return content;
    }

    public async Task<string> CreateFileDataObjectAsync(IReadOnlyList<string> paths, long sessionEpoch, long sequence)
    {
        NativeClipboard.ValidateLocalFiles(paths);
        var reply = await service.SendPlatformAsync(sessionEpoch, sequence, PlatformBoundary.CreateFileDataObject(paths));
        if (PlatformBoundary.TryGetFileDataObjectCreated(reply, out var id, out var error)) return id;
        throw new IOException(error);
    }

    internal async Task<ClipboardContent> CaptureTypedAsync(long sessionEpoch, long sequence, CancellationToken token = default) =>
        await ((ClipboardCapturePort)this).CaptureAsync(sessionEpoch, sequence, token);

    internal async Task<ClipboardContent> ApplyTypedAsync(ClipboardContent content, long expectedRevision, long sessionEpoch, long sequence, CancellationToken token = default) =>
        await ((ClipboardApplyPort)this).ApplyAsync(content, expectedRevision, sessionEpoch, sequence, token);

    private sealed record Preferences(bool CaptureEnabled = true, bool SyncEnabled = false, bool KeepAwake = false, int HistoryRetentionDays = 30);

    public void SetHistoryRetentionDays(int days)
    {
        ArgumentOutOfRangeException.ThrowIfLessThan(days, 1);
        if (days == historyRetentionDays) return;
        int previous = historyRetentionDays;
        historyRetentionDays = days;
        if (!SavePreferences())
        {
            historyRetentionDays = previous;
            throw new IOException("保留天数保存失败，历史保留规则未改变。");
        }
        // Only prune once the new policy is durably saved.
        history.SetRetentionDays(days);
    }

    private void LoadPreferences()
    {
        try
        {
            if (!File.Exists(preferencesFile)) return;
            if (new FileInfo(preferencesFile).Length > 8192) throw new InvalidDataException();
            var preferences = JsonSerializer.Deserialize<Preferences>(File.ReadAllText(preferencesFile)) ?? throw new InvalidDataException();
            if (preferences.HistoryRetentionDays < 1) throw new InvalidDataException();
            captureEnabled = preferences.CaptureEnabled;
            syncEnabled = preferences.SyncEnabled;
            keepAwake = preferences.KeepAwake;
            historyRetentionDays = preferences.HistoryRetentionDays;
        }
        catch (Exception) { preferencesError = "无法读取本机偏好；本次使用默认设置，原文件未改变。"; }
    }

    private bool SavePreferences()
    {
        string temporary = preferencesFile + ".tmp";
        try
        {
            Directory.CreateDirectory(Path.GetDirectoryName(preferencesFile)!);
            File.WriteAllText(temporary, JsonSerializer.Serialize(new Preferences(captureEnabled, syncEnabled, keepAwake, historyRetentionDays)));
            File.Move(temporary, preferencesFile, overwrite: true);
            preferencesError = null;
            return true;
        }
        catch (Exception)
        {
            preferencesError = "设置仅在本次会话生效，无法保存到当前用户目录。";
            Error?.Invoke(preferencesError);
            try { if (File.Exists(temporary)) File.Delete(temporary); } catch (Exception) { }
            return false;
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
        if (ownership is true) { observedNativeSequence = ownSequence; return; }
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
        ObserveFileTransfers(snapshot);
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
            if (disposed || paused || generation != epoch || nativeRevision != revision) return;
            if (!snapshot.Value.TryGetProperty("capabilities", out var capabilities) || !capabilities.Bool("clipboard")) return;
            if (!SyncEnabled) return;
            if (content is not null && content.Files is { Count: > 0 })
            {
                if (!capabilities.Bool("files")) { Error?.Invoke("本机文件已记录；对端或本机暂未启用跨设备文件传输。"); return; }
                await service.SendAsync(ServiceClient.Command("PublishFileClipboard", new { session_epoch = epoch, sequence = ++publishSequence, paths = content.Files }));
                return;
            }
            if (content is null) return;
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
            if (SyncEnabled && !paused && item.String("file_transfer_id") is string fileTransferId && !string.IsNullOrWhiteSpace(fileTransferId))
            {
                var metadata = DeferredOleEntries(fileTransferId);
                native.WriteOle(new ExplorerOleFileDataObject(metadata, () => MaterializeDeferredFileTransfer(fileTransferId, metadata)));
                nativeRevision++;
                observedNativeSequence = native.SequenceNumber;
                applied = true;
            }
            else if (SyncEnabled && !paused && pollNativeSequence is uint expected)
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

    public void SaveEditedContent(ClipboardContent content)
    {
        ObjectDisposedException.ThrowIf(disposed, this);
        if (paused) throw new InvalidOperationException("当前会话不可写入剪贴板，请解锁后重试。");
        native.Write(content);
        // Own writes are deliberately ignored by the clipboard listener; enqueue
        // this explicit edit through the same capture/publication path once.
        LocalChanged(content);
    }

    public void Restore(ClipboardEntry entry)
    {
        history.Prune();
        if (!history.Entries.Any(current => current.Id == entry.Id))
            throw new InvalidOperationException("记录已删除或过期，请重新选择。");
        native.Write(entry.Content);
        observedNativeSequence = native.SequenceNumber;
        _ = InvalidateAndPublishAsync(entry.Content);
    }

    // ---- Cross-device file transfer (deferred clipboard file protocol) ----

    /// <summary>Destination chosen for one incoming transfer (per transfer id).</summary>
    private readonly Dictionary<string, string> destinations = new();
    /// <summary>Transfer ids currently being copied out, to avoid double copy.</summary>
    private readonly HashSet<string> copying = new();
    /// <summary>Offers already announced, so notices fire once per offer.</summary>
    private readonly HashSet<string> seenOffers = new();
    /// <summary>Raised when a new remote file offer arrives (notice/tray).</summary>
    public event Action<string, string>? FileOfferReceived;
    /// <summary>Raised when a transfer settles (transfer id, detail, success).</summary>
    public event Action<string, string, bool>? FileTransferSettled;

    public bool FilesEnabled
    {
        get
        {
            var snapshot = service.LatestSnapshot;
            return snapshot is not null && snapshot.Value.TryGetProperty("capabilities", out var caps) && caps.Bool("files");
        }
    }

    /// <summary>Tracks snapshot transfer rows: new offers raise notices; staged
    /// transfers with a chosen destination are copied out and committed.</summary>
    private void ObserveFileTransfers(JsonElement snapshot)
    {
        foreach (var transfer in snapshot.Array("file_transfers"))
        {
            string id = transfer.String("transfer_id");
            string state = transfer.String("state");
            if (state != "staged" && state != "offered") { seenOffers.Remove(id); continue; }
            if (state == "offered" && seenOffers.Add(id))
            {
                string peer = transfer.String("peer_id");
                string name = service.LatestSnapshot?.Array("peers").FirstOrDefault(p => p.String("peer_id") == peer).String("display_name", peer) ?? peer;
                var names = transfer.Array("names").Where(v => v.ValueKind == JsonValueKind.String).Select(v => v.GetString() ?? "").ToArray();
                FileOfferReceived?.Invoke(name, names.Length > 0 ? string.Join("、", names) : "文件");
            }
            if (state == "staged" && destinations.TryGetValue(id, out var destination) && copying.Add(id))
            {
                _ = CopyStagedAsync(transfer, destination);
            }
        }
    }

    /// <summary>Requests the content of one incoming offer, remembering the
    /// user-chosen destination folder for the copy-out phase.</summary>
    private IReadOnlyList<ExplorerOleFileEntry> DeferredOleEntries(string transferId)
    {
        var snapshot = service.LatestSnapshot;
        if (snapshot is null) return [];
        var transfer = snapshot.Value.Array("file_transfers")
            .FirstOrDefault(item => item.String("transfer_id") == transferId);
        return DeferredOleEntriesFromTransfer(transfer);
    }

    private static IReadOnlyList<ExplorerOleFileEntry> DeferredOleEntriesFromTransfer(JsonElement transfer)
    {
        var entries = transfer.Array("entries")
            .Select(item => new ExplorerOleFileEntry(
                item.String("path"),
                item.String("kind") is "directory" or "symlink",
                item.Number("size"),
                (int)item.Number("lindex")))
            .Where(item => !string.IsNullOrWhiteSpace(item.RelativePath))
            .OrderBy(item => item.Lindex)
            .ToArray();
        if (entries.Length > 0) return entries;
        return transfer.Array("names")
            .Select((item, index) => new ExplorerOleFileEntry(item.GetString() ?? "file", false, 0, index))
            .ToArray();
    }

    private IReadOnlyList<ExplorerOleFileEntry> MaterializeDeferredFileTransfer(
        string transferId,
        IReadOnlyList<ExplorerOleFileEntry> metadata)
    {
        long epoch = generation ?? throw new IOException("桌面会话已失效");
        service.SendAsync(ServiceClient.Command("RequestFileClipboard", new { session_epoch = epoch, transfer_id = transferId })).GetAwaiter().GetResult();
        var deadline = Environment.TickCount64 + 60_000;
        while (Environment.TickCount64 < deadline)
        {
            var reply = service.SendAsync(ServiceClient.Command("Poll", new { session_epoch = epoch, after_sequence = 0L })).GetAwaiter().GetResult();
            if (reply.TryGetProperty("Poll", out var poll))
            {
                foreach (var transfer in poll.GetProperty("snapshot").Array("file_transfers"))
                {
                    if (transfer.String("transfer_id") == transferId && transfer.String("state") == "staged")
                    {
                        var paths = transfer.Array("staged_paths").Where(v => v.ValueKind == JsonValueKind.String).Select(v => v.GetString() ?? "").Where(File.Exists).ToArray();
                        if (paths.Length > 0 || metadata.Any(item => item.IsDirectory))
                        {
                            string cache = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "DeskFuse", "desktop", "ole-files", transferId);
                            Directory.CreateDirectory(cache);
                            var copies = new List<ExplorerOleFileEntry>();
                            foreach (var item in metadata)
                            {
                                string target = Path.Combine(cache, item.RelativePath.Replace('/', Path.DirectorySeparatorChar));
                                if (item.IsDirectory)
                                {
                                    Directory.CreateDirectory(target);
                                    copies.Add(item.WithLocalPath(target));
                                    continue;
                                }
                                string? source = paths.FirstOrDefault(path => path.EndsWith(Path.DirectorySeparatorChar + item.RelativePath.Replace('/', Path.DirectorySeparatorChar), StringComparison.OrdinalIgnoreCase))
                                    ?? paths.FirstOrDefault(path => string.Equals(Path.GetFileName(path), Path.GetFileName(item.RelativePath), StringComparison.OrdinalIgnoreCase));
                                if (source is null) throw new IOException($"暂存文件缺失：{item.RelativePath}");
                                Directory.CreateDirectory(Path.GetDirectoryName(target)!);
                                File.Copy(source, target, overwrite: true);
                                copies.Add(item.WithLocalPath(target));
                            }
                            service.SendAsync(ServiceClient.Command("CommitFileClipboard", new { session_epoch = epoch, transfer_id = transferId, applied = true })).GetAwaiter().GetResult();
                            return copies;
                        }
                    }
                }
            }
            Thread.Sleep(100);
        }
        throw new IOException("远端文件准备超时，Explorer 未收到文件内容");
    }

    public async Task<bool> ReceiveFileAsync(string transferId, string destination)
    {
        try
        {
            Directory.CreateDirectory(destination);
            destinations[transferId] = destination;
            await service.SendAsync(ServiceClient.Command("RequestFileClipboard", new { session_epoch = generation ?? 0, transfer_id = transferId }));
            return true;
        }
        catch (Exception ex) { Error?.Invoke(ex is IOException ? ex.Message : "无法请求文件内容，请稍后重试。"); return false; }
    }

    public async Task CancelFileAsync(string transferId)
    {
        try
        {
            destinations.Remove(transferId);
            await service.SendAsync(ServiceClient.Command("CancelFileClipboard", new { session_epoch = generation ?? 0, transfer_id = transferId }));
        }
        catch (Exception) { Error?.Invoke("取消请求未送达，请检查连接。"); }
    }

    /// <summary>Publishes existing local files as a deferred file clipboard
    /// (shared with local copies and the shelf's send action).</summary>
    public async Task<bool> PublishFilesAsync(IReadOnlyList<string> paths)
    {
        try
        {
            NativeClipboard.ValidateLocalFiles(paths);
            var snapshot = service.LatestSnapshot;
            if (snapshot is null || generation is null) { Error?.Invoke("后台组件尚未就绪，无法发送文件。"); return false; }
            if (!FilesEnabled) { Error?.Invoke("对端设备尚未支持跨设备文件传输。"); return false; }
            long epoch = generation.Value;
            await service.SendAsync(ServiceClient.Command("InvalidateClipboard", new { session_epoch = epoch, sequence = ++publishSequence }));
            await CreateFileDataObjectAsync(paths, epoch, ++publishSequence);
            return true;
        }
        catch (Exception ex) { Error?.Invoke(ex is IOException or InvalidOperationException ? ex.Message : "文件发送未完成，请稍后重试。"); return false; }
    }

    /// <summary>Copies verified staged files from the service cache into the
    /// user destination, places the copies on the clipboard (like the macOS
    /// receiver installing URLs), and commits the transfer.</summary>
    private async Task CopyStagedAsync(JsonElement transfer, string destination)
    {
        string id = transfer.String("transfer_id");
        try
        {
            var staged = transfer.Array("staged_paths").Where(v => v.ValueKind == JsonValueKind.String).Select(v => v.GetString() ?? "").Where(p => File.Exists(p) || Directory.Exists(p)).ToArray();
            var metadata = DeferredOleEntriesFromTransfer(transfer);
            if (metadata.Count > 0)
            {
                var roots = metadata.Select(item => item.RelativePath.Split('/')[0]).Distinct(StringComparer.OrdinalIgnoreCase).ToDictionary(root => root, root => ResolveConflict(Path.Combine(destination, root)), StringComparer.OrdinalIgnoreCase);
                foreach (var item in metadata.Where(item => item.IsDirectory))
                {
                    string root = item.RelativePath.Split('/')[0];
                    int separator = item.RelativePath.IndexOf('/');
                    string suffix = separator < 0 ? "" : item.RelativePath[(separator + 1)..];
                    Directory.CreateDirectory(Path.Combine(roots[root], suffix.Replace('/', Path.DirectorySeparatorChar)));
                }
                foreach (var item in metadata.Where(item => !item.IsDirectory))
                {
                    string? source = staged.FirstOrDefault(path => path.EndsWith(Path.DirectorySeparatorChar + item.RelativePath.Replace('/', Path.DirectorySeparatorChar), StringComparison.OrdinalIgnoreCase));
                    if (source is null) throw new IOException($"暂存文件缺失：{item.RelativePath}");
                    string root = item.RelativePath.Split('/')[0];
                    string suffix = item.RelativePath.Length == root.Length ? "" : item.RelativePath[(root.Length + 1)..];
                    string target = Path.Combine(roots[root], suffix.Replace('/', Path.DirectorySeparatorChar));
                    Directory.CreateDirectory(Path.GetDirectoryName(target)!);
                    File.Copy(source, target, overwrite: false);
                }
                var copiedRoots = roots.Values.ToList();
                destinations.Remove(id);
                var copiedContent = new ClipboardContent(ClipboardContent.LocalFilesMime, Array.Empty<byte>(), copiedRoots);
                native.Write(copiedContent);
                observedNativeSequence = native.SequenceNumber;
                if (CaptureEnabled) history.Add(copiedContent, "local", "本机");
                await service.SendAsync(ServiceClient.Command("CommitFileClipboard", new { session_epoch = generation ?? 0, transfer_id = id, applied = true }));
                FileTransferSettled?.Invoke(id, "文件已接收并放入剪贴板", true);
                return;
            }
            if (staged.Length == 0) throw new IOException("暂存文件不可用，接收已终止。");
            var copies = new List<string>();
            foreach (var path in staged)
            {
                string target = ResolveConflict(Path.Combine(destination, Path.GetFileName(path.TrimEnd(Path.DirectorySeparatorChar))));
                if (File.Exists(path)) File.Copy(path, target, overwrite: false);
                else CopyDirectory(path, target);
                copies.Add(target);
            }
            destinations.Remove(id);
            // The received copies are installed onto the clipboard so the user
            // can paste them in Explorer immediately, mirroring macOS.
            var flatContent = new ClipboardContent(ClipboardContent.LocalFilesMime, Array.Empty<byte>(), copies);
            native.Write(flatContent);
            observedNativeSequence = native.SequenceNumber;
            if (CaptureEnabled) history.Add(flatContent, "local", "本机");
            await service.SendAsync(ServiceClient.Command("CommitFileClipboard", new { session_epoch = generation ?? 0, transfer_id = id, applied = true }));
            FileTransferSettled?.Invoke(id, "文件已接收并放入剪贴板", true);
        }
        catch (Exception ex)
        {
            Error?.Invoke(ex is IOException or UnauthorizedAccessException ? $"文件落地失败：{ex.Message}" : "文件落地失败，未向对端确认成功。");
            try { await service.SendAsync(ServiceClient.Command("CommitFileClipboard", new { session_epoch = generation ?? 0, transfer_id = id, applied = false })); }
            catch (Exception) { Error?.Invoke("文件落地回执未送达，请检查连接。"); }
            FileTransferSettled?.Invoke(id, ex.Message, false);
        }
        finally { copying.Remove(id); }
    }

    private static string ResolveConflict(string target)
    {
        if (!File.Exists(target) && !Directory.Exists(target)) return target;
        string directory = Path.GetDirectoryName(target) ?? ".";
        string name = Path.GetFileName(target.TrimEnd(Path.DirectorySeparatorChar));
        string stem = File.Exists(target) ? Path.GetFileNameWithoutExtension(name) : name;
        string extension = File.Exists(target) ? Path.GetExtension(name) : "";
        for (var number = 2; number < 1000; number++)
        {
            string candidate = Path.Combine(directory, $"{stem} ({number}){extension}");
            if (!File.Exists(candidate) && !Directory.Exists(candidate)) return candidate;
        }
        throw new IOException("目标位置存在同名文件，无法安全落盘");
    }

    private static void CopyDirectory(string source, string target)
    {
        Directory.CreateDirectory(target);
        foreach (var file in Directory.EnumerateFiles(source))
        {
            string destination = ResolveConflict(Path.Combine(target, Path.GetFileName(file)));
            File.Copy(file, destination, overwrite: false);
        }
        foreach (var directory in Directory.EnumerateDirectories(source))
        {
            CopyDirectory(directory, Path.Combine(target, Path.GetFileName(directory.TrimEnd(Path.DirectorySeparatorChar))));
        }
    }

    public void Clear() => history.Clear();
    public void Remove(ClipboardEntry entry) => history.Remove(entry);

    public void ShowShelf(ClipboardEntry? entry = null)
    {
        if (shelf is null) { shelf = ShelfFactory?.Invoke(); if (shelf is null) { Error?.Invoke("暂存区不可用。"); return; } }
        if (entry is not null) shelf.Add(entry);
        shelf.Show();
    }

    private void SessionChanged(object sender, SessionSwitchEventArgs args) => dispatcher.TryEnqueue(() =>
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
        historyMaintenance.Stop();
        native.Dispose();
        shelf?.Shutdown();
        history.Dispose();
    }
}

/// <summary>Shelf window contract, so the controller holds no WinUI type.</summary>
public interface IShelfHost
{
    void Add(ClipboardEntry entry);
    void Show();
    void Hide();
    void Shutdown();
}
