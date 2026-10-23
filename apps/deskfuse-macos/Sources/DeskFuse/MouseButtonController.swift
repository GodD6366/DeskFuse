import AppKit
import CoreGraphics
import Observation
import OSLog
import DeskFuseEngine

private actor MouseSettingsRepository {
    let engine: Engine
    private var cachedJSON: String?
    private var cachedSettings: MouseSettings?
    init(engine: Engine) { self.engine = engine }
    func read() throws -> MouseSettings {
        let json = try engine.mouseSettingsJson()
        if json == cachedJSON, let cachedSettings { return cachedSettings }
        let settings = try JSONDecoder().decode(MouseSettings.self, from: Data(json.utf8))
        cachedJSON = json
        cachedSettings = settings
        return settings
    }
    func save(_ settings: MouseSettings) throws {
        try engine.saveMouseSettingsJson(json: String(decoding: JSONEncoder().encode(settings), as: UTF8.self))
    }
}

/// Session-tail remapping runs after the sharing capture tap. Raw buttons are
/// transported unchanged while remote; only the current target performs actions.
/// HID++ special controls instead resolve to keyboard actions at the physical source.
@MainActor @Observable
final class MouseButtonController {
    static let shared = MouseButtonController()
    private(set) var settings = MouseSettings()
    private(set) var hasLoadedSettings = false
    var isFeatureEnabled: Bool { settings.enabled }
    func setFeatureEnabled(_ enabled: Bool) {
        guard hasLoadedSettings else { return }
        var updated = settings
        updated.enabled = enabled
        applySettings(updated)
    }
    private(set) var error: String?
    private(set) var lastExecution = "等待实体按键"
    private(set) var saving = false
    private(set) var saveStatus = "已加载保存的配置"
    private(set) var tapReady = false
    private(set) var lastButton: Int?
    private(set) var captureReleased = false
    private(set) var hidStatus = "未启动 HID++ 检测"
    private(set) var availableNativeButtons: Set<Int> = []
    private(set) var detectingG502 = false
    private(set) var detectedMice: [DetectedMouseDevice] = []
    /// Battery readings for connected wireless HID++ mice, keyed by mouse name.
    private(set) var batteryInfo: [LogitechHIDManager.BatteryInfo] = []
    /// SmartShift wheel state for connected HID++ mice.
    private(set) var wheelInfo: [LogitechHIDManager.WheelInfo] = []
    private let hid = LogitechHIDManager()
    private let inventory = MouseDeviceInventory()
    private var hardwareMice: [DetectedMouseDevice] = []
    private var logitechNames: [String] = []
    private var scanning = false
    private var shortcutState = MouseShortcutState()
    private let keyboardSource = CGEventSource(stateID: .privateState)
    private var observedButtons: [Int: Int] = [:]
    private var actionCount = 0
    private var captureCount = 0
    private var recentEvents: [[String: Any]] = []
    private var nativeHeld: [String: MouseAction] = [:]
    private var nativeCaptured: Set<String> = []
    private var activeNativeCapture: String?
    private static let nativeMarker: Int64 = 0x44464849
    private var captureTimeout: Task<Void, Never>?
    var detecting = false
    private var repository: MouseSettingsRepository?
    private var poll: Task<Void, Never>?
    private var saveTask: Task<Void, Never>?
    private var editGeneration = 0
    private var refreshCount = 0
    private var refreshFailures = 0
    private var refreshMaxMilliseconds = 0.0
    private var lastRefreshError: String?
    private var tapInstallFailures = 0
    private static let logger = Logger(subsystem: "cc.deskfuse", category: "mouse_buttons")
    private var tap: CFMachPort?
    private var source: CFRunLoopSource?
    private var held: [Int: MouseAction] = [:]
    private var passthrough: Set<Int> = []
    private var detectedHeld: Set<Int> = []
    private static let marker: Int64 = 0x44464D42
    private static let modifiers: UInt64 = 0x1e0000

