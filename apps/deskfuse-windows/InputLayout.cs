using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;

namespace DeskFuse.Windows;

internal sealed record InputLayoutRect(string DeviceId, string DisplayId, double X, double Y, double Width, double Height);

/// <summary>Shared-coordinate geometry; no canvas transform or WinUI dependency.</summary>
internal static class InputLayout
{
    public static (double X, double Y)? Placement(JsonElement settings, string deviceId, string displayId)
    {
        if (settings.ValueKind != JsonValueKind.Object || !settings.TryGetProperty("placements", out var placements)
            || placements.ValueKind != JsonValueKind.Array) return null;
        foreach (var placement in placements.EnumerateArray())
        {
            // DisplayPlacementSetting serializes its DisplayKey under `display`.
            if (!placement.TryGetProperty("display", out var key) || key.ValueKind != JsonValueKind.Object
                || !key.TryGetProperty("device_id", out var device) || device.GetString() != deviceId
                || !key.TryGetProperty("display_id", out var display) || display.GetString() != displayId) continue;
            if (placement.TryGetProperty("x", out var x) && x.TryGetDouble(out double px) && double.IsFinite(px)
                && placement.TryGetProperty("y", out var y) && y.TryGetDouble(out double py) && double.IsFinite(py)) return (px, py);
        }
        return null;
    }

    public static bool Confirms(JsonElement settings, IReadOnlyList<InputLayoutRect> before, string deviceId, double dx, double dy)
    {
        var moving = before.Where(r => r.DeviceId == deviceId).ToArray();
        return moving.Length > 0 && moving.All(r => Placement(settings, deviceId, r.DisplayId) is { } p
            && Math.Abs(p.X - r.X - dx) < 0.01 && Math.Abs(p.Y - r.Y - dy) < 0.01);
    }

    public static bool Overlaps(InputLayoutRect a, InputLayoutRect b, double dx, double dy) =>
        Math.Min(a.X + dx + a.Width, b.X + b.Width) - Math.Max(a.X + dx, b.X) > 0.001
        && Math.Min(a.Y + dy + a.Height, b.Y + b.Height) - Math.Max(a.Y + dy, b.Y) > 0.001;

    /// <summary>Move a whole native desktop, previewing a connected, non-overlapping
    /// seam. Candidate generation mirrors deskfuse-input's snap_device_move.
    /// Overlapping drops express direction (left/right/up/down), not the old seam.</summary>
    public static (double X, double Y) SnapMove(IReadOnlyList<InputLayoutRect> rects, string deviceId, double dx, double dy)
    {
        var moving = rects.Where(r => r.DeviceId == deviceId).ToArray();
        var stationary = rects.Where(r => r.DeviceId != deviceId).ToArray();
        if (moving.Length == 0 || stationary.Length == 0) return (dx, dy);
        var collision = (from a in moving from b in stationary
            let overlapX = Math.Min(a.X + dx + a.Width, b.X + b.Width) - Math.Max(a.X + dx, b.X)
            let overlapY = Math.Min(a.Y + dy + a.Height, b.Y + b.Height) - Math.Max(a.Y + dy, b.Y)
            where overlapX > 0.001 && overlapY > 0.001
            orderby overlapX * overlapY descending
            select (a, b, overlapX, overlapY)).FirstOrDefault();
        if (collision.a is null) return (dx, dy);

        // Resolve the smaller overlap axis. A screen almost aligned with a
        // vertical edge must attach left/right even when the pointer arrived
        // diagonally from below.
        var preferHorizontal = collision.overlapX <= collision.overlapY;
        var desiredX = dx;
        var desiredY = dy;
        if (Math.Max(Math.Abs(dx), Math.Abs(dy)) > 8)
        {
            if (preferHorizontal)
                desiredX = dx < 0 ? collision.b.X - collision.a.X - collision.a.Width : collision.b.X + collision.b.Width - collision.a.X;
            else
                desiredY = dy < 0 ? collision.b.Y - collision.a.Y - collision.a.Height : collision.b.Y + collision.b.Height - collision.a.Y;
        }
        var candidates = new List<(double Score, double X, double Y)>();
        foreach (var a in moving)
        foreach (var b in stationary)
        {
            bool horizontal = preferHorizontal;
            double aStart = horizontal ? a.Y : a.X, aSize = horizontal ? a.Height : a.Width;
            double bStart = horizontal ? b.Y : b.X, bSize = horizontal ? b.Height : b.Width;
            double desired = horizontal ? desiredY : desiredX;
            double overlap = Math.Min(32, Math.Min(aSize / 2, bSize / 2));
            double low = bStart - aStart - aSize + overlap, high = bStart + bSize - aStart - overlap;
            var aligned = new[] { bStart - aStart, bStart + bSize - aStart - aSize, bStart + bSize / 2 - aStart - aSize / 2 };
            double along = Math.Clamp(desired, low, high);
            double best = aligned.OrderBy(p => Math.Abs(p - desired)).First();
            if (Math.Abs(best - desired) <= 48) along = best;
            foreach (double cross in new[] { along, low, high })
            foreach (double seam in horizontal ? new[] { b.X - a.X - a.Width, b.X + b.Width - a.X }
                : new[] { b.Y - a.Y - a.Height, b.Y + b.Height - a.Y })
            {
                double x = horizontal ? seam : cross, y = horizontal ? cross : seam;
                if (moving.Any(m => stationary.Any(n => Overlaps(m, n, x, y)))) continue;
                candidates.Add(((x - desiredX) * (x - desiredX) + (y - desiredY) * (y - desiredY), x, y));
            }
        }
        if (candidates.Count == 0) return (0, 0);
        var selected = candidates.OrderBy(c => c.Score).ThenBy(c => c.X).ThenBy(c => c.Y).First();
        return (selected.X, selected.Y);
    }
}
