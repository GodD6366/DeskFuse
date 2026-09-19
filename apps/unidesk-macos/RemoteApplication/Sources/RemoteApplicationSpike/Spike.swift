import SwiftUI
import AppKit
import Observation
import Network
import UniDeskRemoteApplication

/// Media/input data plane never traverses observable UI state or main-actor tasks.
final class SessionDataPlane: @unchecked Sendable {
    let frames = LatestVideoFrame()
    let input = RemoteInputRelay()
    private let lock = NSLock()
    private var decoder: VideoDecoder?
    private var encoder: VideoEncoder?
    private var peer: RemotePeer?
    private var identity: SurfaceIdentity?
    private var presentedSequence: UInt64 = 0
    private var lastHostHeartbeat = ProcessInfo.processInfo.systemUptime
    private var enabled = false
    private var sequence: UInt64 = 0
    private var connectionToken = UUID()
    func begin(_ token: UUID) { lock.withLock { connectionToken = token } }
    func isCurrent(_ token: UUID) -> Bool { lock.withLock { connectionToken == token } }
    func setPeer(_ peer: RemotePeer?) { lock.withLock { self.peer = peer } }
    func setEncoder(_ value: VideoEncoder?) { lock.withLock { encoder = value } }
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
        _ = try CaptureSurface(identity: surface.identity, displayID: surface.displayID, displayFrame: surface.displayFrame,
            contentRect: surface.contentRect, pointPixelScale: surface.pointPixelScale, sourceHz: surface.sourceHz)
        let next = VideoDecoder(surface: surface.identity, expectedPixels: profile.pixels, expectedCodec: profile.codec, metrics: metrics,
            keyFrameNeeded: { [weak self] in self?.send(RemoteMessage(.keyFrame)) }, output: { [weak self] result in
                switch result {
                case .success(let frame): self?.frames.publish(frame)
                case .failure(let error): self?.release(); failed(error.localizedDescription)
                }
            })
        var accepted = false
        let old = lock.withLock { () -> VideoDecoder? in
            guard connectionToken == token else { return nil }
            accepted = true; frames.reset(surface.identity)
            let old = decoder; decoder = next; identity = surface.identity; presentedSequence = 0; enabled = false
            lastHostHeartbeat = ProcessInfo.processInfo.systemUptime
            return old
        }
        guard accepted else { next.stop(); return }
        old?.stop()
        send(RemoteMessage(.keyFrame))
    }
    func receive(_ message: RemoteMessage, host: Bool) -> Bool {
        if host {
            if [.input, .presented, .heartbeat].contains(message.kind) { input.receive(message); return true }
            if message.kind == .keyFrame { requestKey(); return true }
        } else {
            if message.kind == .frame, let frame = message.frame { lock.withLock { decoder }?.submit(frame); return true }
            if message.kind == .heartbeat { lock.withLock { lastHostHeartbeat = ProcessInfo.processInfo.systemUptime }; return true }
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
                  ProcessInfo.processInfo.systemUptime - lastHostHeartbeat < 1 else { return nil }
            sequence += 1
            var message = RemoteMessage(.input); message.identity = surface; message.sequence = sequence; message.input = event
            return message
        }
        if let message { send(message) }
    }
    func heartbeat(host: Bool) -> Bool {
        let message = lock.withLock { () -> RemoteMessage? in
            if !host && ProcessInfo.processInfo.systemUptime - lastHostHeartbeat >= 1 { enabled = false; return nil }
            var message = RemoteMessage(.heartbeat); message.identity = identity; message.sequence = presentedSequence
            return message
        }
        if let message { send(message); return true }
        release(); return false
    }
    func release() { lock.withLock { enabled = false }; send(RemoteMessage(.release)) }
    func reset() {
        input.stop()
        let old = lock.withLock { () -> VideoDecoder? in
            let old = decoder; decoder = nil; encoder = nil; peer = nil; identity = nil; enabled = false; connectionToken = UUID(); frames.reset(nil); return old
        }
        old?.stop(); frames.reset(nil)
    }
}

