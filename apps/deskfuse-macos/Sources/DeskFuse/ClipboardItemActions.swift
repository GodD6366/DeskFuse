import AppKit
import SwiftUI
import UniformTypeIdentifiers
import DeskFuseEngine

/// Whether the menu-bar preview popover would show anything the card does not.
enum ClipboardPreviewPolicy {
    static func shouldPreview(_ item: HistoryItem, displayMode: ClipboardDisplayMode) -> Bool {
        if item.fileCount > 0 { return false }
        if item.hasImage { return true }
        return textIsTruncated(item.preview, displayMode: displayMode)
    }

    static func textIsTruncated(_ preview: String, displayMode: ClipboardDisplayMode) -> Bool {
        let text = preview.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return false }
        if preview.count >= 160 { return true }
        let lines = preview.split(omittingEmptySubsequences: false, whereSeparator: \.isNewline)
        switch displayMode {
        case .list:
            return lines.count > 1 || preview.count > 28
        case .grid:
            return lines.count > 8
        }
    }
}

enum ClipboardTextFormat: String, CaseIterable, Identifiable {
    case original
    case plain
    case dashBullets
    case starBullets
    case numbered
    case fullPath
    case markdown

    var id: Self { self }

    var menuTitle: String {
        switch self {
        case .original: "原始文本"
        case .plain: "纯文本"
        case .dashBullets: "带 - 短划线项目符号的纯文本"
        case .starBullets: "带 * 星形项目符号的纯文本"
        case .numbered: "带 1. 2. 3. 的纯文本"
        case .fullPath: "完整路径"
        case .markdown: "Markdown"
        }
    }
}

/// Shared copy / paste-as / save / reveal / edit actions for history rows.
@MainActor enum ClipboardItemActions {
    static func contentType(for item: HistoryItem) -> WorkflowContent {
        item.fileCount > 0 ? .files : item.hasImage ? .image : .text
    }

    static func copyAsFormats(for item: HistoryItem) -> [ClipboardTextFormat] {
        var formats: [ClipboardTextFormat] = [.original]
        if item.fileCount > 0 {
            formats += [.fullPath, .markdown]
        }
        if !item.hasImage || item.fileCount > 0 {
            formats += [.dashBullets, .starBullets, .numbered]
        }
        return formats
    }

    static func pasteAsFormats(for item: HistoryItem) -> [ClipboardTextFormat] {
        copyAsFormats(for: item)
    }

    static func copy(_ id: UInt64, format: ClipboardTextFormat, paste: Bool) {
        Task { await perform(id, format: format, paste: paste) }
    }

    static func save(_ item: HistoryItem) {
        Task { await saveContent(item) }
    }

    static func revealInFinder(_ id: UInt64) {
        Task { await reveal(id) }
    }

    static func openInPreview(_ id: UInt64) {
        Task { await openPreview(id) }
    }

    static func edit(_ item: HistoryItem, title: String?) {
        Task { await presentEditor(item, title: title) }
    }

    static func writePlainText(_ text: String) -> Int64 {
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(text, forType: .string)
        return Int64(pasteboard.changeCount)
    }

    /// 把编辑后的文本写回原条目：不产生新历史记录，也不触碰系统剪贴板。
    /// 只有真正带样式（粗体/斜体/下划线/删除线）时才附带 RTF 表示，
    /// 避免纯文本编辑后粘贴到富文本目标时带上编辑器的默认字体样式。
    static func updateText(_ id: UInt64, attributed: NSAttributedString) {
        let range = NSRange(location: 0, length: attributed.length)
        var styled = false
        attributed.enumerateAttributes(in: range) { attrs, _, stop in
            let font = attrs[.font] as? NSFont
            let styledFont = font.map {
                !NSFontManager.shared.traits(of: $0).isDisjoint(with: [.boldFontMask, .italicFontMask])
            } ?? false
            let underlined = ((attrs[.underlineStyle] as? NSNumber)?.intValue ?? 0) != 0
            let struckThrough = ((attrs[.strikethroughStyle] as? NSNumber)?.intValue ?? 0) != 0
            if styledFont || underlined || struckThrough { styled = true; stop.pointee = true }
        }
        let rtf = styled ? attributed.rtf(from: range).map(Array.init) : nil
        ClipboardLibraryStore.shared.updateText(id, text: attributed.string, rtf: rtf)
    }

