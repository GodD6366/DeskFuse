import AppKit
import SwiftUI

/// macOS「系统设置」风格的设计令牌与共享组件。
///
/// 主窗口与设置窗口共用这一套：左侧彩色方块图标导航、右侧亮底内容区加浅灰
/// 圆角 block。所有尺寸与配色只在这里定义一次，页面不再各自手写圆角和底色。
enum SystemSettingsMetrics {
    /// 侧栏导航行。
    static let rowHeight: CGFloat = 30
    static let rowSpacing: CGFloat = 2
    static let rowCorner: CGFloat = 7
    static let sidebarHorizontalInset: CGFloat = 10
    /// List 行内缩进：让导航行贴近侧栏边缘，与系统设置的窄边距一致。
    static let rowInsets = EdgeInsets(top: 1, leading: 6, bottom: 1, trailing: 6)

    /// 彩色图标方块。
    static let iconSize: CGFloat = 21
    static let iconCorner: CGFloat = 6
    static let iconGlyphRatio: CGFloat = 0.57

    /// 内容 block。
    static let blockCorner: CGFloat = 10
    static let blockPadding: CGFloat = 14

    /// 页面留白。
    static let pageHorizontalInset: CGFloat = 20
    static let pageTopInset: CGFloat = 18
    /// 设置页正文容器：最大宽度、水平内缩进与底部留白。
    /// SystemSettingsFormPage 与个别手写 ScrollView 的设置页共用。
    static let formPageMaxWidth: CGFloat = 680
    static let formPageHorizontalInset: CGFloat = 26
    static let formPageBottomInset: CGFloat = 26
}

// MARK: - 彩色图标方块

/// 系统设置同款的彩色圆角方块：纯色底 + 顶部高光 + 白色符号。
struct SystemSettingsIcon: View {
    let symbol: String
    let color: Color
    var size: CGFloat = SystemSettingsMetrics.iconSize

    var body: some View {
        let shape = RoundedRectangle(
            cornerRadius: size * SystemSettingsMetrics.iconCorner / SystemSettingsMetrics.iconSize,
            style: .continuous
        )
        shape
            .fill(color)
            .overlay {
                // 顶部高光只做质感提示，不改变方块本身的识别色。
                shape.fill(
                    LinearGradient(
                        colors: [.white.opacity(0.22), .white.opacity(0)],
                        startPoint: .top,
                        endPoint: .center
                    )
                )
            }
            .overlay {
                Image(systemName: symbol)
                    .font(.system(size: size * SystemSettingsMetrics.iconGlyphRatio, weight: .semibold))
                    .symbolRenderingMode(.monochrome)
                    .foregroundStyle(.white)
            }
            .frame(width: size, height: size)
            .accessibilityHidden(true)
    }
}

// MARK: - 侧栏

/// 侧栏导航行：彩色方块 + 标题，选中时整行实心圆角高亮。
///
/// 仍然嵌在原生 `List(selection:)` 里，配合 `NativeListSelection` 关掉系统高亮；
/// 键盘方向键、VoiceOver 语义和选中滚动定位都由 List 保留。
struct SystemSettingsSidebarRow: View {
    let title: String
    let symbol: String
    let color: Color
    let selected: Bool
    var badge: String? = nil

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: SystemSettingsMetrics.rowCorner, style: .continuous)
        HStack(spacing: 9) {
            SystemSettingsIcon(symbol: symbol, color: color)
            Text(title)
                .font(.system(size: 13, weight: selected ? .semibold : .regular))
                .foregroundStyle(selected ? Color.white : Color.primary)
                .lineLimit(1)
            if let badge {
                FeatureBadge(text: badge, onAccent: selected)
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 8)
        .frame(height: SystemSettingsMetrics.rowHeight)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(selected ? AnyShapeStyle(Color.accentColor) : AnyShapeStyle(.clear), in: shape)
        .animation(Motion.snappy, value: selected)
        .contentShape(shape)
        .background(NativeListSelection())
    }
}

/// 侧栏容器共用同一层材质底，与内容区拉开层次。
struct SystemSettingsSidebarBackground: ViewModifier {
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency

    @ViewBuilder func body(content: Content) -> some View {
        if reduceTransparency {
            content.background(Color(nsColor: .windowBackgroundColor))
        } else {
            content.background(.regularMaterial)
        }
    }
}

extension View {
    func systemSettingsSidebarBackground() -> some View {
        modifier(SystemSettingsSidebarBackground())
    }
}

// MARK: - 内容区

/// 内容区底色：比侧栏亮一档，让浅灰 block 有可辨识的底。
struct SystemSettingsContentBackground: ViewModifier {
    @Environment(\.quietWorkspace) private var quiet
    @Environment(\.colorScheme) private var scheme
    func body(content: Content) -> some View {
        content.background(quiet ? QuietWorkspacePalette.canvas(scheme) : Color(nsColor: .textBackgroundColor))
    }
}

extension View {
    func systemSettingsContentBackground() -> some View {
        modifier(SystemSettingsContentBackground())
    }
}

