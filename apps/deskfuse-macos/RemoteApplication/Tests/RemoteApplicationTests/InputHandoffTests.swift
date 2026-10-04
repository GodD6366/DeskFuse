import Foundation
import Testing
@testable import DeskFuseRemoteApplication

private final class HandoffPackets: @unchecked Sendable {
    private let lock = NSLock()
    private var packets: [InputHelperPacket] = []
    func send(_ packet: InputHelperPacket) -> Bool { lock.withLock { packets.append(packet) }; return true }
    func snapshot() -> [InputHelperPacket] { lock.withLock { packets } }
}

@Test func inputHandoffKeepsBothSurfacesButOnlyNewApplicationAcceptsInput() throws {
    let packets = HandoffPackets()
    let first = RemoteInputRelay(sender: { packets.send($0) })
    let second = RemoteInputRelay(sender: { packets.send($0) })
    let one = SurfaceIdentity(session: UUID(), epoch: 1, generation: 1, configuration: 1)
    let two = SurfaceIdentity(session: UUID(), epoch: 1, generation: 1, configuration: 1)
    let display = CGRect(x: 3840, y: 0, width: 1920, height: 1080)
    let rect = CGRect(x: 5760, y: 0, width: 1920, height: 1080)
    try first.start(processID: 101); try second.start(processID: 102)
    first.setSurface(try CaptureSurface(identity: one, displayID: 2, displayFrame: display,
        contentRect: display, pointPixelScale: 1, sourceHz: 60))
    second.setSurface(try CaptureSurface(identity: two, displayID: 3, displayFrame: rect,
        contentRect: rect, pointPixelScale: 1, sourceHz: 60))
    for (relay, identity) in [(first, one), (second, two)] {
        relay.encoded(EncodedVideoFrame(surface: identity, sequence: 1, presentationNanoseconds: 0,
            keyFrame: true, sps: Data(), pps: Data(), bytes: Data()))
        var presented = RemoteMessage(.presented); presented.identity = identity; presented.sequence = 1
        relay.receive(presented)
    }
    first.focus(true)
    first.focus(false)
    second.focus(true)
    // The old application's media/presentation path remains live after blur.
    var oldPresentation = RemoteMessage(.presented); oldPresentation.identity = one; oldPresentation.sequence = 1
    first.receive(oldPresentation)
    var oldHeartbeat = RemoteMessage(.heartbeat); oldHeartbeat.identity = one
    first.receive(oldHeartbeat)
    var input = RemoteMessage(.input); input.identity = one; input.sequence = 1
    input.input = RemoteInputEvent(kind: .down, point: CGPoint(x: 0.5, y: 0.5))
    first.receive(input)
    input.identity = two
    second.receive(input)
    let sent = packets.snapshot()
    #expect(sent.map(\.kind) == [.configure, .release, .configure, .event])
    #expect(sent[0].processID == 101)
    #expect(sent[1].surface == one)
    #expect(sent[2].processID == 102)
    #expect(sent[2].rect == rect)
    #expect(sent[3].surface == two)
}

@Test func inputHandoffPendingOrFailedRequestDoesNotAutomaticallyRetry() {
    #expect(!RemoteControlRecovery.shouldAcquireAfterSurface(paused: false,
        initialRequest: false, controlling: false, resuming: false))
    #expect(RemoteControlRecovery.shouldAcquireAfterSurface(paused: false,
        initialRequest: true, controlling: false, resuming: false))
    #expect(RemoteControlRecovery.shouldAcquireAfterSurface(paused: false,
        initialRequest: false, controlling: true, resuming: false))
    #expect(!RemoteControlRecovery.shouldAcquireAfterSurface(paused: true,
        initialRequest: true, controlling: true, resuming: true))
}
