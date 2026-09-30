import Foundation
import DeskFuseEngine
import DeskFuseRemoteApplication

/// Uses the engine's already-authenticated connection. No Network.framework TLS,
/// auxiliary listener, password, certificate generation or manual IP selection.
final class RemotePeer: @unchecked Sendable {
    private let channel: RemoteApplicationChannel
    private let lock = NSLock()
    private let writer = DispatchQueue(label: "DeskFuse.remote.media.send", qos: .userInteractive)
    private let reader = DispatchQueue(label: "DeskFuse.remote.media.receive", qos: .userInteractive)
    private var pending: [RemoteMessage] = []
    private var queuedBytes = 0
    private var scheduled = false
    private var stopped = false
    private var waitingKey = false
    private var maximumSendWaitMs = 0.0
    private var lastNetworkSample: (sent: UInt64, lost: UInt64, time: Double)?
    private let receive: @Sendable (RemoteMessage) -> Void
    private let state: @Sendable (String, Bool) -> Void
    init(channel: RemoteApplicationChannel, receive: @escaping @Sendable (RemoteMessage) -> Void,
         state: @escaping @Sendable (String, Bool) -> Void) {
        self.channel = channel; self.receive = receive; self.state = state
    }
    func start() {
        state("已通过 DeskFuse 配对连接", true)
        reader.async { [weak self] in
            guard let self else { return }
            while !self.lock.withLock({ self.stopped }) {
                do {
                    if let data = try self.channel.receive(waitMs: 250) { self.receive(try RemoteWire.decode(data)) }
                } catch { self.fail(error.localizedDescription); break }
            }
        }
    }
    func diagnostics(media: MediaStatistics) -> RemoteMediaDiagnostics? {
        guard !lock.withLock({ stopped }) else { return nil }
        let stats = channel.networkStats()
        var value = RemoteMediaDiagnostics(media: media, sentPackets: stats.sentPackets,
                                           lostPackets: stats.lostPackets, rttMilliseconds: stats.rttMilliseconds)
        lock.withLock {
            let now = ProcessInfo.processInfo.systemUptime
            if let previous = lastNetworkSample, now - previous.time <= 3,
               stats.sentPackets > previous.sent, stats.lostPackets >= previous.lost {
                value.recentLossPercent = min(100, Double(stats.lostPackets - previous.lost) / Double(stats.sentPackets - previous.sent) * 100)
            }
            lastNetworkSample = (stats.sentPackets, stats.lostPackets, now)
            value.sendWaitMilliseconds = maximumSendWaitMs
            maximumSendWaitMs = 0
        }
        value.remoteAddress = stats.remoteAddress
        return value
    }
    func send(_ message: RemoteMessage) {
        var message = message
        var overflow = false
        let wake = lock.withLock { () -> Bool in
            guard !stopped else { return false }
            if message.kind == .surface {
                pending.removeAll { $0.kind == .frame }; queuedBytes = 0; waitingKey = true
            }
            if let frame = message.frame {
                if waitingKey && !frame.keyFrame { return false }
                // Normal backpressure is applied before encoding. Never destroy
                // reference frames merely because a network write is delayed.
                if pending.filter({ $0.kind == .frame }).count >= 24 || queuedBytes + frame.bytes.count > 64 * 1024 * 1024 {
                    overflow = true; return false
                }
                if frame.keyFrame { waitingKey = false }
                queuedBytes += frame.bytes.count
            } else if message.kind == .input, message.input?.kind == .move,
                      pending.last?.kind == .input, pending.last?.input?.kind == .move,
                      pending.last?.identity == message.identity {
                pending[pending.count - 1] = message; return false
            }
            guard pending.count < 128 else { overflow = true; return false }
            if message.kind == .heartbeat || message.kind == .presented {
                if message.kind == .heartbeat && message.diagnostics == nil {
                    message.diagnostics = pending.last(where: { $0.kind == .heartbeat && $0.identity == message.identity })?.diagnostics
                }
                pending.removeAll { $0.kind == message.kind && $0.identity == message.identity }
            }
            pending.append(message)
            guard !scheduled else { return false }; scheduled = true; return true
        }
        if overflow { fail("远程应用队列已满，已释放控制"); return }
        if wake { writer.async { [weak self] in self?.drain() } }
    }
    private func drain() {
        while let message = lock.withLock({ () -> RemoteMessage? in
            guard !stopped, !pending.isEmpty else { scheduled = false; return nil }
            let message = pending.removeFirst(); queuedBytes -= message.frame?.bytes.count ?? 0; return message
        }) {
            do {
                let started = ProcessInfo.processInfo.systemUptime
                try channel.send(bytes: Data(RemoteWire.encode(message).dropFirst(4)))
                if message.kind == .frame {
                    let elapsed = (ProcessInfo.processInfo.systemUptime - started) * 1000
                    lock.withLock { maximumSendWaitMs = max(maximumSendWaitMs, elapsed) }
                }
            }
            catch { fail(error.localizedDescription); return }
        }
    }
    private func fail(_ message: String) {
        let first = lock.withLock { () -> Bool in
            guard !stopped else { return false }; stopped = true; pending.removeAll(); return true
        }
        if first { channel.close(); state(message, false) }
    }
    func stop() { lock.withLock { stopped = true; pending.removeAll() }; channel.close() }
}

/// Structured control bridge to the same signed deskfuse-input-service used by
/// cross-screen sharing. Only compressed media and small input messages cross FFI.
final class RemoteNativeInputBridge: @unchecked Sendable {
    let engine: Engine
    let peerID: String
    init(engine: Engine, peerID: String) { self.engine = engine; self.peerID = peerID }
    func send(_ packet: InputHelperPacket) -> Bool {
        guard let surface = packet.surface else { return true }
        let event = packet.event
        var value: [String: Any] = ["kind": event?.kind.rawValue ?? packet.kind.rawValue,
            "session": surface.session.uuidString, "generation": surface.generation, "sequence": packet.sequence,
            "pid": packet.processID]
        if let rect = packet.rect {
            value["x"] = rect.minX; value["y"] = rect.minY
            value["width"] = Int(ceil(rect.width)); value["height"] = Int(ceil(rect.height))
        }
        if let event {
            value["x"] = event.point?.x ?? 0; value["y"] = event.point?.y ?? 0
            value["button"] = event.button; value["key"] = event.key; value["flags"] = event.flags
            value["repeat"] = event.repeatKey; value["dx"] = event.deltaX; value["dy"] = event.deltaY
        }
        do {
            let data = try JSONSerialization.data(withJSONObject: value)
            try engine.remoteApplicationInput(peerId: peerID, packetJson: String(decoding: data, as: UTF8.self))
            if let event, event.kind == .down || event.kind == .up {
                engine.remoteApplicationDiagnostic(event: "pointer_\(event.kind.rawValue)_helper_queued",
                    detail: "peer=\(peerID) generation=\(surface.generation) sequence=\(packet.sequence)")
            }
            return true
        } catch {
            engine.remoteApplicationDiagnostic(event: "input_ipc_failed", detail: "peer=\(peerID) generation=\(surface.generation) sequence=\(packet.sequence)")
            return false
        }
    }
}
