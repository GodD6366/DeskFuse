import SwiftUI
import AppKit
import Observation
import DeskFuseEngine
import DeskFuseRemoteApplication

@available(macOS 14.2, *)
@MainActor @Observable final class RemoteApplicationManager {
    static let shared = RemoteApplicationManager()
    private(set) var hosts: [UUID: RemoteApplicationSession] = [:]
    private(set) var viewers: [UUID: RemoteApplicationSession] = [:]
    var error: String?
    @ObservationIgnored private var engine: Engine?
    @ObservationIgnored private var incoming: Task<Void, Never>?
    @ObservationIgnored private var windows: [UUID: RemoteApplicationWindow] = [:]
    @ObservationIgnored private var pickers: [String: RemoteApplicationPickerWindow] = [:]
    @ObservationIgnored private var catalogs: [UUID: RemoteApplicationSession] = [:]
    @ObservationIgnored private var viewerOwner: UUID?
    @ObservationIgnored private var hostOwner: UUID?
    @ObservationIgnored private var focusedViewerID: UUID?
    private init() {}

    func start(engine: Engine) {
        self.engine = engine; incoming?.cancel()
        incoming = Task { [weak self] in
            while !Task.isCancelled {
                guard let self else { return }
                let channel = await Task.detached { () -> RemoteApplicationChannel? in
                    // Entering the authenticated device network enables remote
                    // application hosting automatically. There is no separate
                    // per-device switch in the product surface anymore.
                    try? engine.setRemoteApplicationHosting(enabled: true)
                    return try? engine.takeRemoteApplication()
                }.value
                self.hosts = self.hosts.filter { $0.value.active || $0.value.busy }
                if let channel {
                    // Each application uses its own bounded media stream on the
                    // existing authenticated connection; never replace siblings.
                    if self.hosts.count >= 16 { channel.close() }
                    else {
                        let peerID = channel.peerId()
                        let name = AppStore.shared.snapshot?.devices.first { $0.id == peerID }?.name ?? "已配对设备"
                        let session = RemoteApplicationSession(engine: engine, peerID: peerID, peerName: name, role: .host)
                        self.hosts[session.id] = session
                        await session.start(channel: channel)
                    }
                }
                do { try await Task.sleep(for: .milliseconds(100)) } catch { return }
            }
        }
    }
    func stopHosting() {
        let sessions = Array(hosts.values)
        for session in sessions { session.yieldHostControl() }
        hostOwner = nil
        Task { for session in sessions { await session.stop() } }
    }
    func open(_ device: Device) {
        guard device.trusted, device.connected, !device.blocked else { error = "请先连接已配对设备"; return }
        openPicker(peerID: device.id, peerName: device.name)
    }
    func openPicker(peerID: String, peerName: String) {
        guard let engine else { error = "远程应用引擎尚未就绪"; return }
        if let picker = pickers[peerID] {
            picker.showWindow(nil); picker.window?.deminiaturize(nil); picker.window?.makeKeyAndOrderFront(nil)
            picker.session.refreshApplications(); return
        }
        let session = RemoteApplicationSession(engine: engine, peerID: peerID, peerName: peerName, role: .viewer)
        let picker = RemoteApplicationPickerWindow(session: session)
        pickers[peerID] = picker
        picker.showWindow(nil); picker.window?.makeKeyAndOrderFront(nil)
        Task { await session.start() }
    }
    /// Workspace catalogs own only a browsing connection, never viewer windows.
    func makeCatalog(for device: Device) -> RemoteApplicationSession? {
        guard let engine, device.trusted, device.connected, !device.blocked else { return nil }
        let session = RemoteApplicationSession(engine: engine, peerID: device.id, peerName: device.name, role: .viewer)
        catalogs[session.id] = session
        return session
    }
    func closeCatalog(_ session: RemoteApplicationSession) async {
        catalogs.removeValue(forKey: session.id)
        await session.stop()
    }
    func openedApplications(peerID: String) -> Set<String> {
        Set(viewers.values.filter { $0.peerID == peerID }.compactMap(\.applicationKey))
    }
    func openApplication(_ application: RemoteApplicationInfo, from picker: RemoteApplicationSession) {
        guard let engine else { return }
        if let existing = viewers.values.first(where: { $0.peerID == picker.peerID && $0.applicationKey == application.applicationKey }),
           let window = windows[existing.id] {
            focusedViewerID = existing.id
            RemoteApplicationDockController.shared.focus(id: existing.id)
            window.showWindow(nil); window.window?.deminiaturize(nil); window.window?.makeKeyAndOrderFront(nil)
            if !existing.active && !existing.busy { existing.reconnectApplication() }
            return
        }
        let session = RemoteApplicationSession(engine: engine, peerID: picker.peerID, peerName: picker.peerName, role: .viewer)
        session.prepareApplication(application)
        viewers[session.id] = session
        let controller = RemoteApplicationWindow(session: session)
        windows[session.id] = controller
        focusedViewerID = session.id
        let dockTitle = "\(session.applicationName) — \(session.peerName)"
        RemoteApplicationDockController.shared.present(id: session.id, title: dockTitle, iconPNG: application.iconPNG)
        controller.showWindow(nil); controller.window?.makeKeyAndOrderFront(nil)
        Task { await session.start() }
    }
    func closeViewer(_ session: RemoteApplicationSession) {
        session.releaseControl(resumeSharing: true)
        viewers.removeValue(forKey: session.id); windows.removeValue(forKey: session.id)
        if focusedViewerID == session.id { focusedViewerID = viewers.keys.first }
        RemoteApplicationDockController.shared.close(id: session.id)
        if let focusedViewerID { RemoteApplicationDockController.shared.focus(id: focusedViewerID) }
        else {
            // 所有远程应用窗口都已关闭，恢复鼠标穿梭
            restoreMouseShuttleIfNeeded()
        }
        Task { await session.stop() }
    }

