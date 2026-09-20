import Foundation
import Observation
import UniDeskEngine
import UniDeskRemoteApplication

/// Viewer-side orchestration for a Windows RemoteApp host: fetch the catalog
/// over the authenticated remote-application QUIC stream (length-framed JSON),
/// then hand the resolved `.rdp` document to the FreeRDP bridge. This is a
/// separate path from the Mac↔Mac media protocol (`RemotePeer`), which carries
/// PropertyList frames.
@available(macOS 14.2, *)
@MainActor @Observable final class RemoteRdpSession {
    enum Phase: Equatable {
        case idle
        case connecting
        case ready
        case failed(String)
    }

    private(set) var phase: Phase = .idle
    private(set) var apps: [RemoteRdpApp] = []
    private(set) var unavailableReason: String?
    private(set) var launchedName: String?

    let engine: Engine
    let peerID: String
    let peerHost: String
    @ObservationIgnored private var channel: RemoteApplicationChannel?
    @ObservationIgnored private var bridge: RemoteRdpBridge?
    @ObservationIgnored private var reader: Task<Void, Never>?

    var bridgeState: RemoteRdpBridge.State { bridge?.state ?? .idle }

    init(engine: Engine, peerID: String, peerHost: String) {
        self.engine = engine
        self.peerID = peerID
        self.peerHost = peerHost
    }

    deinit { stop() }

    /// Opens the stream and requests the catalog. Called from the picker.
    func connect() async {
        phase = .connecting
        let engine = self.engine
        let peerID = self.peerID
        do {
            // openRemoteApplication blocks up to 5s on the QUIC handshake; keep
            // it off the main actor, matching RemoteApplicationSession.
            let channel = try await Task.detached { try engine.openRemoteApplication(peerId: peerID) }.value
            self.channel = channel
            try channel.send(bytes: try Self.encode(.catalog(version: 1)))
            reader = Task { [weak self] in await self?.readLoop(channel) }
        } catch {
            phase = .failed(error.localizedDescription)
        }
    }

    /// Requests the `.rdp` for `app` and starts the FreeRDP bridge. `peerHost`
    /// replaces the `{host}` token; the RDP connection targets `peerHost:3389`.
    func launch(_ app: RemoteRdpApp, username: String?, password: String?) {
        guard let channel else { return }
        launchedName = app.name
        do {
            try channel.send(bytes: try Self.encode(.launch(version: 1, name: app.name)))
        } catch {
            phase = .failed(error.localizedDescription)
        }
        _ = username
        _ = password
    }

    /// Frame snapshot for the canvas. Returns `(width, height)` on success.
    func copyFrame(destination: UnsafeMutableRawPointer, stride: UInt32) -> (UInt32, UInt32)? {
        bridge?.copyFrame(destination: destination, stride: stride)
    }

    func mouse(x: UInt16, y: UInt16, flags: UInt16) { bridge?.mouse(x: x, y: y, flags: flags) }
    func key(flags: UInt16, scancode: UInt8) { bridge?.key(flags: flags, scancode: scancode) }
    func unicode(flags: UInt16, code: UInt16) { bridge?.unicode(flags: flags, code: code) }

    func stop() {
        reader?.cancel()
        reader = nil
        channel?.close()
        channel = nil
        bridge?.stop()
        bridge = nil
        phase = .idle
    }

    private func readLoop(_ channel: RemoteApplicationChannel) async {
        while !Task.isCancelled {
            do {
                guard let data = try channel.receive(waitMs: 250) else { continue }
                handle(try Self.decode(data))
            } catch {
                phase = .failed(error.localizedDescription)
                return
            }
        }
    }

    private func handle(_ message: RemoteRdpMessage) {
        switch message {
        case .catalog(_, let available, let reason, let apps):
            unavailableReason = available ? nil : reason
            self.apps = available ? apps : []
            phase = .ready
        case .rdp(_, let rdp):
            startBridge(rdp: rdp)
        case .error(_, _, let message):
            phase = .failed(message)
        }
    }

    private func startBridge(rdp: String) {
        let resolved = rdp.replacingOccurrences(of: "{host}", with: peerHost)
        let bridge = RemoteRdpBridge()
        self.bridge = bridge
        bridge.start(rdpFile: resolved, username: nil, password: nil)
    }

    private static func encode(_ request: RemoteRdpRequest) throws -> Data {
        try JSONEncoder().encode(request)
    }

    private static func decode(_ data: Data) throws -> RemoteRdpMessage {
        try JSONDecoder().decode(RemoteRdpMessage.self, from: data)
    }
}
