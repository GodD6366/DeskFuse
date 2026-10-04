using System.Collections.ObjectModel;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;
using System.Threading.Channels;

namespace DeskFuse.Windows;

public sealed record ClipboardEntry(string Id, ClipboardContent Content, string SourceId,
    string SourceName, DateTimeOffset CreatedAt)
{
    [JsonIgnore]
    public string Kind => Content.Files is { Count: > 0 } ? "files" : Content.Mime == "image/png" ? "image" : "text";
    [JsonIgnore]
    public long SizeBytes => Content.SizeBytes;
    /// <summary>Preformatted timestamp: WinUI bindings have no StringFormat.</summary>
    [JsonIgnore]
    public string CreatedLabel => CreatedAt.LocalDateTime.ToString("g");
    [JsonIgnore]
    public string Title => Kind switch
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
    private const int MaxEntries = 200;
    private const long MaxBytes = 64 * 1024 * 1024;
    private static readonly TimeSpan Retention = TimeSpan.FromDays(30);
    private readonly string directory;
    private readonly string file;
    private readonly Channel<ClipboardEntry[]> saves = Channel.CreateBounded<ClipboardEntry[]>(new BoundedChannelOptions(1)
    { FullMode = BoundedChannelFullMode.DropOldest, SingleReader = true, SingleWriter = true });
    private readonly Task writer;
    private bool disposed;
    public ObservableCollection<ClipboardEntry> Entries { get; } = new();
    public string? StorageError { get; private set; }

    public ClipboardHistory()
    {
        directory = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "DeskFuse", "desktop");
        file = Path.Combine(directory, "clipboard-history-v1.json");
        try
        {
            Directory.CreateDirectory(directory);
            if (File.Exists(file))
            {
                if (new FileInfo(file).Length > MaxBytes * 2) throw new IOException("历史索引超过限制");
                var entries = JsonSerializer.Deserialize<ClipboardEntry[]>(File.ReadAllText(file)) ?? [];
                var seen = new HashSet<(string Source, string Fingerprint)>();
                foreach (var entry in entries.Take(MaxEntries))
                    if (Valid(entry) && DateTimeOffset.Now - entry.CreatedAt <= Retention
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
            || entry.Content.Representations?.Any(r => r is null || r.Data is null) is true
            || entry.Content.SizeBytes > MaxBytes) return false;
        if (entry.Content.Mime == ClipboardContent.LocalFilesMime)
            return entry.Content.Files is { Count: > 0 and <= 512 } && entry.SourceId == "local"
                   && entry.Content.Files.All(p => p is not null && Path.IsPathFullyQualified(p) && !p.StartsWith(@"\\", StringComparison.Ordinal));
        return entry.Content.Files is null && entry.Content.Mime is "text/plain" or "image/png" or "text/html" or "text/rtf";
    }

    public ClipboardEntry Add(ClipboardContent content, string sourceId, string sourceName)
    {
        ObjectDisposedException.ThrowIf(disposed, this);
        if (content.Files is not null && sourceId != "local") throw new InvalidOperationException("远端绝对文件路径不可进入本地历史");
        var entry = new ClipboardEntry(Guid.NewGuid().ToString("N"), content, sourceId, sourceName, DateTimeOffset.Now);
        if (!Valid(entry)) throw new InvalidOperationException("历史内容无效或过大");
        var fingerprint = content.Fingerprint();
        // Preserve source and every rich representation; repair only exact duplicates.
        foreach (var duplicate in Entries.Where(e => e.SourceId == sourceId && e.Content.Fingerprint() == fingerprint).ToArray())
            Entries.Remove(duplicate);
        Entries.Insert(0, entry); Trim(); ScheduleSave(); return entry;
    }

    private void Trim()
    {
        long size = 0;
        for (var i = 0; i < Entries.Count;)
        {
            size += Entries[i].SizeBytes;
            if (DateTimeOffset.Now - Entries[i].CreatedAt > Retention || i >= MaxEntries || size > MaxBytes) Entries.RemoveAt(i);
            else i++;
        }
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
