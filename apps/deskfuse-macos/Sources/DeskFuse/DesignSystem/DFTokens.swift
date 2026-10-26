import AppKit
import SwiftUI

// DeskFuse 唯一的设计令牌来源。页面只引用这里的字号、圆角、间距、颜色和动效，
// 不再手写字面量；`scripts/check-ui-tokens.sh` 会拦截目录外的硬编码样式。

/// 字号层级：页面标题 > 卡片标题 > 正文 / 分区标签 > 辅助说明 > 注释。
enum DFType {
    /// 每个页面唯一的标题。
    static let pageTitle = Font.system(size: 20, weight: .semibold)
    /// 卡片、面板与弹窗标题。
    static let title = Font.system(size: 15, weight: .semibold)
    /// 卡片上方的分区标签，配合次要色使用。
    static let section = Font.system(size: 13, weight: .semibold)
    static let body = Font.system(size: 13)
    static let bodyMedium = Font.system(size: 13, weight: .medium)
    /// 设置项说明、元数据等辅助文字。
    static let detail = Font.system(size: 12)
    /// 徽标、键帽、画布内标签等紧凑注释。
    static let caption = Font.system(size: 11)
    /// 快捷键键帽：紧凑、圆角引擎，与其他文字不一致是有意的。
    static let keycap = Font.system(size: 13, weight: .medium, design: .rounded)
    /// 地址、路径等需要逐字符阅读的内容。
    static let mono = Font.system(size: 12, design: .monospaced)
    /// 指标大数字，等宽数字避免跳动。
    static let metric = Font.system(size: 24, weight: .semibold).monospacedDigit()

    static func icon(_ size: DFIconSize, weight: Font.Weight = .regular) -> Font {
        .system(size: size.rawValue, weight: weight)
    }

    /// 随容器缩放的图标（如设备型号图标），尺寸由调用方按容器计算。
    static func scaledIcon(_ size: CGFloat, weight: Font.Weight = .regular) -> Font {
        .system(size: size, weight: weight)
    }
}

/// 图标尺寸档位：行内、导航、强调、空态插图。
enum DFIconSize: CGFloat {
    case small = 12
    case regular = 14
    /// 圆框符号（ellipsis.circle 等）与同排 14pt 的非圆框图标视觉等大。
    case circled = 16
    case medium = 18
    case large = 24
    case hero = 40
}

/// 圆角只有四档：细条、控件、卡片、浮层。
enum DFRadius {
    static let small: CGFloat = 4
    static let control: CGFloat = 8
    static let card: CGFloat = 12
    static let panel: CGFloat = 16
}

/// 间距按 4 的倍数递增。
enum DFSpace {
    static let xxs: CGFloat = 2
    static let xs: CGFloat = 4
    static let s: CGFloat = 8
    static let m: CGFloat = 12
    static let l: CGFloat = 16
    static let xl: CGFloat = 24
    static let xxl: CGFloat = 32
}

enum DFMetrics {
    /// 常规按钮与输入框高度；浮动面板内使用紧凑高度。
    static let controlHeight: CGFloat = 32
    static let compactControlHeight: CGFloat = 28
    static let controlInset: CGFloat = 12
    static let iconButton: CGFloat = 28
    /// 导航与列表行。
    static let rowHeight: CGFloat = 32
    static let sidebarWidth: CGFloat = 212
    /// 隐藏标题栏的高度与红绿灯右缘：品牌位与面包屑在这一行内与红绿灯垂直居中对齐。
    static let titlebarHeight: CGFloat = 52
    static let trafficLightsInset: CGFloat = 92
    /// 页面正文左右留白与表单最大宽度。
    static let pageInset: CGFloat = 24
    static let pageTopInset: CGFloat = 4
    static let pageBottomInset: CGFloat = 24
    static let formMaxWidth: CGFloat = 720
    /// 自适应双栏：侧栏固定宽度，主栏低于最小宽度时改为上下堆叠。
    static let sideColumnWidth: CGFloat = 340
    static let mainColumnMinWidth: CGFloat = 480
    /// 内容较多的侧栏（如麦克风设置步骤）。
    static let wideSideColumnWidth: CGFloat = 400
    /// 单任务聚焦页（身份选择、加载、故障）的最大宽度，水平居中。
    static let focusMaxWidth: CGFloat = 880
    static let cardPadding: CGFloat = 16
    static let hairline: CGFloat = 0.5
    static let selectionStroke: CGFloat = 1.5
}

/// 透明度三档，叠加在语义色上表达填充深浅。
enum DFOpacity {
    static let faint = 0.06
    static let soft = 0.12
    static let strong = 0.18
    /// 剪贴板卡片右下角来源应用水印。
    static let watermark = 0.07
}

