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
    private readonly ClipboardLibrary? localLibrary;
    private readonly DispatcherQueueTimer historyMaintenance;
    private readonly ServiceClient service;
    private readonly CanonicalLibraryClient canonical;
    private readonly DispatcherQueue dispatcher;
    private IShelfHost? shelf;
    private bool paused;
    private bool disposed;
    private Task? canonicalMigration;
    private long publishSequence;
    private uint? pollNativeSequence;
    private uint observedNativeSequence;
    private long nativeRevision;
    private long? generation;
    private bool captureEnabled = true;
    private bool syncEnabled;
    private bool keepAwake;
    private bool skipCanonicalIngest;
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
    internal CanonicalLibraryClient Canonical => canonical;
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
    private readonly Dictionary<ulong, ClipboardEntry> canonicalBodies = [];
    private readonly Dictionary<ulong, SemaphoreSlim> canonicalLoads = [];
    public bool CanonicalConnected => service.IsConnected;
    public event Action<string>? Error;
    public DesktopClipboardController(ServiceClient service, DispatcherQueue dispatcher, ClipboardLibrary? library = null)
    {
        this.service = service;
        this.dispatcher = dispatcher;
        canonical = new CanonicalLibraryClient(service);
        localLibrary = library;
        native = new NativeClipboard(dispatcher);
        LoadPreferences();
        // 库必须先于历史加载完成，过期清理才不会误删手动列表成员。
        history = library is null
            ? new ClipboardHistory(historyRetentionDays)
            : new ClipboardHistory(
                Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "DeskFuse", "desktop"),
                historyRetentionDays, () => DateTimeOffset.UtcNow, library.IsPinned);
        historyMaintenance = dispatcher.CreateTimer();
        historyMaintenance.Interval = TimeSpan.FromHours(1);
        historyMaintenance.Tick += (_, _) => { if (!disposed) history.Prune(); };
        historyMaintenance.Start();
        native.Changed += LocalChanged;
        native.LocalInvalidated += ObserveNativeChange;
        native.Error += message => { ObserveNativeChange(); Error?.Invoke(message); };
        service.ClipboardReceived += RemoteReceived;
        service.LanWebUploadReceived += LanWebReceived;
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

    public async Task<string> CreateFileDataObjectAsync(IReadOnlyList<string> paths, long sessionEpoch, long sequence, string? targetPeerId = null)
    {
        NativeClipboard.ValidateLocalFiles(paths);
        var reply = await service.SendPlatformAsync(sessionEpoch, sequence, PlatformBoundary.CreateFileDataObject(paths, targetPeerId));
        if (PlatformBoundary.TryGetFileDataObjectCreated(reply, out var id, out var error)) return id;
        throw new IOException(error);
    }

    internal async Task<ClipboardContent> CaptureTypedAsync(long sessionEpoch, long sequence, CancellationToken token = default) =>
        await ((ClipboardCapturePort)this).CaptureAsync(sessionEpoch, sequence, token);

    internal async Task<ClipboardContent> ApplyTypedAsync(ClipboardContent content, long expectedRevision, long sessionEpoch, long sequence, CancellationToken token = default) =>
        await ((ClipboardApplyPort)this).ApplyAsync(content, expectedRevision, sessionEpoch, sequence, token);

    /// <summary>
    /// A clipboard another application just wrote belongs to the user. Remote
    /// copies wait this long so a fresh screenshot or copy is never replaced
    /// mid-paste (and so no two synchronizers can rewrite it in turn).
    /// </summary>
    private const long RemoteApplyQuietMilliseconds = 1500;

    private sealed record Preferences(bool CaptureEnabled = true, bool SyncEnabled = false, bool KeepAwake = false,
        int HistoryRetentionDays = 30, string DownloadDirectory = "");

    private string downloadDirectory = "";
    private readonly Dictionary<string, string> receivedPaths = new();

    /// <summary>Default destination for verified incoming files; empty means
    /// the per-user Downloads\DeskFuse folder.</summary>
    public string DownloadDirectory
    {
        get => downloadDirectory;
        set
        {
            if (downloadDirectory == value) return;
            string previous = downloadDirectory;
            downloadDirectory = value;
            if (!SavePreferences())
            {
                downloadDirectory = previous;
                throw new IOException("接收位置保存失败。");
            }
        }
    }

    public bool TryGetReceivedPath(string transferId, out string path) =>
        receivedPaths.TryGetValue(transferId, out path!) && !string.IsNullOrEmpty(path);

    public async Task SetHistoryRetentionDaysAsync(int days)
    {
        await canonical.SettingsAsync(new { retention_days = days });
        SetHistoryRetentionDays(days);
    }

    // ---- Canonical content library ----

    public Task<CanonicalState> CanonicalStateAsync(CancellationToken token = default) =>
        canonical.StateAsync(token);

    public Task<CanonicalPage> QueryCanonicalAsync(
        string query, string? listId, string kind, string? source, int offset, int limit,
        IReadOnlyList<CanonicalRule>? rules = null, CancellationToken token = default) =>
        canonical.PageAsync(query, listId, kind, source, offset, limit, rules, token);

    public Task<Dictionary<string, long>> CanonicalCountsAsync(
        IReadOnlyList<CanonicalList> lists, CancellationToken token = default) =>
        canonical.CountsAsync(lists, token);

    public bool TryResolvedCanonical(ClipboardEntry entry, out ClipboardEntry resolved)
    {
        resolved = entry;
        if (entry.CanonicalId is not ulong id) return false;
        lock (canonicalBodies)
        {
            if (!canonicalBodies.TryGetValue(id, out var loaded)) return false;
            resolved = loaded;
            return true;
        }
    }

    /// <summary>Fetch the private payload once. Concurrent requests share one round trip.</summary>
    public async Task<ClipboardEntry> ResolveCanonicalAsync(ClipboardEntry entry, CancellationToken token = default)
    {
        if (entry.CanonicalId is not ulong id) return entry;
        if (TryResolvedCanonical(entry, out var cached)) return cached;
        SemaphoreSlim gate;
        lock (canonicalLoads)
        {
            if (!canonicalLoads.TryGetValue(id, out gate!))
                canonicalLoads[id] = gate = new SemaphoreSlim(1, 1);
        }
        await gate.WaitAsync(token);
        try
        {
            if (TryResolvedCanonical(entry, out cached)) return cached;
            var content = await canonical.ContentAsync(id, token);
            var resolved = entry with { Content = content, ContentLoaded = true, Available = true, PreviewOverride = null };
            lock (canonicalBodies) canonicalBodies[id] = resolved;
            return resolved;
        }
        finally
        {
            gate.Release();
            lock (canonicalLoads)
                if (gate.CurrentCount == 1) canonicalLoads.Remove(id);
        }
    }

    public async Task<byte[]?> ThumbnailAsync(ClipboardEntry entry, int pixelSize = 160, CancellationToken token = default)
    {
        if (entry.Thumbnail is { Length: > 0 } ready) return ready;
        if (entry.CanonicalId is ulong id) return await canonical.ThumbnailAsync(id, pixelSize, token);
        if (entry.Kind == "image") return entry.Content.Data;
        if (entry.Content.Files is not { Count: 1 } files || !File.Exists(files[0])) return null;
        var file = await global::Windows.Storage.StorageFile.GetFileFromPathAsync(files[0]).AsTask(token);
        using var thumbnail = await file.GetThumbnailAsync(global::Windows.Storage.FileProperties.ThumbnailMode.SingleItem,
            (uint)pixelSize).AsTask(token);
        if (thumbnail is null || !thumbnail.ContentType.StartsWith("image/", StringComparison.Ordinal)) return null;
        using var reader = new global::Windows.Storage.Streams.DataReader(thumbnail.GetInputStreamAt(0));
        await reader.LoadAsync((uint)thumbnail.Size).AsTask(token);
        byte[] bytes = new byte[thumbnail.Size];
        reader.ReadBytes(bytes);
        return bytes;
    }

    public async Task<string?> SetCanonicalTitleAsync(ClipboardEntry entry, string? title, CancellationToken token = default)
    {
        if (entry.CanonicalId is not ulong id) return null;
        await canonical.SetTitleAsync(id, title, token);
        InvalidateCanonical(id);
        return string.IsNullOrWhiteSpace(title) ? null : title.Trim();
    }

    public async Task DeleteCanonicalAsync(IEnumerable<ClipboardEntry> entries, CancellationToken token = default)
    {
        var ids = entries.Select(entry => entry.CanonicalId).OfType<ulong>().Distinct().ToArray();
        if (ids.Length == 0) return;
        await canonical.DeleteAsync(ids, token);
        foreach (ulong id in ids) InvalidateCanonical(id);
    }

    public Task ClearCanonicalHistoryAsync(CancellationToken token = default) => canonical.ClearHistoryAsync(token);

    public async Task SetCanonicalMembershipAsync(string listId, ClipboardEntry entry, bool present, CancellationToken token = default)
    {
        if (entry.CanonicalId is not ulong id) return;
        await canonical.MembershipAsync(listId, id, present, token);
    }

    public Task SaveCanonicalListAsync(CanonicalList list, CancellationToken token = default) =>
        canonical.SaveListAsync(list, token);

    public Task DeleteCanonicalListAsync(string id, CancellationToken token = default) =>
        canonical.DeleteListAsync(id, token);

    public Task ReorderCanonicalListsAsync(IEnumerable<string> ids, CancellationToken token = default) =>
        canonical.ReorderAsync(ids, token);

    public Task ClearCanonicalRemoteIndexAsync(CancellationToken token = default) =>
        canonical.ClearRemoteIndexAsync(token);

    public Task SaveCanonicalTextAsync(ClipboardEntry entry, string text, byte[]? rtf = null, CancellationToken token = default) =>
        entry.CanonicalId is ulong id ? canonical.SetTextAsync(id, text, rtf, token) : Task.CompletedTask;

    private void InvalidateCanonical(ulong id)
    {
        lock (canonicalBodies) canonicalBodies.Remove(id);
    }

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
            downloadDirectory = preferences.DownloadDirectory ?? "";
        }
        catch (Exception) { preferencesError = "无法读取本机偏好；本次使用默认设置，原文件未改变。"; }
    }

    private bool SavePreferences()
    {
        string temporary = preferencesFile + ".tmp";
        try
        {
            Directory.CreateDirectory(Path.GetDirectoryName(preferencesFile)!);
            File.WriteAllText(temporary, JsonSerializer.Serialize(new Preferences(captureEnabled, syncEnabled, keepAwake, historyRetentionDays, downloadDirectory)));
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
        else if (localLibrary is not null)
        {
            canonicalMigration ??= MigrateCanonicalAsync();
        }
    }

    private async Task MigrateCanonicalAsync()
    {
        try { await canonical.MigrateAsync(history.Entries.ToArray(), localLibrary!, historyRetentionDays); }
        catch (Exception ex) when (ex is IOException or InvalidOperationException or OperationCanceledException)
        {
            canonicalMigration = null;
            dispatcher.TryEnqueue(() => { if (!disposed) Error?.Invoke($"历史迁移稍后将重试：{ex.Message}"); });
        }
    }

    private void LocalChanged(ClipboardContent content)
    {
        if (paused || disposed) return;
        string? application = NativeClipboard.ForegroundProcessName();
        try { if (CaptureEnabled) history.Add(content, "local", "本机", application); }
        catch (Exception) { Error?.Invoke("剪贴板历史保存失败。当前系统剪贴板没有被改变。"); }
        if (CaptureEnabled && !skipCanonicalIngest) RecordCanonical(content, "local", "本机", application);
        observedNativeSequence = native.SequenceNumber;
        _ = InvalidateAndPublishAsync(content, application);
    }

    private void RecordCanonical(ClipboardContent content, string deviceId, string source, string? application, long? copiedAtMs = null)
    {
        if (!service.IsConnected) return;
        _ = Task.Run(async () =>
        {
            try { await canonical.IngestAsync(content, deviceId, source, application, copiedAtMs: copiedAtMs); }
            catch (Exception ex) when (ex is IOException or InvalidOperationException or OperationCanceledException)
            {
                dispatcher.TryEnqueue(() => { if (!disposed) Error?.Invoke($"canonical 内容库暂不可用：{ex.Message}"); });
            }
        });
    }

    private async Task InvalidateAndPublishAsync(ClipboardContent? content, string? application = null)
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
            await service.SendAsync(ServiceClient.Command("PublishClipboard", new { session_epoch = epoch, sequence = ++publishSequence, source_app = application, formats = representations.Select(r => new { mime = r.Mime, data_base64 = Convert.ToBase64String(r.Data) }).ToArray() }));
        }
        catch (Exception ex) { if (!disposed && generation == epoch) Error?.Invoke($"本机剪贴板已保留，同步未完成：{ex.Message}"); }
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
            else if (SyncEnabled && !paused && pollNativeSequence is uint expected
                && native.MillisecondsSinceForeignChange >= RemoteApplyQuietMilliseconds)
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
                    var origin = item.TryGetProperty("source_metadata", out var metadata) ? metadata : default;
                    if (CaptureEnabled && !origin.Bool("no_history"))
                    {
                        string originDevice = origin.String("device_id", peer);
                        string? app = origin.String("app_bundle_id") is { Length: > 0 } sourceApp ? sourceApp : null;
                        long copiedAt = origin.Number("created_at_ms");
                        history.Add(content, originDevice, name, app);
                        RecordCanonical(content, originDevice, name, app, copiedAt > 0 ? copiedAt : null);
                    }
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

    private async void LanWebReceived(JsonElement item)
    {
        long? epoch = generation;
        if (epoch is null || disposed) return;
        string id = item.String("id");
        bool applied = false;
        bool historySaved = false;
        bool accepted = false;
        if (!paused)
        {
            try
            {
                var formats = item.Array("formats").Select(f => new ClipboardRepresentation(f.String("mime"), Convert.FromBase64String(f.String("data_base64")))).ToArray();
                if (formats.Length is 0 or > 4 || formats.Sum(f => (long)f.Data.Length) > NativeClipboard.MaxContentBytes) throw new InvalidDataException();
                var primary = formats.FirstOrDefault(f => f.Mime == "image/png") ?? formats.FirstOrDefault(f => f.Mime == "text/plain") ?? formats[0];
                var content = new ClipboardContent(primary.Mime, primary.Data, Representations: formats);
                try
                {
                    if (pollNativeSequence is uint expected
                        && native.MillisecondsSinceForeignChange >= RemoteApplyQuietMilliseconds)
                    {
                        applied = native.TryWrite(content, expected);
                        if (applied)
                        {
                            nativeRevision++;
                            observedNativeSequence = native.SequenceNumber;
                        }
                    }
                }
                catch (Exception) { applied = false; }
                if (CaptureEnabled)
                {
                    try
                    {
                        history.Add(content, "lan-web", "手机网页");
                        RecordCanonical(content, "lan-web", "手机网页", null);
                        historySaved = true;
                    }
                    catch (Exception) { Error?.Invoke("网页内容已接收，但本机历史保存失败。"); }
                }
                accepted = applied || historySaved || !CaptureEnabled;
            }
            catch (Exception) { accepted = applied || historySaved; }
        }
        try
        {
            if (!disposed && generation == epoch)
                await service.SendAsync(ServiceClient.Command("CommitLanWebUpload", new { session_epoch = epoch.Value, upload_id = id, recorded = accepted, clipboard_written = applied }));
        }
        catch (Exception) { if (!disposed) Error?.Invoke("网页上传回执未送达，请检查连接状态。"); }
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

    /// <summary>Update one canonical row in place, then publish the edited text
    /// without creating a duplicate canonical entry for the same edit.</summary>
    public async Task SaveEditedContentAsync(ClipboardEntry entry, ClipboardContent content, CancellationToken token = default)
    {
        string text = System.Text.Encoding.UTF8.GetString(
            content.Representations?.FirstOrDefault(item => item.Mime == "text/plain")?.Data ?? content.Data);
        byte[]? rtf = content.Representations?.FirstOrDefault(item => item.Mime == "text/rtf")?.Data;
        if (entry.CanonicalId is ulong id)
        {
            await canonical.SetTextAsync(id, text, rtf, token);
            InvalidateCanonical(id);
        }
        skipCanonicalIngest = true;
        try { SaveEditedContent(content); }
        finally { skipCanonicalIngest = false; }
    }

    /// <summary>Restore and return the exact native generation for a later paste guard.</summary>
    public uint Restore(ClipboardEntry entry)
    {
        if (!entry.IsCanonical)
        {
            history.Prune();
            if (!history.Entries.Any(current => current.Id == entry.Id))
                throw new InvalidOperationException("记录已删除或过期，请重新选择。");
        }
        native.Write(entry.Content);
        observedNativeSequence = native.SequenceNumber;
        _ = InvalidateAndPublishAsync(entry.Content);
        return observedNativeSequence;
    }

    public async Task<uint> RestoreAsync(ClipboardEntry entry, CancellationToken token = default)
    {
        if (entry.IsCanonical) entry = await ResolveCanonicalAsync(entry, token);
        return Restore(entry);
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
    /// (shared with local copies and the shelf's send action). When
    /// <paramref name="targetPeerId"/> is set, only that peer receives the offer.</summary>
    public async Task<string> BeginFileTransferAsync(IReadOnlyList<string> paths, string? targetPeerId = null)
    {
        NativeClipboard.ValidateLocalFiles(paths);
        if (service.LatestSnapshot is null || generation is null)
            throw new IOException("后台组件尚未就绪，无法发送文件。");
        if (!FilesEnabled) throw new IOException("文件传输尚未就绪，请检查文件共享开关与对端握手。");
        long epoch = generation.Value;
        await service.SendAsync(ServiceClient.Command("InvalidateClipboard", new { session_epoch = epoch, sequence = ++publishSequence }));
        return await CreateFileDataObjectAsync(paths, epoch, ++publishSequence, targetPeerId);
    }

    public async Task<bool> PublishFilesAsync(IReadOnlyList<string> paths, string? targetPeerId = null)
    {
        try
        {
            await BeginFileTransferAsync(paths, targetPeerId);
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
                if (copiedRoots.Count > 0) receivedPaths[id] = copiedRoots[0];
                destinations.Remove(id);
                var copiedContent = new ClipboardContent(ClipboardContent.LocalFilesMime, Array.Empty<byte>(), copiedRoots);
                native.Write(copiedContent);
                observedNativeSequence = native.SequenceNumber;
                if (CaptureEnabled)
                {
                    history.Add(copiedContent, "local", "本机", "DeskFuse");
                    RecordCanonical(copiedContent, "local", "本机", "DeskFuse");
                }
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
            if (copies.Count > 0) receivedPaths[id] = copies[0];
            destinations.Remove(id);
            // The received copies are installed onto the clipboard so the user
            // can paste them in Explorer immediately, mirroring macOS.
            var flatContent = new ClipboardContent(ClipboardContent.LocalFilesMime, Array.Empty<byte>(), copies);
            native.Write(flatContent);
            observedNativeSequence = native.SequenceNumber;
            if (CaptureEnabled)
            {
                history.Add(flatContent, "local", "本机", "DeskFuse");
                RecordCanonical(flatContent, "local", "本机", "DeskFuse");
            }
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
        finally { copying.Remove(id); destinations.Remove(id); }
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

    public void ToggleShelf()
    {
        if (shelf?.IsVisible == true) shelf.Hide();
        else ShowShelf();
    }

    public void ShowShelfAtPointer()
    {
        ShowShelf();
        shelf?.Show(nearPointer: true);
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
    bool IsVisible { get; }
    void Add(ClipboardEntry entry);
    void Show(bool nearPointer = false);
    void Hide();
    void Shutdown();
}
