import AppKit
import Carbon
import Observation
import SwiftUI
import UniDeskEngine

/// Owns the one status-item presentation. Business settings and content stay in Rust.
@MainActor @Observable final class ClipboardPanelController: NSObject, NSPopoverDelegate {
    static let shared = ClipboardPanelController()
    private(set) var panelHeight: CGFloat = 780
    private(set) var session = UUID()
    private(set) var error: String?
    private(set) var shortcut: WorkflowKey? = WorkflowKey(key_code: 9, modifiers: 0x180000)
    @ObservationIgnored private var item: NSStatusItem?
    @ObservationIgnored private let popover = NSPopover()
    @ObservationIgnored private var hosting: NSHostingController<MenuBarClipboardView>?
    @ObservationIgnored private var pointerPanel: ClipboardPointerPanel?
    @ObservationIgnored private var outsideMonitor: Any?
    @ObservationIgnored private var localMouseMonitor: Any?
    @ObservationIgnored private var target: NSRunningApplication?
    @ObservationIgnored private var hotKey: EventHotKeyRef?
    @ObservationIgnored private var handler: EventHandlerRef?
    @ObservationIgnored private var pasteTask: Task<Void, Never>?
    @ObservationIgnored private var closingForPaste = false
    @ObservationIgnored private var suspended = false

