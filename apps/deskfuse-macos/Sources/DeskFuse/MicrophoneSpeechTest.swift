import AppKit
import AVFoundation
import Observation
@preconcurrency import Speech
import SwiftUI

// 麦克风共享页的“语音输入测试”：用系统语音识别读取当前默认输入设备（接收端即 DeskFuse 麦克风），
// 把识别文字实时写入输入框，用来端到端验证远端麦克风链路。只在用户点击时运行，文字不落盘。

/// 音频线程一侧的识别会话：持有识别请求、记录电平。
/// 安装 tap 与创建识别任务都放在这个非隔离类型里，保证回调闭包不被推断为 MainActor 隔离，
/// 否则音频线程 / 识别队列回调会触发 Swift 6 的运行时隔离断言。
private final class SpeechCaptureSession: @unchecked Sendable {
    let request = SFSpeechAudioBufferRecognitionRequest()
    private let lock = NSLock()
    private var peak: Float = 0

    /// 在输入节点上安装 tap：缓冲直接交给识别请求，同时计算本批次电平。
    func install(on input: AVAudioInputNode, format: AVAudioFormat) {
        input.installTap(onBus: 0, bufferSize: 1024, format: format) { [self] buffer, _ in
            request.append(buffer)
            let level = Self.normalizedLevel(buffer)
            lock.lock()
            peak = max(peak, level)
            lock.unlock()
        }
    }

    /// 取出并清零自上次读取以来的峰值电平（0...1）。
    func takeLevel() -> Float {
        lock.lock(); defer { lock.unlock() }
        let value = peak
        peak = 0
        return value
    }

    /// 启动识别任务；结果在识别队列上抽取成可跨线程的值，再交回主线程处理。
    func recognize(with recognizer: SFSpeechRecognizer,
                   handler: @escaping @Sendable @MainActor (String?, Bool, (any Error)?) -> Void) -> SFSpeechRecognitionTask {
        recognizer.recognitionTask(with: request) { result, error in
            let text = result?.bestTranscription.formattedString
            let isFinal = result?.isFinal ?? false
            Task { @MainActor in handler(text, isFinal, error) }
        }
    }

    /// RMS 转分贝后映射到 0...1：-50 dB 及以下视为静音。
    private static func normalizedLevel(_ buffer: AVAudioPCMBuffer) -> Float {
        guard let samples = buffer.floatChannelData?[0], buffer.frameLength > 0 else { return 0 }
        let count = Int(buffer.frameLength)
        var sum: Float = 0
        for index in 0..<count { sum += samples[index] * samples[index] }
        let rms = sqrt(sum / Float(count))
        guard rms > 0 else { return 0 }
        return min(1, max(0, (20 * log10(rms) + 50) / 50))
    }
}

/// Constructed exclusively on a background task, then handed to the main actor.
/// The recognizer is only used on the main actor after this ownership transfer.
struct MicrophoneSpeechPreparation: @unchecked Sendable {
    let recognizer: SFSpeechRecognizer?
    let inputName: String?

    static func load() -> Self {
        Self(recognizer: SFSpeechRecognizer(locale: Locale(identifier: "zh-CN")),
             inputName: AVCaptureDevice.default(for: .audio)?.localizedName)
    }
}

/// 语音输入测试的状态机：后台加载 → 空闲 → 请求权限 → 聆听 → 空闲。
@MainActor
@Observable
final class MicrophoneSpeechTester {
    enum Phase: Equatable { case loading, idle, authorizing, listening }

    /// 需要用户处理的问题；`settingsURL` 非空时提供“打开系统设置”。
    struct Problem: Equatable {
        let title: String
        let message: String
        var settingsURL: URL? = nil
    }

    var text = ""
    private(set) var phase: Phase = .loading
    private(set) var level: Float = 0
    var problem: Problem?
    /// 本次识别使用的输入设备名称，便于确认确实读取的是 DeskFuse 麦克风。
    private(set) var inputName: String?
    private(set) var onDevice = false

    private var recognizer: SFSpeechRecognizer?
    private var preparation: Task<MicrophoneSpeechPreparation, Never>?
    private var prepared = false
    private var engine: AVAudioEngine?
    private var session: SpeechCaptureSession?
    private var task: SFSpeechRecognitionTask?
    private var meter: Task<Void, Never>?
    private var configurationObserver: NSObjectProtocol?
    /// 开始识别前输入框已有的文字，识别结果追加在其后。
    private var prefix = ""
    /// 会话代数：页面离开或重新开始后，旧识别任务的迟到回调一律丢弃。
    private var generation = 0

