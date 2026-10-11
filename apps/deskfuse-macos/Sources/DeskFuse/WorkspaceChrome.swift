import SwiftUI

/// Content stays opaque; only the card casts a shadow onto the window canvas.
struct WorkspaceContentSurface: ViewModifier {
    func body(content: Content) -> some View {
        content
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Color(nsColor: .textBackgroundColor))
            .clipShape(RoundedRectangle(cornerRadius: 20, style: .continuous))
            .shadow(color: .black.opacity(0.08), radius: 10, y: 3)
    }
}

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
        let tint: Color? = destructive ? .red : (prominent ? .blue : nil)
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

/// Sidebar row keeps an explicit per-element style: macOS sidebar Lists tint
/// `Label` icons with the accent color, which would override the design's
/// black navigation glyphs.
struct SidebarNavRow: View {
    let title: String
    let symbol: String
    let selected: Bool
    var badge: String? = nil

    var body: some View {
        HStack(spacing: 9) {
            Image(systemName: symbol)
                .font(.system(size: 17, weight: .medium))
                .symbolRenderingMode(.monochrome)
                .foregroundStyle(selected ? Color.accentColor : Color.secondary)
                .frame(width: 24, alignment: .center)
            Text(title)
                .font(.system(size: 16, weight: selected ? .semibold : .regular))
                .foregroundStyle(selected ? Color.accentColor : Color.primary)
                .lineLimit(1)
            if let badge {
                FeatureBadge(text: badge)
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(selected ? Color(red: 0.89, green: 0.93, blue: 1.0) : .clear,
                    in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .background(NativeListSelection())
        .listRowInsets(EdgeInsets(top: 2, leading: 6, bottom: 2, trailing: 6))
    }
}

struct FeatureBadge: View {
    let text: String

    init(text: String = "Beta") {
        self.text = text
    }

    var body: some View {
        Text(text)
            .font(.system(size: 10, weight: .semibold, design: .rounded))
            .foregroundStyle(.orange)
            .padding(.horizontal, 6)
            .padding(.vertical, 2)
            .background(Color.orange.opacity(0.12), in: Capsule())
            .overlay {
                Capsule().strokeBorder(Color.orange.opacity(0.28), lineWidth: 0.5)
            }
            .accessibilityLabel(text)
    }
}
