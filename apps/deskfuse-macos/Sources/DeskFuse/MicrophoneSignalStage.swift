import SwiftUI

/// 麦克风链路的动态状态：决定信号舞台用哪种动效。
enum MicrophoneSignalMode: Equatable {
    /// 共享关闭：静止虚线。
    case off
    /// 已开启但还在等待（连接中、等待上线）：虚线缓慢流动。
    case waiting
    /// 需要处理（权限、驱动）：语义色静止虚线，不做动效以免分散注意。
    case attention
    /// 已就绪按需连接：单个光点周期性穿过链路，两端节点呼吸。
    case ready
    /// 正在传输：随电平起伏的声波、流动粒子与扩散声波环。
    case streaming

    var animates: Bool {
        switch self {
        case .waiting, .ready, .streaming: true
        case .off, .attention: false
        }
    }
}

/// 链路一端的展示信息。
struct MicrophoneSignalEndpoint {
    let symbol: String
    let title: String
    let subtitle: String
}

/// 麦克风共享的主视觉：来源节点 → 动态信号链路 → 目标节点，中间是状态胶囊。
/// 只反映真实状态与真实电平；系统开启“减弱动态效果”时退化为静态绘制。
struct MicrophoneSignalStage: View {
    let source: MicrophoneSignalEndpoint
    let destination: MicrophoneSignalEndpoint
    let mode: MicrophoneSignalMode
    let tone: DFTone
    let label: String
    /// 0...1 的真实采样电平，仅在传输中使用。
    let level: Float
    var sourceMenu: AnyView? = nil
    @State private var showingSourcePicker = false

    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var paused: Bool { reduceMotion || !mode.animates }

    var body: some View {
        TimelineView(.animation(minimumInterval: 1.0 / 60, paused: paused)) { context in
            let time = paused ? 0 : context.date.timeIntervalSinceReferenceDate
            HStack(alignment: .top, spacing: DFSpace.s) {
                if let sourceMenu {
                    Button {
                        showingSourcePicker = true
                    } label: {
                        MicrophoneSignalNode(endpoint: source, emits: true, mode: mode, tone: tone,
                                             level: level, time: time)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .popover(isPresented: $showingSourcePicker, arrowEdge: .bottom) {
                        VStack(alignment: .leading, spacing: DFSpace.m) {
                            Text("选择麦克风").font(DFType.title)
                            sourceMenu
                        }
                        .padding(DFSpace.l)
                        .frame(minWidth: 240)
                    }
                    .help("点击选择麦克风")
                    .accessibilityLabel("选择麦克风，当前为\(source.title)")
                } else {
                    MicrophoneSignalNode(endpoint: source, emits: true, mode: mode, tone: tone,
                                         level: level, time: time)
                }
                VStack(spacing: DFSpace.s) {
                    MicrophoneSignalTrack(mode: mode, tone: tone, level: level, time: time)
                        .frame(height: MicrophoneSignalNode.diameter)
                    statusPill
                }
                .frame(maxWidth: .infinity)
                MicrophoneSignalNode(endpoint: destination, emits: false, mode: mode, tone: tone,
                                     level: level, time: time)
            }
        }
        .animation(DFMotion.settle, value: mode)
        .accessibilityElement(children: .contain)
        .accessibilityLabel("\(source.title) 到 \(destination.title)，\(label)")
    }

    private var statusPill: some View {
        VStack(spacing: DFSpace.xs) {
            Text(label)
                .font(DFType.caption.weight(.semibold))
                .foregroundStyle(tone.color)
                .contentTransition(.opacity)
                .padding(.horizontal, DFSpace.s + 2)
                .padding(.vertical, DFSpace.xs)
                .background(tone.color.opacity(DFOpacity.soft), in: Capsule())
                .animation(DFMotion.snappy, value: label)
            Text("音频不保存").font(DFType.caption).dfSecondaryText()
        }
    }
}

/// 链路端点：圆形图标、副标题与标题。来源端在传输时向外扩散声波环。
private struct MicrophoneSignalNode: View {
    static let diameter: CGFloat = 64
    static let width: CGFloat = 140

