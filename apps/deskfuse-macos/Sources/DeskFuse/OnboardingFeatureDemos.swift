import SwiftUI

/// 功能卡片里的演示动画。所有动画按同一套相位计算，循环播放；
/// 系统开启「减弱动态效果」时停在有代表性的静止帧。
struct OnboardingFeatureDemo: View {
    let kind: OnboardingFeatureKind

    var body: some View {
        switch kind {
        case .clipboard: ClipboardTransferDemo()
        case .input: InputSharingDemo()
        case .remoteApplications: RemoteApplicationDemo()
        case .mouseButtons: MouseButtonDemo()
        case .microphone: MicrophoneDemo()
        }
    }
}

// MARK: - 通用舞台

private struct OnboardingDemoStage<Content: View>: View {
    var duration: Double
    var offset: Double
    var restingPhase: Double
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @ViewBuilder var content: (Double, CGSize) -> Content

    var body: some View {
        GeometryReader { proxy in
            TimelineView(.animation(minimumInterval: 1.0 / 30.0, paused: reduceMotion)) { timeline in
                content(phase(at: timeline.date), proxy.size)
            }
        }
        .accessibilityHidden(true)
    }

    private func phase(at date: Date) -> Double {
        if reduceMotion { return restingPhase }
        let cycles = (date.timeIntervalSinceReferenceDate + offset) / duration
        return cycles - cycles.rounded(.down)
    }
}

/// 设备卡片：一枚图标加一行说明，用于剪贴板的两端。
private struct OnboardingDemoDevice: View {
    let symbol: String
    let label: String
    var width: CGFloat = 86
    var active = false
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        VStack(spacing: DFSpace.xs) {
            RoundedRectangle(cornerRadius: DFRadius.control, style: .continuous)
                .fill(DFPalette.surface(scheme))
                .frame(width: width, height: width * 0.62)
                .overlay {
                    Image(systemName: symbol)
                        .font(DFType.icon(.large, weight: .light))
                        .foregroundStyle(active ? DFPalette.accent : Color.secondary)
                }
                .overlay {
                    RoundedRectangle(cornerRadius: DFRadius.control, style: .continuous)
                        .strokeBorder(active ? DFPalette.accent.opacity(0.55) : DFPalette.line(scheme),
                                      lineWidth: active ? DFMetrics.selectionStroke : DFMetrics.hairline)
                }
                .shadow(color: active ? DFPalette.accent.opacity(DFOpacity.soft) : .clear, radius: 8)
            Text(label)
                .font(DFType.caption)
                .dfSecondaryText()
                .lineLimit(1)
        }
        .frame(width: width)
    }
}

/// 显示器卡片：可叠加光标、窗口等内容。
private struct OnboardingDemoMonitor<Overlay: View>: View {
    let width: CGFloat
    let height: CGFloat
    let label: String
    var active = false
    @Environment(\.colorScheme) private var scheme
    @ViewBuilder var overlay: () -> Overlay

    var body: some View {
        VStack(spacing: DFSpace.xs) {
            ZStack {
                RoundedRectangle(cornerRadius: DFRadius.small, style: .continuous)
                    .fill(DFPalette.surface(scheme))
                overlay()
            }
            .frame(width: width, height: height)
            .overlay {
                RoundedRectangle(cornerRadius: DFRadius.small, style: .continuous)
                    .strokeBorder(active ? DFPalette.accent.opacity(0.55) : DFPalette.line(scheme),
                                  lineWidth: active ? DFMetrics.selectionStroke : DFMetrics.hairline)
            }
            .shadow(color: active ? DFPalette.accent.opacity(DFOpacity.soft) : .clear, radius: 10)
            Text(label)
                .font(DFType.caption)
                .dfSecondaryText()
                .lineLimit(1)
        }
    }
}

private func demoEase(_ value: Double) -> Double {
    let clamped = min(max(value, 0), 1)
    return clamped * clamped * (3 - 2 * clamped)
}

private func demoWindow(_ phase: Double, from start: Double, to end: Double) -> Double {
    guard end > start else { return phase >= end ? 1 : 0 }
    return demoEase((phase - start) / (end - start))
}

