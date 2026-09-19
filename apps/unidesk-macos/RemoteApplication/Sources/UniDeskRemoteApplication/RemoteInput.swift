import AppKit
@preconcurrency import ApplicationServices

/// P0 remote input adapter. The signed helper posts native events on B only
/// while the selected app is in front and the authenticated A lease is alive.
/// Production integration must replace this with the shipping service lease.
public struct RemoteInputEvent: Codable, Sendable {
    public enum Kind: String, Codable, Sendable { case move, down, up, scroll, keyDown, keyUp, flags }
    public var kind: Kind
    public var point: CGPoint?
    public var button: UInt32 = 0
    public var clicks: Int64 = 1
    public var key: UInt16 = 0
    public var flags: UInt64 = 0
    public var repeatKey = false
    public var deltaX: Double = 0
    public var deltaY: Double = 0
    public var scrollPhase: Int64 = 0
    public var momentumPhase: Int64 = 0
    public init(kind: Kind, point: CGPoint? = nil) { self.kind = kind; self.point = point }
}

public struct InputHelperPacket: Codable, Sendable {
    public enum Kind: String, Codable, Sendable { case configure, event, heartbeat, release }
    public var kind: Kind
    public var surface: SurfaceIdentity?
    public var sequence: UInt64 = 0
    public var processID: Int32 = 0
    public var rect: CGRect?
    public var event: RemoteInputEvent?
    public var sentAt = ProcessInfo.processInfo.systemUptime
    public init(_ kind: Kind) { self.kind = kind }
}

/// In addition to surface generations, reject queued input older than the lease.
/// A heartbeat never resurrects an expired lease; only configure can do that.
public struct InputHelperLease: Sendable {
    public private(set) var surface: SurfaceIdentity?
    public private(set) var sequence: UInt64 = 0
    public private(set) var deadline: Double = 0
    public init() {}
    public mutating func configure(_ identity: SurfaceIdentity, now: Double) {
        surface = identity; sequence = 0; deadline = now + 1
    }
    public mutating func revoke() { surface = nil; deadline = 0 }
    public func alive(now: Double) -> Bool { surface != nil && now < deadline }
    public mutating func accept(_ packet: InputHelperPacket, now: Double) -> Bool {
        guard alive(now: now), packet.surface == surface, packet.sentAt <= now + 0.1,
              now - packet.sentAt < 1 else { return false }
        if packet.kind == .heartbeat { deadline = now + 1; return true }
        guard packet.kind == .event, packet.sequence > sequence else { return false }
        sequence = packet.sequence; return true
    }
}

/// The UI enqueues at most 128 packets. Only adjacent motion may be coalesced.
/// Overflow revokes control rather than losing a key/button boundary.
final class InputPipe: @unchecked Sendable {
    private let lock = NSLock()
    private let queue = DispatchQueue(label: "uniDesk.spike.input.pipe", qos: .userInteractive)
    private var pending: [InputHelperPacket] = []
    private var draining = false
    private var failed = false
    private let handle: FileHandle
    init(_ handle: FileHandle) { self.handle = handle }
    @discardableResult func send(_ packet: InputHelperPacket) -> Bool {
        lock.lock()
        guard !failed else { lock.unlock(); return false }
        if packet.kind == .release || packet.kind == .configure { pending.removeAll() }
        if packet.event?.kind == .move, pending.last?.event?.kind == .move,
           pending.last?.surface == packet.surface { pending[pending.count - 1] = packet }
        else if pending.count >= 128 {
            pending = [InputHelperPacket(.release)]; failed = true
        } else { pending.append(packet) }
        let start = !draining; draining = true
        let accepted = !failed
        lock.unlock()
        if start { queue.async { self.drain() } }
        return accepted
    }
    private func drain() {
        while true {
            lock.lock()
            guard !pending.isEmpty else { draining = false; lock.unlock(); return }
            let packet = pending.removeFirst(); lock.unlock()
            do {
                var data = try JSONEncoder().encode(packet); data.append(10)
                try handle.write(contentsOf: data)
            } catch {
                lock.lock(); failed = true; pending.removeAll(); draining = false; lock.unlock()
                return
            }
        }
    }
    func close() {
        _ = send(InputHelperPacket(.release))
        queue.async { try? self.handle.close() }
    }
}

