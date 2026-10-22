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
    private var keyFrameRequestPending = false
    private var maximumSendWaitMs = 0.0
    private var lastNetworkSample: (sent: UInt64, lost: UInt64, time: Double)?
    private let keyFrameNeeded: @Sendable () -> Void
    private let receive: @Sendable (RemoteMessage) -> Void
    private let state: @Sendable (String, Bool) -> Void
    init(channel: RemoteApplicationChannel, receive: @escaping @Sendable (RemoteMessage) -> Void,
         state: @escaping @Sendable (String, Bool) -> Void,
         keyFrameNeeded: @escaping @Sendable () -> Void = {}) {
        self.channel = channel; self.receive = receive; self.state = state; self.keyFrameNeeded = keyFrameNeeded
    }
    func start() {
        state("已通过 DeskFuse 配对连接", true)
        reader.async { [weak self] in
            guard let self else { return }
            while !self.lock.withLock({ self.stopped }) {
                do {
                    if let data = try self.channel.receive(waitMs: 250) { self.receive(try RemoteWire.decode(data)) }
                } catch { self.fail(self.displayMessage(for: error)); break }
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
        var requestKeyFrame = false, overflow = false
        let wake = lock.withLock { () -> Bool in
            guard !stopped else { return false }
            let frameBytes = message.frame?.bytes.count ?? 0
            if message.kind == .surface {
                pending.removeAll { $0.kind == .frame }; queuedBytes = 0; waitingKey = true; keyFrameRequestPending = false
            }
            if let frame = message.frame {
                if waitingKey && !frame.keyFrame {
                    if !keyFrameRequestPending { keyFrameRequestPending = true; requestKeyFrame = true }
                    return false
                }
                // A slow receiver must not turn media backpressure into a
                // session reset. Drop queued compressed frames as a unit and
                // rebuild the reference chain from a fresh key frame. Control
                // messages and input edges stay in the queue.
                if pending.filter({ $0.kind == .frame }).count >= 24 || queuedBytes + frame.bytes.count > 64 * 1024 * 1024 {
                    let droppedBytes = pending.reduce(into: 0) { total, value in
                        total += value.frame?.bytes.count ?? 0
                    }
                    pending.removeAll { $0.frame != nil }
                    queuedBytes -= droppedBytes
                    queuedBytes = max(0, queuedBytes)
                    waitingKey = true
                    if !frame.keyFrame {
                        if !keyFrameRequestPending { keyFrameRequestPending = true; requestKeyFrame = true }
                        return false
                    }
                }
                if frame.keyFrame { waitingKey = false; keyFrameRequestPending = false }
            } else if message.kind == .input, message.input?.kind == .move,
                      let index = pending.lastIndex(where: {
                          $0.kind == .input && $0.input?.kind == .move && $0.identity == message.identity
                      }) {
                pending[index] = message; return false
            }
            if message.kind == .heartbeat || message.kind == .presented {
                if message.kind == .heartbeat && message.diagnostics == nil {
                    message.diagnostics = pending.last(where: { $0.kind == .heartbeat && $0.identity == message.identity })?.diagnostics
                }
                pending.removeAll { $0.kind == message.kind && $0.identity == message.identity }
            }
            // 控制队列有上限：优先淘汰可被新指针状态覆盖的 move，其次丢弃排队视频帧并
            // 重建关键帧；仍无空间时关闭会话，绝不静默丢弃按键/按钮边沿或生命周期消息。
            if pending.count >= 128 {
                if let staleMove = pending.firstIndex(where: {
                    $0.kind == .input && $0.input?.kind == .move
                }) {
                    pending.remove(at: staleMove)
                } else if pending.contains(where: { $0.frame != nil }) {
                    pending.removeAll { $0.frame != nil }
                    queuedBytes = 0; waitingKey = true
                    if !keyFrameRequestPending { keyFrameRequestPending = true; requestKeyFrame = true }
                    // 参考链已断开：当前若是非关键帧也必须丢弃，关键帧则直接作为新链起点。
                    if let frame = message.frame {
                        guard frame.keyFrame else { return false }
                        waitingKey = false; keyFrameRequestPending = false; requestKeyFrame = false
                    }
                } else {
                    overflow = true; return false
                }
            }
            queuedBytes += frameBytes
            // Heartbeats, presentation ACKs, input and release messages must
            // overtake queued video once the current packet has drained.
            let urgent = message.frame == nil
                && [.heartbeat, .presented, .input, .release, .keyFrame, .error].contains(message.kind)
            if urgent, let firstFrame = pending.firstIndex(where: { $0.kind == .frame }) {
                pending.insert(message, at: firstFrame)
            } else {
                pending.append(message)
            }
            guard !scheduled else { return false }; scheduled = true; return true
        }
        if requestKeyFrame { keyFrameNeeded() }
        if overflow { fail("远程应用控制队列超限，连接已关闭以避免按键卡住"); return }
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
            catch { fail(displayMessage(for: error)); return }
        }
    }
    private func fail(_ message: String) {
        let first = lock.withLock { () -> Bool in
            guard !stopped else { return false }; stopped = true; pending.removeAll(); return true
        }
        if first { channel.close(); state(message, false) }
    }
    private func displayMessage(for error: Error) -> String {
        let message = error.userFacingMessage
        if message.localizedCaseInsensitiveContains("stream reset by peer")
            || message.localizedCaseInsensitiveContains("sending stopped by peer")
            || message.contains("视频通道读取失败")
            || message.contains("视频通道消息不完整")
            || message.contains("远程应用通道读取失败")
            || message.contains("远程应用通道消息不完整") {
            return "远程画面连接已断开，请点击“重新连接”再试。"
        }
        return message
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
