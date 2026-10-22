using System.Diagnostics;
using System.Reflection;
using System.Text.Json;

namespace DeskFuse.Windows;

/// <summary>
/// 统一的「联系开发者」入口：设置窗口与托盘菜单共用，与 macOS 的 SupportCenter 指向同一个仓库。
/// Windows 暂不支持导出脱敏诊断，反馈正文只包含版本、系统与服务状态，不附日志。
/// </summary>
internal static class SupportLinks
{
    /// <summary>必须与 config/distribution.json 的 source_repository 一致。</summary>
    private const string Repository = "GodD6366/DeskFuse";
    internal const string IssuesUrl = $"https://github.com/{Repository}/issues";
    internal const string ReleasesUrl = $"https://github.com/{Repository}/releases";

    internal static string VersionLabel
    {
        get
        {
            var fileVersion = Assembly.GetExecutingAssembly().GetCustomAttributes(typeof(AssemblyFileVersionAttribute), false)
                .Cast<AssemblyFileVersionAttribute>().Single().Version;
            var version = Version.Parse(fileVersion);
            return $"{version.Major}.{version.Minor}.{version.Build} · build {version.Revision}";
        }
    }

    /// <summary>打开已预填环境信息的新问题页。</summary>
    internal static void ReportProblem(JsonElement? snapshot)
    {
        string service = snapshot is JsonElement value
            ? $"服务 {value.String("package_version", "未知")} · {(value.Bool("network_running") ? "网络运行中" : "网络未就绪")}"
            : "服务未连接";
        string body = string.Join("\n",
            "## 发生了什么",
            "<!-- 你原本想做什么？实际看到了什么？ -->",
            "",
            "## 复现步骤",
            "1. ",
            "",
            "## 环境（自动填写）",
            $"- DeskFuse：{VersionLabel}",
            $"- Windows：{Environment.OSVersion.Version} · {RuntimeInformation.OSArchitecture}",
            $"- 服务状态：{service}",
            "",
            "## 诊断",
            "Windows 暂不支持自动导出脱敏诊断。请先不要附原始日志，开发者需要时会在这里说明。");
        Open($"{IssuesUrl}/new?title={Uri.EscapeDataString("[Windows] ")}&body={Uri.EscapeDataString(body)}");
    }

    internal static void Open(string url)
    {
        try { Process.Start(new ProcessStartInfo(url) { UseShellExecute = true }); }
        catch (Exception) { System.Windows.Clipboard.SetText(url); }
    }
}
