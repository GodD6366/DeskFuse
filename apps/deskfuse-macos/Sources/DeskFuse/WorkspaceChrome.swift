import SwiftUI

enum WorkspaceControlMetrics {
    static let height: CGFloat = 36
    static let horizontalInset: CGFloat = 12
    static let cornerRadius: CGFloat = 10
    static let font = Font.system(size: 13, weight: .medium)
}

struct WorkspaceControlLabel: ViewModifier {
    func body(content: Content) -> some View {
        content
            .font(WorkspaceControlMetrics.font)
            .lineLimit(1)
            .padding(.horizontal, WorkspaceControlMetrics.horizontalInset)
            .frame(height: WorkspaceControlMetrics.height)
            .contentShape(RoundedRectangle(cornerRadius: WorkspaceControlMetrics.cornerRadius))
    }
}

private struct WorkspaceControlSurface: ViewModifier {
    let prominent: Bool
    let destructive: Bool
    @Environment(\.isEnabled) private var enabled
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency

    @ViewBuilder func body(content: Content) -> some View {
        let shape = RoundedRectangle(cornerRadius: WorkspaceControlMetrics.cornerRadius)
        let tint: Color? = destructive ? .red : (prominent ? DeskFuseBrand.accent : nil)
        if let tint {
            if #available(macOS 26.0, *), !reduceTransparency {
                content
                    .foregroundStyle(enabled ? Color.white : Color.secondary)
                    .glassEffect(.regular.tint(enabled ? tint : .clear).interactive(), in: shape)
            } else {
                content
                    .foregroundStyle(enabled ? Color.white : Color.secondary)
                    .background(enabled ? tint : Color(nsColor: .controlBackgroundColor), in: shape)
            }
        } else {
            content.modifier(NativeGlassBackdrop(cornerRadius: WorkspaceControlMetrics.cornerRadius))
        }
    }
}

/// Compact in-page action that replaces window toolbar items.
struct WorkspaceChromeButton: View {
    let title: String
    let systemImage: String
    let help: String?
    let prominent: Bool
    let destructive: Bool
    let action: () -> Void

    init(_ title: String, systemImage: String, help: String? = nil, prominent: Bool = false, destructive: Bool = false, action: @escaping () -> Void) {
        self.title = title
        self.systemImage = systemImage
        self.help = help
        self.prominent = prominent
        self.destructive = destructive
        self.action = action
    }

    var body: some View {
        Button(action: action) {
            Label(title, systemImage: systemImage)
                .modifier(WorkspaceControlLabel())
                .modifier(WorkspaceControlSurface(prominent: prominent, destructive: destructive))
        }
        .buttonStyle(.plain)
        .help(help ?? title)
    }
}

struct SidebarVisibilityButton: View {
    @Binding var visibility: NavigationSplitViewVisibility

    var body: some View {
        Button {
            visibility = visibility == .detailOnly ? .all : .detailOnly
        } label: {
            Image(systemName: "sidebar.left")
                .font(.system(size: 13, weight: .semibold))
                .frame(width: 28, height: 28)
        }
        .buttonStyle(.plain)
        .modifier(NativeGlassSurface(cornerRadius: 15))
        .help(visibility == .detailOnly ? "显示侧栏" : "隐藏侧栏")
        .accessibilityLabel(visibility == .detailOnly ? "显示侧栏" : "隐藏侧栏")
    }
}

struct FeatureBadge: View {
    let text: String
    /// 位于蓝色选中行内时改用白字，避免橙字落在强调色底上不可读。
    var onAccent = false

    init(text: String = "Beta", onAccent: Bool = false) {
        self.text = text
        self.onAccent = onAccent
    }

    var body: some View {
        Text(text)
            .font(.system(size: 10, weight: .semibold, design: .rounded))
            .foregroundStyle(onAccent ? Color.white : Color.orange)
            .padding(.horizontal, 6)
            .padding(.vertical, 2)
            .background(onAccent ? Color.white.opacity(0.22) : Color.orange.opacity(0.12), in: Capsule())
            .overlay {
                Capsule().strokeBorder(
                    onAccent ? Color.white.opacity(0.35) : Color.orange.opacity(0.28),
                    lineWidth: 0.5
                )
            }
            .accessibilityLabel(text)
    }
}
