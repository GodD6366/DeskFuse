using System.Diagnostics;
using System.Reflection;
using System.Text.Json;

namespace DeskFuse.Windows;

/// <summary>
/// 统一的「联系开发者」入口：设置窗口与托盘菜单共用，与 macOS 的 SupportCenter 指向同一个仓库。
/// 反馈动作先导出脱敏诊断，再打开已在环境信息中预填诊断文件名的反馈页，并在资源管理器中选中该文件。
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

    /// <summary>打开已预填环境信息的新问题页，并导出可在资源管理器中直接拖入的诊断文件。</summary>
    internal static void ReportProblem(JsonElement? snapshot, int? historyEntries = null,
        int? historyRetentionDays = null, string? clipboardStorageError = null)
    {
        string service = snapshot is JsonElement value
            ? $"服务 {value.String("package_version", "未知")} · {(value.Bool("network_running") ? "网络运行中" : "网络未就绪")}"
            : "服务未连接";
        string? diagnosticFile = null;
        try
        {
            diagnosticFile = DiagnosticsExport.Export(snapshot, historyEntries, historyRetentionDays, clipboardStorageError);
        }
        catch (Exception) { /* 诊断失败不阻塞反馈；正文会说明如何重试。 */ }
        string diagnostics = diagnosticFile is null
            ? "诊断文件生成失败。可在「设置 → 关于与更新 → 导出诊断信息」中重试，把生成的文件拖入本 issue。"
            : $"已生成脱敏诊断文件 `{Path.GetFileName(diagnosticFile)}`，请在资源管理器里把它拖入本 issue。\n"
              + "<!-- 诊断只包含版本、系统、服务状态与计数，不包含剪贴板正文、设备信任材料或文件内容。 -->";
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
            diagnostics);
        Open($"{IssuesUrl}/new?title={Uri.EscapeDataString("[Windows] ")}&body={Uri.EscapeDataString(body)}");
        if (diagnosticFile is not null) RevealInExplorer(diagnosticFile);
    }

    private static void RevealInExplorer(string path)
    {
        try
        {
            Process.Start(new ProcessStartInfo("explorer.exe", $"/select,\"{path}\"") { UseShellExecute = true });
        }
        catch (Exception) { /* The issue page already opened; revealing is best effort. */ }
    }

    internal static void Open(string url)
    {
        try { Process.Start(new ProcessStartInfo(url) { UseShellExecute = true }); }
        catch (Exception)
        {
            var package = new global::Windows.ApplicationModel.DataTransfer.DataPackage();
            package.SetText(url);
            global::Windows.ApplicationModel.DataTransfer.Clipboard.SetContent(package);
            global::Windows.ApplicationModel.DataTransfer.Clipboard.Flush();
        }
    }
}
