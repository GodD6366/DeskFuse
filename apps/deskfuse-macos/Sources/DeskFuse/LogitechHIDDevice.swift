import Foundation
import IOKit.hid
import OSLog

struct LogitechProbeBackoff {
    private(set) var failures = 0
    private(set) var nextAttempt: TimeInterval = 0

    mutating func failed(at now: TimeInterval) {
        failures = min(failures + 1, 2)
        nextAttempt = now + (failures == 1 ? 30 : 60)
    }
}

/// The handle is retained for queued writes; only this serial queue writes/closes it.
/// Input report registration and buffer lifetime remain on the main run loop.
private final class LogitechHIDWriter: @unchecked Sendable {
    let device: IOHIDDevice
    let queue = DispatchQueue(label: "cc.deskfuse.logitech-hid-output")
    init(_ device: IOHIDDevice) { self.device = device }
    func write(_ bytes: [UInt8], completion: @escaping @Sendable (IOReturn) -> Void) {
        queue.async { [self] in
            let result = bytes.withUnsafeBufferPointer {
                IOHIDDeviceSetReport(device, kIOHIDReportTypeOutput, CFIndex(bytes[0]), $0.baseAddress!, bytes.count)
            }
            completion(result)
        }
    }
    func close() {
        let completed = DispatchSemaphore(value: 0)
        queue.async { [self] in
            IOHIDDeviceClose(device, 0)
            completed.signal()
        }
        // App termination does not await Tasks. Give queued restore reports a bounded
        // opportunity to finish, without letting a disconnected device hang shutdown.
        _ = completed.wait(timeout: .now() + .milliseconds(250))
    }
}

@MainActor
final class LogitechHIDDevice {
    struct Mouse {
        enum Backend: String { case reprogrammable, buttonSpy }
        var name: String
        var feature: UInt8
        var available: Set<Int>
        var backend: Backend = .reprogrammable
        var spyEnabled = false
        var owned: Set<Int> = []
        var pressed: Set<Int> = []
    }
    let device: IOHIDDevice
    private let writer: LogitechHIDWriter
    private let buffer: UnsafeMutablePointer<UInt8>
    private let bufferSize: Int
    private var pending: CheckedContinuation<[UInt8]?, Never>?
    private var timeout: Task<Void, Never>?
    private var expected: [UInt8] = []
    private var serial: UInt64 = 0
    private var requestCount = 0, reportCount = 0, timeoutCount = 0, protocolErrors = 0
    private var duplicateReports = 0, ignoredReports = 0, writeFailures = 0
    private var inputFailures = 0
    private var requestStarted: ContinuousClock.Instant?
    private var maxRequestMilliseconds = 0.0
    private var probes: [UInt8: LogitechProbeBackoff] = [:]
    private static let logger = Logger(subsystem: "cc.deskfuse", category: "logitech_hid")
    private var lastError: String?
    private var lastRequest: String?
    private var events: [String] = []
    private func record(_ message: String) {
        events.append("\(Date().ISO8601Format()) \(message)")
        if events.count > 24 { events.removeFirst(events.count - 24) }
        Self.logger.notice("\(message, privacy: .public)")
    }
    func diagnostics() -> [String: Any] {
        ["product": IOHIDDeviceGetProperty(device, kIOHIDProductKey as CFString) as? String ?? "",
         "transport": IOHIDDeviceGetProperty(device, kIOHIDTransportKey as CFString) as? String ?? "",
         "requests": requestCount, "reports": reportCount, "timeouts": timeoutCount, "protocol_errors": protocolErrors,
         "duplicate_button_reports": duplicateReports, "ignored_reports": ignoredReports,
         "write_failures": writeFailures, "request_max_ms": maxRequestMilliseconds,
         "input_failures": inputFailures,
         "probe_backoff": probes.map { ["slot": Int($0.key), "failures": $0.value.failures,
             "retry_in_seconds": max(0, $0.value.nextAttempt - ProcessInfo.processInfo.systemUptime)] as [String: Any] },
         "last_request": lastRequest as Any? ?? NSNull(), "last_error": lastError as Any? ?? NSNull(), "recent_events": events,
         "mice": mice.keys.sorted().compactMap { slot -> [String: Any]? in
             guard let mouse = mice[slot] else { return nil }
             return ["slot": Int(slot), "name": mouse.name, "feature": Int(mouse.feature),
                     "backend": mouse.backend.rawValue, "spy_enabled": mouse.spyEnabled,
                     "available": mouse.available.sorted(), "owned": mouse.owned.sorted(), "pressed": mouse.pressed.sorted()]
         }]
    }
    private(set) var mice: [UInt8: Mouse] = [:]
    private(set) var closed = false
    var onButton: ((UInt8, Int, Bool) -> Void)?
    var onStatus: ((String) -> Void)?
    var onControlsChanged: (() -> Void)?

