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

/// Shared copy / paste-as / save / reveal / edit actions for history rows.
@MainActor enum ClipboardItemActions {
    static func contentType(for item: HistoryItem) -> WorkflowContent {
        item.fileCount > 0 ? .files : item.hasImage ? .image : .text
    }

    /// 纯文本只对文字条目有意义；图片与文件始终恢复原始内容。
    static func canCopyPlainText(_ item: HistoryItem) -> Bool {
        item.fileCount == 0 && !item.hasImage
    }

    static func copy(_ id: UInt64, plainText: Bool = false, paste: Bool) {
        Task { await perform(id, plainText: plainText, paste: paste) }
    }

    static func save(_ item: HistoryItem) {
        Task { await saveContent(id: item.id, hasImage: item.hasImage, fileCount: Int(item.fileCount)) }
    }

    /// WebView 页面只有条目 id 与几个展示字段；保存只需要这三项。
    static func save(id: UInt64, hasImage: Bool, fileCount: Int) {
        Task { await saveContent(id: id, hasImage: hasImage, fileCount: fileCount) }
    }

    static func revealInFinder(_ id: UInt64) {
        Task { await reveal(id) }
    }

    static func openInPreview(_ id: UInt64) {
        Task { await openPreview(id) }
    }

    /// `point` 是触发编辑的屏幕坐标（右键位置等），编辑窗口就近弹出；
    /// 键盘触发时为 nil，窗口居中。
    static func edit(_ item: HistoryItem, title: String?, at point: NSPoint? = nil) {
        Task { await presentEditor(id: item.id, title: title ?? item.clipboardTitle, hasImage: item.hasImage, fileCount: Int(item.fileCount), at: point) }
    }

