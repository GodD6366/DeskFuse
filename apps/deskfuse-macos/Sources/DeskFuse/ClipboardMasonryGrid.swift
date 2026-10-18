import SwiftUI
import AppKit
import DeskFuseEngine

enum ClipboardDisplayMode: String, CaseIterable, Identifiable {
    case list, grid
    var id: Self { self }
    var symbol: String { self == .list ? "list.bullet" : "square.grid.2x2" }
    var label: String { self == .list ? "列表" : "瀑布流" }
}

/// Two-icon list/grid switch shared by the main history pane and the menu bar panel.
struct ClipboardDisplayModePicker: View {
    @Environment(\.quietWorkspace) private var quiet
    @Environment(\.colorScheme) private var scheme
    @Binding var mode: ClipboardDisplayMode

    var body: some View {
        HStack(spacing: 2) {
            ForEach(ClipboardDisplayMode.allCases) { option in
                Button {
                    mode = option
                } label: {
                    Image(systemName: option.symbol)
                        .font(.system(size: 12, weight: .medium))
                        .frame(width: 34, height: 30)
                        .contentShape(Rectangle())
                        .foregroundStyle(mode == option ? (quiet ? Color.primary : Color.accentColor) : Color.secondary)
                        .background(mode == option ? (quiet ? QuietWorkspacePalette.surface(scheme) : Color.accentColor.opacity(0.16)) : .clear,
                                    in: RoundedRectangle(cornerRadius: 6))
                }
                .buttonStyle(ClipboardHeaderButtonStyle())
                .help(option.label)
                .accessibilityLabel(option.label)
                .accessibilityAddTraits(mode == option ? [.isSelected] : [])
            }
        }
        .padding(2)
        .modifier(NativeGlassBackdrop(cornerRadius: 9))
    }
}

/// Keeps a header control's size, weight, and position fixed while pressed, so
/// the glyph does not grow, bolden, or shift on click.
struct ClipboardHeaderButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
    }
}

/// Natural-height column grid shared by the main history pane and the menu bar
/// panel.
enum ClipboardMasonryBlock {
    case columns([[HistoryItem]])
    case fullWidth(HistoryItem)
}

enum ClipboardMasonryLayout {
    static func blocks(for items: [HistoryItem], columnCount: Int) -> [ClipboardMasonryBlock] {
        let count = max(1, columnCount)
        var result: [ClipboardMasonryBlock] = []
        var columns = Array(repeating: [HistoryItem](), count: count)
        var heights = Array(repeating: CGFloat.zero, count: count)

        func flushColumns() {
            guard columns.contains(where: { !$0.isEmpty }) else { return }
            result.append(.columns(columns))
            columns = Array(repeating: [HistoryItem](), count: count)
            heights = Array(repeating: CGFloat.zero, count: count)
        }

        for item in items {
            let index = heights.indices.min { heights[$0] < heights[$1] } ?? 0
            columns[index].append(item)
            heights[index] += estimatedHeight(item) + 12
        }
        flushColumns()
        return result
    }

    static func estimatedHeight(_ item: HistoryItem) -> CGFloat {
        if item.hasImage { return 128 }
        if item.fileCount > 0 { return 96 }
        let lines = min(4, max(1, item.preview.count / 36))
        return 70 + CGFloat(lines) * 18
    }
}

private struct ClipboardMasonryPosition {
    let id: UInt64
    let blockIndex: Int
    let column: Int?
    let centerY: CGFloat
}

enum ClipboardMasonryNavigation {
    enum Direction: Equatable {
        case left, right, up, down
    }

