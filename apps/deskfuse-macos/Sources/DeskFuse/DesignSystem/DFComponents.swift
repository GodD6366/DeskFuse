import AppKit
import SwiftUI

// MARK: - 环境

private struct DFOpaqueKey: EnvironmentKey {
    static let defaultValue = false
}

extension EnvironmentValues {
    /// 主窗口使用稳定的实心表面；浮动面板（菜单栏、暂存区、弹出内容）保留系统材质。
    var dfOpaque: Bool {
        get { self[DFOpaqueKey.self] }
        set { self[DFOpaqueKey.self] = newValue }
    }
}

private struct DFWorkspaceStyle: ViewModifier {
    @Environment(\.colorScheme) private var scheme
    func body(content: Content) -> some View {
        content
            .environment(\.dfOpaque, true)
            .font(DFType.body)
            .controlSize(.small)
            .buttonStyle(.dfSecondary)
            .background(DFPalette.canvas(scheme))
    }
}

private struct DFSecondaryText: ViewModifier {
    @Environment(\.dfOpaque) private var opaque
    @Environment(\.colorScheme) private var scheme
    func body(content: Content) -> some View {
        content.foregroundStyle(opaque ? DFPalette.secondaryText(scheme) : Color.secondary)
    }
}

private struct DFCanvas: ViewModifier {
    @Environment(\.dfOpaque) private var opaque
    @Environment(\.colorScheme) private var scheme
    func body(content: Content) -> some View {
        content.background(opaque ? DFPalette.canvas(scheme) : Color(nsColor: .textBackgroundColor))
    }
}

extension View {
    /// 主窗口工作区：画布底色、默认字体与按钮样式。
    func dfWorkspace() -> some View { modifier(DFWorkspaceStyle()) }

    /// 浮动面板边界：不继承主窗口的实心表面，但共用控件规格。
    func dfFloating() -> some View {
        environment(\.dfOpaque, false)
            .font(DFType.body)
            .controlSize(.small)
            .buttonStyle(.dfSecondary)
    }

    func dfSecondaryText() -> some View { modifier(DFSecondaryText()) }
    func dfCanvas() -> some View { modifier(DFCanvas()) }
}

// MARK: - 表面

/// 内容卡片：白色（深色模式为深灰）圆角表面加细描边。
private struct DFCardSurface: ViewModifier {
    @Environment(\.colorScheme) private var scheme
    @Environment(\.dfOpaque) private var opaque
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    var radius: CGFloat

    func body(content: Content) -> some View {
        let shape = RoundedRectangle(cornerRadius: radius, style: .continuous)
        content
            .background(opaque ? DFPalette.surface(scheme) : Color.primary.opacity(reduceTransparency ? DFOpacity.soft : DFOpacity.faint),
                        in: shape)
            .overlay {
                if opaque { shape.strokeBorder(DFPalette.line(scheme), lineWidth: DFMetrics.hairline) }
            }
    }
}

/// 控件底：搜索框、分段按钮等次级表面。
struct DFControlSurface: ViewModifier {
    @Environment(\.colorScheme) private var scheme
    var radius: CGFloat = DFRadius.control
    func body(content: Content) -> some View {
        content.background(DFPalette.control(scheme), in: RoundedRectangle(cornerRadius: radius, style: .continuous))
    }
}

extension View {
    func dfCard(radius: CGFloat = DFRadius.card) -> some View { modifier(DFCardSurface(radius: radius)) }
    func dfControlSurface(radius: CGFloat = DFRadius.control) -> some View { modifier(DFControlSurface(radius: radius)) }
}

/// 卡片内的分隔线，左侧可留出图标列。
struct DFDivider: View {
    var inset: CGFloat = 0
    @Environment(\.colorScheme) private var scheme
    var body: some View {
        Rectangle()
            .fill(DFPalette.line(scheme))
            .frame(height: DFMetrics.hairline)
            .padding(.leading, inset)
    }
}

// MARK: - 页面

/// 每个页面唯一的标题行：标题、可选说明与右侧页面级操作。
struct DFPageHeader<Trailing: View>: View {
    let title: String
    var subtitle: String? = nil
    var badge: String? = nil
    @ViewBuilder var trailing: () -> Trailing