/// 卡片阴影只做层次提示：静止极轻，悬停加深一档。
enum DFShadow {
    static let color = Color.black.opacity(0.08)
    static let radius: CGFloat = 8
    static let y: CGFloat = 2
    static let hoverColor = Color.black.opacity(0.12)
    static let hoverRadius: CGFloat = 12
    static let hoverY: CGFloat = 4
    /// 侧栏选中行：浮起一层的贴边阴影，不靠放大字号表达选中。
    static let rowColor = Color.black.opacity(0.07)
    static let rowRadius: CGFloat = 2
    static let rowY: CGFloat = 1
}

/// 语义色调：状态胶囊、提示条、徽标统一从这里取色。
enum DFTone {
    case neutral, accent, success, warning, danger, beta

    var color: Color {
        switch self {
        case .neutral: Color.secondary
        case .accent: DFPalette.accent
        case .success: DFPalette.success
        case .warning: DFPalette.warning
        case .danger: DFPalette.danger
        case .beta: DFPalette.beta
        }
    }
}

enum DFPalette {
    /// 品牌蓝强调色：深色模式提亮一档避免发闷。
    static let accent = Color(nsColor: NSColor(name: nil) { appearance in
        switch appearance.bestMatch(from: [.darkAqua, .aqua]) {
        case .darkAqua:
            NSColor(srgbRed: 0x4E / 255.0, green: 0x89 / 255.0, blue: 0xFF / 255.0, alpha: 1)
        default:
            NSColor(srgbRed: 0x32 / 255.0, green: 0x76 / 255.0, blue: 0xF3 / 255.0, alpha: 1)
        }
    })
    static let success = Color(nsColor: .systemGreen)
    static let warning = Color(nsColor: .systemOrange)
    static let danger = Color(nsColor: .systemRed)
    static let beta = Color(nsColor: .systemOrange)

    /// 内容选中与悬停填充：列表行、卡片、分组共用。
    static let selection = accent.opacity(DFOpacity.strong)
    static let hover = accent.opacity(DFOpacity.faint)

    /// 品牌签名色，仅用于品牌位与配对确认等少量强调场景。
    static let brandCyan = Color(nsColor: NSColor(srgbRed: 0x22 / 255.0, green: 0xC9 / 255.0, blue: 0xEA / 255.0, alpha: 1))
    static let brandViolet = Color(nsColor: NSColor(srgbRed: 0x9B / 255.0, green: 0x5C / 255.0, blue: 0xFF / 255.0, alpha: 1))
    static let brandGradient = LinearGradient(colors: [brandCyan, accent, brandViolet],
                                              startPoint: .leading, endPoint: .trailing)

    /// 键鼠画布按设备身份区分屏幕的配色，按稳定顺序循环使用。
    static let identity: [Color] = [
        accent,
        Color(nsColor: .systemPurple),
        Color(nsColor: .systemTeal),
        Color(nsColor: .systemOrange),
        Color(nsColor: .systemPink),
        Color(nsColor: .systemIndigo),
        Color(nsColor: .systemGreen),
        Color(nsColor: .systemBrown),
    ]

    /// 不依赖环境配色方案的细描边，用于浮动面板内的卡片。
    static let border = Color(nsColor: NSColor(name: nil) { appearance in
        switch appearance.bestMatch(from: [.darkAqua, .aqua]) {
        case .darkAqua: NSColor(white: 1, alpha: 0.10)
        default: NSColor(white: 0, alpha: 0.08)
        }
    })

    /// 主窗口画布、实心阅读表面、控件底、分隔线与次要文字。
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
    /// 浅/深画布上的辅助文字，对比度高于系统 secondary。
    static func secondaryText(_ scheme: ColorScheme) -> Color {
        Color(white: scheme == .dark ? 0.75 : 0.35)
    }
}

/// 共享动画曲线；页面不手写 spring。
enum DFMotion {
    /// 交互反馈：按下、选中、开关。
    static let snappy = Animation.spring(response: 0.25, dampingFraction: 0.8)
    /// 状态落定：面板展开、内容替换。
    static let settle = Animation.spring(response: 0.4, dampingFraction: 0.85)
    /// 持续遥测跟随 1 秒采样节奏，不回弹。
    static let telemetryDuration: Double = 0.9
}

/// 包一层 `withAnimation`；系统开启「减弱动态效果」时直接执行。
func withMotion<Result>(_ animation: Animation, _ body: () -> Result) -> Result {
    if NSWorkspace.shared.accessibilityDisplayShouldReduceMotion {
        return body()
    }
    return withAnimation(animation, body)
}
