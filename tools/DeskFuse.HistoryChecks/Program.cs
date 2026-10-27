using System.Text;
using System.Text.Json;
using DeskFuse.Windows;

var checks = new (string Name, Action<string> Run)[]
{
    ("Detail title is never derived from the body", DetailTitle),
    ("Thumbnail cache respects LRU count and byte bounds", ThumbnailCacheBounds),
    ("Legacy history keeps all representations when a title is edited", LegacyTitleRoundTrip),
    ("Title changes and exact duplicate capture preserve content identity", DuplicateTitle),
    ("Unicode titles are limited by scalar values and can be cleared", UnicodeTitle),
    ("Expired history is removed by maintenance without another capture", Maintenance),
    ("Configured retention is applied before loading legacy entries", LoadRetention),
    ("Shorter retention prunes only expired references", ShortenRetention),
    ("No count or aggregate byte limit truncates retained history", NoHistoryLimit),
    ("Undated canonical rows use stable ordering without inventing a copy time", UndatedCanonicalRows),
    ("Background rows retain identity without collection resets", StableHistoryRows),
    ("Clipboard hotkeys persist, disable and reject invalid settings", ClipboardHotkeySettings),
    ("Calendar filters preserve 23-hour and 25-hour local days", CalendarDaylightSaving),
    ("Smart-list count and item requests use the same date windows", CalendarCountParity),
    ("Date ranges use local calendar boundaries and reject malformed rules", CalendarRanges),
    ("Malformed history is preserved and title editing reports failure", CorruptHistory),
    ("Device history replacement is atomic and beats queued saves", AtomicReplacement),
};
foreach (var check in checks)
{
    var directory = Path.Combine(Path.GetTempPath(), "DeskFuse-history-checks-" + Guid.NewGuid().ToString("N"));
    Directory.CreateDirectory(directory);
    try { check.Run(directory); Console.WriteLine("PASS: " + check.Name); }
    finally { Directory.Delete(directory, recursive: true); }
}
Console.WriteLine($"{checks.Length} history checks passed. No system clipboard or user history was accessed.");

static ClipboardContent Text(string value) => new("text/plain", Encoding.UTF8.GetBytes(value));
static string Index(string directory) => Path.Combine(directory, "clipboard-history-v1.json");
static void Check(bool condition, string reason) { if (!condition) throw new Exception(reason); }
static void Seed(string directory, params ClipboardEntry[] entries) => File.WriteAllText(Index(directory), JsonSerializer.Serialize(entries));

static void DetailTitle(string directory)
{
    var entry = new ClipboardEntry("1", Text("body"), "local", "local", DateTimeOffset.UtcNow);
    Check(entry.DetailTitle is null && entry.Title == "body", "body became detail title");
    Check((entry with { CustomTitle = "  " }).DetailTitle is null, "blank title is displayed");
    Check((entry with { CustomTitle = " name " }).DetailTitle == "name", "explicit title was lost");
}

static void ThumbnailCacheBounds(string directory)
{
    var cache = new BoundedCache<string, string>(2, 10);
    cache.Put("a", "a", 4); cache.Put("b", "b", 4);
    Check(cache.TryGet("a", out _), "cache miss");
    cache.Put("c", "c", 4);
    Check(!cache.TryGet("b", out _) && cache.Count == 2 && cache.Cost == 8, "LRU eviction failed");
    cache.Put("a", "new", 8);
    Check(!cache.TryGet("c", out _) && cache.Cost == 8, "byte bound failed");
    cache.Put("large", "large", 11);
    Check(cache.Count == 1 && cache.Cost == 8, "oversized entry retained");
    cache.Remove("a");
    Check(cache.Count == 0 && cache.Cost == 0, "remove did not release cost");
}

