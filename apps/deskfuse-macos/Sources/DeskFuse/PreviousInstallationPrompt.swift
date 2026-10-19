import AppKit
import Observation
import SwiftUI

@MainActor @Observable
final class PreviousInstallationController {
    private struct Installation: Decodable {
        let name: String
        let bundleIdentifier: String
        let dataDirectory: String
        let appBundle: String
        let cacheDirectory: String
    }
    private static let completedKey = "deskfuse.previousInstallationPrompt.filesCompleted"
    var isPresented = false
    var busy = false
    var imported = false
    var name = "旧应用"
    var error: String?
    var importedCount: Int?
    var hasHistory = false
    var appURLs: [URL] = []
    /// 本次启动呈现过迁移 sheet：属于老用户，不再进入新手引导。
    private(set) var wasPresented = false
    private var installation: Installation?
    private var source: URL?
    private var sourceCache: URL?
    private var completion: (() -> Void)?
    private var destination: URL {
        FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support/DeskFuse")
    }

    func evaluate(completion: @escaping () -> Void) {
        guard !ProcessInfo.processInfo.arguments.contains("--update-self-test"),
              !UserDefaults.standard.bool(forKey: Self.completedKey),
              let config = Bundle.main.url(forResource: "previous-installation", withExtension: "json"),
              let data = try? Data(contentsOf: config),
              let installation = try? JSONDecoder().decode(Installation.self, from: data) else {
            completion()
            return
        }
        self.installation = installation
        name = installation.name
        let home = FileManager.default.homeDirectoryForCurrentUser
        let source = home.appendingPathComponent("Library/Application Support").appendingPathComponent(installation.dataDirectory)
        self.source = source
        let sourceCache = home.appendingPathComponent("Library/Caches").appendingPathComponent(installation.cacheDirectory)
        self.sourceCache = sourceCache
        hasHistory = FileManager.default.fileExists(atPath: source.appendingPathComponent("state.sqlite3").path)
        var candidates = [URL(fileURLWithPath: "/Applications").appendingPathComponent(installation.appBundle),
                          home.appendingPathComponent("Applications").appendingPathComponent(installation.appBundle)]
        if let registered = NSWorkspace.shared.urlForApplication(withBundleIdentifier: installation.bundleIdentifier) {
            candidates.append(registered)
        }
        appURLs = Array(Set(candidates.map { $0.resolvingSymlinksInPath() })).filter {
            $0 != Bundle.main.bundleURL.resolvingSymlinksInPath()
                && Bundle(url: $0)?.bundleIdentifier == installation.bundleIdentifier
        }.sorted { $0.path < $1.path }
        guard hasHistory || !appURLs.isEmpty else {
            UserDefaults.standard.set(true, forKey: Self.completedKey)
            completion()
            return
        }
        do {
            imported = try hasHistory && HistoryImport.alreadyImported(source: source, destination: destination)
                && HistoryImport.filesImported(sourceCache: sourceCache, destination: destination)
        }
        catch { self.error = error.localizedDescription }
        self.completion = completion
        wasPresented = true
        isPresented = true
    }

    func finish() {
        guard !busy else { return }
        UserDefaults.standard.set(true, forKey: Self.completedKey)
        isPresented = false
        let completion = completion
        self.completion = nil
        // Allow the import sheet to close before presenting permission guidance.
        Task { @MainActor in
            try? await Task.sleep(for: .milliseconds(300))
            completion?()
        }
    }

    private func stopPreviousApp() async throws {
        guard let installation else { return }
        let apps = NSRunningApplication.runningApplications(withBundleIdentifier: installation.bundleIdentifier)
        for app in apps where !app.isTerminated {
            guard app.terminate() else { throw HistoryImport.Failure(message: "请先退出\(name)，然后重试") }
        }
        for _ in 0..<100 {
            if apps.allSatisfy(\.isTerminated) { return }
            try await Task.sleep(for: .milliseconds(100))
        }
        throw HistoryImport.Failure(message: "\(name)尚未退出，请处理其弹窗后重试")
    }

