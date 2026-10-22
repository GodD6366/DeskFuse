import AppKit
import DeskFuseEngine
import Foundation

/// 统一的「联系开发者」入口。
/// 帮助菜单、侧栏、错误提示、菜单栏面板和「设置 → 关于与更新」都调用这里，
/// 用户无论在哪遇到问题，走的都是同一套流程：导出脱敏诊断 → 打开已预填环境与诊断摘要的反馈页
/// → 在访达中选中完整的脱敏诊断文件，需要时再拖进去。
@MainActor
enum SupportCenter {
    /// 源码、发行版、问题反馈都在同一个公开仓库；必须与 `config/distribution.json` 的 `source_repository` 一致。
    static let repository = "GodD6366/DeskFuse"
    static let issuesURL = URL(string: "https://github.com/\(repository)/issues")!
    static let releasesURL = URL(string: "https://github.com/\(repository)/releases")!
    static let securityURL = URL(string: "https://github.com/\(repository)/blob/main/SECURITY.md")!

    /// 与引擎 `diagnostics_file.rs` 写入的位置一致；诊断导出也放在这里，用户只需拖一个文件。
    static let logsDirectory = FileManager.default.homeDirectoryForCurrentUser
        .appendingPathComponent("Library/Logs/DeskFuse", isDirectory: true)

    /// GitHub 对过长的新建问题链接直接拒绝；留出余量后，正文按编码后的整条链接计算长度。
    static let maximumIssueURLLength = 7_500
    /// 引擎未就绪时，从最近的日志末尾取的行数。
    private static let fallbackLogLines = 40

    /// 版本标签：公开发行版只显示版本号，内部通道附带 build 号。
    static let versionLabel: String = {
        let info = Bundle.main.infoDictionary ?? [:]
        let version = info["CFBundleShortVersionString"] as? String ?? "—"
        if UpdateInstallationChannel.current != .github,
           let build = info["CFBundleVersion"] as? String, !build.isEmpty {
            return "v\(version) · build \(build)"
        }
        return "v\(version)"
    }()

    /// 一键反馈：尽量附上诊断，但诊断失败（例如后台服务没启动）也照样打开反馈页。
    /// - Parameter error: 触发反馈的原始错误文本；为空时使用当前窗口上的错误提示。
    static func reportProblem(_ store: AppStore, error: String? = nil) {
        let message = (error ?? store.errorNotice.rawMessage).map { redactDiagnosticText(text: $0) }
        Task { @MainActor in
            let export = await exportDiagnostics(store, to: defaultDiagnosticsURL())
            let summary = export?.summary ?? recentLogSummary()
            let url = newIssueURL(environment: environment(store), error: message,
                                  summary: summary, diagnostics: export?.url)
            NSWorkspace.shared.open(url)
            if let file = export?.url {
                NSWorkspace.shared.activateFileViewerSelecting([file])
            }
        }
    }

    /// 让用户自选位置导出诊断；完成后在访达中选中文件，用户不必再去找它。
    static func exportDiagnosticsInteractively(_ store: AppStore, completion: @escaping () -> Void = {}) {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = defaultDiagnosticsURL().lastPathComponent
        // Do not enter a nested AppKit run loop from a SwiftUI action.
        panel.begin { response in
            guard response == .OK, let url = panel.url else { completion(); return }
            Task { @MainActor in
                defer { completion() }
                if let export = await exportDiagnostics(store, to: url) {
                    NSWorkspace.shared.activateFileViewerSelecting([export.url])
                }
            }
        }
    }