    static func edit(id: UInt64, title: String?, hasImage: Bool, fileCount: Int, at point: NSPoint? = nil) {
        Task { await presentEditor(id: id, title: title, hasImage: hasImage, fileCount: fileCount, at: point) }
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

    /// WebView 面板/页面等待真实完成：返回写入后的 change count，失败抛出原始错误。
    static func performAwaiting(id: UInt64, plainText: Bool, paste: Bool) async throws -> Int64 {
        guard let client = AppStore.shared.client else { throw ClipboardActionFailure.message("引擎尚未就绪") }
        let changeCount = plainText
            ? writePlainText(try await loadContent(id, client: client).text)
            : try await client.prepareClipboardPaste(id, session: UUID().uuidString)
        if paste {
            try await ClipboardPanelController.shared.pasteFromWebViewPanel(expectedChangeCount: changeCount)
        } else {
            ClipboardPanelController.shared.webViewPanelDidClose()
        }
        return changeCount
    }

    /// Copy or paste from the WebView clipboard panel. The panel runs in another
    /// process, so the AppKit popover session check does not apply; focus
    /// handoff and change-count validation still do.
    static func copyFromWebViewPanel(_ id: UInt64, plainText: Bool = false, paste: Bool) {
        Task { await performFromWebViewPanel(id, plainText: plainText, paste: paste) }
    }

    private static func performFromWebViewPanel(_ id: UInt64, plainText: Bool, paste: Bool) async {
        guard let client = AppStore.shared.client else { report("引擎尚未就绪"); return }
        do {
            let changeCount = plainText
                ? writePlainText(try await loadContent(id, client: client).text)
                : try await client.prepareClipboardPaste(id, session: UUID().uuidString)
            guard paste else {
                ClipboardPanelController.shared.webViewPanelDidClose()
                return
            }
            try await ClipboardPanelController.shared.pasteFromWebViewPanel(expectedChangeCount: changeCount)
        } catch {
            report(error.userFacingMessage)
        }
    }

    private static func perform(_ id: UInt64, plainText: Bool, paste: Bool) async {
        guard let client = AppStore.shared.client else {
            report("引擎尚未就绪")
            return
        }
        do {
            let panelSession = ClipboardPanelController.shared.session
            if !plainText {
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
            let changeCount = writePlainText(try await loadContent(id, client: client).text)
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

    private static func saveContent(id: UInt64, hasImage: Bool, fileCount: Int) async {
        guard let client = AppStore.shared.client else { report("引擎尚未就绪"); return }
        do {
            let content = try await loadContent(id, client: client)
            if hasImage, !content.imagePng.isEmpty {
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

    /// WebView 的保存动作等待用户选完保存位置：返回是否保存以及保存路径，
    /// 失败抛出真实错误，页面据此提示，而不是只确认“已派发”。
    static func saveAwaiting(id: UInt64, hasImage: Bool, fileCount: Int) async throws -> [String: Any] {
        guard let client = AppStore.shared.client else { throw ClipboardActionFailure.message("引擎尚未就绪") }
        let content = try await loadContent(id, client: client)
        if hasImage, !content.imagePng.isEmpty {
            return try await saveDataAwaiting(content.imagePng, name: "剪贴板图片.png", types: [.png])
        }
        if !content.filePaths.isEmpty {
            let urls = content.filePaths.map { URL(fileURLWithPath: $0) }
            guard urls.allSatisfy({ FileManager.default.fileExists(atPath: $0.path) }) else {
                throw ClipboardActionFailure.message("文件已不可用，请重新获取。")
            }
            guard let folder = await chooseFolderAwaiting() else { return ["saved": false] }
            for url in urls {
                let destination = folder.appendingPathComponent(url.lastPathComponent)
                try FileManager.default.copyItem(at: url, to: uniqueURL(destination))
            }
            return ["saved": true, "path": folder.path]
        }
        return try await saveDataAwaiting(Data(content.text.utf8), name: "剪贴板文本.txt", types: [.plainText])
    }

    /// WebView 保存纯文本（如鼠标适配数据 Markdown）：等待用户选完保存位置。
    static func saveTextAwaiting(_ text: String, name: String) async throws -> [String: Any] {
        try await saveDataAwaiting(Data(text.utf8), name: name,
                                   types: [UTType(filenameExtension: "md") ?? .plainText])
    }

    static func saveDataAwaiting(_ data: Data, name: String, types: [UTType]) async throws -> [String: Any] {
        let panel = NSSavePanel()
        panel.allowedContentTypes = types
        panel.canCreateDirectories = true
        panel.nameFieldStringValue = name
        NSApp.activate(ignoringOtherApps: true)
        autoResolveSmokePanel(panel)
        let url: URL? = await withCheckedContinuation { continuation in
            panel.begin { response in
                continuation.resume(returning: response == .OK ? panel.url : nil)
            }
        }
        guard let url else { return ["saved": false] }
        try await Task.detached { try data.write(to: url, options: .atomic) }.value
        return ["saved": true, "path": url.path]
    }

    private static func chooseFolderAwaiting() async -> URL? {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.canCreateDirectories = true
        panel.prompt = "保存"
        panel.message = "选择保存位置"
        NSApp.activate(ignoringOtherApps: true)
        autoResolveSmokePanel(panel)
        return await withCheckedContinuation { continuation in
            panel.begin { response in
                continuation.resume(returning: response == .OK ? panel.url : nil)
            }
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

    /// 冒烟专用：设置 `DESKFUSE_UI_SMOKE_SAVE` 时自动取消保存面板，让
    /// “保存结果回传（取消）”能在无人值守的冒烟里被真实走通。正常启动不生效。
    /// （NSSavePanel 没有可用的程序化“确认”入口：`ok:` 未实现，确认路径只由
    /// 用户点击完成。）
    private static func autoResolveSmokePanel(_ panel: NSSavePanel) {
        guard ProcessInfo.processInfo.environment["DESKFUSE_UI_SMOKE_SAVE"] != nil else { return }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.8) {
            panel.cancel(nil)
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

    private static func presentEditor(id: UInt64, title: String?, hasImage: Bool, fileCount: Int, at point: NSPoint?) async {
        guard let client = AppStore.shared.client else { report("引擎尚未就绪"); return }
        do {
            let content = try await loadContent(id, client: client)
            if hasImage, fileCount == 0, !content.imagePng.isEmpty {
                let url = FileManager.default.temporaryDirectory
                    .appendingPathComponent("deskfuse-edit-\(id).png")
                try content.imagePng.write(to: url, options: .atomic)
                if let preview = NSWorkspace.shared.urlForApplication(withBundleIdentifier: "com.apple.Preview") {
                    _ = try await NSWorkspace.shared.open([url], withApplicationAt: preview, configuration: NSWorkspace.OpenConfiguration())
                } else {
                    NSWorkspace.shared.open(url)
                }
                return
            }
            if fileCount > 0, !content.filePaths.isEmpty {
                await openPreview(id)
                return
            }
            ClipboardContentEditorController.shared.present(
                itemID: id,
                title: title.flatMap { $0.isEmpty ? nil : $0 } ?? "",
                text: content.text,
                rtf: content.rtf,
                at: point
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
