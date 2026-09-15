import SwiftUI
import AppKit
import UniDeskEngine

enum ClipboardDisplayMode: String, CaseIterable, Identifiable {
    case list, grid
    var id: Self { self }
    var symbol: String { self == .list ? "list.bullet" : "square.grid.2x2" }
    var label: String { self == .list ? "列表" : "瀑布流" }
}

/// Two-icon list/grid switch shared by the main history pane and the menu bar panel.
struct ClipboardDisplayModePicker: View {
    @Binding var mode: ClipboardDisplayMode

    var body: some View {
        HStack(spacing: 2) {
            ForEach(ClipboardDisplayMode.allCases) { option in
                Button {
                    mode = option
                } label: {
                    Image(systemName: option.symbol)
                        .font(.system(size: 12, weight: .medium))
                        .frame(width: 26, height: 22)
                        .foregroundStyle(mode == option ? Color.accentColor : Color.secondary)
                        .background(mode == option ? Color.accentColor.opacity(0.16) : .clear, in: RoundedRectangle(cornerRadius: 6))
                }
                .buttonStyle(ClipboardHeaderButtonStyle())
                .help(option.label)
                .accessibilityLabel(option.label)
                .accessibilityAddTraits(mode == option ? [.isSelected] : [])
            }
        }
        .padding(2)
        .background(Color(nsColor: .quaternaryLabelColor).opacity(0.3), in: RoundedRectangle(cornerRadius: 8))
    }
}

/// Keeps a header control's size, weight, and position fixed while pressed, so
/// the glyph does not grow, bolden, or shift on click.
struct ClipboardHeaderButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
    }
}

/// Inline editable label for a clipboard record. Callers choose the fallback:
/// masonry cards show only a real title (empty otherwise), while rows fall back
/// to the derived content title.
struct ClipboardEditableTitle: View {
    let title: String?
    let fallback: String
    @Binding var editing: Bool
    let onCommit: (String) -> Void
    var font: Font = .system(size: 13, weight: .semibold)
    @State private var draft = ""
    @FocusState private var focused: Bool

    var body: some View {
        Group {
            if editing {
                TextField("", text: $draft)
                    .textFieldStyle(.plain)
                    .font(font)
                    .focused($focused)
                    .onSubmit(commit)
                    .onExitCommand { editing = false }
                    .onChange(of: focused) { _, value in if !value { commit() } }
            } else {
                Text(display)
                    .font(font)
                    .foregroundStyle(hasTitle ? Color.primary : Color.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .frame(minHeight: 16, alignment: .leading)
                    .contentShape(Rectangle())
                    .onTapGesture(count: 2) { editing = true }
                    .help(hasTitle ? title! : "双击编辑标题")
            }
        }
        .onChange(of: editing) { _, value in
            if value { draft = title ?? ""; focused = true }
        }
    }

    private var hasTitle: Bool { title?.isEmpty == false }
    private var display: String { hasTitle ? title! : fallback }

    private func commit() {
        guard editing else { return }
        editing = false
        let value = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        if value != (title ?? "") { onCommit(value) }
    }
}

/// Natural-height column grid shared by the main history pane and the menu bar
/// panel. Cards cap their own content height so one long text or tall image
/// cannot dominate a column.
struct ClipboardMasonryGrid<Card: View>: View {
    let items: [HistoryItem]
    let selected: UInt64?
    let columnCount: Int
    let hasMore: Bool
    let searching: Bool
    let loadMore: () -> Void
    @ViewBuilder let card: (HistoryItem, Bool) -> Card

    private var buckets: [[HistoryItem]] {
        let count = max(1, columnCount)
        var result = Array(repeating: [HistoryItem](), count: count)
        var heights = Array(repeating: CGFloat.zero, count: count)
        for item in items {
            let index = heights.indices.min { heights[$0] < heights[$1] } ?? 0
            result[index].append(item)
            heights[index] += Self.estimatedHeight(item) + 12
        }
        return result
    }

