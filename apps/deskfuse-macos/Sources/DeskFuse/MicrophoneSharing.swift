import AppKit
import AVFoundation
import CoreAudio
import CoreGraphics
import Observation
import DeskFuseEngine
import MicrophoneNative

struct MicrophoneSettings: Codable, Equatable, Sendable {
    var enabled = false
    var source = ""
    var allowed: [String] = []
    var input_uid = ""
    var shortcut_key: UInt16 = 63
    var shortcut_modifiers: UInt64 = 0
}
struct MicrophoneNativeObservation: Decodable, Sendable {
    var driver: Bool; var readers: Bool; var streaming: Bool; var locked: Bool
    var permission: String; var level: Float; var error: String
}
struct MicrophoneSharingSnapshot: Decodable, Sendable {
    var settings: MicrophoneSettings
    var phase: String; var detail: String; var session_id: UInt64
    var role: String; var peer: String; var source_name: String
    var native: MicrophoneNativeObservation
}
/// UI holds only a projection of Rust settings/session state.
@MainActor @Observable final class MicrophoneSharing {
    static let shared = MicrophoneSharing()
    var snapshot: MicrophoneSharingSnapshot?
    var inputs: [MicrophoneInput] = []
    var error: String?
    private(set) var connectionError: String?
    var saving = false
    var managingDriver = false
    var driverInstalled = MicrophoneDriverManager.isInstalled
    var driverMessage: String?
    private var engine: Engine?
    private var worker: MicrophoneWorker?
    private var polling: Task<Void, Never>?
    private var inputRefresh: Task<Void, Never>?
    private var observers: [NSObjectProtocol] = []
    private var askedPermission = false
    private var connectionDeadline: ContinuousClock.Instant?
    private static let connectionTimeout: Duration = .seconds(5)
    func start(engine: Engine) {
        guard worker == nil else { return }
        self.engine = engine
        let worker = MicrophoneWorker(engine: engine)
        self.worker = worker
        connectionError = nil
        connectionDeadline = .now.advanced(by: Self.connectionTimeout)
        for name in [NSWorkspace.screensDidSleepNotification, NSWorkspace.willSleepNotification] {
            observers.append(NSWorkspace.shared.notificationCenter.addObserver(forName: name, object: nil, queue: .main) { _ in worker.setSleeping(true) })
        }
        for name in [NSWorkspace.screensDidWakeNotification, NSWorkspace.didWakeNotification] {
            observers.append(NSWorkspace.shared.notificationCenter.addObserver(forName: name, object: nil, queue: .main) { _ in worker.setSleeping(false) })
        }
        observers.append(DistributedNotificationCenter.default().addObserver(forName: NSNotification.Name("com.apple.screenIsLocked"), object: nil, queue: .main) { _ in worker.setLocked(true) })
        observers.append(DistributedNotificationCenter.default().addObserver(forName: NSNotification.Name("com.apple.screenIsUnlocked"), object: nil, queue: .main) { _ in worker.setLocked(false) })
        // Do not enumerate HAL devices on the main actor during app launch.
        // The settings page requests the list asynchronously when it is opened.
        worker.start()
        polling = Task { [weak self] in
            while !Task.isCancelled {
                guard let self else { break }
                let status = worker.status()
                if let snapshot = status.snapshot {
                    self.snapshot = snapshot
                    self.connectionError = status.error
                    self.connectionDeadline = nil
                } else if let deadline = self.connectionDeadline, ContinuousClock.now >= deadline {
                    worker.expireInitialization()
                    self.connectionDeadline = nil
                    self.connectionError = "连接麦克风服务超时（5 秒）。" + (status.error.map { "\n原因：\($0)" } ?? "服务尚未响应，请重试。")
                }
                self.driverInstalled = MicrophoneDriverManager.isInstalled
                if self.snapshot?.role == "sender", AVCaptureDevice.authorizationStatus(for: .audio) == .notDetermined, !self.askedPermission {
                    self.askedPermission = true
                    _ = await AVCaptureDevice.requestAccess(for: .audio)
                }
                try? await Task.sleep(for: .milliseconds(250))
            }
        }
    }
    func ensureStarted(engine: Engine?) {
        guard worker == nil else { return }
        guard let engine else {
            connectionError = "DeskFuse 引擎尚未就绪，请稍后重试。"
            return
        }
        start(engine: engine)
    }
    func retryConnection(engine: Engine?) {
        guard snapshot == nil, connectionError != nil else { return }
        if let worker {
            // Retry on the existing thread. A blocked native call cannot be
            // cancelled safely; never create another capture/driver worker.
            worker.retryInitialization()
            connectionError = nil
            connectionDeadline = .now.advanced(by: Self.connectionTimeout)
        } else {
            ensureStarted(engine: engine)
        }
    }
    func stop() {
        connectionDeadline = nil
        polling?.cancel(); polling = nil
        inputRefresh?.cancel(); inputRefresh = nil
        worker?.stop(); worker = nil
        snapshot = nil; engine = nil; askedPermission = false
        for token in observers { NSWorkspace.shared.notificationCenter.removeObserver(token); DistributedNotificationCenter.default().removeObserver(token) }
        observers.removeAll()
    }
    func configure(_ settings: MicrophoneSettings) {
        guard let engine, !saving else { return }
        saving = true; error = nil
        Task {
            defer { saving = false }
            do {
                let json = String(decoding: try JSONEncoder().encode(settings), as: UTF8.self)
                try await Task.detached { try engine.configureMicrophone(settingsJson: json) }.value
                snapshot = try await Task.detached { try JSONDecoder().decode(MicrophoneSharingSnapshot.self, from: Data(engine.microphoneSnapshot().utf8)) }.value
            } catch { self.error = error.localizedDescription }
        }
    }
    func refreshInputs() {
        // Coalesce refreshes: a slow HAL call must not accumulate more queries.
        guard inputRefresh == nil else { return }
        let engine = self.engine
        inputRefresh = Task { [weak self] in
            let values = await Task.detached(priority: .utility) {
                engine?.recordStartupStage(stage: "microphone_devices_loading")
                let values = MicrophoneAudioDevices.physicalInputs()
                engine?.recordStartupStage(stage: "microphone_devices_loaded")
                return values
            }.value
            guard !Task.isCancelled, let self else { return }
            self.inputs = values
            self.inputRefresh = nil
        }
    }
    func setDefaultInput() {
        Task {
            do {
                try await Task.detached(priority: .utility) {
                    if let id = MicrophoneAudioDevices.defaultInput(), let uid = MicrophoneAudioDevices.string(id, kAudioDevicePropertyDeviceUID), uid != MicrophoneAudioDevices.virtualUID {
                        UserDefaults.standard.set(uid, forKey: "microphone.previousDefaultInput")
                    }
                    try MicrophoneAudioDevices.setDefault(uid: MicrophoneAudioDevices.virtualUID)
                }.value
                error = nil
            } catch { self.error = error.localizedDescription }
        }
    }
    func manageDriver(_ action: MicrophoneDriverManager.Action, restartAudio: Bool) {
        guard !managingDriver, !saving, let engine, let resources = Bundle.main.resourceURL else { return }
        managingDriver = true; error = nil; driverMessage = nil
        Task {
            defer { managingDriver = false; driverInstalled = MicrophoneDriverManager.isInstalled; refreshInputs() }
            do {
                // Stop sharing before changing a loaded HAL plugin. Leave the explicit
                // switch off so audio cannot resume unexpectedly after authorization.
                if var settings = snapshot?.settings, settings.enabled {
                    settings.enabled = false
                    let json = String(decoding: try JSONEncoder().encode(settings), as: UTF8.self)
                    try await Task.detached { try engine.configureMicrophone(settingsJson: json) }.value
                }
                let completed = try await Task.detached {
                    try MicrophoneDriverManager.run(action, resources: resources, restartAudio: restartAudio)
                }.value
                if completed {
                    driverMessage = action == .install ? "驱动安装完成。" : "驱动卸载完成。"
                    driverMessage! += restartAudio ? "已请求重启音频服务，请等待系统重新加载音频设备。" : "请重启 Mac 后生效。"
                    if action == .install { driverMessage! += "准备就绪后可重新开启麦克风共享。" }
                } else {
                    driverMessage = "已取消驱动操作。若共享原先已开启，请按需重新开启。"
                }
            } catch { self.error = error.localizedDescription }
        }
    }
    func requestPermission() {
        Task {
            if AVCaptureDevice.authorizationStatus(for: .audio) == .notDetermined { _ = await AVCaptureDevice.requestAccess(for: .audio) }
            else if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone") { NSWorkspace.shared.open(url) }
        }
    }
}

