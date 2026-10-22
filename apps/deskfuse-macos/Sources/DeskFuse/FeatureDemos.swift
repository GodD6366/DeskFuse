import SwiftUI

// 各功能引导页的演示动画。每个演示由一个纯函数“画面”结构描述某一时刻的状态，
// 视图只负责绘制；时间轴统一用 DemoCurve，设备外观统一用 DemoScreen。

// MARK: - 键鼠共享

/// 光标从“此 Mac”右缘穿出，相邻边缘亮起，光标出现在“另一台电脑”。
struct InputCrossingDemo: View {
    var body: some View {
        DemoLoop(period: 4.2, still: 0.72) { progress in
            let frame = CrossingFrame(progress: progress, screen: DemoStage.screen, gap: DemoStage.gap)
            DemoPair(
                leftTitle: "此 Mac", rightTitle: "另一台电脑",
                leftActive: !frame.onRemote, rightActive: frame.onRemote,
                left: { DemoScreen(tint: DemoStage.localTint, active: !frame.onRemote).overlay(alignment: .trailing) { edgeGlow(frame.glow, offset: 1.5) } },
                right: { DemoScreen(tint: DemoStage.remoteTint, active: frame.onRemote).overlay(alignment: .leading) { edgeGlow(frame.glow, offset: -1.5) } },
                overlay: { DemoCursor(position: frame.cursor, opacity: frame.cursorOpacity) }
            )
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("演示：鼠标移到此 Mac 屏幕右侧边缘后，进入另一台电脑的屏幕")
    }

    /// 相邻边缘在穿越瞬间亮起，提示“边缘就是入口”。
    private func edgeGlow(_ glow: Double, offset: CGFloat) -> some View {
        Capsule()
            .fill(DFPalette.accent.opacity(glow))
            .frame(width: 3, height: DemoStage.screen.height - 24)
            .shadow(color: DFPalette.accent.opacity(glow), radius: 6)
            .offset(x: offset)
    }
}

/// 阶段：淡入 → 向右缘移动 → 穿越（边缘高亮衰减）→ 在对端减速停下 → 停留 → 淡出。
struct CrossingFrame {
    let cursor: CGPoint
    let onRemote: Bool
    let glow: Double
    let cursorOpacity: Double

    private static let crossAt = 0.48

    init(progress raw: Double, screen: CGSize, gap: CGFloat) {
        let p = DemoCurve.clamp(raw)
        let localStart = screen.width * 0.3
        let localEdge = screen.width - 2
        let remoteEdge = screen.width + gap + 2
        let remoteEnd = screen.width + gap + screen.width * 0.55
        let y = screen.height * (0.62 - 0.17 * DemoCurve.smoothstep(p / 0.78))
        if p < Self.crossAt {
            let t = DemoCurve.smoothstep(DemoCurve.segment(p, 0.1, Self.crossAt))
            cursor = CGPoint(x: DemoCurve.mix(localStart, localEdge, t), y: y)
            onRemote = false
        } else {
            let t = DemoCurve.easeOut(DemoCurve.segment(p, Self.crossAt, 0.78))
            cursor = CGPoint(x: DemoCurve.mix(remoteEdge, remoteEnd, t), y: y)
            onRemote = true
        }
        glow = p < Self.crossAt - 0.06 ? 0
            : p < Self.crossAt ? (p - (Self.crossAt - 0.06)) / 0.06
            : max(0, 1 - (p - Self.crossAt) / 0.22)
        cursorOpacity = DemoCurve.fade(p)
    }
}

/// 系统光标；SF 光标的尖端在左上角，把尖端而不是中心对准轨迹点。
private struct DemoCursor: View {
    let position: CGPoint
    let opacity: Double