    init?(_ device: IOHIDDevice) {
        guard IOHIDDeviceOpen(device, 0) == kIOReturnSuccess else { return nil }
        self.device = device
        writer = LogitechHIDWriter(device)
        bufferSize = max(64, min(4096, (IOHIDDeviceGetProperty(device, kIOHIDMaxInputReportSizeKey as CFString) as? NSNumber)?.intValue ?? 64))
        buffer = .allocate(capacity: bufferSize)
        IOHIDDeviceRegisterInputReportCallback(device, buffer, bufferSize, { context, result, _, _, reportID, bytes, length in
            guard let context else { return }
            let instance = Unmanaged<LogitechHIDDevice>.fromOpaque(context).takeUnretainedValue()
            MainActor.assumeIsolated {
                guard result == kIOReturnSuccess else {
                    instance.inputFailures += 1
                    if instance.inputFailures == 1 || instance.inputFailures.isMultiple(of: 60) {
                        instance.record("input_failed code=\(result) count=\(instance.inputFailures)")
                    }
                    return
                }
                instance.receive(Array(UnsafeBufferPointer(start: bytes, count: length)), reportID: reportID)
            }
        }, Unmanaged.passUnretained(self).toOpaque())
        IOHIDDeviceScheduleWithRunLoop(device, CFRunLoopGetMain(), CFRunLoopMode.commonModes.rawValue)
    }

    func retryDiscovery() { probes.removeAll() }

    func discover() async {
        let product = IOHIDDeviceGetProperty(device, kIOHIDProductKey as CFString) as? String ?? ""
        let receiver = product.lowercased().contains("receiver")
        // Direct Bluetooth HID++ devices use either ff or 00. Do not probe both
        // after finding the mouse: they may be aliases of the same endpoint.
        let slots: [UInt8] = receiver ? [1, 2, 3, 4, 5, 6] : [0xff, 0]
        if !receiver && !mice.isEmpty { return }
        for slot in slots where mice[slot] == nil {
            guard !closed, !Task.isCancelled else { return }
            let now = ProcessInfo.processInfo.systemUptime
            guard now >= (probes[slot]?.nextAttempt ?? 0) else { continue }
            defer {
                if mice[slot] == nil {
                    probes[slot, default: LogitechProbeBackoff()].failed(at: ProcessInfo.processInfo.systemUptime)
                } else { probes.removeValue(forKey: slot) }
            }
            guard let nameFeature = await request(slot, 0, 0, [0, 5]), let nf = nameFeature.first, nf != 0,
                  let length = await request(slot, nf, 0, []), let size = length.first, size > 0, size < 128 else { continue }
            var nameBytes: [UInt8] = []
            while nameBytes.count < Int(size) {
                guard let part = await request(slot, nf, 1, [UInt8(nameBytes.count)]), !part.isEmpty else { break }
                nameBytes += part.prefix(Int(size) - nameBytes.count)
            }
            guard nameBytes.count == Int(size) else { continue }
            let name = String(decoding: nameBytes, as: UTF8.self)
            guard let model = LogitechHIDProtocol.model(name) else { continue }
            if model == .g502Lightspeed {
                guard let feature = await request(slot, 0, 0, [0x81, 0x10]), let index = feature.first, index != 0,
                      let count = await request(slot, index, 0, []), let n = count.first,
                      let controls = LogitechHIDProtocol.spyButtons(count: Int(n)), !closed else { continue }
                mice[slot] = Mouse(name: name, feature: index, available: controls, backend: .buttonSpy)
                record("discovered slot=\(slot) button_spy count=\(n)")
                onControlsChanged?()
                onStatus?("\(name)：已识别 \(n) 个实体键；可采集并附加快捷键，原有功能保留。")
                if !receiver { break }
                continue
            }
            guard let feature = await request(slot, 0, 0, [0x1b, 4]), let index = feature.first, index != 0,
                  let count = await request(slot, index, 0, []), let n = count.first, n > 0, n <= 64 else { continue }
            var controls: Set<Int> = []
            var complete = true
            for i in 0..<n {
                guard let info = await request(slot, index, 1, [i]) else { complete = false; break }
                if let cid = LogitechHIDProtocol.control(info) { controls.insert(cid) }
            }
            guard !closed else { return }
            guard complete else { continue }
            mice[slot] = Mouse(name: name, feature: index, available: controls)
            onControlsChanged?()
            record("discovered slot=\(slot) controls=\(controls.sorted())")
            onStatus?("\(name)：发现 \(controls.count) 个可接管的扩展按键")
            if !receiver { break }
        }
        if mice.isEmpty {
            onStatus?("HID++ 接口已连接，但未识别到在线的 MX Master 3 / 3S 或 G502 LIGHTSPEED。\(lastError ?? "请唤醒鼠标后重新采集。")")
        }
    }