    /// 复制版本、系统与运行状态，方便贴到邮件、群聊等任意渠道。
    static func copyEnvironment(_ store: AppStore) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(environment(store).map { "\($0.0)：\($0.1)" }.joined(separator: "\n"),
                                       forType: .string)
    }

    static func open(_ url: URL) { NSWorkspace.shared.open(url) }

    // MARK: - 诊断

    struct DiagnosticExport {
        let url: URL
        /// 已脱敏的紧凑摘要，写进反馈正文。
        let summary: String
    }

    /// 引擎诊断、本机内存与鼠标按键诊断由引擎统一脱敏后写入同一个 JSONL 文件；失败时返回 nil 并在界面提示。
    static func exportDiagnostics(_ store: AppStore, to url: URL) async -> DiagnosticExport? {
        guard let client = store.client else { return nil }
        do {
            try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            var extra = await MemoryDiagnostics.capture(
                ocr: ClipboardOCRIndexer.shared.diagnostics(),
                selectedPage: store.selectedPage.rawValue,
                visibleWindowCount: NSApp.windows.filter { $0.occlusionState.contains(.visible) }.count)
            extra.append(MouseButtonController.shared.diagnostics())
            let names = deviceNames(store)
            let summary = try await client.exportDiagnostics(
                url.path, extra: String(decoding: extra, as: UTF8.self),
                localNames: names.local, peerNames: names.peers)
            return DiagnosticExport(url: url, summary: summary)
        } catch {
            store.error = error.userFacingMessage
            return nil
        }
    }

    /// 需要替换成代号的名称：本机名和所有已知电脑名（含未配对但被发现的设备）。
    private static func deviceNames(_ store: AppStore) -> (local: [String], peers: [String]) {
        guard let snapshot = store.snapshot else { return ([], []) }
        // 主机名由引擎自行补充；这里不调用 Host.current()，它可能因反查 DNS 阻塞主线程。
        return ([snapshot.localName], snapshot.devices.map(\.name))
    }

    /// 后台服务未就绪时的兜底：最近日志的末尾若干行，同样经过脱敏。
    static func recentLogSummary() -> String? {
        let files = (try? FileManager.default.contentsOfDirectory(
            at: logsDirectory, includingPropertiesForKeys: [.contentModificationDateKey])) ?? []
        let latest = files
            .filter { $0.lastPathComponent.hasPrefix("diagnostics-") && $0.pathExtension == "jsonl" }
            .max { modified($0) < modified($1) }
        guard let latest, let handle = try? FileHandle(forReadingFrom: latest) else { return nil }
        defer { try? handle.close() }
        // 只读文件尾部：单个小时文件可能有数 MB。
        let size = (try? handle.seekToEnd()) ?? 0
        try? handle.seek(toOffset: size > 65_536 ? size - 65_536 : 0)
        guard let data = try? handle.readToEnd() else { return nil }
        let lines = String(decoding: data, as: UTF8.self)
            .split(separator: "\n").suffix(fallbackLogLines).joined(separator: "\n")
        return lines.isEmpty ? nil : redactDiagnosticText(text: lines)
    }

    private static func modified(_ url: URL) -> Date {
        (try? url.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate) ?? .distantPast
    }

    private static func defaultDiagnosticsURL() -> URL {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.dateFormat = "yyyyMMdd-HHmmss"
        return logsDirectory.appendingPathComponent("DeskFuse-diagnostics-\(formatter.string(from: Date())).jsonl")
    }

    // MARK: - 反馈内容

    /// 只包含版本、系统与状态元数据，不包含剪贴板内容、设备名称或地址。
    static func environment(_ store: AppStore) -> [(String, String)] {
        let system = ProcessInfo.processInfo.operatingSystemVersion
        #if arch(arm64)
        let architecture = "Apple 芯片"
        #else
        let architecture = "Intel"
        #endif
        let devices = store.snapshot?.devices.filter { $0.trusted && !$0.blocked } ?? []
        let issues = HealthCenter.issues(store)
        return [
            ("DeskFuse", "\(versionLabel)（\(UpdateInstallationChannel.current.title)）"),
            ("macOS", "\(system.majorVersion).\(system.minorVersion).\(system.patchVersion) · \(architecture)"),
            ("当前页面", store.selectedPage.rawValue),
            ("已配对电脑", store.snapshot == nil ? "后台服务未就绪" : "\(devices.count) 台，\(devices.filter(\.connected).count) 台在线"),
            ("运行状态", issues.isEmpty ? HealthCenter.summary(of: issues) : issues.map(\.title).joined(separator: "；")),
        ]
    }

    /// 预填 GitHub 新问题；正文按仓库 Bug 模板的分节组织，用户只需补充“发生了什么”。
    /// 诊断摘要放在折叠区，链接超长时从最早的摘要行开始裁剪，保证链接一定能打开。
    static func newIssueURL(environment: [(String, String)], error: String?, summary: String?, diagnostics: URL?) -> URL {
        var head = ["## 发生了什么", "<!-- 你原本想做什么？实际看到了什么？ -->", "", "## 复现步骤", "1. ", ""]
        if let error = error.map(redacted), !error.isEmpty {
            // 原始错误可能很长，截断后仍足以定位，同时给诊断摘要留出链接长度。
            head += ["## 界面提示", "```", String(error.prefix(600)), "```", ""]
        }
        head += ["## 环境（自动填写）"] + environment.map { "- \($0.0)：\($0.1)" } + [""]
        var tail = ["## 完整诊断文件"]
        if let diagnostics {
            tail.append("已在访达中选中 `\(redacted(diagnostics.path))`，已脱敏；如开发者需要，可把它拖到这里上传。")
        } else {
            tail.append("后台服务未就绪，未能导出完整诊断，上方摘要取自最近的日志。")
        }
        let title = error.map(friendlyError).map { "[macOS] \($0.prefix(60))" } ?? "[macOS] "

        func url(summaryLines: [String]) -> URL {
            var lines = head
            if !summaryLines.isEmpty {
                lines += ["## 诊断摘要（已脱敏，自动填写）", "<details><summary>展开</summary>", "", "```",
                          summaryLines.joined(separator: "\n"), "```", "</details>", ""]
            }
            var components = URLComponents(url: issuesURL.appendingPathComponent("new"), resolvingAgainstBaseURL: false)!
            components.queryItems = [URLQueryItem(name: "title", value: title),
                                     URLQueryItem(name: "body", value: (lines + tail).joined(separator: "\n"))]
            return components.url ?? issuesURL
        }

        var summaryLines = (summary ?? "").split(separator: "\n", omittingEmptySubsequences: true).map(String.init)
        // 前两行是版本与记录统计，最后才裁；其余按时间顺序，先裁最旧的。
        let pinned = min(2, summaryLines.count)
        while url(summaryLines: summaryLines).absoluteString.count > maximumIssueURLLength, !summaryLines.isEmpty {
            if summaryLines.count > pinned { summaryLines.remove(at: pinned) } else { summaryLines.removeFirst() }
        }
        return url(summaryLines: summaryLines)
    }

    /// 把本机用户目录替换为 `~`，避免在公开反馈里暴露用户名。
    static func redacted(_ text: String) -> String {
        text.replacingOccurrences(of: NSHomeDirectory(), with: "~")
    }
}