    var body: some View {
        Image(systemName: "cursorarrow")
            .font(DFType.icon(.medium))
            .foregroundStyle(.primary)
            .shadow(color: DFShadow.color, radius: 2, y: 1)
            .position(x: position.x + 6, y: position.y + 9)
            .opacity(opacity)
    }
}

// MARK: - 剪贴板

/// 在此 Mac 按 ⌘C，内容飞到另一台电脑，落进剪贴板历史顶部，随后 ⌘V 粘贴。
struct ClipboardSyncDemo: View {
    private let rowWidth: CGFloat = 92
    private let rowHeight: CGFloat = 12

    var body: some View {
        DemoLoop(period: 4.6, still: 0.8) { progress in
            let frame = ClipboardFrame(progress: progress)
            let screen = DemoStage.screen
            DemoPair(
                leftTitle: "此 Mac", rightTitle: "另一台电脑",
                leftActive: !frame.onRemote, rightActive: frame.onRemote,
                left: {
                    DemoScreen(tint: DemoStage.localTint, active: !frame.onRemote) {
                        VStack(alignment: .leading, spacing: 6) {
                            ForEach(0..<3, id: \.self) { line in
                                Capsule().fill(.quaternary).frame(width: line == 2 ? 56 : 92, height: 5)
                            }
                        }
                        .offset(y: 6)
                    }
                },
                right: {
                    DemoScreen(tint: DemoStage.remoteTint, active: frame.onRemote) {
                        // 已有的历史记录在新内容落下时整体下移一行。
                        ForEach(0..<3, id: \.self) { row in
                            RoundedRectangle(cornerRadius: DFRadius.small, style: .continuous)
                                .fill(.quaternary)
                                .frame(width: rowWidth, height: rowHeight)
                                .position(x: screen.width / 2,
                                          y: frame.historyTop + CGFloat(row) * (rowHeight + 4)
                                              + (rowHeight + 4) * CGFloat(frame.landed))
                        }
                    }
                },
                overlay: {
                    DemoKeycap(label: "⌘C", pressed: frame.copy)
                        .position(x: screen.width - 22, y: screen.height - 14)
                        .opacity(frame.landed < 1 ? 1 : 0.4)
                    DemoKeycap(label: "⌘V", pressed: frame.paste)
                        .position(x: screen.width * 2 + DemoStage.gap - 22, y: screen.height - 14)
                        .opacity(frame.landed > 0 ? 1 : 0.4)
                    RoundedRectangle(cornerRadius: DFRadius.small, style: .continuous)
                        .fill(DFPalette.accent.opacity(DFOpacity.strong))
                        .overlay(RoundedRectangle(cornerRadius: DFRadius.small, style: .continuous)
                            .stroke(DFPalette.accent, lineWidth: DFMetrics.hairline * 2))
                        .frame(width: DemoCurve.mix(56, rowWidth, frame.landed), height: rowHeight)
                        .position(frame.chip)
                        .opacity(frame.chipOpacity)
                }
            )
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("演示：在此 Mac 复制的内容出现在另一台电脑的剪贴板历史中，并可直接粘贴")
    }
}

struct ClipboardFrame {
    /// 剪贴板卡片中心点（整块画布坐标）。
    let chip: CGPoint
    let chipOpacity: Double
    /// ⌘C、⌘V 按下的程度，0...1...0。
    let copy: Double
    let paste: Double
    /// 卡片落入历史列表的程度，同时驱动旧记录下移。
    let landed: Double
    let onRemote: Bool
    let historyTop: CGFloat