    func configure(_ desired: Set<Int>) async {
        for slot in mice.keys.sorted() {
            guard let mouse = mice[slot] else { continue }
            if mouse.backend == .buttonSpy {
                await configureSpy(slot, desired: desired)
                continue
            }
            for cid in mouse.available.sorted() {
                guard !closed, !Task.isCancelled, let current = mice[slot] else { return }
                let wants = desired.contains(cid)
                if !wants && !current.owned.contains(cid) { continue }
                // Keep the release notification when a capture or setting changes during a hold.
                if !wants && current.pressed.contains(cid) { continue }
                let id: [UInt8] = [UInt8(cid >> 8), UInt8(cid & 255)]
                guard let report = await request(slot, current.feature, 2, id), report.count >= 5,
                      Array(report.prefix(2)) == id else {
                    onStatus?(lastError ?? "HID++ 响应无效，请唤醒鼠标后重新采集")
                    continue
                }
                let flags = report[2]
                if wants && !current.owned.contains(cid) && flags & 0x05 != 0 {
                    onStatus?("\(current.name)：\(MouseSettings.buttonName(cid)) 已被其他程序接管")
                    continue
                }
                if wants && current.owned.contains(cid) && flags & 1 != 0 { continue }
                if !wants && !current.owned.contains(cid) { continue }
                // Track intent before write, so a lost reply still gets a restore on shutdown.
                if wants { mice[slot]?.owned.insert(cid) }
                let payload = LogitechHIDProtocol.reporting(cid, divert: wants)
                _ = await request(slot, current.feature, 3, payload)
                guard !closed else { return }
                let verified = await request(slot, current.feature, 2, id)
                if let verified, verified.count >= 5, Array(verified.prefix(2)) == id,
                   (verified[2] & 1 != 0) == wants {
                    if !wants { mice[slot]?.owned.remove(cid) }
                    record("diversion slot=\(slot) cid=\(cid) enabled=\(wants)")
                    onStatus?("\(current.name)：\(MouseSettings.buttonName(cid))\(wants ? "已接管" : "已恢复原始功能")")
                } else {
                    onStatus?("\(current.name)：按键接管未确认，将重试")
                }
            }
        }
    }

    private func configureSpy(_ slot: UInt8, desired: Set<Int>) async {
        guard let mouse = mice[slot], !closed, !Task.isCancelled else { return }
        let wanted = desired.intersection(mouse.available).union(mouse.pressed)
        let enable = !wanted.isEmpty
        guard enable || mouse.spyEnabled else { return }
        // Mark before sending: a lost start reply must still be paired with stop.
        if enable {
            mice[slot]?.spyEnabled = true
            // Reports can arrive before the start acknowledgement.
            mice[slot]?.owned = wanted
        }
        guard await request(slot, mouse.feature, enable ? 1 : 2, []) != nil, !closed else {
            onStatus?(lastError ?? "G502 实体键监听未确认，请重新采集。")
            return
        }
        mice[slot]?.spyEnabled = enable
        mice[slot]?.owned = wanted
        if mouse.spyEnabled != enable { record("button_spy slot=\(slot) enabled=\(enable)") }
        onStatus?(enable ? "\(mouse.name)：实体键监听已开启，鼠标原有功能保留。" : "\(mouse.name)：实体键监听已停止。")
    }

