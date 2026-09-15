import SwiftUI
import AppKit
import UniDeskEngine

struct MenuBarClipboardRow: View {
    let item: HistoryItem
    let index: Int
    let showShortcuts: Bool
    let selected: Bool
    let client: EngineClient?
    let appID: String?
    let title: String?
    let memberships: [String]
    let remoteUnavailable: Bool
    let ocrHit: Bool
    let presentation: UUID
    let active: Bool
    let availability: (Bool) -> Void
    let select: () -> Void
    let onCopy: () -> Void
    let onRename: (String) -> Void
    let onDelete: () -> Void
    @State private var image: NSImage?
    @State private var media: ClipboardMedia?
    @State private var fileUnavailable = false
    @State private var requestFiles = false
    @State private var loading = false
    @State private var editingTitle = false
    private var loadKey: String { "\(item.id)-\(item.copiedAtMs ?? 0)-\(presentation)-\(active)" }

    var body: some View {
        Button(action: select) {
            HStack(spacing: 12) {
                // Leading: the app that produced the copy.
                appIcon
                    .frame(width: 28, height: 28)
                // Middle: the content itself; images get a real preview.
                VStack(alignment: .leading, spacing: 6) {
                    content
                    if requestFiles {
                        Text("使用时从来源设备获取").font(.system(size: 11)).foregroundStyle(.secondary)
                    }
                    if fileUnavailable {
                        Label("文件不可用", systemImage: "exclamationmark.triangle")
                            .font(.system(size: 11)).foregroundStyle(.orange)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                VStack(alignment: .trailing, spacing: 15) {
                    HStack(spacing: 4) {
                        if ocrHit {
                            Image(systemName: "text.viewfinder").font(.system(size: 11))
                                .foregroundStyle(.secondary).help("由图片识别文字匹配")
                        }
                        if remoteUnavailable {
                            Image(systemName: "arrow.down.circle").font(.system(size: 11))
                                .foregroundStyle(.secondary).help("使用时从来源设备获取")
                        }
                        if let date = item.copiedDate {
                            Text(Self.timeFormatter.string(from: date)).font(.system(size: 11)).foregroundStyle(.secondary)
                        }
                    }
                    if index < 9 {
                        Text("⌘ \(index + 1)").font(.system(size: 12)).monospacedDigit()
                            .opacity(showShortcuts ? 1 : 0).accessibilityHidden(!showShortcuts)
                    }
                }
                .fixedSize(horizontal: true, vertical: false)
            }
            .foregroundStyle(.primary)
            .padding(.horizontal, 10).padding(.vertical, 10)
            .frame(minHeight: item.hasImage ? 84 : 56)
            .background(selected ? Color.accentColor.opacity(0.12) : .clear, in: RoundedRectangle(cornerRadius: 10))
            .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(selected ? Color.accentColor.opacity(0.7) : .clear, lineWidth: 1))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel("\(item.clipboardTitle)\(fileUnavailable ? "，文件不可用" : "")")
        .accessibilityAddTraits(selected ? [.isSelected] : [])
        .contextMenu {
            Button("仅复制", action: onCopy).disabled(client == nil)
            Divider()
            Button("编辑标题") { editingTitle = true }
            Divider()
            ClipboardLibraryMembershipMenu(itemID: item.id, listIDs: memberships)
            Divider()
            Button("删除", role: .destructive, action: onDelete)
        }
        .task(id: loadKey) { await loadPreview() }
    }

    @ViewBuilder private var appIcon: some View {
        if let icon = ClipboardAppInfoCache.shared.icon(for: appID) {
            Image(nsImage: icon).resizable().scaledToFit()
                .clipShape(RoundedRectangle(cornerRadius: 6))
                .accessibilityHidden(true)
        } else {
            Image(systemName: media?.firstFileIsDirectory == true ? "folder.fill" : item.clipboardSymbol)
                .font(.system(size: 20, weight: .regular))
                .foregroundStyle(media?.firstFileIsDirectory == true ? Color.accentColor : Color.secondary)
        }
    }

    @ViewBuilder private var content: some View {
        if item.hasImage {
            if let image {
                VStack(alignment: .leading, spacing: 5) {
                    Image(nsImage: image).resizable().aspectRatio(contentMode: .fit)
                        .frame(width: 108, height: 72)
                        .clipShape(RoundedRectangle(cornerRadius: 6))
                    if editingTitle || title?.isEmpty == false { titleEditor }
                }
            } else {
                // Keep a stable slot while the thumbnail loads so rows never jump.
                RoundedRectangle(cornerRadius: 6)
                    .fill(Color(nsColor: .quaternaryLabelColor).opacity(0.25))
                    .frame(width: 108, height: 72)
                    .overlay { if loading { ProgressView().controlSize(.small) } }
            }
        } else {
            titleEditor
        }
    }

    private var titleEditor: some View {
        ClipboardEditableTitle(
            title: title,
            fallback: item.clipboardTitle,
            editing: $editingTitle,
            onCommit: onRename
        )
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private static let timeFormatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.dateFormat = "HH:mm"
        return formatter
    }()

    @MainActor private func loadPreview() async {
        // Reuse the decoded thumbnail so reopening the panel never flashes or
        // jumps while the same image is read again.
        if item.hasImage, let cached = ClipboardThumbnailCache.shared.cached(item.id, maxPixelSize: 320) {
            image = cached; media = nil; fileUnavailable = false; requestFiles = false; loading = false
            return
        }
        image = nil; media = nil; fileUnavailable = false; requestFiles = false; loading = false
        guard active, let client else { return }
        if item.hasImage {
            loading = true
            let cached = await ClipboardThumbnailCache.shared.thumbnail(for: item.id, client: client, maxPixelSize: 320)
            guard !Task.isCancelled else { return }
            loading = false
            image = cached
            return
        }
        guard item.fileCount > 0 else { return }
        do {
            let content = try await client.content(item.id)
            guard !Task.isCancelled else { return }
            let loaded = await ClipboardMediaLoader.shared.load(content, maxPixelSize: 320)
            guard !Task.isCancelled else { return }
            media = loaded
            image = loaded.thumbnail.flatMap { NSImage(data: $0) }
            requestFiles = content.canRequestFiles
            fileUnavailable = !requestFiles && (content.filePaths.count != Int(item.fileCount) || loaded.localPaths.count != Set(content.filePaths).count)
            availability(!fileUnavailable)
        } catch {
            guard !Task.isCancelled else { return }
            fileUnavailable = true; availability(false)
        }
    }
}

/// Grid card for the menu bar panel. Mirrors `MenuBarClipboardRow` behavior:
/// single click selects, double click pastes, and the context menu copies,
/// edits the title, manages lists, or deletes.
struct MenuBarClipboardCard: View {
    let item: HistoryItem
    let selected: Bool
    let client: EngineClient?
    let appID: String?
    let memberships: [String]
    let title: String?
    let index: Int
    let showShortcut: Bool
    let remoteUnavailable: Bool
    let ocrHit: Bool
    let presentation: UUID
    let active: Bool
    let availability: (Bool) -> Void
    let onSelect: () -> Void
    let onPaste: () -> Void
    let onCopy: () -> Void
    let onRename: (String) -> Void
    let onDelete: () -> Void
    @State private var image: NSImage?
    @State private var media: ClipboardMedia?
    @State private var fileUnavailable = false
    @State private var requestFiles = false
    @State private var loading = false
    @State private var editingTitle = false

    private static let maxContentHeight: CGFloat = 160

    var body: some View {
        ZStack(alignment: .topLeading) {
            RoundedRectangle(cornerRadius: 12)
                .fill(selected ? Color.accentColor.opacity(0.12) : Color(nsColor: .quaternaryLabelColor).opacity(0.22))
                .overlay(alignment: .bottomTrailing) { watermark }
                .clipShape(RoundedRectangle(cornerRadius: 12))
            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: 6) {
                    // Cards show only a real user title; empty stays empty.
                    ClipboardEditableTitle(
                        title: title,
                        fallback: "",
                        editing: $editingTitle,
                        onCommit: onRename
                    )
                    Spacer(minLength: 4)
                    if ocrHit {
                        Image(systemName: "text.viewfinder").font(.system(size: 11))
                            .foregroundStyle(.secondary).help("由图片识别文字匹配")
                    }
                    if remoteUnavailable {
                        Image(systemName: "arrow.down.circle").font(.system(size: 11))
                            .foregroundStyle(.secondary).help("使用时从来源设备获取")
                    }
                    if let date = item.copiedDate {
                        Text(Self.timeFormatter.string(from: date))
                            .font(.system(size: 11)).monospacedDigit().foregroundStyle(.secondary)
                    }
                }
                content
                if requestFiles {
                    Text("使用时从来源设备获取").font(.system(size: 11)).foregroundStyle(.secondary)
                }
                if fileUnavailable {
                    Label("文件不可用", systemImage: "exclamationmark.triangle")
                        .font(.system(size: 11)).foregroundStyle(.orange)
                }
                if index < 9 {
                    Text("⌘ \(index + 1)").font(.system(size: 11)).monospacedDigit()
                        .opacity(showShortcut ? 1 : 0).accessibilityHidden(!showShortcut)
                        .frame(maxWidth: .infinity, alignment: .trailing)
                }
            }
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .overlay {
            RoundedRectangle(cornerRadius: 12)
                .strokeBorder(selected ? Color.accentColor.opacity(0.7) : Color(nsColor: .separatorColor).opacity(0.6), lineWidth: 1)
        }
        .contentShape(RoundedRectangle(cornerRadius: 12))
        // A stacked single + double tap makes the single click wait out the
        // double-click interval, which reads as the card responding half a beat
        // late. The double tap is simultaneous instead, exactly like the list
        // rows, so the first click selects immediately.
        .onTapGesture(perform: onSelect)
        .simultaneousGesture(TapGesture(count: 2).onEnded { onSelect(); onPaste() })
        .contextMenu {
            Button("仅复制", action: onCopy).disabled(client == nil)
            Divider()
            Button("编辑标题") { editingTitle = true }
            Divider()
            ClipboardLibraryMembershipMenu(itemID: item.id, listIDs: memberships)
            Divider()
            Button("删除", role: .destructive, action: onDelete)
        }
        .accessibilityLabel("\(item.clipboardTitle)\(fileUnavailable ? "，文件不可用" : "")")
        .accessibilityAddTraits(selected ? [.isSelected] : [])
        .task(id: "\(item.id)-\(item.copiedAtMs ?? 0)-\(presentation)-\(active)") { await loadPreview() }
    }

    /// Bottom-right app icon, tilted 45° left, with roughly a quarter clipped off.
    @ViewBuilder private var watermark: some View {
        if let icon = ClipboardAppInfoCache.shared.icon(for: appID) {
            Image(nsImage: icon).resizable().scaledToFit()
                .frame(width: 88, height: 88)
                .rotationEffect(.degrees(-45))
                .offset(x: 22, y: 22)
                .opacity(0.05)
                .allowsHitTesting(false)
                .accessibilityHidden(true)
        }
    }

    @ViewBuilder private var content: some View {
        if item.hasImage {
            if let image {
                Image(nsImage: image).resizable().scaledToFit()
                    .frame(maxWidth: .infinity, maxHeight: Self.maxContentHeight)
                    .clipShape(RoundedRectangle(cornerRadius: 8))
            } else {
                RoundedRectangle(cornerRadius: 8)
                    .fill(Color(nsColor: .quaternaryLabelColor).opacity(0.25))
                    .frame(maxWidth: .infinity, minHeight: 96)
                    .overlay { if loading { ProgressView().controlSize(.small) } }
            }
        } else {
            Text(item.clipboardTitle)
                .font(.system(size: 13))
                .foregroundStyle(Color(nsColor: .labelColor))
                .lineLimit(8)
                .multilineTextAlignment(.leading)
                .frame(maxWidth: .infinity, alignment: .topLeading)
                .frame(maxHeight: Self.maxContentHeight, alignment: .top)
                .clipped()
        }
    }

    private static let timeFormatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.dateFormat = "HH:mm"
        return formatter
    }()

