using System.Collections.ObjectModel;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;
using System.Threading.Channels;

namespace DeskFuse.Windows;

public sealed record ClipboardEntry(string Id, ClipboardContent Content, string SourceId,
    string SourceName, DateTimeOffset CreatedAt)
{
    public string? CustomTitle { get; init; }
    /// <summary>Foreground application at copy time; null for remote entries.</summary>
    public string? SourceApp { get; init; }
    [JsonIgnore]
    public string Kind => Content.Files is { Count: > 0 } ? "files" : Content.Mime == "image/png" ? "image" : "text";
    [JsonIgnore]
    public long SizeBytes => Content.SizeBytes;
    /// <summary>Preformatted timestamp: WinUI bindings have no StringFormat.</summary>
    [JsonIgnore]
    public string CreatedLabel => CreatedAt.LocalDateTime.ToString("g");
    [JsonIgnore]
    public string TimeLabel => CreatedAt.LocalDateTime.ToString("HH:mm");
    [JsonIgnore]
    public string SourceLabel => $"来自 {(string.IsNullOrEmpty(SourceApp) ? SourceName : SourceApp)}";
    [JsonIgnore]
    public string KindGlyph => Kind switch { "image" => "\uE8B9", "files" => "\uE8B7", _ => "\uE7C3" };
    [JsonIgnore]
    public string Title => !string.IsNullOrWhiteSpace(CustomTitle) ? CustomTitle : Kind switch
    {
        "files" => Content.Files!.Count == 1 ? Path.GetFileName(Content.Files[0].TrimEnd(Path.DirectorySeparatorChar)) : $"{Content.Files!.Count} 个文件或文件夹",
        "image" => "PNG 图片",
        _ => Preview.Length > 60 ? Preview[..60] + "…" : Preview.Length > 0 ? Preview : "空文本"
    };
    [JsonIgnore]
    public string Preview => Kind switch
    {
        "files" => string.Join(Environment.NewLine, Content.Files ?? Array.Empty<string>()),
        "image" => $"PNG · {SizeBytes / 1024.0:N1} KiB",
        _ => Encoding.UTF8.GetString(Content.Representations?.FirstOrDefault(r => r.Mime == "text/plain")?.Data ?? Content.Data)
    };
}

/// <summary>User-local history only. Clearing references never deletes original or pasted files.</summary>
public sealed class ClipboardHistory : IDisposable
{
    private readonly Func<DateTimeOffset> now;
    private readonly Func<ClipboardEntry, bool>? pinned;
    public int RetentionDays { get; private set; }
    private readonly string directory;
    private readonly string file;
    private readonly Channel<ClipboardEntry[]> saves = Channel.CreateBounded<ClipboardEntry[]>(new BoundedChannelOptions(1)
    { FullMode = BoundedChannelFullMode.DropOldest, SingleReader = true, SingleWriter = true });
    private readonly Task writer;
    private bool disposed;
    public ObservableCollection<ClipboardEntry> Entries { get; } = new();
    public string? StorageError { get; private set; }

    public ClipboardHistory() : this(30) { }