private final class MicrophoneWorker: @unchecked Sendable {
    private let engine: Engine
    private let lock = NSLock()
    private var locked = false
    private var sleeping = false
    private var stopped = false
    private var systemLocked = true
    private var projection: MicrophoneSharingSnapshot?
    private var initializationGeneration: UInt64 = 1
    private var initializationPending = true
    private var initializationError: String?
    init(engine: Engine) { self.engine = engine }
    func status() -> (snapshot: MicrophoneSharingSnapshot?, error: String?) {
        lock.withLock { (projection, initializationError) }
    }
    func retryInitialization() {
        lock.withLock {
            guard projection == nil, !stopped else { return }
            initializationGeneration &+= 1
            initializationPending = true
            initializationError = nil
        }
    }
    func expireInitialization() {
        lock.withLock {
            guard projection == nil else { return }
            initializationGeneration &+= 1
            initializationPending = false
        }
    }
    private func cycleGeneration() -> UInt64? {
        lock.withLock { projection != nil || initializationPending ? initializationGeneration : nil }
    }
    private func publish(_ value: MicrophoneSharingSnapshot, generation: UInt64) -> Bool {
        lock.withLock {
            guard !stopped, generation == initializationGeneration,
                  projection != nil || initializationPending else { return false }
            projection = value
            initializationError = nil
            initializationPending = false
            return true
        }
    }
    private func recordInitializationError(_ message: String, generation: UInt64) {
        lock.withLock {
            guard !stopped, generation == initializationGeneration,
                  projection != nil || initializationPending else { return }
            initializationError = message
        }
    }
    func setLocked(_ value: Bool) { lock.lock(); locked = value; lock.unlock(); if value { engine.stopMicrophone() } }
    func setSleeping(_ value: Bool) { lock.lock(); sleeping = value; lock.unlock(); if value { engine.stopMicrophone() } }
    func stop() { lock.lock(); stopped = true; lock.unlock(); engine.stopMicrophone() }
    private func flags() -> (Bool, Bool) { lock.lock(); defer { lock.unlock() }; return (locked || sleeping || systemLocked, stopped) }
    private func refreshSessionLock() {
        let session = CGSessionCopyCurrentDictionary() as? [String: Any]
        let screenLocked = session?["CGSSessionScreenIsLocked"] as? Bool ?? false
        let onConsole = session?[kCGSessionOnConsoleKey as String] as? Bool ?? false
        lock.lock(); systemLocked = screenLocked || !onConsole; lock.unlock()
    }
    func start() { Thread.detachNewThread { [self] in run() } }
    private func run() {
        var channel: MicrophoneChannel?
        var sender: OpaquePointer?
        var capture: MicrophoneCapture?
        var loopback: MicrophoneLoopback?
        var currentID: UInt64 = 0
        var driver = false
        var readers = false
        var configuredName = ""
        var driverEnabled = false
        var needsDriverCleanup = true
        var failure = ""
        var lastAudio = Date.distantPast
        var remoteLevel: Float = 0
        var captureStarted = Date.distantPast
        var nextPoll = Date.distantPast
        var nextDriverCheck = Date.distantPast
        func nativeError() -> String { String(cString: ud_mic_last_error()) }
        func resetSession() {
            capture?.stop(); capture = nil
            if let sender { ud_mic_sender_destroy(sender) }
            sender = nil; channel?.close(); channel = nil
            if loopback != nil { _ = ud_mic_driver_reset() }
            loopback = nil; currentID = 0; lastAudio = .distantPast; remoteLevel = 0
        }
        func restoreDefaultInput() {
            // Respect a newer choice made by the user or another audio app.
            if let id = MicrophoneAudioDevices.defaultInput(),
               MicrophoneAudioDevices.string(id, kAudioDevicePropertyDeviceUID) == MicrophoneAudioDevices.virtualUID,
               let uid = UserDefaults.standard.string(forKey: "microphone.previousDefaultInput") {
                try? MicrophoneAudioDevices.setDefault(uid: uid)
            }
        }
        defer {
            resetSession()
            if driverEnabled {
                restoreDefaultInput()
                _ = ud_mic_driver_set_enabled(0)
            }
        }
        while !flags().1 {
            autoreleasepool {
                guard let generation = cycleGeneration() else { return }
                do {
                    if Date() >= nextPoll {
                        nextPoll = Date().addingTimeInterval(0.2)
                        refreshSessionLock()
                        let state = try JSONDecoder().decode(MicrophoneSharingSnapshot.self, from: Data(engine.microphoneSnapshot().utf8))
                        let wantsReceiver = state.settings.enabled && !state.settings.source.isEmpty && !flags().0
                        if Date() >= nextDriverCheck && capture == nil {
                            driver = ud_mic_driver_available() == 1
                            nextDriverCheck = Date().addingTimeInterval(2)
                        }
                        if driver {
                            if wantsReceiver {
                                let name = "DeskFuse 麦克风（来自 \(state.source_name)）"
                                if configuredName != name {
                                    resetSession()
                                    guard name.withCString({ ud_mic_driver_configure($0) }) == 0 else { throw MicrophoneAudioError.message(nativeError()) }
                                    configuredName = name
                                }
                                if !driverEnabled {
                                    guard ud_mic_driver_set_enabled(1) == 0 else { throw MicrophoneAudioError.message(nativeError()) }
                                    driverEnabled = true
                                }
                                let reading = ud_mic_driver_has_readers()
                                guard reading >= 0 else { throw MicrophoneAudioError.message(nativeError()) }
                                readers = reading == 1
                            } else {
                                readers = false
                                if driverEnabled || !configuredName.isEmpty || needsDriverCleanup {
                                    resetSession()
                                    // Restore only if our explicit default selection is still active.
                                    restoreDefaultInput()
                                    _ = ud_mic_driver_set_enabled(0); driverEnabled = false; configuredName = ""; needsDriverCleanup = false
                                }
                            }
                        } else { readers = false; driverEnabled = false; configuredName = "" }
                        let permission: String
                        switch AVCaptureDevice.authorizationStatus(for: .audio) {
                        case .authorized: permission = "authorized"
                        case .denied, .restricted: permission = "denied"
                        default: permission = "undetermined"
                        }
                        if let capture, let channel {
                            var bits = capture.level.bitPattern.bigEndian
                            let meter = withUnsafeBytes(of: &bits) { Data([4]) + Data($0) }
                            try channel.send(bytes: meter)
                        }
                        try engine.reportMicrophoneNative(state: MicrophoneNativeState(
                            driver: driver,
                            readers: readers,
                            streaming: Date().timeIntervalSince(lastAudio) < 1,
                            locked: flags().0,
                            permission: permission,
                            level: capture?.level ?? (Date().timeIntervalSince(lastAudio) < 1 ? remoteLevel : 0),
                            error: failure
                        ))
                        failure = ""
                        let latest = try JSONDecoder().decode(MicrophoneSharingSnapshot.self, from: Data(engine.microphoneSnapshot().utf8))
                        // A completion from an expired attempt cannot clear the
                        // timeout or open a session for a later retry.
                        guard publish(latest, generation: generation) else {
                            resetSession()
                            if driverEnabled {
                                restoreDefaultInput()
                                _ = ud_mic_driver_set_enabled(0)
                                driverEnabled = false
                            }
                            return
                        }
                        if currentID != latest.session_id || flags().0 { resetSession() }
                        if latest.session_id != 0 && currentID == 0 && !flags().0 {
                            channel = try engine.microphoneChannel()
                            guard channel?.sessionId() == latest.session_id else { channel?.close(); channel = nil; return }
                            currentID = latest.session_id
                        }
                        if channel != nil && latest.role == "sender" && sender == nil && permission == "authorized" {
                            guard let opened = ud_mic_sender_create() else { throw MicrophoneAudioError.message(nativeError()) }
                            sender = opened
                            let microphone = MicrophoneCapture()
                            try microphone.start(uid: latest.settings.input_uid)
                            capture = microphone; captureStarted = Date()
                        }
                        if channel != nil && latest.role == "receiver" && loopback == nil {
                            guard ud_mic_driver_reset() == 0 else { throw MicrophoneAudioError.message(nativeError()) }
                            loopback = try MicrophoneLoopback()
                        }
                    }
                    if flags().0 { resetSession(); return }
                    if let channel {
                        if let sender, let capture {
                            for samples in try capture.take() {
                                lastAudio = Date()
                                guard samples.withUnsafeBufferPointer({ ud_mic_sender_push(sender, $0.baseAddress, $0.count) }) == 0 else { throw MicrophoneAudioError.message(nativeError()) }
                            }
                            if Date().timeIntervalSince(max(lastAudio, captureStarted)) > 2 { throw MicrophoneAudioError.message(capture.timeoutReason) }
                            for kind in 1...3 {
                                for _ in 0..<16 {
                                    var packet = [UInt8](repeating: 0, count: 4096)
                                    let count = ud_mic_sender_pop(sender, Int32(kind), &packet, packet.count)
                                    guard count >= 0 else { throw MicrophoneAudioError.message(nativeError()) }
                                    if count == 0 { break }
                                    try channel.send(bytes: Data([UInt8(kind)] + packet.prefix(Int(count))))
                                }
                            }
                        }
                        for _ in 0..<24 {
                            guard let packet = try channel.receive() else { break }
                            if let sender {
                                guard packet.first == 3 else { throw MicrophoneAudioError.message("来源端收到无效反馈类型") }
                                let result = packet.dropFirst().withUnsafeBytes { ud_mic_sender_feedback(sender, $0.bindMemory(to: UInt8.self).baseAddress, $0.count) }
                                guard result == 0 else { throw MicrophoneAudioError.message(nativeError()) }
                            } else if let loopback {
                                if packet.first == 4 {
                                    guard packet.count == 5 else { throw MicrophoneAudioError.message("无效的麦克风电平") }
                                    let bits = packet.dropFirst().reduce(UInt32(0)) { ($0 << 8) | UInt32($1) }
                                    let value = Float(bitPattern: bits)
                                    remoteLevel = value.isFinite ? min(1, max(0, value)) : 0
                                } else { try loopback.send(packet); if packet.first == 1 { lastAudio = Date() } }
                            }
                        }
                        if let loopback { for _ in 0..<8 { guard let packet = loopback.feedback() else { break }; try channel.send(bytes: packet) } }
                    }
                } catch {
                    failure = error.userFacingMessage
                    recordInitializationError(failure, generation: generation)
                    if currentID != 0 { engine.failMicrophoneSession(id: currentID, detail: failure) }
                    resetSession()
                    nextPoll = Date().addingTimeInterval(0.5)
                }
            }
            Thread.sleep(forTimeInterval: 0.003)
        }
    }
}
