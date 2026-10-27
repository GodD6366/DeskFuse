using System.Text;
using System.Text.Json.Serialization;

namespace DeskFuse.Windows;

/// <summary>Read-only projection of the shared canonical library. Payload bytes
/// are never part of a page; rows carry metadata and a preview only.</summary>
public sealed class CanonicalItem
{
    [JsonPropertyName("id")] public ulong Id { get; set; }
    [JsonPropertyName("copied_at_ms")] public ulong? CopiedAtMs { get; set; }
    [JsonPropertyName("mime")] public string Mime { get; set; } = "text/plain";
    [JsonPropertyName("length")] public long Length { get; set; }
    [JsonPropertyName("preview")] public string Preview { get; set; } = "";
    [JsonPropertyName("source")] public string Source { get; set; } = "";
    [JsonPropertyName("has_image")] public bool HasImage { get; set; }
    [JsonPropertyName("file_count")] public int FileCount { get; set; }
    [JsonPropertyName("source_device_id")] public string? SourceDeviceId { get; set; }
    [JsonPropertyName("source_app_id")] public string? SourceAppId { get; set; }
    [JsonPropertyName("source_app_name")] public string? SourceAppName { get; set; }
    [JsonPropertyName("kind")] public string Kind { get; set; } = "text";
    [JsonPropertyName("available")] public bool Available { get; set; }
    [JsonPropertyName("title")] public string Title { get; set; } = "";
    [JsonPropertyName("ocr_text")] public string OcrText { get; set; } = "";
    [JsonPropertyName("ocr_version")] public ulong OcrVersion { get; set; }
    [JsonPropertyName("remote")] public bool Remote { get; set; }
    [JsonPropertyName("sort_at_ms")] public ulong SortAtMs { get; set; }
    [JsonPropertyName("memberships")] public List<string> Memberships { get; set; } = [];
    [JsonPropertyName("related_ids")] public List<ulong> RelatedIds { get; set; } = [];
}

public sealed class CanonicalPage
{
    [JsonPropertyName("items")] public List<CanonicalItem> Items { get; set; } = [];
    [JsonPropertyName("total")] public long Total { get; set; }
    [JsonPropertyName("next_offset")] public long? NextOffset { get; set; }
    [JsonPropertyName("next_cursor")] public CanonicalCursor? NextCursor { get; set; }
    [JsonPropertyName("revision")] public long Revision { get; set; }
}

public sealed class CanonicalCursor
{
    [JsonPropertyName("sort_at_ms")] public ulong SortAtMs { get; set; }
    [JsonPropertyName("id")] public ulong Id { get; set; }
}

public sealed class CanonicalList
{
    [JsonPropertyName("id")] public string Id { get; set; } = "";
    [JsonPropertyName("name")] public string Name { get; set; } = "";
    [JsonPropertyName("kind")] public string Kind { get; set; } = "manual";
    [JsonPropertyName("match_mode")] public string MatchMode { get; set; } = "all";
    [JsonPropertyName("rules")] public List<CanonicalRule> Rules { get; set; } = [];
    [JsonPropertyName("position")] public long Position { get; set; }
    public bool IsSmart => Kind == "smart";
}

public sealed class CanonicalRule
{
    [JsonPropertyName("field")] public string Field { get; set; } = "";
    [JsonPropertyName("op")] public string Op { get; set; } = "";
    [JsonPropertyName("value")] public string Value { get; set; } = "";
    [JsonPropertyName("value2")] public string Value2 { get; set; } = "";
}

public sealed class CanonicalApp
{
    [JsonPropertyName("bundle_id")] public string BundleId { get; set; } = "";
    [JsonPropertyName("name")] public string Name { get; set; } = "";
}

public sealed class CanonicalDevice
{
    [JsonPropertyName("id")] public string Id { get; set; } = "";
    [JsonPropertyName("name")] public string Name { get; set; } = "";
}

public sealed class CanonicalSettings
{
    [JsonPropertyName("retention_days")] public int RetentionDays { get; set; } = 30;
    [JsonPropertyName("excluded_apps")] public List<CanonicalApp> ExcludedApps { get; set; } = [];
    [JsonPropertyName("shortcut")] public CanonicalShortcut Shortcut { get; set; } = new();
}

public sealed class CanonicalShortcut
{
    [JsonPropertyName("enabled")] public bool Enabled { get; set; }
    [JsonPropertyName("key_code")] public ushort KeyCode { get; set; }
    [JsonPropertyName("modifiers")] public ulong Modifiers { get; set; }
}

public sealed class CanonicalState
{
    [JsonPropertyName("lists")] public List<CanonicalList> Lists { get; set; } = [];
    [JsonPropertyName("settings")] public CanonicalSettings Settings { get; set; } = new();
    [JsonPropertyName("apps")] public List<CanonicalApp> Apps { get; set; } = [];
    [JsonPropertyName("devices")] public List<CanonicalDevice> Devices { get; set; } = [];
    [JsonPropertyName("revision")] public long Revision { get; set; }
}

/// <summary>Adapts one canonical metadata row to the row type shared with the
/// local history UI. Bodies stay unloaded until the row is actually used.</summary>
internal static class CanonicalRow
{
    public static ClipboardEntry ToEntry(CanonicalItem item)
    {
        string kind = item.FileCount > 0 ? "files" : item.HasImage || item.Kind == "image" ? "image" : "text";
        string preview = kind switch
        {
            "files" => item.Preview,
            "image" => $"{MimeLabel(item.Mime)} · {item.Length / 1024.0:N1} KiB",
            _ => item.Preview,
        };
        // Text previews are small and safe to carry as a placeholder; image and
        // file rows load their real payload on demand.
        var content = new ClipboardContent(
            kind == "text" ? "text/plain" : kind == "image" ? "image/png" : ClipboardContent.LocalFilesMime,
            kind == "text" ? Encoding.UTF8.GetBytes(item.Preview) : Array.Empty<byte>());
        const ulong lastSupportedUnixMs = 253402300799999;
        bool copyTimeKnown = item.CopiedAtMs is > 0 and <= lastSupportedUnixMs;
        ulong timestamp = copyTimeKnown ? item.CopiedAtMs!.Value : Math.Min(item.SortAtMs, lastSupportedUnixMs);
        var copied = DateTimeOffset.FromUnixTimeMilliseconds((long)timestamp);
        return new ClipboardEntry(item.Id.ToString(), content,
            string.IsNullOrEmpty(item.Source) ? "内容库" : item.Source,
            string.IsNullOrEmpty(item.Source) ? "内容库" : item.Source,
            copied)
        {
            CustomTitle = string.IsNullOrWhiteSpace(item.Title) ? null : item.Title,
            SourceApp = string.IsNullOrWhiteSpace(item.SourceAppName) ? null : item.SourceAppName,
            CopyTimeKnown = copyTimeKnown,
            CanonicalId = item.Id,
            Available = item.Available,
            Remote = item.Remote,
            OcrText = item.OcrText,
            MembershipIds = item.Memberships,
            ContentLoaded = false,
            KindOverride = kind,
            PreviewOverride = preview,
            SizeOverride = kind == "files" ? null : item.Length,
            CanonicalFileCount = item.FileCount,
        };
    }

    private static string MimeLabel(string mime) => mime switch
    {
        "image/jpeg" => "JPEG",
        "image/heic" or "image/heif" => "HEIC",
        "image/gif" => "GIF",
        "image/webp" => "WebP",
        "image/tiff" => "TIFF",
        _ => "PNG",
    };
}