static void LegacyTitleRoundTrip(string directory)
{
    var now = DateTimeOffset.UtcNow;
    // Original v1 JSON has no CustomTitle field.
    var body = new ClipboardContent("text/plain", Encoding.UTF8.GetBytes("hello"), Representations:
        [new("text/plain", Encoding.UTF8.GetBytes("hello")), new("text/html", Encoding.UTF8.GetBytes("<b>hello</b>"))]);
    var legacy = new ClipboardEntry("legacy", body, "peer-1", "Mac", now);
    var node = JsonSerializer.SerializeToNode(new[] { legacy })!;
    node[0]!.AsObject().Remove("CustomTitle");
    File.WriteAllText(Index(directory), node.ToJsonString());
    using (var history = new ClipboardHistory(directory, 30, () => now))
    {
        var edited = history.SetTitle("legacy", "  Useful snippet  ");
        Check(edited.Title == "Useful snippet", "title not normalized");
        Check(edited.Content.Fingerprint() == body.Fingerprint(), "title modified the native payload");
    }
    using var reopened = new ClipboardHistory(directory, 30, () => now);
    var entry = reopened.Entries.Single();
    Check(entry.Title == "Useful snippet" && entry.SourceId == "peer-1" && entry.CreatedAt == now, "metadata round trip failed");
    Check(entry.Content.Representations!.Count == 2 && entry.Content.Fingerprint() == body.Fingerprint(), "rich formats lost");
}

static void DuplicateTitle(string directory)
{
    var now = DateTimeOffset.UtcNow;
    using var history = new ClipboardHistory(directory, 30, () => now);
    var first = history.Add(Text("same"), "local", "This PC");
    history.SetTitle(first.Id, "Saved title");
    now = now.AddHours(1);
    var second = history.Add(Text("same"), "local", "This PC");
    Check(history.Entries.Count == 1 && second.Title == "Saved title", "recapture dropped the title");
    Check(second.CreatedAt == now && second.Content.Fingerprint() == first.Content.Fingerprint(), "recapture changed content");
    var remote = history.Add(Text("same"), "remote", "Mac");
    Check(history.Entries.Count == 2 && remote.CustomTitle is null, "title leaked between source identities");
    try { history.SetTitle("deleted-id", "title"); throw new Exception("missing row accepted"); }
    catch (InvalidOperationException) { }
}

static void UnicodeTitle(string directory)
{
    using var history = new ClipboardHistory(directory, 30, () => DateTimeOffset.UtcNow);
    var entry = history.Add(Text("original"), "local", "This PC");
    var titled = history.SetTitle(entry.Id, string.Concat(Enumerable.Repeat("😀", 201)));
    Check(titled.CustomTitle!.EnumerateRunes().Count() == 200 && !titled.CustomTitle.Contains('�'), "Unicode scalar was split");
    Check(history.SetTitle(entry.Id, " \t ").Title == "original", "empty title did not restore fallback");
}

static void Maintenance(string directory)
{
    var now = DateTimeOffset.UtcNow;
    using (var history = new ClipboardHistory(directory, 30, () => now))
    {
        history.Add(Text("expires"), "local", "This PC");
        now = now.AddDays(30).AddSeconds(1);
        history.Prune();
        Check(history.Entries.Count == 0, "maintenance did not prune");
    }
    Check(JsonSerializer.Deserialize<ClipboardEntry[]>(File.ReadAllText(Index(directory)))!.Length == 0, "prune not persisted");
}

static void LoadRetention(string directory)
{
    var now = DateTimeOffset.UtcNow;
    Seed(directory, new ClipboardEntry("old", Text("keep for 90 days"), "local", "This PC", now.AddDays(-60)));
    using var history = new ClipboardHistory(directory, 90, () => now);
    Check(history.Entries.Count == 1, "default 30-day pruning ran before configured retention");
    try { history.SetRetentionDays(0); throw new Exception("zero retention accepted"); }
    catch (ArgumentOutOfRangeException) { }
    Check(history.RetentionDays == 90 && history.Entries.Count == 1, "invalid policy modified history");
}

static void ShortenRetention(string directory)
{
    var now = DateTimeOffset.UtcNow;
    var source = Path.Combine(directory, "user-source.txt");
    File.WriteAllText(source, "must survive");
    Seed(directory,
        new("recent", Text("recent"), "local", "This PC", now.AddDays(-1)),
        new("old-file", new ClipboardContent(ClipboardContent.LocalFilesMime, [], Files: [source]), "local", "This PC", now.AddDays(-20)));
    using var history = new ClipboardHistory(directory, 30, () => now);
    history.SetRetentionDays(7);
    Check(history.Entries.Single().Id == "recent", "retention pruned the wrong rows");
    Check(File.ReadAllText(source) == "must survive", "source file was modified");
}