    func importHistory() {
        guard !busy, let source, let sourceCache else { return }
        busy = true
        error = nil
        let destination = destination
        Task {
            defer { busy = false }
            do {
                try await stopPreviousApp()
                importedCount = try await Task.detached {
                    try HistoryImport.run(source: source, destination: destination, sourceCache: sourceCache)
                }.value
                imported = true
            } catch { self.error = error.localizedDescription }
        }
    }

    func removePreviousApp() {
        guard !busy, imported || !hasHistory, let installation else { return }
        let confirmation = NSAlert()
        confirmation.messageText = "将\(name)移到废纸篓？"
        confirmation.informativeText = "只移除以下应用，历史数据和迁移备份会保留。\n" + appURLs.map(\.path).joined(separator: "\n")
        confirmation.addButton(withTitle: "移到废纸篓")
        confirmation.addButton(withTitle: "取消")
        guard confirmation.runModal() == .alertFirstButtonReturn else { return }
        busy = true
        error = nil
        Task {
            do {
                try await stopPreviousApp()
                for url in appURLs {
                    guard url == url.resolvingSymlinksInPath(), url.pathExtension == "app",
                          url != Bundle.main.bundleURL.resolvingSymlinksInPath(),
                          Bundle(url: url)?.bundleIdentifier == installation.bundleIdentifier else {
                        throw HistoryImport.Failure(message: "旧应用位置已变化，未执行删除")
                    }
                    try FileManager.default.trashItem(at: url, resultingItemURL: nil)
                }
                appURLs = []
                busy = false
                finish()
            } catch {
                appURLs = appURLs.filter { FileManager.default.fileExists(atPath: $0.path) }
                self.error = error.localizedDescription
                busy = false
            }
        }
    }
}

struct PreviousInstallationPrompt: View {
    @Bindable var controller: PreviousInstallationController

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            Label(controller.imported ? "历史数据已导入" : "发现\(controller.name)",
                  systemImage: controller.imported ? "checkmark.circle" : "tray.and.arrow.down")
                .font(DFType.pageTitle)
            if let count = controller.importedCount {
                Text("已新增 \(count) 条剪贴板记录，历史接收文件已迁移；DeskFuse 原有记录保持不变。")
            } else if controller.imported {
                Text("已检测到之前完成的导入，无需重复导入。")
            } else if controller.hasHistory {
                Text("将剪贴板历史、归档、列表和历史接收文件导入 DeskFuse。导入前会退出\(controller.name)并备份，旧数据不会删除。")
                Text("未下载的远端内容仅保留索引，已清理的文件无法恢复；设备需要重新配对和授权。")
                    .font(DFType.detail).foregroundStyle(.secondary)
            } else {
                Text("检测到旧应用，但没有找到本机历史数据库。")
            }
            if controller.imported && !controller.appURLs.isEmpty {
                Text("可以将\(controller.name)移到废纸篓。旧数据和备份会保留。")
            }
            if let error = controller.error {
                Text(error).foregroundStyle(DFPalette.danger).textSelection(.enabled)
            }
            if controller.busy {
                HStack { ProgressView().controlSize(.small); Text("正在处理，请稍候…") }
            }
            HStack {
                Button(controller.imported ? "保留旧应用并继续" : "暂不处理") { controller.finish() }
                Spacer()
                if controller.hasHistory && !controller.imported {
                    Button("导入历史") { controller.importHistory() }.buttonStyle(.dfPrimary)
                } else if !controller.appURLs.isEmpty {
                    Button("移除\(controller.name)", role: .destructive) { controller.removePreviousApp() }
                } else {
                    Button("继续") { controller.finish() }.buttonStyle(.dfPrimary)
                }
            }
            .disabled(controller.busy)
        }
        .padding(24)
        .frame(width: 520)
        .fixedSize(horizontal: false, vertical: true)
        .interactiveDismissDisabled()
    }
}