/// Remote input fast path stays off the main actor. State changes are serialized
/// with incoming events; the child alone owns native pressed-state and watchdog.
public final class RemoteInputRelay: @unchecked Sendable {
    private let lock = NSLock()
    private var child: Process?
    private var pipe: InputPipe?
    private var gate = SurfaceGate()
    private var surface: CaptureSurface?
    private var target: Int32 = 0
    private var focused = false
    private var configured = false
    private var buttons = Set<UInt32>()
    private var lastPresented: UInt64 = 0
    private var lastProgress = ProcessInfo.processInfo.systemUptime
    private var latestEncoded: UInt64 = 0
    private let externalSend: (@Sendable (InputHelperPacket) -> Bool)?
    public init(sender: (@Sendable (InputHelperPacket) -> Bool)? = nil) { externalSend = sender }
    private func transmit(_ packet: InputHelperPacket) -> Bool { externalSend?(packet) ?? (pipe?.send(packet) == true) }
    public var gestureActive: Bool { lock.withLock { !buttons.isEmpty } }
    public static var authorized: Bool { AXIsProcessTrusted() }
    public static func requestPermission() {
        _ = AXIsProcessTrustedWithOptions([kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String: true] as CFDictionary)
    }
    public func start(processID: Int32) throws {
        stop()
        if externalSend != nil { lock.withLock { target = processID }; return }
        guard Self.authorized else { throw RemoteApplicationError.permission("B 端需要辅助功能权限才能控制应用") }
        let process = Process(), input = Pipe()
        process.executableURL = Bundle.main.executableURL
        process.arguments = ["--remote-input-helper"]
        process.standardInput = input; process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.standardError
        try process.run(); try? input.fileHandleForReading.close()
        lock.withLock { child = process; pipe = InputPipe(input.fileHandleForWriting); target = processID }
    }
    public func setSurface(_ value: CaptureSurface) {
        lock.withLock {
            releaseLocked(); surface = value; gate.begin(value.identity)
            lastPresented = 0; latestEncoded = 0; lastProgress = ProcessInfo.processInfo.systemUptime
        }
    }

    public func encoded(_ frame: EncodedVideoFrame) {
        lock.withLock { if frame.surface == surface?.identity { latestEncoded = frame.sequence } }
    }
    public func focus(_ value: Bool) {
        lock.withLock { releaseLocked(); focused = value; if value { armLocked() } }
    }
    private func armLocked() {
        guard focused, gate.presented, !configured, let surface, (externalSend != nil || child?.isRunning == true) else { return }
        var packet = InputHelperPacket(.configure)
        packet.surface = surface.identity; packet.processID = target; packet.rect = surface.contentRect
        configured = transmit(packet)
    }
    /// Called directly on the QUIC receive queue, not through a UI snapshot/task.
    public func receive(_ message: RemoteMessage) {
        lock.withLock {
            guard message.identity == surface?.identity else { return }
            if message.kind == .presented {
                guard gate.acknowledge(message.identity!), message.sequence <= latestEncoded else { return }
                if message.sequence > lastPresented {
                    lastPresented = message.sequence; lastProgress = ProcessInfo.processInfo.systemUptime
                }
                armLocked(); return
            }
            if message.kind == .heartbeat {
                let now = ProcessInfo.processInfo.systemUptime
                guard configured, latestEncoded == lastPresented || now - lastProgress < 1 else {
                    releaseLocked(); return
                }
                var packet = InputHelperPacket(.heartbeat); packet.surface = surface?.identity
                if !transmit(packet) { releaseLocked() }
                return
            }
            guard message.kind == .input, configured, let event = message.input,
                  gate.accept(message.identity!, sequence: message.sequence) else { return }
            var packet = InputHelperPacket(.event); packet.surface = message.identity
            packet.sequence = message.sequence; packet.event = event
            guard transmit(packet) else { releaseLocked(); return }
            if event.kind == .down { buttons.insert(event.button) }
            if event.kind == .up { buttons.remove(event.button) }
        }
    }
    private func releaseLocked() {
        if configured {
            var packet = InputHelperPacket(.release); packet.surface = surface?.identity; _ = transmit(packet)
        }
        configured = false; buttons.removeAll()
    }
    public func suspend() {
        lock.withLock { releaseLocked(); gate.revoke(); surface = nil }
    }
    public func stop() {
        lock.withLock {
            releaseLocked(); gate.revoke(); surface = nil; focused = false
            pipe?.close(); pipe = nil; child = nil; target = 0
        }
    }
}

