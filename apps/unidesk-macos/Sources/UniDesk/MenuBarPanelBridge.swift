import SwiftUI
import AppKit

@MainActor final class MenuBarPanelHandle {
    weak var window: NSWindow?
    var generation: UInt64 = 0
}

/// MenuBarExtra retains its SwiftUI content. Window-scoped lifecycle callbacks
/// prevent hidden content from handling keys or closing a later presentation.
struct MenuBarPanelBridge: NSViewRepresentable {
    let handle: MenuBarPanelHandle
    let opened: () -> Void
    let closed: () -> Void
    let modifiersChanged: (NSEvent.ModifierFlags) -> Void
    let keyDown: (NSEvent) -> Bool

    @MainActor final class Marker: NSView {
        weak var coordinator: Coordinator?
        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            coordinator?.attach(window)
        }
    }

    @MainActor final class Coordinator {
        var owner: MenuBarPanelBridge
        weak var window: NSWindow?
        var monitor: Any?
        var observers: [NSObjectProtocol] = []
        var active = false

        init(_ owner: MenuBarPanelBridge) { self.owner = owner }
        func attach(_ window: NSWindow?) {
            guard self.window !== window else { return }
            detach()
            self.window = window
            owner.handle.window = window
            guard let window else { return }
            for name in [NSWindow.didBecomeKeyNotification, NSWindow.didResignKeyNotification,
                         NSWindow.didChangeOcclusionStateNotification, NSWindow.willCloseNotification] {
                observers.append(NotificationCenter.default.addObserver(forName: name, object: window, queue: .main) { [weak self] notification in
                    let willClose = notification.name == NSWindow.willCloseNotification
                    let closing = willClose || notification.name == NSWindow.didResignKeyNotification
                    let menuFocusChange = MainActor.assumeIsolated {
                        let ignore = !willClose
                            && ClipboardHistoryMenuSession.isTracking
                        if closing && !ignore { self?.owner.handle.generation &+= 1 }
                        return ignore
                    }
                    // Decide synchronously: a queued resign-key callback must not
                    // close the session after menu tracking has already ended.
                    guard !menuFocusChange else { return }
                    Task { @MainActor [weak self] in self?.reconcile(forceClosed: closing) }
                })
            }
            observers.append(NotificationCenter.default.addObserver(
                forName: ClipboardHistoryMenuSession.didEnd, object: nil, queue: .main
            ) { [weak self] _ in
                // Reconcile after AppKit restores the panel's key-window state.
                Task { @MainActor [weak self] in self?.reconcile() }
            })
            monitor = NSEvent.addLocalMonitorForEvents(matching: [.keyDown, .flagsChanged]) { [weak self] event in
                let consumed: Bool = MainActor.assumeIsolated {
                    guard !ClipboardHistoryMenuSession.isTracking,
                          let self, let window = self.window,
                          window.isVisible, window.isKeyWindow else { return false }
                    guard event.type == .flagsChanged || event.window === window else { return false }
                    self.owner.modifiersChanged(event.modifierFlags)
                    // Observe modifier transitions without consuming them or monitoring motion.
                    guard event.type == .keyDown else { return false }
                    return self.owner.keyDown(event)
                }
                return consumed ? nil : event
            }
            // Never mutate SwiftUI state during an NSView update callback.
            Task { @MainActor [weak self] in self?.reconcile() }
        }
        func reconcile(forceClosed: Bool = false) {
            // Opening a native submenu is not the end of the clipboard session.
            // Do not clear its rows, invalidate copy, or steal its keyboard input.
            if ClipboardHistoryMenuSession.isTracking, window?.isVisible == true { return }
            let next = !forceClosed && window?.isVisible == true && window?.isKeyWindow == true
            guard next != active else { return }
            active = next
            if next { owner.opened() } else { owner.closed() }
        }
        func detach() {
            if let monitor { NSEvent.removeMonitor(monitor) }
            monitor = nil
            for observer in observers { NotificationCenter.default.removeObserver(observer) }
            observers = []
            active = false
            owner.handle.window = nil
            window = nil
        }
    }

    func makeCoordinator() -> Coordinator { Coordinator(self) }
    func makeNSView(context: Context) -> Marker {
        let marker = Marker()
        marker.coordinator = context.coordinator
        return marker
    }
    func updateNSView(_ view: Marker, context: Context) { context.coordinator.owner = self }
    static func dismantleNSView(_ view: Marker, coordinator: Coordinator) { coordinator.detach() }
}