static void NoHistoryLimit(string directory)
{
    var now = DateTimeOffset.UtcNow;
    var entries = Enumerable.Range(0, 201)
        .Select(index => new ClipboardEntry($"entry-{index}", Text($"unique-{index}"), "local", "This PC", now.AddSeconds(-index)))
        .ToList();
    // The old aggregate limit was 64 MiB. A single local history item can be
    // larger than that while still remaining a valid history snapshot; transport
    // limits are enforced by the desktop protocol separately.
    entries.Add(new ClipboardEntry("large", new ClipboardContent("text/plain", new byte[65 * 1024 * 1024]), "local", "This PC", now));
    Seed(directory, entries.ToArray());
    using var history = new ClipboardHistory(directory, 30, () => now);
    Check(history.Entries.Count == 202, "history was truncated by count or aggregate byte size");
    Check(history.Entries.Any(entry => entry.Id == "large" && entry.SizeBytes > 64 * 1024 * 1024), "large local history payload was rejected");
}

static void UndatedCanonicalRows(string directory)
{
    var item = new CanonicalItem { Id = 7, Preview = "undated", SortAtMs = 1700000000000 };
    var first = CanonicalRow.ToEntry(item);
    var second = CanonicalRow.ToEntry(item);
    Check(first.CreatedAt == second.CreatedAt && first.CreatedAt.ToUnixTimeMilliseconds() == 1700000000000,
        "refresh invented a new timestamp for an undated row");
    Check(!first.CopyTimeKnown && first.TimeLabel == "—", "unknown copy time was presented as a real time");
}

static void StableHistoryRows(string directory)
{
    var now = DateTimeOffset.UtcNow;
    var first = new ClipboardEntry("first", Text("first"), "local", "This PC", now);
    var second = new ClipboardEntry("second", Text("second"), "local", "This PC", now);
    var added = new ClipboardEntry("new", Text("new"), "remote", "Mac", now);
    var rows = new System.Collections.ObjectModel.ObservableCollection<ClipboardEntry> { first, second };
    int resets = 0;
    rows.CollectionChanged += (_, e) => { if (e.Action == System.Collections.Specialized.NotifyCollectionChangedAction.Reset) resets++; };
    StableCollection.Apply(rows, new[] { added, first with { }, second with { } }, e => e.Id, (a, b) => a.Preview == b.Preview && a.Title == b.Title);
    Check(ReferenceEquals(rows[1], first) && ReferenceEquals(rows[2], second), "index arrival replaced existing rows");
    StableCollection.Apply(rows, new[] { second with { }, first with { CustomTitle = "updated" } }, e => e.Id, (a, b) => a.Title == b.Title);
    Check(ReferenceEquals(rows[0], second), "moving a row replaced its identity");
    Check(rows[1].Title == "updated" && rows.Count == 2 && resets == 0, "update requires a collection reset");
}

static void ClipboardHotkeySettings(string directory)
{
    string path = Path.Combine(directory, "shortcut.json");
    Check(ClipboardHotkey.Load(path) == ClipboardHotkey.Default, "default shortcut changed");
    var shortcut = new ClipboardHotkey(0x56, 3);
    shortcut.Save(path);
    Check(ClipboardHotkey.Load(path) == shortcut && shortcut.Label == "Ctrl + Alt + V", "shortcut did not round trip");
    var disabled = shortcut with { Enabled = false };
    disabled.Save(path);
    Check(!ClipboardHotkey.Load(path).Enabled, "disabled setting did not persist");
    try { new ClipboardHotkey(0x56, 4).Save(path); throw new Exception("bare Shift chord accepted"); }
    catch (InvalidOperationException) { }
    Check(ClipboardHotkey.Load(path) == disabled, "invalid save damaged the prior shortcut");
}

static JsonElement CalendarQuery(DateTimeOffset now, IReadOnlyList<CanonicalRule> rules, int limit = 50) =>
    JsonSerializer.SerializeToElement(CanonicalLibraryQuery.Build(null, "smart", "all", null, 0, limit,
        rules, now, TimeZoneInfo.FindSystemTimeZoneById("America/New_York")));