    func start(engine: Engine) {
        guard repository == nil else { return }
        repository = MouseSettingsRepository(engine: engine)
        hid.onStatus = { [weak self] value in
            guard let self, self.hidStatus != value else { return }
            self.hidStatus = value
            self.record("hid_status", fields: ["status": value])
        }
        hid.onControlsChanged = { [weak self] value in
            guard let self, self.availableNativeButtons != value else { return }
            self.availableNativeButtons = value
        }
        hid.onButton = { [weak self] id, button, down in self?.nativeButton(id, button: button, down: down) }
        hid.onIdentitiesChanged = { [weak self] names in
            guard let self else { return }
            self.logitechNames = names
            self.rebuildDetectedMice()
        }
        hid.onCapabilitiesChanged = { [weak self] in
            guard let self else { return }
            self.batteryInfo = self.hid.batteryInfo()
            self.wheelInfo = self.hid.wheelInfo()
        }
        inventory.onChange = { [weak self] devices in
            guard let self else { return }
            self.hardwareMice = devices
            self.rebuildDetectedMice()
        }
        startPolling()
    }

    func beginDeviceScan() {
        scanning = true
        inventory.start()
        hid.start()
        configureHID()
        startPolling()
    }

    func endDeviceScan() {
        guard scanning else { return }
        scanning = false
        inventory.stop()
        hardwareMice = []
        rebuildDetectedMice()
        if settings.enabled { configureHID() } else { stop() }
    }

    private func rebuildDetectedMice() {
        var merged: [DetectedMouseDevice] = []
        var seenNames = Set<String>()
        for name in logitechNames where !name.isEmpty {
            let key = name.lowercased()
            guard seenNames.insert(key).inserted else { continue }
            let hardware = hardwareMice.first { $0.name.caseInsensitiveCompare(name) == .orderedSame }
            merged.append(DetectedMouseDevice(
                id: "hidpp:\(key)",
                name: name,
                vendorID: hardware?.vendorID ?? 0x046D,
                productID: hardware?.productID ?? 0,
                transport: hardware?.transport ?? "HID++"))
        }
        for device in hardwareMice {
            guard !seenNames.contains(device.name.lowercased()) else { continue }
            if device.vendorID == 0x046D, !logitechNames.isEmpty, Self.isReceiverAlias(device.name) { continue }
            guard !merged.contains(where: { $0.id == device.id }) else { continue }
            merged.append(device)
        }
        merged.sort {
            if ($0.model != nil) != ($1.model != nil) { return $0.model != nil }
            return $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending
        }
        guard merged != detectedMice else { return }
        detectedMice = merged
    }