    private static func perform(_ id: UInt64, format: ClipboardTextFormat, paste: Bool) async {
        guard let client = AppStore.shared.client else {
            report("引擎尚未就绪")
            return
        }
        do {
            let panelSession = ClipboardPanelController.shared.session
            if format == .original {
                let token = UUID().uuidString
                let changeCount = try await client.prepareClipboardPaste(id, session: token)
                if paste {
                    try await ClipboardPanelController.shared.pasteRestoredContent(
                        expectedChangeCount: changeCount,
                        in: panelSession
                    )
                } else if ClipboardPanelController.shared.isShown {
                    ClipboardPanelController.shared.close()
                }
                return
            }
            let rendered = try await render(id, format: format, client: client)
            let changeCount = writePlainText(rendered)
            if paste {
                try await ClipboardPanelController.shared.pasteRestoredContent(
                    expectedChangeCount: changeCount,
                    in: panelSession
                )
            } else if ClipboardPanelController.shared.isShown {
                ClipboardPanelController.shared.close()
            }
        } catch {
            report(error.userFacingMessage)
        }
    }

    private static func render(_ id: UInt64, format: ClipboardTextFormat, client: EngineClient) async throws -> String {
        let content = try await loadContent(id, client: client)
        switch format {
        case .original, .plain:
            return content.text
        case .dashBullets:
            return transformList(content.text, marker: .dash)
        case .starBullets:
            return transformList(content.text, marker: .star)
        case .numbered:
            return transformList(content.text, marker: .numbered)
        case .fullPath:
            let paths = content.filePaths
            guard !paths.isEmpty else { throw ClipboardActionFailure.message("没有可复制的路径") }
            return paths.joined(separator: "\n")
        case .markdown:
            let paths = content.filePaths
            guard !paths.isEmpty else { throw ClipboardActionFailure.message("没有可复制的路径") }
            return paths.map { path in
                let name = URL(fileURLWithPath: path).lastPathComponent
                let image = UTType(filenameExtension: URL(fileURLWithPath: path).pathExtension)?.conforms(to: .image) == true
                return image ? "![\(name)](\(path))" : "[\(name)](\(path))"
            }.joined(separator: "\n")
        }
    }

    private static func saveContent(_ item: HistoryItem) async {
        guard let client = AppStore.shared.client else { report("引擎尚未就绪"); return }
        do {
            let content = try await loadContent(item.id, client: client)
            if item.hasImage, !content.imagePng.isEmpty {
                await saveData(content.imagePng, name: "剪贴板图片.png", types: [.png])
            } else if !content.filePaths.isEmpty {
                let urls = content.filePaths.map { URL(fileURLWithPath: $0) }
                guard urls.allSatisfy({ FileManager.default.fileExists(atPath: $0.path) }) else {
                    throw ClipboardActionFailure.message("文件已不可用，请重新获取。")
                }
                let panel = NSOpenPanel()
                panel.canChooseFiles = false
                panel.canChooseDirectories = true
                panel.canCreateDirectories = true
                panel.prompt = "保存"
                panel.message = "选择保存位置"
                guard panel.runModal() == .OK, let folder = panel.url else { return }
                for url in urls {
                    let destination = folder.appendingPathComponent(url.lastPathComponent)
                    try FileManager.default.copyItem(at: url, to: uniqueURL(destination))
                }
            } else {
                let data = Data(content.text.utf8)
                await saveData(data, name: "剪贴板文本.txt", types: [.plainText])
            }
        } catch {
            report(error.userFacingMessage)
        }
    }

    private static func saveData(_ data: Data, name: String, types: [UTType]) async {
        let panel = NSSavePanel()
        panel.allowedContentTypes = types
        panel.canCreateDirectories = true
        panel.nameFieldStringValue = name
        guard panel.runModal() == .OK, let url = panel.url else { return }
        do {
            try await Task.detached { try data.write(to: url, options: .atomic) }.value
        } catch {
            report(error.userFacingMessage)
        }
    }

    private static func uniqueURL(_ url: URL) -> URL {
        let folder = url.deletingLastPathComponent()
        let stem = url.deletingPathExtension().lastPathComponent
        let ext = url.pathExtension
        var candidate = url
        var index = 2
        while FileManager.default.fileExists(atPath: candidate.path) {
            let name = ext.isEmpty ? "\(stem) \(index)" : "\(stem) \(index).\(ext)"
            candidate = folder.appendingPathComponent(name)
            index += 1
        }
        return candidate
    }

    private static func reveal(_ id: UInt64) async {
        guard let client = AppStore.shared.client else { report("引擎尚未就绪"); return }
        do {
            let content = try await loadContent(id, client: client)
            let urls = content.filePaths.map { URL(fileURLWithPath: $0) }
            guard !urls.isEmpty else { throw ClipboardActionFailure.message("没有可显示的文件") }
            guard urls.allSatisfy({ FileManager.default.fileExists(atPath: $0.path) }) else {
                ClipboardLibraryStore.shared.wantPayload(id)
                throw ClipboardActionFailure.message("文件尚未就绪，请等待获取完成后重试。")
            }
            NSWorkspace.shared.activateFileViewerSelecting(urls)
        } catch {
            report(error.userFacingMessage)
        }
    }

