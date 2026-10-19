import SwiftUI
import AppKit
import Charts
import DeskFuseEngine

/// 图表底部色条的卡顿来源颜色。建议由引擎侧生成，Swift 只负责呈现。
private let hitchCaptureColor = DFPalette.warning
private let hitchNetworkColor = DFPalette.accent

struct InputView: View {
    @Bindable var store: AppStore
    let snapshot: EngineSnapshot
    @State private var showAddDevice = false
    @State private var updatingSettings = false
    @State private var retryingInput = false
    @State private var reconnectingDevices: Set<String> = []
    @State private var openSuggestionId: String?
    @State private var latencyScale = InputLatencyChartScale()
    @State private var windowVisible = false

    private var chartVisible: Bool { windowVisible && store.selectedPage == .input }

    private var latencyPeak: Double {
        guard chartVisible else { return 0 }
        let quality = snapshot.input.quality
        return (quality.samples.compactMap(\.rttMs) + [quality.rttMaxMs].compactMap { $0 })
            .filter { $0.isFinite && $0 >= 0 }.max() ?? 0
    }

    var body: some View {
        GeometryReader { geometry in
            if snapshot.input.canConfigure {
                ScrollView(.vertical) {
                    VStack(alignment: .leading, spacing: 18) {
                    ViewThatFits(in: .horizontal) {
                        HStack(alignment: .top, spacing: 16) {
                            heading
                            Spacer(minLength: 16)
                            headerActions
                        }
                        VStack(alignment: .leading, spacing: 12) {
                            heading
                            HStack { Spacer(minLength: 0); headerActions }
                        }
                    }
                    if snapshot.input.locked { Label("鼠标已锁定在当前设备", systemImage: "lock").foregroundStyle(DFPalette.warning) }
                    layout
                        .frame(height: layoutHeight(in: geometry.size))
                    if chartVisible && snapshot.input.settings.controller && snapshot.input.settings.enabled && !snapshot.input.managed {
                        qualityChart
                    }
                    }
                    .padding(.horizontal, DFMetrics.pageInset)
                    .padding(.vertical, DFMetrics.pageTopInset)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            } else if !snapshot.input.settings.enabled && !snapshot.input.settings.controller {
                FeatureGateView(
                    symbol: "rectangle.on.rectangle",
                    title: "键鼠共享",
                    message: "用一套键鼠控制多台电脑。",
                    bullets: [
                        "鼠标穿越屏幕边缘切换设备",
                        "剪贴板随焦点设备自动跟随",
                        "紧急收回快捷键随时可用",
                    ],
                    permission: .accessibility,
                    actionTitle: "启用键鼠共享",
                    action: { setSharingEnabled(true) }
                )
                .padding(28)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                receiverState
                    .padding(28)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .dfCanvas()
        .background(WindowVisibilityObserver { windowVisible = $0 })
        .onAppear { latencyScale.include(peak: latencyPeak) }
        .onChange(of: latencyPeak) { _, peak in latencyScale.include(peak: peak) }
        .sheet(isPresented: $showAddDevice) {
            InputDevicePicker(snapshot: snapshot, adding: updatingSettings, error: store.error, onAdd: { id in
                setParticipant(id, included: true)
            }, onOpenDevices: {
                showAddDevice = false
                store.selectedPage = .devices
            }).dfFloating()
        }
        .onChange(of: snapshot.input.canConfigure) { _, allowed in
            if !allowed { showAddDevice = false }
        }
        .onChange(of: snapshot.input.displays.filter(\.ready).map(\.deviceId)) { _, ready in
            reconnectingDevices.subtract(Set(ready))
        }
        .task(id: snapshot.input.canConfigure) {
            guard snapshot.input.canConfigure else { return }
            await store.refreshDisplayWallpapers()
        }
        .onDisappear {
            store.clearDisplayWallpapers()
        }
    }

    /// 最近 60 秒里某一秒的往返延迟。秒数从窗口起点算起，0 是 60 秒前。
    /// 相邻采样间隔超过 2 秒就断开，避免把空档连成一条斜线。
    private typealias LatencyPoint = InputLatencyPoint

    /// 图表底部色条的一格：这一秒出现过卡顿，颜色区分来源。
    private struct HitchTick: Identifiable {
        let second: Double
        let isCapture: Bool
        var id: Double { second }
    }

    private var compactQuality: Bool { !snapshot.input.canConfigure }

    private var qualityChart: some View {
        let quality = snapshot.input.quality
        let peak = latencyPeak
        let sampledPoints = latencyPoints(quality)
        let points = InputLatencyTrend.smooth(sampledPoints.isEmpty
            ? [LatencyPoint(second: 60, rttMs: 0, series: 0)] : sampledPoints)
        let hitches = hitchTicks(quality)
        let hasSignal = quality.rttP50Ms != nil || quality.rttP95Ms != nil || quality.hitchesPerMinute > 0
        let typical = quality.rttP50Ms ?? quality.rttP95Ms ?? sampledPoints.last?.rttMs
        let ceiling = max(latencyScale.ceiling, InputLatencyChartScale.requiredCeiling(for: peak))
        let series = latencySeriesColor(quality.rttP95Ms ?? typical)
        let status = qualityStatus(quality, hasSignal: hasSignal)
        let blocked = inputBlockedReason
        let showsPeak = peak > 0 && (typical.map {
            peak > $0 + 0.2 && peak > $0 * 1.25
        } ?? true)
        return VStack(alignment: .leading, spacing: compactQuality ? 6 : 16) {
            qualityHeader(status: blocked != nil ? ("暂不可用", DFPalette.warning) : status)
            VStack(alignment: .leading, spacing: 10) {
                if !compactQuality, let typical {
                    latencyHero(typical: typical, peak: showsPeak ? peak : nil)
                }
                InputLatencyTransitionView(points: points, active: chartVisible) { displayed in
                    latencyChart(points: displayed,
                        runCount: Dictionary(grouping: displayed, by: \.series).mapValues(\.count),
                        series: series, ceiling: ceiling, calloutSecond: latencyCalloutSecond(displayed),
                        typical: typical, hitchTicks: hitches)
                }
            }
            Divider()
            HStack(spacing: 16) {
                qualityMetric("P50", milliseconds(quality.rttP50Ms))
                qualityMetric("P95", milliseconds(quality.rttP95Ms))
                qualityMetric("网络卡顿", hitchRate(quality.hitchesPerMinute))
                qualityMetric("慢注入", "\(quality.slowInjections)")
            }
            if !compactQuality, !hitches.isEmpty {
                HStack(spacing: 12) {
                    legendItem(hitchCaptureColor, "注入慢")
                    legendItem(hitchNetworkColor, "网络卡顿")
                }
                .font(DFType.caption)
                .dfSecondaryText()
            }
            if !compactQuality {
                qualityAdvice(quality)
            }
            DisclosureGroup("查看诊断详情") {
                VStack(alignment: .leading, spacing: 6) {
                    if compactQuality {
                        qualityAdvice(quality)
                    }
                    if let blocked {
                        Text(blocked).foregroundStyle(DFPalette.warning)
                    }
                    Text("每秒更新；空闲时显示设备连接延迟，没有可用延迟时显示 0。")
                    Text("当前模式：\(pointerModeLabel)")
                    Text("数据报通道：\(datagramChannelDescription)")
                    Text("慢注入 \(quality.slowInjections) 次")
                    Text("平台没有光标呈现时间。端到端时间是采集排队、发送、往返和注入的估计，不是两台电脑的时钟同步。")
                    if let error = snapshot.input.error, !error.isEmpty {
                        Text(error)
                    }
                }
                .font(DFType.caption)
                .dfSecondaryText()
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.top, 6)
            }
            .font(DFType.caption.weight(.medium))
            if !compactQuality {
                Text("最近 60 秒 · 3 秒平滑趋势 · 指标与峰值为原始采样")
                    .font(DFType.caption)
                    .foregroundStyle(.tertiary)
            }
        }
        .padding(compactQuality ? 10 : 18)
        .dfCard()
        .transaction { transaction in
            transaction.animation = nil
        }
    }

    @ViewBuilder
    private func qualityAdvice(_ quality: InputQuality) -> some View {
        if let fallback = pointerFallback {
            Label(fallback, systemImage: "info.circle")
                .font(DFType.caption)
                .dfSecondaryText()
                .fixedSize(horizontal: false, vertical: true)
        }
        if !quality.suggestions.isEmpty {
            VStack(alignment: .leading, spacing: 8) {
                Text("优化建议")
                    .font(DFType.caption.weight(.semibold))
                ForEach(quality.suggestions, id: \.id) { suggestion in
                    Button {
                        openSuggestionId = openSuggestionId == suggestion.id ? nil : suggestion.id
                    } label: {
                        HStack(alignment: .firstTextBaseline, spacing: 6) {
                            Label {
                                Text(suggestion.trigger)
                                    .multilineTextAlignment(.leading)
                                    .fixedSize(horizontal: false, vertical: true)
                            } icon: {
                                Image(systemName: suggestionSymbol(suggestion.priority))
                                    .foregroundStyle(suggestionTint(suggestion.priority))
                            }
                            Spacer(minLength: 0)
                            Image(systemName: "info.circle")
                                .font(DFType.caption)
                                .foregroundStyle(.tertiary)
                        }
                        .font(DFType.caption)
                        .dfSecondaryText()
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .popover(isPresented: Binding(
                        get: { openSuggestionId == suggestion.id },
                        set: { shown in if !shown { openSuggestionId = nil } }
                    ), arrowEdge: .bottom) {
                        suggestionDetail(suggestion).dfFloating()
                    }
                }
            }
        }
    }

    /// 卡片头部：左侧标题，右侧当前输入模式与状态胶囊。
    private func qualityHeader(status: (title: String, color: Color)) -> some View {
        HStack(alignment: .center, spacing: 10) {
            Label("流畅度", systemImage: "chart.xyaxis.line")
                .font(DFType.icon(.regular).weight(.semibold))
                .fixedSize(horizontal: true, vertical: false)
            Spacer(minLength: 8)
            Text(pointerModeLabel)
                .font(DFType.caption)
                .dfSecondaryText()
            datagramChannelBadge
            HStack(spacing: 5) {
                Circle()
                    .fill(status.color)
                    .frame(width: 6, height: 6)
                Text(status.title)
                    .font(DFType.caption.weight(.semibold))
            }
            .foregroundStyle(status.color)
            .padding(.horizontal, 9)
            .padding(.vertical, 4)
            .background(status.color.opacity(0.12), in: Capsule())
            .overlay {
                Capsule().strokeBorder(status.color.opacity(0.25), lineWidth: 0.5)
            }
        }
    }

    /// 数据报通道是否就绪。运动降级到可靠通道后鼠标照常可用，只有这里能看出
    /// 通道换过，也解释了为什么曲线在降级后仍然只有可靠通道的延迟。
    private var datagramChannelDescription: String {
        snapshot.input.quality.datagramReady.map { $0 ? "就绪" : "不可用，运动走可靠传输" } ?? "尚无采样"
    }

    @ViewBuilder
    private var datagramChannelBadge: some View {
        if snapshot.input.quality.datagramReady == false {
            HStack(spacing: 4) {
                Image(systemName: "exclamationmark.triangle.fill")
                    .font(DFType.caption)
                Text("已降级到可靠通道")
                    .font(DFType.caption.weight(.medium))
            }
            .foregroundStyle(DFPalette.warning)
            .padding(.horizontal, 8)
            .padding(.vertical, 3)
            .background(DFPalette.warning.opacity(0.12), in: Capsule())
            .help("数据报运动通道不可用，移动改走可靠传输：延迟更高，但输入不会中断。检查网络质量或重新连接设备。")
        }
    }

    /// 有数据时的主读数：典型往返延迟大号显示，峰值仅在明显偏离典型值时给出。
    private func latencyHero(typical: Double, peak: Double?) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Text(milliseconds(typical))
                .font(DFType.metric.weight(.semibold))
                .monospacedDigit()
            Text("典型往返延迟")
                .font(DFType.caption)
                .dfSecondaryText()
            Spacer(minLength: 8)
            if let peak {
                Text("峰值 \(milliseconds(peak))")
                    .font(DFType.caption.monospacedDigit())
                    .dfSecondaryText()
            }
        }
    }

    private func latencyChart(points: [LatencyPoint], runCount: [Int: Int], series: Color,
                              ceiling: Double, calloutSecond: Double?, typical: Double?,
                              hitchTicks: [HitchTick]) -> some View {
        Chart {
            ForEach(points) { point in
                if (runCount[point.series] ?? 0) >= 2 {
                    AreaMark(
                        x: .value("时间", point.second),
                        y: .value("往返", point.rttMs),
                        series: .value("段", point.series)
                    )
                    .interpolationMethod(.monotone)
                    .foregroundStyle(
                        LinearGradient(
                            colors: [series.opacity(0.30), series.opacity(0.03)],
                            startPoint: .top,
                            endPoint: .bottom
                        )
                    )
                    LineMark(
                        x: .value("时间", point.second),
                        y: .value("往返", point.rttMs),
                        series: .value("段", point.series)
                    )
                    .interpolationMethod(.monotone)
                    .lineStyle(StrokeStyle(lineWidth: 2, lineCap: .round, lineJoin: .round))
                    .foregroundStyle(series)
                } else if points.count == 1 {
                    RuleMark(
                        x: .value("时间", point.second),
                        yStart: .value("往返", 0.0),
                        yEnd: .value("往返", point.rttMs)
                    )
                    .lineStyle(StrokeStyle(lineWidth: 2, lineCap: .round))
                    .foregroundStyle(series)
                }
            }
            ForEach(points) { point in
                if calloutSecond == point.second as Double? {
                    PointMark(
                        x: .value("时间", point.second),
                        y: .value("往返", point.rttMs)
                    )
                    .symbolSize(points.count == 1 ? 64 : 28)
                    .foregroundStyle(series)
                    .annotation(
                        position: latencyCalloutPosition(point, ceiling: ceiling),
                        spacing: 4,
                        overflowResolution: AnnotationOverflowResolution(x: .fit(to: .chart), y: .fit(to: .chart))
                    ) {
                        Text(milliseconds(point.rttMs))
                            .font(DFType.caption.weight(.medium).monospacedDigit())
                            .dfSecondaryText()
                    }
                } else if points.count <= 8 || (runCount[point.series] ?? 0) == 1 || points.last?.second == point.second as Double? {
                    PointMark(
                        x: .value("时间", point.second),
                        y: .value("往返", point.rttMs)
                    )
                    .symbolSize(28)
                    .foregroundStyle(series)
                }
            }
        }
        .chartYScale(domain: 0.0...(ceiling * 1.28))
        .chartXScale(domain: -1.0...61.0)
        .chartXAxis {
            AxisMarks(values: [0.0, 30.0, 60.0]) { value in
                AxisValueLabel {
                    Text(latencyTimeLabel(value.as(Double.self) ?? 0))
                        .font(DFType.caption)
                        .dfSecondaryText()
                }
            }
        }
        .chartYAxis {
            AxisMarks(position: .leading, values: [0.0, ceiling / 2, ceiling]) { value in
                AxisGridLine(stroke: StrokeStyle(lineWidth: 1))
                    .foregroundStyle(Color.primary.opacity(0.06))
                AxisValueLabel {
                    Text(latencyAxisTick(value.as(Double.self) ?? 0, ceiling: ceiling))
                        .font(DFType.caption.monospacedDigit())
                        .dfSecondaryText()
                        .frame(width: 64, alignment: .trailing)
                }
            }
        }
        .chartLegend(.hidden)
        .chartOverlay { proxy in
            GeometryReader { geometry in
                let plot = proxy.plotFrame.map { geometry[$0] } ?? geometry.frame(in: .local)
                Canvas { context, _ in
                    for tick in hitchTicks {
                        guard let x = proxy.position(forX: tick.second) else { continue }
                        let rect = CGRect(x: x - 1, y: plot.maxY - 6, width: 2, height: 6)
                        context.fill(Path(roundedRect: rect, cornerRadius: DFRadius.small),
                                     with: .color(hitchTickColor(tick)))
                    }
                }
            }
            .allowsHitTesting(false)
        }
        .frame(height: compactQuality ? 84 : 168)
        .accessibilityLabel("最近 60 秒往返延迟，3 秒平滑趋势")
        .accessibilityValue(milliseconds(typical))
    }

    /// 底部色条：这一秒出现过卡顿，本机采集和网络分开着色。
    private func hitchTicks(_ quality: InputQuality) -> [HitchTick] {
        quality.samples.compactMap { sample in
            guard sample.hitches > 0 else { return nil }
            return HitchTick(second: Double(sample.second), isCapture: sample.hitchCapture > 0)
        }
    }

    private func hitchTickColor(_ tick: HitchTick) -> Color {
        tick.isCapture ? hitchCaptureColor : hitchNetworkColor
    }

    private func legendItem(_ color: Color, _ title: String) -> some View {
        HStack(spacing: 4) {
            RoundedRectangle(cornerRadius: DFRadius.small)
                .fill(color)
                .frame(width: 3, height: 8)
            Text(title)
        }
    }

    /// 建议详情：做法、预期和复测方法。
    private func suggestionDetail(_ suggestion: InputQualitySuggestion) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(suggestion.trigger)
                .font(DFType.caption.weight(.semibold))
            suggestionDetailRow("做法", suggestion.action)
            suggestionDetailRow("预期", suggestion.expected)
            suggestionDetailRow("复测", suggestion.retest)
        }
        .padding(12)
        .frame(width: 320, alignment: .leading)
    }

    private func suggestionDetailRow(_ title: String, _ body: String) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title)
                .font(DFType.caption)
                .foregroundStyle(.tertiary)
            Text(body)
                .font(DFType.caption)
                .dfSecondaryText()
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private func suggestionSymbol(_ priority: String) -> String {
        switch priority {
        case "info": return "info.circle.fill"
        case "ok": return "checkmark.circle.fill"
        default: return "lightbulb.fill"
        }
    }

    private func suggestionTint(_ priority: String) -> Color {
        switch priority {
        case "info": return .secondary
        case "ok": return DFPalette.success
        default: return DFPalette.warning
        }
    }

    private func qualityMetric(_ title: String, _ value: String) -> some View {
        let layout = compactQuality
            ? AnyLayout(HStackLayout(alignment: .firstTextBaseline, spacing: 6))
            : AnyLayout(VStackLayout(alignment: .leading, spacing: 3))
        return layout {
            Text(title)
                .font(DFType.caption)
                .foregroundStyle(.tertiary)
            Text(value)
                .font(compactQuality ? DFType.body.weight(.semibold) : DFType.title)
                .monospacedDigit()
                .foregroundStyle(.primary)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func latencyPoints(_ quality: InputQuality) -> [LatencyPoint] {
        var series = 0
        var previous: Double?
        return quality.samples.compactMap { sample in
            guard let rtt = sample.rttMs, rtt.isFinite, rtt >= 0 else { return nil }
            let second = Double(sample.second)
            if let previous, second - previous > 2 {
                series += 1
            }
            previous = second
            return LatencyPoint(second: second, rttMs: rtt, series: series)
        }
    }

    private func latencySeriesColor(_ p95: Double?) -> Color {
        guard let p95 else { return DFPalette.accent }
        if p95 > 50 { return DFPalette.danger }
        if p95 > 20 { return DFPalette.warning }
        return DFPalette.accent
    }

    private func latencyAxisTick(_ value: Double, ceiling: Double) -> String {
        let body: String
        if ceiling < 10 {
            body = value == 0 || value.rounded() == value
                ? String(format: "%.0f", value)
                : String(format: "%.1f", value)
        } else {
            body = String(format: "%.0f", value)
        }
        return abs(value - ceiling) < 0.05 ? "\(body) ms" : body
    }

    private func latencyTimeLabel(_ second: Double) -> String {
        if second <= 0 { return "60秒前" }
        if second >= 60 { return "现在" }
        return "30秒前"
    }

    private func latencyCalloutSecond(_ points: [LatencyPoint]) -> Double? {
        guard points.count <= 4, let last = points.last else { return nil }
        guard let peak = points.max(by: { $0.rttMs < $1.rttMs }) else { return last.second }
        return last.rttMs >= peak.rttMs * 0.98 ? last.second : peak.second
    }

    private func latencyCalloutPosition(_ point: LatencyPoint, ceiling: Double) -> AnnotationPosition {
        if point.rttMs > ceiling * 0.72 { return .bottom }
        if point.second > 48 { return .topLeading }
        if point.second < 8 { return .topTrailing }
        return .top
    }

    private func qualityStatus(_ quality: InputQuality, hasSignal: Bool) -> (title: String, color: Color) {
        switch quality.grade {
        case "smooth": return ("流畅", DFPalette.success)
        case "fair": return ("一般", DFPalette.warning)
        case "poor": return ("较差", DFPalette.danger)
        default:
            return hasSignal ? ("采样较少", .secondary) : ("空闲", .secondary)
        }
    }

    private func hitchRate(_ value: Double) -> String {
        value == 0 ? "0" : String(format: "%.1f 次/分钟", value)
    }

    private var pointerModeLabel: String {
        let summary = snapshot.input.summary
        if summary.contains("相对移动") { return "相对移动" }
        if summary.contains("绝对坐标") { return "绝对坐标" }
        if summary.contains("可靠路径") { return "可靠路径" }
        return "等待会话"
    }

    private var pointerFallback: String? {
        let summary = snapshot.input.summary
        guard let marker = ["相对移动", "绝对坐标", "可靠路径"].first(where: { summary.contains($0) }),
              let range = summary.range(of: marker) else { return nil }
        let rest = summary[range.upperBound...].trimmingCharacters(in: .whitespaces)
        guard rest.hasPrefix("·") else { return nil }
        let reason = rest.dropFirst().trimmingCharacters(in: .whitespaces)
        return reason.isEmpty ? nil : String(reason)
    }

    private var inputBlockedReason: String? {
        if let error = snapshot.input.error?.trimmingCharacters(in: .whitespacesAndNewlines), !error.isEmpty {
            return error
        }
        if snapshot.input.summary.contains("权限") {
            return snapshot.input.summary
        }
        return nil
    }

    private func milliseconds(_ value: Double?) -> String {
        guard let value else { return "—" }
        if value > 0 && value < 0.1 {
            return String(format: "%.2f ms", value)
        }
        return String(format: "%.1f ms", value)
    }

    private var receiverState: some View {
        VStack(spacing: 20) {
            Image(systemName: snapshot.input.managedPaused ? "pause.circle" : "keyboard")
                .font(DFType.icon(.hero, weight: .light)).dfSecondaryText()
                .accessibilityHidden(true)
            Text(snapshot.input.managedPaused ? "已停止被控" : snapshot.input.managed ? "被控状态" : snapshot.input.settings.controller ? "主控已自动暂停" : "等待主控")
                .font(DFType.title)
            Text(snapshot.input.summary).font(DFType.detail).dfSecondaryText()
                .multilineTextAlignment(.center)
            if snapshot.input.managed {
                Button("停止被控") { store.reclaim() }
                    .buttonStyle(.dfPrimary)
                    .disabled(snapshot.input.managedPaused)
            } else {
                if snapshot.input.settings.controller {
                    Text(snapshot.input.environmentMessage).font(DFType.detail).dfSecondaryText()
                        .multilineTextAlignment(.center)
                }
                Button(snapshot.input.settings.controller ? "本次使用此 Mac 作为主控" : "在此 Mac 启用主控") { enableAsController() }
                    .buttonStyle(.dfPrimary)
            }
        }.frame(maxWidth: 520)
    }

    private var heading: some View {
        /// 标题与开关行视作页头的一组：行间距对齐 SystemSettingsPageHeader
        /// 标题与说明沿用主窗口的统一层级。
        VStack(alignment: .leading, spacing: 10) {
            Text("当前屏幕排布").font(DFType.title).fixedSize()
            sharingSetting
        }
    }

    private var sharingSetting: some View {
        VStack(alignment: .leading, spacing: 3) {
            Toggle("共享键鼠", isOn: Binding(
                get: { snapshot.input.settings.controller && snapshot.input.settings.enabled },
                set: { setSharingEnabled($0) }
            ))
            .font(DFType.detail.weight(.medium))
            .toggleStyle(.switch)
            .controlSize(.small)
            .disabled(updatingSettings || snapshot.input.managed)
            Text("由这台 Mac 管理屏幕排布，并使用本机键鼠控制已添加的设备。")
                .font(DFType.caption)
                .dfSecondaryText()
        }
        .help("当前 Wi-Fi 会记住主控与屏幕排布；回到该 Wi-Fi 时自动恢复，未匹配的网络自动暂停。")
    }

    private var headerActions: some View {
        NativeGlassGroup {
            HStack(alignment: .center, spacing: 12) {
                DFActionButton("添加设备", systemImage: "plus", kind: .primary) {
                    showAddDevice = true
                }
            }
            .fixedSize(horizontal: true, vertical: false)
            .disabled(updatingSettings)
        }
    }

    /// 单一开关同步主控角色和共享状态，一次提交避免中间状态。
    private func setSharingEnabled(_ enabled: Bool) {
        var settings = (store.snapshot ?? snapshot).input.settings
        settings.enabled = enabled
        settings.controller = enabled
        applySettings(settings)
    }

    /// 从等待页面启用主控时，同步开启键鼠共享。
    private func enableAsController() {
        let current = store.snapshot ?? snapshot
        if current.input.settings.controller {
            store.send(.chooseController(id: current.localId, remember: false))
            return
        }
        setSharingEnabled(true)
    }

    /// Dial the peer again and start a new input activation so the receiver can prepare.
    private func reconnectDevice(_ id: String) {
        guard !reconnectingDevices.contains(id) else { return }
        reconnectingDevices.insert(id)
        store.error = nil
        store.send(.connect(peerId: id))
        store.send(.restartInputActivation(peerId: id))
        Task { @MainActor in
            try? await Task.sleep(for: .seconds(12))
            reconnectingDevices.remove(id)
        }
    }

    private func applySettings(_ settings: InputSettings) {
        guard !updatingSettings else { return }
        updatingSettings = true
        store.error = nil
        store.send(.setInput(settings: settings), applied: { updatingSettings = false },
                   failed: { _ in updatingSettings = false })
    }

    private func setParticipant(_ id: String, included: Bool) {
        let current = store.snapshot ?? snapshot
        guard current.input.canConfigure, id != current.localId else { return }
        var settings = current.input.settings
        if included {
            guard current.devices.contains(where: { $0.id == id && $0.member && !$0.blocked }),
                  !settings.participants.contains(id) else { return }
            settings.participants.append(id)
        } else {
            settings.participants.removeAll { $0 == id }
        }
        // Keep saved placement and pairing so adding this device again restores its position.
        applySettings(settings)
    }

    private var layout: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .center, spacing: 12) {
                Text("\(snapshot.input.displays.count) 个屏幕")
                    .font(DFType.detail.weight(.medium))
                    .dfSecondaryText()
                Spacer(minLength: 8)
                Text("拖动屏幕以匹配实际摆放位置 · pt 为 macOS 逻辑尺寸")
                    .font(DFType.caption).dfSecondaryText()
                    .fixedSize(horizontal: false, vertical: true)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 12)
            DisplayLayout(store: store, snapshot: snapshot, onRemoveDevice: { id in
                setParticipant(id, included: false)
            }, onReconnectDevice: reconnectDevice, reconnecting: reconnectingDevices,
               editingDisabled: updatingSettings)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .frame(maxWidth: .infinity)
        .dfCard()
    }

    /// The user requested twice the previous canvas height so vertically
    /// arranged screens have enough space to remain large and readable.
    private func layoutHeight(in size: CGSize) -> CGFloat {
        let displays = snapshot.input.displays
        guard !displays.isEmpty else { return min(300, max(220, size.height - 430)) * 2 }
        let minX = displays.map(\.x).min() ?? 0
        let minY = displays.map(\.y).min() ?? 0
        let maxX = displays.map { $0.x + $0.width }.max() ?? 1
        let maxY = displays.map { $0.y + $0.height }.max() ?? 1
        let aspect = max(0.5, (maxX - minX) / max(1, maxY - minY))
        let contentWidth = max(320, size.width - DFMetrics.pageInset * 2 - 64)
        let aspectHeight = contentWidth / aspect + 110
        let availableHeight = max(220, min(400, size.height - 430))
        return min(max(220, aspectHeight), availableHeight) * 2
    }

}
