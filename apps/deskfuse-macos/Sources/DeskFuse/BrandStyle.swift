import AppKit
import SwiftUI

/// DeskFuse 品牌设计令牌。
///
/// 品牌图标签名的青 → 蓝 → 紫渐变是全局 accent 与卡片的唯一来源，
/// 页面不再各自手写蓝色。深浅色差异统一用 `NSColor` 动态提供方表达。
enum DeskFuseBrand {
    /// 品牌青。
    static let fusionCyan = Color(hex: 0x22C9EA)
    /// 品牌蓝，accent 的浅色模式基准。
    static let fusionBlue = Color(hex: 0x3276F3)
    /// 品牌紫。
    static let fusionViolet = Color(hex: 0x9B5CFF)

    /// 全局强调色：基于品牌蓝，深色模式提亮一档避免发闷。
    static let accent = Color(nsColor: NSColor(name: nil) { appearance in
        switch appearance.bestMatch(from: [.darkAqua, .aqua]) {
        case .darkAqua:
            return NSColor(srgbRed: 0x4E / 255.0, green: 0x89 / 255.0, blue: 0xFF / 255.0, alpha: 1)
        default:
            return NSColor(srgbRed: 0x32 / 255.0, green: 0x76 / 255.0, blue: 0xF3 / 255.0, alpha: 1)
        }
    })

    /// 品牌签名渐变：青 → 蓝 → 紫。
    static let fusionGradient = LinearGradient(
        colors: [fusionCyan, fusionBlue, fusionViolet],
        startPoint: .leading,
        endPoint: .trailing
    )

    /// 卡片圆角。
    static let cardRadius: CGFloat = 14
    /// 卡片描边：浅色模式黑、深色模式白，都是低透明度细线。
    static let cardBorder = Color(nsColor: NSColor(name: nil) { appearance in
        switch appearance.bestMatch(from: [.darkAqua, .aqua]) {
        case .darkAqua:
            return NSColor(white: 1, alpha: 0.10)
        default:
            return NSColor(white: 0, alpha: 0.08)
        }
    })
    /// 卡片阴影：极轻，只做层次提示。
    static let cardShadowColor = Color.black.opacity(0.10)
    static let cardShadowRadius: CGFloat = 8
    static let cardShadowY: CGFloat = 2
}

// MARK: - 品牌光晕

/// 品牌位图标背后的极淡 fusion 径向渐变光晕：浅色 0.18、深色 0.28，
/// 边缘自然淡出到透明，只做辨识度不抢内容。主窗口侧栏与菜单栏面板共用。
struct BrandGlow: ViewModifier {
    @Environment(\.colorScheme) private var colorScheme

    func body(content: Content) -> some View {
        content.background {
            RadialGradient(
                colors: [
                    DeskFuseBrand.fusionCyan.opacity(strength),
                    DeskFuseBrand.fusionBlue.opacity(strength * 0.6),
                    DeskFuseBrand.fusionViolet.opacity(0),
                ],
                center: .center,
                startRadius: 1,
                endRadius: 24
            )
            .blur(radius: 4)
            .padding(-8)
            .allowsHitTesting(false)
            .accessibilityHidden(true)
        }
    }

    private var strength: Double { colorScheme == .dark ? 0.28 : 0.18 }
}

// MARK: - 剪贴板卡片

extension DeskFuseBrand {
    /// 悬停态：阴影加深一档，卡片轻微上浮（y -1）。
    static let cardHoverShadowColor = Color.black.opacity(0.14)
    static let cardHoverShadowRadius: CGFloat = 12
    static let cardHoverShadowY: CGFloat = 4
    /// 选中填充与描边：卡片与列表行共用同一组。
    ///
    /// 填充浓度和描边宽度都要能在**半透明面板**上立住：菜单栏面板底下的
    /// 玻璃会透出壁纸和后面的窗口，8% 的填充叠上去几乎看不出选中。
    static let selectionFill = DeskFuseBrand.accent.opacity(0.18)
    static let selectionStrokeWidth: CGFloat = 2
    static let cardRestStrokeWidth: CGFloat = 0.5
    /// 列表行的圆角与悬停填充。
    static let listRowRadius: CGFloat = 10
    static let listRowHoverFill = DeskFuseBrand.accent.opacity(0.05)
    /// 右下角应用图标水印。
    static let watermarkOpacity: Double = 0.04
}

// MARK: - 动画

/// 共享动画曲线。所有页面动效从这里取曲线，不要各自手写 spring。
enum Motion {
    /// 交互反馈：按下、选中、开关。
    static let snappy = Animation.spring(response: 0.25, dampingFraction: 0.8)
    /// 状态落定：面板展开、内容替换。
    static let settle = Animation.spring(response: 0.4, dampingFraction: 0.85)
    /// 缓入缓出的氛围动效：背景、装饰层。
    static let gentle = Animation.spring(response: 0.55, dampingFraction: 0.9)
}

/// 包一层 `withAnimation`，用户在系统辅助功能里开了「减弱动态效果」时直接执行。
func withMotion<Result>(_ animation: Animation, _ body: () -> Result) -> Result {
    if NSWorkspace.shared.accessibilityDisplayShouldReduceMotion {
        return body()
    }
    return withAnimation(animation, body)
}

private extension Color {
    init(hex: UInt32) {
        self.init(
            .sRGB,
            red: Double((hex >> 16) & 0xFF) / 255,
            green: Double((hex >> 8) & 0xFF) / 255,
            blue: Double(hex & 0xFF) / 255,
            opacity: 1
        )
    }
}
