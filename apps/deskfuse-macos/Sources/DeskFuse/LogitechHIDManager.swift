import Foundation
import IOKit.hid

/// Owns only Logitech vendor-report interfaces, never seizes the normal mouse interface.
@MainActor
final class LogitechHIDManager {
    private var manager: IOHIDManager?
    private var stopping = false
    private var inventoryTimeout: Task<Void, Never>?
    private var devices: [ObjectIdentifier: LogitechHIDDevice] = [:]
    private var tasks: [ObjectIdentifier: Task<Void, Never>] = [:]
    private var wakes: [ObjectIdentifier: AsyncStream<Void>.Continuation] = [:]
    private var refreshTask: Task<Void, Never>?
    private var ignoredInterfaces = 0
    private var openFailures = 0
    private var openResult: IOReturn?
    private var inventory: [ObjectIdentifier: [String: Any]] = [:]
    private var lastIdentities: Set<String> = []
    var desired: Set<Int> = [] {
        didSet { if desired != oldValue { wakes.values.forEach { $0.yield(()) } } }
    }
    var onButton: ((String, Int, Bool) -> Void)?
    var onStatus: ((String) -> Void)?
    var onControlsChanged: ((Set<Int>) -> Void)?
    var onIdentitiesChanged: (([String]) -> Void)?
    private func publishControls() {
        onControlsChanged?(Set(devices.values.flatMap { $0.mice.values.flatMap { $0.available } }))
    }
    private func publishIdentities() {
        let names = Set(devices.values.flatMap { $0.identities.values.map(\.name) })
        guard names != lastIdentities else { return }
        lastIdentities = names
        onIdentitiesChanged?(names.sorted())
    }

    // Only the explicit capture/permission action may ask macOS for a grant.
    // A failed open is retried on the next capture, not in a hot polling loop.
    func prepareCapture() {
        // Explicit capture retries immediately, even if offline receiver slots backed off.
        devices.values.forEach { $0.retryDiscovery() }
        wakes.values.forEach { $0.yield(()) }
        if IOHIDCheckAccess(kIOHIDRequestTypeListenEvent) == kIOHIDAccessTypeUnknown {
            _ = IOHIDRequestAccess(kIOHIDRequestTypeListenEvent)
        }
        if openFailures > 0 || (openResult != nil && openResult != kIOReturnSuccess) { stop() }
    }
    func start() {
        guard manager == nil, !stopping else { return }
        ignoredInterfaces = 0; openFailures = 0; inventory.removeAll()
        let manager = IOHIDManagerCreate(kCFAllocatorDefault, IOOptionBits(kIOHIDOptionsTypeNone))
        self.manager = manager
        IOHIDManagerSetDeviceMatching(manager, [kIOHIDVendorIDKey: 0x046d] as CFDictionary)
        IOHIDManagerRegisterDeviceMatchingCallback(manager, { context, _, _, device in
            guard let context else { return }
            MainActor.assumeIsolated {
                Unmanaged<LogitechHIDManager>.fromOpaque(context).takeUnretainedValue().add(device)
            }
        }, Unmanaged.passUnretained(self).toOpaque())
        IOHIDManagerRegisterDeviceRemovalCallback(manager, { context, _, _, device in
            guard let context else { return }
            MainActor.assumeIsolated {
                Unmanaged<LogitechHIDManager>.fromOpaque(context).takeUnretainedValue().remove(device)
            }
        }, Unmanaged.passUnretained(self).toOpaque())
        IOHIDManagerScheduleWithRunLoop(manager, CFRunLoopGetMain(), CFRunLoopMode.commonModes.rawValue)
        let result = IOHIDManagerOpen(manager, 0)
        openResult = result
        onStatus?(result == kIOReturnSuccess
            ? "正在查找 MX Master 3 / 3S、G502 LIGHTSPEED 的 HID++ 接口…"
            : "无法访问 HID 设备（\(result)），请检查输入监控权限并重启应用")
        if result == kIOReturnSuccess {
            refreshTask = Task { [weak self] in
                while !Task.isCancelled {
                    do { try await Task.sleep(for: .seconds(30)) } catch { return }
                    guard let self else { return }
                    if !self.desired.isEmpty { self.wakes.values.forEach { $0.yield(()) } }
                }
            }
            inventoryTimeout = Task { [weak self] in
                do { try await Task.sleep(for: .seconds(2)) } catch { return }
                guard let self, self.devices.isEmpty else { return }
                self.onStatus?(IOHIDCheckAccess(kIOHIDRequestTypeListenEvent) == kIOHIDAccessTypeGranted
                    ? "未发现可用的 Logitech HID++ 接口，请确认鼠标已连接到此 Mac。"
                    : "HID++ 需要输入监控权限，请在系统设置中授权 DeskFuse 后重新采集。")
            }
        }
    }

