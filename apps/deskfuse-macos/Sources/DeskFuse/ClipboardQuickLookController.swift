import AppKit
import Observation
import SwiftUI
import DeskFuseEngine

/// Owns the space-bar preview window.
///
/// The preview used to be an overlay inside whichever surface opened it, so it
/// could never be larger than the 440 pt menu bar panel. It now lives in its own
/// screen-sized window, which is also why it is a *display-only* window: the
/// opening surface keeps keyboard focus and therefore keeps owning Space/Esc, and
/// the window is attached as a child so it follows that surface and stays out of
/// the panel's outside-click dismissal.
@MainActor @Observable final class ClipboardQuickLookModel {
    var content: ClipboardContent?
    var image: NSImage?
    @ObservationIgnored var dismiss: () -> Void = {}
}

@MainActor final class ClipboardQuickLookController {
    static let shared = ClipboardQuickLookController()
    @ObservationIgnored private let model = ClipboardQuickLookModel()
    @ObservationIgnored private var panel: QuickLookPanel?
    @ObservationIgnored private var hosting: NSHostingController<QuickLookHost>?
    @ObservationIgnored private var mouseMonitor: Any?
    private var presented = false

    /// The window the preview is drawn in, so surfaces that dismiss on outside
    /// clicks can treat it as part of themselves.
    var window: NSWindow? { presented ? panel : nil }

    func present(over parent: NSWindow?, dismiss: @escaping () -> Void) {
        model.dismiss = { [weak self] in
            guard let self else { return }
            dismiss()
            // `dismiss` clears the caller's state, which calls `close()`; closing
            // here as well keeps a caller that only flips its flag consistent.
            self.close()
        }
        let panel = self.panel ?? makePanel()
        self.panel = panel
        if hosting == nil {
            let hosting = NSHostingController(rootView: QuickLookHost(model: model))
            self.hosting = hosting
            panel.contentViewController = hosting
        }
        let screen = parent?.screen ?? NSScreen.main
        panel.setFrame(screen?.frame ?? NSRect(x: 0, y: 0, width: 1440, height: 900), display: true)
        if let parent, panel.parent !== parent {
            panel.parent?.removeChildWindow(panel)
            parent.addChildWindow(panel, ordered: .above)
        }
        panel.orderFront(nil)
        presented = true
        installMouseMonitor()
    }

    func update(content: ClipboardContent?, image: NSImage?) {
        model.content = content
        model.image = image
    }

    func close() {
        guard presented || panel?.isVisible == true else { return }
        presented = false
        model.content = nil
        model.image = nil
        model.dismiss = {}
        removeMouseMonitor()
        if let panel {
            panel.parent?.removeChildWindow(panel)
            panel.orderOut(nil)
        }
    }

    private func makePanel() -> QuickLookPanel {
        let panel = QuickLookPanel(contentRect: .zero,
            styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: true)
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.isMovable = false
        panel.isReleasedWhenClosed = false
        panel.hidesOnDeactivate = false
        panel.hasShadow = false
        panel.level = .popUpMenu
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .transient, .ignoresCycle]
        return panel
    }

    /// A click anywhere on the preview dismisses it. The hosted view also handles
    /// the tap; this monitor additionally covers the first click on a window that
    /// deliberately cannot become key.
    private func installMouseMonitor() {
        guard mouseMonitor == nil else { return }
        mouseMonitor = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown]) { [weak self] event in
            let handled: Bool = MainActor.assumeIsolated {
                guard let self, let panel = self.panel, event.window === panel else { return false }
                self.model.dismiss()
                return true
            }
            return handled ? nil : event
        }
    }

    private func removeMouseMonitor() {
        if let mouseMonitor { NSEvent.removeMonitor(mouseMonitor) }
        mouseMonitor = nil
    }
}

/// Bridges the observable model into the one shared preview view.
private struct QuickLookHost: View {
    @Bindable var model: ClipboardQuickLookModel
    var body: some View {
        ClipboardQuickLook(content: model.content, image: model.image) { model.dismiss() }
    }
}

private final class QuickLookPanel: NSPanel {
    // Key handling stays with the surface that opened the preview.
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
}