    init(progress raw: Double, screen: CGSize = DemoStage.screen, gap: CGFloat = DemoStage.gap) {
        let p = DemoCurve.clamp(raw)
        historyTop = screen.height * 0.36
        let start = CGPoint(x: screen.width * 0.42, y: screen.height * 0.5)
        let end = CGPoint(x: screen.width + gap + screen.width / 2, y: historyTop)
        let t = DemoCurve.smoothstep(DemoCurve.segment(p, 0.24, 0.6))
        // 沿弧线飞过两台设备之间。
        chip = CGPoint(x: DemoCurve.mix(start.x, end.x, t),
                       y: DemoCurve.mix(start.y, end.y, t) - 26 * sin(.pi * t))
        copy = DemoCurve.pulse(p, 0.08, 0.24)
        landed = DemoCurve.easeOut(DemoCurve.segment(p, 0.58, 0.7))
        paste = DemoCurve.pulse(p, 0.7, 0.86)
        onRemote = t >= 0.5
        chipOpacity = DemoCurve.fade(p) * DemoCurve.segment(p, 0.06, 0.14)
    }
}

// MARK: - 局域网传输

/// 左侧是电脑浏览器和手机浏览器：手机先发一张图片，电脑浏览器再发一份数据，
/// 两个文件依次沿局域网连线落到右侧这台电脑上。
struct FileTransferDemo: View {
    var body: some View {
        DemoLoop(period: 5.4, still: 0.85) { progress in
            let frame = FileTransferFrame(progress: progress)
            let screen = DemoStage.screen
            DemoPair(
                leftTitle: "电脑浏览器 · 手机", rightTitle: "这台电脑",
                leftActive: frame.browserActive || frame.phoneActive, rightActive: frame.receiving,
                left: {
                    HStack(spacing: FileTransferFrame.Layout.senderSpacing) {
                        DemoBrowser(style: .desktop, active: frame.browserActive) {
                            Image(systemName: "doc.text")
                        }
                        DemoBrowser(style: .phone, active: frame.phoneActive) {
                            Image(systemName: "photo")
                        }
                    }
                },
                right: {
                    DemoScreen(tint: DemoStage.localTint, active: frame.receiving) {
                        HStack(spacing: FileTransferFrame.Layout.tileSpacing) {
                            DemoReceivedTile(symbol: "photo.fill", landed: frame.image.landed)
                            DemoReceivedTile(symbol: "doc.text.fill", landed: frame.data.landed)
                        }
                        .offset(y: 4)
                    }
                },
                overlay: {
                    // 局域网连线：不经过云端，浏览器页面与电脑直连。
                    Path { path in
                        path.move(to: CGPoint(x: screen.width + 4, y: screen.height / 2))
                        path.addLine(to: CGPoint(x: screen.width + DemoStage.gap - 4, y: screen.height / 2))
                    }
                    .stroke(DFPalette.accent.opacity(0.45), style: StrokeStyle(lineWidth: 1.5, lineCap: .round, dash: [2, 4]))
                    Image(systemName: "photo.fill")
                        .font(DFType.icon(.large))
                        .foregroundStyle(DFPalette.accent)
                        .position(frame.image.position)
                        .opacity(frame.image.opacity)
                    Image(systemName: "doc.text.fill")
                        .font(DFType.icon(.large))
                        .foregroundStyle(DFPalette.accent)
                        .position(frame.data.position)
                        .opacity(frame.data.opacity)
                }
            )
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("演示：手机和电脑浏览器把图片与数据经局域网直接发送到这台电脑")
    }
}

/// 浏览器窗口示意：电脑是带三色点和地址栏的横向窗口，手机是只有地址栏的竖屏。
/// 发送方处于活动状态时描边用身份色强调。
private struct DemoBrowser<Content: View>: View {
    enum Style { case desktop, phone }

    let style: Style
    let active: Bool
    @ViewBuilder var content: () -> Content
    @Environment(\.colorScheme) private var scheme

    private var size: CGSize {
        style == .desktop ? FileTransferFrame.Layout.browser : FileTransferFrame.Layout.phone
    }

