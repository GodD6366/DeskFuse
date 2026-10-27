using System.Text.Json;

namespace DeskFuse.Windows;

/// Authenticated IPC projection of the shared Rust canonical clipboard library.
/// OCR runs in the interactive Windows session and is written back as an index mutation.
internal sealed class CanonicalLibraryClient(ServiceClient service)
{
    private static readonly JsonSerializerOptions JsonOptions = new(JsonSerializerDefaults.Web);
    private readonly SemaphoreSlim writes = new(1, 1);

    public async Task<string> QueryAsync(string request, CancellationToken token = default) =>
        await SendAsync("LibraryQuery", request, token);

    public async Task<string> MutateAsync(string request, CancellationToken token = default) =>
        await SendAsync("LibraryMutate", request, token);

    public async Task<CanonicalState> StateAsync(CancellationToken token = default) =>
        JsonSerializer.Deserialize<CanonicalState>(await QueryAsync("{\"op\":\"state\"}", token), JsonOptions)
        ?? throw new IOException("内容库状态为空。");

    public async Task<CanonicalPage> PageAsync(
        string? query,
        string? listId,
        string kind,
        string? source,
        int offset,
        int limit,
        IReadOnlyList<CanonicalRule>? rules = null,
        CancellationToken token = default)
    {
        var request = JsonSerializer.Serialize(CanonicalLibraryQuery.Build(
            query, listId, kind, source, offset, limit, rules, DateTimeOffset.Now, TimeZoneInfo.Local));
        return JsonSerializer.Deserialize<CanonicalPage>(await QueryAsync(request, token), JsonOptions)
            ?? throw new IOException("内容库查询结果为空。");
    }

    public async Task<Dictionary<string, long>> CountsAsync(
        IReadOnlyList<CanonicalList> lists,
        CancellationToken token = default)
    {
        var now = DateTimeOffset.Now;
        var zone = TimeZoneInfo.Local;
        var requests = new List<object> { CanonicalLibraryQuery.Build(null, null, "all", null, 0, 1, null, now, zone) };
        requests.AddRange(lists.Select(list => CanonicalLibraryQuery.Build(
            null, list.Id, "all", null, 0, 1, list.Rules, now, zone)));
        string json = await QueryAsync(JsonSerializer.Serialize(new
        {
            op = "counts",
            requests,
        }), token);
        return JsonSerializer.Deserialize<Dictionary<string, long>>(json, JsonOptions) ?? [];
    }

    public async Task<ClipboardContent> ContentAsync(ulong id, CancellationToken token = default)
    {
        var reply = await service.SendAsync(ServiceClient.Command("LibraryContent", new { id }), token);
        var payload = reply.GetProperty("LibraryContent");
        var representations = payload.Array("formats")
            .Select(format => new ClipboardRepresentation(
                format.String("mime"),
                Convert.FromBase64String(format.String("data_base64"))))
            .ToArray();
        var files = payload.Array("files")
            .Where(value => value.ValueKind == JsonValueKind.String)
            .Select(value => value.GetString() ?? "")
            .Where(path => path.Length > 0)
            .ToArray();
        if (files.Length > 0)
            return new ClipboardContent(ClipboardContent.LocalFilesMime, Array.Empty<byte>(), files);
        var primary = representations.FirstOrDefault(item => item.Mime == "image/png")
            ?? representations.FirstOrDefault(item => item.Mime == "text/plain")
            ?? representations.FirstOrDefault()
            ?? throw new IOException("内容库条目没有可恢复的格式。");
        return new ClipboardContent(primary.Mime, primary.Data, Representations: representations);
    }

    public async Task<byte[]?> ThumbnailAsync(ulong id, int pixelSize = 160, CancellationToken token = default)
    {
        var reply = await service.SendAsync(ServiceClient.Command("LibraryThumbnail", new { id, max_pixel_size = Math.Clamp(pixelSize, 64, 480) }), token);
        string encoded = reply.GetProperty("LibraryThumbnail").String("data_base64");
        return encoded.Length == 0 ? null : Convert.FromBase64String(encoded);
    }

    public Task<string> SetTitleAsync(ulong id, string? title, CancellationToken token = default) =>
        MutateAsync(JsonSerializer.Serialize(new { op = "set_item_title", id, title = title ?? "" }), token);

    public Task<string> SetTextAsync(ulong id, string text, byte[]? rtf = null, CancellationToken token = default) =>
        MutateAsync(JsonSerializer.Serialize(new
        {
            op = "set_item_text",
            id,
            text,
            rtf = rtf is { Length: > 0 } ? rtf.Select(value => (int)value).ToArray() : null,
        }), token);

    public Task<string> DeleteAsync(IEnumerable<ulong> ids, CancellationToken token = default) =>
        MutateAsync(JsonSerializer.Serialize(new { op = "delete_items", ids = ids.ToArray() }), token);

    public Task<string> ClearHistoryAsync(CancellationToken token = default) =>
        MutateAsync(JsonSerializer.Serialize(new { op = "clear_history" }), token);

