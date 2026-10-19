import AppKit
import SwiftUI
import DeskFuseEngine

/// The page and popup share the same reading order: thumbnail, title, origin.
/// This is display-only; selection and paste remain owned by each presentation.
struct ClipboardRecordLabel: View {
    let item: HistoryItem
    let title: String?
    let appID: String?
    let thumbnail: NSImage?
    let meta: ClipboardItemMeta?

    var body: some View {
        HStack(spacing: 12) {
            thumbnailView
                .frame(width: 42, height: 42)
                .clipShape(RoundedRectangle(cornerRadius: DFRadius.control))
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 5) {
                Text(displayTitle)
                    .font(DFType.bodyMedium)
                    .foregroundStyle(.primary)
                    .lineLimit(2)
                    .truncationMode(.tail)
                    .multilineTextAlignment(.leading)
                Text("来自 \(meta?.originDevice ?? item.source)")
                    .font(DFType.caption)
                    .dfSecondaryText()
                    .lineLimit(1)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private var displayTitle: String {
        let custom = (title ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        return custom.isEmpty ? item.clipboardTitle : custom
    }

    @ViewBuilder private var thumbnailView: some View {
        if let thumbnail {
            Image(nsImage: thumbnail).resizable().aspectRatio(contentMode: .fill)
        } else {
            ZStack {
                RoundedRectangle(cornerRadius: DFRadius.control)
                    .fill(Color.primary.opacity(0.035))
                if let icon = ClipboardAppInfoCache.shared.icon(for: appID) {
                    Image(nsImage: icon).resizable().scaledToFit().padding(6)
                } else {
                    Image(systemName: item.clipboardSymbol)
                        .font(DFType.pageTitle.weight(.regular))
                        .dfSecondaryText()
                }
            }
        }
    }
}

/// Glass is chrome; reading content uses a quiet, adaptive material instead.
struct ClipboardReadingSurface: ViewModifier {
    let cornerRadius: CGFloat
    func body(content: Content) -> some View {
        content
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: cornerRadius))
            .overlay {
                RoundedRectangle(cornerRadius: cornerRadius)
                    .strokeBorder(.quaternary, lineWidth: 0.5)
            }
    }
}

/// 弹出面板使用与主窗口一致的实心表面，不透出背后的窗口内容。
struct ClipboardPopupSurface: ViewModifier {
    @Environment(\.colorScheme) private var scheme

    func body(content: Content) -> some View {
        let shape = RoundedRectangle(cornerRadius: DFRadius.panel, style: .continuous)
        content
            .background(DFPalette.canvas(scheme), in: shape)
            .overlay { shape.strokeBorder(DFPalette.line(scheme), lineWidth: DFMetrics.hairline) }
    }
}
