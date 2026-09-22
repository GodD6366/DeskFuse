import SwiftUI
import AppKit
import DeskFuseEngine

struct MenuBarClipboardRow: View {
    let item: HistoryItem
    let index: Int
    let showShortcuts: Bool
    let selected: Bool
    let client: EngineClient?
    let appID: String?
    let title: String?
    let meta: ClipboardItemMeta?
    let memberships: [String]
    let remoteUnavailable: Bool
    let ocrHit: Bool
    let presentation: UUID
    let active: Bool
    let availability: (Bool) -> Void
    let select: () -> Void
    let onCopy: () -> Void
    var onPaste: (() -> Void)? = nil
    var showsPreview = false
    var onPreview: (() -> Void)? = nil
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
                // Middle: the content itself; text and images are both centered.
                VStack(spacing: 6) {
                    content
                    if requestFiles {
                        Text("使用时从来源设备获取").font(.system(size: 11)).foregroundStyle(.secondary)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .center)
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
        .accessibilityLabel(item.clipboardTitle)
        .accessibilityAddTraits(selected ? [.isSelected] : [])
        .clipboardHistoryContextMenu(ClipboardHistoryMenuContext(
            item: item, title: title, memberships: memberships, meta: meta,
            copyEnabled: client != nil,
            pasteTargetName: ClipboardPanelController.shared.pasteTargetName,
            onCopy: onCopy, onPaste: onPaste,
            onEditTitle: { editingTitle = true },
            showsPreview: showsPreview, onPreview: onPreview, onDelete: onDelete))
        .sheet(isPresented: $editingTitle) {
            ClipboardTitleEditor(title: title, onSave: onRename)
        }
        .task(id: loadKey) { await loadPreview() }
        .onDisappear { image = nil; media = nil; loading = false }
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
                VStack(spacing: 5) {
                    Image(nsImage: image).resizable().aspectRatio(contentMode: .fit)
                        .frame(width: 108, height: 72)
                        .clipShape(RoundedRectangle(cornerRadius: 6))
                    if title?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false { titleLabel }
                }
                .frame(maxWidth: .infinity, alignment: .center)
            } else {
                // Keep a stable slot while the thumbnail loads so rows never jump.
                RoundedRectangle(cornerRadius: 6)
                    .fill(Color(nsColor: .quaternaryLabelColor).opacity(0.25))
                    .frame(width: 108, height: 72)
                    .overlay { if loading { ProgressView().controlSize(.small) } }
                    .frame(maxWidth: .infinity, alignment: .center)
            }
        } else {
            titleLabel
        }
    }

    private var titleLabel: some View {
        ClipboardTitleLabel(
            title: title,
            fallback: item.clipboardTitle,
            editing: $editingTitle,
            editsOnDoubleClick: false
        )
        .multilineTextAlignment(.center)
        .frame(maxWidth: .infinity, alignment: .center)
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
    let meta: ClipboardItemMeta?
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
    var showsPreview = false
    var onPreview: (() -> Void)? = nil
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
            VStack(spacing: 0) {
                if hasTitle {
                    ClipboardCardHeader(title: title, selected: selected, editing: $editingTitle)
                }
                VStack(alignment: .leading, spacing: 10) {
                    content
                    if requestFiles {
                        Text("使用时从来源设备获取").font(.system(size: 11)).foregroundStyle(.secondary)
                    }
                    if hasMetadata || index < 9 {
                        metadata
                            .frame(maxWidth: .infinity)
                            .overlay(alignment: .trailing) {
                                if index < 9 {
                                    Text("⌘ \(index + 1)").font(.system(size: 11)).monospacedDigit()
                                        .opacity(showShortcut ? 1 : 0).accessibilityHidden(!showShortcut)
                                }
                            }
                    }
                }
                .padding(10)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .clipShape(RoundedRectangle(cornerRadius: 12))
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
        .clipboardHistoryContextMenu(ClipboardHistoryMenuContext(
            item: item, title: title, memberships: memberships, meta: meta,
            copyEnabled: client != nil,
            pasteTargetName: ClipboardPanelController.shared.pasteTargetName,
            onCopy: onCopy, onPaste: onPaste,
            onEditTitle: { editingTitle = true },
            showsPreview: showsPreview, onPreview: onPreview, onDelete: onDelete))
        .sheet(isPresented: $editingTitle) {
            ClipboardTitleEditor(title: title, onSave: onRename)
        }
        .accessibilityLabel(item.clipboardTitle)
        .accessibilityAddTraits(selected ? [.isSelected] : [])
        .task(id: "\(item.id)-\(item.copiedAtMs ?? 0)-\(presentation)-\(active)") { await loadPreview() }
        .onDisappear { image = nil; media = nil; loading = false }
    }

    private var hasTitle: Bool { title?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false }
    private var hasMetadata: Bool { item.copiedDate != nil || ocrHit || remoteUnavailable }

    private var metadata: some View {
        ZStack {
            if let date = item.copiedDate {
                Text(Self.timeFormatter.string(from: date))
                    .font(.system(size: 11)).monospacedDigit().foregroundStyle(.secondary)
            }
            HStack(spacing: 4) {
                if ocrHit {
                    Image(systemName: "text.viewfinder")
                        .font(.system(size: 11)).foregroundStyle(.secondary)
                        .help("由图片识别文字匹配")
                }
                if remoteUnavailable {
                    Image(systemName: "arrow.down.circle")
                        .font(.system(size: 11)).foregroundStyle(.secondary)
                        .help("使用时从来源设备获取")
                }
                Spacer(minLength: 0)
            }
        }
        .frame(maxWidth: .infinity)
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
            ClipboardCardImage(
                image: image,
                loading: loading,
                maxHeight: Self.maxContentHeight,
                unavailable: remoteUnavailable ? "待获取图片" : "图片不可用"
            )
        } else {
            ClipboardCardText(text: item.clipboardTitle, lineLimit: 8, maxHeight: Self.maxContentHeight)
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