/// 浅灰圆角 block：浅色模式比背景略深、深色模式比背景略亮。
///
/// 用相对叠色而不是 `windowBackgroundColor`/`controlBackgroundColor`：后两者是
/// 绝对语义色，深浅模式间会互换明暗关系，无法稳定表达「比背景深一档的分组」。
/// `Color.primary` 在浅色是黑、深色是白，同一个表达式自动满足双向翻转。
struct SystemSettingsBlockSurface: ViewModifier {
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    @Environment(\.quietWorkspace) private var quiet
    var cornerRadius: CGFloat = SystemSettingsMetrics.blockCorner

    func body(content: Content) -> some View {
        content
            .background(quiet ? QuietWorkspacePalette.surface(colorScheme) : Color.primary.opacity(fillOpacity),
                        in: RoundedRectangle(cornerRadius: quiet ? 12 : cornerRadius, style: .continuous))
            .overlay {
                if quiet {
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .strokeBorder(QuietWorkspacePalette.line(colorScheme), lineWidth: 0.5)
                }
            }
    }

    /// 减弱透明度时侧栏回退成实心底色，对比变弱，block 需要加深一档才看得出分组。
    private var fillOpacity: Double {
        if reduceTransparency { return colorScheme == .dark ? 0.11 : 0.07 }
        return colorScheme == .dark ? 0.075 : 0.05
    }
}

extension View {
    /// 把一个功能分组包成灰底 block。
    func systemSettingsBlock(cornerRadius: CGFloat = SystemSettingsMetrics.blockCorner) -> some View {
        modifier(SystemSettingsBlockSurface(cornerRadius: cornerRadius))
    }
}

/// block 内的次级分隔线：右侧对齐，左侧留出图标列宽。
struct SystemSettingsBlockDivider: View {
    var inset: CGFloat = 14

    var body: some View {
        Divider()
            .padding(.leading, inset)
    }
}

/// block 上方的小节标题。
struct SystemSettingsSectionLabel: View {
    let title: String
    var symbol: String? = nil

    var body: some View {
        HStack(spacing: 6) {
            if let symbol {
                Image(systemName: symbol)
                    .font(.system(size: 12, weight: .medium))
            }
            Text(title)
                .font(.system(size: 13, weight: .semibold))
        }
        .foregroundStyle(.secondary)
        .padding(.leading, 4)
    }
}

/// 设置页的分组容器：替代 macOS 原生分组表单的 `Section`。
///
/// 原生 `.formStyle(.grouped)` 在 macOS 上是「灰底 + 白卡」，与本设计的
/// 「亮底 + 灰卡」明暗相反，且卡片填充没有公开 API 可改，所以设置页统一
/// 用这个容器手写分组，行内容（Toggle / LabeledContent / Picker…）原样保留。
struct SystemSettingsFormBlock<Content: View>: View {
    @Environment(\.quietWorkspace) private var quiet
    var title: String? = nil
    var titleSymbol: String? = nil
    /// 分组下方的说明文字，对应原生 Section 的 footer。
    var footer: String? = nil
    @ViewBuilder let content: () -> Content

    init(title: String? = nil, titleSymbol: String? = nil, footer: String? = nil,
         @ViewBuilder content: @escaping () -> Content) {
        self.title = title
        self.titleSymbol = titleSymbol
        self.footer = footer
        self.content = content
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let title {
                SystemSettingsSectionLabel(title: title, symbol: titleSymbol)
            }
            VStack(alignment: .leading, spacing: quiet ? 6 : 10, content: content)
                .font(quiet ? .system(size: 13) : nil)
                .padding(14)
                .frame(maxWidth: .infinity, alignment: .leading)
                .systemSettingsBlock()
            if let footer {
                Text(footer)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.horizontal, 4)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// 设置页正文的滚动容器：统一最大宽度与留白，页面只负责列分组。
struct SystemSettingsFormPage<Content: View>: View {
    @Environment(\.quietWorkspace) private var quiet
    @ViewBuilder let content: () -> Content

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18, content: content)
                .frame(maxWidth: quiet ? .infinity : SystemSettingsMetrics.formPageMaxWidth, alignment: .leading)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, quiet ? 20 : SystemSettingsMetrics.formPageHorizontalInset)
                .padding(.bottom, SystemSettingsMetrics.formPageBottomInset)
        }
    }
}

// MARK: - 页面头部

/// 页面标题行：大标题 + 副标题，右侧可放页面级操作。
struct SystemSettingsPageHeader<Trailing: View>: View {
    @Environment(\.quietWorkspace) private var quiet
    let title: String
    let subtitle: String?
    @ViewBuilder let trailing: () -> Trailing

    init(_ title: String, subtitle: String? = nil, @ViewBuilder trailing: @escaping () -> Trailing) {
        self.title = title
        self.subtitle = subtitle
        self.trailing = trailing
    }

    var body: some View {
        HStack(alignment: .top, spacing: 16) {
            VStack(alignment: .leading, spacing: 5) {
                if !quiet {
                    Text(title).font(WorkspaceTypography.section)
                }
                if let subtitle {
                    Text(subtitle)
                        .font(WorkspaceTypography.detail)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer(minLength: 0)
            trailing()
        }
    }
}

extension SystemSettingsPageHeader where Trailing == EmptyView {
    init(_ title: String, subtitle: String? = nil) {
        self.init(title, subtitle: subtitle) { EmptyView() }
    }
}