/// 以 center 为中心的循环脉冲，考虑相位在 0/1 处回绕。
private func demoPulse(_ phase: Double, center: Double, width: Double) -> Double {
    let distance = abs(phase - center)
    let wrapped = min(distance, 1 - distance)
    return max(0, 1 - wrapped / max(width, 0.001))
}

// MARK: - 剪贴板

private struct ClipboardTransferDemo: View {
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        OnboardingDemoStage(duration: 5.2, offset: 0.3, restingPhase: 0.72) { phase, size in
            let deviceWidth = min(84, size.width * 0.25)
            let leftX = size.width * 0.18
            let rightX = size.width * 0.82
            let centerY = size.height * 0.56
            let travel = demoWindow(phase, from: 0.24, to: 0.58)
            let copyPulse = demoPulse(phase, center: 0.12, width: 0.11)
            let landed = phase > 0.6 && phase < 0.9
            let chipOpacity = phase < 0.1
                ? 0
                : phase < 0.2 ? demoWindow(phase, from: 0.1, to: 0.2)
                : phase > 0.82 ? 1 - demoWindow(phase, from: 0.82, to: 0.92)
                : 1

            ZStack {
                Path { path in
                    path.move(to: CGPoint(x: leftX + deviceWidth * 0.55, y: centerY - 8))
                    path.addLine(to: CGPoint(x: rightX - deviceWidth * 0.55, y: centerY - 8))
                }
                .stroke(DFPalette.accent.opacity(0.28),
                        style: StrokeStyle(lineWidth: DFMetrics.hairline * 2, lineCap: .round, dash: [3, 5]))

                OnboardingDemoDevice(symbol: "laptopcomputer", label: "这台 Mac",
                                     width: deviceWidth, active: copyPulse > 0.15)
                    .position(x: leftX, y: centerY)

                OnboardingDemoDevice(symbol: "laptopcomputer", label: "另一台 Mac",
                                     width: deviceWidth, active: landed)
                    .position(x: rightX, y: centerY)

                Text("⌘C")
                    .font(DFType.keycap)
                    .foregroundStyle(DFPalette.accent)
                    .padding(.horizontal, DFSpace.s)
                    .padding(.vertical, DFSpace.xxs + 1)
                    .background(DFPalette.surface(scheme), in: Capsule())
                    .overlay { Capsule().strokeBorder(DFPalette.accent.opacity(0.4), lineWidth: DFMetrics.hairline) }
                    .position(x: leftX, y: centerY - deviceWidth * 0.42)
                    .opacity(copyPulse)
                    .scaleEffect(CGFloat(0.85 + copyPulse * 0.15))

                HStack(spacing: DFSpace.xs) {
                    Image(systemName: "doc.text.fill")
                        .font(DFType.caption)
                        .foregroundStyle(DFPalette.accent)
                    Text("周报草稿")
                        .font(DFType.caption)
                        .lineLimit(1)
                }
                .padding(.horizontal, DFSpace.s)
                .padding(.vertical, DFSpace.xs)
                .background(DFPalette.surface(scheme), in: Capsule())
                .overlay { Capsule().strokeBorder(DFPalette.accent.opacity(0.45), lineWidth: DFMetrics.hairline) }
                .shadow(color: DFPalette.accent.opacity(DFOpacity.strong), radius: 8, y: 2)
                .position(x: leftX + (rightX - leftX) * CGFloat(travel),
                          y: centerY - 10 - CGFloat(sin(travel * .pi) * 8))
                .opacity(chipOpacity)

                Image(systemName: "checkmark.circle.fill")
                    .font(DFType.icon(.medium))
                    .foregroundStyle(DFPalette.success)
                    .background(Circle().fill(DFPalette.surface(scheme)))
                    .position(x: rightX + deviceWidth * 0.4, y: centerY - deviceWidth * 0.28)
                    .opacity(landed ? 1 : 0)
                    .scaleEffect(landed ? 1 : 0.6)
            }
        }
    }
}

// MARK: - 键鼠共享

