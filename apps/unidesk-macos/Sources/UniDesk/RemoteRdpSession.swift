import Foundation
import Observation
import UniDeskEngine
import UniDeskRemoteApplication

/// Catalog messages use QUIC; pixels, NLA credentials and input use a separate
/// RDP connection. Closing the catalog also disconnects this viewer's RDP client.
@available(macOS 14.2, *)
@MainActor @Observable final class RemoteRdpSession {
    enum Phase: Equatable { case idle, connecting, ready, failed(String) }
    private(set) var phase: Phase = .idle
    private(set) var apps: [RemoteRdpApp] = []
    private(set) var unavailableReason: String?
    private(set) var launchedName: String?
    private(set) var bridgeState: RemoteRdpBridge.State = .idle
    let engine: Engine
    let peerID: String
    let peerHost: String
    @ObservationIgnored private var channel: RemoteApplicationChannel?
    @ObservationIgnored private var bridge: RemoteRdpBridge?
    @ObservationIgnored private var reader: Task<Void, Never>?
    @ObservationIgnored private var generation = UUID()
    @ObservationIgnored private var credentials: (String?, String?)?
    @ObservationIgnored private let writer = DispatchQueue(label: "uniDesk.rdp.catalog.send")

    init(engine: Engine, peerID: String, peerHost: String) {
        self.engine = engine; self.peerID = peerID; self.peerHost = peerHost
    }

    func connect() async {
        stop()
        let token = generation
        phase = .connecting
        let engine = engine, peerID = peerID
        do {
            let channel = try await Task.detached { try engine.openRemoteApplication(peerId: peerID) }.value
            guard generation == token, !Task.isCancelled else { channel.close(); return }
            self.channel = channel
            // Windows sends the initial catalog without a request. Blocking
            // receives must never run on the main actor's cooperative executor.
            reader = Task.detached { [weak self] in
                while !Task.isCancelled {
                    do {
                        guard let data = try channel.receive(waitMs: 250) else { continue }
                        let message = try JSONDecoder().decode(RemoteRdpMessage.self, from: data)
                        await self?.handle(message, token: token)
                    } catch {
                        await self?.fail(error.localizedDescription, token: token)
                        return
                    }
                }
            }
        } catch { fail(error.localizedDescription, token: token) }
    }

    func refresh() {
        if channel != nil { send(.catalog(version: 1)) }
        else { Task { await connect() } }
    }

    func launch(_ app: RemoteRdpApp, username: String?, password: String?) {
        guard channel != nil else { return }
        launchedName = app.name; credentials = (username, password)
        bridgeState = .connecting
        send(.launch(version: 1, name: app.name))
    }

    private func send(_ request: RemoteRdpRequest) {
        guard let channel else { return }
        let token = generation
        writer.async { [weak self] in
            do { try channel.send(bytes: JSONEncoder().encode(request)) }
            catch { Task { @MainActor [weak self] in self?.fail(error.localizedDescription, token: token) } }
        }
    }
    func frame() -> RemoteRdpBridge.Frame? {
        if let bridge { bridgeState = bridge.state }
        return bridge?.frame()
    }
    func mouse(x: UInt16, y: UInt16, flags: UInt16) { bridge?.mouse(x: x, y: y, flags: flags) }
    func key(flags: UInt16, scancode: UInt8) { bridge?.key(flags: flags, scancode: scancode) }
    func unicode(flags: UInt16, code: UInt16) { bridge?.unicode(flags: flags, code: code) }

    func stop() {
        generation = UUID()
        reader?.cancel(); reader = nil
        channel?.close(); channel = nil
        let previous = bridge; bridge = nil
        // FreeRDP disconnect joins its native thread; never join on AppKit.
        if let previous { Task.detached { previous.stop() } }
        credentials = nil; apps = []; unavailableReason = nil
        phase = .idle; bridgeState = .idle
    }
    private func fail(_ message: String, token: UUID) {
        guard generation == token else { return }
        stop(); phase = .failed(message)
    }
    private func handle(_ message: RemoteRdpMessage, token: UUID) {
        guard token == generation else { return }
        switch message {
        case .catalog(_, let available, let reason, let apps):
            unavailableReason = available ? nil : (reason ?? "远端 RemoteApp 不可用")
            self.apps = available ? apps : []; phase = .ready
        case .rdp(_, let rdp):
            guard let credentials else { return } // Ignore unsolicited/replayed launch replies.
            self.credentials = nil
            let bridge = RemoteRdpBridge()
            self.bridge = bridge
            bridge.start(rdpFile: rdp.replacingOccurrences(of: "{host}", with: peerHost),
                         username: credentials.0, password: credentials.1)
        case .error(_, _, let message): fail(message, token: token)
        }
    }
}
