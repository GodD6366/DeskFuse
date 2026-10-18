import AppKit
@preconcurrency import ApplicationServices
import DeskFuseVirtualDisplay

/// Geometry in Quartz's global, top-left coordinate space (also used by AX).
enum RemoteDesktopGeometry {
    static func quartzFrame(_ frame: CGRect, primaryHeight: CGFloat) -> CGRect {
        CGRect(x: frame.minX, y: primaryHeight - frame.maxY, width: frame.width, height: frame.height)
    }

    static func fit(_ frame: CGRect, inside bounds: CGRect) -> CGRect {
        guard frame.width > 0, frame.height > 0, bounds.width > 0, bounds.height > 0 else { return frame }
        // AX window frames are logical points. Clamp with one scale factor so
        // restoring or moving a window never changes its aspect ratio.
        let scale = min(1, bounds.width / frame.width, bounds.height / frame.height)
        let size = CGSize(width: frame.width * scale, height: frame.height * scale)
        return CGRect(x: min(max(frame.minX, bounds.minX), max(bounds.minX, bounds.maxX - size.width)),
                      y: min(max(frame.minY, bounds.minY), max(bounds.minY, bounds.maxY - size.height)),
                      width: size.width, height: size.height)
    }

    /// Placement is measured against the display, not its volatile Dock/menu
    /// work area. AX may enforce a minimum window size larger than the screen.
    static func isPlaced(_ frame: CGRect, on display: CGRect) -> Bool {
        guard !frame.isEmpty, !display.isEmpty else { return false }
        let footprint = CGRect(origin: frame.origin,
            size: CGSize(width: min(frame.width, display.width), height: min(frame.height, display.height)))
        return display.insetBy(dx: -4, dy: -4).contains(footprint)
    }

    static func contains(_ frame: CGRect, in bounds: CGRect) -> Bool {
        !frame.isEmpty && bounds.insetBy(dx: -2, dy: -2).contains(frame)
    }
}

struct RemoteDesktopWindowPolicy {
    static func shouldManage(role: String?, movable: Bool, desktopSurface: Bool) -> Bool {
        role == kAXWindowRole && movable && !desktopSurface
    }
}

struct RemoteDesktopReservations {
    private var owners: [pid_t: UUID] = [:]
    mutating func reserve(processID: pid_t, owner: UUID, limit: Int) throws {
        guard owners[processID] == nil else {
            throw RemoteApplicationError.unsupported("该应用已被另一个远程会话使用")
        }
        guard owners.count < limit else {
            throw RemoteApplicationError.unsupported("最多同时打开 4 个远程应用，请先关闭一个远程窗口")
        }
        owners[processID] = owner
    }
    mutating func release(processID: pid_t, owner: UUID) {
        if owners[processID] == owner { owners.removeValue(forKey: processID) }
    }
}

/// One process lifetime per display. Paused media retains this allocation;
/// closing a session restores windows before releasing the native display.
@MainActor public final class RemoteApplicationDesktop {
    public static let maximumDisplays = 4
    private static var reservations = RemoteDesktopReservations()
    private let owner = UUID()
    private let process: NSRunningApplication
    private let application: AXUIElement
    private var native: DFRemoteVirtualDisplay?
    private var arranged = false
    private var displayReady = false
    private struct Window {
        let element: AXUIElement
        let original: CGRect
        let minimized: Bool
        var attempts = 0
    }
    private var windows: [Window] = []
    public private(set) var presentationDisplaySize: CGSize?
    public var displayID: CGDirectDisplayID? { native?.displayID }
    /// Last preparation stage, including failures before capture enumeration.
    public private(set) var preparationStage = "created"
    public static func hasMovableWindow(processID: pid_t) -> Bool {
        let application = AXUIElementCreateApplication(processID)
        AXUIElementSetMessagingTimeout(application, 0.2)
        let elements = attribute(application, kAXWindowsAttribute) as? [AXUIElement] ?? []
        let desktopFrames = desktopFrames(processID: processID)
        return elements.contains { isManagedWindow($0, desktopFrames: desktopFrames) }
    }

