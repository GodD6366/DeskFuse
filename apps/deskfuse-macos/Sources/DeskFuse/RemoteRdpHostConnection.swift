import AppKit
import Foundation
import Observation
import DeskFuseEngine
import DeskFuseRemoteApplication

/// Owns the single RDP connection for one Windows host. The first application
/// starts the session through its `.rdp` document; later applications are
/// launched with RAIL `ClientExecute` on the same connection, so Windows never
/// sees a second logon for the same user.
///
/// When the tunnel or FreeRDP connection drops unexpectedly, the same session is
/// re-established for up to a minute and the open applications are relaunched on
/// the new connection. Credentials stay in memory only and are discarded on stop.
@available(macOS 14.2, *)
@MainActor @Observable final class RemoteRdpHostConnection {
    enum Phase: Equatable {
        case idle
        case preparing
        case reconnecting(attempt: Int)
        case ready
        case failed(String)
    }

    private(set) var phase: Phase = .idle
    private(set) var bridgeState: RemoteRdpBridge.State = .idle
    private(set) var activeApps: [String] = []
    private(set) var error: String?
    let engine: Engine
    let peerID: String
    let peerName: String
    let catalog: RemoteRdpSession

    @ObservationIgnored private var bridge: RemoteRdpBridge?
    @ObservationIgnored private var tunnel: RemoteRdpTunnelConnection?
    @ObservationIgnored private var clipboard: RemoteRdpClipboard?
    @ObservationIgnored private var credentials: (username: String, password: String)?
    @ObservationIgnored private var reconnectTask: Task<Void, Never>?
    @ObservationIgnored private var metrics = RemoteRdpSessionMetrics()
    @ObservationIgnored private var connectedReported = false
    @ObservationIgnored private var suspendedForSleep = false
    @ObservationIgnored nonisolated(unsafe) private var sleepObserver: NSObjectProtocol?
    @ObservationIgnored nonisolated(unsafe) private var wakeObserver: NSObjectProtocol?
    private let reconnectPolicy = RemoteRdpReconnectPolicy()

    init(engine: Engine, peerID: String, peerName: String) {
        self.engine = engine
        self.peerID = peerID
        self.peerName = peerName
        self.catalog = RemoteRdpSession(engine: engine, peerID: peerID, peerName: peerName)
        let center = NSWorkspace.shared.notificationCenter
        sleepObserver = center.addObserver(forName: NSWorkspace.willSleepNotification, object: nil, queue: .main) { [weak self] _ in
            Task { @MainActor in self?.suspendForSleep() }
        }
        wakeObserver = center.addObserver(forName: NSWorkspace.didWakeNotification, object: nil, queue: .main) { [weak self] _ in
            Task { @MainActor in self?.resumeAfterSleep() }
        }
    }

    deinit {
        let center = NSWorkspace.shared.notificationCenter
        if let sleepObserver { center.removeObserver(sleepObserver) }
        if let wakeObserver { center.removeObserver(wakeObserver) }
    }

    /// Sleep drops the tunnel anyway; tear it down first so the window shows a
    /// reconnect state instead of a stale frame, and reconnect once on wake.
    private func suspendForSleep() {
        guard bridge != nil || metrics.isActive else { return }
        suspendedForSleep = true
        reconnectTask?.cancel()
        reconnectTask = nil
        teardownSession(reason: "sleep")
        phase = .reconnecting(attempt: 0)
    }

    private func resumeAfterSleep() {
        guard suspendedForSleep else { return }
        suspendedForSleep = false
        guard !activeApps.isEmpty, credentials != nil else {
            phase = .idle
            return
        }
        beginReconnect(reason: "wake")
    }

