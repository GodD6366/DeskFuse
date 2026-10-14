using System.Net.Http;
using System.Text.Json;

namespace DeskFuse.Windows;

/// <summary>
/// Read-only update status for the Windows client. Mirrors the engine's
/// source priority: the GitHub Releases API is canonical; the standalone
/// domain mirror is only consulted when GitHub fails or returns an empty
/// list. This class never downloads or installs anything.
/// </summary>
internal static class UpdateChecker
{
    private const string Repository = "GodD6366/DeskFuse";
    private const string Api = $"https://api.github.com/repos/{Repository}/releases?per_page=100";
    private const string Feed = "https://deskfuse.godd.cc/api/releases.json";
    private static readonly HttpClient Client = Create();

    public sealed record Result(bool Ok, string Summary, string? LatestTag, string? ReleaseUrl)
    {
        public bool UpdateAvailable { get; init; }
    }

    public static async Task<Result> CheckAsync(Version current, CancellationToken token = default)
    {
        var (error, releases) = await FetchCandidates(token).ConfigureAwait(false);
        if (releases is null)
            return new Result(false, $"更新检查失败：{error}", null, null);
        var latest = releases
            .Where(release => !release.Draft && release.PublishedAt is not null && release.HasMetadataAssets)
            .Select(release => (Release: release, Version: ParseReleaseTag(release.TagName)))
            .Where(pair => pair.Version is not null)
            .OrderByDescending(pair => pair.Version)
            .FirstOrDefault();
        if (latest.Release is null)
            return new Result(false, "更新检查失败：发布列表中没有完整的版本元数据。", null, null);
        bool newer = latest.Version! > current;
        string url = $"https://github.com/{Repository}/releases/tag/{latest.Release.TagName}";
        string summary = newer
            ? $"发现新版本 {latest.Release.TagName}（当前 v{current}）。Windows 安装包与自动安装更新尚未随发行提供；请在发布页查看并按需升级客户端与服务。"
            : $"已是最新（最新发布 {latest.Release.TagName}）。";
        return new Result(true, summary, latest.Release.TagName, url) { UpdateAvailable = newer };
    }

    /// <summary>GitHub first; the independent mirror only on failure or empty list.</summary>
    private static async Task<(string? Error, List<Release>?)> FetchCandidates(CancellationToken token)
    {
        string? githubError = null;
        try
        {
            var releases = await FetchList(Api, token).ConfigureAwait(false);
            if (releases.Count > 0) return (null, releases);
            githubError = "GitHub 返回空发布列表";
        }
        catch (Exception ex) when (ex is HttpRequestException or TaskCanceledException or IOException or JsonException)
        {
            githubError = ex is TaskCanceledException ? "GitHub 请求超时" : ex.Message;
        }
        try
        {
            var releases = await FetchList(Feed, token).ConfigureAwait(false);
            if (releases.Count > 0) return (null, releases);
            return ($"{githubError}；独立域名也返回空发布列表", null);
        }
        catch (Exception ex) when (ex is HttpRequestException or TaskCanceledException or IOException or JsonException)
        {
            string siteError = ex is TaskCanceledException ? "独立域名请求超时" : ex.Message;
            return ($"{githubError}；独立域名（{siteError}）", null);
        }
    }

    private static async Task<List<Release>> FetchList(string url, CancellationToken token)
    {
        using var request = new HttpRequestMessage(HttpMethod.Get, url);
        request.Headers.UserAgent.ParseAdd("DeskFuse-Windows-client");
        using var response = await Client.SendAsync(request, token).ConfigureAwait(false);
        response.EnsureSuccessStatusCode();
        await using var stream = await response.Content.ReadAsStreamAsync(token).ConfigureAwait(false);
        using var document = await JsonDocument.ParseAsync(stream, cancellationToken: token).ConfigureAwait(false);
        var root = document.RootElement;
        if (root.ValueKind != JsonValueKind.Array) throw new IOException("发布列表格式无效。");
        var releases = new List<Release>();
        foreach (var item in root.EnumerateArray())
        {
            if (!item.TryGetProperty("tag_name", out var tag) || tag.ValueKind != JsonValueKind.String) continue;
            string[] metadata = ["release.json", "build-info.json", "SHA256SUMS"];
            bool hasAssets = item.TryGetProperty("assets", out var assets) && assets.ValueKind == JsonValueKind.Array
                && metadata.All(name => assets.EnumerateArray().Any(asset =>
                    asset.TryGetProperty("name", out var assetName) && assetName.GetString() == name
                    && asset.TryGetProperty("state", out var state) && state.GetString() == "uploaded"
                    && asset.TryGetProperty("size", out var size) && size.TryGetInt64(out long bytes) && bytes > 0));
            releases.Add(new Release(
                tag.GetString() ?? "",
                item.TryGetProperty("draft", out var draft) && draft.ValueKind == JsonValueKind.True,
                item.TryGetProperty("published_at", out var published) && published.ValueKind == JsonValueKind.String ? published.GetString() : null,
                hasAssets));
        }
        return releases;
    }

    /// <summary>Only exact `vX.Y.Z` tags, matching the engine's SemVer filter.</summary>
    private static Version? ParseReleaseTag(string tag)
    {
        if (!tag.StartsWith('v')) return null;
        var parts = tag[1..].Split('.');
        if (parts.Length != 3 || parts.Any(part => part.Length == 0 || part.Any(character => !char.IsAsciiDigit(character)))) return null;
        return new Version(int.Parse(parts[0]), int.Parse(parts[1]), int.Parse(parts[2]));
    }

    private static HttpClient Create()
    {
        var client = new HttpClient { Timeout = TimeSpan.FromSeconds(10) };
        return client;
    }

    private sealed record Release(string TagName, bool Draft, string? PublishedAt, bool HasMetadataAssets);
}
