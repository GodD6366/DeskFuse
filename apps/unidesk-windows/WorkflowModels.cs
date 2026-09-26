using System.Text.Json.Serialization;

namespace UniDesk.Windows;

/// <summary>JSON shape shared with unidesk-core::workflow::Workflow.</summary>
public sealed class WorkflowRecord
{
    [JsonPropertyName("version")] public uint Version { get; set; } = 2;
    [JsonPropertyName("id")] public string Id { get; set; } = Guid.NewGuid().ToString();
    [JsonPropertyName("name")] public string Name { get; set; } = "新工作流";
    [JsonPropertyName("enabled")] public bool Enabled { get; set; } = true;
    [JsonPropertyName("shortcut")] public ShortcutRecord? Shortcut { get; set; }
    [JsonPropertyName("steps")] public List<WorkflowStepRecord> Steps { get; set; } = [WorkflowStepRecord.Copy()];
    [JsonPropertyName("input_type")] public string InputType { get; set; } = "Any";
    [JsonIgnore] public string ShortcutLabel => Shortcut is null ? "无快捷键" : $"{Shortcut.KeyCode}, {Shortcut.Modifiers}";
}

public sealed class ShortcutRecord
{
    [JsonPropertyName("key_code")] public ushort KeyCode { get; set; }
    [JsonPropertyName("modifiers")] public ulong Modifiers { get; set; }
}

public sealed class WorkflowStepRecord
{
    [JsonPropertyName("kind")] public string Kind { get; set; } = "Copy";
    [JsonPropertyName("quality")] public byte Quality { get; set; } = 90;
    [JsonPropertyName("target_bytes")] public ulong? TargetBytes { get; set; }
    [JsonPropertyName("max_edge")] public uint? MaxEdge { get; set; }
    [JsonPropertyName("scale_percent")] public uint? ScalePercent { get; set; }
    [JsonPropertyName("format")] public string Format { get; set; } = "Png";
    [JsonPropertyName("shortcut_id")] public string ShortcutId { get; set; } = "";
    [JsonPropertyName("shortcut_name")] public string ShortcutName { get; set; } = "";
    [JsonPropertyName("output_shape")] public string OutputShape { get; set; } = "FollowSource";
    [JsonPropertyName("find")] public string Find { get; set; } = "";
    [JsonPropertyName("replacement")] public string Replacement { get; set; } = "";
    [JsonPropertyName("prefix")] public string Prefix { get; set; } = "";
    [JsonPropertyName("suffix")] public string Suffix { get; set; } = "";
    [JsonPropertyName("shortcut_output")] public string ShortcutOutput { get; set; } = "Any";
    public static WorkflowStepRecord Copy() => new();
}