    func open(_ app: RemoteRdpApp, username: String?, password: String?) async {
        switch phase {
        case .preparing, .reconnecting:
            error = "上一个应用仍在启动，请稍后重试。"
            return
        default:
            break
        }
        if activeApps.contains(app.name) { return }
        phase = .preparing
        error = nil
        do {
            let ready = try await catalog.prepare(app.name)
            if let bridge {
                switch bridge.state {
                case .failed, .disconnected:
                    reconnectTask?.cancel()
                    reconnectTask = nil
                    teardownSession(reason: "reopen")
                    activeApps = []
                default:
                    guard bridge.execute(app.name) else {
                        throw RemoteRdpCatalogError.unavailable("无法在现有 Windows 会话中启动该应用，请关闭窗口后重试。")
                    }
                    activeApps.append(app.name)
                    phase = .ready
                    return
                }
            }
            if bridge == nil, !confirmConsoleSession(ready.host.consoleSession) {
                phase = .idle
                return
            }
            try start(ready: ready, username: username, password: password)
            if let username, let password {
                credentials = (username, password)
            }
            activeApps = [app.name]
            phase = .ready
        } catch {
            let message = error.userFacingMessage
            self.error = message
            phase = .failed(message)
        }
    }

    private func start(ready: RemoteRdpReady, username: String?, password: String?) throws {
        guard let pin = ready.host.certificateSha256,
              RemoteRdpHost.isValidFingerprint(pin) else {
            throw RemoteRdpCatalogError.unavailable("Windows 未提供可用的 RDP 证书指纹，请刷新目录后重试。")
        }
        let preferences = RemoteRdpPreferences.load()
        let bridge = RemoteRdpBridge()
        guard bridge.prepare(rdpFile: ready.rdp, username: username, password: password,
                             expectedCertificateFingerprint: pin) else {
            throw RemoteRdpCatalogError.unavailable("无法创建 Windows RDP 会话。")
        }
        bridge.setAudioEnabled(preferences.audioEnabled)
        guard let fd = bridge.takeTunnelFD() else {
            bridge.stop()
            throw RemoteRdpCatalogError.unavailable("无法创建 RDP 隧道。")
        }
        let tunnel: RemoteRdpTunnelConnection
        do {
            tunnel = try engine.openRemoteRdpTunnel(peerId: peerID, ticket: ready.ticket, fd: fd)
        } catch {
            bridge.stop()
            throw error
        }
        self.bridge = bridge
        self.tunnel = tunnel
        bridgeState = .connecting
        metrics.begin(at: Date())
        connectedReported = false
        engine.remoteApplicationDiagnostic(event: "rdp_session_started",
            detail: "peer=\(peerID) audio=\(preferences.audioEnabled) clipboard=\(preferences.clipboardEnabled)")
        bridge.start()
        if preferences.clipboardEnabled {
            let clipboard = RemoteRdpClipboard()
            clipboard.attach(to: bridge)
            self.clipboard = clipboard
        } else {
            clipboard = nil
        }
    }

    /// Windows 专业版同一账户只允许一个会话：本机有人登录时，远程接管会锁屏
    /// 或挤掉当前用户。只有在准备建立新会话时才提示，RAIL 追加应用不重复询问。
    private func confirmConsoleSession(_ session: RemoteRdpConsoleSession?) -> Bool {
        guard let session, !session.user.isEmpty else { return true }
        let alert = NSAlert()
        alert.messageText = "Windows 本机已有登录会话"
        alert.informativeText = session.connected
            ? "连接后将占用 Windows 的唯一远程会话：本机会锁屏，当前登录的“\(session.user)”会被断开。"
            : "Windows 仍保留“\(session.user)”的登录会话；继续连接会结束该会话。"
        alert.alertStyle = .warning
        alert.addButton(withTitle: "继续连接")
        alert.addButton(withTitle: "取消")
        return alert.runModal() == .alertFirstButtonReturn
    }

