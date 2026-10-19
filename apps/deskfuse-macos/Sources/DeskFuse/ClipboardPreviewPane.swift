import SwiftUI
import AppKit
import UniformTypeIdentifiers
import DeskFuseEngine

struct ClipboardPreviewPane: View, Equatable {
    let store: AppStore
    let item: HistoryItem
    let title: String?
    var appID: String? = nil
    let content: ClipboardContent
    let media: ClipboardMedia
    let image: NSImage?
    let ocrText: String?
    let copying: Bool
    let copied: Bool
    let copyContent: () -> Void
    @State private var fillImage = false
    @State private var exportingImage = false
    @State private var savedImageURL: URL?

    // Engine snapshots recreate the parent's copy closure even when this record
    // is unchanged. Keep selectable text out of those unrelated hosting updates.
    nonisolated static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.store === rhs.store && lhs.item == rhs.item
            && lhs.title == rhs.title && lhs.appID == rhs.appID
            && lhs.content == rhs.content && lhs.media == rhs.media
            && lhs.image === rhs.image && lhs.ocrText == rhs.ocrText
            && lhs.copying == rhs.copying && lhs.copied == rhs.copied
    }

    var body: some View {
        paneContent
            .id(item.id)
            .transition(.opacity)
            .animation(DFMotion.settle, value: item.id)
    }

    private var paneContent: some View {
        GeometryReader { geometry in
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    header
                    Group {
                        if let image {
                            imagePreview(image)
                                .frame(height: max(240, min(440, geometry.size.height * 0.55)))
                        }
                        else { previewCanvas }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    if let ocrText, !ocrText.isEmpty { ocrSection(ocrText) }
                    WorkflowRunStatusView(compact: true)
                    Divider()
                    metadataCard
                    VStack(alignment: .leading, spacing: 12) {
                        if let stagingHint {
                            Text(stagingHint)
                                .font(.caption).dfSecondaryText()
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        NativeGlassGroup {
                            actions.frame(maxWidth: .infinity, alignment: .trailing)
                        }
                    }
                }
                .padding(20)
                .frame(maxWidth: .infinity, alignment: .topLeading)
                .background(PreviewScrollIndicatorPolicy())
            }
            // macOS can override .hidden when a mouse is connected.
            .scrollIndicators(.never)
        }
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 10) {
            if let displayTitle {
                Text(displayTitle)
                    .font(DFType.title.weight(.semibold))
                    .lineLimit(2).truncationMode(.middle)
                    .textSelection(.enabled)
            }
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 10) { sourceLabel; if item.fileCount > 0 { availabilityBadge } }
                VStack(alignment: .leading, spacing: 8) { sourceLabel; if item.fileCount > 0 { availabilityBadge } }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .animation(DFMotion.snappy, value: availability)
    }

    private var sourceLabel: some View {
        Text("来自 \(item.source) · \(item.fileCount > 0 ? "\(item.fileCount) 项" : byteCount(item.length))")
            .font(.callout).dfSecondaryText().lineLimit(1)
            .help(item.copiedDate.map { "复制于 \($0.formatted(date: .abbreviated, time: .standard))" } ?? item.source)
    }

    private var availabilityBadge: some View {
        Label(availability, systemImage: availabilitySymbol)
            .font(.caption)
            .foregroundStyle(hasMissingFiles ? DFPalette.warning : DFPalette.accent)
            .padding(.horizontal, 8).padding(.vertical, 4)
            .background((hasMissingFiles ? DFPalette.warning : DFPalette.accent).opacity(0.1), in: RoundedRectangle(cornerRadius: DFRadius.control))
            .fixedSize()
            .transition(.scale(scale: 0.9).combined(with: .opacity))
    }

    private var previewCanvas: some View {
        Group {
            if !content.filePaths.isEmpty {
                filesPreview.frame(height: min(280, CGFloat(content.filePaths.count) * 82 + 24))
            } else if item.fileCount > 0 {
                ContentUnavailableView(content.canRequestFiles ? "文件尚未在本地" : "文件不可用", systemImage: "doc.badge.arrow.up", description: Text(content.canRequestFiles ? "使用时从来源设备获取；来源设备需要保持连接。" : "来源文件或缓存已不可用，请重新复制。"))
            } else if item.hasImage {
                ContentUnavailableView("暂时无法显示图片", systemImage: "photo", description: Text("可以尝试复制原始内容，或从来源设备重新复制。"))
            } else {
                textPreview
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// Recognized image text is local derived data shown read-only so the user
    /// can confirm what search matched and copy it if useful.
    private func ocrSection(_ text: String) -> some View {
        DisclosureGroup {
            ScrollView {
                Text(text)
                    .font(DFType.detail)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.top, 6)
            }
            .frame(maxHeight: 140)
        } label: {
            Label("识别文字", systemImage: "text.viewfinder").font(.callout)
        }
        .padding(12)
    }

    private func imagePreview(_ image: NSImage) -> some View {
        GeometryReader { geometry in
            let aspect = max(image.size.width, 1) / max(image.size.height, 1)
            let width = max(1, fillImage ? geometry.size.width : min(geometry.size.width, geometry.size.height * aspect))
            let height = max(1, fillImage ? geometry.size.height : width / aspect)
            Image(nsImage: image)
                .resizable()
                .aspectRatio(contentMode: fillImage ? .fill : .fit)
                .frame(width: width, height: height)
                .clipShape(RoundedRectangle(cornerRadius: DFRadius.card))
                .accessibilityLabel(displayTitle ?? item.clipboardTitle)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .center)
        }
    }

    private var textPreview: some View {
        VStack(spacing: 0) {
        if let document = ClipboardCodeDocument.detect(content.text) {
            ClipboardCodePreview(document: document)
                .frame(height: min(300, CGFloat(document.text.components(separatedBy: "\n").count) * 22 + 80))
            if content.truncated {
                Text("预览已截断；复制会恢复完整内容。")
                    .font(.caption).dfSecondaryText().padding(8)
            }
        } else {
            VStack(alignment: .leading, spacing: 16) {
                Text(content.text)
                    .font(DFType.title)
                    .lineSpacing(6)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                if content.truncated {
                    Label("预览已截断；复制会恢复完整内容。", systemImage: "info.circle")
                        .font(.caption).dfSecondaryText()
                }
            }
            .padding(.vertical, 12)
        }
        }
    }

    private var filesPreview: some View {
        ScrollView {
            LazyVStack(spacing: 0) {
                ForEach(content.filePaths, id: \.self) { path in
                    HStack(spacing: 16) {
                        Image(nsImage: NSWorkspace.shared.icon(forFile: path))
                            .resizable().frame(width: 44, height: 44)
                        VStack(alignment: .leading, spacing: 6) {
                            Text(URL(fileURLWithPath: path).lastPathComponent)
                                .font(DFType.icon(.regular).weight(.medium))
                                .lineLimit(2).truncationMode(.middle)
                            Text(media.localPaths.contains(path) ? "本地可用" : "文件已不可用，请重新获取")
                                .font(.caption)
                                .foregroundStyle(media.localPaths.contains(path) ? Color.secondary : .orange)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .padding(.vertical, 18)
                    Divider()
                }
            }
            .padding(.horizontal, 24)
            .padding(.top, 8)
            .padding(.bottom, 16)
        }
    }

    private var imageSizingMenu: some View {
        Menu {
            Picker("预览方式", selection: $fillImage) {
                Text("适合窗口（显示完整图片）").tag(false)
                Text("填满预览（裁切边缘）").tag(true)
            }
        } label: {
            Label(fillImage ? "填满预览" : "适合窗口", systemImage: "arrow.up.left.and.arrow.down.right")
                .modifier(DFControlLabel())
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .fixedSize()
        .frame(minHeight: DFMetrics.controlHeight)
        .modifier(NativeGlassBackdrop(cornerRadius: DFRadius.control))
        .help("调整图片在预览区的显示方式，不改变原图")
    }

    private var actions: some View {
        ViewThatFits(in: .horizontal) {
            actionRow(compact: false)
            actionRow(compact: true)
            VStack(alignment: .trailing, spacing: 8) {
                copyButton
                stagingButton(compact: true)
                moreMenu
            }
        }
    }

    private func actionRow(compact: Bool) -> some View {
        HStack(spacing: 10) {
            copyButton
            stagingButton(compact: compact)
            moreMenu
        }
        .fixedSize(horizontal: true, vertical: false)
    }

    private var copyButton: some View {
        DFActionButton(copying ? "正在准备…" : copied ? "已复制" : "复制",
                              systemImage: copied ? "checkmark" : "doc.on.doc",
                              help: hasMissingFiles ? "来源文件已不可用，请重新复制" : "将原始内容放回剪贴板",
                              kind: copied ? .secondary : .primary, action: copyContent)
            .disabled(copying || hasMissingFiles)
            .keyboardShortcut("c", modifiers: .command)
            /// 复制成功：实心主按钮退回 glass，对勾用品牌 accent 色。
            .foregroundStyle(copied ? DFPalette.accent : Color.primary)
    }

    @ViewBuilder private func stagingButton(compact: Bool) -> some View {
        if !content.imagePng.isEmpty && content.filePaths.isEmpty && !canStageFiles {
            DFActionButton(exportingImage ? "正在保存…" : "保存图片…", systemImage: "square.and.arrow.down", action: saveImage)
                .disabled(exportingImage)
        } else if item.fileCount > 0 || image != nil || savedImageURL != nil {
            DFActionButton(compact ? "暂存" : "加入暂存区", systemImage: "tray.and.arrow.down", action: stageFiles)
                .disabled(!canStageFiles || store.shelf == nil || store.shelf?.preparing == true)
                .help(stagingHint ?? "暂存本地文件，可从暂存区拖出或发送")
        }
    }

    private var moreMenu: some View {
        Menu {
            Button("复制原始内容", systemImage: "doc.on.doc", action: copyContent)
                .disabled(copying || hasMissingFiles)
            if item.fileCount == 0 {
                Button(item.hasImage ? "编辑图片" : "编辑文本", systemImage: "pencil") {
                    ClipboardItemActions.edit(item, title: title)
                }
            }
            Button(item.hasImage ? "保存图片…" : item.fileCount > 0 ? "保存文件…" : "保存文本…", systemImage: "square.and.arrow.down") {
                ClipboardItemActions.save(item)
            }
            WorkflowMenu(store: store, historyID: item.id,
                         contentType: item.fileCount > 0 ? .files : (item.hasImage ? .image : .text))
            WorkflowShareMenu(store: store, historyID: item.id,
                              contentType: item.fileCount > 0 ? .files : (item.hasImage ? .image : .text))
            if !content.filePaths.isEmpty {
                Button("在访达中显示", systemImage: "folder", action: revealFiles)
                    .disabled(!canStageFiles)
                Button("在“预览”中打开", systemImage: "eye") {
                    ClipboardItemActions.openInPreview(item.id)
                }
            }
            Divider()
            Button("打开文件暂存区", systemImage: "tray") { store.shelf?.show() }
        } label: {
            Image(systemName: "ellipsis")
                .frame(width: DFMetrics.controlHeight, height: DFMetrics.controlHeight)
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .fixedSize()
        .frame(minWidth: DFMetrics.controlHeight, minHeight: DFMetrics.controlHeight)
        .modifier(NativeGlassBackdrop(cornerRadius: DFRadius.control))
        .accessibilityLabel("更多操作")
    }

    private var footer: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 12) {
                metadata
                Spacer(minLength: 8)
                if image != nil { imageSizingMenu }
                else { finderButton }
            }
            VStack(alignment: .leading, spacing: 8) {
                metadata
                if image != nil { imageSizingMenu }
                else { finderButton }
            }
        }
        .font(.caption).dfSecondaryText()
    }

    private var metadataCard: some View {
        VStack(alignment: .leading, spacing: 10) {
            metadataRow("种类", value: formatName)
            metadataRow("大小", value: item.fileCount > 0 ? "\(byteCount(item.length))（\(item.fileCount) 项）" : byteCount(item.length))
            if let width = media.pixelWidth, let height = media.pixelHeight {
                metadataRow("尺寸", value: "\(width) × \(height)")
            }
            metadataRow("来源设备", value: item.source)
            if let appID, !appID.isEmpty {
                metadataRow("来源应用", value: ClipboardAppInfoCache.shared.name(for: appID) ?? appID)
            }
            if let date = item.copiedDate {
                metadataRow("添加时间", value: date.formatted(date: .abbreviated, time: .shortened))
            }
        }
        .font(.callout)
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func metadataRow(_ label: String, value: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 14) {
            Text(label)
                .dfSecondaryText()
                .frame(width: 62, alignment: .leading)
            Text(value)
                .foregroundStyle(.primary)
                .textSelection(.enabled)
                .lineLimit(1)
                .truncationMode(.middle)
        }
    }

    private var metadata: some View {
        HStack(spacing: 8) {
            Text(formatName)
            if let width = media.pixelWidth, let height = media.pixelHeight {
                Text("\(width) × \(height)").monospacedDigit()
            } else {
                Label("保留原始格式", systemImage: "checkmark")
            }
        }
        .lineLimit(1)
    }

    @ViewBuilder private var finderButton: some View {
        if !content.filePaths.isEmpty {
            Button(action: revealFiles) {
                Label("在访达中显示", systemImage: "arrow.up.right.square")
                    .labelStyle(.titleAndIcon)
            }
            .buttonStyle(.plain)
            .disabled(!canStageFiles)
        }
    }

    private var displayTitle: String? {
        if let title = title?.trimmingCharacters(in: .whitespacesAndNewlines), !title.isEmpty {
            return title
        }
        if content.filePaths.count == 1, let path = content.filePaths.first {
            return URL(fileURLWithPath: path).lastPathComponent
        }
        return nil
    }

    private var canStageFiles: Bool {
        if let savedImageURL { return FileManager.default.fileExists(atPath: savedImageURL.path) }
        return !content.filePaths.isEmpty
            && (item.fileCount == 0 || content.filePaths.count == Int(item.fileCount))
            && content.filePaths.allSatisfy { media.localPaths.contains($0) }
    }
    private var stagingHint: String? {
        if savedImageURL != nil { return canStageFiles ? "图片已保存，可加入暂存区。" : "已保存的图片不在原位置，请重新保存。" }
        if !content.imagePng.isEmpty && content.filePaths.isEmpty { return "这是剪贴板图片；先保存为文件，即可加入暂存区。" }
        if item.fileCount > 0 && !canStageFiles {
            return content.canRequestFiles ? "文件尚未在本地；先复制以获取文件，来源设备需保持连接。" : "来源文件已不可用，请重新复制文件。"
        }
        return nil
    }
    private var hasMissingFiles: Bool { item.fileCount > 0 && !content.canRequestFiles && !canStageFiles }
    private var availability: String {
        if hasMissingFiles { return "文件不可用" }
        if canStageFiles { return "本地可用" }
        return item.fileCount > 0 ? "使用时获取" : "可复制"
    }
    private var availabilitySymbol: String {
        hasMissingFiles ? "exclamationmark.triangle" : item.fileCount > 0 && !canStageFiles ? "arrow.down.circle" : "checkmark"
    }
    private var formatName: String {
        if let path = content.filePaths.first, content.filePaths.count == 1,
           let type = UTType(filenameExtension: URL(fileURLWithPath: path).pathExtension) {
            return type.localizedDescription ?? "文件"
        }
        if item.fileCount > 0 { return "\(item.fileCount) 个文件" }
        if let type = UTType(mimeType: item.mime), let name = type.localizedDescription { return name }
        return item.hasImage ? "图片" : "文字"
    }

    private func stageFiles() {
        guard canStageFiles else { return }
        // ShelfController acquires the existing Rust cache lease before staging.
        store.shelf?.append(savedImageURL.map { [$0] } ?? content.filePaths.map { URL(fileURLWithPath: $0) })
    }

    private func saveImage() {
        guard !exportingImage, !content.imagePng.isEmpty else { return }
        let panel = NSSavePanel()
        panel.allowedContentTypes = [.png]
        panel.nameFieldStringValue = "剪贴板图片.png"
        exportingImage = true
        let original = content.imagePng
        panel.begin { response in
            guard response == .OK, let url = panel.url else { exportingImage = false; return }
            Task { @MainActor in
                defer { exportingImage = false }
                do {
                    try await Task.detached { try original.write(to: url, options: .atomic) }.value
                    savedImageURL = url
                } catch { store.error = error.userFacingMessage }
            }
        }
    }
    private func revealFiles() {
        guard canStageFiles, content.filePaths.allSatisfy({ FileManager.default.fileExists(atPath: $0) }) else {
            store.error = "文件已不可用，请重新获取。"; return
        }
        NSWorkspace.shared.activateFileViewerSelecting(content.filePaths.map { URL(fileURLWithPath: $0) })
    }
}

/// Scope the native fallback to the preview; never reconfigure unrelated scroll views.
private struct PreviewScrollIndicatorPolicy: NSViewRepresentable {
    final class Marker: NSView {
        override func hitTest(_ point: NSPoint) -> NSView? { nil }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            applyPolicy()
        }

        override func layout() {
            super.layout()
            applyPolicy()
        }

        func applyPolicy() {
            if let scroll = enclosingScrollView {
                if scroll.hasVerticalScroller { scroll.hasVerticalScroller = false }
                if scroll.hasHorizontalScroller { scroll.hasHorizontalScroller = false }
            }
        }
    }

    func makeNSView(context: Context) -> Marker { Marker() }
    func updateNSView(_ view: Marker, context: Context) { view.applyPolicy() }
}