    func viewerDidBecomeKey(_ session: RemoteApplicationSession) {
        guard viewers[session.id] != nil else { return }
        focusedViewerID = session.id
        RemoteApplicationDockController.shared.focus(id: session.id)
    }

    func focusViewer() -> Bool {
        focusViewer(id: focusedViewerID ?? viewers.keys.first)
    }

    func focusViewer(id: UUID?) -> Bool {
        guard let id,
              let controller = windows[id] else { return false }
        focusedViewerID = id
        RemoteApplicationDockController.shared.focus(id: id)
        controller.showWindow(nil)
        controller.window?.deminiaturize(nil)
        controller.window?.makeKeyAndOrderFront(nil)
        return true
    }
    func closePicker(_ session: RemoteApplicationSession) {
        guard pickers[session.peerID]?.session === session else { return }
        pickers.removeValue(forKey: session.peerID)
        Task { await session.stop() }
    }
    func claimViewerControl(_ session: RemoteApplicationSession) throws {
        if let owner = viewerOwner, owner != session.id { viewers[owner]?.releaseControl(resumeSharing: false) }
        try session.engine.remoteApplicationViewerControl(active: true)
        viewerOwner = session.id
    }
    func releaseViewerControl(_ session: RemoteApplicationSession) {
        // Late blur/close/transport callbacks from an old window must not clear
        // the input helper's global pause now owned by the newly focused window.
        guard viewerOwner == session.id else { return }
        viewerOwner = nil
        try? session.engine.remoteApplicationViewerControl(active: false)
        
        // 如果还有其他 viewer 窗口，不需要恢复鼠标穿梭
        // 只有当所有远程应用窗口都失去焦点时，才延迟恢复鼠标穿梭
        if viewers.isEmpty {
            restoreMouseShuttleIfNeeded()
        }
    }
    
    private func restoreMouseShuttleIfNeeded() {
        // 延迟恢复鼠标穿梭，避免在快速切换窗口时误触发
        Task { @MainActor in
            try? await Task.sleep(for: .milliseconds(100))
            // 再次确认没有活跃的 viewer 控制
            if self.viewerOwner == nil, !self.viewers.isEmpty {
                // 尝试恢复鼠标穿梭状态
                try? self.engine?.remoteApplicationViewerControl(active: false)
            }
        }
    }
    func claimHostControl(_ session: RemoteApplicationSession) -> Bool {
        if let owner = hostOwner, owner != session.id, let previous = hosts[owner] {
            guard previous.peerID == session.peerID else { return false }
            previous.yieldHostControl()
        }
        hostOwner = session.id
        return true
    }
    func releaseHostControl(_ session: RemoteApplicationSession) {
        if hostOwner == session.id { hostOwner = nil }
    }
    func stop() {
        incoming?.cancel(); incoming = nil
        try? engine?.setRemoteApplicationHosting(enabled: false)
        let sessions = Array(hosts.values) + Array(viewers.values) + pickers.values.map(\.session) + Array(catalogs.values)
        for session in sessions { session.releaseControl(resumeSharing: true); session.yieldHostControl() }
        focusedViewerID = nil
        RemoteApplicationDockController.shared.reset()
        Task { for session in sessions { await session.stop() } }
    }
}