enum SpikeRole: String, CaseIterable { case viewer = "A 端 · 控制远端", host = "B 端 · 共享应用" }

@MainActor @Observable final class SpikeSession {
    var role: SpikeRole = .viewer
    var address = ""
    var localAddresses: String {
        var pointer: UnsafeMutablePointer<ifaddrs>?
        guard getifaddrs(&pointer) == 0 else { return ProcessInfo.processInfo.hostName }
        defer { freeifaddrs(pointer) }
        var result: [String] = []
        var current = pointer
        while let item = current {
            defer { current = item.pointee.ifa_next }
            guard let address = item.pointee.ifa_addr, address.pointee.sa_family == UInt8(AF_INET),
                  item.pointee.ifa_flags & UInt32(IFF_LOOPBACK) == 0 else { continue }
            var buffer = [CChar](repeating: 0, count: Int(NI_MAXHOST))
            if getnameinfo(address, socklen_t(address.pointee.sa_len), &buffer, socklen_t(buffer.count), nil, 0, NI_NUMERICHOST) == 0 {
                result.append(String(cString: buffer))
            }
        }
        return result.isEmpty ? ProcessInfo.processInfo.hostName : result.joined(separator: " / ")
    }
    var code = ""
    var applications: [RemoteApplicationInfo] = []
    var selected: UUID?
    var applicationQuery = ""
    var preferences = VideoPreferences()
    private var remoteCapabilities = MediaCapabilities.h264Only
    var scale: VideoViewScale = .fit
    var status = "两台 Mac 打开此测试程序；先在 B 端开始共享"
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
    @ObservationIgnored let plane = SessionDataPlane()
    @ObservationIgnored var metrics = MediaMetrics()
    @ObservationIgnored private let catalog = ApplicationCatalog()
    @ObservationIgnored private var capture = RemoteAppCapture()
    @ObservationIgnored private var encoder: VideoEncoder?
    @ObservationIgnored private var listener: RemoteListener?
    @ObservationIgnored private var peer: RemotePeer?
    @ObservationIgnored private var monitor: Task<Void, Never>?
    @ObservationIgnored private var operation = UUID()
    @ObservationIgnored private var generation: UInt64 = 0
    @ObservationIgnored private var process: NSRunningApplication?
    @ObservationIgnored private var lastSurface: CaptureSurface?
    @ObservationIgnored private var lastProfile: VideoProfile?
    @ObservationIgnored private var lastMode: CaptureViewMode?
    @ObservationIgnored private var changing = false
    @ObservationIgnored private var controlPending = false
    @ObservationIgnored private var controlRequest = UUID()
    @ObservationIgnored private var pendingClick: [(RemoteInputEvent, SurfaceIdentity)] = []
    @ObservationIgnored private var lastClientHeartbeat = ProcessInfo.processInfo.systemUptime
    static let port: UInt16 = 42427