    private func add(_ device: IOHIDDevice) {
        let id = ObjectIdentifier(device)
        guard devices[id] == nil else { return }
        let descriptor = IOHIDDeviceGetProperty(device, kIOHIDReportDescriptorKey as CFString) as? Data
        let hasLongReports = descriptor.map { LogitechHIDProtocol.hasLongReports(Array($0)) } ?? false
        if inventory.count < 32 {
            inventory[id] = ["product": IOHIDDeviceGetProperty(device, kIOHIDProductKey as CFString) as? String ?? "",
                "transport": IOHIDDeviceGetProperty(device, kIOHIDTransportKey as CFString) as? String ?? "",
                "product_id": IOHIDDeviceGetProperty(device, kIOHIDProductIDKey as CFString) as? NSNumber ?? 0,
                "usage_page": IOHIDDeviceGetProperty(device, kIOHIDPrimaryUsagePageKey as CFString) as? NSNumber ?? 0,
                "hidpp_long_reports": hasLongReports]
        }
        guard hasLongReports else {
            ignoredInterfaces += 1
            return
        }
        guard let endpoint = LogitechHIDDevice(device) else {
            openFailures += 1
            onStatus?("无法打开 Logitech HID++ 接口，请检查输入监控权限或设备占用")
            return
        }
        devices[id] = endpoint
        endpoint.onButton = { [weak self] slot, cid, down in
            self?.onButton?("\(id)-\(slot)-\(cid)", cid, down)
            // A pending restore must run as soon as the physical key is released.
            if !down, let self, !self.desired.contains(cid) { self.wakes[id]?.yield(()) }
        }
        endpoint.onStatus = { [weak self] in self?.onStatus?($0) }
        endpoint.onControlsChanged = { [weak self] in self?.publishControls() }
        endpoint.onIdentitiesChanged = { [weak self] in self?.publishIdentities() }
        let (events, wake) = AsyncStream<Void>.makeStream(bufferingPolicy: .bufferingNewest(1))
        wakes[id] = wake
        wake.yield(())
        tasks[id] = Task { [weak self, weak endpoint] in
            for await _ in events {
                guard !Task.isCancelled, let self, let endpoint, !endpoint.closed else { return }
                await endpoint.discover()
                guard !Task.isCancelled else { return }
                await endpoint.configure(self.desired)
            }
        }
    }
    func diagnostics() -> [String: Any] {
        ["running": manager != nil, "ignored_interfaces": ignoredInterfaces, "open_failures": openFailures,
         "open_result": openResult as Any? ?? NSNull(),
         "input_monitoring_access": IOHIDCheckAccess(kIOHIDRequestTypeListenEvent).rawValue,
         "inventory": Array(inventory.values), "desired_controls": desired.sorted(),
         "endpoints": devices.values.map { $0.diagnostics() }]
    }

    private func remove(_ device: IOHIDDevice) {
        let id = ObjectIdentifier(device)
        inventory.removeValue(forKey: id)
        wakes.removeValue(forKey: id)?.finish()
        tasks.removeValue(forKey: id)?.cancel()
        devices.removeValue(forKey: id)?.close()
        publishControls()
        publishIdentities()
        if devices.isEmpty { onStatus?("未连接可用的 Logitech HID++ 设备") }
    }
    func stop() {
        guard !stopping else { return }
        stopping = true
        defer { stopping = false }
        inventoryTimeout?.cancel(); inventoryTimeout = nil
        refreshTask?.cancel(); refreshTask = nil
        wakes.values.forEach { $0.finish() }; wakes.removeAll()
        tasks.values.forEach { $0.cancel() }; tasks.removeAll()
        devices.values.forEach { $0.close() }; devices.removeAll()
        publishControls()
        publishIdentities()
        if let manager {
            IOHIDManagerUnscheduleFromRunLoop(manager, CFRunLoopGetMain(), CFRunLoopMode.commonModes.rawValue)
            IOHIDManagerRegisterDeviceMatchingCallback(manager, nil, nil)
            IOHIDManagerRegisterDeviceRemovalCallback(manager, nil, nil)
            IOHIDManagerClose(manager, 0)
        }
        manager = nil
    }
}