    func install(store: AppStore) {
        guard item == nil else { return }
        let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        self.item = item
        let image = Bundle.main.url(forResource: "unidesk-status-icon", withExtension: "png")
            .flatMap { NSImage(contentsOf: $0) }
            ?? NSImage(systemSymbolName: "rectangle.on.rectangle", accessibilityDescription: "uniDesk")
        image?.size = NSSize(width: 18, height: 18)
        image?.isTemplate = true
        item.button?.image = image
        item.button?.toolTip = "uniDesk 剪贴板"
        item.button?.target = self
        item.button?.action = #selector(toggle)
        popover.behavior = .transient
        popover.animates = false
        popover.delegate = self
        hosting = NSHostingController(rootView: MenuBarClipboardView(store: store))
        popover.contentViewController = hosting
        let pointerPanel = ClipboardPointerPanel(contentRect: .zero,
            styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: true)
        pointerPanel.titleVisibility = .hidden
        pointerPanel.isOpaque = false
        pointerPanel.backgroundColor = .clear
        pointerPanel.isMovable = false
        pointerPanel.isReleasedWhenClosed = false
        pointerPanel.hidesOnDeactivate = false
        pointerPanel.hasShadow = true
        pointerPanel.level = .popUpMenu
        pointerPanel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .transient, .ignoresCycle]
        for button in [NSWindow.ButtonType.closeButton, .miniaturizeButton, .zoomButton] {
            pointerPanel.standardWindowButton(button)?.isHidden = true
        }
        self.pointerPanel = pointerPanel
        outsideMonitor = NSEvent.addGlobalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown, .otherMouseDown]) { _ in
            Task { @MainActor in
                // Any click outside the app collapses the popup, whichever
                // presentation (status item or global shortcut) opened it.
                if self.pointerPanel?.isVisible == true || self.popover.isShown { self.close() }
            }
        }
        localMouseMonitor = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown, .otherMouseDown]) { event in
            MainActor.assumeIsolated {
                let panel = self.pointerPanel
                let popoverWindow = self.popover.contentViewController?.view.window
                let visible = panel?.isVisible == true || self.popover.isShown
                if visible, let clicked = event.window,
                   clicked !== panel, clicked.sheetParent !== panel,
                   clicked !== popoverWindow,
                   clicked !== self.item?.button?.window {
                    self.close()
                }
            }
            return event
        }
        var spec = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
        InstallEventHandler(GetApplicationEventTarget(), { _, event, _ in
            guard let event else { return OSStatus(eventNotHandledErr) }
            var id = EventHotKeyID()
            let status = GetEventParameter(event, EventParamName(kEventParamDirectObject), EventParamType(typeEventHotKeyID), nil, MemoryLayout<EventHotKeyID>.size, nil, &id)
            guard status == noErr, id.signature == 0x55444350 else { return OSStatus(eventNotHandledErr) }
            Task { @MainActor in ClipboardPanelController.shared.toggleAtPointer() }
            return noErr
        }, 1, &spec, nil, &handler)
        // Do not register the default before persisted disabled/custom settings
        // have loaded: that would briefly hijack a user's disabled shortcut.
        Task { @MainActor in
            while store.client == nil {
                try? await Task.sleep(for: .milliseconds(100))
                if Task.isCancelled { return }
            }
            await self.reloadShortcut()
        }
    }

    func reloadShortcut() async {
        do {
            guard let client = AppStore.shared.client else { return }
            let response = try await client.libraryQuery("{\"op\":\"state\"}")
            guard let state = try JSONSerialization.jsonObject(with: Data(response.utf8)) as? [String: Any],
                  let settings = state["settings"] as? [String: Any],
                  let value = settings["shortcut"] as? [String: Any],
                  let enabled = value["enabled"] as? Bool,
                  let key = value["key_code"] as? UInt16,
                  let flags = value["modifiers"] as? UInt64 else {
                throw ClipboardPanelFailure.message("剪贴板快捷键设置格式无效")
            }
            configure(enabled ? WorkflowKey(key_code: key, modifiers: flags) : nil)
        } catch { self.error = error.localizedDescription }
    }

    /// Call after loading/saving the Rust-owned local clipboard shortcut setting.
    func configure(_ shortcut: WorkflowKey?) {
        if let hotKey { UnregisterEventHotKey(hotKey) }
        hotKey = nil
        self.shortcut = shortcut
        error = nil
        guard let shortcut, !suspended else { return }
        var modifiers: UInt32 = 0
        if shortcut.modifiers & 0x100000 != 0 { modifiers |= UInt32(cmdKey) }
        if shortcut.modifiers & 0x80000 != 0 { modifiers |= UInt32(optionKey) }
        if shortcut.modifiers & 0x40000 != 0 { modifiers |= UInt32(controlKey) }
        if shortcut.modifiers & 0x20000 != 0 { modifiers |= UInt32(shiftKey) }
        let status = RegisterEventHotKey(UInt32(shortcut.key_code), modifiers,
            EventHotKeyID(signature: 0x55444350, id: 1), GetApplicationEventTarget(), 0, &hotKey)
        if status != noErr { error = "剪贴板快捷键注册失败（\(status)），请换一组按键。" }
    }

    func suspendShortcut(_ value: Bool) {
        suspended = value
        configure(shortcut)
    }

    @objc func toggle() { present(atPointer: false) }

    func toggleAtPointer() { present(atPointer: true) }

    private func present(atPointer: Bool) {
        guard !suspended else { return }
        if popover.isShown || pointerPanel?.isVisible == true { close(); return }
        pasteTask?.cancel()
        closingForPaste = false
        session = UUID()
        let frontmost = NSWorkspace.shared.frontmostApplication
        target = frontmost?.processIdentifier == ProcessInfo.processInfo.processIdentifier ? nil : frontmost
        let mouse = NSEvent.mouseLocation
        let screen = atPointer
            ? (NSScreen.screens.first { $0.frame.contains(mouse) } ?? NSScreen.main)
            : (item?.button?.window?.screen ?? NSScreen.main)
        guard let screen else { return }
        let visible = screen.visibleFrame.insetBy(dx: 8, dy: 8)
        // Leave room for native window chrome / the status popover arrow.
        panelHeight = min(780, max(1, visible.height - 40))
        if atPointer {
            guard let panel = pointerPanel else { return }
            // Move the same SwiftUI host; there is never a second clipboard session.
            popover.contentViewController = nil
            panel.contentViewController = hosting
            panel.setContentSize(NSSize(width: 440, height: panelHeight))
            let size = panel.frame.size
            let preferredX = mouse.x + 12 + size.width <= visible.maxX ? mouse.x + 12 : mouse.x - size.width - 12
            let x = min(max(preferredX, visible.minX), max(visible.minX, visible.maxX - size.width))
            let y = min(max(mouse.y + 16 - size.height, visible.minY), max(visible.minY, visible.maxY - size.height))
            panel.setFrameOrigin(NSPoint(x: x, y: y))
            panel.makeKeyAndOrderFront(nil)
        } else {
            guard let button = item?.button else { return }
            pointerPanel?.contentViewController = nil
            popover.contentViewController = hosting
            popover.contentSize = NSSize(width: 440, height: panelHeight)
            popover.show(relativeTo: button.bounds, of: button, preferredEdge: .minY)
            popover.contentViewController?.view.window?.makeKey()
        }
    }

    func reportPasteFailure(_ message: String) { error = message }

    func close() {
        closingForPaste = false
        session = UUID()
        pasteTask?.cancel()
        popover.performClose(nil)
        pointerPanel?.orderOut(nil)
    }

    func popoverDidClose(_ notification: Notification) {
        if !closingForPaste { session = UUID(); pasteTask?.cancel() }
    }

    /// Run only after the explicit restore command's applied event. Capture the
    /// change count at that event, not after activating the destination app.
    func pasteRestoredContent(expectedChangeCount: Int64, in presentation: UUID) async throws {
        guard presentation == session else { throw ClipboardPanelFailure.cancelled }
        guard AXIsProcessTrusted() else { throw ClipboardPanelFailure.message("请在系统设置 → 隐私与安全性 → 辅助功能中授权 uniDesk；内容已复制，可手动粘贴。") }
        guard let target, !target.isTerminated else { throw ClipboardPanelFailure.message("原目标应用不可用；内容已复制，可手动粘贴。") }
        closingForPaste = true
        popover.performClose(nil)
        pointerPanel?.orderOut(nil)
        guard target.activate(options: []) else { throw ClipboardPanelFailure.message("无法激活原目标应用；内容已复制。") }
        for _ in 0..<100 {
            try Task.checkCancellation()
            guard presentation == session else { throw ClipboardPanelFailure.cancelled }
            if NSWorkspace.shared.frontmostApplication?.processIdentifier == target.processIdentifier,
               NSEvent.modifierFlags.intersection([.command, .option, .control, .shift]).isEmpty {
                try clipboardPasteIfCurrent(targetPid: target.processIdentifier, expectedChangeCount: expectedChangeCount)
                return
            }
            try await Task.sleep(for: .milliseconds(20))
        }
        throw ClipboardPanelFailure.message("目标应用未获得焦点或修饰键尚未释放，已取消自动粘贴。")
    }
}

private final class ClipboardPointerPanel: NSPanel {
    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { false }
}

enum ClipboardPanelFailure: LocalizedError {
    case cancelled
    case message(String)
    var errorDescription: String? {
        switch self {
        case .cancelled: "剪贴板面板会话已变化，已取消粘贴。"
        case .message(let message): message
        }
    }
}
