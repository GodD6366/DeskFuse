import SwiftUI

private struct QuietWorkspaceKey: EnvironmentKey {
    static let defaultValue = false
}

extension EnvironmentValues {
    var quietWorkspace: Bool {
        get { self[QuietWorkspaceKey.self] }
        set { self[QuietWorkspaceKey.self] = newValue }
    }
}

/// Stable main-window surfaces. Floating panels share control metrics while
/// opting out of opaque workspace surfaces.
enum WorkspaceTypography {
    static let body = Font.system(size: 13)
    static let detail = Font.system(size: 12)
    static let heading = Font.system(size: 14, weight: .semibold)
    static let section = Font.system(size: 17, weight: .semibold)
}

enum QuietWorkspacePalette {
    static func canvas(_ scheme: ColorScheme) -> Color {
        scheme == .dark ? Color(red: 0.102, green: 0.102, blue: 0.118)
                        : Color(red: 0.957, green: 0.957, blue: 0.965)
    }
    static func surface(_ scheme: ColorScheme) -> Color {
        scheme == .dark ? Color(red: 0.137, green: 0.137, blue: 0.153) : .white
    }
    static func control(_ scheme: ColorScheme) -> Color {
        scheme == .dark ? Color.white.opacity(0.07) : Color.black.opacity(0.04)
    }
    static func line(_ scheme: ColorScheme) -> Color {
        Color.primary.opacity(scheme == .dark ? 0.14 : 0.09)
    }
    static func secondaryText(_ scheme: ColorScheme) -> Color {
        Color(white: scheme == .dark ? 0.75 : 0.35)
    }
}

private struct WorkspaceSecondaryText: ViewModifier {
    @Environment(\.quietWorkspace) private var quiet
    @Environment(\.colorScheme) private var scheme
    func body(content: Content) -> some View {
        content.foregroundStyle(quiet ? QuietWorkspacePalette.secondaryText(scheme) : Color.secondary)
    }
}

private struct QuietWorkspaceStyle: ViewModifier {
    @Environment(\.colorScheme) private var scheme
    func body(content: Content) -> some View {
        content
            .environment(\.quietWorkspace, true)
            .font(WorkspaceTypography.body)
            .controlSize(.small)
            .buttonStyle(WorkspaceButtonStyle())
            .background(QuietWorkspacePalette.canvas(scheme))
    }
}

struct QuietControlSurface: ViewModifier {
    @Environment(\.colorScheme) private var scheme
    var cornerRadius: CGFloat = 8
    func body(content: Content) -> some View {
        content.background(QuietWorkspacePalette.control(scheme),
                           in: RoundedRectangle(cornerRadius: cornerRadius))
    }
}

extension View {
    func workspaceSecondaryText() -> some View { modifier(WorkspaceSecondaryText()) }
    func quietWorkspaceStyle() -> some View { modifier(QuietWorkspaceStyle()) }

    /// Presentation boundaries must not inherit the main window's opaque mode.
    func floatingWorkspaceStyle() -> some View {
        environment(\.quietWorkspace, false)
            .font(WorkspaceTypography.body)
            .controlSize(.small)
            .buttonStyle(WorkspaceButtonStyle())
    }
}

struct QuietPageHeader: View {
    let section: String
    let title: String
    var badge: String? = nil
    var body: some View {
        HStack(spacing: 9) {
            Text(section).foregroundStyle(.secondary)
            Text("/").foregroundStyle(.secondary)
            Text(title).fontWeight(.semibold)
            if let badge { FeatureBadge(text: badge) }
            Spacer(minLength: 0)
        }
        .font(.system(size: 14))
        .padding(.horizontal, 20)
        .frame(height: 44)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isHeader)
    }
}

struct QuietSidebarRow: View {
    let title: String
    var symbol: String? = nil
    let selected: Bool
    var badge: String? = nil
    var trailingText: String? = nil
    @Environment(\.colorScheme) private var scheme
    @State private var hovering = false

    var body: some View {
        HStack(spacing: 10) {
            if let symbol {
                Image(systemName: symbol)
                    .font(.system(size: 14))
                    .foregroundStyle(selected ? Color.primary : Color.secondary)
                    .frame(width: 18)
                    .accessibilityHidden(true)
            }
            Text(title).lineLimit(1)
            if let badge { FeatureBadge(text: badge) }
            Spacer(minLength: 0)
            if let trailingText {
                Text(trailingText)
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
        .font(.system(size: 13, weight: selected ? .medium : .regular))
        .foregroundStyle(selected ? Color.primary : Color.secondary)
        .padding(.horizontal, 10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .frame(height: 32)
        .background(selected ? QuietWorkspacePalette.surface(scheme)
                    : hovering ? QuietWorkspacePalette.control(scheme) : .clear,
                    in: RoundedRectangle(cornerRadius: 8))
        .overlay {
            RoundedRectangle(cornerRadius: 8)
                .strokeBorder(selected ? QuietWorkspacePalette.line(scheme) : .clear, lineWidth: 0.5)
        }
        .contentShape(RoundedRectangle(cornerRadius: 8))
        .onHover { hovering = $0 }
        .accessibilityAddTraits(selected ? .isSelected : [])
        .background(NativeListSelection())
    }
}

/// A setting keeps its description beside the control it explains.
struct QuietSettingsRow<Control: View>: View {
    let title: String
    var detail: String? = nil
    var symbol: String? = nil
    var badge: String? = nil
    @ViewBuilder let control: () -> Control

    var body: some View {
        HStack(spacing: 16) {
            HStack(alignment: .top, spacing: 12) {
                if let symbol {
                    Image(systemName: symbol)
                        .font(.system(size: 14))
                        .foregroundStyle(.secondary)
                        .frame(width: 18)
                        .padding(.top, 1)
                        .accessibilityHidden(true)
                }
                VStack(alignment: .leading, spacing: 4) {
                    HStack(spacing: 8) {
                        Text(title)
                        if let badge { FeatureBadge(text: badge) }
                    }
                    if let detail {
                        Text(detail)
                            .font(.system(size: 12))
                            .foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            control().fixedSize(horizontal: true, vertical: false)
        }
        .font(.system(size: 13))
        .frame(minHeight: 32)
        .padding(.vertical, 4)
    }
}

struct QuietSettingsToggle: View {
    let title: String
    var detail: String? = nil
    @Binding var isOn: Bool
    init(_ title: String, detail: String? = nil, isOn: Binding<Bool>) {
        self.title = title
        self.detail = detail
        _isOn = isOn
    }
    var body: some View {
        QuietSettingsRow(title: title, detail: detail) {
            Toggle(title, isOn: $isOn)
                .labelsHidden().toggleStyle(.switch).controlSize(.small)
        }
    }
}