    @MainActor private func loadPreview() async {
        if item.hasImage, let cached = ClipboardThumbnailCache.shared.cached(item.id, maxPixelSize: 320) {
            image = cached; loading = false
            return
        }
        image = nil; media = nil; fileUnavailable = false; requestFiles = false; loading = false
        guard active, let client else { return }
        if item.hasImage {
            loading = true
            let cached = await ClipboardThumbnailCache.shared.thumbnail(for: item.id, client: client, maxPixelSize: 320)
            guard !Task.isCancelled else { return }
            loading = false
            image = cached
            return
        }
        guard item.fileCount > 0 else { return }
        do {
            let content = try await client.content(item.id)
            guard !Task.isCancelled else { return }
            let loaded = await ClipboardMediaLoader.shared.load(content, maxPixelSize: 320)
            guard !Task.isCancelled else { return }
            media = loaded
            image = loaded.thumbnail.flatMap { NSImage(data: $0) }
            requestFiles = content.canRequestFiles
            fileUnavailable = !requestFiles && (content.filePaths.count != Int(item.fileCount) || loaded.localPaths.count != Set(content.filePaths).count)
            availability(!fileUnavailable)
        } catch {
            guard !Task.isCancelled else { return }
            fileUnavailable = true; availability(false)
        }
    }
}
