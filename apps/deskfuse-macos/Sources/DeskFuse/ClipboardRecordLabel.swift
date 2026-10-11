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
                .clipShape(RoundedRectangle(cornerRadius: 9))
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 5) {
                Text(displayTitle)
                    .font(.system(size: 13, weight: .medium))
                    .foregroundStyle(.primary)
                    .lineLimit(2)
                    .truncationMode(.tail)
                    .multilineTextAlignment(.leading)
                Text("来自 \(meta?.originDevice ?? item.source)")
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
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
                RoundedRectangle(cornerRadius: 9)
                    .fill(Color.primary.opacity(0.035))
                if let icon = ClipboardAppInfoCache.shared.icon(for: appID) {
                    Image(nsImage: icon).resizable().scaledToFit().padding(6)
                } else {
                    Image(systemName: item.clipboardSymbol)
                        .font(.system(size: 20, weight: .regular))
                        .foregroundStyle(.secondary)
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

/// The popup owns one glass shell; its content never animates as a button.
struct ClipboardPopupSurface: ViewModifier {
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency

    @ViewBuilder func body(content: Content) -> some View {
        let shape = RoundedRectangle(cornerRadius: 22, style: .continuous)
        if reduceTransparency {
            content.background(Color(nsColor: .windowBackgroundColor), in: shape)
        } else if #available(macOS 26.0, *) {
            content.glassEffect(.regular, in: shape)
        } else {
            content.background(.regularMaterial, in: shape)
        }
    }
}
