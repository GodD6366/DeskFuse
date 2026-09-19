import SwiftUI
import AppKit
import Observation
import UniDeskEngine
import UniDeskRemoteApplication

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
    @ObservationIgnored private var viewerOwner: UUID?
    @ObservationIgnored private var hostOwner: UUID?
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
    func openedApplications(peerID: String) -> Set<String> {
        Set(viewers.values.filter { $0.peerID == peerID }.compactMap(\.applicationKey))
    }
    func openApplication(_ application: RemoteApplicationInfo, from picker: RemoteApplicationSession) {
        guard let engine else { return }
        if let existing = viewers.values.first(where: { $0.peerID == picker.peerID && $0.applicationKey == application.applicationKey }),
           let window = windows[existing.id] {
            window.showWindow(nil); window.window?.deminiaturize(nil); window.window?.makeKeyAndOrderFront(nil)
            if !existing.active && !existing.busy { existing.reconnectApplication() }
            return
        }
        let session = RemoteApplicationSession(engine: engine, peerID: picker.peerID, peerName: picker.peerName, role: .viewer)
        session.prepareApplication(application)
        viewers[session.id] = session
        let controller = RemoteApplicationWindow(session: session)
        windows[session.id] = controller
        controller.showWindow(nil); controller.window?.makeKeyAndOrderFront(nil)
        Task { await session.start() }
    }
    func closeViewer(_ session: RemoteApplicationSession) {
        session.releaseControl(resumeSharing: true)
        viewers.removeValue(forKey: session.id); windows.removeValue(forKey: session.id)
        Task { await session.stop() }
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
        let sessions = Array(hosts.values) + Array(viewers.values) + pickers.values.map(\.session)
        for session in sessions { session.releaseControl(resumeSharing: true); session.yieldHostControl() }
        Task { for session in sessions { await session.stop() } }
    }
}

@available(macOS 14.2, *)
@MainActor final class RemoteApplicationPickerWindow: NSWindowController, NSWindowDelegate {
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
    func windowWillClose(_ notification: Notification) { RemoteApplicationManager.shared.closePicker(session) }
}

@available(macOS 14.2, *)
@MainActor private final class RemoteCanvasWindow: NSWindow {
    enum LocalAction { case menu, close, minimize, zoom, release }
    var localAction: ((LocalAction) -> Void)?
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
        if event.type == .keyUp, consumedKeys.remove(event.keyCode) != nil { return }
        if handleLocalShortcut(event) { return }
        super.sendEvent(event)
    }
}

@available(macOS 14.2, *)
@MainActor final class RemoteApplicationWindow: NSWindowController, NSWindowDelegate {
    let session: RemoteApplicationSession
    private var canvasAspect: CGFloat?
    private var lastContentSize: CGSize?
    private var fittedInitially = false
    private let chrome = RemoteWindowChrome()
    private var frameBeforeZoom: NSRect?
    init(session: RemoteApplicationSession) {
        self.session = session
        let screen = NSApp.keyWindow?.screen ?? NSScreen.main
        let available = screen?.visibleFrame ?? NSRect(x: 0, y: 0, width: 844, height: 577)
        let initial = NSRect(x: available.midX - 220, y: available.midY - 130, width: 440, height: 260)
        let window = RemoteCanvasWindow(contentRect: initial,
            styleMask: [.borderless, .closable, .resizable, .miniaturizable], backing: .buffered, defer: false)
        window.title = "\(session.applicationName) — \(session.peerName)"
        window.isOpaque = false; window.backgroundColor = .clear; window.hasShadow = true
        window.isReleasedWhenClosed = false
        super.init(window: window); window.delegate = self
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
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }
    private func toggleZoom() {
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
        let aspect = canvasAspect ?? window.frame.width / max(1, window.frame.height)
        let width = min(available.width, available.height * aspect)
        let size = NSSize(width: width, height: width / aspect)
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
        session.autoAcquire = session.running && !session.viewerPaused && !session.awaitingResumeSurface
    }

}