    var body: some View {
        // Balance once per body evaluation. `buckets` is a computed property, so
        // reading it per column re-ran the whole assignment for every column on
        // every selection change.
        let columns = buckets
        ScrollView {
            HStack(alignment: .top, spacing: 12) {
                ForEach(columns.indices, id: \.self) { index in
                    LazyVStack(spacing: 12) {
                        ForEach(columns[index], id: \.id) { item in
                            card(item, item.id == selected)
                                .id(item.id)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .top)
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            if hasMore {
                Button("加载更多", action: loadMore)
                    .disabled(searching)
                    .padding(.vertical, 12)
            }
        }
    }

    /// Rough column balancing only; real heights settle once cards render.
    private static func estimatedHeight(_ item: HistoryItem) -> CGFloat {
        if item.hasImage { return 236 }
        if item.fileCount > 0 { return 96 }
        let lines = min(8, max(1, item.preview.count / 36))
        return 70 + CGFloat(lines) * 18
    }
}

struct ClipboardHistoryCard: View {
    let item: HistoryItem
    let selected: Bool
    let selectedImage: NSImage?
    let client: EngineClient?
    let appID: String?
    let title: String?
    let meta: ClipboardItemMeta?
    let ocrHit: Bool
    let memberships: [String]
    let onRename: (String) -> Void
    let onSelect: () -> Void
    let onDelete: () -> Void
    @State private var thumbnail: NSImage?
    @State private var loading = false
    @State private var editingTitle = false

    /// Keeps a single card from growing with a long text or a tall image.
    private static let maxContentHeight: CGFloat = 236

    var body: some View {
        ZStack(alignment: .topLeading) {
            RoundedRectangle(cornerRadius: 12)
                .fill(selected ? Color.accentColor.opacity(0.12) : Color(nsColor: .quaternaryLabelColor).opacity(0.22))
                .overlay(alignment: .bottomTrailing) { watermark }
                .clipShape(RoundedRectangle(cornerRadius: 12))
            VStack(alignment: .leading, spacing: 8) {
                header
                content
            }
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .overlay {
            RoundedRectangle(cornerRadius: 12)
                .strokeBorder(selected ? Color.accentColor.opacity(0.7) : Color(nsColor: .separatorColor).opacity(0.6), lineWidth: 1)
        }
        .contentShape(RoundedRectangle(cornerRadius: 12))
        .onTapGesture(perform: onSelect)
        .contextMenu {
            if let date = item.copiedDate {
                Text("复制于 \(date.formatted(date: .abbreviated, time: .standard))")
            }
            Divider()
            Button("编辑标题") { editingTitle = true }
            Divider()
            ClipboardLibraryMembershipMenu(itemID: item.id, listIDs: memberships)
            Divider()
            Button("删除", role: .destructive, action: onDelete)
        }
        .help(item.preview)
        .task(id: "\(item.id)-\(item.copiedAtMs ?? 0)") {
            guard item.hasImage, let client else { return }
            if let cached = ClipboardThumbnailCache.shared.cached(item.id, maxPixelSize: 480) { thumbnail = cached; return }
            loading = true
            let image = await ClipboardThumbnailCache.shared.thumbnail(for: item.id, client: client, maxPixelSize: 480)
            guard !Task.isCancelled else { return }
            loading = false
            thumbnail = image
        }
    }

    /// Bottom-right app icon, tilted 45° left, with roughly a quarter clipped off.
    @ViewBuilder private var watermark: some View {
        if let icon = ClipboardAppInfoCache.shared.icon(for: appID) {
            Image(nsImage: icon).resizable().scaledToFit()
                .frame(width: 96, height: 96)
                .rotationEffect(.degrees(-45))
                .offset(x: 24, y: 24)
                .opacity(0.05)
                .allowsHitTesting(false)
                .accessibilityHidden(true)
        }
    }

    private var header: some View {
        HStack(spacing: 6) {
            // Cards show only a real user title; empty stays empty.
            ClipboardEditableTitle(
                title: title,
                fallback: "",
                editing: $editingTitle,
                onCommit: onRename
            )
            Spacer(minLength: 4)
            HStack(spacing: 4) {
                if ocrHit {
                    Image(systemName: "text.viewfinder")
                        .font(.system(size: 11)).foregroundStyle(.secondary)
                        .help("由图片识别文字匹配")
                }
                if meta?.remote == true && meta?.available == false {
                    Image(systemName: "arrow.down.circle")
                        .font(.system(size: 11)).foregroundStyle(.secondary)
                        .help("使用时从来源设备获取")
                }
            }
            if let date = item.copiedDate {
                Text(Self.timeFormatter.string(from: date))
                    .font(.system(size: 11)).monospacedDigit().foregroundStyle(.secondary)
            }
        }
    }

    @ViewBuilder private var content: some View {
        if item.hasImage {
            if let image = thumbnail ?? selectedImage {
                Image(nsImage: image).resizable().scaledToFit()
                    .frame(maxWidth: .infinity, maxHeight: Self.maxContentHeight)
                    .clipShape(RoundedRectangle(cornerRadius: 8))
            } else if loading {
                ProgressView().controlSize(.small)
                    .frame(maxWidth: .infinity, minHeight: 80)
            } else {
                fallbackTitle
            }
        } else {
            Text(item.clipboardTitle)
                .font(.system(size: 13))
                .foregroundStyle(Color(nsColor: .labelColor))
                .lineLimit(10)
                .multilineTextAlignment(.leading)
                .frame(maxWidth: .infinity, alignment: .topLeading)
                .frame(maxHeight: Self.maxContentHeight, alignment: .top)
                .clipped()
        }
    }

    private var fallbackTitle: some View {
        Text(item.clipboardTitle)
            .font(.system(size: 13))
            .frame(maxWidth: .infinity, alignment: .leading)
    }

    private static let timeFormatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.dateFormat = "HH:mm"
        return formatter
    }()
}
