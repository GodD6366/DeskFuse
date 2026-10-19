import SwiftUI

/// Loading is presentation state, not a new window or a replacement for the
/// workspace. Keep the underlying selection and scroll positions intact.
struct WorkspaceLoadingOverlay: ViewModifier {
    let isPresented: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    func body(content: Content) -> some View {
        content
            .disabled(isPresented)
            .accessibilityHidden(isPresented)
            .overlay {
                ZStack {
                    if isPresented {
                        WorkspaceLoadingView()
                            .transition(.opacity)
                    }
                }
                // Scope the transaction to the overlay: snapshot updates,
                // navigation, and clipboard selection must remain immediate.
                .animation(
                    reduceMotion ? .easeOut(duration: 0.15)
                        : .timingCurve(0.23, 1, 0.32, 1, duration: 0.2),
                    value: isPresented
                )
                .allowsHitTesting(isPresented)
            }
    }
}

private struct WorkspaceLoadingView: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        VStack(spacing: 20) {
            Image(systemName: "rectangle.on.rectangle")
                .font(DFType.metric.weight(.medium))
                .foregroundStyle(DFPalette.accent)
                .frame(width: 72, height: 72)
                .background(
                    DFPalette.accent.opacity(0.08),
                    in: RoundedRectangle(cornerRadius: DFRadius.panel, style: .continuous)
                )
                .accessibilityHidden(true)

            VStack(spacing: 8) {
                Text("正在启动 DeskFuse")
                    .font(.title2.weight(.semibold))
                Text("正在连接本地引擎，准备你的工作区。")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .fixedSize(horizontal: false, vertical: true)
            }

            Group {
                if reduceMotion {
                    Image(systemName: "hourglass")
                        .foregroundStyle(.secondary)
                } else {
                    ProgressView()
                        .controlSize(.small)
                }
            }
            .frame(height: 20)
            .accessibilityHidden(true)
        }
        .padding(32)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(nsColor: .windowBackgroundColor))
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
    }
}

/// Real errors remain readable and selectable; they are not transient toasts.
/// Only locally dismissible errors get a close action.
struct WorkspaceErrorNotice: View {
    let message: String
    let dismiss: (() -> Void)?
    @Environment(\.colorSchemeContrast) private var contrast

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.body)
                .foregroundStyle(.orange)
                .accessibilityHidden(true)

            VStack(alignment: .leading, spacing: 4) {
                Text("操作未完成")
                    .font(.callout.weight(.semibold))
                Text(message)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            if let dismiss {
                Button(action: dismiss) {
                    Image(systemName: "xmark")
                        .font(.caption.weight(.semibold))
                        .frame(width: 24, height: 24)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .foregroundStyle(.secondary)
                .help("关闭错误提示")
                .accessibilityLabel("关闭错误提示")
            }
        }
        .padding(14)
        .background(
            Color(nsColor: .controlBackgroundColor),
            in: RoundedRectangle(cornerRadius: DFRadius.card, style: .continuous)
        )
        .overlay {
            RoundedRectangle(cornerRadius: DFRadius.card, style: .continuous)
                .strokeBorder(
                    Color.primary.opacity(contrast == .increased ? 0.5 : 0.1),
                    lineWidth: 1
                )
                .allowsHitTesting(false)
        }
        .padding(12)
    }
}
