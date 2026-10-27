namespace DeskFuse.Windows;

/// Count- and cost-bounded LRU. Owned by the UI dispatcher, like its images.
internal sealed class BoundedCache<TKey, TValue>(int countLimit, long costLimit) where TKey : notnull
{
    private sealed record Entry(TValue Value, long Cost);
    private readonly Dictionary<TKey, Entry> entries = [];
    private readonly LinkedList<TKey> order = new();
    private readonly Dictionary<TKey, LinkedListNode<TKey>> nodes = [];
    public int Count => entries.Count;
    public long Cost { get; private set; }

    public bool TryGet(TKey key, out TValue value)
    {
        if (!entries.TryGetValue(key, out var entry)) { value = default!; return false; }
        order.Remove(nodes[key]);
        order.AddLast(nodes[key]);
        value = entry.Value;
        return true;
    }

    public void Put(TKey key, TValue value, long cost)
    {
        Remove(key);
        cost = Math.Max(0, cost);
        if (cost > costLimit || countLimit < 1) return;
        entries[key] = new Entry(value, cost);
        nodes[key] = order.AddLast(key);
        Cost += cost;
        while (Count > countLimit || Cost > costLimit) Remove(order.First!.Value);
    }

    public void Remove(TKey key)
    {
        if (!entries.Remove(key, out var entry)) return;
        Cost -= entry.Cost;
        order.Remove(nodes[key]);
        nodes.Remove(key);
    }
}
