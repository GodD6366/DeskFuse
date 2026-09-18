import Foundation
import UniDeskEngine
import UniDeskRemoteApplication

/// Uses the engine's already-authenticated connection. No Network.framework TLS,
/// auxiliary listener, password, certificate generation or manual IP selection.
final class RemotePeer: @unchecked Sendable {
    private let channel: RemoteApplicationChannel
    private let lock = NSLock()
    private let writer = DispatchQueue(label: "uniDesk.remote.media.send", qos: .userInteractive)
    private let reader = DispatchQueue(label: "uniDesk.remote.media.receive", qos: .userInteractive)
    private var pending: [RemoteMessage] = []
    private var queuedBytes = 0
    private var scheduled = false
    private var stopped = false
    private var waitingKey = false
    private let receive: @Sendable (RemoteMessage) -> Void
    private let state: @Sendable (String, Bool) -> Void
    private let keyNeeded: @Sendable () -> Void
    init(channel: RemoteApplicationChannel, receive: @escaping @Sendable (RemoteMessage) -> Void,
         state: @escaping @Sendable (String, Bool) -> Void, keyNeeded: @escaping @Sendable () -> Void) {
        self.channel = channel; self.receive = receive; self.state = state; self.keyNeeded = keyNeeded
    }
    func start() {
        state("已通过 uniDesk 配对连接", true)
        reader.async { [weak self] in
            guard let self else { return }
            while !self.lock.withLock({ self.stopped }) {
                do {
                    if let data = try self.channel.receive(waitMs: 250) { self.receive(try RemoteWire.decode(data)) }
                } catch { self.fail(error.localizedDescription); break }
            }
        }
    }
    func send(_ message: RemoteMessage) {
        var needKey = false, overflow = false
        let wake = lock.withLock { () -> Bool in
            guard !stopped else { return false }
            if message.kind == .surface {
                pending.removeAll { $0.kind == .frame }; queuedBytes = 0; waitingKey = true
            }
            if let frame = message.frame {
                if waitingKey && !frame.keyFrame { return false }
                if pending.filter({ $0.kind == .frame }).count >= 3 || queuedBytes + frame.bytes.count > 24 * 1024 * 1024 {
                    pending.removeAll { $0.kind == .frame }; queuedBytes = 0; waitingKey = true; needKey = true
                    if !frame.keyFrame { return false }
                }
                if frame.keyFrame { waitingKey = false }
                queuedBytes += frame.bytes.count
            } else if message.kind == .input, message.input?.kind == .move,
                      pending.last?.kind == .input, pending.last?.input?.kind == .move,
                      pending.last?.identity == message.identity {
                pending[pending.count - 1] = message; return false
            }
            guard pending.count < 128 else { overflow = true; return false }
            pending.append(message)
            guard !scheduled else { return false }; scheduled = true; return true
        }
        if needKey { keyNeeded() }
        if overflow { fail("远程应用队列已满，已释放控制"); return }
        if wake { writer.async { [weak self] in self?.drain() } }
    }
    private func drain() {
        while let message = lock.withLock({ () -> RemoteMessage? in
            guard !stopped, !pending.isEmpty else { scheduled = false; return nil }
            let message = pending.removeFirst(); queuedBytes -= message.frame?.bytes.count ?? 0; return message
        }) {
            do { try channel.send(bytes: Data(RemoteWire.encode(message).dropFirst(4))) }
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

/// Structured control bridge to the same signed unidesk-input-service used by
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