    var listening: Bool { phase == .listening }

    /// No Speech/Core Audio calls during init or view evaluation. Coalesce
    /// reappearances while a slow, non-cancellable system query is in flight.
    func prepare(load: @escaping @Sendable () -> MicrophoneSpeechPreparation = MicrophoneSpeechPreparation.load) async {
        guard !prepared else { return }
        let work: Task<MicrophoneSpeechPreparation, Never>
        if let preparation {
            work = preparation
        } else {
            work = Task.detached(priority: .utility) { load() }
            preparation = work
        }
        let result = await work.value
        guard !Task.isCancelled, !prepared else { return }
        recognizer = result.recognizer
        inputName = result.inputName
        prepared = true
        preparation = nil
        phase = .idle
    }

    func refreshInputName() async {
        let current = generation
        let name = await Task.detached(priority: .utility) {
            AVCaptureDevice.default(for: .audio)?.localizedName
        }.value
        guard !Task.isCancelled, current == generation else { return }
        inputName = name
    }

    func toggle() {
        switch phase {
        case .idle: Task { await start() }
        case .listening: finish()
        case .loading, .authorizing: break
        }
    }

    func clear() {
        text = ""
        prefix = ""
    }

    // MARK: - 开始

    private func start() async {
        problem = nil
        phase = .authorizing
        let current = generation
        guard await authorize() else { phase = .idle; return }
        await refreshInputName()
        guard current == generation else { return }
        guard let recognizer, recognizer.isAvailable else {
            phase = .idle
            problem = Problem(title: "语音识别暂不可用",
                              message: "系统中文（zh-CN）语音识别当前不可用，请检查网络或稍后重试。")
            return
        }
        do {
            try begin(with: recognizer)
            phase = .listening
        } catch {
            teardown()
            problem = Problem(title: "无法开始录音", message: Self.message(for: error))
        }
    }

