import SwiftUI

/// 引导动画的共享时间轴：把 0...1 的循环进度映射到各阶段。
/// 画面由纯函数算出，便于测试，也保证每个功能的节奏一致。
enum DemoCurve {
    static func clamp(_ value: Double) -> Double { min(max(value, 0), 1) }

    static func smoothstep(_ value: Double) -> Double {
        let t = clamp(value)
        return t * t * (3 - 2 * t)
    }

    /// 三次缓出：快速出发、轻轻落定。
    static func easeOut(_ value: Double) -> Double {
        1 - pow(1 - clamp(value), 3)
    }

    /// `progress` 在 [start, end] 区间内的局部进度，区间外夹到 0 或 1。
    static func segment(_ progress: Double, _ start: Double, _ end: Double) -> Double {
        clamp((progress - start) / (end - start))
    }

    /// 在 [start, end] 内先升后降的脉冲（0→1→0），用于按键按下、短暂高亮。
    static func pulse(_ progress: Double, _ start: Double, _ end: Double) -> Double {
        let t = segment(progress, start, end)
        return t > 0 && t < 1 ? sin(.pi * t) : 0
    }

    /// 每轮首尾淡入淡出，循环衔接时画面不跳变。
    static func fade(_ progress: Double, edge: Double = 0.1) -> Double {
        progress < edge ? clamp(progress / edge)
            : progress > 1 - edge ? clamp((1 - progress) / edge) : 1
    }

    /// 两点间的线性插值。
    static func mix(_ from: CGFloat, _ to: CGFloat, _ t: Double) -> CGFloat {
        from + (to - from) * CGFloat(t)
    }
}

/// 循环驱动引导动画；系统开启“减弱动态效果”时停在 `still` 对应的静止画面。
/// 每帧都要重建整棵视图树，因此限制在 30fps，并在窗口被遮挡、最小化或关闭时暂停。
struct DemoLoop<Content: View>: View {
    let period: Double
    let still: Double
    @ViewBuilder let content: (Double) -> Content
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var windowVisible = false

    var body: some View {
        TimelineView(.animation(minimumInterval: 1.0 / 30, paused: reduceMotion || !windowVisible)) { context in
            content(reduceMotion ? still
                : context.date.timeIntervalSinceReferenceDate.truncatingRemainder(dividingBy: period) / period)
        }
        .padding(.vertical, DFSpace.s)
        .background(WindowVisibilityObserver { windowVisible = $0 })
    }
}

/// 引导动画里的设备尺寸与配色，所有演示共用，读起来是同一套插画。
enum DemoStage {
    static let screen = CGSize(width: 132, height: 84)
    /// 两台设备之间的距离，所有演示保持一致。
    static let gap: CGFloat = 40
    static let localTint = DFPalette.accent
    static let remoteTint = DFPalette.identity[1]
}

/// 示意屏幕：顶部菜单栏条加描边；当前活动的设备用其身份色强调。
struct DemoScreen<Content: View>: View {
    let tint: Color
    let active: Bool
    @ViewBuilder var content: () -> Content
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: DFRadius.control, style: .continuous)
        ZStack(alignment: .top) {
            content()
                .frame(width: DemoStage.screen.width, height: DemoStage.screen.height)
            Rectangle()
                .fill(tint.opacity(active ? DFOpacity.strong : DFOpacity.faint))
                .frame(height: 8)
        }
        // 内容为空时 ZStack 只剩菜单栏条，必须顶对齐，否则会被居中到屏幕中间。
        .frame(width: DemoStage.screen.width, height: DemoStage.screen.height, alignment: .top)
        .background(DFPalette.surface(scheme), in: shape)
        .clipShape(shape)
        .overlay(shape.stroke(tint.opacity(active ? 0.9 : 0.25), lineWidth: DFMetrics.selectionStroke))
        .animation(DFMotion.snappy, value: active)
    }
}

extension DemoScreen where Content == EmptyView {
    init(tint: Color, active: Bool) {
        self.init(tint: tint, active: active) { EmptyView() }
    }
}

/// 屏幕下方的设备名；当前活动的设备加粗。
struct DemoCaption: View {
    let title: String
    let active: Bool
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        Text(title)
            .font(DFType.caption.weight(active ? .semibold : .regular))
            .foregroundStyle(active ? AnyShapeStyle(.primary) : AnyShapeStyle(DFPalette.secondaryText(scheme)))
            .frame(width: DemoStage.screen.width)
            .animation(DFMotion.snappy, value: active)
    }
}

/// 两台设备并排的舞台：上方屏幕、下方设备名，`overlay` 在整块画布坐标系里绘制移动元素。
struct DemoPair<Left: View, Right: View, Overlay: View>: View {
    var gap: CGFloat = DemoStage.gap
    let leftTitle: String
    let rightTitle: String
    let leftActive: Bool
    let rightActive: Bool
    @ViewBuilder let left: () -> Left
    @ViewBuilder let right: () -> Right
    @ViewBuilder let overlay: () -> Overlay

    var body: some View {
        VStack(spacing: DFSpace.s) {
            ZStack(alignment: .topLeading) {
                HStack(spacing: gap) {
                    left()
                    right()
                }
                overlay()
            }
            .frame(width: DemoStage.screen.width * 2 + gap, height: DemoStage.screen.height)
            HStack(spacing: gap) {
                DemoCaption(title: leftTitle, active: leftActive)
                DemoCaption(title: rightTitle, active: rightActive)
            }
        }
    }
}

/// 键帽徽标（⌘C、⌘V 等），按下时放大并染上强调色。
struct DemoKeycap: View {
    let label: String
    let pressed: Double
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        Text(label)
            .font(DFType.caption.weight(.semibold))
            .foregroundStyle(pressed > 0.5 ? AnyShapeStyle(DFPalette.accent) : AnyShapeStyle(.secondary))
            .padding(.horizontal, DFSpace.xs + DFSpace.xxs)
            .padding(.vertical, DFSpace.xxs)
            .background(DFPalette.control(scheme),
                        in: RoundedRectangle(cornerRadius: DFRadius.small, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: DFRadius.small, style: .continuous)
                .stroke(DFPalette.accent.opacity(pressed), lineWidth: DFMetrics.hairline * 2))
            .scaleEffect(1 + 0.12 * pressed)
    }
}
