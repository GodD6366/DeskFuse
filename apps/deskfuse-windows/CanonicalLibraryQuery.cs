using System.Globalization;

namespace DeskFuse.Windows;

/// Calendar-day windows shared by item and count queries. Adding 24 hours to a
/// DateTimeOffset is not a calendar operation across daylight-saving changes.
internal static class CanonicalLibraryQuery
{
    public static Dictionary<string, object?> Build(
        string? query, string? listId, string kind, string? source, int offset, int limit,
        IReadOnlyList<CanonicalRule>? rules, DateTimeOffset now, TimeZoneInfo zone)
    {
        var today = TimeZoneInfo.ConvertTime(now, zone).Date;
        var tomorrow = today.AddDays(1);
        var windows = new Dictionary<string, object>();
        if (rules is not null)
        {
            for (int index = 0; index < rules.Count; index++)
            {
                var rule = rules[index];
                if (rule.Field != "date") continue;
                DateTime? start = null, end = null;
                switch (rule.Op)
                {
                    case "today": start = today; end = tomorrow; break;
                    case "yesterday": start = today.AddDays(-1); end = today; break;
                    case "last_days":
                        if (int.TryParse(rule.Value, out int days) && days > 0 && days <= (today - DateTime.MinValue).Days + 1)
                        {
                            start = today.AddDays(1 - days);
                            end = tomorrow;
                        }
                        break;
                    case "between":
                    case "range":
                        if (DateTime.TryParseExact(rule.Value, "yyyy-MM-dd", CultureInfo.InvariantCulture,
                                DateTimeStyles.None, out var from)
                            && DateTime.TryParseExact(rule.Value2, "yyyy-MM-dd", CultureInfo.InvariantCulture,
                                DateTimeStyles.None, out var last) && last.Date < DateTime.MaxValue.Date)
                        {
                            start = from.Date;
                            end = last.Date.AddDays(1);
                        }
                        break;
                }
                if (start is { } a && end is { } b && a < b)
                    windows[index.ToString(CultureInfo.InvariantCulture)] = new { start = DayStart(a, zone), end = DayStart(b, zone) };
            }
        }
        return new Dictionary<string, object?>
        {
            ["op"] = "items",
            ["query"] = query ?? "",
            ["list_id"] = string.IsNullOrEmpty(listId) ? null : listId,
            ["kind"] = kind,
            ["source"] = string.IsNullOrEmpty(source) ? null : source,
            ["offset"] = offset,
            ["limit"] = Math.Clamp(limit, 1, 100),
            ["include_total"] = true,
            ["day_start_ms"] = DayStart(today, zone),
            ["yesterday_start_ms"] = DayStart(today.AddDays(-1), zone),
            ["day_end_ms"] = DayStart(tomorrow, zone),
            ["timezone"] = zone.Id,
            ["rule_windows"] = windows,
        };
    }

    private static long DayStart(DateTime date, TimeZoneInfo zone)
    {
        var local = DateTime.SpecifyKind(date.Date, DateTimeKind.Unspecified);
        // Some zones change offset at midnight. Match start-of-day semantics:
        // first valid instant, and the earlier occurrence of an ambiguous time.
        while (zone.IsInvalidTime(local)) local = local.AddMinutes(1);
        var offset = zone.IsAmbiguousTime(local) ? zone.GetAmbiguousTimeOffsets(local).Max() : zone.GetUtcOffset(local);
        return new DateTimeOffset(local, offset).ToUnixTimeMilliseconds();
    }
}