    func close() {
        guard !closed else { return }
        closed = true
        finish(nil)
        for (slot, mouse) in mice {
            for cid in mouse.pressed { onButton?(slot, cid, false) }
            if mouse.backend == .buttonSpy {
                if mouse.spyEnabled { writer.write(packet(slot, mouse.feature, 2, [])) { _ in } }
                continue
            }
            for cid in mouse.owned {
                writer.write(packet(slot, mouse.feature, 3, LogitechHIDProtocol.reporting(cid, divert: false))) { _ in }
            }
        }
        IOHIDDeviceUnscheduleFromRunLoop(device, CFRunLoopGetMain(), CFRunLoopMode.commonModes.rawValue)
        IOHIDDeviceRegisterInputReportCallback(device, buffer, bufferSize, nil, nil)
        buffer.deallocate()
        writer.close()
        mice.removeAll()
    }
    private func packet(_ slot: UInt8, _ feature: UInt8, _ function: UInt8, _ parameters: [UInt8]) -> [UInt8] {
        serial &+= 1
        expected = [slot, feature, function << 4 | UInt8(serial % 15 + 1)]
        let bytes = [UInt8(0x11)] + expected + parameters
        return bytes + Array(repeating: 0, count: max(0, 20 - bytes.count))
    }
    private func request(_ slot: UInt8, _ feature: UInt8, _ function: UInt8, _ parameters: [UInt8]) async -> [UInt8]? {
        guard !closed, !Task.isCancelled, pending == nil else { return nil }
        return await withCheckedContinuation { continuation in
            let bytes = packet(slot, feature, function, parameters)
            let generation = serial
            requestCount += 1
            requestStarted = .now
            lastRequest = expected.map { String(format: "%02x", $0) }.joined(separator: " ")
            pending = continuation
            writer.write(bytes) { [weak self] result in
                guard result != kIOReturnSuccess else { return }
                Task { @MainActor in
                    guard let self, self.serial == generation, result != kIOReturnSuccess else { return }
                    self.writeFailures += 1
                    self.lastError = "HID++ 写入失败（\(result)），请检查输入监控权限或重新连接鼠标。"
                    self.record(self.lastError!)
                    self.finish(nil)
                }
            }
            timeout = Task { [weak self] in
                do { try await Task.sleep(for: .milliseconds(600)) } catch { return }
                guard let self, self.serial == generation else { return }
                self.timeoutCount += 1
                self.lastError = "HID++ 请求 \(self.lastRequest ?? "") 超时。"
                self.record(self.lastError!)
                self.finish(nil)
            }
        }
    }
    private func finish(_ value: [UInt8]?) {
        if let requestStarted {
            let duration = requestStarted.duration(to: .now).components
            maxRequestMilliseconds = max(maxRequestMilliseconds,
                Double(duration.seconds) * 1_000 + Double(duration.attoseconds) / 1e15)
            self.requestStarted = nil
        }
        timeout?.cancel(); timeout = nil
        let continuation = pending; pending = nil
        continuation?.resume(returning: value)
    }
    private func receive(_ report: [UInt8], reportID: UInt32) {
        guard !closed, let bytes = LogitechHIDProtocol.report(report, reportID: reportID), bytes.count >= 6 else { return }
        reportCount += 1
        if pending != nil && Array(bytes.prefix(3)) == expected { lastError = nil; finish(Array(bytes.dropFirst(3))); return }
        if pending != nil, let code = LogitechHIDProtocol.error(bytes, matching: expected) {
            protocolErrors += 1
            lastError = String(format: "HID++ 请求 %@ 返回错误 0x%02x（设备未就绪或不支持）。", lastRequest ?? "", code)
            record(lastError!)
            finish(nil); return
        }
        let slot = bytes[0]
        guard let mouse = mice[slot], bytes[1] == mouse.feature, bytes[2] == 0 else {
            ignoredReports += 1
            return
        }
        let payload = Array(bytes.dropFirst(3))
        let state = mouse.backend == .buttonSpy
            ? LogitechHIDProtocol.spyPressed(payload, count: mouse.available.count)
            : LogitechHIDProtocol.pressed(payload)
        guard let all = state else { return }
        let next = all.intersection(mouse.owned)
        guard next != mouse.pressed else { duplicateReports += 1; return }
        mice[slot]?.pressed = next
        for cid in mouse.pressed.subtracting(next) { onButton?(slot, cid, false) }
        for cid in next.subtracting(mouse.pressed) { onButton?(slot, cid, true) }
    }
}
