import SwiftUI
import AppKit
import Observation
import UniDeskEngine
import UniDeskRemoteApplication

/// Media/input data plane never traverses observable UI state or main-actor tasks.
final class SessionDataPlane: @unchecked Sendable {
    let frames = LatestVideoFrame()
    let input: RemoteInputRelay
    private let engine: Engine
    private let peerID: String
    private var lastDiagnostic = 0.0
    private var receivedInput: UInt64 = 0
    init(engine: Engine, peerID: String) {
        self.engine = engine; self.peerID = peerID
        let bridge = RemoteNativeInputBridge(engine: engine, peerID: peerID)
        input = RemoteInputRelay(sender: { bridge.send($0) })
    }
    private let lock = NSLock()
    private var decoder: H264Decoder?
    private var encoder: H264Encoder?
    private var metrics: MediaMetrics?
    private var peer: RemotePeer?
    private var identity: SurfaceIdentity?
    private var presentedSequence: UInt64 = 0
    private var lastPeerHeartbeat = ProcessInfo.processInfo.systemUptime
    private var heartbeatTimer: DispatchSourceTimer?
    private let heartbeatQueue = DispatchQueue(label: "uniDesk.remote.heartbeat", qos: .userInteractive)
    private var enabled = false
    private var sequence: UInt64 = 0
    private var connectionToken = UUID()
    func begin(_ token: UUID) { lock.withLock { connectionToken = token } }
    func isCurrent(_ token: UUID) -> Bool { lock.withLock { connectionToken == token } }
    func setPeer(_ peer: RemotePeer?) { lock.withLock { self.peer = peer } }
    func setEncoder(_ value: H264Encoder?) { lock.withLock { encoder = value } }
    func setMetrics(_ value: MediaMetrics?) { lock.withLock { metrics = value } }
    func requestKey() { lock.withLock { encoder }?.requestKeyFrame() }
    func sendFrame(_ frame: EncodedVideoFrame) {
        input.encoded(frame)
        var message = RemoteMessage(.frame); message.frame = frame
        lock.withLock { peer }?.send(message)
    }
    func configure(surface: CaptureSurface, profile: VideoProfile, metrics: MediaMetrics, token: UUID,
                   failed: @escaping @Sendable (String) -> Void) throws {
        guard profile.pixels.isValid, profile.framesPerSecond.isFinite, (1...120).contains(profile.framesPerSecond) else {
            throw RemoteApplicationError.invalidGeometry
        }
        setMetrics(metrics)
        _ = try CaptureSurface(identity: surface.identity, displayID: surface.displayID, displayFrame: surface.displayFrame,
            contentRect: surface.contentRect, pointPixelScale: surface.pointPixelScale, sourceHz: surface.sourceHz,
            applicationRect: surface.applicationRect, auxiliaryRects: surface.auxiliaryRects)
        let next = H264Decoder(surface: surface.identity, expectedPixels: profile.pixels, metrics: metrics,
            keyFrameNeeded: { [weak self] in self?.send(RemoteMessage(.keyFrame)) }, output: { [weak self] result in
                guard let self, self.lock.withLock({ self.connectionToken == token && self.identity == surface.identity }) else { return }
                switch result {
                case .success(let frame): self.frames.publish(frame)
                case .failure(let error): self.release(); failed(error.localizedDescription)
                }
            })
        var accepted = false
        let old = lock.withLock { () -> H264Decoder? in
            guard connectionToken == token else { return nil }
            accepted = true; frames.transition(to: surface.identity, layout: surface)
            let old = decoder; decoder = next; identity = surface.identity; presentedSequence = 0; enabled = false
            lastPeerHeartbeat = ProcessInfo.processInfo.systemUptime
            return old
        }
        guard accepted else { next.stop(); return }
        old?.stop()
        send(RemoteMessage(.keyFrame))
    }
    func receive(_ message: RemoteMessage, host: Bool) -> Bool {
        if message.kind == .input { lock.withLock { receivedInput += 1 } }
        if message.kind == .heartbeat {
            lock.withLock { lastPeerHeartbeat = ProcessInfo.processInfo.systemUptime }
        }
        if host {
            if [.input, .presented, .heartbeat].contains(message.kind) { input.receive(message); return true }
            if message.kind == .keyFrame { requestKey(); return true }
        } else {
            if message.kind == .frame, let frame = message.frame { lock.withLock { decoder }?.submit(frame); return true }
            if message.kind == .heartbeat { lock.withLock { lastPeerHeartbeat = ProcessInfo.processInfo.systemUptime }; return true }
        }
        return false
    }
    func send(_ message: RemoteMessage) { lock.withLock { peer }?.send(message) }
    func presented(_ surface: SurfaceIdentity, sequence: UInt64) {
        let matches = lock.withLock { () -> Bool in
            guard identity == surface else { return false }; presentedSequence = sequence; return true
        }
        guard matches else { return }
        var message = RemoteMessage(.presented); message.identity = surface; message.sequence = sequence; send(message)
    }
    func setEnabled(_ value: Bool) { lock.withLock { enabled = value } }
    func event(_ event: RemoteInputEvent, surface: SurfaceIdentity) {
        let message = lock.withLock { () -> RemoteMessage? in
            guard enabled, identity == surface, presentedSequence > 0,
                  ProcessInfo.processInfo.systemUptime - lastPeerHeartbeat < 1 else { return nil }
            sequence += 1
            var message = RemoteMessage(.input); message.identity = surface; message.sequence = sequence; message.input = event
            return message
        }
        if let message { send(message) }
        if event.kind == .down || event.kind == .up {
            engine.remoteApplicationDiagnostic(event: "pointer_\(event.kind.rawValue)_\(message == nil ? "gate_rejected" : "queued")",
                detail: "peer=\(peerID) generation=\(surface.generation) sequence=\(message?.sequence ?? 0)")
        }
    }
    var connectionHealthy: Bool {
        lock.withLock { ProcessInfo.processInfo.systemUptime - lastPeerHeartbeat < 1 }
    }
    func startHeartbeats(host: Bool) {
        let timer = DispatchSource.makeTimerSource(queue: heartbeatQueue)
        timer.schedule(deadline: .now(), repeating: .milliseconds(250), leeway: .milliseconds(10))
        timer.setEventHandler { [weak self] in
            guard let self else { return }
            let (message, peer, healthy) = self.lock.withLock { () -> (RemoteMessage, RemotePeer?, Bool) in
                let healthy = ProcessInfo.processInfo.systemUptime - self.lastPeerHeartbeat < 1
                if !healthy { self.enabled = false }
                var message = RemoteMessage(.heartbeat)
                message.identity = self.identity; message.sequence = self.presentedSequence
                return (message, self.peer, healthy)
            }
            if !healthy && host { self.input.focus(false) }
            peer?.send(message)
            let detail = self.lock.withLock { () -> String? in
                let now = ProcessInfo.processInfo.systemUptime
                guard now - self.lastDiagnostic >= 1 else { return nil }
                self.lastDiagnostic = now
                let counters = self.metrics?.snapshot()
                return "peer=\(self.peerID) host=\(host) healthy=\(healthy) enabled=\(self.enabled) sentInput=\(self.sequence) receivedInput=\(self.receivedInput) presented=\(self.presentedSequence) generation=\(self.identity?.generation ?? 0) heartbeatAgeMs=\(Int((now-self.lastPeerHeartbeat)*1000)) captured=\(counters?.captured ?? 0) idle=\(counters?.capturedIdle ?? 0) blank=\(counters?.capturedBlank ?? 0) started=\(counters?.capturedStarted ?? 0) other=\(counters?.capturedOther ?? 0) encoded=\(counters?.encoded ?? 0) decoded=\(counters?.decoded ?? 0) chainResets=\(counters?.chainResets ?? 0)"
            }
            if let detail { self.engine.remoteApplicationDiagnostic(event: "data_progress", detail: detail) }
        }
        lock.withLock {
            heartbeatTimer?.cancel()
            lastPeerHeartbeat = ProcessInfo.processInfo.systemUptime
            heartbeatTimer = timer
        }
        timer.resume()
    }
    func release() { lock.withLock { enabled = false }; send(RemoteMessage(.release)) }
    func reset() {
        input.stop()
        let old = lock.withLock { () -> H264Decoder? in
            heartbeatTimer?.cancel(); heartbeatTimer = nil
            let old = decoder; decoder = nil; encoder = nil; peer = nil; identity = nil; enabled = false; connectionToken = UUID(); frames.reset(nil); return old
        }
        old?.stop(); frames.reset(nil)
    }
}