    /// 依次确认语音识别与麦克风权限；被拒绝时给出可跳转的中文提示。
    private func authorize() async -> Bool {
        var speech = SFSpeechRecognizer.authorizationStatus()
        if speech == .notDetermined { speech = await Self.requestSpeechAuthorization() }
        guard speech == .authorized else {
            problem = Problem(
                title: speech == .restricted ? "语音识别受限" : "未允许语音识别",
                message: "请在“系统设置 › 隐私与安全性 › 语音识别”中允许 DeskFuse，然后重试。",
                settingsURL: URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_SpeechRecognition"))
            return false
        }
        var microphone = AVCaptureDevice.authorizationStatus(for: .audio)
        if microphone == .notDetermined {
            microphone = await AVCaptureDevice.requestAccess(for: .audio) ? .authorized : .denied
        }
        guard microphone == .authorized else {
            problem = Problem(
                title: "未允许使用麦克风",
                message: "语音输入测试需要读取系统默认输入设备。请在“系统设置 › 隐私与安全性 › 麦克风”中允许 DeskFuse。",
                settingsURL: URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"))
            return false
        }
        return true
    }

    /// 授权回调在任意队列触发，放在非隔离函数里包装，避免闭包继承 MainActor 隔离。
    private nonisolated static func requestSpeechAuthorization() async -> SFSpeechRecognizerAuthorizationStatus {
        await withCheckedContinuation { continuation in
            SFSpeechRecognizer.requestAuthorization { continuation.resume(returning: $0) }
        }
    }

    /// 创建 audio engine（输入节点即系统默认输入设备），安装 tap 并启动识别。
    private func begin(with recognizer: SFSpeechRecognizer) throws {
        generation += 1
        let current = generation
        let engine = AVAudioEngine()
        let input = engine.inputNode
        let format = input.outputFormat(forBus: 0)
        guard format.sampleRate > 0, format.channelCount > 0 else {
            throw SpeechTestError.message("当前输入设备没有可用的音频格式，请确认系统默认输入设备正常。")
        }
        let session = SpeechCaptureSession()
        session.request.shouldReportPartialResults = true
        session.request.addsPunctuation = true
        // 支持本机识别时不把音频发往服务器。
        onDevice = recognizer.supportsOnDeviceRecognition
        session.request.requiresOnDeviceRecognition = onDevice
        session.install(on: input, format: format)

        self.engine = engine
        self.session = session
        prefix = text.isEmpty || text.hasSuffix("\n") ? text : text + "\n"

        engine.prepare()
        try engine.start()

        task = session.recognize(with: recognizer) { [weak self] transcript, isFinal, error in
            self?.receive(transcript, isFinal: isFinal, error: error, generation: current)
        }
        // 录音中切换系统默认输入或设备断开时，engine 会停止，这里结束本次测试并提示。
        configurationObserver = NotificationCenter.default.addObserver(
            forName: .AVAudioEngineConfigurationChange, object: engine, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self, self.generation == current, self.listening else { return }
                self.finish()
                self.problem = Problem(title: "输入设备已变化", message: "系统默认输入设备发生变化，请重新开始测试。")
            }
        }
        startMeter(generation: current)
    }

    /// 以约 20 Hz 读取电平，平滑衰减，驱动按钮光环与电平条。
    private func startMeter(generation current: Int) {
        meter?.cancel()
        meter = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .milliseconds(50))
                guard let self, self.generation == current, let session = self.session else { return }
                let sample = session.takeLevel()
                self.level = max(sample, self.level * 0.75)
            }
        }
    }

    // MARK: - 识别结果

    private func receive(_ transcript: String?, isFinal: Bool, error: (any Error)?, generation current: Int) {
        guard current == generation else { return }
        if let transcript { text = prefix + transcript }
        if let error, task != nil, phase == .listening {
            // 用户主动停止后的取消类错误不提示；聆听中出错才提示。
            teardown()
            problem = Problem(title: "识别已停止", message: Self.message(for: error))
            return
        }
        if isFinal || error != nil {
            // 识别结束（单次识别时长达到上限或已收尾），释放资源。
            teardown()
        }
    }

    // MARK: - 停止与释放

    /// 用户再次点击：停止采集，等待识别器给出最后结果后再释放任务。
    func finish() {
        guard phase == .listening else { return }
        phase = .idle
        stopAudio()
        session?.request.endAudio()
    }

    /// 离开页面或出错：立即停止录音、取消识别并释放 audio engine。
    func teardown() {
        generation += 1
        phase = prepared ? .idle : .loading
        stopAudio()
        task?.cancel()
        task = nil
        session = nil
    }

    private func stopAudio() {
        meter?.cancel()
        meter = nil
        if let configurationObserver { NotificationCenter.default.removeObserver(configurationObserver) }
        configurationObserver = nil
        if let engine {
            engine.stop()
            engine.inputNode.removeTap(onBus: 0)
        }
        engine = nil
        level = 0
    }

    private static func message(for error: any Error) -> String {
        if case SpeechTestError.message(let text) = error { return text }
        let nsError = error as NSError
        // 1110：没有检测到语音；1101/1107：本机识别服务异常；203：服务端暂时无法识别。
        switch (nsError.domain, nsError.code) {
        case ("kAFAssistantErrorDomain", 1110):
            return "没有检测到语音。请确认远端 Mac 正在共享麦克风，并靠近麦克风说话。"
        case ("kAFAssistantErrorDomain", 1101), ("kAFAssistantErrorDomain", 1107):
            return "系统语音识别服务暂时异常，请稍后重试。"
        case ("kAFAssistantErrorDomain", 203):
            return "语音识别服务暂时无法识别，请检查网络后重试。"
        default:
            return "语音识别出错：\(nsError.localizedDescription)"
        }
    }
}

private enum SpeechTestError: Error {
    case message(String)
}

// MARK: - 界面