private final class RemoteInputInjector: @unchecked Sendable {
    var lease = InputHelperLease()
    var pid: Int32 = 0
    var rect = CGRect.zero
    var point = CGPoint.zero
    var buttons = Set<UInt32>()
    var keys = Set<UInt16>()
    let source = CGEventSource(stateID: .privateState)
    func release() {
        for button in buttons { postMouse(button: button, down: false, flags: 0, clicks: 1) }
        for key in keys {
            let event = CGEvent(keyboardEventSource: source, virtualKey: key, keyDown: false)
            event?.flags = []; post(event)
        }
        buttons.removeAll(); keys.removeAll(); lease.revoke()
    }
    func tick() {
        if !lease.alive(now: ProcessInfo.processInfo.systemUptime) || !AXIsProcessTrusted()
            || (pid > 0 && NSWorkspace.shared.frontmostApplication?.processIdentifier != pid) { release() }
    }
    func receive(_ packet: InputHelperPacket) {
        let now = ProcessInfo.processInfo.systemUptime
        if packet.kind == .release { release(); return }
        if packet.kind == .configure {
            release()
            guard AXIsProcessTrusted(), packet.processID > 0, packet.processID != getpid(),
                  kill(packet.processID, 0) == 0, let identity = packet.surface, let rect = packet.rect,
                  [rect.minX, rect.minY, rect.width, rect.height].allSatisfy(\.isFinite),
                  !rect.isEmpty, now - packet.sentAt < 1 else { return }
            pid = packet.processID; self.rect = rect
            point = CGPoint(x: rect.midX, y: rect.midY)
            lease.configure(identity, now: now); return
        }
        guard lease.accept(packet, now: now), AXIsProcessTrusted(), kill(pid, 0) == 0 else { tick(); return }
        if NSWorkspace.shared.frontmostApplication?.processIdentifier != pid {
            release(); return
        }
        guard let input = packet.event else { return }
        if let normalized = input.point {
            guard normalized.x.isFinite, normalized.y.isFinite,
                  (0...1).contains(normalized.x), (0...1).contains(normalized.y) else { return }
            point = CGPoint(x: rect.minX + normalized.x * rect.width, y: rect.minY + normalized.y * rect.height)
        }
        switch input.kind {
        case .down, .up:
            guard input.button <= 2 else { return }
            if input.kind == .down { buttons.insert(input.button) }
            else { guard buttons.remove(input.button) != nil else { return } }
            postMouse(button: input.button, down: input.kind == .down, flags: input.flags, clicks: input.clicks)
        case .move:
            let button = buttons.sorted().first
            let type: CGEventType = button == nil ? .mouseMoved : (button == 0 ? .leftMouseDragged : (button == 1 ? .rightMouseDragged : .otherMouseDragged))
            let event = CGEvent(mouseEventSource: source, mouseType: type, mouseCursorPosition: point,
                                mouseButton: CGMouseButton(rawValue: button ?? 0) ?? .left)
            postPointer(event, flags: input.flags)
        case .scroll:
            guard input.deltaX.isFinite, input.deltaY.isFinite else { return }
            let event = CGEvent(scrollWheelEvent2Source: source, units: .pixel, wheelCount: 2,
                wheel1: Int32(max(-10000, min(10000, input.deltaY))), wheel2: Int32(max(-10000, min(10000, input.deltaX))), wheel3: 0)
            event?.setDoubleValueField(.scrollWheelEventPointDeltaAxis1, value: input.deltaY)
            event?.setDoubleValueField(.scrollWheelEventPointDeltaAxis2, value: input.deltaX)
            event?.setIntegerValueField(.scrollWheelEventScrollPhase, value: input.scrollPhase)
            event?.setIntegerValueField(.scrollWheelEventMomentumPhase, value: input.momentumPhase)
            postPointer(event, flags: input.flags)
        case .keyDown, .keyUp, .flags:
            guard input.key < 128 else { return }
            let down = input.kind == .keyDown || (input.kind == .flags && modifierDown(input.key, input.flags))
            if down { keys.insert(input.key) } else { keys.remove(input.key) }
            let event = CGEvent(keyboardEventSource: source, virtualKey: input.key, keyDown: down)
            event?.flags = CGEventFlags(rawValue: input.flags)
            if input.kind == .flags { event?.type = .flagsChanged }
            event?.setIntegerValueField(.keyboardEventAutorepeat, value: input.repeatKey ? 1 : 0)
            post(event)
        }
    }
    func modifierDown(_ key: UInt16, _ flags: UInt64) -> Bool {
        let mask: CGEventFlags
        switch key {
        case 54, 55: mask = .maskCommand
        case 56, 60: mask = .maskShift
        case 58, 61: mask = .maskAlternate
        case 59, 62: mask = .maskControl
        case 57: mask = .maskAlphaShift
        case 63: mask = .maskSecondaryFn
        default: return false
        }
        return CGEventFlags(rawValue: flags).contains(mask)
    }
    func postMouse(button: UInt32, down: Bool, flags: UInt64, clicks: Int64) {
        let type: CGEventType = button == 0 ? (down ? .leftMouseDown : .leftMouseUp)
            : button == 1 ? (down ? .rightMouseDown : .rightMouseUp) : (down ? .otherMouseDown : .otherMouseUp)
        let event = CGEvent(mouseEventSource: source, mouseType: type, mouseCursorPosition: point,
                            mouseButton: CGMouseButton(rawValue: button) ?? .left)
        event?.setIntegerValueField(.mouseEventClickState, value: max(1, min(3, clicks)))
        postPointer(event, flags: flags)
    }
    func postPointer(_ event: CGEvent?, flags: UInt64) {
        guard let event else { return }
        event.location = point; event.flags = CGEventFlags(rawValue: flags)
        post(event)
    }
    func post(_ event: CGEvent?) {
        guard let event else { return }
        event.setIntegerValueField(.eventSourceUserData, value: 0x55445350494b45)
        event.post(tap: .cghidEventTap)
    }
}

public enum RemoteInputHelper {
    /// No listening socket. The only authority is this inherited parent pipe.
    public static func run() {
        let worker = DispatchQueue(label: "uniDesk.spike.input.inject", qos: .userInteractive)
        let injector = RemoteInputInjector()
        let timer = DispatchSource.makeTimerSource(queue: worker)
        timer.schedule(deadline: .now(), repeating: .milliseconds(100))
        timer.setEventHandler { injector.tick() }; timer.resume()
        while let line = readLine() {
            guard line.utf8.count < 8192, let data = line.data(using: .utf8),
                  let packet = try? JSONDecoder().decode(InputHelperPacket.self, from: data) else { break }
            worker.sync { injector.receive(packet) }
        }
        timer.cancel(); worker.sync { injector.release() }
    }
}
