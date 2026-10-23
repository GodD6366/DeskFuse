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
        // MARK: Button remapping
        var name: String
        var feature: UInt8
        var available: Set<Int>
        var backend: Backend = .reprogrammable
        var spyEnabled = false
        var owned: Set<Int> = []
        var pressed: Set<Int> = []
        // MARK: Battery (feature 0x1004 unified, 0x1000 legacy, or 0x1001 voltage)
        enum BatteryKind: String { case unified, legacy, voltage }
        var battery: (kind: BatteryKind, index: UInt8)? = nil
        var batteryPercent: UInt8? = nil      // nil = unknown
        var batteryCharging: Bool = false
        // MARK: SmartShift wheel mode (feature 0x2110)
        var smartshiftFeature: UInt8? = nil
        var wheelFreespin: Bool? = nil          // nil = not supported / unknown
    }
    struct Identity: Equatable {
        var name: String
        var supported: Bool
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
         "identities": identities.keys.sorted().compactMap { slot -> [String: Any]? in
             guard let identity = identities[slot] else { return nil }
             return ["slot": Int(slot), "name": identity.name, "supported": identity.supported]
         },
         "mice": mice.keys.sorted().compactMap { slot -> [String: Any]? in
             guard let mouse = mice[slot] else { return nil }
             return ["slot": Int(slot), "name": mouse.name, "feature": Int(mouse.feature),
                     "backend": mouse.backend.rawValue, "spy_enabled": mouse.spyEnabled,
                     "available": mouse.available.sorted(), "owned": mouse.owned.sorted(), "pressed": mouse.pressed.sorted(),
                     "battery_feature": mouse.battery.map { "\($0.kind.rawValue):\($0.index)" } as Any? ?? NSNull(),
                     "battery_percent": mouse.batteryPercent.map { Int($0) } as Any? ?? NSNull(),
                     "battery_charging": mouse.batteryCharging,
                     "smartshift_feature": mouse.smartshiftFeature.map { Int($0) } as Any? ?? NSNull(),
                     "wheel_freespin": mouse.wheelFreespin as Any? ?? NSNull()] as [String: Any]
         }]
    }
    private(set) var mice: [UInt8: Mouse] = [:]
    private(set) var identities: [UInt8: Identity] = [:]
    private(set) var closed = false
    var onButton: ((UInt8, Int, Bool) -> Void)?
    var onStatus: ((String) -> Void)?
    var onControlsChanged: (() -> Void)?
    var onIdentitiesChanged: (() -> Void)?
    /// Fired when battery or SmartShift state changes for any slot.
    var onCapabilitiesChanged: (() -> Void)?

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
        if !receiver && !identities.isEmpty { return }
        for slot in slots where mice[slot] == nil && identities[slot] == nil {
            guard !closed, !Task.isCancelled else { return }
            let now = ProcessInfo.processInfo.systemUptime
            guard now >= (probes[slot]?.nextAttempt ?? 0) else { continue }
            defer {
                if mice[slot] == nil && identities[slot] == nil {
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
            let model = LogitechHIDProtocol.model(name)
            if model == .g502Lightspeed {
                guard let feature = await request(slot, 0, 0, [0x81, 0x10]), let index = feature.first, index != 0,
                      let count = await request(slot, index, 0, []), let n = count.first,
                      let controls = LogitechHIDProtocol.spyButtons(count: Int(n)), !closed else { continue }
                mice[slot] = Mouse(name: name, feature: index, available: controls, backend: .buttonSpy)
                identify(slot, name: name, supported: true)
                record("discovered slot=\(slot) button_spy count=\(n)")
                onControlsChanged?()
                onStatus?("\(name)：已识别 \(n) 个实体键；可采集并附加快捷键，原有功能保留。")
                if !receiver { break }
                continue
            }
            // Reprogrammable controls (feature 0x1b04) is a standard HID++ 2.0
            // capability shared by most Logitech mice. Try it for every model
            // we don't route to button-spy — named MX devices and unknown
            // Logitech mice alike — instead of rejecting unknown names up front.
            guard let feature = await request(slot, 0, 0, [0x1b, 4]), let index = feature.first, index != 0,
                  let count = await request(slot, index, 0, []), let n = count.first, n > 0, n <= 64 else {
                identify(slot, name: name, supported: false)
                record("identified unsupported slot=\(slot) name=\(name)")
                onStatus?("\(name)：暂未适配，可使用通用鼠标映射。")
                if !receiver { break }
                continue
            }
            var controls: Set<Int> = []
            var complete = true
            for i in 0..<n {
                guard let info = await request(slot, index, 1, [i]) else { complete = false; break }
                if let cid = LogitechHIDProtocol.control(info) { controls.insert(cid) }
            }
            guard !closed else { return }
            guard complete else { continue }
            guard !controls.isEmpty else {
                identify(slot, name: name, supported: false)
                record("slot=\(slot) name=\(name) has no divertable controls")
                onStatus?("\(name)：未发现可接管的扩展按键，可使用通用鼠标映射。")
                if !receiver { break }
                continue
            }
            mice[slot] = Mouse(name: name, feature: index, available: controls)
            identify(slot, name: name, supported: true)
            onControlsChanged?()
            record("discovered slot=\(slot) name=\(name) controls=\(controls.sorted())")
            onStatus?("\(name)：发现 \(controls.count) 个可接管的扩展按键")
            if !receiver { break }
        }
        // Probe battery and SmartShift for every successfully discovered mouse.
        for slot in mice.keys.sorted() {
            guard !closed, !Task.isCancelled else { return }
            await probeCapabilities(slot)
        }
        if mice.isEmpty && identities.isEmpty {
            onStatus?("HID++ 接口已连接，但未识别到在线鼠标。\(lastError ?? "请唤醒鼠标后重新采集。")")
        }
    }

    // MARK: - Battery & SmartShift capability probes

    private func probeCapabilities(_ slot: UInt8) async {
        guard mice[slot] != nil, !closed else { return }
        await probeBattery(slot)
        await probeSmartShift(slot)
    }

    private func probeBattery(_ slot: UInt8) async {
        guard !closed, !Task.isCancelled else { return }
        // Prefer 0x1004 (UnifiedBattery), then 0x1000 (legacy BatteryStatus), then
        // 0x1001 (BatteryVoltage) — the only one G502 LIGHTSPEED exposes.
        let candidates: [(Mouse.BatteryKind, [UInt8])] = [(.unified, [0x10, 0x04]), (.legacy, [0x10, 0x00]), (.voltage, [0x10, 0x01])]
        for (kind, id) in candidates {
            guard !closed, let reply = await request(slot, 0, 0, id),
                  let idx = reply.first, idx != 0 else { continue }
            mice[slot]?.battery = (kind, idx)
            await refreshBattery(slot)
            return
        }
    }

    private func refreshBattery(_ slot: UInt8) async {
        guard let battery = mice[slot]?.battery else { return }
        switch battery.kind {
        case .unified: await refreshBattery1004(slot, featureIndex: battery.index)
        case .legacy: await refreshBattery1000(slot, featureIndex: battery.index)
        case .voltage: await refreshBattery1001(slot, featureIndex: battery.index)
        }
    }

    private func refreshBattery1004(_ slot: UInt8, featureIndex: UInt8) async {
        // getStatus = function 1; payload[0]=percent, payload[1]=level, payload[2]=status
        // status: 0=discharging, 1/2=charging, 3=full, 4=slow, 5-7=error
        guard let reply = await request(slot, featureIndex, 1, []),
              reply.count >= 3 else { return }
        let pct = reply[0]
        let statusByte = reply[2]
        let charging = statusByte >= 1 && statusByte <= 4
        let changed = mice[slot]?.batteryPercent != pct || mice[slot]?.batteryCharging != charging
        mice[slot]?.batteryPercent = pct
        mice[slot]?.batteryCharging = charging
        if changed {
            record("battery slot=\(slot) pct=\(pct) charging=\(charging)")
            onCapabilitiesChanged?()
        }
    }

    private func refreshBattery1000(_ slot: UInt8, featureIndex: UInt8) async {
        // getBatteryLevelStatus = function 0; payload[0]=percent, payload[2]=status
        // status: 0=discharging, 1=recharging, 2=almost-full
        guard let reply = await request(slot, featureIndex, 0, []),
              reply.count >= 3 else { return }
        let pct = reply[0]
        let statusByte = reply[2]
        let charging = statusByte == 1 || statusByte == 2
        let changed = mice[slot]?.batteryPercent != pct || mice[slot]?.batteryCharging != charging
        mice[slot]?.batteryPercent = pct
        mice[slot]?.batteryCharging = charging
        if changed {
            record("battery_legacy slot=\(slot) pct=\(pct) charging=\(charging)")
            onCapabilitiesChanged?()
        }
    }

    private func refreshBattery1001(_ slot: UInt8, featureIndex: UInt8) async {
        // getBatteryInfo = function 0; payload[0...1]=millivolts, payload[2]=flags
        guard let reply = await request(slot, featureIndex, 0, []),
              let (pct, charging) = LogitechHIDProtocol.voltageBattery(reply) else { return }
        let changed = mice[slot]?.batteryPercent != pct || mice[slot]?.batteryCharging != charging
        mice[slot]?.batteryPercent = pct
        mice[slot]?.batteryCharging = charging
        if changed {
            record("battery_voltage slot=\(slot) pct=\(pct) charging=\(charging)")
            onCapabilitiesChanged?()
        }
    }

    private func probeSmartShift(_ slot: UInt8) async {
        guard !closed, !Task.isCancelled else { return }
        guard let reply = await request(slot, 0, 0, [0x21, 0x10]),
              let idx = reply.first, idx != 0 else { return }
        mice[slot]?.smartshiftFeature = idx
        // getRatchetControlMode = function 0; payload[0] = WheelMode (1=freespin, 2=ratchet)
        guard let mode = await request(slot, idx, 0, []),
              let byte = mode.first else { return }
        let freespin = byte == 1
        mice[slot]?.wheelFreespin = freespin
        record("smartshift slot=\(slot) freespin=\(freespin)")
        onCapabilitiesChanged?()
    }

    // MARK: - Public read / write APIs

    /// Re-reads battery for all slots that have a battery feature. Called by manager on a timer.
    func refreshBatteries() async {
        for slot in mice.keys.sorted() {
            guard !closed, !Task.isCancelled else { return }
            await refreshBattery(slot)
        }
    }

    /// Toggle wheel between free-spin (true) and ratchet (false) for the given slot.
    @discardableResult
    func setWheelMode(slot: UInt8, freespin: Bool) async -> Bool {
        guard let mouse = mice[slot], let feat = mouse.smartshiftFeature, !closed else { return false }
        // setRatchetControlMode = function 1; byte 0 = WheelMode (1=freespin, 2=ratchet), bytes 1-2 = 0 (keep defaults)
        let modeValue: UInt8 = freespin ? 1 : 2
        guard let reply = await request(slot, feat, 1, [modeValue, 0, 0]),
              let applied = reply.first else { return false }
        let confirmed = applied == modeValue
        if confirmed {
            mice[slot]?.wheelFreespin = freespin
            onCapabilitiesChanged?()
            record("wheel_mode slot=\(slot) freespin=\(freespin)")
        }
        return confirmed
    }

    private func identify(_ slot: UInt8, name: String, supported: Bool) {
        let identity = Identity(name: name, supported: supported)
        guard identities[slot] != identity else { return }
        identities[slot] = identity
        onIdentitiesChanged?()
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
                // HID++ has no owner field. A divert left by a previous DeskFuse run
                // (crash, kill, reinstall) survives until the mouse reconnects, and
                // diverted notifications reach every host client anyway: adopt it.
                if wants && flags & 1 != 0 {
                    if mice[slot]?.owned.insert(cid).inserted == true {
                        record("diversion slot=\(slot) cid=\(cid) adopted")
                        onStatus?("\(current.name)：\(MouseSettings.buttonName(cid))已接管")
                    }
                    continue
                }
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
        identities.removeAll()
        onIdentitiesChanged?()
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
