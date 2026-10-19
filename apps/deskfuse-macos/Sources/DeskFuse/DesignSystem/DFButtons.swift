import SwiftUI

/// 全局按钮：主要、次要、低调文字、危险四种语义，几何规格一致，只换表面。
struct DFButtonStyle: ButtonStyle {
    enum Kind { case primary, secondary, plain, destructive }
    var kind: Kind = .secondary
    var compact = false

    func makeBody(configuration: Configuration) -> some View {
        DFButtonBody(configuration: configuration,
                     kind: configuration.role == .destructive && kind != .plain ? .destructive : kind,
                     compact: compact)
    }
}

private struct DFButtonBody: View {
    let configuration: ButtonStyleConfiguration
    let kind: DFButtonStyle.Kind
    let compact: Bool
    @Environment(\.isEnabled) private var enabled
    @Environment(\.dfOpaque) private var opaque
    @Environment(\.colorScheme) private var scheme
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    @State private var hovering = false

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: DFRadius.control, style: .continuous)
        configuration.label
            .font(DFType.bodyMedium)
            .lineLimit(1)
            .padding(.horizontal, kind == .plain ? DFSpace.s : DFMetrics.controlInset)
            .frame(minHeight: compact ? DFMetrics.compactControlHeight : DFMetrics.controlHeight)
            .foregroundStyle(foreground)
            .background { background(shape) }
            .overlay {
                if kind == .secondary && opaque {
                    shape.strokeBorder(DFPalette.line(scheme), lineWidth: DFMetrics.hairline)
                }
            }
            .contentShape(shape)
            .opacity(configuration.isPressed ? 0.72 : 1)
            .onHover { hovering = $0 }
    }

    private var foreground: Color {
        guard enabled else { return Color.secondary.opacity(0.7) }
        switch kind {
        case .primary, .destructive: return .white
        case .secondary: return .primary
        case .plain: return DFPalette.accent
        }
    }

    @ViewBuilder private func background(_ shape: RoundedRectangle) -> some View {
        switch kind {
        case .primary:
            shape.fill(enabled ? DFPalette.accent : DFPalette.control(scheme))
                .brightness(hovering && enabled ? 0.04 : 0)
        case .destructive:
            shape.fill(enabled ? DFPalette.danger : DFPalette.control(scheme))
                .brightness(hovering && enabled ? 0.04 : 0)
        case .secondary:
            if opaque {
                shape.fill(hovering && enabled ? DFPalette.control(scheme) : DFPalette.surface(scheme))
            } else if reduceTransparency {
                shape.fill(Color(nsColor: .controlBackgroundColor))
            } else {
                shape.fill(.regularMaterial)
            }
        case .plain:
            shape.fill(hovering && enabled ? DFPalette.hover : .clear)
        }
    }
}

/// 纯图标按钮：方形热区、悬停底色，用于工具栏与行内操作。
struct DFIconButtonStyle: ButtonStyle {
    var size: CGFloat = DFMetrics.iconButton

    func makeBody(configuration: Configuration) -> some View {
        DFIconButtonBody(configuration: configuration, size: size)
    }
}

private struct DFIconButtonBody: View {
    let configuration: ButtonStyleConfiguration
    let size: CGFloat
    @Environment(\.isEnabled) private var enabled
    @Environment(\.colorScheme) private var scheme
    @State private var hovering = false

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: DFRadius.control, style: .continuous)
        configuration.label
            .font(DFType.icon(.regular, weight: .medium))
            .foregroundStyle(enabled ? Color.primary.opacity(0.78) : Color.secondary.opacity(0.5))
            .frame(width: size, height: size)
            .background(hovering && enabled ? DFPalette.control(scheme) : .clear, in: shape)
            .contentShape(shape)
            .opacity(configuration.isPressed ? 0.6 : 1)
            .onHover { hovering = $0 }
    }
}

extension ButtonStyle where Self == DFButtonStyle {
    static var dfPrimary: DFButtonStyle { DFButtonStyle(kind: .primary) }
    static var dfSecondary: DFButtonStyle { DFButtonStyle(kind: .secondary) }
    static var dfPlain: DFButtonStyle { DFButtonStyle(kind: .plain) }
    static var dfDestructive: DFButtonStyle { DFButtonStyle(kind: .destructive) }
    static func df(_ kind: DFButtonStyle.Kind, compact: Bool = false) -> DFButtonStyle {
        DFButtonStyle(kind: kind, compact: compact)
    }
}

extension ButtonStyle where Self == DFIconButtonStyle {
    static var dfIcon: DFIconButtonStyle { DFIconButtonStyle() }
}

/// 带图标与文字的页面操作按钮。
struct DFActionButton: View {
    let title: String
    let systemImage: String
    var help: String? = nil
    var kind: DFButtonStyle.Kind = .secondary
    let action: () -> Void

    init(_ title: String, systemImage: String, help: String? = nil,
         kind: DFButtonStyle.Kind = .secondary, action: @escaping () -> Void) {
        self.title = title
        self.systemImage = systemImage
        self.help = help
        self.kind = kind
        self.action = action
    }

    var body: some View {
        Button(action: action) { Label(title, systemImage: systemImage) }
            .buttonStyle(.df(kind))
            .help(help ?? title)
    }
}

/// 让菜单等非按钮控件拥有与按钮一致的几何规格。
struct DFControlLabel: ViewModifier {
    var compact = false
    func body(content: Content) -> some View {
        content
            .font(DFType.bodyMedium)
            .lineLimit(1)
            .padding(.horizontal, DFMetrics.controlInset)
            .frame(minHeight: compact ? DFMetrics.compactControlHeight : DFMetrics.controlHeight)
            .contentShape(RoundedRectangle(cornerRadius: DFRadius.control, style: .continuous))
    }
}