    private static func isReceiverAlias(_ name: String) -> Bool {
        let value = name.lowercased()
        return value == "receiver" || value.contains("receiver")
    }
    private func startPolling() {
        guard poll == nil else { return }
        poll = Task { [weak self] in
            while !Task.isCancelled {
                await self?.refresh()
                try? await Task.sleep(for: .seconds(1))
            }
        }
    }
    private func refresh() async {
        guard let repository, saveTask == nil else { return }
        let started = ContinuousClock.now
        refreshCount += 1
        defer {
            let duration = started.duration(to: .now).components
            let milliseconds = Double(duration.seconds) * 1_000 + Double(duration.attoseconds) / 1e15
            refreshMaxMilliseconds = max(refreshMaxMilliseconds, milliseconds)
        }
        let generation = editGeneration
        do {
            let incoming = try await repository.read()
            guard generation == editGeneration, saveTask == nil else { return }
            lastRefreshError = nil
            hasLoadedSettings = true
            let changed = settings != incoming
            if changed {
                settings = incoming
                record("settings_refreshed", fields: ["enabled": incoming.enabled, "bindings": incoming.bindings.count])
            }
            let needsTap = (settings.enabled || detecting) && !tapReady
            if settings.enabled || detecting { installTap() }
            if changed || needsTap { configureHID() }
            if !settings.enabled && !scanning { stop() }
        } catch {
            refreshFailures += 1
            if lastRefreshError != error.localizedDescription {
                lastRefreshError = error.localizedDescription
                self.error = error.localizedDescription
                record("settings_read_failed", fields: ["error": error.localizedDescription])
            }
        }
    }
    func update(_ settings: MouseSettings) {
        var settings = settings
        settings.enabled = isFeatureEnabled
        applySettings(settings)
    }
    private func applySettings(_ settings: MouseSettings) {
        guard let repository else { return }
        let previousSettings = self.settings
        self.settings = settings
        saving = true; saveStatus = "正在保存…"
        editGeneration += 1
        let generation = editGeneration
        error = nil
        if settings.enabled { installTap(); configureHID(); startPolling() }
        else { stop() }
        // Serialize writes. A newer edit never cancels an already submitted save.
        let previous = saveTask
        saveTask = Task { [weak self] in
            await previous?.value
            do {
                try await repository.save(settings)
                self?.record("settings_saved", fields: ["generation": generation, "bindings": settings.bindings.count])
                if self?.editGeneration == generation { self?.saveStatus = "已保存并应用" }
            } catch {
                self?.record("settings_save_failed", fields: ["generation": generation, "error": error.localizedDescription])
                if self?.editGeneration == generation {
                    self?.error = error.localizedDescription
                    self?.saveStatus = "保存失败，请重试"
                    self?.settings = previousSettings
                    if previousSettings.enabled { self?.installTap(); self?.configureHID(); self?.startPolling() }
                    else { self?.stop() }
                }
            }
            if self?.editGeneration == generation { self?.saveTask = nil; self?.saving = false }
        }
    }
    func stop() {
        endDetection()
        if !scanning {
            hid.stop()
            poll?.cancel(); poll = nil
        }
        let location = CGEvent(source: nil)?.location ?? .zero
        for action in held.values { emit(action, down: false, proxy: nil, location: location) }
        held.removeAll()
        for action in nativeHeld.values { emit(action, down: false, proxy: nil, location: location, native: true) }
        nativeHeld.removeAll(); nativeCaptured.removeAll(); activeNativeCapture = nil
        passthrough.removeAll(); detectedHeld.removeAll()
        availableNativeButtons.removeAll()
        if let tap { CGEvent.tapEnable(tap: tap, enable: false); CFMachPortInvalidate(tap) }
        if let source { CFRunLoopRemoveSource(CFRunLoopGetMain(), source, .commonModes) }
        source = nil; tap = nil; tapReady = false
    }
    func clearCapture() {
        endDetection()
        lastButton = nil
        activeNativeCapture = nil
        captureReleased = false
    }

