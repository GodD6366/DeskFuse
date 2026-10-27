using System.Collections.ObjectModel;

namespace DeskFuse.Windows;

/// Keeps item identity when a paged source changes. No Reset notification: UI
/// containers, selection and scroll anchors survive background index updates.
internal static class StableCollection
{
    public static void Apply<T>(ObservableCollection<T> current, IReadOnlyList<T> desired,
        Func<T, string> key, Func<T, T, bool> equivalent)
    {
        var wanted = desired.Select(key).ToHashSet(StringComparer.Ordinal);
        for (int i = current.Count - 1; i >= 0; i--)
            if (!wanted.Contains(key(current[i]))) current.RemoveAt(i);
        for (int i = 0; i < desired.Count; i++)
        {
            string id = key(desired[i]);
            int existing = -1;
            for (int j = i; j < current.Count; j++)
                if (StringComparer.Ordinal.Equals(key(current[j]), id)) { existing = j; break; }
            if (existing < 0) current.Insert(i, desired[i]);
            else
            {
                if (existing != i) current.Move(existing, i);
                if (!equivalent(current[i], desired[i])) current[i] = desired[i];
            }
        }
        while (current.Count > desired.Count) current.RemoveAt(current.Count - 1);
    }
}