private struct InputSharingDemo: View {
    var body: some View {
        OnboardingDemoStage(duration: 6.0, offset: 1.4, restingPhase: 0.68) { phase, size in
            let width = min(112, size.width * 0.31)
            let height = width * 0.6
            let leftX = size.width * 0.24
            let rightX = size.width * 0.76
            let centerY = size.height * 0.45
            let travel = demoWindow(phase, from: 0.1, to: 0.72)
            let cursorX = leftX + (rightX - leftX) * CGFloat(travel)
            let cursorY = centerY - 6 + CGFloat(sin(travel * .pi * 1.6) * 6)
            let seam = demoPulse(phase, center: 0.44, width: 0.1)
            let fading = phase > 0.9 ? 1 - demoWindow(phase, from: 0.9, to: 0.98) : 1

            ZStack {
                Path { path in
                    path.move(to: CGPoint(x: size.width * 0.5, y: centerY - height * 0.55))
                    path.addLine(to: CGPoint(x: size.width * 0.5, y: centerY + height * 0.45))
                }
                .stroke(DFPalette.accent.opacity(0.22),
                        style: StrokeStyle(lineWidth: DFMetrics.hairline * 2, dash: [2, 5]))

                Circle()
                    .stroke(DFPalette.accent.opacity(0.45), lineWidth: DFMetrics.selectionStroke)
                    .frame(width: CGFloat(26 + 18 * seam), height: CGFloat(26 + 18 * seam))
                    .position(x: size.width * 0.5, y: centerY - 5)
                    .opacity(seam * 0.8)

                OnboardingDemoMonitor(width: width, height: height, label: "这台 Mac",
                                      active: travel > 0.32 && travel < 0.78) {
                    OnboardingDemoDesktop()
                }
                .position(x: leftX, y: centerY)

                OnboardingDemoMonitor(width: width, height: height, label: "另一台 Mac",
                                      active: travel > 0.55) {
                    OnboardingDemoDesktop()
                }
                .position(x: rightX, y: centerY)

                Image(systemName: "cursorarrow")
                    .font(DFType.icon(.medium, weight: .semibold))
                    .foregroundStyle(.primary)
                    .shadow(color: .black.opacity(0.28), radius: 2, y: 1)
                    .position(x: cursorX, y: cursorY)
                    .opacity(fading)
            }
        }
    }
}

// MARK: - 远程应用

private struct OnboardingDemoDesktop: View {
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        GeometryReader { proxy in
            ZStack {
                VStack(spacing: 0) {
                    Rectangle()
                        .fill(DFPalette.control(scheme))
                        .frame(height: 6)
                    Spacer(minLength: 0)
                }
                RoundedRectangle(cornerRadius: DFRadius.small, style: .continuous)
                    .fill(DFPalette.control(scheme))
                    .frame(width: proxy.size.width * 0.52, height: proxy.size.height * 0.42)
                    .overlay {
                        RoundedRectangle(cornerRadius: DFRadius.small, style: .continuous)
                            .strokeBorder(DFPalette.line(scheme), lineWidth: DFMetrics.hairline)
                    }
            }
        }
    }
}