    var body: some View {
        let tint = DemoStage.remoteTint
        let shape = RoundedRectangle(cornerRadius: DFRadius.control, style: .continuous)
        VStack(spacing: 0) {
            HStack(spacing: 3) {
                if style == .desktop {
                    ForEach(0..<3, id: \.self) { _ in
                        Circle().fill(tint.opacity(active ? DFOpacity.strong : DFOpacity.faint))
                            .frame(width: 4, height: 4)
                    }
                }
                Capsule()
                    .fill(tint.opacity(active ? DFOpacity.strong : DFOpacity.faint))
                    .frame(width: style == .desktop ? 30 : 20, height: 4)
            }
            .frame(maxWidth: .infinity, alignment: style == .desktop ? .leading : .center)
            .padding(.horizontal, 6)
            .frame(height: 12)
            content()
                .font(DFType.icon(.large, weight: .light))
                .foregroundStyle(.tertiary)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .frame(width: size.width, height: size.height)
        .background(DFPalette.surface(scheme), in: shape)
        .clipShape(shape)
        .overlay(shape.stroke(tint.opacity(active ? 0.9 : 0.25), lineWidth: DFMetrics.selectionStroke))
        .animation(DFMotion.snappy, value: active)
    }
}

/// 落到这台电脑上的文件：弹入后右上角出现对勾。
private struct DemoReceivedTile: View {
    let symbol: String
    let landed: Double
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: DFRadius.control, style: .continuous)
        Image(systemName: symbol)
            .font(DFType.icon(.large))
            .foregroundStyle(DFPalette.accent)
            .frame(width: FileTransferFrame.Layout.tile, height: FileTransferFrame.Layout.tile)
            .background(DFPalette.control(scheme), in: shape)
            .overlay(alignment: .topTrailing) {
                Image(systemName: "checkmark.circle.fill")
                    .font(DFType.icon(.small))
                    .foregroundStyle(DFPalette.success)
                    .background(Circle().fill(DFPalette.surface(scheme)))
                    .offset(x: 4, y: -4)
            }
            .scaleEffect(0.7 + 0.3 * landed)
            .opacity(landed)
    }
}

/// 一次传输：文件图标从发送方飞到电脑上，落地后在电脑屏幕里留下对应的文件块。
struct FileTransferFrame {
    /// 舞台里各元素的尺寸与落点，视图和时间轴共用，保证图标正好落在文件块上。
    enum Layout {
        static let browser = CGSize(width: 84, height: 58)
        static let phone = CGSize(width: 42, height: 76)
        static let senderSpacing: CGFloat = DemoStage.screen.width - browser.width - phone.width
        static let tile: CGFloat = 40
        static let tileSpacing: CGFloat = 8
    }

    struct Leg {
        /// 飞行中的图标位置与不透明度；未出发和已落地时不可见。
        let position: CGPoint
        let opacity: Double
        /// 落地后文件块的弹入程度，0 到 1。
        let landed: Double

        init(progress p: Double, start: Double, end: Double, from: CGPoint, to: CGPoint) {
            let t = DemoCurve.smoothstep(DemoCurve.segment(p, start, end))
            position = CGPoint(x: DemoCurve.mix(from.x, to.x, t),
                               y: DemoCurve.mix(from.y, to.y, t) - 10 * sin(.pi * t))
            let edge = 0.03
            opacity = DemoCurve.fade(p)
                * DemoCurve.segment(p, start, start + edge)
                * (1 - DemoCurve.segment(p, end - edge, end))
            landed = DemoCurve.easeOut(DemoCurve.segment(p, end, end + 0.06)) * DemoCurve.fade(p)
        }
    }

    let image: Leg
    let data: Leg
    let phoneActive: Bool
    let browserActive: Bool
    let receiving: Bool