    public init(process: NSRunningApplication) throws {
        guard AXIsProcessTrusted() else {
            throw RemoteApplicationError.permission("B 端需要为 DeskFuse 开启辅助功能权限，以安置远程应用窗口")
        }
        guard !process.isTerminated else { throw RemoteApplicationError.noWindow }
        self.process = process
        application = AXUIElementCreateApplication(process.processIdentifier)
        AXUIElementSetMessagingTimeout(application, 0.2)
        try Self.reservations.reserve(processID: process.processIdentifier, owner: owner, limit: Self.maximumDisplays)
        do { native = try DFRemoteVirtualDisplay(serial: UInt32.random(in: 1...UInt32.max)) }
        catch {
            Self.reservations.release(processID: process.processIdentifier, owner: owner)
            throw error
        }
    }

    /// Called on the host monitor, independently of input focus. New document
    /// windows and dialogs are moved too; an active drag is never repositioned.
    public func prepareWindows(gestureActive: Bool = false) throws {
        preparationStage = "process"
        guard !process.isTerminated else { throw RemoteApplicationError.noWindow }
        preparationStage = "display_online"
        guard let displayID, CGDisplayIsOnline(displayID) != 0 else {
            if !displayReady { throw RemoteApplicationError.noWindow }
            throw RemoteApplicationError.unsupported("远程应用专用屏幕已断开，请重新连接应用")
        }
        if !arranged {
            try arrange(displayID)
            arranged = true
        }
        preparationStage = "display_screen"
        guard let screen = NSScreen.screens.first(where: { Self.id($0) == displayID }) else {
            throw RemoteApplicationError.noWindow
        }
        guard CGDisplayIsActive(displayID) != 0 else {
            if !displayReady { throw RemoteApplicationError.noWindow }
            throw RemoteApplicationError.unsupported("远程应用专用屏幕已停止显示，请重新连接应用")
        }
        guard displayID != CGMainDisplayID(), CGDisplayMirrorsDisplay(displayID) == kCGNullDirectDisplay else {
            throw RemoteApplicationError.unsupported("远程应用专用屏幕不可用或处于镜像模式")
        }
        displayReady = true
        let bounds = Self.quartzFrame(screen, visible: true)
        let displayBounds = Self.quartzFrame(screen)
        preparationStage = "display_bounds"
        guard !bounds.isEmpty else { throw RemoteApplicationError.noWindow }
        guard !gestureActive else { return }
        let desktopFrames = Self.desktopFrames(processID: process.processIdentifier)
        let elements = (Self.attribute(application, kAXWindowsAttribute) as? [AXUIElement] ?? [])
            .filter { Self.isManagedWindow($0, desktopFrames: desktopFrames) }
        preparationStage = "ax_managed_windows"
        guard !elements.isEmpty else { throw RemoteApplicationError.noWindow }
        preparationStage = "ax_placement"
        for element in elements {
            guard let frame = Self.frame(element), !frame.isEmpty else { continue }
            if (Self.attribute(element, "AXFullScreen") as? Bool) == true {
                throw RemoteApplicationError.unsupported("请先退出 B 端应用的 macOS 全屏模式，再连接远程应用")
            }
            let index: Int
            if let existing = windows.firstIndex(where: { CFEqual($0.element, element) }) { index = existing }
            else {
                let minimized = (Self.attribute(element, kAXMinimizedAttribute) as? Bool) == true
                windows.append(Window(element: element, original: frame, minimized: minimized))
                index = windows.count - 1
                if minimized { AXUIElementSetAttributeValue(element, kAXMinimizedAttribute as CFString, kCFBooleanFalse) }
                // Preserve the window's proportion of its original screen;
                // moving into a dedicated screen must not maximize the app.
                let sourceScreen = NSScreen.screens.map { Self.quartzFrame($0) }
                    .max { a, b in
                        func area(_ rect: CGRect) -> CGFloat {
                            let intersection = frame.intersection(rect)
                            return intersection.isNull ? 0 : intersection.width * intersection.height
                        }
                        return area(a) < area(b)
                    } ?? displayBounds
                let preferred = RemotePresentationGeometry.proportionalSize(frame.size,
                    sourceScreen: sourceScreen.size, destinationScreen: displayBounds.size)
                let fit = min(1, bounds.width / preferred.width, bounds.height / preferred.height)
                let size = CGSize(width: preferred.width * fit, height: preferred.height * fit)
                if index == 0 {
                    let scale = size.width / frame.width
                    presentationDisplaySize = CGSize(width: sourceScreen.width * scale, height: sourceScreen.height * scale)
                }
                let target = CGRect(x: bounds.midX - size.width / 2,
                    y: bounds.midY - size.height / 2, width: size.width, height: size.height)
                try move(element, to: target)
                continue
            }
            if !RemoteDesktopGeometry.isPlaced(frame, on: displayBounds) {
                windows[index].attempts += 1
                guard windows[index].attempts <= 4 else {
                    throw RemoteApplicationError.unsupported("应用窗口无法安置到专用屏幕，请退出全屏或关闭不可移动的窗口后重试")
                }
                try move(element, to: RemoteDesktopGeometry.fit(frame, inside: bounds))
            } else { windows[index].attempts = 0 }
        }
    }