    init(_ title: String, subtitle: String? = nil, badge: String? = nil,
         @ViewBuilder trailing: @escaping () -> Trailing) {
        self.title = title
        self.subtitle = subtitle
        self.badge = badge
        self.trailing = trailing
    }

    var body: some View {
        HStack(alignment: .center, spacing: DFSpace.m) {
            VStack(alignment: .leading, spacing: DFSpace.xxs) {
                HStack(spacing: DFSpace.s) {
                    Text(title).font(DFType.pageTitle)
                    if let badge { DFBadge(badge) }
                }
                if let subtitle {
                    Text(subtitle)
                        .font(DFType.detail)
                        .dfSecondaryText()
                        .lineLimit(2)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .accessibilityElement(children: .combine)
            .accessibilityAddTraits(.isHeader)
            Spacer(minLength: DFSpace.m)
            trailing()
        }
        .padding(.horizontal, DFMetrics.pageInset)
        .padding(.top, DFSpace.l)
        .padding(.bottom, DFSpace.m)
        .frame(minHeight: 64)
    }
}

extension DFPageHeader where Trailing == EmptyView {
    init(_ title: String, subtitle: String? = nil, badge: String? = nil) {
        self.init(title, subtitle: subtitle, badge: badge) { EmptyView() }
    }
}

/// 可滚动的页面正文：统一留白，表单页限制最大宽度避免设置行被拉得过长。
struct DFScrollPage<Content: View>: View {
    var maxWidth: CGFloat? = DFMetrics.formMaxWidth
    var spacing: CGFloat = DFSpace.xl
    @ViewBuilder var content: () -> Content

    init(maxWidth: CGFloat? = DFMetrics.formMaxWidth, spacing: CGFloat = DFSpace.xl,
         @ViewBuilder content: @escaping () -> Content) {
        self.maxWidth = maxWidth
        self.spacing = spacing
        self.content = content
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: spacing, content: content)
                .frame(maxWidth: maxWidth ?? .infinity, alignment: .leading)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, DFMetrics.pageInset)
                .padding(.top, DFMetrics.pageTopInset)
                .padding(.bottom, DFMetrics.pageBottomInset)
        }
    }
}

/// 卡片上方的分区标签。
struct DFSectionLabel: View {
    let title: String
    var symbol: String? = nil

    init(_ title: String, symbol: String? = nil) {
        self.title = title
        self.symbol = symbol
    }

    var body: some View {
        HStack(spacing: DFSpace.xs + 2) {
            if let symbol { Image(systemName: symbol).font(DFType.icon(.small, weight: .medium)) }
            Text(title).font(DFType.section)
        }
        .dfSecondaryText()
        .padding(.leading, DFSpace.xs)
        .accessibilityAddTraits(.isHeader)
    }
}

/// 一组相关内容：可选分区标签 + 卡片 + 可选说明。
struct DFSection<Content: View>: View {
    var title: String? = nil
    var symbol: String? = nil
    var footer: String? = nil
    var padding: CGFloat = DFMetrics.cardPadding
    @ViewBuilder var content: () -> Content

    init(_ title: String? = nil, symbol: String? = nil, footer: String? = nil,
         padding: CGFloat = DFMetrics.cardPadding, @ViewBuilder content: @escaping () -> Content) {
        self.title = title
        self.symbol = symbol
        self.footer = footer
        self.padding = padding
        self.content = content
    }