    init(progress raw: Double, screen: CGSize = DemoStage.screen, gap: CGFloat = DemoStage.gap) {
        let p = DemoCurve.clamp(raw)
        let midY = screen.height / 2
        let browserCenter = CGPoint(x: Layout.browser.width / 2, y: midY)
        let phoneCenter = CGPoint(x: screen.width - Layout.phone.width / 2, y: midY)
        let macCenterX = screen.width + gap + screen.width / 2
        let tileOffset = (Layout.tile + Layout.tileSpacing) / 2
        let tileY = midY + 4
        image = Leg(progress: p, start: 0.08, end: 0.34, from: phoneCenter,
                    to: CGPoint(x: macCenterX - tileOffset, y: tileY))
        data = Leg(progress: p, start: 0.46, end: 0.72, from: browserCenter,
                   to: CGPoint(x: macCenterX + tileOffset, y: tileY))
        phoneActive = (0.04..<0.38).contains(p)
        browserActive = (0.42..<0.76).contains(p)
        receiving = (0.2..<0.9).contains(p)
    }
}

// MARK: - 远程应用

/// 另一台电脑上的应用窗口浮起，移到此 Mac 并放大成本地窗口；原位置留下虚线轮廓。
struct RemoteWindowDemo: View {
    var body: some View {
        DemoLoop(period: 4.6, still: 0.8) { progress in
            let frame = RemoteWindowFrame(progress: progress)
            DemoPair(
                leftTitle: "此 Mac", rightTitle: "另一台电脑",
                leftActive: frame.onLocal, rightActive: !frame.onLocal,
                left: { DemoScreen(tint: DemoStage.localTint, active: frame.onLocal) },
                right: {
                    DemoScreen(tint: DemoStage.remoteTint, active: !frame.onLocal) {
                        // 应用仍运行在对端，这里只是它的画面来源。
                        RoundedRectangle(cornerRadius: DFRadius.small, style: .continuous)
                            .stroke(.tertiary, style: StrokeStyle(lineWidth: 1, dash: [3, 3]))
                            .frame(width: DemoWindow.size.width, height: DemoWindow.size.height)
                            .offset(y: 5)
                            .opacity(frame.ghost)
                    }
                },
                overlay: {
                    DemoWindow()
                        .scaleEffect(frame.scale)
                        .shadow(color: DFShadow.hoverColor, radius: 4 + 6 * frame.lift, y: 2 + 2 * frame.lift)
                        .position(frame.center)
                        .opacity(frame.opacity)
                }
            )
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("演示：另一台电脑上的应用窗口出现在此 Mac 上，像本地窗口一样使用")
    }
}

struct RemoteWindowFrame {
    let center: CGPoint
    let scale: Double
    /// 窗口离开原位时的浮起程度，用于加深阴影。
    let lift: Double
    let ghost: Double
    let opacity: Double
    let onLocal: Bool

