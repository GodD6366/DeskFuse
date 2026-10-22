using System.Text.Json;

namespace DeskFuse.Windows;

/// <summary>
/// 剪贴板库：类型智能列表（文字/图片/文件）由历史内容推导，不占存储；
/// 手动列表是用户命名的归档，成员豁免保留期清理。与历史同目录持久化到
/// clipboard-library-v1.json。跨设备同步依赖引擎能力，当前仅本机有效。
/// </summary>
public sealed class ClipboardLibrary
{
    /// <summary>侧栏选中项：全部、类型智能列表或某个手动列表。</summary>
    public readonly record struct Selection(string? ListId, string? Kind)
    {
        public static Selection All => default;
        public static Selection OfKind(string kind) => new(null, kind);
        public static Selection OfList(string id) => new(id, null);
        public bool IsAll => ListId is null && Kind is null;
    }

    public sealed class ManualList
    {
        public string Id { get; set; } = "";
        public string Name { get; set; } = "";
        public HashSet<string> Members { get; set; } = new(StringComparer.Ordinal);
    }

    private readonly string file;
    private readonly List<ManualList> lists = [];
    public string? StorageError { get; private set; }
    public event Action? Changed;

    public ClipboardLibrary()
        : this(Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "DeskFuse", "desktop")) { }

    internal ClipboardLibrary(string directory)
    {
        file = Path.Combine(directory, "clipboard-library-v1.json");
        try
        {
            if (!File.Exists(file)) return;
            var loaded = JsonSerializer.Deserialize<ManualList[]>(File.ReadAllText(file));
            if (loaded is null) return;
            foreach (var list in loaded)
            {
                if (string.IsNullOrWhiteSpace(list.Id) || string.IsNullOrWhiteSpace(list.Name)) continue;
                list.Members.RemoveWhere(string.IsNullOrWhiteSpace);
                lists.Add(list);
            }
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or JsonException or ArgumentException)
        { StorageError = "无法读取剪贴板列表；原文件保留，不会自动覆盖"; }
    }

    public IReadOnlyList<ManualList> Lists => lists;

    public ManualList? Find(string? id) => id is null ? null : lists.FirstOrDefault(list => list.Id == id);

    public ManualList Create(string name)
    {
        var list = new ManualList { Id = Guid.NewGuid().ToString("N"), Name = NormalizeName(name) };
        lists.Add(list);
        Save();
        return list;
    }

    public void Rename(string id, string name)
    {
        var list = Find(id) ?? throw new InvalidOperationException("列表已删除");
        list.Name = NormalizeName(name);
        Save();
    }

    public void Delete(string id)
    {
        if (lists.RemoveAll(list => list.Id == id) > 0) Save();
    }

    /// <summary>向前/向后移动一位；返回是否发生变化。</summary>
    public bool Move(string id, int delta)
    {
        int index = lists.FindIndex(list => list.Id == id);
        int target = index + delta;
        if (index < 0 || target < 0 || target >= lists.Count) return false;
        (lists[index], lists[target]) = (lists[target], lists[index]);
        Save();
        return true;
    }

    public void Add(string listId, IEnumerable<string> entryIds)
    {
        var list = Find(listId) ?? throw new InvalidOperationException("列表已删除");
        bool changed = false;
        foreach (string entryId in entryIds) changed |= list.Members.Add(entryId);
        if (changed) Save();
    }

    public void Remove(string listId, IEnumerable<string> entryIds)
    {
        var list = Find(listId);
        if (list is null) return;
        bool changed = false;
        foreach (string entryId in entryIds) changed |= list.Members.Remove(entryId);
        if (changed) Save();
    }

    /// <summary>历史条目删除时同步摘掉所有列表成员关系。</summary>
    public void ForgetEntry(string entryId)
    {
        bool changed = false;
        foreach (var list in lists) changed |= list.Members.Remove(entryId);
        if (changed) Save();
    }

    /// <summary>历史清空时摘掉全部成员关系，列表本身保留。</summary>
    public void ClearMemberships()
    {
        if (lists.All(list => list.Members.Count == 0)) return;
        foreach (var list in lists) list.Members.Clear();
        Save();
    }

    /// <summary>手动列表成员长期保留，不参与保留期清理。</summary>
    public bool IsPinned(ClipboardEntry entry) => lists.Any(list => list.Members.Contains(entry.Id));

    /// <summary>条目所属的手动列表名，供行内标记与右键菜单展示。</summary>
    public IReadOnlyList<string> Memberships(string entryId) =>
        lists.Where(list => list.Members.Contains(entryId)).Select(list => list.Name).ToArray();

    public IEnumerable<ClipboardEntry> Filter(Selection selection, IEnumerable<ClipboardEntry> entries)
    {
        if (selection.Kind is string kind) return entries.Where(entry => entry.Kind == kind);
        if (selection.ListId is string id)
        {
            var list = Find(id);
            return list is null ? [] : entries.Where(entry => list.Members.Contains(entry.Id));
        }
        return entries;
    }

    public int Count(Selection selection, IEnumerable<ClipboardEntry> entries) => Filter(selection, entries).Count();

    private static string NormalizeName(string name)
    {
        string trimmed = string.Concat(name.Trim().EnumerateRunes().Take(60).Select(rune => rune.ToString()));
        return trimmed.Length == 0 ? throw new InvalidOperationException("列表名称不能为空") : trimmed;
    }

    private void Save()
    {
        if (StorageError is not null) return;
        string temporary = Path.Combine(Path.GetDirectoryName(file)!, $".library-{Guid.NewGuid():N}.tmp");
        try
        {
            File.WriteAllText(temporary, JsonSerializer.Serialize(lists));
            File.Move(temporary, file, overwrite: true);
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or JsonException)
        {
            StorageError = "剪贴板列表保存失败；当前修改仍在内存中";
            try { if (File.Exists(temporary)) File.Delete(temporary); } catch (IOException) { } catch (UnauthorizedAccessException) { }
        }
        Changed?.Invoke();
    }
}
