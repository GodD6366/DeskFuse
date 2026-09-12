import SwiftUI
import AppKit
import UniformTypeIdentifiers
import UniDeskEngine

struct ClipboardPreviewPane: View {
    @Bindable var store: AppStore
    let item: HistoryItem
    let content: ClipboardContent
    let media: ClipboardMedia
    let image: NSImage?
    @State private var fillImage = false

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            header
            NativeGlassGroup {
                if let image {
                    imagePreview(image)
                } else {
                    previewCanvas
                        .overlay(alignment: .bottom) {
                            actions.padding(.horizontal, 12).padding(.bottom, 16)
                        }
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            WorkflowRunStatusView(compact: true)
            footer
        }
        .padding(24)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    private var header: some View {
        HStack(alignment: .top, spacing: 16) {
            VStack(alignment: .leading, spacing: 8) {
                Text(title)
                    .font(.system(size: 22, weight: .bold))
                    .lineLimit(2).truncationMode(.middle)
                    .textSelection(.enabled)
                Text("来自 \(item.source) · \(byteCount(item.length))")
                    .font(.callout).foregroundStyle(.secondary).lineLimit(1)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            Label(availability, systemImage: availabilitySymbol)
                .font(.caption)
                .foregroundStyle(hasMissingFiles ? Color.orange : .secondary)
                .padding(.horizontal, 10).padding(.vertical, 6)
                .overlay(Capsule().strokeBorder(.quaternary, lineWidth: 1))
                .fixedSize()
                .padding(.top, 4)
        }
    }

    private var previewCanvas: some View {
        Group {
            if !content.filePaths.isEmpty {
                filesPreview
            } else if item.fileCount > 0 {
                ContentUnavailableView("文件尚未在本地", systemImage: "doc.badge.arrow.up", description: Text("复制后粘贴时获取文件；来源设备需要保持连接。"))
            } else if item.hasImage {
                ContentUnavailableView("暂时无法显示图片", systemImage: "photo", description: Text("可以尝试复制原始内容，或从来源设备重新复制。"))
            } else {
                textPreview
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(nsColor: .controlBackgroundColor))
        .clipShape(RoundedRectangle(cornerRadius: 14))
    }

    private func imagePreview(_ image: NSImage) -> some View {
        GeometryReader { geometry in
            let aspect = max(image.size.width, 1) / max(image.size.height, 1)
            let fittedWidth = fillImage ? geometry.size.width : min(geometry.size.width, geometry.size.height * aspect)
            let fittedHeight = fillImage ? geometry.size.height : fittedWidth / aspect
            // Attach controls to the rendered image, not the letterboxed canvas.
            // Narrow or shallow images reserve space for controls below the image.
            let inlineControls = fittedHeight >= 180 && fittedWidth >= 320
            let availableHeight = max(1, geometry.size.height - (inlineControls ? 0 : 112))
            let width = fillImage ? geometry.size.width : min(geometry.size.width, availableHeight * aspect)
            let height = fillImage ? availableHeight : width / aspect
            VStack(spacing: 12) {
                Image(nsImage: image)
                    .resizable()
                    .aspectRatio(contentMode: fillImage ? .fill : .fit)
                    .frame(width: width, height: height)
                    .clipShape(RoundedRectangle(cornerRadius: 14))
                    .accessibilityLabel(title)
                    .overlay(alignment: .topTrailing) {
                        if inlineControls { imageSizingMenu.padding(12) }
                    }
                    .overlay(alignment: .bottom) {
                        if inlineControls { actions.padding(.horizontal, 12).padding(.bottom, 12) }
                    }
                if !inlineControls {
                    ViewThatFits(in: .horizontal) {
                        HStack { imageSizingMenu; Spacer(minLength: 8); actions }
                        VStack(spacing: 8) { imageSizingMenu; actions }
                    }
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        }
    }

    private var textPreview: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                Text(content.text)
                    .font(.system(size: 15))
                    .lineSpacing(6)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                if content.truncated {
                    Label("预览已截断；复制会恢复完整内容。", systemImage: "info.circle")
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
            .padding(24)
            // The last line can scroll above the floating action bar.
            .padding(.bottom, 72)
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
                                .font(.system(size: 14, weight: .medium))
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
            .padding(.bottom, 88)
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
                .font(.caption)
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .fixedSize()
        // Native macOS Menu labels do not preserve label-internal padding.
        // Size the glass surface around the control instead.
        .padding(.horizontal, 12).padding(.vertical, 10)
        .modifier(NativeGlassBackdrop(cornerRadius: 20))
        .help("调整图片在预览区的显示方式，不改变原图")
    }

    private var actions: some View {
        HStack(spacing: 10) {
            Button("复制", systemImage: "doc.on.doc", action: restore)
                .modifier(NativeGlassAction())
                .controlSize(.large)
                .buttonBorderShape(.capsule)
                .keyboardShortcut("c", modifiers: .command)
                .help("将这条历史的原始内容放回剪贴板")
            if item.fileCount > 0 || image != nil {
                Divider().frame(height: 22).padding(.horizontal, 3)
                Button("加入暂存区", systemImage: "tray.and.arrow.down", action: stageFiles)
                    .buttonStyle(.plain)
                    .padding(.vertical, 8)
                    .disabled(!canStageFiles || store.shelf?.preparing == true)
                    .help(canStageFiles ? "暂存本地文件，可从暂存区拖出或发送" : "暂存区接受本地文件；请先在 Finder 中保存或重新获取文件")
            }
            Menu {
                Button("复制原始内容", systemImage: "doc.on.doc", action: restore)
                if item.hasImage || image != nil || item.fileCount == 1 {
                    ImageWorkflowMenu(store: store, historyID: item.id)
                }
                if !content.filePaths.isEmpty {
                    Button("在 Finder 中显示", systemImage: "folder", action: revealFiles)
                        .disabled(!canStageFiles)
                }
                Divider()
                Button("打开文件暂存区", systemImage: "tray") { store.shelf?.show() }
            } label: {
                Image(systemName: "ellipsis").frame(width: 26, height: 30)
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .fixedSize()
            .accessibilityLabel("更多操作")
        }
        .font(.system(size: 12, weight: .medium))
        .padding(8)
        .modifier(NativeGlassBackdrop(cornerRadius: 28))
    }

    private var footer: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 12) {
                metadata
                Spacer(minLength: 8)
                finderButton
            }
            VStack(alignment: .leading, spacing: 8) {
                metadata
                finderButton
            }
        }
        .font(.caption).foregroundStyle(.secondary)
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
                Label("在 Finder 中显示", systemImage: "arrow.up.right.square")
                    .labelStyle(.titleAndIcon)
            }
            .buttonStyle(.plain)
            .disabled(!canStageFiles)
        }
    }

    private var title: String {
        if content.filePaths.count == 1, let path = content.filePaths.first {
            return URL(fileURLWithPath: path).lastPathComponent
        }
        return item.clipboardTitle
    }

    private var canStageFiles: Bool {
        !content.filePaths.isEmpty && content.filePaths.allSatisfy { media.localPaths.contains($0) }
    }
    private var hasMissingFiles: Bool { !content.filePaths.isEmpty && !canStageFiles }
    private var availability: String {
        if hasMissingFiles { return "文件不可用" }
        if canStageFiles { return "本地可用" }
        return item.fileCount > 0 ? "粘贴时获取" : "历史中可用"
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

    private func restore() { store.send(.restoreHistory(id: content.id)) }
    private func stageFiles() {
        guard canStageFiles else { return }
        // ShelfController acquires the existing Rust cache lease before staging.
        store.shelf?.append(content.filePaths.map { URL(fileURLWithPath: $0) })
    }
    private func revealFiles() {
        guard canStageFiles, content.filePaths.allSatisfy({ FileManager.default.fileExists(atPath: $0) }) else {
            store.error = "文件已不可用，请重新获取。"; return
        }
        NSWorkspace.shared.activateFileViewerSelecting(content.filePaths.map { URL(fileURLWithPath: $0) })
    }
}
