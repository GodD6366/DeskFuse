import SwiftUI
import AppKit
import DeskFuseEngine

/// 底部横向面板的卡片。结构与 Clipbara 的 ClipboardCardView 对齐：顶部是类型
/// 徽标与相对时间，中间是内容，底部是尺寸徽标与 ⌘n 提示；来源 App 只保留竖版
/// 瀑布流卡片同款的背景水印，不再单独画头部图标。
/// 单击选中、双击粘贴、右键菜单与纵向面板的卡片完全一致。
struct ClipboardDockCard: View {
    let item: HistoryItem
    let title: String?
    let appID: String?
    let meta: ClipboardItemMeta?
    let memberships: [String]
    let selected: Bool
    let index: Int
    let showShortcut: Bool
    let remoteUnavailable: Bool
    let ocrHit: Bool
    let client: EngineClient?
    let presentation: UUID
    let active: Bool
    let availability: (Bool) -> Void
    let onSelect: () -> Void
    let onPaste: () -> Void
    let onCopy: () -> Void
    let onPreview: () -> Void
    let showsPreview: Bool
    let onRename: (String) -> Void
    let onDelete: () -> Void

    @Environment(\.colorScheme) private var scheme
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var image: NSImage?
    @State private var loading = false
    @State private var hovering = false
    @State private var editingTitle = false