    public ClipboardHistory(int retentionDays) : this(
        Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "DeskFuse", "desktop"),
        retentionDays, () => DateTimeOffset.UtcNow) { }

    // Explicit storage and clock keep migration/retention checks away from user history.
    internal ClipboardHistory(string directory, int retentionDays, Func<DateTimeOffset> now)
        : this(directory, retentionDays, now, null) { }

    /// <param name="pinned">手动列表成员豁免保留期；加载时的过期清理同样遵守。</param>
    internal ClipboardHistory(string directory, int retentionDays, Func<DateTimeOffset> now, Func<ClipboardEntry, bool>? pinned)
    {
        ArgumentOutOfRangeException.ThrowIfLessThan(retentionDays, 1);
        RetentionDays = retentionDays;
        this.now = now;
        this.pinned = pinned;
        this.directory = directory;
        file = Path.Combine(directory, "clipboard-history-v1.json");
        try
        {
            Directory.CreateDirectory(directory);
            if (File.Exists(file))
            {
                var entries = JsonSerializer.Deserialize<ClipboardEntry[]>(File.ReadAllText(file)) ?? [];
                var seen = new HashSet<(string Source, string Fingerprint)>();
                foreach (var entry in entries)
                    if (Valid(entry) && !Expired(entry)
                        && seen.Add((entry.SourceId, entry.Content.Fingerprint()))) Entries.Add(entry);
                Trim();
            }
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or JsonException or ArgumentException)
        { StorageError = "无法读取剪贴板历史；原文件保留，不会自动覆盖"; }
        writer = Task.Run(SaveLoop);
    }

    private static bool Valid(ClipboardEntry? entry)
    {
        if (entry is null || entry.Content is null || entry.Content.Data is null || entry.Id is null
            || entry.SourceName is null || entry.SourceId is null
            || entry.Content.Representations?.Any(r => r is null || r.Data is null) is true) return false;
        if (entry.Content.Mime == ClipboardContent.LocalFilesMime)
            return entry.Content.Files is { Count: > 0 } && entry.SourceId == "local"
                   && entry.Content.Files.All(p => p is not null && Path.IsPathFullyQualified(p) && !p.StartsWith(@"\\", StringComparison.Ordinal));
        return entry.Content.Files is null && entry.Content.Mime is "text/plain" or "image/png" or "text/html" or "text/rtf";
    }

    public ClipboardEntry Add(ClipboardContent content, string sourceId, string sourceName, string? sourceApp = null)
    {
        ObjectDisposedException.ThrowIf(disposed, this);
        if (content.Files is not null && sourceId != "local") throw new InvalidOperationException("远端绝对文件路径不可进入本地历史");
        var entry = new ClipboardEntry(Guid.NewGuid().ToString("N"), content, sourceId, sourceName, now())
        {
            SourceApp = sourceId == "local" ? sourceApp : null,
        };
        if (!Valid(entry)) throw new InvalidOperationException("历史内容格式无效");
        var fingerprint = content.Fingerprint();
        // Preserve source and every rich representation; repair only exact duplicates.
        var duplicates = Entries.Where(e => e.SourceId == sourceId && e.Content.Fingerprint() == fingerprint).ToArray();
        entry = entry with { CustomTitle = duplicates.FirstOrDefault(e => !string.IsNullOrWhiteSpace(e.CustomTitle))?.CustomTitle };
        foreach (var duplicate in duplicates) Entries.Remove(duplicate);
        Entries.Insert(0, entry); Trim(); ScheduleSave(); return entry;
    }

    private void Trim()
    {
        for (var i = 0; i < Entries.Count;)
        {
            var entry = Entries[i];
            if (Expired(entry)) Entries.RemoveAt(i);
            else i++;
        }
    }

    private bool Expired(ClipboardEntry entry) =>
        pinned?.Invoke(entry) != true && (now() - entry.CreatedAt).TotalDays > RetentionDays;

    public void SetRetentionDays(int days)
    {
        ObjectDisposedException.ThrowIf(disposed, this);
        ArgumentOutOfRangeException.ThrowIfLessThan(days, 1);
        RetentionDays = days;
        Prune();
    }

    public void Prune()
    {
        ObjectDisposedException.ThrowIf(disposed, this);
        int count = Entries.Count;
        Trim();
        if (Entries.Count != count) ScheduleSave();
    }

    public ClipboardEntry SetTitle(string id, string? title)
    {
        ObjectDisposedException.ThrowIf(disposed, this);
        if (StorageError is not null) throw new IOException(StorageError);
        Prune();
        int index = Entries.ToList().FindIndex(entry => entry.Id == id);
        if (index < 0) throw new InvalidOperationException("记录已删除或过期，请重新选择。");
        var normalized = string.Concat((title ?? "").Trim().EnumerateRunes().Take(200).Select(rune => rune.ToString()));
        var updated = Entries[index] with { CustomTitle = normalized.Length == 0 ? null : normalized };
        Entries[index] = updated;
        ScheduleSave();
        return updated;
    }

    public void Remove(ClipboardEntry entry) { Entries.Remove(entry); ScheduleSave(); }
    public void Clear() { Entries.Clear(); ScheduleSave(); }

    private void ScheduleSave()
    {
        // A malformed/unreadable old index is not replaced silently by a blank one.
        if (StorageError is null && !disposed) saves.Writer.TryWrite(Entries.ToArray());
    }

    private async Task SaveLoop()
    {
        await foreach (var snapshot in saves.Reader.ReadAllAsync())
        {
            var temporary = Path.Combine(directory, $".history-{Guid.NewGuid():N}.tmp");
            try
            {
                await using (var output = new FileStream(temporary, FileMode.CreateNew, FileAccess.Write, FileShare.None, 65536, FileOptions.Asynchronous))
                { await JsonSerializer.SerializeAsync(output, snapshot); await output.FlushAsync(); output.Flush(flushToDisk: true); }
                File.Move(temporary, file, overwrite: true);
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or JsonException)
            { StorageError = "剪贴板历史保存失败；当前内容仍在内存中"; }
            finally { try { if (File.Exists(temporary)) File.Delete(temporary); } catch (IOException) { } catch (UnauthorizedAccessException) { } }
        }
    }

    public void Dispose()
    {
        if (disposed) return;
        ScheduleSave(); disposed = true; saves.Writer.TryComplete();
        // No dispatcher continuation is needed by the writer, so normal exit can flush safely.
        try { writer.GetAwaiter().GetResult(); } catch (IOException) { }
    }
}