private struct RemoteApplicationDemo: View {
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        OnboardingDemoStage(duration: 6.8, offset: 3.1, restingPhase: 0.8) { phase, size in
            let width = min(124, size.width * 0.33)
            let height = width * 0.6
            let leftX = size.width * 0.27
            let rightX = size.width * 0.77
            let centerY = size.height * 0.44
            let appear = demoWindow(phase, from: 0.08, to: 0.2)
            let travel = demoWindow(phase, from: 0.24, to: 0.62)
            let settle = demoWindow(phase, from: 0.6, to: 0.76)
            let windowWidth = width * CGFloat(0.7 + settle * 0.2)
            let windowHeight = height * CGFloat(0.74 + settle * 0.16)
            let windowX = rightX + (leftX - rightX) * CGFloat(travel)

            ZStack {
                Path { path in
                    path.move(to: CGPoint(x: leftX + width * 0.52, y: centerY - 8))
                    path.addLine(to: CGPoint(x: rightX - width * 0.52, y: centerY - 8))
                }
                .stroke(DFPalette.accent.opacity(0.22),
                        style: StrokeStyle(lineWidth: DFMetrics.hairline * 2, dash: [3, 5]))

                OnboardingDemoMonitor(width: width, height: height, label: "这台 Mac",
                                      active: settle > 0.1) {
                    EmptyView()
                }
                .position(x: leftX, y: centerY)

                OnboardingDemoMonitor(width: width, height: height, label: "另一台 Mac",
                                      active: travel < 0.4) {
                    HStack(spacing: DFSpace.xs) {
                        appTile(DFPalette.accent)
                        appTile(DFPalette.brandViolet)
                        appTile(DFPalette.brandCyan)
                    }
                    .opacity(1 - travel)
                }
                .position(x: rightX, y: centerY)

                VStack(spacing: 0) {
                    HStack(spacing: DFSpace.xs) {
                        Circle().fill(DFPalette.danger).frame(width: 4, height: 4)
                        Circle().fill(DFPalette.warning).frame(width: 4, height: 4)
                        Circle().fill(DFPalette.success).frame(width: 4, height: 4)
                        Spacer(minLength: 0)
                    }
                    .padding(.horizontal, DFSpace.s)
                    .frame(height: 16)
                    Spacer(minLength: 0)
                    VStack(alignment: .leading, spacing: 3) {
                        Capsule().fill(DFPalette.accent.opacity(0.22)).frame(width: windowWidth * 0.55, height: 3)
                        Capsule().fill(DFPalette.line(scheme)).frame(width: windowWidth * 0.7, height: 3)
                        Capsule().fill(DFPalette.line(scheme)).frame(width: windowWidth * 0.4, height: 3)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(DFSpace.s)
                    Spacer(minLength: 0)
                }
                .frame(width: windowWidth, height: windowHeight)
                .background(DFPalette.surface(scheme), in: RoundedRectangle(cornerRadius: DFRadius.control, style: .continuous))
                .overlay {
                    RoundedRectangle(cornerRadius: DFRadius.control, style: .continuous)
                        .strokeBorder(DFPalette.line(scheme), lineWidth: DFMetrics.hairline)
                }
                .shadow(color: .black.opacity(0.18), radius: 8, y: 3)
                .position(x: windowX, y: centerY - 4)
                .opacity(appear)
                .scaleEffect(CGFloat(0.94 + settle * 0.06))
            }
        }
    }

    private func appTile(_ color: Color) -> some View {
        RoundedRectangle(cornerRadius: DFRadius.small, style: .continuous)
            .fill(color.opacity(0.28))
            .frame(width: 15, height: 15)
            .overlay {
                RoundedRectangle(cornerRadius: DFRadius.small, style: .continuous)
                    .strokeBorder(color.opacity(0.45), lineWidth: DFMetrics.hairline)
            }
    }
}

// MARK: - 鼠标按键

