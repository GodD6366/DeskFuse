import SwiftUI
import AppKit

enum ClipboardPreviewTrigger: String, CaseIterable, Identifiable {
    case hover
    case space

    var id: Self { self }
    var label: String { self == .hover ? "鼠标悬停" : "空格" }
}

@MainActor enum ClipboardPreviewHoverGate {
    /// Idle state is hover preview. Keyboard ownership is the non-default mode
    /// and is released as soon as the preview closes.
    private(set) static var keyboardLocked = false

    /// A hover-owned preview keeps hover as the active trigger. Calling this
    /// also drops any pending dwell so the popover does not immediately reopen.
    static func beginHoverPreview() {
        keyboardLocked = false
        suppressCurrentPointer()
    }

    static func lockForKeyboardPreview() {
        keyboardLocked = true
        suppressCurrentPointer()
    }

    static func unlockAfterClosingPreview() {
        keyboardLocked = false
        suppressCurrentPointer()
    }

    static func suppressCurrentPointer() {
        ClipboardHoverPreviewCoordinator.shared.suppressCurrentPointer()
    }

    static func pointerIsInside(_ view: NSView) -> Bool {
        guard let window = view.window, window.isVisible, !view.clipboardPreviewVisibleRect.isEmpty,
              NSWindow.windowNumber(at: NSEvent.mouseLocation, belowWindowWithWindowNumber: 0) == window.windowNumber else { return false }
        let point = view.convert(window.convertPoint(fromScreen: NSEvent.mouseLocation), from: nil)
        return view.clipboardPreviewVisibleRect.contains(point)
    }
}

/// One dwell timer for the entire list. Only real mouse movement can acquire a
/// row: layout/scroll/SwiftUI hover replay must never compete with keyboard input.
@MainActor final class ClipboardHoverPreviewCoordinator {
    static let shared = ClipboardHoverPreviewCoordinator()
    private struct Row { weak var view: ClipboardHoverTrackingView? }
    private var rows: [ObjectIdentifier: Row] = [:]
    private var monitor: Any?
    private weak var window: NSWindow?
    private var previouslyAcceptedMouseMoved = false
    private weak var candidate: ClipboardHoverTrackingView?
    private var pending: Task<Void, Never>?
    private var lastLocation = NSEvent.mouseLocation

    func register(_ view: ClipboardHoverTrackingView) {
        guard view.active, let parent = view.window else { unregister(view); return }
        rows[ObjectIdentifier(view)] = Row(view: view)
        guard monitor == nil else { return }
        window = parent
        previouslyAcceptedMouseMoved = parent.acceptsMouseMovedEvents
        parent.acceptsMouseMovedEvents = true
        lastLocation = NSEvent.mouseLocation
        monitor = NSEvent.addLocalMonitorForEvents(matching: [.mouseMoved, .leftMouseDragged, .scrollWheel, .leftMouseDown, .rightMouseDown]) { [weak self] event in
            MainActor.assumeIsolated { self?.handle(event) }
            return event
        }
    }

    func unregister(_ view: ClipboardHoverTrackingView) {
        rows.removeValue(forKey: ObjectIdentifier(view))
        if candidate === view { suppressCurrentPointer() }
        rows = rows.filter { $0.value.view != nil }
        guard rows.isEmpty else { return }
        if let monitor { NSEvent.removeMonitor(monitor) }
        monitor = nil
        window?.acceptsMouseMovedEvents = previouslyAcceptedMouseMoved
        window = nil
    }

    func suppressCurrentPointer() {
        pending?.cancel()
        pending = nil
        candidate = nil
        lastLocation = NSEvent.mouseLocation
    }

    private func handle(_ event: NSEvent) {
        guard event.type == .mouseMoved else {
            suppressCurrentPointer()
            if event.type == .scrollWheel {
                // AppKit applies the scroll after local monitors return.
                Task { @MainActor in
                    await Task.yield()
                    ClipboardQuickLookController.shared.refreshPosition()
                }
            }
            return
        }
        guard !ClipboardPreviewHoverGate.keyboardLocked else { return }
        let location = NSEvent.mouseLocation
        guard location != lastLocation else { return }
        lastLocation = location
        let row = rows.values.compactMap(\.view).first {
            $0.active && $0.window === event.window && ClipboardPreviewHoverGate.pointerIsInside($0)
        }
        guard candidate !== row else { return }
        pending?.cancel()
        pending = nil
        candidate = row
        guard let row else { return }
        pending = Task { @MainActor [weak self, weak row] in
            do { try await Task.sleep(for: .milliseconds(300)) } catch { return }
            guard !Task.isCancelled, !ClipboardPreviewHoverGate.keyboardLocked,
                  let self, let row, self.candidate === row else { return }
            self.pending = nil
            guard row.active, ClipboardPreviewHoverGate.pointerIsInside(row) else {
                self.candidate = nil
                return
            }
            row.open?(row)
        }
    }
}

@MainActor final class ClipboardHoverTrackingView: NSView {
    var active = false
    var itemID: UInt64?
    var session: UUID?
    var open: ((NSView) -> Void)?

    override func hitTest(_ point: NSPoint) -> NSView? { nil }
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        ClipboardHoverPreviewCoordinator.shared.register(self)
    }
}

private struct ClipboardHoverTrackingArea: NSViewRepresentable {
    let itemID: UInt64
    let session: UUID
    let active: Bool
    let open: (NSView) -> Void

    func makeNSView(context: Context) -> ClipboardHoverTrackingView { ClipboardHoverTrackingView() }
    func updateNSView(_ view: ClipboardHoverTrackingView, context: Context) {
        if view.session != session || view.itemID != itemID { ClipboardHoverPreviewCoordinator.shared.unregister(view) }
        view.itemID = itemID
        view.session = session
        view.active = active
        view.open = open
        ClipboardHoverPreviewCoordinator.shared.register(view)
    }
    static func dismantleNSView(_ view: ClipboardHoverTrackingView, coordinator: Void) {
        ClipboardHoverPreviewCoordinator.shared.unregister(view)
        view.open = nil
    }
}

/// Hover previews do not change the keyboard/paste selection. Moving into the
/// bubble cancels pending dwell only; the already displayed preview stays open.
struct ClipboardHoverPreview: ViewModifier {
    let itemID: UInt64
    let session: UUID
    let active: Bool
    let open: (NSView) -> Void

    func body(content: Content) -> some View {
        content.background { ClipboardHoverTrackingArea(itemID: itemID, session: session, active: active, open: open) }
    }
}