@available(macOS 14.2, *)
@MainActor final class RemoteApplicationPickerWindow: NSWindowController, NSWindowDelegate, NSToolbarDelegate {
    let session: RemoteApplicationSession
    init(session: RemoteApplicationSession) {
        self.session = session
        let size = NSSize(width: 844, height: 577)
        let window = NSWindow(contentRect: NSRect(origin: .zero, size: size),
            styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView], backing: .buffered, defer: false)
        window.title = "应用程序 — \(session.peerName)"
        window.titleVisibility = .hidden; window.titlebarAppearsTransparent = true
        window.isOpaque = false; window.backgroundColor = .clear
        window.contentMinSize = NSSize(width: 844, height: 420)
        window.isReleasedWhenClosed = false; window.center()
        super.init(window: window); window.delegate = self
        // A bare titlebar uses compact edge spacing. Let AppKit supply the
        // standard unified-toolbar height and traffic-light insets instead of
        // moving the standard buttons inside private titlebar views.
        let toolbar = NSToolbar(identifier: "deskfuse-remote-application-picker")
        toolbar.delegate = self
        toolbar.displayMode = .iconOnly
        toolbar.allowsUserCustomization = false
        toolbar.showsBaselineSeparator = false
        window.toolbar = toolbar
        window.toolbarStyle = .unified
        window.titlebarSeparatorStyle = .none
        let content = NSHostingController(rootView: RemoteApplicationPicker(session: session) { application in
            RemoteApplicationManager.shared.openApplication(application, from: session)
        })
        // The launcher's default size belongs to the window, not SwiftUI's
        // transient loading/catalog ideal size. Set it after attaching the host.
        content.sizingOptions = []
        window.contentViewController = content
        window.setContentSize(size)
        window.center()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }
    func toolbarDefaultItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] { [.flexibleSpace] }
    func toolbarAllowedItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] { [.flexibleSpace] }
    func windowWillClose(_ notification: Notification) { RemoteApplicationManager.shared.closePicker(session) }
}

@available(macOS 14.2, *)
@MainActor private final class RemoteCanvasWindow: NSWindow {
    enum LocalAction { case menu, close, minimize, zoom, release }
    var localAction: ((LocalAction) -> Void)?
    var pointerMoved: ((NSPoint) -> Void)?
    var localMenuClick: ((NSPoint) -> Bool)?
    private var consumedMenuMouseDown = false
    private var consumedKeys = Set<UInt16>()
    private var lastShortcutTimestamp: TimeInterval?
    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { true }
    private func handleLocalShortcut(_ event: NSEvent) -> Bool {
        guard event.type == .keyDown else { return false }
        let flags = event.modifierFlags.intersection([.command, .control, .option, .shift])
        let action: LocalAction
        if event.keyCode == 53 && flags == [.control, .option] { action = .release }
        else {
            guard flags == [.control, .option, .command] else { return false }
            switch event.keyCode {
            case 49: action = .menu
            case 13: action = .close
            case 46: action = .minimize
            case 3: action = .zoom
            default: return false
            }
        }
        // Intercept before the remote canvas handles Command equivalents, and
        // consume the matching key-up. Repeats must not toggle windows/menus.
        consumedKeys.insert(event.keyCode)
        // Minimizing or opening a popover can deliver key-up to another window.
        // A new physical press must still work; only deduplicate the same event
        // passing through both AppKit entry points.
        if !event.isARepeat && lastShortcutTimestamp != event.timestamp {
            lastShortcutTimestamp = event.timestamp
            localAction?(action)
        }
        return true
    }
    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        handleLocalShortcut(event) || super.performKeyEquivalent(with: event)
    }
    override func sendEvent(_ event: NSEvent) {
        // Resolve the native button's actual bounds before the Metal canvas
        // can claim the click. Consume the matching up/drag as one local gesture.
        if event.type == .leftMouseDown {
            consumedMenuMouseDown = false
            if localMenuClick?(event.locationInWindow) == true {
                consumedMenuMouseDown = true
                return
            }
        }
        if consumedMenuMouseDown {
            if event.type == .leftMouseUp { consumedMenuMouseDown = false; return }
            if event.type == .leftMouseDragged { return }
        }
        if event.type == .mouseMoved {
            pointerMoved?(event.locationInWindow)
        }
        if event.type == .keyUp, consumedKeys.remove(event.keyCode) != nil { return }
        if handleLocalShortcut(event) { return }
        super.sendEvent(event)
    }
}

