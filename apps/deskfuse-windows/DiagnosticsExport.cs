using System.Diagnostics;
using System.Text.Json;
using System.Text.Json.Nodes;

namespace DeskFuse.Windows;

/// <summary>
/// Writes a timestamped diagnostics file the user can attach to a report.
/// Collects the service snapshot, client/runtime state and memory counters;
/// it never includes clipboard content, device trust material or file paths
/// beyond storage directories.
/// </summary>
internal static class DiagnosticsExport
{
    public static string Export(JsonElement? snapshot, int historyEntries, int historyRetentionDays, string? clipboardStorageError)
    {
        string directory = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "DeskFuse", "diagnostics");
        Directory.CreateDirectory(directory);
        string path = Path.Combine(directory, $"deskfuse-diagnostics-{DateTime.Now:yyyyMMdd-HHmmss}.json");
        using var process = Process.GetCurrentProcess();
        var payload = new JsonObject
        {
            ["captured_at"] = DateTimeOffset.Now.ToString("O"),
            ["app"] = new JsonObject { ["version"] = AppVersion.Display },
            ["client_memory"] = new JsonObject
            {
                ["working_set_mb"] = Math.Round(process.WorkingSet64 / 1048576.0, 1),
                ["private_mb"] = Math.Round(process.PrivateMemorySize64 / 1048576.0, 1),
                ["gc_total_mb"] = Math.Round(GC.GetTotalMemory(false) / 1048576.0, 1),
            },
            ["clipboard_history"] = new JsonObject
            {
                ["entries"] = historyEntries,
                ["retention_days"] = historyRetentionDays,
                ["storage_error"] = clipboardStorageError,
            },
        };
        if (snapshot is JsonElement state && state.ValueKind == JsonValueKind.Object)
            payload["service"] = JsonNode.Parse(state.GetRawText());
        File.WriteAllText(path, payload.ToJsonString(new JsonSerializerOptions { WriteIndented = true }));
        return path;
    }
}