    let endpoint: MicrophoneSignalEndpoint
    let emits: Bool
    let mode: MicrophoneSignalMode
    let tone: DFTone
    let level: Float
    let time: TimeInterval
    @Environment(\.colorScheme) private var scheme

    private var active: Bool { mode == .streaming || mode == .ready }

    var body: some View {
        VStack(spacing: DFSpace.s) {
            ZStack {
                Circle()
                    .fill(active ? tone.color.opacity(DFOpacity.soft) : DFPalette.control(scheme))
                Circle()
                    .strokeBorder(active ? tone.color.opacity(DFOpacity.strong * 2) : DFPalette.line(scheme),
                                  lineWidth: active ? DFMetrics.selectionStroke : DFMetrics.hairline)
                Image(systemName: endpoint.symbol)
                    .font(DFType.icon(.large))
                    .foregroundStyle(active ? tone.color : Color.secondary)
                    .symbolEffect(.pulse, options: .repeating, isActive: mode == .streaming)
            }
            .frame(width: Self.diameter, height: Self.diameter)
            // 声波环画在节点背后并允许溢出，不参与节点尺寸计算。
            .background { rings }
            .scaleEffect(breathing)
            VStack(spacing: DFSpace.xxs) {
                Text(endpoint.subtitle).font(DFType.caption).dfSecondaryText()
                Text(endpoint.title).font(DFType.bodyMedium)
                    .multilineTextAlignment(.center)
                    .lineLimit(2)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .frame(width: Self.width)
    }

    /// 就绪时两端轻微呼吸；目标端相位错开半拍，形成“等待对端”的节奏。
    private var breathing: CGFloat {
        guard mode == .ready else { return 1 }
        return 1 + 0.035 * CGFloat(sin(time * 2.2 + (emits ? 0 : .pi)))
    }

    /// 来源端声波环：三道环依次扩散，半径随电平放大。
    private var rings: some View {
        Canvas { context, size in
            guard emits, mode == .streaming || mode == .ready else { return }
            let center = CGPoint(x: size.width / 2, y: size.height / 2)
            let base = Self.diameter / 2
            let count = mode == .streaming ? 3 : 1
            let speed = mode == .streaming ? 0.75 : 0.35
            let reach = mode == .streaming ? 14 + CGFloat(min(1, max(0, level))) * 22 : 12
            for index in 0..<count {
                let phase = (time * speed + Double(index) / Double(count)).truncatingRemainder(dividingBy: 1)
                let radius = base + CGFloat(phase) * reach
                let rect = CGRect(x: center.x - radius, y: center.y - radius, width: radius * 2, height: radius * 2)
                context.stroke(Path(ellipseIn: rect),
                               with: .color(tone.color.opacity((1 - phase) * 0.45)),
                               lineWidth: 1.5)
            }
        }
        // 声波环允许画到节点之外。
        .frame(width: Self.diameter * 2, height: Self.diameter * 2)
        .allowsHitTesting(false)
    }
}

/// 两端之间的信号链路。
private struct MicrophoneSignalTrack: View {
    let mode: MicrophoneSignalMode
    let tone: DFTone
    let level: Float
    let time: TimeInterval
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        Canvas { context, size in
            let midY = size.height / 2
            let width = size.width
            guard width > 0 else { return }
            switch mode {
            case .off:
                dashed(context, width: width, midY: midY, color: DFPalette.line(scheme), phase: 0)
            case .attention:
                dashed(context, width: width, midY: midY, color: tone.color.opacity(0.5), phase: 0)
            case .waiting:
                dashed(context, width: width, midY: midY, color: tone.color.opacity(0.45), phase: -time * 14)
            case .ready:
                solid(context, width: width, midY: midY, color: tone.color.opacity(0.22))
                pulse(context, width: width, midY: midY)
            case .streaming:
                wave(context, width: width, midY: midY)
            }
        }
        .allowsHitTesting(false)
    }

    private func dashed(_ context: GraphicsContext, width: CGFloat, midY: CGFloat, color: Color, phase: Double) {
        var path = Path()
        path.move(to: CGPoint(x: 0, y: midY))
        path.addLine(to: CGPoint(x: width, y: midY))
        context.stroke(path, with: .color(color),
                       style: StrokeStyle(lineWidth: 1.5, lineCap: .round, dash: [4, 6], dashPhase: phase))
    }

    private func solid(_ context: GraphicsContext, width: CGFloat, midY: CGFloat, color: Color) {
        var path = Path()
        path.move(to: CGPoint(x: 0, y: midY))
        path.addLine(to: CGPoint(x: width, y: midY))
        context.stroke(path, with: .color(color), style: StrokeStyle(lineWidth: 1.5, lineCap: .round))
    }

    /// 就绪：一颗带拖尾的光点每 2.6 秒穿过链路一次。
    private func pulse(_ context: GraphicsContext, width: CGFloat, midY: CGFloat) {
        let progress = (time / 2.6).truncatingRemainder(dividingBy: 1)
        let x = CGFloat(progress) * width
        let alpha = sin(progress * .pi)
        let tail: CGFloat = min(56, width / 3)
        var trail = Path()
        trail.move(to: CGPoint(x: max(0, x - tail), y: midY))
        trail.addLine(to: CGPoint(x: x, y: midY))
        context.stroke(trail,
                       with: .linearGradient(Gradient(colors: [tone.color.opacity(0), tone.color.opacity(alpha)]),
                                             startPoint: CGPoint(x: x - tail, y: midY),
                                             endPoint: CGPoint(x: x, y: midY)),
                       style: StrokeStyle(lineWidth: 2, lineCap: .round))
        dot(context, at: CGPoint(x: x, y: midY), radius: 3.5, alpha: alpha)
    }

    /// 传输：中间高两端低的正弦声波，振幅跟随真实电平；粒子沿声波向目标端流动。
    private func wave(_ context: GraphicsContext, width: CGFloat, midY: CGFloat) {
        let amplitude = 3 + CGFloat(min(1, sqrt(max(0, Double(level))))) * 16
        func y(at x: CGFloat) -> CGFloat {
            let envelope = sin(Double(x / width) * .pi)
            return midY + CGFloat(envelope) * amplitude * CGFloat(sin(Double(x) * 0.09 - time * 7))
        }
        var path = Path()
        path.move(to: CGPoint(x: 0, y: y(at: 0)))
        for x in stride(from: CGFloat(2), through: width, by: 2) {
            path.addLine(to: CGPoint(x: x, y: y(at: x)))
        }
        context.stroke(path,
                       with: .linearGradient(Gradient(colors: [tone.color.opacity(0.35), tone.color, tone.color.opacity(0.35)]),
                                             startPoint: CGPoint(x: 0, y: midY),
                                             endPoint: CGPoint(x: width, y: midY)),
                       style: StrokeStyle(lineWidth: 2, lineCap: .round, lineJoin: .round))
        let particles = 6
        for index in 0..<particles {
            let progress = (time * 0.42 + Double(index) / Double(particles)).truncatingRemainder(dividingBy: 1)
            let x = CGFloat(progress) * width
            dot(context, at: CGPoint(x: x, y: y(at: x)), radius: 2.5, alpha: sin(progress * .pi))
        }
    }

    private func dot(_ context: GraphicsContext, at point: CGPoint, radius: CGFloat, alpha: Double) {
        let glow = CGRect(x: point.x - radius * 2.5, y: point.y - radius * 2.5, width: radius * 5, height: radius * 5)
        context.fill(Path(ellipseIn: glow), with: .color(tone.color.opacity(alpha * 0.18)))
        let core = CGRect(x: point.x - radius, y: point.y - radius, width: radius * 2, height: radius * 2)
        context.fill(Path(ellipseIn: core), with: .color(tone.color.opacity(alpha)))
    }
}