    /// Called only after the native release barrier. Accessibility activation
    /// also handles a background menu-bar host whose AppKit activation request
    /// is not honored while the previous remote application is frontmost.
    public func activateWindow() {
        guard !process.isTerminated else { return }
        _ = process.activate(options: [.activateAllWindows])
        if Self.isSettable(application, kAXFrontmostAttribute) {
            AXUIElementSetAttributeValue(application, kAXFrontmostAttribute as CFString, kCFBooleanTrue)
        }
        let main = Self.attribute(application, kAXMainWindowAttribute)
        let element = windows.first { window in
            main.map { CFEqual($0, window.element) } == true
        }?.element ?? windows.first(where: { Self.frame($0.element) != nil })?.element
        guard let element else { return }
        if Self.isSettable(element, kAXMainAttribute) {
            AXUIElementSetAttributeValue(element, kAXMainAttribute as CFString, kCFBooleanTrue)
        }
        AXUIElementPerformAction(element, kAXRaiseAction as CFString)
        if Self.isSettable(application, kAXFocusedWindowAttribute) {
            AXUIElementSetAttributeValue(application, kAXFocusedWindowAttribute as CFString, element)
        }
    }

    @discardableResult public func close() -> Int {
        guard native != nil else { return 0 }
        let fallback = NSScreen.screens.filter { Self.id($0) != displayID }
            .filter { CGDisplayVendorNumber(Self.id($0)) != 0x4453 || CGDisplayModelNumber(Self.id($0)) != 0x5241 }
            .map { Self.quartzFrame($0, visible: true) }
        var failures = 0
        if !process.isTerminated {
            for window in windows {
                // AX elements preserve identity across window-ID reuse. Closed
                // windows are ignored rather than applying old geometry to new ones.
                guard Self.frame(window.element) != nil else { continue }
                var target = window.original
                if !fallback.contains(where: { RemoteDesktopGeometry.contains(target, in: $0) }), let first = fallback.first {
                    target = RemoteDesktopGeometry.fit(target, inside: first)
                }
                do { try move(window.element, to: target) } catch { failures += 1 }
                if window.minimized {
                    AXUIElementSetAttributeValue(window.element, kAXMinimizedAttribute as CFString, kCFBooleanTrue)
                }
            }
        }
        windows.removeAll()
        // ARC releases CGVirtualDisplay only after the window restore attempts.
        native = nil
        Self.reservations.release(processID: process.processIdentifier, owner: owner)
        return failures
    }

    private static func id(_ screen: NSScreen) -> CGDirectDisplayID {
        (screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber)?.uint32Value ?? 0
    }

    /// Convert AppKit's bottom-left logical-point coordinates to the Quartz
    /// top-left logical-point coordinates used by AXUIElement window frames.
    /// Keep the work-area calculation in the same logical coordinate space as
    /// the accessibility window frames, including when the Dock changes it.
    private static func quartzFrame(_ screen: NSScreen, visible: Bool = false) -> CGRect {
        let primaryHeight = NSScreen.screens.first(where: { id($0) == CGMainDisplayID() })?.frame.maxY
            ?? screen.frame.maxY
        return RemoteDesktopGeometry.quartzFrame(visible ? screen.visibleFrame : screen.frame,
            primaryHeight: primaryHeight)
    }