    func start() async {
        guard !busy else { return }
        busy = true; defer { busy = false }
        await stop()
        operation = UUID(); let token = operation; plane.begin(token)
        metrics = MediaMetrics(); statistics = MediaStatistics()
        do {
            if role == .host {
                guard RemoteAppCapture.hasScreenRecordingPermission else {
                    throw RemoteApplicationError.permission("请先允许 B 端屏幕录制权限，授权后重新打开此程序")
                }
                guard RemoteInputRelay.authorized else {
                    RemoteInputRelay.requestPermission()
                    throw RemoteApplicationError.permission("请允许 B 端辅助功能权限，再开始共享")
                }
                guard !NSWorkspace.shared.runningApplications.contains(where: { $0.bundleIdentifier == "com.unidesk.app" }) else {
                    throw RemoteApplicationError.unsupported("请先退出 B 端 uniDesk 主程序，避免与跨屏键鼠共享争用输入")
                }
                let tls = try await Task.detached { try RemoteTLSIdentity() }.value
                guard operation == token else { return }
                code = tls.code
                applications = catalog.applications().filter { $0.processID != ProcessInfo.processInfo.processIdentifier }
                listener = try RemoteListener(port: Self.port, identity: tls, accepted: { [weak self] connection in
                    Task { @MainActor [weak self] in
                        guard let self, self.operation == token, self.peer == nil else { connection.cancel(); return }
                        self.attach(connection, token: token)
                    }
                }, status: { [weak self] message in
                    Task { @MainActor [weak self] in if self?.operation == token { self?.status = message } }
                })
                status = "正在开启共享"
            } else {
                let host = address.trimmingCharacters(in: .whitespacesAndNewlines)
                guard !host.isEmpty else { throw RemoteApplicationError.unsupported("请输入 B 端局域网 IP 或主机名") }
                let connection = NWConnection(host: NWEndpoint.Host(host), port: NWEndpoint.Port(rawValue: Self.port)!,
                                              using: try RemoteWire.parameters(code: code))
                attach(connection, token: token); status = "正在连接 B 端"
            }
            active = true
            monitor = Task { [weak self] in
                var previous: UInt64 = 0, ticks = 0
                var time = ProcessInfo.processInfo.systemUptime
                while !Task.isCancelled {
                    do { try await Task.sleep(for: .milliseconds(250)) } catch { return }
                    guard let self, self.operation == token else { return }
                    if self.connected {
                        if !self.plane.heartbeat(host: self.role == .host), self.controlling {
                            self.releaseControl(); self.status = "画面链路无响应，已释放控制"
                        }
                        if self.role == .host && self.controlling &&
                            (ProcessInfo.processInfo.systemUptime - self.lastClientHeartbeat > 1 ||
                             NSWorkspace.shared.frontmostApplication?.processIdentifier != self.process?.processIdentifier ||
                             !RemoteInputRelay.authorized) {
                            self.controlRequest = UUID(); self.controlling = false; self.plane.input.focus(false)
                            self.plane.send(RemoteMessage(.release))
                        }
                    }
                    ticks += 1
                    guard ticks % 4 == 0 else { continue }
                    if self.role == .host && self.running {
                        if NSWorkspace.shared.runningApplications.contains(where: { $0.bundleIdentifier == "com.unidesk.app" }) {
                            await self.stop(); self.status = "检测到 uniDesk 主程序，已停止测试共享以释放输入"; return
                        }
                        if self.process?.isTerminated != false { await self.stopCapture(); self.sendError("B 端应用已退出"); continue }
                        do { try await self.refreshCapture(token: token) }
                        catch { await self.stopCapture(); self.sendError(error.localizedDescription) }
                    }
                    let now = ProcessInfo.processInfo.systemUptime
                    self.statistics = self.metrics.snapshot()
                    self.effectiveFPS = Double(self.statistics.presented - previous) / max(0.001, now - time)
                    previous = self.statistics.presented; time = now
                }
            }
        } catch { await stop(); status = error.localizedDescription }
    }
    private func attach(_ connection: NWConnection, token: UUID) {
        let host = role == .host, plane = plane, metrics = metrics
        let expectedToken: String?
        do { expectedToken = host ? try RemoteTLSIdentity.token(code: code) : nil }
        catch { connection.cancel(); status = error.localizedDescription; return }
        let remote = RemotePeer(connection: connection, serverToken: expectedToken, receive: { [weak self] message in
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
                if host && message.kind == .heartbeat {
                    Task { @MainActor [weak self] in if self?.operation == token { self?.lastClientHeartbeat = ProcessInfo.processInfo.systemUptime } }
                }
                return
            }
            Task { @MainActor [weak self] in
                guard let self, self.operation == token else { return }
                await self.receive(message, token: token)
            }
        }, state: { [weak self] message, ready in
            Task { @MainActor [weak self] in
                guard let self, self.operation == token else { return }
                self.connected = ready; self.status = message
                if ready {
                    self.lastClientHeartbeat = ProcessInfo.processInfo.systemUptime
                    if !host {
                        var hello = RemoteMessage(.hello); hello.text = try? RemoteTLSIdentity.token(code: self.code)
                        hello.capabilities = .local; self.plane.send(hello)
                    }
                } else { await self.stop(); self.status = message }
            }
        }, keyNeeded: { plane.requestKey() })
        peer = remote; plane.setPeer(remote); remote.start()
    }
    private func receive(_ message: RemoteMessage, token: UUID) async {
        if role == .host {
            switch message.kind {
            case .hello:
                remoteCapabilities = message.capabilities ?? .h264Only
                var value = RemoteMessage(.applications); value.applications = applications; plane.send(value)
            case .launch:
                guard !busy, let application = message.application else { return }
                busy = true; defer { busy = false }
                await stopCapture()
                do {
                    if let preferences = message.preferences { self.preferences = preferences }
                    process = try await catalog.launch(application)
                    guard operation == token, let process else { return }
                    _ = process.activate(options: [.activateAllWindows])
                    try plane.input.start(processID: process.processIdentifier)
                    running = true
                    try await refreshCapture(token: token)
                } catch { await stopCapture(); sendError(error.localizedDescription) }
            case .acquire:
                guard running, message.identity == lastSurface?.identity, let process else { return }
                guard RemoteInputRelay.authorized else { sendError("B 端辅助功能权限不可用"); return }
                let request = UUID(); controlRequest = request
                _ = process.activate(options: [.activateAllWindows])
                // Activation is asynchronous. Never arm against a different foreground app.
                for _ in 0..<10 {
                    if NSWorkspace.shared.frontmostApplication?.processIdentifier == process.processIdentifier { break }
                    try? await Task.sleep(for: .milliseconds(30))
                }
                guard operation == token, running, controlRequest == request,
                      NSWorkspace.shared.frontmostApplication?.processIdentifier == process.processIdentifier else { return }
                plane.input.focus(true); controlling = true
                var ack = RemoteMessage(.controlReady); ack.identity = lastSurface?.identity; plane.send(ack)
            case .release: controlRequest = UUID(); controlling = false; plane.input.focus(false)
            case .preferences:
                if let preferences = message.preferences { self.preferences = preferences }
            case .stop: await stopCapture()
            default: break
            }
        } else {
            switch message.kind {
            case .applications:
                applications = message.applications ?? []; status = "已连接 B 端，请选择远端应用并启动"
            case .surface:
                guard let surface = message.surface, let profile = message.profile else { return }
                releaseControl(); lastSurface = surface; running = true
                sourceDescription = "B 源：\(Int(surface.contentRect.width)) × \(Int(surface.contentRect.height)) pt · \(surface.nativePixels.label) px"
                actualDescription = "传输：\(profile.codec.label) · \(profile.pixels.label) · \(String(format: "%.1f", profile.framesPerSecond)) Hz\(profile.reason.map { " · " + $0 } ?? "")"
                status = "远端画面已配置，点击画面获取控制"
            case .controlReady:
                guard controlPending, message.identity == lastSurface?.identity else { return }
                controlPending = false; controlling = true; plane.setEnabled(true)
                for (event, identity) in pendingClick { plane.event(event, surface: identity) }
                pendingClick.removeAll(); status = "正在控制 B 端 · ⌃⌥Esc 或点击画面外释放"
            case .release:
                releaseControl(); status = "B 端已释放控制，点击画面重新申请"
            case .error:
                releaseControl(); running = false; plane.frames.reset(nil); status = message.text ?? "远端错误"
            default: break
            }
        }
    }
    func launchSelected() {
        guard connected, role == .viewer, let selected else { return }
        releaseControl()
        var message = RemoteMessage(.launch); message.application = selected; message.preferences = preferences; plane.send(message)
        status = "正在启动远端应用"
    }
    func changePreferences() {
        guard role == .viewer, connected else { return }
        releaseControl(); var message = RemoteMessage(.preferences); message.preferences = preferences; plane.send(message)
    }
    func canvasFocus(_ focused: Bool) {
        if !focused { releaseControl(); return }
        guard role == .viewer, running, let surface = lastSurface else { return }
        controlPending = true; pendingClick.removeAll()
        var message = RemoteMessage(.acquire); message.identity = surface.identity; plane.send(message)
    }
    func canvasInput(_ event: RemoteInputEvent, surface: SurfaceIdentity) {
        if controlPending {
            if pendingClick.count < 32 { pendingClick.append((event, surface)) }
            else { releaseControl() }
            return
        }
        // Clicking again after emergency release requests a fresh lease.
        if !controlling && event.kind == .down { canvasFocus(true); pendingClick.append((event, surface)); return }
        plane.event(event, surface: surface)
    }
    func releaseControl() {
        controlPending = false; pendingClick.removeAll(); controlling = false; plane.release()
    }
    private func selectedVideoCodec() throws -> VideoCodec {
        switch preferences.codec {
        case .h264: return .h264
        case .hevc:
            guard remoteCapabilities.supportsDecoder(.hevc) else {
                throw RemoteApplicationError.unsupported("远端不支持 H.265 硬件解码")
            }
            return .hevc
        case .automatic:
            return remoteCapabilities.supportsDecoder(.hevc) ? .hevc : .h264
        }
    }

    private func refreshCapture(token: UUID) async throws {
        guard !changing, let process, operation == token else { return }
        changing = true; defer { changing = false }
        let mode: CaptureViewMode = desktop && allowDesktop ? .desktop : .application
        let identity = SurfaceIdentity(session: token, epoch: 1, generation: generation + 1, configuration: generation + 1)
        let source = try await capture.source(processID: process.processIdentifier, displayID: nil, mode: mode,
            identity: identity, gestureActive: plane.input.gestureActive)
        guard operation == token else { return }
        let surface = source.surface
        let selectedCodec = try selectedVideoCodec()
        var profile = try VideoProfile.requested(native: surface.nativePixels, sourceHz: surface.sourceHz,
                                                 preferences: preferences, codec: selectedCodec)
        if lastSurface?.contentRect == surface.contentRect, lastSurface?.displayID == surface.displayID,
           lastSurface?.pointPixelScale == surface.pointPixelScale, lastSurface?.sourceHz == surface.sourceHz,
           lastProfile == profile, lastMode == mode { return }
        plane.input.suspend(); await capture.stop()
        let old = encoder; encoder = nil; plane.setEncoder(nil)
        if let old { await Task.detached { old.stop() }.value }
        guard operation == token else { return }
        let plane = plane, metrics = metrics
        let makeEncoder: (VideoProfile) async throws -> VideoEncoder = { [weak self] requestedProfile in
            try await Task.detached { [weak self] in
            try VideoEncoder(surface: identity, profile: requestedProfile, metrics: metrics) { [weak self] result in
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
        }
        let next: VideoEncoder
        do {
            next = try await makeEncoder(profile)
        } catch {
            guard preferences.codec == .automatic, profile.codec == .hevc else { throw error }
            profile = VideoProfile(codec: .h264, pixels: profile.pixels, framesPerSecond: profile.framesPerSecond,
                                   bitrate: profile.bitrate, reason: "H.265 硬件编码不可用，已回退 H.264")
            next = try await makeEncoder(profile)
        }
        guard operation == token else { next.stop(); return }
        encoder = next; plane.setEncoder(next); plane.input.setSurface(surface)
        var message = RemoteMessage(.surface); message.surface = surface; message.profile = profile; plane.send(message)
        try await capture.start(source: source, profile: profile, metrics: metrics, sink: { next.submit($0) }, failed: { [weak self] message in
            plane.input.suspend()
            Task { @MainActor [weak self] in
                guard let self, self.operation == token else { return }
                await self.stopCapture(); self.sendError(message)
            }
        })
        guard operation == token else { return }
        generation += 1; lastSurface = surface; lastProfile = profile; lastMode = mode
        sourceDescription = "源：\(surface.nativePixels.label) px"
        actualDescription = "编码：\(profile.codec.label) · \(profile.pixels.label) · \(String(format: "%.1f", profile.framesPerSecond)) Hz\(profile.reason.map { " · " + $0 } ?? "")"
        status = "正在共享 \(process.localizedName ?? "应用") · 在 A 端点击画面操作"
    }
    private func sendError(_ text: String) {
        status = text; var message = RemoteMessage(.error); message.text = text; plane.send(message)
    }
    private func stopCapture() async {
        running = false; controlRequest = UUID(); plane.input.stop(); await capture.stop()
        let old = encoder; encoder = nil; plane.setEncoder(nil)
        if let old { await Task.detached { old.stop() }.value }
        lastSurface = nil; lastProfile = nil; lastMode = nil
    }
    func stop() async {
        operation = UUID(); monitor?.cancel(); monitor = nil
        releaseControl(); peer?.stop(); peer = nil; listener?.stop(); listener = nil
        active = false; connected = false
        await stopCapture(); plane.reset(); applications = []; selected = nil
        status = "已断开；B 端应用继续运行"
    }
}

struct VideoCanvas: NSViewRepresentable {
    let session: SpikeSession
    func makeNSView(context: Context) -> RemoteVideoView { RemoteVideoView() }
    func updateNSView(_ view: RemoteVideoView, context: Context) {
        view.frames = session.plane.frames; view.metrics = session.metrics
        view.inputEnabled = session.role == .viewer && session.running
        view.onFocus = { [weak session] in session?.canvasFocus($0) }
        view.onInput = { [weak session] in session?.canvasInput($0, surface: $1) }
        let plane = session.plane
        view.onPresented = { plane.presented($0, sequence: $1) }
        if view.scaleMode != session.scale { view.scaleMode = session.scale }
    }
}

struct SpikeView: View {
    @State private var session = SpikeSession()
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("远程应用 · 双机交互测试").font(.title2.bold())
                Spacer()
                Picker("角色", selection: $session.role) {
                    ForEach(SpikeRole.allCases, id: \.self) { Text($0.rawValue).tag($0) }
                }.frame(width: 260).disabled(session.active)
            }
            if session.role == .host {
                Text("先退出 uniDesk 主程序。开启共享后，把本机 IP 和连接密钥发到 A 端；A 端选择应用后，B 端保持该应用在前台。")
                    .font(.callout).foregroundStyle(.secondary)
                HStack {
                    Button("屏幕录制权限") { RemoteAppCapture.requestScreenRecordingPermission() }
                    Button("辅助功能权限") { RemoteInputRelay.requestPermission() }
                    Toggle("允许查看完整桌面", isOn: $session.allowDesktop)
                    Toggle("桌面画面", isOn: $session.desktop).disabled(!session.allowDesktop)
                }
                Text("B 端地址：\(session.localAddresses) · UDP \(SpikeSession.port)").font(.caption).textSelection(.enabled)
                HStack {
                    Text("连接密钥")
                    Text(session.code.isEmpty ? "开始共享后生成" : session.code).font(.system(.caption, design: .monospaced)).textSelection(.enabled)
                    Button("复制") { NSPasteboard.general.clearContents(); NSPasteboard.general.setString(session.code, forType: .string) }
                        .disabled(session.code.isEmpty)
                }
            } else {
                HStack {
                    TextField("B 端 IP / 主机名", text: $session.address).frame(width: 200)
                    SecureField("粘贴 B 端连接密钥", text: $session.code)
                }.disabled(session.active)
            }
            HStack {
                Button(session.active ? "断开 / 停止共享" : (session.role == .host ? "开始共享" : "连接 B 端")) {
                    Task { if session.active { await session.stop() } else { await session.start() } }
                }.disabled(session.busy)
                Text(session.status).font(.callout).textSelection(.enabled)
            }
            if session.role == .viewer {
                HStack {
                    TextField("搜索远端应用", text: $session.applicationQuery).frame(width: 140)
                    Picker("应用", selection: $session.selected) {
                        Text("选择远端应用").tag(nil as UUID?)
                        ForEach(session.applications.filter { session.applicationQuery.isEmpty || $0.name.localizedCaseInsensitiveContains(session.applicationQuery) || $0.id == session.selected }) { app in
                            Text(app.name + (app.processID == nil ? "" : " · 运行中")).tag(Optional(app.id))
                        }
                    }
                    Button("启动 / 切换应用") { session.launchSelected() }.disabled(!session.connected || session.selected == nil)
                    Button("释放控制") { session.releaseControl() }.disabled(!session.controlling)
                }
                HStack {
                    Picker("分辨率", selection: $session.preferences.resolution) {
                        ForEach(ResolutionPreference.allCases, id: \.self) { Text($0.label).tag($0) }
                    }
                    Picker("视频编码", selection: $session.preferences.codec) {
                        ForEach(VideoCodec.allCases, id: \.self) { Text($0.label).tag($0) }
                    }
                    Picker("刷新率", selection: $session.preferences.refresh) {
                        ForEach(RefreshPreference.allCases, id: \.self) { Text($0.label).tag($0) }
                    }
                    Picker("显示", selection: $session.scale) {
                        ForEach(VideoViewScale.allCases, id: \.self) { Text($0.label).tag($0) }
                    }
                }
                VideoCanvas(session: session).frame(minHeight: 300).background(.black).clipShape(RoundedRectangle(cornerRadius: 8))
                Text("点击画面控制；⌃⌥Esc 释放。中文输入法在 B 端切换。1:1 显示下按住 ⌥ 滚轮平移。")
                    .font(.caption).foregroundStyle(.secondary)
            } else {
                Spacer()
                Text("等待 A 端操作。停止共享不会退出目标应用。此测试版只接受一个持有连接密钥的控制端。")
                    .foregroundStyle(.secondary)
                Spacer()
            }
            VStack(alignment: .leading, spacing: 4) {
                Text(session.sourceDescription)
                Text(session.actualDescription)
                Text("\(session.role == .viewer ? "有效呈现" : "B 端编码") · \(session.role == .viewer ? String(format: "%.1f FPS", session.effectiveFPS) : String(session.statistics.encoded) + " 帧") · 编码 P95 \(session.statistics.encodeP95Milliseconds, specifier: "%.1f") ms · 解码 P95 \(session.statistics.decodeP95Milliseconds, specifier: "%.1f") ms")
            }.font(.system(.caption, design: .monospaced)).textSelection(.enabled)
        }
        .padding(20).frame(minWidth: 960, minHeight: 680)
        .onChange(of: session.preferences) { _, _ in session.changePreferences() }
        .onDisappear { Task { await session.stop() } }
        .onReceive(NotificationCenter.default.publisher(for: NSWindow.didMiniaturizeNotification)) { _ in
            Task { await session.stop() }
        }
    }
}

struct RemoteApplicationSpike: App {
    @NSApplicationDelegateAdaptor(SpikeDelegate.self) private var delegate
    var body: some Scene { WindowGroup("uniDesk Remote Application Spike") { SpikeView() } }
}

@main enum SpikeEntry {
    static func main() {
        signal(SIGPIPE, SIG_IGN)
        if CommandLine.arguments.contains("--remote-input-helper") { RemoteInputHelper.run() }
        else { RemoteApplicationSpike.main() }
    }
}

@MainActor final class SpikeDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.regular); NSApp.activate(ignoringOtherApps: true)
    }
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }
}
