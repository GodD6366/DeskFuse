import SwiftUI
import AppKit
import UniDeskEngine

struct MenuBarClipboardRow: View {
    let item: HistoryItem
    let index: Int
    let selected: Bool
    let client: EngineClient?
    let presentation: UUID
    let active: Bool
    let availability: (Bool) -> Void
    let select: () -> Void
    @State private var image: NSImage?
    @State private var media: ClipboardMedia?
    @State private var fileUnavailable = false
    @State private var loading = false
    private var loadKey: String { "\(item.id)-\(item.copiedAtMs ?? 0)-\(presentation)-\(active)" }

    var body: some View {
        Button(action: select) {
            HStack(spacing: 12) {
                preview
                VStack(alignment: .leading, spacing: 6) {
                    Text(item.clipboardTitle)
                        .font(.system(size: 13, weight: .medium))
                        .lineLimit(item.hasImage || item.fileCount > 0 ? 1 : 2)
                        .multilineTextAlignment(.leading)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    Text(metadata).font(.system(size: 11)).foregroundStyle(.secondary).lineLimit(1)
                    if fileUnavailable {
                        Label("文件不可用", systemImage: "exclamationmark.triangle")
                            .font(.system(size: 11)).foregroundStyle(.orange)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                VStack(alignment: .trailing, spacing: 15) {
                    TimelineView(.periodic(from: .now, by: 30)) { context in
                        Text(timeLabel(now: context.date)).font(.system(size: 11)).foregroundStyle(.secondary)
                    }
                    if index < 9 { Text("⌘ \(index + 1)").font(.system(size: 12)).monospacedDigit() }
                }
                .fixedSize(horizontal: true, vertical: false)
            }
            .foregroundStyle(.primary)
            .padding(.horizontal, 10).padding(.vertical, 10)
            .frame(minHeight: item.hasImage ? 96 : 65)
            .background(selected ? Color.accentColor.opacity(0.12) : .clear, in: RoundedRectangle(cornerRadius: 10))
            .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(selected ? Color.accentColor.opacity(0.7) : .clear, lineWidth: 1))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel("\(item.clipboardTitle)，\(metadata)\(fileUnavailable ? "，文件不可用" : "")")
        .accessibilityAddTraits(selected ? [.isSelected] : [])
        .task(id: loadKey) { await loadPreview() }
    }

    @ViewBuilder private var preview: some View {
        if let image {
            Image(nsImage: image).resizable().aspectRatio(contentMode: .fit)
                .frame(width: item.hasImage ? 108 : 44, height: item.hasImage ? 72 : 44)
                .clipShape(RoundedRectangle(cornerRadius: 6))
        } else {
            ZStack {
                RoundedRectangle(cornerRadius: 8).fill(Color(nsColor: .controlBackgroundColor))
                if loading && item.hasImage { ProgressView().controlSize(.small) }
                else {
                    Image(systemName: media?.firstFileIsDirectory == true ? "folder.fill" : item.clipboardSymbol)
                        .font(.system(size: item.hasImage ? 25 : 23, weight: .regular))
                        .foregroundStyle(media?.firstFileIsDirectory == true ? Color.accentColor : Color.secondary)
                }
            }
            .frame(width: item.hasImage ? 108 : 44, height: item.hasImage ? 72 : 44)
        }
    }

    private var metadata: String {
        let source = item.source.isEmpty ? "本机" : item.source
        if item.fileCount > 0 {
            // HistoryItem.length is a reference count for files, not their bytes.
            return "\(source) · \(media?.firstFileIsDirectory == true && item.fileCount == 1 ? "文件夹" : "\(item.fileCount) 个文件")"
        }
        return "\(source) · \(item.hasImage ? byteCount(item.length) : "文字")"
    }

    private func timeLabel(now: Date) -> String {
        guard let timestamp = item.copiedAtMs else { return "" }
        let date = Date(timeIntervalSince1970: Double(timestamp) / 1000)
        let elapsed = now.timeIntervalSince(date)
        if elapsed < -60 { return date.formatted(date: .abbreviated, time: .shortened) }
        if elapsed < 60 { return "刚刚" }
        if elapsed < 3600 { return "\(Int(elapsed / 60)) 分钟前" }
        if elapsed < 86400 { return "\(Int(elapsed / 3600)) 小时前" }
        return date.formatted(.dateTime.month().day())
    }

    @MainActor private func loadPreview() async {
        image = nil; media = nil; fileUnavailable = false; loading = false
        guard active, item.hasImage || item.fileCount > 0, let client else { return }
        loading = true
        defer { if !Task.isCancelled { loading = false } }
        do {
            let content = try await client.content(item.id)
            guard !Task.isCancelled else { return }
            let loaded = await ClipboardMediaLoader.shared.load(content, maxPixelSize: 320)
            guard !Task.isCancelled else { return }
            media = loaded
            image = loaded.thumbnail.flatMap { NSImage(data: $0) }
            if item.fileCount > 0 {
                fileUnavailable = content.filePaths.count != Int(item.fileCount) || loaded.localPaths.count != Set(content.filePaths).count
                availability(!fileUnavailable)
            }
        } catch {
            guard !Task.isCancelled else { return }
            if item.fileCount > 0 { fileUnavailable = true; availability(false) }
        }
    }
}
