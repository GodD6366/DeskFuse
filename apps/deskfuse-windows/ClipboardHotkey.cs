using System.Text.Json;

namespace DeskFuse.Windows;

internal sealed record ClipboardHotkey(uint Key = 0x56, uint Modifiers = 6, bool Enabled = true)
{
    public static ClipboardHotkey Default => new();
    public bool IsValid => Key is >= 8 and <= 254 && Key is not (16 or 17 or 18 or 91 or 92)
        && (Modifiers & ~15u) == 0 && (Modifiers & 11u) != 0;
    public string Label => !Enabled ? "已关闭" : string.Join(" + ", new[]
    {
        (Modifiers & 2) != 0 ? "Ctrl" : null, (Modifiers & 1) != 0 ? "Alt" : null,
        (Modifiers & 4) != 0 ? "Shift" : null, (Modifiers & 8) != 0 ? "Win" : null,
        Key is >= 65 and <= 90 or >= 48 and <= 57 ? ((char)Key).ToString()
            : Key is >= 112 and <= 135 ? $"F{Key - 111}"
            : Key switch { 8 => "Backspace", 9 => "Tab", 13 => "Enter", 27 => "Esc", 32 => "Space", 37 => "Left", 38 => "Up", 39 => "Right", 40 => "Down", 46 => "Delete", _ => $"Key {Key}" },
    }.Where(part => part is not null));

    public static ClipboardHotkey Load(string path)
    {
        if (!File.Exists(path)) return Default;
        if (new FileInfo(path).Length > 4096) throw new IOException("剪贴板快捷键配置无效。");
        var value = JsonSerializer.Deserialize<ClipboardHotkey>(File.ReadAllText(path));
        return value is { IsValid: true } ? value : throw new IOException("剪贴板快捷键配置无效。");
    }

    public void Save(string path)
    {
        if (!IsValid) throw new InvalidOperationException("快捷键必须包含 Ctrl、Alt 或 Win，并使用有效按键。");
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        string temporary = path + ".tmp";
        try
        {
            File.WriteAllText(temporary, JsonSerializer.Serialize(this));
            File.Move(temporary, path, overwrite: true);
        }
        finally { if (File.Exists(temporary)) File.Delete(temporary); }
    }
}