    init(progress raw: Double, screen: CGSize = DemoStage.screen, gap: CGFloat = DemoStage.gap) {
        let p = DemoCurve.clamp(raw)
        let y = screen.height / 2 + 5
        let t = DemoCurve.smoothstep(DemoCurve.segment(p, 0.22, 0.62))
        center = CGPoint(x: DemoCurve.mix(screen.width + gap + screen.width / 2, screen.width / 2, t),
                         y: y - 18 * sin(.pi * t))
        lift = DemoCurve.pulse(p, 0.14, 0.7)
        // 落到本机后放大一档，表示获得完整的本地窗口尺寸。
        scale = 0.85 + 0.25 * DemoCurve.easeOut(DemoCurve.segment(p, 0.6, 0.74))
        ghost = DemoCurve.segment(p, 0.24, 0.4) * DemoCurve.fade(p)
        opacity = DemoCurve.fade(p)
        onLocal = t >= 0.5
    }
}

/// 示意应用窗口：标题栏三色按钮加两行内容。
private struct DemoWindow: View {
    static let size = CGSize(width: 62, height: 42)
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: DFRadius.small, style: .continuous)
        VStack(alignment: .leading, spacing: 5) {
            HStack(spacing: 3) {
                ForEach([DFPalette.danger, DFPalette.warning, DFPalette.success].indices, id: \.self) { index in
                    Circle().fill([DFPalette.danger, DFPalette.warning, DFPalette.success][index]).frame(width: 4, height: 4)
                }
            }
            Capsule().fill(.quaternary).frame(width: 44, height: 4)
            Capsule().fill(.quaternary).frame(width: 30, height: 4)
        }
        .padding(DFSpace.xs + DFSpace.xxs)
        .frame(width: Self.size.width, height: Self.size.height, alignment: .topLeading)
        .background(DFPalette.surface(scheme), in: shape)
        .overlay(shape.stroke(DFPalette.line(scheme), lineWidth: DFMetrics.hairline * 2))
    }
}

// MARK: - 麦克风共享

/// 另一台 Mac 的麦克风拾音，声波沿连线传到此 Mac，电平条随之起伏。
struct MicrophoneRelayDemo: View {
    var body: some View {
        DemoLoop(period: 2.4, still: 0.3) { progress in
            let frame = MicrophoneFrame(progress: progress)
            let screen = DemoStage.screen
            DemoPair(
                leftTitle: "另一台 Mac", rightTitle: "此 Mac",
                leftActive: true, rightActive: true,
                left: {
                    DemoScreen(tint: DemoStage.remoteTint, active: true) {
                        ZStack {
                            Circle()
                                .stroke(DemoStage.remoteTint.opacity(0.5 * (1 - frame.ring)), lineWidth: 1.5)
                                .frame(width: 28 + 26 * frame.ring, height: 28 + 26 * frame.ring)
                            Image(systemName: "mic.fill")
                                .font(DFType.icon(.medium))
                                .foregroundStyle(DemoStage.remoteTint)
                        }
                        .offset(y: 4)
                    }
                },
                right: {
                    DemoScreen(tint: DemoStage.localTint, active: true) {
                        HStack(alignment: .center, spacing: 4) {
                            ForEach(frame.levels.indices, id: \.self) { index in
                                Capsule()
                                    .fill(DFPalette.accent)
                                    .frame(width: 5, height: 6 + 30 * frame.levels[index])
                            }
                        }
                        .offset(y: 4)
                    }
                },
                overlay: {
                    ForEach(frame.waves.indices, id: \.self) { index in
                        let wave = frame.waves[index]
                        Circle()
                            .fill(DFPalette.accent)
                            .frame(width: 5, height: 5)
                            .position(x: DemoCurve.mix(screen.width * 0.72, screen.width + DemoStage.gap + screen.width * 0.25, wave),
                                      y: screen.height / 2 + 4)
                            .opacity(sin(.pi * wave))
                    }
                }
            )
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("演示：另一台 Mac 的麦克风声音实时传到此 Mac")
    }
}

struct MicrophoneFrame {
    /// 三个声波点在两台设备之间的位置，0 为来源、1 为此 Mac。
    let waves: [Double]
    /// 此 Mac 电平条高度，0...1。
    let levels: [Double]
    /// 来源麦克风的拾音圈扩散程度。
    let ring: Double

