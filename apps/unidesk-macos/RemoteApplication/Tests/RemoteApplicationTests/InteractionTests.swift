import Foundation
import Testing
@preconcurrency import Network
@testable import UniDeskRemoteApplication

@Test func inputLeaseRejectsReplayOldSurfaceAndExpiredHeartbeat() {
    let now = ProcessInfo.processInfo.systemUptime
    let identity = SurfaceIdentity(session: UUID(), epoch: 1, generation: 1, configuration: 1)
    var lease = InputHelperLease(); lease.configure(identity, now: now)
    var packet = InputHelperPacket(.event); packet.surface = identity; packet.sequence = 1; packet.sentAt = now
    #expect({ lease.accept(packet, now: now + 0.1) }())
    #expect({ !lease.accept(packet, now: now + 0.2) }())
    packet.sequence = 2
    packet.surface = SurfaceIdentity(session: identity.session, epoch: 1, generation: 2, configuration: 2)
    #expect({ !lease.accept(packet, now: now + 0.3) }())
    packet.kind = .heartbeat; packet.surface = identity; packet.sentAt = now + 2
    #expect({ !lease.accept(packet, now: now + 2) }())
    lease.revoke()
    #expect(!lease.alive(now: now))
}

@Test func compressedWireAndInputRoundTripPreserveBoundaries() throws {
    let surface = SurfaceIdentity(session: UUID(), epoch: 2, generation: 5, configuration: 7)
    var message = RemoteMessage(.frame)
    message.frame = EncodedVideoFrame(surface: surface, sequence: 10, presentationNanoseconds: 123,
        keyFrame: true, sps: Data([0x67]), pps: Data([0x68]), bytes: Data([0, 0, 0, 1, 0x65]))
    let wire = try RemoteWire.encode(message)
    #expect(wire.prefix(4).reduce(0) { ($0 << 8) | Int($1) } == wire.count - 4)
    let decoded = try RemoteWire.decode(Data(wire.dropFirst(4)))
    #expect(decoded.frame?.surface == surface)
    #expect(decoded.frame?.bytes == message.frame?.bytes)
    var key = RemoteInputEvent(kind: .keyDown); key.key = 9; key.flags = 1 << 20
    message = RemoteMessage(.input); message.input = key; message.sequence = 10; message.identity = surface
    let input = try RemoteWire.decode(Data(RemoteWire.encode(message).dropFirst(4)))
    #expect(input.input?.key == 9)
    #expect(input.input?.flags == 1 << 20)
    #expect(input.sequence == 10)
    #expect(throws: (any Error).self) { try RemoteWire.decode(Data([0, 1, 2])) }
    #expect(throws: (any Error).self) { try RemoteWire.parameters(code: "123456") }
}

/// Exercises only the authenticated protocol. Never captures screen or injects
/// input, and makes no claim about two-device reachability or user interaction.
private final class ProtocolFixture: @unchecked Sendable {
    let lock = NSLock()
    var server: RemotePeer?
    var client: RemotePeer?
    var outcome: String?
    func finish(_ value: String) { lock.withLock { if outcome == nil { outcome = value } } }
    func result() -> String? { lock.withLock { outcome } }
    func stop() { server?.stop(); client?.stop() }
}

@Test(arguments: ["valid", "wrong-certificate", "wrong-token"])
func quicPinnedCertificateProtocolRoundTrip(mode: String) async throws {
    let fixture = ProtocolFixture(), identity = try RemoteTLSIdentity()
    let code = identity.code
    var changed = Data(base64Encoded: code)!
    if mode == "wrong-certificate" { changed[0] ^= 1 }
    if mode == "wrong-token" { changed[63] ^= 1 }
    let clientCode = changed.base64EncodedString()
    let queue = DispatchQueue(label: "uniDesk.test.quic")
    let listener = try NWListener(using: RemoteWire.parameters(code: code, identity: identity), on: .any)
    listener.newConnectionHandler = { connection in
        let server = RemotePeer(connection: connection, serverToken: try! RemoteTLSIdentity.token(code: code), receive: { message in
            if message.kind == .keyFrame { fixture.server?.send(message) }
        }, state: { message, ready in
            if ready { fixture.server?.send(RemoteMessage(.applications)) }
            else { fixture.finish("server: " + message) }
        })
        fixture.server = server; server.start()
    }
    listener.stateUpdateHandler = { state in
        if case .ready = state, let port = listener.port {
            do {
                let client = try RemotePeer(host: "127.0.0.1", port: port.rawValue, code: clientCode, receive: { message in
                    if message.kind == .applications {
                        var ping = RemoteMessage(.keyFrame); ping.text = "protocol-test"; fixture.client?.send(ping)
                    }
                    if message.kind == .keyFrame && message.text == "protocol-test" { fixture.finish("ok") }
                }, state: { message, ready in
                    if ready {
                        var hello = RemoteMessage(.hello); hello.text = try? RemoteTLSIdentity.token(code: clientCode); fixture.client?.send(hello)
                    } else { fixture.finish("client: " + message) }
                })
                fixture.client = client; client.start()
            } catch { fixture.finish(error.localizedDescription) }
        } else if case .failed(let error) = state { fixture.finish(error.localizedDescription) }
    }
    listener.start(queue: queue)
    defer { listener.cancel(); fixture.stop() }
    for _ in 0..<100 {
        if fixture.result() != nil { break }
        try await Task.sleep(for: .milliseconds(100))
    }
    if mode == "valid" {
        #expect(fixture.result() == "ok", "QUIC pinned certificate handshake: \(fixture.result() ?? "timeout")")
    } else {
        #expect(fixture.result() != nil && fixture.result() != "ok", "Invalid credentials must fail before application access")
    }
}