enum RemoteApplicationRole: String, CaseIterable { case viewer = "A 端 · 控制远端", host = "B 端 · 共享应用" }

@available(macOS 14.2, *)
@MainActor @Observable final class RemoteApplicationSession {
    let id = UUID()
    var choosingApplication = true
    private(set) var viewerPaused = false
    private(set) var awaitingResumeSurface = false
    var autoAcquire = false
    var applicationsLoaded = false
    var sortedApplications: [RemoteApplicationInfo] {
        applications.filter { applicationQuery.isEmpty || $0.name.localizedCaseInsensitiveContains(applicationQuery) }
            .sorted {
                if ($0.processID != nil) != ($1.processID != nil) { return $0.processID != nil }
                return $0.name.localizedStandardCompare($1.name) == .orderedAscending
            }
    }
    var applicationName: String { requestedApplication?.name ?? "远程应用" }
    private var requestedApplication: RemoteApplicationInfo?
    @ObservationIgnored private var reconnectTarget: RemoteApplicationInfo?
    @ObservationIgnored private var startupReconnect: Task<Void, Never>?
    @ObservationIgnored private var startupAttempts = 0
    @ObservationIgnored private var hasPresentedFrame = false
    var applicationKey: String? { requestedApplication?.applicationKey }
    func prepareApplication(_ application: RemoteApplicationInfo) {
        requestedApplication = application; reconnectTarget = application; choosingApplication = false
    }
    func refreshApplications() {
        if connected { plane.send(RemoteMessage(.hello)) }
        else if !busy { Task { await start() } }
    }
    func setViewerVisible(_ visible: Bool) {
        guard role == .viewer, !choosingApplication else { return }
        viewerPaused = !visible
        awaitingResumeSurface = visible
        autoAcquire = false
        if !visible { autoAcquire = false; releaseControl(resumeSharing: true) }
        if connected { plane.send(RemoteMessage(visible ? .resume : .pause)) }
    }
    func selectApplication(_ id: UUID) {
        requestedApplication = applications.first { $0.id == id }
        selected = id; choosingApplication = false; launchSelected()
    }
    func reconnectApplication() {
        guard !busy else { return }
        startupAttempts = 0
        reconnectTarget = requestedApplication
        Task { await start() }
    }
    func didPresent(_ size: CGSize) {
        hasPresentedFrame = true; startupAttempts = 0
        firstFrameWatch?.cancel(); firstFrameWatch = nil
        presentationSizeChanged?(size)
    }
    /// Only failed startup connections retry automatically. A stopped/closed
    /// window cancels this task; reconnect never replays queued input.
    private func recoverStartupConnection() -> Bool {
        guard role == .viewer, let requestedApplication, !hasPresentedFrame, startupAttempts < 3 else { return false }
        startupAttempts += 1
        reconnectTarget = requestedApplication
        failure = nil; status = "正在自动重新连接远程应用…"
        let token = operation, delay = Double(startupAttempts) * 0.5
        startupReconnect = Task { [weak self] in
            do { try await Task.sleep(for: .seconds(delay)) } catch { return }
            guard let self, self.operation == token, !Task.isCancelled else { return }
            self.startupReconnect = nil
            await self.start()
        }
        return true
    }
    func showApplications() {
        RemoteApplicationManager.shared.openPicker(peerID: peerID, peerName: peerName)
    }
    func yieldHostControl() {
        guard role == .host else { return }
        controlRequest = UUID(); controlling = false; plane.input.focus(false)
        RemoteApplicationManager.shared.releaseHostControl(self)
        var message = RemoteMessage(.release); message.text = "控制已交给其他应用窗口，点击画面可切回"
        plane.send(message)
    }
    private func denyControl(_ reason: String) {
        controlling = false; plane.input.focus(false)
        RemoteApplicationManager.shared.releaseHostControl(self)
        var response = RemoteMessage(.release); response.text = reason; plane.send(response)
        status = reason; diagnostic("control_denied")
        engine.remoteApplicationDiagnostic(event: "control_denied_reason", detail: "peer=\(peerID) reason=\(String(reason.prefix(300)))")
    }
    var role: RemoteApplicationRole = .viewer
    let engine: Engine
    let peerID: String
    let peerName: String
    init(engine: Engine, peerID: String, peerName: String, role: RemoteApplicationRole) {
        self.engine = engine; self.peerID = peerID; self.peerName = peerName; self.role = role
        plane = SessionDataPlane(engine: engine, peerID: peerID)
    }
    var applications: [RemoteApplicationInfo] = []
    var selected: UUID?
    var applicationQuery = ""
    var preferences = VideoPreferences()
    var scale: VideoViewScale = .fit
    var presentationSurface: CaptureSurface?
    @ObservationIgnored var presentationSizeChanged: ((CGSize) -> Void)?
    var status = "正在连接已配对设备"
    var failure: String?
    var sourceDescription = "尚未采集"
    var actualDescription = "尚未协商"
    var statistics = MediaStatistics()
    var effectiveFPS: Double = 0
    var active = false
    var connected = false
    var running = false
    var busy = false
    var controlling = false
    var allowDesktop = false
    var desktop = false
    @ObservationIgnored let plane: SessionDataPlane
    @ObservationIgnored var metrics = MediaMetrics()
    @ObservationIgnored private let catalog = ApplicationCatalog()
    @ObservationIgnored private var capture = RemoteAppCapture()
    @ObservationIgnored private var encoder: H264Encoder?
    @ObservationIgnored private var peer: RemotePeer?
    @ObservationIgnored private var monitor: Task<Void, Never>?
    @ObservationIgnored private var operation = UUID()
    @ObservationIgnored private var generation: UInt64 = 0
    @ObservationIgnored private var process: NSRunningApplication?
    @ObservationIgnored private var capturePaused = false
    @ObservationIgnored private var resumePending = false
    @ObservationIgnored private var lastSurface: CaptureSurface?
    @ObservationIgnored private var lastProfile: VideoProfile?
    @ObservationIgnored private var lastMode: CaptureViewMode?
    @ObservationIgnored private var changing = false
    @ObservationIgnored private var controlPending = false
    @ObservationIgnored private var controlRequest = UUID()
    @ObservationIgnored private var pendingClick: [(RemoteInputEvent, SurfaceIdentity)] = []
    @ObservationIgnored private var firstFrameWatch: Task<Void, Never>?