    private static func openPreview(_ id: UInt64) async {
        guard let client = AppStore.shared.client else { report("引擎尚未就绪"); return }
        do {
            let content = try await loadContent(id, client: client)
            var urls = content.filePaths.map { URL(fileURLWithPath: $0) }
                .filter { FileManager.default.fileExists(atPath: $0.path) }
            if urls.isEmpty, !content.imagePng.isEmpty {
                let url = FileManager.default.temporaryDirectory
                    .appendingPathComponent("deskfuse-preview-\(id).png")
                try content.imagePng.write(to: url, options: .atomic)
                urls = [url]
            }
            guard !urls.isEmpty else { throw ClipboardActionFailure.message("没有可打开的文件") }
            if let preview = NSWorkspace.shared.urlForApplication(withBundleIdentifier: "com.apple.Preview") {
                _ = try await NSWorkspace.shared.open(urls, withApplicationAt: preview, configuration: NSWorkspace.OpenConfiguration())
            } else {
                urls.forEach { NSWorkspace.shared.open($0) }
            }
        } catch {
            report(error.userFacingMessage)
        }
    }

    private static func presentEditor(_ item: HistoryItem, title: String?) async {
        guard let client = AppStore.shared.client else { report("引擎尚未就绪"); return }
        do {
            let content = try await loadContent(item.id, client: client)
            if item.hasImage, item.fileCount == 0, !content.imagePng.isEmpty {
                let url = FileManager.default.temporaryDirectory
                    .appendingPathComponent("deskfuse-edit-\(item.id).png")
                try content.imagePng.write(to: url, options: .atomic)
                if let preview = NSWorkspace.shared.urlForApplication(withBundleIdentifier: "com.apple.Preview") {
                    _ = try await NSWorkspace.shared.open([url], withApplicationAt: preview, configuration: NSWorkspace.OpenConfiguration())
                } else {
                    NSWorkspace.shared.open(url)
                }
                return
            }
            if item.fileCount > 0, !content.filePaths.isEmpty {
                await openPreview(item.id)
                return
            }
            ClipboardContentEditorController.shared.present(
                itemID: item.id,
                title: (title?.isEmpty == false ? title! : nil) ?? item.clipboardTitle,
                text: content.text
            )
        } catch {
            report(error.userFacingMessage)
        }
    }

    private static func loadContent(_ id: UInt64, client: EngineClient) async throws -> ClipboardContent {
        if let content = try? await client.content(id) { return content }
        ClipboardLibraryStore.shared.wantPayload(id)
        for _ in 0..<10 {
            try? await Task.sleep(for: .milliseconds(400))
            if let content = try? await client.content(id) { return content }
        }
        throw ClipboardActionFailure.message("内容尚未就绪，请等待获取完成后重试。")
    }

    private enum ListMarker { case dash, star, numbered }

    private static func transformList(_ text: String, marker: ListMarker) -> String {
        var index = 1
        return text.split(omittingEmptySubsequences: false, whereSeparator: \.isNewline).map { line in
            let raw = String(line)
            let body = stripListMarker(raw)
            if body.trimmingCharacters(in: .whitespaces).isEmpty { return raw }
            switch marker {
            case .dash: return "- \(body)"
            case .star: return "* \(body)"
            case .numbered:
                defer { index += 1 }
                return "\(index). \(body)"
            }
        }.joined(separator: "\n")
    }

    private static func stripListMarker(_ line: String) -> String {
        let trimmed = line.trimmingCharacters(in: .whitespaces)
        for prefix in ["- ", "* ", "• ", "· "] {
            if trimmed.hasPrefix(prefix) { return String(trimmed.dropFirst(prefix.count)) }
        }
        var digits = trimmed.startIndex
        while digits < trimmed.endIndex, trimmed[digits].isNumber {
            digits = trimmed.index(after: digits)
        }
        if digits > trimmed.startIndex, digits < trimmed.endIndex {
            let punct = trimmed[digits]
            if punct == "." || punct == ")" {
                let next = trimmed.index(after: digits)
                if next < trimmed.endIndex, trimmed[next].isWhitespace {
                    return String(trimmed[trimmed.index(after: next)...])
                }
            }
        }
        return trimmed
    }

    private static func report(_ message: String) {
        ClipboardPanelController.shared.reportPasteFailure(message)
        AppStore.shared.error = message
    }
}

private enum ClipboardActionFailure: LocalizedError {
    case message(String)
    var errorDescription: String? {
        switch self {
        case .message(let message): message
        }
    }
}