    func frame() -> RemoteRdpBridge.Frame? {
        guard let bridge else { return nil }
        bridgeState = bridge.state

        if bridgeState == .connected {
            let now = Date()
            if !connectedReported {
                connectedReported = true
                metrics.markConnected(at: now)
                engine.remoteApplicationDiagnostic(event: "rdp_connected",
                    detail: "peer=\(peerID) connect_ms=\(metrics.connectMilliseconds() ?? 0) apps=\(activeApps.count)")
            }
            guard let frame = bridge.frame() else { return nil }
            if metrics.markFrame(at: now) {
                engine.remoteApplicationDiagnostic(event: "rdp_first_frame",
                    detail: "peer=\(peerID) first_frame_ms=\(metrics.firstFrameMilliseconds() ?? 0)")
            }
            return frame
        }

        if case .failed(let bridgeMessage) = bridgeState {
            // 隧道先断时 FreeRDP 只能看到 EOF 并报“安全协商失败”，真实原因记录在隧道上。
            let message = tunnel?.closedReason().map { "DeskFuse RDP 隧道中断：\($0)" } ?? bridgeMessage
            bridgeState = .failed(message)
            error = message
            clipboard?.detach()
            clipboard = nil
            switch phase {
            case .failed, .idle, .reconnecting:
                break
            case .preparing, .ready:
                if bridge.consumeAuthenticationFailure() {
                    engine.remoteApplicationDiagnostic(event: "rdp_auth_failed",
                        detail: "peer=\(peerID) apps=\(activeApps.joined(separator: ","))")
                    if promptForCredentials() {
                        beginReconnect(reason: "auth")
                    } else {
                        activeApps = []
                        phase = .failed(message)
                    }
                } else {
                    engine.remoteApplicationDiagnostic(event: "rdp_bridge_failed",
                        detail: "peer=\(peerID) apps=\(activeApps.joined(separator: ",")) reason=\(message.prefix(512))")
                    beginReconnect(reason: message)
                }
            }
        } else if bridgeState == .disconnected {
            // Server-side logoff or a clean transport close: retry on the same
            // terms as a failure so a transient drop does not strand the window.
            let message = "Windows RDP 会话已断开"
            error = message
            clipboard?.detach()
            clipboard = nil
            switch phase {
            case .failed, .idle, .reconnecting:
                break
            case .preparing, .ready:
                engine.remoteApplicationDiagnostic(event: "rdp_bridge_disconnected",
                    detail: "peer=\(peerID) apps=\(activeApps.joined(separator: ","))")
                beginReconnect(reason: message)
            }
        }
        return bridge.frame()
    }

    /// Asks for a new password without closing the window. Returns false when the
    /// user cancels; callers then stop retrying instead of looping on a modal.
    private func promptForCredentials() -> Bool {
        guard let current = credentials else { return false }
        let alert = NSAlert()
        alert.messageText = "Windows 登录失败"
        alert.informativeText = "请重新输入“\(current.username)”的密码。Microsoft 账户请使用账户密码，不是 PIN。"
        alert.alertStyle = .warning
        alert.addButton(withTitle: "重新连接")
        alert.addButton(withTitle: "取消")
        let field = NSSecureTextField(frame: NSRect(x: 0, y: 0, width: 260, height: 24))
        field.placeholderString = "密码"
        alert.accessoryView = field
        alert.window.initialFirstResponder = field
        guard alert.runModal() == .alertFirstButtonReturn, !field.stringValue.isEmpty else {
            return false
        }
        credentials = (current.username, field.stringValue)
        return true
    }

    /// Keeps the same window and retries for up to `reconnectPolicy.deadline`.
    /// The task is cancelled by `stop()` and by a manual reopen.
    private func beginReconnect(reason: String) {
        if reconnectTask != nil { return }
        guard !activeApps.isEmpty, credentials != nil else {
            activeApps = []
            phase = .failed(reason)
            return
        }
        phase = .reconnecting(attempt: 0)
        let apps = activeApps
        reconnectTask = Task { [weak self] in
            await self?.runReconnect(apps: apps, reason: reason)
        }
    }