    init(progress raw: Double) {
        let p = DemoCurve.clamp(raw)
        waves = (0..<3).map { index in (p * 2 + Double(index) / 3).truncatingRemainder(dividingBy: 1) }
        levels = (0..<5).map { index in 0.25 + 0.75 * abs(sin(2 * .pi * (p * 2 + Double(index) * 0.19))) }
        ring = (p * 2).truncatingRemainder(dividingBy: 1)
    }
}

// MARK: - 鼠标自定义

/// 按下鼠标侧键，出现“切换桌面”动作提示，屏幕滑到下一个桌面。
struct MouseSideButtonDemo: View {
    var body: some View {
        DemoLoop(period: 4.0, still: 0.6) { progress in
            let frame = MouseButtonFrame(progress: progress)
            HStack(alignment: .top, spacing: DFSpace.xxl) {
                VStack(spacing: DFSpace.s) {
                    DemoMouse(press: frame.press)
                        .frame(height: DemoStage.screen.height)
                    DemoCaption(title: "侧键", active: frame.press > 0.3)
                        .frame(width: 64)
                }
                VStack(spacing: DFSpace.s) {
                    DemoScreen(tint: DemoStage.localTint, active: true) {
                        DemoDesktops(offset: frame.desktop)
                    }
                    .overlay(alignment: .bottom) {
                        Label("切换桌面", systemImage: "arrow.right")
                            .font(DFType.caption.weight(.semibold))
                            .foregroundStyle(DFPalette.accent)
                            .padding(.horizontal, DFSpace.s)
                            .padding(.vertical, DFSpace.xxs)
                            .background(.regularMaterial, in: Capsule())
                            .offset(y: -6 + 6 * (1 - frame.chip))
                            .opacity(frame.chip)
                    }
                    DemoCaption(title: "此 Mac", active: true)
                }
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("演示：按下鼠标侧键即可切换到下一个桌面")
    }
}

struct MouseButtonFrame {
    let press: Double
    let chip: Double
    /// 桌面滑动进度：0 为当前桌面，1 为下一个桌面；每轮末尾平滑回到起点。
    let desktop: Double

    init(progress raw: Double) {
        let p = DemoCurve.clamp(raw)
        press = DemoCurve.pulse(p, 0.1, 0.3)
        chip = DemoCurve.easeOut(DemoCurve.segment(p, 0.2, 0.32)) * (1 - DemoCurve.segment(p, 0.78, 0.9))
        desktop = DemoCurve.smoothstep(DemoCurve.segment(p, 0.3, 0.58)) * (1 - DemoCurve.smoothstep(DemoCurve.segment(p, 0.86, 1)))
    }
}

/// 侧视鼠标：滚轮、左右键分隔线，左侧侧键按下时染色并内陷。
private struct DemoMouse: View {
    let press: Double
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        ZStack {
            Capsule()
                .fill(DFPalette.surface(scheme))
                .overlay(Capsule().stroke(DFPalette.line(scheme), lineWidth: DFMetrics.selectionStroke))
                .frame(width: 46, height: 72)
            Rectangle().fill(DFPalette.line(scheme)).frame(width: 1, height: 22).offset(y: -25)
            Capsule().fill(.tertiary).frame(width: 4, height: 10).offset(y: -22)
            Capsule()
                .fill(DFPalette.accent.opacity(0.25 + 0.75 * press))
                .frame(width: 5, height: 16)
                .offset(x: -23 + 1.5 * press, y: -2)
                .shadow(color: DFPalette.accent.opacity(press), radius: 4)
        }
        .frame(width: 64)
    }
}

/// 屏幕里的两个桌面并排，按偏移量整体左移。
private struct DemoDesktops: View {
    let offset: Double

    var body: some View {
        let screen = DemoStage.screen
        HStack(spacing: 0) {
            desktop(windows: [CGSize(width: 54, height: 34), CGSize(width: 40, height: 26)], tint: DemoStage.localTint)
            desktop(windows: [CGSize(width: 70, height: 40)], tint: DemoStage.remoteTint)
        }
        .frame(width: screen.width, height: screen.height, alignment: .leading)
        .offset(x: -screen.width * CGFloat(offset))
    }

    private func desktop(windows: [CGSize], tint: Color) -> some View {
        HStack(spacing: DFSpace.xs + DFSpace.xxs) {
            ForEach(windows.indices, id: \.self) { index in
                RoundedRectangle(cornerRadius: DFRadius.small, style: .continuous)
                    .fill(tint.opacity(DFOpacity.soft))
                    .overlay(RoundedRectangle(cornerRadius: DFRadius.small, style: .continuous)
                        .stroke(tint.opacity(0.45), lineWidth: DFMetrics.hairline * 2))
                    .frame(width: windows[index].width, height: windows[index].height)
            }
        }
        .offset(y: 4)
        .frame(width: DemoStage.screen.width, height: DemoStage.screen.height)
    }
}