static void CalendarDaylightSaving(string directory)
{
    foreach (var (date, hours) in new[] { ("2026-03-08T16:00:00Z", 23), ("2026-11-01T17:00:00Z", 25) })
    {
        var query = CalendarQuery(DateTimeOffset.Parse(date), [new() { Field = "date", Op = "today" }]);
        long start = query.GetProperty("day_start_ms").GetInt64(), end = query.GetProperty("day_end_ms").GetInt64();
        Check(end - start == hours * 3_600_000L, "DST day incorrectly treated as 24 hours");
        var window = query.GetProperty("rule_windows").GetProperty("0");
        Check(window.GetProperty("start").GetInt64() == start && window.GetProperty("end").GetInt64() == end, "today rule differs from day bounds");
        // The next day's yesterday window must include the same full calendar day.
        var next = CalendarQuery(DateTimeOffset.Parse(date).AddDays(1), [new() { Field = "date", Op = "yesterday" }]);
        var previous = next.GetProperty("rule_windows").GetProperty("0");
        Check(previous.GetProperty("start").GetInt64() == start && previous.GetProperty("end").GetInt64() == end, "yesterday does not match the prior calendar day");
    }
}

static void CalendarCountParity(string directory)
{
    CanonicalRule[] rules = [new() { Field = "kind", Value = "text" }, new() { Field = "date", Op = "last_days", Value = "2" }];
    var now = DateTimeOffset.Parse("2026-03-09T16:00:00Z");
    var page = CalendarQuery(now, rules);
    var count = CalendarQuery(now, rules, 1);
    Check(page.GetProperty("rule_windows").GetRawText() == count.GetProperty("rule_windows").GetRawText(), "count and page windows differ");
    var window = page.GetProperty("rule_windows").GetProperty("1");
    Check(window.GetProperty("end").GetInt64() - window.GetProperty("start").GetInt64() == 47 * 3_600_000L, "last two calendar days lost DST boundary");
}

static void CalendarRanges(string directory)
{
    var query = CalendarQuery(DateTimeOffset.Parse("2026-07-01T12:00:00Z"),
        [new() { Field = "date", Op = "range", Value = "2026-03-07", Value2 = "2026-03-08" },
         new() { Field = "date", Op = "range", Value = "not-a-date", Value2 = "2026-03-08" },
         new() { Field = "date", Op = "last_days", Value = "2147483647" }]);
    var windows = query.GetProperty("rule_windows");
    Check(windows.EnumerateObject().Count() == 1, "invalid dates or unbounded day count accepted");
    var window = windows.GetProperty("0");
    Check(window.GetProperty("start").GetInt64() == DateTimeOffset.Parse("2026-03-07T05:00:00Z").ToUnixTimeMilliseconds(), "range borrowed the current summer offset");
    Check(window.GetProperty("end").GetInt64() == DateTimeOffset.Parse("2026-03-09T04:00:00Z").ToUnixTimeMilliseconds(), "range end is not the next local midnight");
}

static void AtomicReplacement(string directory)
{
    var now = DateTimeOffset.UtcNow;
    var history = new ClipboardHistory(directory, 30, () => now);
    history.Add(Text("local-a"), "local", "This PC");
    history.Add(Text("local-b"), "local", "This PC");
    history.Add(Text("queued-save"), "local", "This PC");
    var accepted = history.ReplaceAllAsync(
    [
        new ClipboardEntry("device-new", Text("remote"), "peer", "Mac", now),
        new ClipboardEntry("device-old", Text("expired"), "peer", "Mac", now.AddDays(-31)),
    ]).GetAwaiter().GetResult();
    Check(accepted.Count == 1 && history.Entries.Single().Id == "device-new", "replacement was not applied");
    history.Dispose();
    using var reopened = new ClipboardHistory(directory, 30, () => now);
    Check(reopened.Entries.Count == 1 && reopened.Entries[0].Id == "device-new", "queued save overwrote the replacement");
    Check(reopened.Entries[0].SourceId == "peer" && reopened.Entries[0].Title == "remote", "replacement metadata lost");
}

static void CorruptHistory(string directory)
{
    const string damaged = "{broken history";
    File.WriteAllText(Index(directory), damaged);
    using (var history = new ClipboardHistory(directory, 30, () => DateTimeOffset.UtcNow))
    {
        Check(history.StorageError is not null, "corrupt file accepted");
        var entry = history.Add(Text("in memory"), "local", "This PC");
        try { history.SetTitle(entry.Id, "not saved"); throw new Exception("persist failure was hidden"); }
        catch (IOException) { }
    }
    Check(File.ReadAllText(Index(directory)) == damaged, "corrupt history was overwritten");
}