    public Task<string> MembershipAsync(string listId, ulong id, bool present, CancellationToken token = default) =>
        MutateAsync(JsonSerializer.Serialize(new { op = "membership", list_id = listId, history_id = id, present }), token);

    public Task<string> SaveListAsync(CanonicalList list, CancellationToken token = default) =>
        MutateAsync(JsonSerializer.Serialize(new
        {
            op = "save_list",
            list = new
            {
                id = list.Id,
                name = list.Name,
                kind = list.Kind,
                match_mode = list.MatchMode,
                rules = list.Rules,
                position = list.Position,
            },
        }), token);

    public Task<string> DeleteListAsync(string id, CancellationToken token = default) =>
        MutateAsync(JsonSerializer.Serialize(new { op = "delete_list", id }), token);

    public Task<string> ReorderAsync(IEnumerable<string> ids, CancellationToken token = default) =>
        MutateAsync(JsonSerializer.Serialize(new { op = "reorder", ids = ids.ToArray() }), token);

    public Task<string> ClearRemoteIndexAsync(CancellationToken token = default) =>
        MutateAsync(JsonSerializer.Serialize(new { op = "clear_remote_index" }), token);

    public Task<string> SettingsAsync(object settings, CancellationToken token = default) =>
        MutateAsync(JsonSerializer.Serialize(new { op = "settings", settings }), token);

    public async Task<ulong?> IngestAsync(
        ClipboardContent content,
        string deviceId,
        string source,
        string? application,
        ClipboardEntry? legacy = null,
        CancellationToken token = default,
        long? copiedAtMs = null)
    {
        await writes.WaitAsync(token);
        try
        {
            var representations = content.Representations ?? [new ClipboardRepresentation(content.Mime, content.Data)];
            var formats = representations.Select(item => new
            {
                mime = item.Mime,
                data_base64 = Convert.ToBase64String(item.Data),
            }).ToArray();
            var reply = await service.SendAsync(ServiceClient.Command("LibraryIngest", new
            {
                formats,
                files = content.Files ?? [],
                source,
                device_id = deviceId,
                app_id = legacy?.SourceApp ?? application,
                app_name = legacy?.SourceApp ?? application,
                copied_at_ms = copiedAtMs ?? (legacy?.CreatedAt ?? DateTimeOffset.UtcNow).ToUnixTimeMilliseconds(),
            }), token);
            string json = reply.GetProperty("LibraryResult").GetProperty("json").GetString() ?? "null";
            ulong? id = JsonSerializer.Deserialize<ulong?>(json);
            if (id is not > 0) return null;
            if (!string.IsNullOrWhiteSpace(legacy?.CustomTitle))
                await MutateAsync(JsonSerializer.Serialize(new { op = "set_item_title", id = id.Value, title = legacy.CustomTitle }), token);
            var png = representations.FirstOrDefault(item => item.Mime == "image/png")?.Data;
            if (png is not null && png.Length > 0)
            {
                try
                {
                    string text = await WindowsOcr.RecognizeAsync(png, token);
                    if (text.Length > 0)
                        await MutateAsync(JsonSerializer.Serialize(new { op = "set_item_ocr", id = id.Value, text }), token);
                }
                catch (InvalidOperationException) { /* Missing OCR languages do not block history ingestion. */ }
            }
            return id;
        }
        finally { writes.Release(); }
    }

    public async Task MigrateAsync(IReadOnlyList<ClipboardEntry> entries, ClipboardLibrary library, int retentionDays, CancellationToken token = default)
    {
        string marker = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "DeskFuse", "desktop", "canonical-migration-v2-profile-path.done");
        if (File.Exists(marker)) return;
        await MutateAsync(JsonSerializer.Serialize(new { op = "settings", settings = new { retention_days = retentionDays } }), token);
        var migrated = new Dictionary<string, ulong>(StringComparer.Ordinal);
        foreach (var entry in entries.Reverse())
        {
            var id = await IngestAsync(entry.Content, entry.SourceId, entry.SourceName, entry.SourceApp, entry, token);
            if (id is > 0) migrated[entry.Id] = id.Value;
        }
        for (int index = 0; index < library.Lists.Count; index++)
        {
            var list = library.Lists[index];
            await MutateAsync(JsonSerializer.Serialize(new
            {
                op = "save_list",
                list = new { id = list.Id, name = list.Name, kind = "manual", match_mode = "all", rules = Array.Empty<object>(), position = index },
            }), token);
            foreach (string member in list.Members)
                if (migrated.TryGetValue(member, out ulong id))
                    await MutateAsync(JsonSerializer.Serialize(new { op = "membership", list_id = list.Id, history_id = id, present = true }), token);
        }
        // Keep the JSON source untouched. A failed or partial migration is safe to retry;
        // canonical content digests and stable list IDs make the retry idempotent.
        await File.WriteAllTextAsync(marker, DateTimeOffset.UtcNow.ToString("O"), token);
    }

    private async Task<string> SendAsync(string command, string request, CancellationToken token)
    {
        var reply = await service.SendAsync(ServiceClient.Command(command, new { request }), token);
        return reply.GetProperty("LibraryResult").GetProperty("json").GetString()
            ?? throw new IOException("服务未返回内容库结果。");
    }
}