    static func target(items: [HistoryItem], selected: UInt64?, columnCount: Int, direction: Direction) -> UInt64? {
        let allPositions = positions(items: items, columnCount: columnCount)
        guard let selected, let current = allPositions.first(where: { $0.id == selected }) else {
            return nil
        }
        let layout = ClipboardMasonryLayout.blocks(for: items, columnCount: columnCount)

        switch direction {
        case .left, .right:
            guard let currentColumn = current.column,
                  layout.indices.contains(current.blockIndex),
                  case .columns(let columns) = layout[current.blockIndex] else { return nil }
            let targetColumn = direction == .left ? currentColumn - 1 : currentColumn + 1
            guard columns.indices.contains(targetColumn) else { return nil }
            return allPositions
                .filter { $0.blockIndex == current.blockIndex && $0.column == targetColumn }
                .min { abs($0.centerY - current.centerY) < abs($1.centerY - current.centerY) }?.id
        case .up, .down:
            let sameColumn = allPositions.filter { position in
                position.blockIndex == current.blockIndex && position.column == current.column
            }
            let inDirection: (ClipboardMasonryPosition) -> Bool = { position in
                direction == .up ? position.centerY < current.centerY : position.centerY > current.centerY
            }
            if let nearby = sameColumn.filter(inDirection).min(by: { verticalDistance($0, from: current) < verticalDistance($1, from: current) }) {
                return nearby.id
            }
            return allPositions
                .filter { $0.blockIndex != current.blockIndex && inDirection($0) }
                .min {
                    let left = verticalDistance($0, from: current)
                    let right = verticalDistance($1, from: current)
                    return left == right ? columnDistance($0, from: current) < columnDistance($1, from: current) : left < right
                }?.id
        }
    }

    private static func positions(items: [HistoryItem], columnCount: Int) -> [ClipboardMasonryPosition] {
        let blocks = ClipboardMasonryLayout.blocks(for: items, columnCount: columnCount)
        var result: [ClipboardMasonryPosition] = []
        var baseY: CGFloat = 0

        for (blockIndex, block) in blocks.enumerated() {
            switch block {
            case .columns(let columns):
                var columnHeights: [CGFloat] = []
                for (columnIndex, column) in columns.enumerated() {
                    var currentY = baseY
                    for item in column {
                        let height = ClipboardMasonryLayout.estimatedHeight(item)
                        result.append(ClipboardMasonryPosition(id: item.id, blockIndex: blockIndex, column: columnIndex, centerY: currentY + height / 2))
                        currentY += height + 12
                    }
                    columnHeights.append(currentY > baseY ? currentY - 12 - baseY : 0)
                }
                baseY += columnHeights.max() ?? 0
                baseY += 12
            case .fullWidth(let item):
                let height = ClipboardMasonryLayout.estimatedHeight(item)
                result.append(ClipboardMasonryPosition(id: item.id, blockIndex: blockIndex, column: nil, centerY: baseY + height / 2))
                baseY += height + 12
            }
        }
        return result
    }

    private static func verticalDistance(_ position: ClipboardMasonryPosition, from current: ClipboardMasonryPosition) -> CGFloat {
        abs(position.centerY - current.centerY)
    }

    private static func columnDistance(_ position: ClipboardMasonryPosition, from current: ClipboardMasonryPosition) -> Int {
        abs((position.column ?? 0) - (current.column ?? 0))
    }
}

struct ClipboardMasonryGrid<Card: View>: View {
    let items: [HistoryItem]
    let selected: UInt64?
    let columnCount: Int
    let hasMore: Bool
    let paginationOffset: Int
    let paginationGeneration: UUID
    let searching: Bool
    let loadMore: () -> Void
    @ViewBuilder let card: (HistoryItem, Bool) -> Card

    private var blocks: [ClipboardMasonryBlock] {
        ClipboardMasonryLayout.blocks(for: items, columnCount: columnCount)
    }