    private func runReconnect(apps: [String], reason: String) async {
        defer { reconnectTask = nil }
        guard let credentials, let first = apps.first else { return }
        let started = Date()
        var attempt = 0
        while !Task.isCancelled,
              reconnectPolicy.mayRetry(elapsed: Date().timeIntervalSince(started)) {
            attempt += 1
            phase = .reconnecting(attempt: attempt)
            teardownSession(reason: "reconnect_attempt_\(attempt)")
            do {
                await catalog.connect()
                let ready = try await catalog.prepare(first)
                if Task.isCancelled { return }
                try start(ready: ready, username: credentials.username, password: credentials.password)
                activeApps = [first]
                for app in apps.dropFirst() where bridge?.execute(app) == true {
                    activeApps.append(app)
                }
                bridgeState = .connecting
                phase = .ready
                error = nil
                engine.remoteApplicationDiagnostic(event: "rdp_reconnected",
                    detail: "peer=\(peerID) attempt=\(attempt) apps=\(activeApps.joined(separator: ","))")
                return
            } catch {
                if Task.isCancelled { return }
                let delay = reconnectPolicy.delay(afterAttempt: attempt)
                try? await Task.sleep(for: .seconds(delay))
            }
        }
        guard !Task.isCancelled else { return }
        activeApps = []
        let message = "Windows 连接已中断，自动重连失败：\(reason)"
        phase = .failed(message)
        error = message
        engine.remoteApplicationDiagnostic(event: "rdp_reconnect_failed",
            detail: "peer=\(peerID) apps=\(apps.joined(separator: ",")) reason=\(reason.prefix(512))")
    }

    private func teardownSession(reason: String) {
        if metrics.isActive {
            let now = Date()
            engine.remoteApplicationDiagnostic(event: "rdp_session_closed",
                detail: "peer=\(peerID) reason=\(reason.prefix(200)) uptime_ms=\(metrics.uptimeMilliseconds(now: now) ?? 0) frames=\(metrics.frames) fps=\(String(format: "%.1f", metrics.frameRate(now: now)))")
            metrics = RemoteRdpSessionMetrics()
        }
        connectedReported = false
        clipboard?.detach()
        clipboard = nil
        tunnel?.close()
        tunnel = nil
        let previous = bridge
        bridge = nil
        bridgeState = .idle
        if let previous { Task.detached { previous.stop() } }
    }

    var connected: Bool { bridge?.connected == true }

    /// Read on demand by the window's diagnostics popover; values change every
    /// frame, so this is a computed snapshot instead of observable state.
    var diagnosticsText: String {
        let now = Date()
        let preferences = RemoteRdpPreferences.load()
        let phaseText: String = switch phase {
        case .idle: "空闲"
        case .preparing: "准备中"
        case .reconnecting(let attempt): attempt > 0 ? "重连第 \(attempt) 次" : "等待重连"
        case .ready: "已就绪"
        case .failed(let message): "失败：\(message)"
        }
        let bridgeText: String = switch bridgeState {
        case .idle: "空闲"
        case .connecting: "连接中"
        case .connected: "已连接"
        case .disconnected: "已断开"
        case .failed(let message): "失败：\(message)"
        }
        func milliseconds(_ value: Int?) -> String { value.map { "\($0) ms" } ?? "—" }
        return """
        \(peerName) · \(peerID.prefix(12))
        状态：\(phaseText)
        RDP：\(bridgeText)
        连接耗时：\(milliseconds(metrics.connectMilliseconds()))
        首帧：\(milliseconds(metrics.firstFrameMilliseconds()))
        在线：\(milliseconds(metrics.uptimeMilliseconds(now: now)))
        帧数：\(metrics.frames) · \(String(format: "%.1f", metrics.frameRate(now: now))) fps
        应用：\(activeApps.count) · 声音：\(preferences.audioEnabled ? "开" : "关") · 剪贴板：\(preferences.clipboardEnabled ? "开" : "关")
        """
    }

    func mouse(x: UInt16, y: UInt16, flags: UInt16) { bridge?.mouse(x: x, y: y, flags: flags) }
    func key(flags: UInt16, scancode: UInt8) { bridge?.key(flags: flags, scancode: scancode) }
    func unicode(flags: UInt16, code: UInt16) { bridge?.unicode(flags: flags, code: code) }

    func stop() {
        suspendedForSleep = false
        reconnectTask?.cancel()
        reconnectTask = nil
        teardownSession(reason: "stopped")
        catalog.stop()
        credentials = nil
        activeApps = []
        bridgeState = .idle
        phase = .idle
        error = nil
    }
}
