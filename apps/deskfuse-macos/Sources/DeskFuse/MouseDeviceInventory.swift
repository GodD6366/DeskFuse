import Foundation
import IOKit.hid

struct DetectedMouseDevice: Identifiable, Equatable {
    let id: String
    let name: String
    let vendorID: Int
    let productID: Int
    let transport: String

    var model: LogitechHIDProtocol.Model? { LogitechHIDProtocol.model(name) }
    var isSupported: Bool { model != nil }

    var transportLabel: String {
        let value = transport.lowercased()
        if value.contains("bluetooth") { return "蓝牙" }
        if value.contains("usb") { return "USB" }
        return transport.isEmpty ? "未知连接" : transport
    }
}

/// Enumerates every mouse-shaped HID collection, including models DeskFuse
/// cannot remap yet. HID++ identities take precedence over receiver aliases.
@MainActor
final class MouseDeviceInventory {
    private(set) var devices: [DetectedMouseDevice] = []
    private(set) var permissionDenied = false
    var onChange: (([DetectedMouseDevice]) -> Void)?

    private var manager: IOHIDManager?
    private var entries: [ObjectIdentifier: DetectedMouseDevice] = [:]

    func start() {
        guard manager == nil else { return }
        entries.removeAll()
        let manager = IOHIDManagerCreate(kCFAllocatorDefault, IOOptionBits(kIOHIDOptionsTypeNone))
        self.manager = manager
        IOHIDManagerSetDeviceMatching(manager, [
            kIOHIDDeviceUsagePageKey: 0x01,
            kIOHIDDeviceUsageKey: 0x02,
        ] as CFDictionary)
        IOHIDManagerRegisterDeviceMatchingCallback(manager, { context, _, _, device in
            guard let context else { return }
            MainActor.assumeIsolated {
                Unmanaged<MouseDeviceInventory>.fromOpaque(context).takeUnretainedValue().add(device)
            }
        }, Unmanaged.passUnretained(self).toOpaque())
        IOHIDManagerRegisterDeviceRemovalCallback(manager, { context, _, _, device in
            guard let context else { return }
            MainActor.assumeIsolated {
                Unmanaged<MouseDeviceInventory>.fromOpaque(context).takeUnretainedValue().remove(device)
            }
        }, Unmanaged.passUnretained(self).toOpaque())
        IOHIDManagerScheduleWithRunLoop(manager, CFRunLoopGetMain(), CFRunLoopMode.commonModes.rawValue)
        let result = IOHIDManagerOpen(manager, 0)
        permissionDenied = result != kIOReturnSuccess
        if let seeded = IOHIDManagerCopyDevices(manager) as? Set<IOHIDDevice> {
            seeded.forEach(add)
        }
        publish()
    }

    func stop() {
        guard let manager else { return }
        IOHIDManagerUnscheduleFromRunLoop(manager, CFRunLoopGetMain(), CFRunLoopMode.commonModes.rawValue)
        IOHIDManagerRegisterDeviceMatchingCallback(manager, nil, nil)
        IOHIDManagerRegisterDeviceRemovalCallback(manager, nil, nil)
        IOHIDManagerClose(manager, 0)
        self.manager = nil
        entries.removeAll()
        permissionDenied = false
        if !devices.isEmpty {
            devices = []
            onChange?([])
        }
    }

    private func add(_ device: IOHIDDevice) {
        let key = ObjectIdentifier(device)
        guard entries[key] == nil else { return }
        let usagePage = number(device, kIOHIDPrimaryUsagePageKey)
        let usage = number(device, kIOHIDPrimaryUsageKey)
        // Composite keyboards can expose an extra mouse collection; only the
        // primary mouse collection describes a device the user thinks of as a mouse.
        guard usagePage == 0 || usagePage == 0x01, usage == 0 || usage == 0x02 else { return }
        let vendorID = number(device, kIOHIDVendorIDKey)
        let productID = number(device, kIOHIDProductIDKey)
        let locationID = number(device, kIOHIDLocationIDKey)
        let serial = string(device, kIOHIDSerialNumberKey)
        let product = string(device, kIOHIDProductKey)
        // Mac 自带键盘与触控板同样暴露鼠标集合；它们不是可以改键的外接鼠标。
        let builtIn = (IOHIDDeviceGetProperty(device, "Built-In" as CFString) as? NSNumber)?.boolValue == true
        guard !Self.isBuiltInPointer(name: product, builtIn: builtIn) else { return }
        let name = product.isEmpty ? "未命名鼠标" : product
        let identity = serial.isEmpty
            ? "\(vendorID):\(productID):\(locationID):\(name)"
            : "\(vendorID):\(productID):\(serial)"
        entries[key] = DetectedMouseDevice(
            id: identity,
            name: name,
            vendorID: vendorID,
            productID: productID,
            transport: string(device, kIOHIDTransportKey))
        publish()
    }

    private func remove(_ device: IOHIDDevice) {
        entries.removeValue(forKey: ObjectIdentifier(device))
        publish()
    }

    private func publish() {
        var seen = Set<String>()
        let next = entries.values
            .filter { seen.insert($0.id).inserted }
            .sorted { lhs, rhs in
                if (lhs.model != nil) != (rhs.model != nil) { return lhs.model != nil }
                return lhs.name.localizedCaseInsensitiveCompare(rhs.name) == .orderedAscending
            }
        guard next != devices else { return }
        devices = next
        onChange?(next)
    }

    /// 内建指针设备（MacBook 的键盘/触控板、带内建标记的 HID 集合）不参与鼠标改键。
    /// 这类设备没有可改键的实体按键，列出来只会显示成一台永远未适配的鼠标。
    nonisolated static func isBuiltInPointer(name: String, builtIn: Bool) -> Bool {
        if builtIn { return true }
        let value = name.lowercased()
        return value.contains("internal keyboard") || value.contains("internal trackpad")
    }

    private func number(_ device: IOHIDDevice, _ key: String) -> Int {
        (IOHIDDeviceGetProperty(device, key as CFString) as? NSNumber)?.intValue ?? 0
    }

    private func string(_ device: IOHIDDevice, _ key: String) -> String {
        IOHIDDeviceGetProperty(device, key as CFString) as? String ?? ""
    }
}