    var body: some View {
        let layout = blocks
        ScrollView {
            LazyVStack(spacing: 12) {
                ForEach(Array(layout.enumerated()), id: \.offset) { _, block in
                    switch block {
                    case .columns(let columns):
                        HStack(alignment: .top, spacing: 12) {
                            ForEach(columns.indices, id: \.self) { index in
                                LazyVStack(spacing: 12) {
                                    ForEach(columns[index], id: \.id) { item in
                                        card(item, item.id == selected)
                                            .id(item.id)
                                            .transition(.opacity)
                                    }
                                }
                                .frame(maxWidth: .infinity, alignment: .top)
                            }
                        }
                    case .fullWidth(let item):
                        card(item, item.id == selected)
                            .id(item.id)
                            .transition(.opacity)
                    }
                }
                if hasMore {
                    ClipboardPaginationTrigger(itemCount: paginationOffset, searching: searching, loadMore: loadMore)
                        .id(paginationGeneration)
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
        }
        .scrollIndicators(.hidden)
        /// 新条目插入时淡入；不带动效的列重排（move）会让两列抖动，瀑布流只用 opacity。
        .animation(Motion.snappy, value: items.map(\.id))
    }
}

/// 卡片内容类型，决定卡片内部的排版分支。
enum ClipboardCardKind {
    case link, code, image, file, text

    static func kind(for item: HistoryItem) -> ClipboardCardKind {
        if item.hasImage { return .image }
        if item.fileCount > 0 { return .file }
        let text = item.preview.trimmingCharacters(in: .whitespacesAndNewlines)
        if linkURL(in: text) != nil { return .link }
        if looksLikeCode(text) { return .code }
        return .text
    }

    static func linkURL(in text: String) -> URL? {
        guard !text.contains(where: \.isWhitespace),
              let url = URL(string: text),
              let scheme = url.scheme?.lowercased(), ["http", "https"].contains(scheme),
              let host = url.host, !host.isEmpty else { return nil }
        return url
    }

    /// 保守的代码/JSON 启发式：任何一条都不命中就当纯文本，宁漏判勿误判。
    static func looksLikeCode(_ text: String) -> Bool {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.count >= 24 else { return false }
        let lines = trimmed.components(separatedBy: "\n")
        guard lines.count >= 2 else { return false }
        if (trimmed.hasPrefix("{") || trimmed.hasPrefix("[")) && trimmed.contains(":") { return true }
        let first = lines.first?.trimmingCharacters(in: .whitespaces) ?? ""
        let keywords = ["func ", "def ", "import ", "class ", "struct ", "enum ", "return ", "const ",
                        "let ", "var ", "function ", "#include", "public ", "private ", "if let ", "for (", "await "]
        if keywords.contains(where: { first.hasPrefix($0) }) { return true }
        let keyValueLines = lines.filter { line in
            let line = line.trimmingCharacters(in: .whitespaces)
            return line.hasPrefix("\"") && line.contains("\":")
        }.count
        if keyValueLines >= 2 { return true }
        let indented = lines.filter { $0.hasPrefix("    ") || $0.hasPrefix("\t") }.count
        return indented >= 2 && trimmed.contains("=")
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
    @State private var hovering = false
    @State private var hoveringActions = false
    @State private var showingImagePreview = false
    @State private var hoverImage: NSImage?
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency

    /// 图片卡片只限制最大高度，横竖图都按自身比例展示。
    private static let maxImageHeight: CGFloat = 200

    var body: some View {
        let kind = ClipboardCardKind.kind(for: item)
        ZStack(alignment: .topLeading) {
            RoundedRectangle(cornerRadius: DeskFuseBrand.cardRadius, style: .continuous)
                .fill(selected ? DeskFuseBrand.selectionFill : Color(nsColor: .controlBackgroundColor))

            VStack(spacing: 0) {
                if hasTitle {
                    ClipboardCardHeader(title: title, selected: selected, editing: $editingTitle)
                }
                VStack(alignment: .leading, spacing: 6) {
                    sourceAppBadge
                    cardContent(kind)
                    footer
                }
                .padding(10)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .clipShape(RoundedRectangle(cornerRadius: DeskFuseBrand.cardRadius, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: DeskFuseBrand.cardRadius, style: .continuous)
                .strokeBorder(
                    selected ? DeskFuseBrand.accent : DeskFuseBrand.cardBorder,
                    lineWidth: selected ? DeskFuseBrand.selectionStrokeWidth : DeskFuseBrand.cardRestStrokeWidth
                )
        }
        .shadow(
            color: hovering ? DeskFuseBrand.cardHoverShadowColor : DeskFuseBrand.cardShadowColor,
            radius: hovering ? DeskFuseBrand.cardHoverShadowRadius : DeskFuseBrand.cardShadowRadius,
            y: hovering ? DeskFuseBrand.cardHoverShadowY : DeskFuseBrand.cardShadowY
        )
        .offset(y: hovering ? -1 : 0)
        .animation(Motion.snappy, value: selected)
        .contentShape(RoundedRectangle(cornerRadius: DeskFuseBrand.cardRadius, style: .continuous))
        .onTapGesture(perform: onSelect)
        .onHover { over in withMotion(.snappy) { hovering = over } }
        .clipboardHistoryContextMenu(ClipboardHistoryMenuContext(
            item: item, title: title, memberships: memberships, meta: meta,
            copyEnabled: client != nil,
            onEditTitle: { editingTitle = true }, onDelete: onDelete))
        /// 浮层放在右键菜单层之上，保证按钮是全卡最顶层的可命中元素。
        .overlay(alignment: .topTrailing) { hoverActions }
        .sheet(isPresented: $editingTitle) {
            ClipboardTitleEditor(title: title, onSave: onRename)
        }
        .help(item.preview)
        .popover(isPresented: $showingImagePreview, arrowEdge: .trailing) {
            if let image = hoverImage ?? thumbnail ?? selectedImage {
                Image(nsImage: image).resizable().scaledToFit()
                    .frame(maxWidth: 600, maxHeight: 440).padding(12)
                    .accessibilityLabel("图片放大预览")
            }
        }
        .task(id: hovering) {
            guard hovering, item.hasImage else {
                showingImagePreview = false; hoverImage = nil; return
            }
            do { try await Task.sleep(for: .milliseconds(500)) } catch { return }
            guard !Task.isCancelled else { return }
            if let client {
                hoverImage = await ClipboardThumbnailCache.shared.thumbnail(for: item.id, client: client, maxPixelSize: 1400)
            }
            guard !Task.isCancelled else { return }
            showingImagePreview = hoverImage != nil || thumbnail != nil || selectedImage != nil
        }
        .task(id: "\(item.id)-\(item.copiedAtMs ?? 0)") {
            guard item.hasImage, let client else { return }
            if let cached = ClipboardThumbnailCache.shared.cached(item.id, maxPixelSize: 480) { thumbnail = cached; return }
            loading = true
            let image = await ClipboardThumbnailCache.shared.thumbnail(for: item.id, client: client, maxPixelSize: 480)
            guard !Task.isCancelled else { return }
            loading = false
            thumbnail = image
        }
        .onDisappear { thumbnail = nil; loading = false }
    }

    @ViewBuilder private var sourceAppBadge: some View {
        if let appID, !appID.isEmpty {
            HStack(spacing: 6) {
                if let icon = ClipboardAppInfoCache.shared.icon(for: appID) {
                    Image(nsImage: icon).resizable().scaledToFit().frame(width: 18, height: 18)
                } else {
                    Image(systemName: "app").frame(width: 18, height: 18)
                }
                Text(ClipboardAppInfoCache.shared.name(for: appID) ?? appID)
                    .font(.caption).lineLimit(1).workspaceSecondaryText()
            }
            .help("来源应用：\(ClipboardAppInfoCache.shared.name(for: appID) ?? appID)")
            .padding(.bottom, 2)
        }
    }

    /// 悬停时右上角浮现的快捷操作。容器不施加任何 hit-test 修改：SwiftUI
    /// 布局容器只在其内容（按钮）处接收点击，padding 与间距留白自动穿透到
    /// 卡片的点击选中与拖拽框选。hoveringActions 保证指针移到 glass 按钮上
    /// 时浮层不闪失；浮层位于右键菜单 overlay 之上，按钮是全卡最顶层的命中目标。
    @ViewBuilder private var hoverActions: some View {
        if hovering || hoveringActions {
            HStack(spacing: 6) {
                ClipboardCardActionButton(systemImage: "doc.on.doc", help: "复制") {
                    ClipboardItemActions.copy(item.id, format: .original, paste: false)
                }
                .disabled(client == nil)
                ClipboardCardActionButton(systemImage: "trash", help: "删除", action: onDelete)
            }
            .padding(6)
            .transition(.opacity)
            .onHover { over in withMotion(.snappy) { hoveringActions = over } }
        }
    }

    private var hasTitle: Bool { title?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false }

    @ViewBuilder private func cardContent(_ kind: ClipboardCardKind) -> some View {
        switch kind {
        case .image:
            ClipboardCardImage(
                image: thumbnail ?? selectedImage,
                loading: loading,
                maxHeight: Self.maxImageHeight,
                unavailable: meta?.remote == true && meta?.available == false ? "待获取图片" : "图片不可用"
            )
        case .file:
            fileContent
        case .link:
            linkContent
        case .code:
            codeContent
        case .text:
            textContent
        }
    }

    private var linkParts: (host: String, path: String)? {
        let text = item.preview.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let url = ClipboardCardKind.linkURL(in: text), let host = url.host else { return nil }
        let path = url.path.removingPercentEncoding ?? url.path
        return (host, path == "/" ? "" : path)
    }

    /// 链接卡片：链接图标 + 域名/路径两行，正文省略。
    private var linkContent: some View {
        HStack(spacing: 8) {
            Image(systemName: "link")
                .font(.system(size: 13, weight: .medium))
                .workspaceSecondaryText()
                .frame(width: 24, height: 24)
            VStack(alignment: .leading, spacing: 2) {
                Text(linkParts?.host ?? item.clipboardTitle)
                    .font(.system(size: 13, weight: .semibold))
                    .lineLimit(1)
                if let path = linkParts?.path, !path.isEmpty {
                    Text(path)
                        .font(.system(size: 11))
                        .workspaceSecondaryText()
                        .lineLimit(1)
                }
            }
        }
    }

    /// 代码/JSON 卡片：等宽字体，底板比背景深一档。
    private var codeContent: some View {
        Text(item.clipboardTitle)
            .font(.system(size: 12.5, design: .monospaced))
            .lineSpacing(2)
            .lineLimit(4).truncationMode(.tail)
            .multilineTextAlignment(.leading)
            .padding(8)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Color.primary.opacity(codeFillOpacity), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
    }

    private var codeFillOpacity: Double {
        if reduceTransparency { return colorScheme == .dark ? 0.08 : 0.06 }
        return colorScheme == .dark ? 0.06 : 0.04
    }

    /// 文件卡片：文件图标 + 文件名 + 大小的紧凑行。
    private var fileContent: some View {
        HStack(spacing: 8) {
            Image(systemName: "doc.fill")
                .font(.system(size: 16))
                .foregroundStyle(DeskFuseBrand.fusionBlue)
                .frame(width: 24, height: 24)
            VStack(alignment: .leading, spacing: 2) {
                Text(item.clipboardTitle)
                    .font(.system(size: 13, weight: .medium))
                    .lineLimit(1)
                Text(item.fileCount > 1 ? "\(item.fileCount) 项 · \(byteCount(item.length))" : byteCount(item.length))
                    .font(.system(size: 11))
                    .workspaceSecondaryText()
            }
        }
    }

    /// Four readable lines with a consistent trailing ellipsis.
    private var textContent: some View {
        Text(item.clipboardTitle)
            .font(.system(size: 13))
            .lineSpacing(3)
            .lineLimit(4).truncationMode(.tail)
            .multilineTextAlignment(.leading)
            .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// 卡片底部固定 footer：时间 + 来源设备，右侧保留状态图标。
    private var footer: some View {
        HStack(spacing: 6) {
            if let date = item.copiedDate {
                Text(Self.timeFormatter.string(from: date))
                    .monospacedDigit()
            }
            Text(meta?.originDevice ?? item.source)
                .lineLimit(1)
            Spacer(minLength: 0)
            if ocrHit {
                Image(systemName: "text.viewfinder")
                    .help("由图片识别文字匹配")
            }
            if meta?.remote == true && meta?.available == false {
                Image(systemName: "arrow.down.circle")
                    .help("使用时从来源设备获取")
            }
        }
        .font(.system(size: 10.5))
        .workspaceSecondaryText()
    }

    private static let timeFormatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.dateFormat = "HH:mm"
        return formatter
    }()
}

/// 悬停浮层的圆钮：macOS 26 用 glass，旧版本与减弱透明度时用实色底。
private struct ClipboardCardActionButton: View {
    let systemImage: String
    let help: String
    let action: () -> Void
    @State private var copied = false
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    @Environment(\.isEnabled) private var enabled

    var body: some View {
        Button(action: perform) {
            Image(systemName: copied ? "checkmark" : systemImage)
                .font(.system(size: 11, weight: .semibold))
                .frame(width: 24, height: 24)
                .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .foregroundStyle(copied ? DeskFuseBrand.accent : (enabled ? Color.primary : Color.secondary))
        .modifier(NativeGlassBackdrop(cornerRadius: 12))
        .help(help)
        .accessibilityLabel(help)
    }

    /// 复制是即发即弃的请求；按钮本地给 1.2 秒对勾反馈并触发触感。
    private func perform() {
        action()
        guard !copied else { return }
        copied = true
        NSHapticFeedbackManager.defaultPerformer.perform(.generic, performanceTime: .default)
        Task { @MainActor in
            try? await Task.sleep(for: .seconds(1.2))
            guard !Task.isCancelled else { return }
            withMotion(.snappy) { copied = false }
        }
    }
}

/// 按图片自身比例占位，只限制最大高度，避免横图被塞进竖向空框。
struct ClipboardCardImage: View {
    let image: NSImage?
    let loading: Bool
    let maxHeight: CGFloat
    let unavailable: String

    /// 未加载时按常见截图比例占位，减少竖向留白。
    private static let placeholderAspect: CGFloat = 16 / 10

    var body: some View {
        Group {
            if let image, image.size.width > 1, image.size.height > 1 {
                Image(nsImage: image)
                    .resizable()
                    .aspectRatio(image.size, contentMode: .fit)
                    .frame(maxWidth: .infinity, maxHeight: maxHeight)
            } else {
                ZStack {
                    RoundedRectangle(cornerRadius: 8)
                        .fill(Color(nsColor: .quaternaryLabelColor).opacity(0.25))
                    if loading {
                        ProgressView().controlSize(.small)
                    } else {
                        Label(unavailable, systemImage: "photo.badge.exclamationmark")
                            .font(.caption).workspaceSecondaryText()
                    }
                }
                .aspectRatio(Self.placeholderAspect, contentMode: .fit)
                .frame(maxWidth: .infinity, maxHeight: maxHeight)
            }
        }
        .clipShape(RoundedRectangle(cornerRadius: 8))
        .fixedSize(horizontal: false, vertical: true)
    }
}

/// Fit against the actual column width, not a character-count estimate.
struct ClipboardCardText: View {
    let text: String
    let lineLimit: Int
    let maxHeight: CGFloat

    var body: some View {
        Text(text)
            .font(.system(size: 13))
            .lineLimit(min(4, lineLimit)).truncationMode(.tail)
            .multilineTextAlignment(.leading)
            .foregroundStyle(Color(nsColor: .labelColor))
            .frame(maxWidth: .infinity, alignment: .leading)

    }
}