@available(macOS 14.2, *)
@MainActor final class RemoteApplicationWindow: NSWindowController, NSWindowDelegate, NSToolbarDelegate {
    let session: RemoteApplicationSession
    private var canvasAspect: CGFloat?
    private var lastContentSize: CGSize?
    private var fittedInitially = false
    private let chrome = RemoteWindowChrome()
    private var frameBeforeZoom: NSRect?
    private static let menuItem = NSToolbarItem.Identifier("deskfuse-remote-application-menu")
    private static let metricsItem = NSToolbarItem.Identifier("deskfuse-remote-application-metrics")
    init(session: RemoteApplicationSession) {
        self.session = session
        let screen = NSApp.keyWindow?.screen ?? NSScreen.main
        let available = screen?.visibleFrame ?? NSRect(x: 0, y: 0, width: 844, height: 577)
        let initial = NSRect(x: available.midX - 220, y: available.midY - 130, width: 440, height: 260)
        let window = RemoteCanvasWindow(contentRect: initial,
            styleMask: [.borderless, .closable, .resizable, .miniaturizable], backing: .buffered, defer: false)
        window.title = "\(session.applicationName) — \(session.peerName)"
        window.isOpaque = false; window.backgroundColor = .clear; window.hasShadow = true
        window.acceptsMouseMovedEvents = true
        window.isReleasedWhenClosed = false
        super.init(window: window); window.delegate = self
        window.pointerMoved = { [weak self, weak window] point in
            guard let self, let window else { return }
            let size = window.contentView?.bounds.size ?? window.frame.size
            self.chrome.updatePointer(point, windowSize: size)
        }
        window.localMenuClick = { [weak self, weak window] point in
            guard let self, let window else { return false }
            return self.chrome.activateMenu(at: point, in: window)
        }
        window.localAction = { [weak self] action in
            guard let self else { return }
            self.session.canvasFocus(false)
            self.window?.makeFirstResponder(nil)
            switch action {
            case .menu: self.chrome.menuPresented.toggle()
            case .close: self.window?.close()
            case .minimize: self.window?.miniaturize(nil)
            case .zoom: self.toggleZoom()
            case .release: self.chrome.menuPresented = false
            }
        }
        session.presentationSizeChanged = { [weak self] size in self?.fitPresentation(size) }
        let content = NSHostingController(rootView: RemoteApplicationView(session: session, chooseApplication: {
            RemoteApplicationManager.shared.openPicker(peerID: session.peerID, peerName: session.peerName)
        }, closeWindow: { [weak self] in self?.window?.close() }, minimizeWindow: { [weak self] in self?.window?.miniaturize(nil) },
            zoomWindow: { [weak self] in self?.toggleZoom() }, chrome: chrome))
        content.sizingOptions = []
        window.contentViewController = content
        window.setFrame(initial, display: false)
        chrome.navigationChanged = { [weak self] in
            // Let the popover finish dismissing before replacing its anchor.
            Task { @MainActor [weak self] in self?.configureNavigation() }
        }
        chrome.metricsChanged = { [weak self] in
            Task { @MainActor [weak self] in self?.configureMetrics() }
        }
        chrome.titlebarColorChanged = { [weak self] in self?.configureTitlebarColor() }
        configureNavigation()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }
    private func configureNavigation() {
        guard let window else { return }
        session.canvasFocus(false)
        let oldFrame = window.frame
        let canvasSize = window.contentLayoutRect.size
        window.toolbar = nil
        if chrome.normalNavigation {
            window.styleMask = [.titled, .closable, .resizable, .miniaturizable]
            window.titleVisibility = .hidden
            window.titlebarAppearsTransparent = true
            window.titlebarSeparatorStyle = .none
            window.isOpaque = true
            window.backgroundColor = chrome.titlebarBackground
            let toolbar = NSToolbar(identifier: "deskfuse-remote-application-navigation")
            toolbar.delegate = self
            toolbar.displayMode = .iconOnly
            toolbar.allowsUserCustomization = false
            toolbar.showsBaselineSeparator = false
            window.toolbarStyle = .unified
            window.toolbar = toolbar
            window.standardWindowButton(.zoomButton)?.target = self
            window.standardWindowButton(.zoomButton)?.action = #selector(toggleZoom)
        } else {
            window.styleMask = [.borderless, .closable, .resizable, .miniaturizable]
            window.isOpaque = false
            window.backgroundColor = .clear
        }
        configureTitlebarColor()
        window.title = "\(session.applicationName) — \(session.peerName)"
        window.contentView?.layoutSubtreeIfNeeded()
        let chromeHeight = window.frame.height - window.contentLayoutRect.height
        let frame = NSRect(x: oldFrame.minX, y: oldFrame.maxY - canvasSize.height - chromeHeight,
                           width: canvasSize.width, height: canvasSize.height + chromeHeight)
        window.setFrame(frame, display: true)
        frameBeforeZoom = nil
        if let lastContentSize { fitPresentation(lastContentSize) }
    }
    private func configureTitlebarColor() {
        guard let window else { return }
        window.backgroundColor = chrome.normalNavigation ? chrome.titlebarBackground : .clear
        window.appearance = chrome.titlebarColor.map { NSAppearance(named: $0.isDark ? .darkAqua : .aqua) } ?? nil
    }
    private func configureMetrics() {
        guard chrome.normalNavigation, let toolbar = window?.toolbar else { return }
        let index = toolbar.items.firstIndex { $0.itemIdentifier == Self.metricsItem }
        if chrome.metricsPresented, index == nil {
            let menuIndex = toolbar.items.firstIndex { $0.itemIdentifier == Self.menuItem } ?? toolbar.items.count
            toolbar.insertItem(withItemIdentifier: Self.metricsItem, at: menuIndex)
        } else if !chrome.metricsPresented, let index {
            toolbar.removeItem(at: index)
        }
    }
    func toolbarDefaultItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        chrome.metricsPresented ? [.flexibleSpace, Self.metricsItem, Self.menuItem] : [.flexibleSpace, Self.menuItem]
    }
    func toolbarAllowedItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        [Self.metricsItem, Self.menuItem, .flexibleSpace]
    }
    func toolbar(_ toolbar: NSToolbar, itemForItemIdentifier identifier: NSToolbarItem.Identifier,
                 willBeInsertedIntoToolbar flag: Bool) -> NSToolbarItem? {
        if identifier == Self.metricsItem {
            let item = NSToolbarItem(itemIdentifier: identifier)
            item.label = "性能信息"
            item.visibilityPriority = .low
            item.isBordered = false
            item.view = NSHostingView(rootView: RemoteMetricsReadout(session: session))
            return item
        }
        guard identifier == Self.menuItem else { return nil }
        let item = NSToolbarItem(itemIdentifier: identifier)
        item.label = "远程应用菜单"
        item.visibilityPriority = .high
        // The hosted view draws its own circle; avoid an extra toolbar bezel.
        item.isBordered = false
        item.view = NSHostingView(rootView: RemoteApplicationMenu(session: session, chrome: chrome,
            chooseApplication: { [weak self] in
                guard let self else { return }
                RemoteApplicationManager.shared.openPicker(peerID: session.peerID, peerName: session.peerName)
            }, closeWindow: { [weak self] in self?.window?.close() },
            minimizeWindow: { [weak self] in self?.window?.miniaturize(nil) },
            zoomWindow: { [weak self] in self?.toggleZoom() }, inNavigation: true))
        return item
    }
    @objc private func toggleZoom() {
        guard let window else { return }
        session.canvasFocus(false)
        if let previous = frameBeforeZoom {
            frameBeforeZoom = nil
            window.setFrame(previous, display: true)
            if let lastContentSize { fitPresentation(lastContentSize) }
            return
        }
        frameBeforeZoom = window.frame
        let available = (window.screen?.visibleFrame ?? window.frame).insetBy(dx: 16, dy: 16)
        let layout = window.contentLayoutRect
        let chromeSize = NSSize(width: window.frame.width - layout.width, height: window.frame.height - layout.height)
        let aspect = canvasAspect ?? layout.width / max(1, layout.height)
        let width = min(available.width - chromeSize.width, (available.height - chromeSize.height) * aspect)
        let size = NSSize(width: width + chromeSize.width, height: width / aspect + chromeSize.height)
        window.setFrame(NSRect(x: available.midX - size.width / 2, y: available.midY - size.height / 2,
                               width: size.width, height: size.height), display: true)
    }
    private func fitPresentation(_ size: CGSize) {
        guard size.width > 0, size.height > 0, let window, !session.choosingApplication else { return }
        lastContentSize = size
        guard session.scale == .fit else { canvasAspect = nil; return }
        let aspect = size.width / size.height
        canvasAspect = aspect
        guard !window.styleMask.contains(.fullScreen) else { return }
        let old = window.frame, layout = window.contentLayoutRect
        let chrome = CGSize(width: old.width - layout.width, height: old.height - layout.height)
        let available = (window.screen?.visibleFrame ?? old).insetBy(dx: 16, dy: 16)
        let minimumWidth = min(240, 180 * aspect, size.width)
        window.contentMinSize = CGSize(width: minimumWidth, height: minimumWidth / aspect)
        let preferredWidth = fittedInitially ? layout.width : size.width
        let width = min(max(minimumWidth, preferredWidth), available.width - chrome.width,
                        (available.height - chrome.height) * aspect)
        let frameSize = CGSize(width: width + chrome.width, height: width / aspect + chrome.height)
        guard abs(old.width - frameSize.width) > 0.5 || abs(old.height - frameSize.height) > 0.5 else { return }
        let x = min(max(fittedInitially ? old.minX : available.midX - frameSize.width / 2, available.minX), available.maxX - frameSize.width)
        let y = min(max(fittedInitially ? old.maxY - frameSize.height : available.midY - frameSize.height / 2, available.minY), available.maxY - frameSize.height)
        fittedInitially = true
        window.setFrame(CGRect(x: x, y: y, width: frameSize.width, height: frameSize.height), display: true)
    }
    func windowWillResize(_ sender: NSWindow, to frameSize: NSSize) -> NSSize {
        frameBeforeZoom = nil
        guard let aspect = canvasAspect, session.scale == .fit, !session.choosingApplication,
              !sender.styleMask.contains(.fullScreen) else { return frameSize }
        let chrome = CGSize(width: sender.frame.width - sender.contentLayoutRect.width,
                            height: sender.frame.height - sender.contentLayoutRect.height)
        if abs(frameSize.width - sender.frame.width) >= abs(frameSize.height - sender.frame.height) {
            return CGSize(width: frameSize.width, height: (frameSize.width - chrome.width) / aspect + chrome.height)
        }
        return CGSize(width: (frameSize.height - chrome.height) * aspect + chrome.width, height: frameSize.height)
    }
    func windowDidExitFullScreen(_ notification: Notification) {
        canvasAspect = nil
        if let lastContentSize { fitPresentation(lastContentSize) }
    }
    func windowDidChangeScreen(_ notification: Notification) {
        if let lastContentSize { fitPresentation(lastContentSize) }
    }
    func windowDidResignKey(_ notification: Notification) { session.canvasFocus(false) }
    func windowWillClose(_ notification: Notification) { RemoteApplicationManager.shared.closeViewer(session) }
    func windowDidMiniaturize(_ notification: Notification) { session.setViewerVisible(false) }
    func windowDidDeminiaturize(_ notification: Notification) { session.setViewerVisible(true) }
    func windowDidBecomeKey(_ notification: Notification) {
        RemoteApplicationManager.shared.viewerDidBecomeKey(session)
        session.autoAcquire = session.running && !session.viewerPaused && !session.awaitingResumeSurface
    }

}
