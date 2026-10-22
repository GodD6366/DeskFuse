import Foundation
import Observation
import DeskFuseEngine
import DeskFuseRemoteApplication

/// Owns the single RDP connection for one Windows host. The first application
/// starts the session through its `.rdp` document; later applications are
/// launched with RAIL `ClientExecute` on the same connection, so Windows never
/// sees a second logon for the same user.
@available(macOS 14.2, *)
@MainActor @Observable final class RemoteRdpHostConnection {
    enum Phase: Equatable {
        case idle
        case preparing
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

    init(engine: Engine, peerID: String, peerName: String) {
        self.engine = engine
        self.peerID = peerID
        self.peerName = peerName
        self.catalog = RemoteRdpSession(engine: engine, peerID: peerID, peerName: peerName)
    }

    func open(_ app: RemoteRdpApp, username: String?, password: String?) async {
        guard phase != .preparing else {
            error = "上一个应用仍在启动，请稍后重试。"
            return
        }
        if activeApps.contains(app.name) { return }
        phase = .preparing
        error = nil
        do {
            let ready = try await catalog.prepare(app.name)
            if let bridge {
                switch bridge.state {
                case .failed, .disconnected:
                    // The old session is gone; reconnect once and then launch.
                    self.bridge = nil
                    tunnel?.close()
                    tunnel = nil
                    Task.detached { bridge.stop() }
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
            try start(ready: ready, username: username, password: password)
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
        let bridge = RemoteRdpBridge()
        guard bridge.prepare(rdpFile: ready.rdp, username: username, password: password,
                             expectedCertificateFingerprint: pin) else {
            throw RemoteRdpCatalogError.unavailable("无法创建 Windows RDP 会话。")
        }
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
        bridge.start()
    }

    func frame() -> RemoteRdpBridge.Frame? {
        guard let bridge else { return nil }
        bridgeState = bridge.state
        if case .failed(let bridgeMessage) = bridgeState {
            // 隧道先断时 FreeRDP 只能看到 EOF 并报“安全协商失败”，真实原因记录在隧道上。
            let message = tunnel?.closedReason().map { "DeskFuse RDP 隧道中断：\($0)" } ?? bridgeMessage
            bridgeState = .failed(message)
            error = message
            if phase != .failed(message) {
                phase = .failed(message)
                // 每次失败只记录一次，界面提示之外保留可追溯的原因。
                engine.remoteApplicationDiagnostic(event: "rdp_bridge_failed",
                    detail: "peer=\(peerID) apps=\(activeApps.joined(separator: ",")) reason=\(message.prefix(512))")
            }
        }
        return bridge.frame()
    }

    var connected: Bool { bridge?.connected == true }

    func mouse(x: UInt16, y: UInt16, flags: UInt16) { bridge?.mouse(x: x, y: y, flags: flags) }
    func key(flags: UInt16, scancode: UInt8) { bridge?.key(flags: flags, scancode: scancode) }
    func unicode(flags: UInt16, code: UInt16) { bridge?.unicode(flags: flags, code: code) }

    func stop() {
        tunnel?.close()
        tunnel = nil
        let previous = bridge
        bridge = nil
        if let previous { Task.detached { previous.stop() } }
        catalog.stop()
        activeApps = []
        bridgeState = .idle
        phase = .idle
        error = nil
    }
}