/// “语音输入测试”卡片：两行输入框 + 麦克风按钮，识别文字实时写入输入框。
struct MicrophoneSpeechTestCard: View {
    /// 接收端读取的是 DeskFuse 麦克风（远端声音），提供端读取本机默认输入。
    let receiver: Bool
    @State private var tester = MicrophoneSpeechTester()
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        VStack(alignment: .leading, spacing: DFSpace.m) {
            DFCardHeader("语音输入测试", symbol: "text.bubble", detail: headerDetail)
            editor
            controls
            if let problem = tester.problem {
                DFNotice(tone: .warning, title: problem.title, message: problem.message,
                         actionTitle: problem.settingsURL == nil ? nil : "打开系统设置",
                         action: problem.settingsURL.map { url in { _ = NSWorkspace.shared.open(url) } },
                         onDismiss: { tester.problem = nil })
                    .transition(.opacity.combined(with: .move(edge: .top)))
            }
        }
        .padding(DFSpace.l)
        .dfCard()
        .animation(DFMotion.snappy, value: tester.problem)
        .animation(DFMotion.snappy, value: tester.phase)
        .task { await tester.prepare() }
        .task(id: MicrophoneSharing.shared.isDefaultInput) {
            // The default can change after the user completes receiver setup.
            // Keep that refresh off the view-rendering path too.
            if tester.phase != .loading { await tester.refreshInputName() }
        }
        .onDisappear { tester.teardown() }
    }

    private var headerDetail: String {
        receiver
            ? "点击麦克风，测试远端声音输入与实时识别。"
            : "点击麦克风，测试本机默认输入与实时识别。"
    }

    private var editor: some View {
        TextField(tester.listening ? "正在聆听，请说话…" : "识别结果，也可直接输入", text: $tester.text, axis: .vertical)
            .lineLimit(2, reservesSpace: true)
            .textFieldStyle(.plain)
            .font(DFType.body)
            .padding(.horizontal, DFSpace.m)
            .padding(.vertical, DFSpace.s)
            .disabled(tester.listening)
            .dfControlSurface()
            .overlay {
                RoundedRectangle(cornerRadius: DFRadius.control, style: .continuous)
                    .strokeBorder(tester.listening ? DFPalette.accent : DFPalette.line(scheme),
                                  lineWidth: tester.listening ? DFMetrics.selectionStroke : DFMetrics.hairline)
            }
            .accessibilityLabel("语音识别结果")
    }

    private var controls: some View {
        HStack(spacing: DFSpace.m) {
            MicrophoneSpeechButton(phase: tester.phase, level: tester.level) { tester.toggle() }
            VStack(alignment: .leading, spacing: DFSpace.xxs) {
                Text(statusTitle).font(DFType.bodyMedium)
                Text(statusDetail)
                    .font(DFType.detail).dfSecondaryText()
                    .lineLimit(2)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: DFSpace.s)
            if tester.listening {
                MicrophoneLevelView(level: tester.level)
                    .frame(width: 72, height: 24)
                    .accessibilityLabel("输入电平")
                    .accessibilityValue("\(Int(tester.level * 100))%")
                    .transition(.opacity)
            }
            Button("清空") { tester.clear() }
                .buttonStyle(.dfPlain)
                .disabled(tester.text.isEmpty || tester.listening)
        }
    }

    private var statusTitle: String {
        switch tester.phase {
        case .loading: "正在加载语音测试…"
        case .idle: "点击麦克风开始说话"
        case .authorizing: "正在确认权限…"
        case .listening: "正在聆听，再次点击停止"
        }
    }

    private var statusDetail: String {
        if tester.phase == .loading { return "正在读取音频设备与识别服务，可继续操作页面。" }
        let input = tester.inputName ?? "系统默认输入"
        let engine = tester.listening ? (tester.onDevice ? "本机识别" : "Apple 语音识别") : "中文识别"
        return "输入设备：\(input) · \(engine) · 文字不保存"
    }
}

/// 圆形麦克风按钮：聆听中填充强调色，外圈光环随真实电平扩散。
private struct MicrophoneSpeechButton: View {
    let phase: MicrophoneSpeechTester.Phase
    let level: Float
    let action: () -> Void
    @Environment(\.colorScheme) private var scheme
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private let diameter = DFMetrics.controlHeight + DFSpace.s

    var body: some View {
        let listening = phase == .listening
        Button(action: action) {
            ZStack {
                if listening {
                    // 电平光环：减弱动态效果时保持静止描边。
                    Circle()
                        .fill(DFPalette.accent.opacity(DFOpacity.soft))
                        .scaleEffect(reduceMotion ? 1.15 : 1 + CGFloat(level) * 0.45)
                        .animation(.easeOut(duration: 0.12), value: level)
                }
                Circle()
                    .fill(listening ? DFPalette.accent : DFPalette.control(scheme))
                Circle()
                    .strokeBorder(listening ? .clear : DFPalette.line(scheme), lineWidth: DFMetrics.hairline)
                if phase == .loading || phase == .authorizing {
                    ProgressView().controlSize(.small)
                } else {
                    Image(systemName: listening ? "stop.fill" : "mic.fill")
                        .font(DFType.icon(.medium, weight: .medium))
                        .foregroundStyle(listening ? Color.white : DFPalette.accent)
                        .contentTransition(.symbolEffect(.replace))
                }
            }
            .frame(width: diameter, height: diameter)
            .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .disabled(phase == .loading || phase == .authorizing)
        .help(listening ? "停止语音输入" : "开始语音输入")
        .accessibilityLabel(listening ? "停止语音输入" : "开始语音输入")
    }
}
