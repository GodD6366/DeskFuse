using System.Text;
using System.Text.Json;
using DeskFuse.Windows;

var checks = new (string Name, Action<string> Run)[]
{
    ("Legacy history keeps all representations when a title is edited", LegacyTitleRoundTrip),
    ("Title changes and exact duplicate capture preserve content identity", DuplicateTitle),
    ("Unicode titles are limited by scalar values and can be cleared", UnicodeTitle),
    ("Expired history is removed by maintenance without another capture", Maintenance),
    ("Configured retention is applied before loading legacy entries", LoadRetention),
    ("Shorter retention prunes only expired references", ShortenRetention),
    ("No count or aggregate byte limit truncates retained history", NoHistoryLimit),
    ("Malformed history is preserved and title editing reports failure", CorruptHistory),
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