private struct MouseButtonDemo: View {
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        OnboardingDemoStage(duration: 4.8, offset: 2.2, restingPhase: 0.66) { phase, size in
            let mouseX = size.width * 0.22
            let centerY = size.height * 0.5
            let press = demoPulse(phase, center: 0.16, width: 0.13)
            let chipAppear = demoWindow(phase, from: 0.2, to: 0.36)
            let arrowAppear = demoWindow(phase, from: 0.16, to: 0.3)

            ZStack {
                Image(systemName: "computermouse.fill")
                    .font(DFType.icon(.hero, weight: .light))
                    .foregroundStyle(.secondary)
                    .position(x: mouseX, y: centerY)
                    .scaleEffect(CGFloat(1 + press * 0.05))

                Circle()
                    .stroke(DFPalette.accent, lineWidth: DFMetrics.selectionStroke)
                    .frame(width: CGFloat(14 + press * 14), height: CGFloat(14 + press * 14))
                    .position(x: mouseX - 10, y: centerY - 4)
                    .opacity(press)

                Path { path in
                    path.move(to: CGPoint(x: mouseX + 28, y: centerY))
                    path.addLine(to: CGPoint(x: size.width * 0.48, y: centerY))
                }
                .stroke(DFPalette.accent.opacity(0.4),
                        style: StrokeStyle(lineWidth: DFMetrics.hairline * 2, dash: [3, 4]))
                .opacity(arrowAppear)

                Image(systemName: "chevron.right")
                    .font(DFType.caption.weight(.semibold))
                    .foregroundStyle(DFPalette.accent)
                    .position(x: size.width * 0.47, y: centerY)
                    .opacity(arrowAppear)

                HStack(spacing: DFSpace.s) {
                    Text("⌘C")
                        .font(DFType.keycap)
                        .foregroundStyle(DFPalette.accent)
                        .padding(.horizontal, DFSpace.xs + 2)
                        .padding(.vertical, DFSpace.xxs)
                        .background(DFPalette.control(scheme), in: RoundedRectangle(cornerRadius: DFRadius.small, style: .continuous))
                    Text("复制")
                        .font(DFType.detail)
                }
                .padding(.horizontal, DFSpace.m)
                .padding(.vertical, DFSpace.s)
                .background(DFPalette.surface(scheme), in: RoundedRectangle(cornerRadius: DFRadius.control, style: .continuous))
                .overlay {
                    RoundedRectangle(cornerRadius: DFRadius.control, style: .continuous)
                        .strokeBorder(DFPalette.line(scheme), lineWidth: DFMetrics.hairline)
                }
                .shadow(color: .black.opacity(0.12), radius: 6, y: 2)
                .position(x: size.width * 0.72, y: centerY)
                .opacity(chipAppear)
                .scaleEffect(CGFloat(0.9 + chipAppear * 0.1))
            }
        }
    }
}

// MARK: - 麦克风共享

private struct MicrophoneDemo: View {
    private let bars = 13

    var body: some View {
        OnboardingDemoStage(duration: 3.4, offset: 0.9, restingPhase: 0.42) { phase, size in
            let centerY = size.height * 0.5
            let micX = size.width * 0.18
            let receiverX = size.width * 0.84
            let waveStart = size.width * 0.32
            let waveEnd = size.width * 0.7
            let arrival = demoPulse(phase, center: 0.55, width: 0.25)

            ZStack {
                Path { path in
                    path.move(to: CGPoint(x: waveStart - 8, y: centerY))
                    path.addLine(to: CGPoint(x: receiverX - 20, y: centerY))
                }
                .stroke(DFPalette.accent.opacity(0.22),
                        style: StrokeStyle(lineWidth: DFMetrics.hairline * 2, lineCap: .round, dash: [2, 5]))

                Circle()
                    .fill(DFPalette.accent.opacity(DFOpacity.soft))
                    .frame(width: 40, height: 40)
                    .position(x: micX, y: centerY)
                Image(systemName: "mic.fill")
                    .font(DFType.icon(.medium, weight: .medium))
                    .foregroundStyle(DFPalette.accent)
                    .position(x: micX, y: centerY)

                HStack(alignment: .center, spacing: 2) {
                    ForEach(0..<bars, id: \.self) { index in
                        let wobble = abs(sin((phase * 2 + Double(index) * 0.075) * .pi * 2))
                        Capsule()
                            .fill(DFPalette.accent.opacity(0.5 + wobble * 0.5))
                            .frame(width: 3, height: CGFloat(4 + wobble * 24))
                    }
                }
                .frame(width: waveEnd - waveStart)
                .position(x: (waveStart + waveEnd) / 2, y: centerY)

                Circle()
                    .stroke(DFPalette.accent.opacity(0.45), lineWidth: DFMetrics.selectionStroke)
                    .frame(width: CGFloat(30 + arrival * 14), height: CGFloat(30 + arrival * 14))
                    .position(x: receiverX, y: centerY)
                    .opacity(0.7 - arrival * 0.4)
                Image(systemName: "laptopcomputer")
                    .font(DFType.icon(.large, weight: .light))
                    .foregroundStyle(.secondary)
                    .position(x: receiverX, y: centerY)
            }
        }
    }
}