    private func arrange(_ display: CGDirectDisplayID) throws {
        var count: UInt32 = 0
        guard CGGetActiveDisplayList(0, nil, &count) == .success else { throw RemoteApplicationError.noWindow }
        var displays = [CGDirectDisplayID](repeating: 0, count: Int(count))
        guard CGGetActiveDisplayList(count, &displays, &count) == .success,
              displays.prefix(Int(count)).contains(display) else { throw RemoteApplicationError.noWindow }
        let right = displays.prefix(Int(count)).filter { $0 != display }.map { CGDisplayBounds($0).maxX }.max() ?? 0
        var configuration: CGDisplayConfigRef?
        guard CGBeginDisplayConfiguration(&configuration) == .success, let configuration else {
            throw RemoteApplicationError.unsupported("无法配置远程应用专用屏幕")
        }
        guard CGConfigureDisplayMirrorOfDisplay(configuration, display, kCGNullDirectDisplay) == .success,
              CGConfigureDisplayOrigin(configuration, display, Int32(right.rounded(.up)), 0) == .success else {
            CGCancelDisplayConfiguration(configuration)
            throw RemoteApplicationError.unsupported("无法安置远程应用专用屏幕")
        }
        guard CGCompleteDisplayConfiguration(configuration, .forSession) == .success else {
            throw RemoteApplicationError.unsupported("无法应用远程应用专用屏幕配置")
        }
    }

    private static func isSettable(_ element: AXUIElement, _ name: String) -> Bool {
        var value = DarwinBoolean(false)
        return AXUIElementIsAttributeSettable(element, name as CFString, &value) == .success && value.boolValue
    }

    private static func desktopFrames(processID: pid_t) -> [CGRect] {
        guard let info = CGWindowListCopyWindowInfo(.optionAll, kCGNullWindowID) as? [[String: Any]] else { return [] }
        return info.compactMap { window in
            guard (window[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value == processID,
                  let layer = window[kCGWindowLayer as String] as? Int, layer < 0,
                  let rect = window[kCGWindowBounds as String] as? NSDictionary else { return nil }
            return CGRect(dictionaryRepresentation: rect)
        }
    }

    private static func isManagedWindow(_ element: AXUIElement, desktopFrames: [CGRect]) -> Bool {
        let desktop = frame(element).map { frame in
            desktopFrames.contains { abs($0.minX - frame.minX) < 2 && abs($0.minY - frame.minY) < 2
                && abs($0.width - frame.width) < 2 && abs($0.height - frame.height) < 2 }
        } ?? false
        return RemoteDesktopWindowPolicy.shouldManage(role: attribute(element, kAXRoleAttribute) as? String,
            movable: isSettable(element, kAXPositionAttribute), desktopSurface: desktop)
    }

    private static func attribute(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success else { return nil }
        return value
    }

    private static func frame(_ element: AXUIElement) -> CGRect? {
        guard let position = attribute(element, kAXPositionAttribute), CFGetTypeID(position) == AXValueGetTypeID(),
              let size = attribute(element, kAXSizeAttribute), CFGetTypeID(size) == AXValueGetTypeID() else { return nil }
        var point = CGPoint.zero, dimensions = CGSize.zero
        guard AXValueGetValue(unsafeDowncast(position, to: AXValue.self), .cgPoint, &point),
              AXValueGetValue(unsafeDowncast(size, to: AXValue.self), .cgSize, &dimensions) else { return nil }
        return CGRect(origin: point, size: dimensions)
    }

    private func move(_ element: AXUIElement, to rect: CGRect) throws {
        var point = rect.origin, size = rect.size
        guard let position = AXValueCreate(.cgPoint, &point), let dimensions = AXValueCreate(.cgSize, &size) else {
            throw RemoteApplicationError.invalidGeometry
        }
        // Move first: some apps constrain sizes to the current screen. Not all
        // dialogs are resizable, but every captured window must be movable.
        guard AXUIElementSetAttributeValue(element, kAXPositionAttribute as CFString, position) == .success else {
            throw RemoteApplicationError.unsupported("应用不支持移动窗口到专用屏幕")
        }
        AXUIElementSetAttributeValue(element, kAXSizeAttribute as CFString, dimensions)
        AXUIElementSetAttributeValue(element, kAXPositionAttribute as CFString, position)
    }
}