    /// Toggle the SmartShift wheel mode for the mouse with the given name.
    func setWheelMode(name: String, freespin: Bool) {
        Task { [weak self] in
            guard let self else { return }
            let ok = await self.hid.setWheelMode(name: name, freespin: freespin)
            if ok {
                self.wheelInfo = self.hid.wheelInfo()
            }
        }
    }
    func openInputMonitoringSettings() {
        hid.prepareCapture()
        if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent") {
            NSWorkspace.shared.open(url)
        }
    }
    func beginDetection(g502: Bool = false) {
        guard settings.enabled else { return }
        hid.prepareCapture()
        lastButton = nil
        activeNativeCapture = nil
        captureReleased = false
        detecting = true
        detectingG502 = g502
        captureCount += 1
        record("capture_started", fields: ["g502": g502])
        installTap()
        configureHID()
        captureTimeout?.cancel()
        captureTimeout = Task { [weak self] in
            do { try await Task.sleep(for: .seconds(10)) } catch { return }
            self?.endDetection()
        }
    }
    func endDetection() {
        if detecting { record("capture_ended", fields: ["button": lastButton as Any? ?? NSNull()]) }
        detecting = false
        configureHID()
        captureTimeout?.cancel(); captureTimeout = nil
    }

    private func configureHID() {
        if settings.enabled || detecting || scanning { hid.start() }
        hid.desired = detecting ? (detectingG502 ? LogitechHIDProtocol.g502Buttons : LogitechHIDProtocol.mxButtons) : settings.enabled && tapReady
            ? Set(settings.bindings.map(\.button)).intersection(LogitechHIDProtocol.buttons) : []
        if !settings.enabled && !detecting && !scanning && nativeCaptured.isEmpty { hid.stop() }
    }
    private func nativeButton(_ id: String, button: Int, down: Bool) {
        guard settings.enabled else { return }
        record("button", fields: ["source": "hidpp", "button": button, "down": down])
        if down {
            observedButtons[button, default: 0] += 1
            lastExecution = "已收到\(MouseSettings.buttonName(button))"
        }
        if nativeCaptured.contains(id) {
            if !down {
                nativeCaptured.remove(id)
                if activeNativeCapture == id { captureReleased = true }
                configureHID()
            }
            return
        }
        if down && detecting && (LogitechHIDProtocol.g502Buttons.contains(button) == detectingG502) {
            nativeCaptured.insert(id)
            activeNativeCapture = id
            lastButton = button
            endDetection()
            return
        }
        let location = CGEvent(source: nil)?.location ?? .zero
        if down {
            let flags = CGEventSource.flagsState(.combinedSessionState).rawValue & Self.modifiers
            guard nativeHeld[id] == nil, let configured = settings.action(button: button, modifiers: flags),
                  let action = resolve(configured) else { return }
            nativeHeld[id] = action
            emit(action, down: true, proxy: nil, location: location, native: true)
        } else if let action = nativeHeld.removeValue(forKey: id) {
            emit(action, down: false, proxy: nil, location: location, native: true)
        }
    }

    private func installTap() {
        guard tap == nil else { return }
        let types: [CGEventType] = [.leftMouseDown, .leftMouseUp, .rightMouseDown, .rightMouseUp,
                                    .otherMouseDown, .otherMouseUp, .leftMouseDragged, .rightMouseDragged, .otherMouseDragged]
        let mask = types.reduce(CGEventMask(0)) { $0 | (CGEventMask(1) << $1.rawValue) }
        guard let tap = CGEvent.tapCreate(tap: .cgSessionEventTap, place: .tailAppendEventTap,
                                         options: .defaultTap, eventsOfInterest: mask, callback: { proxy, type, event, info in
            guard let info else { return Unmanaged.passUnretained(event) }
            let controller = Unmanaged<MouseButtonController>.fromOpaque(info).takeUnretainedValue()
            let pass = MainActor.assumeIsolated { controller.handle(proxy: proxy, type: type, event: event) != nil }
            return pass ? Unmanaged.passUnretained(event) : nil
        }, userInfo: Unmanaged.passUnretained(self).toOpaque()) else {
            tapReady = false
            tapInstallFailures += 1
            error = "无法监听鼠标按键，请在系统设置 → 隐私与安全性 → 辅助功能中允许 DeskFuse。"
            if tapInstallFailures == 1 || tapInstallFailures.isMultiple(of: 60) {
                record("tap_failed", fields: ["attempts": tapInstallFailures])
            }
            return
        }
        self.tap = tap
        source = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, tap, 0)
        CFRunLoopAddSource(CFRunLoopGetMain(), source, .commonModes)
        CGEvent.tapEnable(tap: tap, enable: true)
        tapReady = true
        record("tap_installed")
    }
    private func handle(proxy: CGEventTapProxy, type: CGEventType, event: CGEvent) -> Unmanaged<CGEvent>? {
        if type == .tapDisabledByTimeout || type == .tapDisabledByUserInput {
            record("tap_disabled", fields: ["type": type.rawValue])
            // Release generated keys/buttons, but swallow the eventual physical up.
            for (button, action) in held {
                emit(action, down: false, proxy: proxy, location: event.location)
                detectedHeld.insert(button)
            }
            held.removeAll()
            if let tap { CGEvent.tapEnable(tap: tap, enable: true) }
            return Unmanaged.passUnretained(event)
        }
        if [Self.marker, Self.nativeMarker].contains(event.getIntegerValueField(.eventSourceUserData)) { return Unmanaged.passUnretained(event) }
        let button = Int(event.getIntegerValueField(.mouseEventButtonNumber))
        guard (0...31).contains(button) else { return Unmanaged.passUnretained(event) }
        let down = [.leftMouseDown, .rightMouseDown, .otherMouseDown].contains(type)
        let up = [.leftMouseUp, .rightMouseUp, .otherMouseUp].contains(type)
        if down || up {
            record("button", fields: ["source": "quartz", "button": button, "down": down,
                "modifiers": event.flags.rawValue & Self.modifiers, "detecting": detecting])
        }
        if detectedHeld.contains(button) {
            if up {
                detectedHeld.remove(button)
                if lastButton == button { captureReleased = true }
            }
            return nil
        }
        if down { observedButtons[button, default: 0] += 1 }
        if detecting && down && detectingG502 {
            // Spy capture observes the physical key independently of its current firmware action.
            passthrough.insert(button)
            return Unmanaged.passUnretained(event)
        }
        if detecting && down {
            lastButton = button
            endDetection()
            detectedHeld.insert(button)
            return nil
        }
        if down {
            let modifiers = event.flags.rawValue & Self.modifiers
            if let configured = settings.action(button: button, modifiers: modifiers), let action = resolve(configured) {
                held[button] = action
                if action.kind == "mouse", let target = action.button {
                    return rewriteMouse(event, target: target, down: true)
                }
                emit(action, down: true, proxy: proxy, location: event.location)
                return nil
            }
            passthrough.insert(button)
        }
        if passthrough.contains(button) {
            if up { passthrough.remove(button) }
            return Unmanaged.passUnretained(event)
        }
        if let action = held[button] {
            if up {
                held.removeValue(forKey: button)
                if action.kind == "mouse", let target = action.button {
                    return rewriteMouse(event, target: target, down: false)
                }
                emit(action, down: false, proxy: proxy, location: event.location)
            } else if action.kind == "mouse", let target = action.button {
                event.type = target == 0 ? .leftMouseDragged : target == 1 ? .rightMouseDragged : .otherMouseDragged
                event.setIntegerValueField(.mouseEventButtonNumber, value: Int64(target))
                return Unmanaged.passUnretained(event)
            }
            return nil
        }
        return Unmanaged.passUnretained(event)
    }
    private func rewriteMouse(_ event: CGEvent, target: Int, down: Bool) -> Unmanaged<CGEvent> {
        event.type = target == 0 ? (down ? .leftMouseDown : .leftMouseUp)
            : target == 1 ? (down ? .rightMouseDown : .rightMouseUp) : (down ? .otherMouseDown : .otherMouseUp)
        event.setIntegerValueField(.mouseEventButtonNumber, value: Int64(target))
        // Preserve click count, pressure, timestamp and physical modifier flags.
        return Unmanaged.passUnretained(event)
    }
    private func resolve(_ action: MouseAction) -> MouseAction? {
        guard action.kind == "shortcut", let key = action.key_code else { return action }
        do {
            let hotKeys = CFPreferencesCopyAppValue("AppleSymbolicHotKeys" as CFString,
                "com.apple.symbolichotkeys" as CFString) as? [String: Any] ?? [:]
            let shortcut = try MouseShortcut(key: key, modifiers: action.modifiers ?? 0).resolved(in: hotKeys)
            return .shortcut(shortcut.key, shortcut.modifiers)
        } catch {
            self.error = error.localizedDescription
            lastExecution = error.localizedDescription
            record("shortcut_resolution_failed", fields: ["error": error.localizedDescription])
            return nil
        }
    }
    private func record(_ event: String, fields: [String: Any] = [:]) {
        var entry = fields
        entry["event"] = event
        entry["timestamp"] = Date().ISO8601Format()
        recentEvents.append(entry)
        if recentEvents.count > 64 { recentEvents.removeFirst(recentEvents.count - 64) }
        // Keep button details in the bounded snapshot, not in the disk log hot path.
        if event != "button" && event != "action" {
            Self.logger.notice("\(event, privacy: .public) \(String(describing: fields), privacy: .public)")
        }
    }
    func diagnostics() -> Data {
        let entry: [String: Any] = ["subsystem": "mouse_buttons", "event": "native_snapshot", "timestamp": Date().ISO8601Format(),
            "pid": ProcessInfo.processInfo.processIdentifier,
            "bundle_path": Bundle.main.bundlePath,
            "app_build": Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as Any? ?? NSNull(),
            "enabled": settings.enabled, "tap_ready": tapReady, "detecting": detecting,
            "detecting_g502": detectingG502, "native_buttons": availableNativeButtons.sorted(),
            "detected_mice": detectedMice.map {
                ["id": $0.id, "name": $0.name, "vendor_id": $0.vendorID, "product_id": $0.productID,
                 "transport": $0.transport, "supported": $0.isSupported] as [String: Any]
            },
            "inventory_permission_denied": inventory.permissionDenied, "scanning": scanning,
            "tap_enabled": tap.map { CGEvent.tapIsEnabled(tap: $0) } ?? false,
            "settings": (try? JSONSerialization.jsonObject(with: JSONEncoder().encode(settings))) ?? NSNull(),
            "recent_events": recentEvents,
            "settings_refreshes": refreshCount, "settings_refresh_failures": refreshFailures,
            "settings_refresh_max_ms": refreshMaxMilliseconds,
            "tap_install_failures": tapInstallFailures,
            "capture_attempts": captureCount, "button_down_counts": Dictionary(uniqueKeysWithValues: observedButtons.map { (String($0.key), $0.value) }),
            "shortcut_actions": actionCount, "error": error as Any? ?? NSNull(),
            "hid_status": hidStatus, "hid": hid.diagnostics()]
        guard let data = try? JSONSerialization.data(withJSONObject: entry, options: [.sortedKeys]) else { return Data() }
        return data + Data([10])
    }
    private func emit(_ action: MouseAction, down: Bool, proxy: CGEventTapProxy?, location: CGPoint, native: Bool = false) {
        record("action", fields: ["kind": action.kind, "down": down, "native": native,
            "key": action.key_code as Any? ?? NSNull(), "modifiers": action.modifiers as Any? ?? NSNull()])
        let output: CGEvent?
        if action.kind == "shortcut", let key = action.key_code {
            let shortcut = MouseShortcut(key: key, modifiers: action.modifiers ?? 0)
            let physical = CGEventSource.flagsState(.combinedSessionState).rawValue & 0x9e0000
            for event in shortcutState.events(for: shortcut, down: down, physicalFlags: physical) {
                let keyEvent = CGEvent(keyboardEventSource: keyboardSource, virtualKey: event.key, keyDown: event.down)
                keyEvent?.flags = CGEventFlags(rawValue: event.flags)
                keyEvent?.timestamp = CGEventTimestamp(DispatchTime.now().uptimeNanoseconds)
                keyEvent?.setIntegerValueField(.eventSourceUserData, value: native ? Self.nativeMarker : Self.marker)
                keyEvent?.post(tap: .cghidEventTap)
            }
            if down {
                actionCount += 1
                lastExecution = "已发送快捷键 · \(Date().formatted(date: .omitted, time: .standard))"
            }
            return
        } else if action.kind == "mouse", let button = action.button,
                  let mouseButton = CGMouseButton(rawValue: UInt32(button)) {
            let type: CGEventType = button == 0 ? (down ? .leftMouseDown : .leftMouseUp)
                : button == 1 ? (down ? .rightMouseDown : .rightMouseUp) : (down ? .otherMouseDown : .otherMouseUp)
            output = CGEvent(mouseEventSource: nil, mouseType: type, mouseCursorPosition: location, mouseButton: mouseButton)
        } else { return }
        output?.setIntegerValueField(.eventSourceUserData, value: native ? Self.nativeMarker : Self.marker)
        // System shortcuts (Spaces/Mission Control) must enter at HID, before session taps.
        // DFMB is ignored by sharing; DFHI represents a physical HID++ keyboard action.
        output?.post(tap: .cghidEventTap)
    }
}