    private var kind: Kind { Kind(item: item) }
    private var customTitle: String? {
        let value = title?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return value.isEmpty ? nil : value
    }
    private var badgeLabel: String { customTitle ?? kind.label }
    private var tint: Color { DFPalette.clipTint(kind.key, scheme) }
    private var secondary: Color { DFPalette.secondaryText(scheme) }
    private var metaLabel: String { kind.metaLabel(for: item) }
    /// 中文相对时间：刚刚 / N 分钟前 / N 小时前 / 昨天 / N 天前 / 8月12日。
    /// 不依赖 RelativeDateTimeFormatter，避免系统语言为英文时面板里出现 "34 min. ago"。
    private var relativeTime: String {
        guard let date = item.copiedDate else { return "" }
        let seconds = Date().timeIntervalSince(date)
        if seconds < 60 { return "刚刚" }
        if seconds < 3_600 { return "\(Int(seconds / 60)) 分钟前" }
        if seconds < 86_400 { return "\(Int(seconds / 3_600)) 小时前" }
        if seconds < 172_800 { return "昨天" }
        if seconds < 2_592_000 { return "\(Int(seconds / 86_400)) 天前" }
        return date.formatted(.dateTime.locale(Locale(identifier: "zh_CN")).month().day())
    }
    private var linkParts: (host: String, path: String)? {
        let text = item.preview.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let url = URL(string: text), ["http", "https"].contains(url.scheme?.lowercased() ?? ""),
              let host = url.host, !text.contains(where: \.isWhitespace) else { return nil }
        let path = (url.path.removingPercentEncoding ?? url.path)
        return (host, path == "/" ? "" : path)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            header
            content
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            footer
        }
        .padding(10)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .background(cardBackground, in: cardShape)
        // 来源 App 图标同时保留两种竖版做法：顶部小图标用于快速辨认（列表行同款），
        // 右下角倾斜水印用于卡片身份（瀑布流卡片同款）。
        .overlay(alignment: .bottomTrailing) { watermark }
        .clipShape(cardShape)
        .overlay {
            cardShape.strokeBorder(borderColor, lineWidth: borderWidth)
        }
        .shadow(color: shadow.color, radius: shadow.radius, y: shadow.y)
        .scaleEffect(hovering && !selected && !reduceMotion ? 1.012 : 1)
        .offset(y: hovering && !selected && !reduceMotion ? -2 : 0)
        .animation(.easeOut(duration: 0.16), value: hovering)
        .animation(.easeInOut(duration: 0.14), value: selected)
        .contentShape(RoundedRectangle(cornerRadius: DFRadius.card, style: .continuous))
        .onHover { hovering = $0 }
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
        .help(item.clipboardTitle)
        .task(id: "\(item.id)-\(item.copiedAtMs ?? 0)-\(presentation)-\(active)") { await loadThumbnail() }
        .onDisappear { image = nil; loading = false }
    }

    // MARK: - 头部

    private var header: some View {
        HStack(spacing: 6) {
            HStack(spacing: 4) {
                Image(systemName: kind.symbol)
                    .font(DFType.caption.weight(.semibold))
                Text(badgeLabel)
                    .font(DFType.caption.weight(.semibold))
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .frame(maxWidth: 168, alignment: .leading)
            }
            .foregroundStyle(tint)
            .padding(.horizontal, 6)
            .padding(.vertical, 3)
            .background(tint.opacity(scheme == .dark ? 0.26 : 0.15),
                        in: RoundedRectangle(cornerRadius: DFRadius.badge, style: .continuous))

            Spacer(minLength: 6)

            Text(relativeTime)
                .font(DFType.caption)
                .foregroundStyle(secondary)
                .lineLimit(1)
                .fixedSize()
        }
        .frame(height: 24)
    }

    // MARK: - 内容

    @ViewBuilder private var content: some View {
        switch kind {
        case .image:
            ClipboardCardImage(
                image: image,
                loading: loading,
                maxHeight: 150,
                unavailable: remoteUnavailable ? "待获取图片" : "图片不可用"
            )
        case .files:
            VStack(alignment: .leading, spacing: 6) {
                Label("\(item.fileCount) 个文件", systemImage: "doc")
                    .font(DFType.detail.weight(.medium))
                    .foregroundStyle(secondary)
                Text(item.preview.trimmingCharacters(in: .whitespacesAndNewlines))
                    .font(DFType.detail)
                    .foregroundStyle(Color(nsColor: .labelColor))
                    .lineLimit(3)
                    .truncationMode(.middle)
            }
        case .link:
            VStack(alignment: .leading, spacing: 3) {
                Text(linkParts?.host ?? item.clipboardTitle)
                    .font(DFType.body.weight(.semibold))
                    .foregroundStyle(Color(nsColor: .labelColor))
                    .lineLimit(1)
                    .truncationMode(.tail)
                if let path = linkParts?.path, !path.isEmpty {
                    Text(path)
                        .font(DFType.caption)
                        .foregroundStyle(secondary)
                        .lineLimit(2)
                        .truncationMode(.middle)
                }
            }
        case .text:
            // 比竖版卡片高 40pt：多给两行，长文本能少一次打开主窗口。
            Text(item.preview.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                ? item.clipboardTitle : item.preview)
                .font(DFType.body)
                .lineLimit(6)
                .truncationMode(.tail)
                .multilineTextAlignment(.leading)
                .foregroundStyle(Color(nsColor: .labelColor))
                .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    // MARK: - 底部

    private var footer: some View {
        HStack(spacing: 5) {
            if ocrHit {
                Image(systemName: "text.viewfinder")
                    .font(DFType.caption)
                    .foregroundStyle(secondary)
                    .help("由图片识别文字匹配")
            }
            if remoteUnavailable {
                Image(systemName: "arrow.down.circle")
                    .font(DFType.caption)
                    .foregroundStyle(secondary)
                    .help("使用时从来源设备获取")
            }
            if !metaLabel.isEmpty {
                Text(metaLabel)
                    .font(DFType.caption)
                    .foregroundStyle(secondary)
                    .lineLimit(1)
                    .padding(.horizontal, 7)
                    .padding(.vertical, 2)
                    .background(DFPalette.control(scheme), in: RoundedRectangle(cornerRadius: DFRadius.control, style: .continuous))
            }
            Spacer(minLength: 2)
            if showShortcut, index < 9 {
                Text("⌘ \(index + 1)")
                    .font(DFType.caption)
                    .monospacedDigit()
                    .foregroundStyle(secondary)
            }
        }
        .frame(height: 22)
    }

    // MARK: - 外观

    private var cardShape: RoundedRectangle {
        RoundedRectangle(cornerRadius: DFRadius.card, style: .continuous)
    }

    /// 和竖版瀑布流卡片一致：来源 App 图标 45° 倾斜、约四分之一裁出卡片。
    @ViewBuilder private var watermark: some View {
        if let icon = ClipboardAppInfoCache.shared.icon(for: appID) {
            Image(nsImage: icon)
                .resizable()
                .scaledToFit()
                .frame(width: 88, height: 88)
                .rotationEffect(.degrees(-45))
                .offset(x: 22, y: 22)
                .opacity(0.05)
                .allowsHitTesting(false)
                .accessibilityHidden(true)
        }
    }

    private var cardBackground: some ShapeStyle {
        if selected {
            return AnyShapeStyle(DFPalette.accent.opacity(scheme == .dark ? 0.16 : 0.08))
        }
        return AnyShapeStyle(DFPalette.surface(scheme))
    }

    private var borderColor: Color {
        if selected { return DFPalette.accent }
        if hovering { return DFPalette.accent.opacity(0.45) }
        return DFPalette.border
    }

    private var borderWidth: CGFloat {
        selected ? 2 : (hovering ? 1.25 : DFMetrics.hairline + 0.25)
    }

    private var shadow: (color: Color, radius: CGFloat, y: CGFloat) {
        if selected { return (DFPalette.accent.opacity(0.22), 12, 5) }
        if hovering { return (Color.black.opacity(0.16), 12, 4) }
        return (Color.black.opacity(0.07), 3, 1)
    }

    @MainActor private func loadThumbnail() async {
        guard item.hasImage, !remoteUnavailable else { image = nil; loading = false; return }
        if let cached = ClipboardThumbnailCache.shared.cached(item.id, maxPixelSize: 480) {
            image = cached; loading = false
            return
        }
        image = nil
        guard active, let client else { loading = false; return }
        loading = true
        let cached = await ClipboardThumbnailCache.shared.thumbnail(for: item.id, client: client, maxPixelSize: 480)
        guard !Task.isCancelled else { return }
        loading = false
        image = cached
    }

    /// 内容类型与 Clipbara 的 ContentType 对应；决定徽标、图标与强调色。
    private enum Kind {
        case text, image, files, link

        init(item: HistoryItem) {
            if item.fileCount > 0 { self = .files; return }
            if item.hasImage { self = .image; return }
            let text = item.preview.trimmingCharacters(in: .whitespacesAndNewlines)
            if let url = URL(string: text), ["http", "https"].contains(url.scheme?.lowercased() ?? ""),
               url.host != nil, !text.contains(where: \.isWhitespace) {
                self = .link
                return
            }
            self = .text
        }

        var key: String {
            switch self {
            case .text: "text"
            case .image: "image"
            case .files: "files"
            case .link: "link"
            }
        }

        var label: String {
            switch self {
            case .text: "文本"
            case .image: "图片"
            case .files: "文件"
            case .link: "链接"
            }
        }

        var symbol: String {
            switch self {
            case .text: "text.alignleft"
            case .image: "photo"
            case .files: "doc"
            case .link: "link"
            }
        }

        func metaLabel(for item: HistoryItem) -> String {
            switch self {
            case .files: return "文件"
            // 图片摘要的 length 可能为 0（尚未取到体积）；此时显示类型而不是“0 字节”。
            case .image: return item.length > 0 ? Self.sizeLabel(item.length) : "图片"
            case .link: return "URL"
            case .text:
                // `length` 是原始文本的 UTF-8 字节数；整段文本都在 preview 里时
                // 才能换算成字符数，否则退回字节数。
                let preview = item.preview
                if !preview.isEmpty, preview.utf8.count == Int(clamping: item.length) {
                    return "\(preview.count) 字符"
                }
                return item.length > 0 ? Self.sizeLabel(item.length) : ""
            }
        }

        /// 面板内统一用中文尺寸说明，和卡片字符数徽标同一种读法。
        static func sizeLabel(_ bytes: UInt64) -> String {
            if bytes < 1_000 { return "\(bytes) 字节" }
            let units: [(String, Double)] = [("KB", 1_000), ("MB", 1_000_000), ("GB", 1_000_000_000)]
            var chosen = units[0]
            for unit in units where Double(bytes) >= unit.1 { chosen = unit }
            let value = Double(bytes) / chosen.1
            let text = value >= 100 ? String(format: "%.0f", value) : String(format: "%.1f", value)
                .replacingOccurrences(of: ".0", with: "")
            return "\(text) \(chosen.0)"
        }
    }
}
