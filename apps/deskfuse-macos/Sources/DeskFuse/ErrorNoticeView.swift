import AppKit
import SwiftUI

/// Window-level floating notice with an opaque surface, independent of page layout.
struct ErrorNoticeView: View {
    let notice: ErrorNotice
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        if let message = notice.message {
            VStack(alignment: .leading, spacing: DFSpace.m) {
                HStack(spacing: DFSpace.s) {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .foregroundStyle(DFPalette.warning)
                        .accessibilityHidden(true)
                    Text("操作未完成").font(DFType.bodyMedium)
                    Spacer(minLength: DFSpace.m)
                    Button(action: notice.dismiss) { Image(systemName: "xmark") }
                        .buttonStyle(.dfIcon)
                        .accessibilityLabel("关闭提示")
                        .help("关闭提示")
                }
                Text(message)
                    .font(DFType.detail)
                    .foregroundStyle(DFPalette.secondaryText(scheme))
                    .lineLimit(6)
                    .fixedSize(horizontal: false, vertical: true)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                HStack {
                    Spacer()
                    Button("复制详情", action: copyDetails)
                        .buttonStyle(.dfSecondary)
                }
            }
                .padding(DFSpace.l)
                .background(DFPalette.surface(scheme),
                            in: RoundedRectangle(cornerRadius: DFRadius.card, style: .continuous))
                .overlay {
                    RoundedRectangle(cornerRadius: DFRadius.card, style: .continuous)
                        .strokeBorder(DFPalette.line(scheme), lineWidth: DFMetrics.hairline)
                }
                .shadow(color: .black.opacity(scheme == .dark ? 0.3 : 0.14), radius: 16, y: 6)
                .accessibilityElement(children: .contain)
        }
    }

    private func copyDetails() {
        let details = notice.rawMessage ?? notice.message ?? ""
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(details, forType: .string)
    }
}