    var body: some View {
        VStack(alignment: .leading, spacing: DFSpace.s) {
            if let title { DFSectionLabel(title, symbol: symbol) }
            VStack(alignment: .leading, spacing: DFSpace.s, content: content)
                .padding(padding)
                .frame(maxWidth: .infinity, alignment: .leading)
                .dfCard()
            if let footer {
                Text(footer)
                    .font(DFType.detail)
                    .dfSecondaryText()
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.horizontal, DFSpace.xs)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// 品牌图标：侧栏、关于页共用。颜色随浅色/深色切换。
struct DeskFuseBrandMark: View {
    var size: CGFloat = 22
    @Environment(\.colorScheme) private var colorScheme

    private var image: NSImage? {
        let name = colorScheme == .dark ? "DeskFuse-icon-1024" : "DeskFuse-icon-light-1024"
        return Bundle.main.url(forResource: name, withExtension: "png")
            .flatMap { NSImage(contentsOf: $0) }
    }

    var body: some View {
        Group {
            if let image {
                Image(nsImage: image)
                    .resizable()
                    .interpolation(.high)
                    .scaledToFit()
            } else {
                Image(systemName: "rectangle.on.rectangle")
                    .resizable()
                    .scaledToFit()
            }
        }
        .frame(width: size, height: size)
        .accessibilityHidden(true)
    }
}

/// 卡片标题行：标题 + 可选说明 + 右侧操作。
struct DFCardHeader<Trailing: View>: View {
    let title: String
    var symbol: String? = nil
    var detail: String? = nil
    @ViewBuilder var trailing: () -> Trailing

    init(_ title: String, symbol: String? = nil, detail: String? = nil,
         @ViewBuilder trailing: @escaping () -> Trailing) {
        self.title = title
        self.symbol = symbol
        self.detail = detail
        self.trailing = trailing
    }

    var body: some View {
        HStack(alignment: .center, spacing: DFSpace.m) {
            if let symbol {
                Image(systemName: symbol)
                    .font(DFType.icon(.regular, weight: .medium))
                    .dfSecondaryText()
                    .frame(width: 18)
                    .accessibilityHidden(true)
            }
            VStack(alignment: .leading, spacing: DFSpace.xxs) {
                Text(title).font(DFType.title)
                if let detail {
                    Text(detail).font(DFType.detail).dfSecondaryText()
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer(minLength: DFSpace.s)
            trailing()
        }
        .accessibilityAddTraits(.isHeader)
    }
}

extension DFCardHeader where Trailing == EmptyView {
    init(_ title: String, symbol: String? = nil, detail: String? = nil) {
        self.init(title, symbol: symbol, detail: detail) { EmptyView() }
    }
}

// MARK: - 设置行

/// 设置项：名称与说明在左，控件在右。说明紧跟它解释的选项。
struct DFSettingRow<Control: View>: View {
    let title: String
    var detail: String? = nil
    var symbol: String? = nil
    var badge: String? = nil
    @ViewBuilder let control: () -> Control

    var body: some View {
        HStack(spacing: DFSpace.l) {
            HStack(alignment: .top, spacing: DFSpace.m) {
                if let symbol {
                    Image(systemName: symbol)
                        .font(DFType.icon(.regular))
                        .dfSecondaryText()
                        .frame(width: 18)
                        .padding(.top, 1)
                        .accessibilityHidden(true)
                }
                VStack(alignment: .leading, spacing: DFSpace.xxs) {
                    HStack(spacing: DFSpace.s) {
                        Text(title)
                        if let badge { DFBadge(badge) }
                    }
                    if let detail {
                        Text(detail)
                            .font(DFType.detail)
                            .dfSecondaryText()
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            control().fixedSize(horizontal: true, vertical: false)
        }
        .font(DFType.body)
        .frame(minHeight: DFMetrics.rowHeight)
        .padding(.vertical, DFSpace.xxs)
    }
}

struct DFToggleRow: View {
    let title: String
    var detail: String? = nil
    var symbol: String? = nil
    var badge: String? = nil
    @Binding var isOn: Bool

    init(_ title: String, detail: String? = nil, symbol: String? = nil, badge: String? = nil, isOn: Binding<Bool>) {
        self.title = title
        self.detail = detail
        self.symbol = symbol
        self.badge = badge
        _isOn = isOn
    }

    var body: some View {
        DFSettingRow(title: title, detail: detail, symbol: symbol, badge: badge) {
            Toggle(title, isOn: $isOn)
                .labelsHidden().toggleStyle(.switch).controlSize(.small)
        }
    }
}

/// 只读的信息行：名称在左、值在右。
struct DFValueRow: View {
    let title: String
    let value: String
    var selectable = false

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: DFSpace.l) {
            Text(title).dfSecondaryText()
            Spacer(minLength: DFSpace.s)
            if selectable {
                Text(value).multilineTextAlignment(.trailing).textSelection(.enabled)
            } else {
                Text(value).multilineTextAlignment(.trailing)
            }
        }
        .font(DFType.body)
        .frame(minHeight: 24)
    }
}

// MARK: - 导航

/// 侧栏与二级导航行：单色线性图标，选中使用白底细描边。
struct DFSidebarRow: View {
    let title: String
    var symbol: String? = nil
    let selected: Bool
    var badge: String? = nil
    var trailingText: String? = nil
    var trailingTone: DFTone? = nil
    @Environment(\.colorScheme) private var scheme
    @State private var hovering = false

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: DFRadius.control, style: .continuous)
        HStack(spacing: DFSpace.s + 2) {
            if let symbol {
                Image(systemName: symbol)
                    .font(DFType.icon(.regular))
                    .foregroundStyle(selected ? DFPalette.accent : Color.secondary)
                    .frame(width: 18)
                    .accessibilityHidden(true)
            }
            Text(title).lineLimit(1)
            if let badge { DFBadge(badge) }
            Spacer(minLength: 0)
            if let trailingText {
                HStack(spacing: DFSpace.xs) {
                    if let trailingTone {
                        Circle().fill(trailingTone.color).frame(width: 6, height: 6)
                    }
                    Text(trailingText).font(DFType.caption).monospacedDigit()
                }
                .foregroundStyle(.secondary)
                .lineLimit(1)
            }
        }
        .font(selected ? DFType.bodyMedium : DFType.body)
        .foregroundStyle(selected ? Color.primary : Color.primary.opacity(0.82))
        .padding(.horizontal, DFSpace.s + 2)
        .frame(maxWidth: .infinity, alignment: .leading)
        .frame(height: DFMetrics.rowHeight)
        .background(selected ? DFPalette.surface(scheme)
                    : hovering ? DFPalette.control(scheme) : .clear, in: shape)
        .overlay { shape.strokeBorder(selected ? DFPalette.line(scheme) : .clear, lineWidth: DFMetrics.hairline) }
        .contentShape(shape)
        .onHover { hovering = $0 }
        .accessibilityAddTraits(selected ? .isSelected : [])
        .background(NativeListSelection())
    }
}

// MARK: - 状态

/// Beta 等功能标识，统一文案与系统字体。
struct DFBadge: View {
    let text: String
    var tone: DFTone = .beta

    init(_ text: String = "Beta", tone: DFTone = .beta) {
        self.text = text
        self.tone = tone
    }

    var body: some View {
        Text(text)
            .font(DFType.caption.weight(.semibold))
            .foregroundStyle(tone.color)
            .padding(.horizontal, DFSpace.xs + 2)
            .padding(.vertical, 1)
            .background(tone.color.opacity(DFOpacity.soft), in: Capsule())
            .accessibilityLabel(text)
    }
}

/// 状态胶囊：圆点 + 简短状态文字。
struct DFStatusPill: View {
    let text: String
    var tone: DFTone = .neutral

    var body: some View {
        HStack(spacing: DFSpace.xs + 2) {
            Circle().fill(tone.color).frame(width: 7, height: 7)
            Text(text).font(DFType.detail.weight(.medium)).lineLimit(1)
        }
        .padding(.horizontal, DFSpace.s + 2)
        .frame(height: 24)
        .background(tone.color.opacity(DFOpacity.soft), in: Capsule())
        .accessibilityElement(children: .combine)
    }
}

/// 页面内提示条：说明问题，并给出下一步。
struct DFNotice: View {
    let tone: DFTone
    let title: String
    var message: String? = nil
    var actionTitle: String? = nil
    var action: (() -> Void)? = nil
    var onDismiss: (() -> Void)? = nil

    private var symbol: String {
        switch tone {
        case .success: "checkmark.circle.fill"
        case .warning, .beta: "exclamationmark.triangle.fill"
        case .danger: "xmark.octagon.fill"
        case .accent, .neutral: "info.circle.fill"
        }
    }

    var body: some View {
        HStack(alignment: .top, spacing: DFSpace.m) {
            Image(systemName: symbol)
                .font(DFType.icon(.regular))
                .foregroundStyle(tone.color)
                .padding(.top, 1)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: DFSpace.xxs) {
                Text(title).font(DFType.bodyMedium)
                if let message {
                    Text(message)
                        .font(DFType.detail)
                        .dfSecondaryText()
                        .fixedSize(horizontal: false, vertical: true)
                        .textSelection(.enabled)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if let actionTitle, let action {
                Button(actionTitle, action: action).buttonStyle(.dfSecondary)
            }
            if let onDismiss {
                Button(action: onDismiss) { Image(systemName: "xmark") }
                    .buttonStyle(.dfIcon)
                    .accessibilityLabel("关闭提示")
            }
        }
        .padding(DFSpace.m)
        .background(tone.color.opacity(DFOpacity.faint), in: RoundedRectangle(cornerRadius: DFRadius.control, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: DFRadius.control, style: .continuous)
                .strokeBorder(tone.color.opacity(DFOpacity.strong), lineWidth: DFMetrics.hairline)
        }
        .accessibilityElement(children: .contain)
    }
}

/// 空状态：说明当前为什么是空的，并给出明确的下一步。
struct DFEmptyState: View {
    let symbol: String
    let title: String
    var message: String? = nil
    var actionTitle: String? = nil
    var action: (() -> Void)? = nil
    var secondaryTitle: String? = nil
    var secondaryAction: (() -> Void)? = nil

    var body: some View {
        VStack(spacing: DFSpace.m) {
            Image(systemName: symbol)
                .font(DFType.icon(.hero, weight: .light))
                .foregroundStyle(.secondary)
                .accessibilityHidden(true)
            VStack(spacing: DFSpace.xs) {
                Text(title).font(DFType.title)
                if let message {
                    Text(message)
                        .font(DFType.body)
                        .dfSecondaryText()
                        .multilineTextAlignment(.center)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .frame(maxWidth: 360)
            if actionTitle != nil || secondaryTitle != nil {
                HStack(spacing: DFSpace.s) {
                    if let secondaryTitle, let secondaryAction {
                        Button(secondaryTitle, action: secondaryAction).buttonStyle(.dfSecondary)
                    }
                    if let actionTitle, let action {
                        Button(actionTitle, action: action).buttonStyle(.dfPrimary)
                    }
                }
                .padding(.top, DFSpace.xs)
            }
        }
        .padding(DFSpace.xl)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .accessibilityElement(children: .contain)
    }
}

/// 折叠区：高级信息、故障排除等默认收起的内容。
struct DFDisclosure<Content: View>: View {
    let title: String
    var symbol: String? = nil
    @State private var expanded: Bool
    @ViewBuilder var content: () -> Content

    init(_ title: String, symbol: String? = nil, expanded: Bool = false,
         @ViewBuilder content: @escaping () -> Content) {
        self.title = title
        self.symbol = symbol
        _expanded = State(initialValue: expanded)
        self.content = content
    }

    var body: some View {
        VStack(alignment: .leading, spacing: DFSpace.m) {
            Button {
                withMotion(DFMotion.snappy) { expanded.toggle() }
            } label: {
                HStack(spacing: DFSpace.s) {
                    if let symbol {
                        Image(systemName: symbol).font(DFType.icon(.small)).dfSecondaryText()
                    }
                    Text(title).font(DFType.bodyMedium)
                    Spacer(minLength: 0)
                    Image(systemName: "chevron.right")
                        .font(DFType.icon(.small, weight: .semibold))
                        .dfSecondaryText()
                        .rotationEffect(.degrees(expanded ? 90 : 0))
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityValue(expanded ? "已展开" : "已收起")
            if expanded {
                content()
            }
        }
    }
}

/// 统一的分段选择：等宽胶囊按钮组。
struct DFSegmented<Value: Hashable>: View {
    let options: [(Value, String)]
    @Binding var selection: Value
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        HStack(spacing: DFSpace.xxs) {
            ForEach(options.indices, id: \.self) { index in
                let (value, title) = options[index]
                Button {
                    selection = value
                } label: {
                    Text(title)
                        .font(selection == value ? DFType.bodyMedium : DFType.body)
                        .padding(.horizontal, DFSpace.m)
                        .frame(height: DFMetrics.compactControlHeight - 4)
                        .background(selection == value ? DFPalette.surface(scheme) : .clear,
                                    in: RoundedRectangle(cornerRadius: DFRadius.control - 2, style: .continuous))
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(selection == value ? .isSelected : [])
            }
        }
        .padding(DFSpace.xxs)
        .dfControlSurface()
    }
}