    private func diagnostic(_ event: String) {
        engine.remoteApplicationDiagnostic(event: event,
            detail: "peer=\(peerID) host=\(role == .host) session=\(operation.uuidString) generation=\(lastSurface?.identity.generation ?? 0) controlling=\(controlling) pending=\(controlPending)")
    }
    private func layoutDiagnostic(_ surface: CaptureSurface) {
        func rect(_ value: CGRect) -> String {
            "\(Int(value.minX)),\(Int(value.minY)),\(Int(value.width)),\(Int(value.height))"
        }
        engine.remoteApplicationDiagnostic(event: "surface_layout",
            detail: "peer=\(peerID) host=\(role == .host) generation=\(surface.identity.generation) capture=\(rect(surface.contentRect)) application=\(surface.applicationRect.map(rect) ?? "missing") auxiliary=\((surface.auxiliaryRects ?? []).map(rect).joined(separator: ";"))")
    }
    func start(channel incoming: RemoteApplicationChannel? = nil) async {
        guard !busy else { return }
        busy = true; defer { busy = false }
        await stop(); failure = nil; viewerPaused = false; awaitingResumeSurface = false
        hasPresentedFrame = false
        capturePaused = false; resumePending = false
        operation = UUID(); let token = operation; plane.begin(token)
        metrics = MediaMetrics(); statistics = MediaStatistics()
        plane.setMetrics(metrics)
        do {
            if role == .host {
                guard RemoteAppCapture.hasScreenRecordingPermission else {
                    incoming?.close()
                    throw RemoteApplicationError.permission("请在被控 Mac 上允许 uniDesk 屏幕录制权限")
                }
                applications = catalog.applications().filter { $0.processID != ProcessInfo.processInfo.processIdentifier }
            }
            let channel: RemoteApplicationChannel
            if let incoming { channel = incoming }
            else {
                let engine = engine, peerID = peerID
                channel = try await Task.detached { try engine.openRemoteApplication(peerId: peerID) }.value
            }
            guard operation == token else { channel.close(); return }
            active = true; attach(channel, token: token)
            monitor = Task { [weak self] in
                var previous: UInt64 = 0, ticks = 0
                var time = ProcessInfo.processInfo.systemUptime
                while !Task.isCancelled {
                    do { try await Task.sleep(for: .milliseconds(250)) } catch { return }
                    guard let self, self.operation == token else { return }
                    if self.connected {
                        if self.role == .viewer && self.controlling && (try? self.engine.remoteApplicationViewerReady()) != true {
                            self.diagnostic("viewer_pause_lost")
                            self.releaseControl(); self.status = "本机已收回控制"
                        }
                        if !self.plane.connectionHealthy, self.controlling {
                            self.diagnostic("peer_heartbeat_timeout")
                            self.releaseControl(); self.status = "画面链路无响应，已释放控制"
                        }
                        if self.role == .host && self.controlling &&
                            (!self.plane.connectionHealthy ||
                             NSWorkspace.shared.frontmostApplication?.processIdentifier != self.process?.processIdentifier ||
                             self.lastSurface.map { (try? self.engine.remoteApplicationControlReady(peerId: self.peerID, session: $0.identity.session.uuidString, generation: $0.identity.generation)) != true } == true) {
                            self.diagnostic("host_control_lost")
                            self.controlRequest = UUID(); self.controlling = false; self.plane.input.focus(false)
                            RemoteApplicationManager.shared.releaseHostControl(self)
                            var release = RemoteMessage(.release)
                            release.text = self.controlProblem()
                            self.status = release.text!
                            self.plane.send(release)
                        }
                    }
                    if self.role == .host && !self.busy && !self.changing {
                        if self.capturePaused && self.running {
                            self.busy = true
                            await self.stopCapture()
                            self.busy = false
                        } else if !self.capturePaused && self.resumePending {
                            self.resumePending = false; self.busy = true
                            do {
                                await self.stopCapture()
                                guard self.operation == token else { self.busy = false; return }
                                guard let process = self.process, !process.isTerminated else { throw RemoteApplicationError.noWindow }
                                try self.plane.input.start(processID: process.processIdentifier)
                                self.running = true
                                try await self.refreshCapture(token: token)
                            } catch { await self.stopCapture(); self.sendError(error.localizedDescription) }
                            self.busy = false
                        }
                    }
                    if self.role == .host && self.running && !self.capturePaused && !self.busy {
                        if self.process?.isTerminated != false { await self.stopCapture(); self.sendError("B 端应用已退出"); continue }
                        do { try await self.refreshCapture(token: token) }
                        catch { await self.stopCapture(); self.sendError(error.localizedDescription) }
                    }
                    ticks += 1
                    guard ticks % 4 == 0 else { continue }
                    let now = ProcessInfo.processInfo.systemUptime
                    self.statistics = self.metrics.snapshot()
                    self.effectiveFPS = Double(self.statistics.presented - previous) / max(0.001, now - time)
                    previous = self.statistics.presented; time = now
                }
            }
        } catch {
            guard operation == token else { return }
            await stop()
            if !recoverStartupConnection() { status = error.localizedDescription; failure = status }
        }
    }
    private func attach(_ channel: RemoteApplicationChannel, token: UUID) {
        let host = role == .host, plane = plane, metrics = metrics
        let remote = RemotePeer(channel: channel, receive: { [weak self] message in
            guard plane.isCurrent(token) else { return }
            if message.kind == .surface, !host, let surface = message.surface, let profile = message.profile {
                do {
                    try plane.configure(surface: surface, profile: profile, metrics: metrics, token: token) { [weak self] error in
                        Task { @MainActor [weak self] in
                            guard let self, self.operation == token else { return }
                            self.releaseControl(); self.status = error
                        }
                    }
                } catch {
                    plane.release()
                    Task { @MainActor [weak self] in self?.status = error.localizedDescription }; return
                }
            }
            if plane.receive(message, host: host) {
                return
            }
            Task { @MainActor [weak self] in
                guard let self, self.operation == token else { return }
                await self.receive(message, token: token)
            }
        }, state: { [weak self] message, ready in
            Task { @MainActor [weak self] in
                guard let self, self.operation == token else { return }
                self.diagnostic(ready ? "transport_ready" : "transport_closed")
                if !ready {
                    self.engine.remoteApplicationDiagnostic(event: "transport_close_reason",
                        detail: "peer=\(self.peerID) reason=\(String(message.prefix(300)))")
                }
                self.connected = ready; self.status = message
                if ready {
                    if host { var value = RemoteMessage(.applications); value.applications = self.applications; self.plane.send(value) }
                    else {
                        self.plane.send(RemoteMessage(.hello))
                    }
                } else {
                    let hadApplications = !self.applications.isEmpty
                    await self.stop()
                    if self.recoverStartupConnection() { return }
                    self.status = !host && !hadApplications
                        ? "连接未建立：请确认两端均已更新，并在被控 Mac 的本机设备页开启远程应用、授予屏幕录制权限。\n\(message)"
                        : message
                    self.failure = self.status
                }
            }
        }, keyNeeded: { plane.requestKey() })
        peer = remote; plane.setPeer(remote); plane.startHeartbeats(host: host); remote.start()
    }
    private func receive(_ message: RemoteMessage, token: UUID) async {
        diagnostic("receive_\(message.kind.rawValue)")
        if role == .host {
            switch message.kind {
            case .hello:
                applications = catalog.applications().filter { $0.processID != ProcessInfo.processInfo.processIdentifier }
                var value = RemoteMessage(.applications); value.applications = applications; plane.send(value)
            case .launch:
                guard !busy, let application = message.application else { return }
                busy = true; defer { busy = false }
                await stopCapture()
                do {
                    guard RemoteApplicationManager.shared.claimHostControl(self) else {
                        throw RemoteApplicationError.unsupported("另一台设备正在控制本机，请先释放控制")
                    }
                    if let preferences = message.preferences { self.preferences = preferences }
                    process = try await catalog.launch(application)
                    guard operation == token, let process else { return }
                    guard !capturePaused else { return }
                    _ = process.activate(options: [.activateAllWindows])
                    // Launch completion can precede the first visible window.
                    // Give LaunchServices a short bounded window to finish
                    // activation before ScreenCaptureKit enumerates windows.
                    for _ in 0..<20 {
                        guard operation == token, !process.isTerminated else { throw RemoteApplicationError.noWindow }
                        if NSWorkspace.shared.frontmostApplication?.processIdentifier == process.processIdentifier { break }
                        try? await Task.sleep(for: .milliseconds(50))
                    }
                    try plane.input.start(processID: process.processIdentifier)
                    running = true
                    try await waitForInitialCapture(token: token)
                } catch { await stopCapture(); sendError(error.localizedDescription) }
            case .acquire:
                guard running, message.identity == lastSurface?.identity, let process else {
                    denyControl("画面配置正在变化，请等待新画面后重试"); return
                }
                guard RemoteApplicationManager.shared.claimHostControl(self) else {
                    denyControl("另一台设备正在控制本机，请先释放控制"); return
                }
                let request = UUID(); controlRequest = request
                if NSWorkspace.shared.frontmostApplication?.processIdentifier != process.processIdentifier {
                    _ = process.activate(options: [.activateAllWindows])
                }
                // Activation is asynchronous. Never arm against a different foreground app.
                for _ in 0..<10 {
                    if NSWorkspace.shared.frontmostApplication?.processIdentifier == process.processIdentifier { break }
                    try? await Task.sleep(for: .milliseconds(30))
                }
                guard operation == token, running, controlRequest == request,
                      NSWorkspace.shared.frontmostApplication?.processIdentifier == process.processIdentifier else {
                    if operation == token, controlRequest == request { denyControl("无法激活远端应用，请在 B 端确认应用窗口可用后重试") }
                    return
                }
                plane.input.focus(true)
                let engine = engine, peerID = peerID, surface = lastSurface!.identity
                var armed = false
                for _ in 0..<20 {
                    armed = (try? await Task.detached {
                        try engine.remoteApplicationControlReady(peerId: peerID, session: surface.session.uuidString, generation: surface.generation)
                    }.value) == true
                    if armed { break }
                    try? await Task.sleep(for: .milliseconds(50))
                }
                guard operation == token, controlRequest == request, armed else {
                    diagnostic("control_arm_failed")
                    if operation == token, controlRequest == request {
                        denyControl(controlProblem())
                    }
                    return
                }
                controlling = true
                var ack = RemoteMessage(.controlReady); ack.identity = lastSurface?.identity; plane.send(ack)
            case .release:
                controlRequest = UUID(); controlling = false; plane.input.focus(false)
                RemoteApplicationManager.shared.releaseHostControl(self)
            case .pause:
                capturePaused = true; resumePending = false; yieldHostControl()
            case .resume:
                capturePaused = false; resumePending = true
            case .preferences:
                if let preferences = message.preferences { self.preferences = preferences }
            case .stop: await stopCapture()
            default: break
            }
        } else {
            switch message.kind {
            case .applications:
                applications = message.applications ?? []; applicationsLoaded = true
                status = "选择应用即可进入远程交互"
                if let target = reconnectTarget {
                    reconnectTarget = nil
                    if let app = applications.first(where: {
                        target.bundleIdentifier != nil ? $0.bundleIdentifier == target.bundleIdentifier : $0.name == target.name
                    }) { selectApplication(app.id) }
                    else { choosingApplication = true; status = "原应用已不可用，请重新选择" }
                }
            case .surface:
                guard let surface = message.surface, let profile = message.profile else { return }
                layoutDiagnostic(surface)
                autoAcquire = !viewerPaused && (autoAcquire || controlling || controlPending || awaitingResumeSurface)
                awaitingResumeSurface = false
                releaseControl(); lastSurface = surface; presentationSurface = surface; running = true; failure = nil
                presentationSizeChanged?((surface.applicationRect ?? surface.contentRect).size)
                sourceDescription = "B 源：\(Int(surface.contentRect.width)) × \(Int(surface.contentRect.height)) pt · \(surface.nativePixels.label) px"
                actualDescription = "传输：\(profile.pixels.label) · \(String(format: "%.1f", profile.framesPerSecond)) Hz\(profile.reason.map { " · " + $0 } ?? "")"
                status = "远端画面已配置，点击画面获取控制"
                // A configured surface with no arriving frame currently looks
                // identical to a live stream. Surface the stall, nudge a fresh
                // key frame, and leave a diagnostic breadcrumb.
                firstFrameWatch?.cancel()
                firstFrameWatch = Task { [weak self] in
                    guard let self else { return }
                    try? await Task.sleep(for: .seconds(10))
                    guard !Task.isCancelled, self.operation == token, self.running, !self.hasPresentedFrame,
                          !self.controlling, !self.controlPending, self.failure == nil, self.connected else { return }
                    self.status = "正在等待远端画面…"
                    self.plane.send(RemoteMessage(.keyFrame))
                    self.engine.remoteApplicationDiagnostic(event: "first_frame_timeout",
                        detail: "peer=\(self.peerID) generation=\(surface.identity.generation)")
                }
            case .layout:
                guard let surface = message.surface, surface.identity == lastSurface?.identity,
                      surface.contentRect == lastSurface?.contentRect else { return }
                guard let validated = try? surface.withIdentity(surface.identity) else { return }
                layoutDiagnostic(validated)
                plane.frames.updateLayout(validated)
                lastSurface = validated; presentationSurface = validated
            case .controlReady:
                guard controlPending, message.identity == lastSurface?.identity else { return }
                controlPending = false; controlling = true; plane.setEnabled(true)
                for (event, identity) in pendingClick { plane.event(event, surface: identity) }
                pendingClick.removeAll(); status = "正在控制 B 端 · ⌃⌥Esc 或点击画面外释放"
            case .release:
                releaseControl(); status = message.text ?? "B 端已释放控制，点击画面重新申请"
            case .error:
                releaseControl(); running = false; plane.frames.reset(nil); status = message.text ?? "远端错误"
                failure = status
            default: break
            }
        }
    }
    func launchSelected() {
        guard connected, role == .viewer, let selected else { return }
        releaseControl(); autoAcquire = !viewerPaused; running = false; failure = nil
        plane.frames.reset(nil)
        var message = RemoteMessage(.launch); message.application = selected; message.preferences = preferences; plane.send(message)
        status = "正在启动远端应用"
    }
    func changePreferences() {
        guard role == .viewer, connected else { return }
        releaseControl(); var message = RemoteMessage(.preferences); message.preferences = preferences; plane.send(message)
    }
    func canvasFocus(_ focused: Bool) {
        diagnostic(focused ? "canvas_focus" : "canvas_blur")
        if !focused { autoAcquire = false; releaseControl(resumeSharing: true); return }
        guard role == .viewer, running, !viewerPaused, !awaitingResumeSurface,
              !controlPending, !controlling, let surface = lastSurface else { return }
        status = "正在申请控制权…"
        do { try RemoteApplicationManager.shared.claimViewerControl(self) }
        catch { status = error.localizedDescription; return }
        controlPending = true; pendingClick.removeAll()
        let request = UUID(); controlRequest = request
        let engine = engine
        Task {
            var ready = false
            for _ in 0..<20 {
                ready = (try? await Task.detached { try engine.remoteApplicationViewerReady() }.value) == true
                if ready { break }
                try? await Task.sleep(for: .milliseconds(50))
            }
            guard controlPending, controlRequest == request else { return }
            guard ready else { releaseControl(); status = "请先结束当前键鼠控制或更新操作后重试"; return }
            var message = RemoteMessage(.acquire); message.identity = surface.identity; plane.send(message)
            try? await Task.sleep(for: .seconds(3))
            if controlPending, controlRequest == request {
                releaseControl(); status = "控制申请未得到响应，请点击画面重试"; diagnostic("acquire_timeout")
            }
        }
    }
    func canvasInput(_ event: RemoteInputEvent, surface: SurfaceIdentity) {
        if event.kind == .down || event.kind == .up { diagnostic("canvas_\(event.kind.rawValue)") }
        if controlPending {
            // Pointer motion is not an input edge and must not cancel the handshake.
            if event.kind == .move { return }
            if pendingClick.count < 32 { pendingClick.append((event, surface)) }
            else { releaseControl() }
            return
        }
        // Clicking again after emergency release requests a fresh lease.
        if !controlling && event.kind == .down { canvasFocus(true); pendingClick.append((event, surface)); return }
        plane.event(event, surface: surface)
    }
    func canvasInputDiagnostic(_ event: String) { diagnostic(event) }
    private func controlProblem() -> String {
        guard let surface = lastSurface else { return "远端画面已失效，请重新选择应用" }
        if NSWorkspace.shared.frontmostApplication?.processIdentifier != process?.processIdentifier {
            return "被控 Mac 的前台应用已切换，点击画面重新获取控制"
        }
        return (try? engine.remoteApplicationControlProblem(peerId: peerID,
            session: surface.identity.session.uuidString, generation: surface.identity.generation))
            ?? "被控 Mac 的输入进程状态不可用，请重新连接"
    }
    func releaseControl(resumeSharing: Bool = true) {
        diagnostic("release_control")
        controlRequest = UUID()
        // Explicit release, errors, timeout and blur all restore sharing now.
        // Only an ownership handoff may deliberately retain the global pause.
        if role == .viewer && resumeSharing { RemoteApplicationManager.shared.releaseViewerControl(self) }
        controlPending = false; pendingClick.removeAll(); controlling = false; plane.release()
    }
    private func waitForInitialCapture(token: UUID) async throws {
        // NSWorkspace launch completion precedes creation of the first window
        // and ScreenCaptureKit's shareable-content registration.
        let deadline = ProcessInfo.processInfo.systemUptime + 30
        while operation == token && !capturePaused && !Task.isCancelled {
            guard process?.isTerminated == false else { throw RemoteApplicationError.noWindow }
            do {
                try await refreshCapture(token: token)
                if lastSurface != nil { return }
            } catch RemoteApplicationError.noWindow {
                if ProcessInfo.processInfo.systemUptime >= deadline { throw RemoteApplicationError.noWindow }
            }
            guard ProcessInfo.processInfo.systemUptime < deadline else { throw RemoteApplicationError.noWindow }
            try await Task.sleep(for: .milliseconds(250))
        }
    }
    private func refreshCapture(token: UUID) async throws {
        guard !changing, !capturePaused, let process, operation == token else { return }
        changing = true; defer { changing = false }
        let mode: CaptureViewMode = desktop && allowDesktop ? .desktop : .application
        let identity = SurfaceIdentity(session: token, epoch: 1, generation: generation + 1, configuration: generation + 1)
        let source = try await capture.source(processID: process.processIdentifier, displayID: nil, mode: mode,
            identity: identity, gestureActive: plane.input.gestureActive)
        engine.remoteApplicationDiagnostic(event: "capture_source", detail: "peer=\(peerID) \(source.note)")
        guard operation == token, !capturePaused else { return }
        let surface = source.surface
        let profile = try VideoProfile.requested(native: surface.nativePixels, sourceHz: surface.sourceHz, preferences: preferences)
        if encoder != nil, lastSurface?.contentRect == surface.contentRect, lastSurface?.displayID == surface.displayID,
           lastSurface?.pointPixelScale == surface.pointPixelScale, lastSurface?.sourceHz == surface.sourceHz,
           lastProfile == profile, lastMode == mode {
            try await capture.updateFilter(source)
            if let previous = lastSurface,
               previous.applicationRect != surface.applicationRect || previous.auxiliaryRects != surface.auxiliaryRects {
                let layout = try surface.withIdentity(previous.identity)
                lastSurface = layout
                layoutDiagnostic(layout)
                var message = RemoteMessage(.layout); message.surface = layout; plane.send(message)
            }
            return
        }
        controlRequest = UUID(); controlling = false; plane.input.focus(false)
        plane.input.suspend(); await capture.stop()
        let old = encoder; encoder = nil; plane.setEncoder(nil)
        if let old { await Task.detached { old.stop() }.value }
        guard operation == token else { return }
        let plane = plane, metrics = metrics
        let next = try await Task.detached { [weak self] in
            try H264Encoder(surface: identity, profile: profile, metrics: metrics) { [weak self] result in
                guard plane.isCurrent(token) else { return }
                switch result {
                case .success(let frame): plane.sendFrame(frame)
                case .failure(let error):
                    plane.input.suspend()
                    Task { @MainActor [weak self] in
                        guard let self, self.operation == token else { return }
                        self.sendError(error.localizedDescription)
                    }
                }
            }
        }.value
        guard operation == token, !capturePaused else { next.stop(); return }
        encoder = next; plane.setEncoder(next); plane.input.setSurface(surface)
        // Publish host control geometry before A can present and acknowledge
        // the new stream. startCapture can suspend after delivering a first frame.
        generation += 1; lastSurface = surface; lastProfile = profile; lastMode = mode
        layoutDiagnostic(surface)
        var message = RemoteMessage(.surface); message.surface = surface; message.profile = profile; plane.send(message)
        try await capture.start(source: source, profile: profile, metrics: metrics, sink: { next.submit($0) }, failed: { [weak self] message in
            plane.input.suspend()
            Task { @MainActor [weak self] in
                guard let self, self.operation == token else { return }
                await self.stopCapture(); self.sendError(message)
            }
        })
        guard operation == token else { return }
        sourceDescription = "源：\(surface.nativePixels.label) px"
        actualDescription = "编码：\(profile.pixels.label) · \(String(format: "%.1f", profile.framesPerSecond)) Hz"
        status = "正在共享 \(process.localizedName ?? "应用") · 在 A 端点击画面操作"
    }
    private func sendError(_ text: String) {
        status = text; var message = RemoteMessage(.error); message.text = text; plane.send(message)
    }
    private func stopCapture() async {
        running = false; controlRequest = UUID(); plane.input.stop(); await capture.stop()
        if role == .host { RemoteApplicationManager.shared.releaseHostControl(self) }
        let old = encoder; encoder = nil; plane.setEncoder(nil)
        if let old { await Task.detached { old.stop() }.value }
        lastSurface = nil; lastProfile = nil; lastMode = nil
    }
    func stop() async {
        startupReconnect?.cancel(); startupReconnect = nil
        firstFrameWatch?.cancel(); firstFrameWatch = nil
        operation = UUID(); monitor?.cancel(); monitor = nil
        releaseControl(resumeSharing: true); peer?.stop(); peer = nil
        active = false; connected = false
        presentationSurface = nil
        await stopCapture(); plane.reset(); applications = []; applicationsLoaded = false; selected = nil
        status = "已断开；B 端应用继续运行"; failure = nil
    }
}

@available(macOS 14.2, *)
struct VideoCanvas: NSViewRepresentable {
    let session: RemoteApplicationSession
    func makeNSView(context: Context) -> RemoteVideoView { RemoteVideoView() }
    func updateNSView(_ view: RemoteVideoView, context: Context) {
        view.frames = session.plane.frames; view.metrics = session.metrics
        view.surfaceLayout = session.presentationSurface
        view.onContentSize = { [weak session] in session?.didPresent($0) }
        view.inputEnabled = session.role == .viewer && session.running && !session.viewerPaused && !session.awaitingResumeSurface
        view.onFocus = { [weak session] in session?.canvasFocus($0) }
        view.onInput = { [weak session] in session?.canvasInput($0, surface: $1) }
        view.onInputDiagnostic = { [weak session] in session?.canvasInputDiagnostic($0) }
        view.requestControlOnNextPresentation = session.autoAcquire
        view.onAutomaticControl = { [weak session] in
            guard let session else { return }
            session.autoAcquire = false; session.canvasFocus(true)
        }
        let plane = session.plane
        view.onPresented = { plane.presented($0, sequence: $1) }
        if view.scaleMode != session.scale { view.scaleMode = session.scale }
    }
}
