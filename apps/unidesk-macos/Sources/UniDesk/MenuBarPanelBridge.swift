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
                    let closing = notification.name == NSWindow.willCloseNotification || notification.name == NSWindow.didResignKeyNotification
                    MainActor.assumeIsolated {
                        if closing { self?.owner.handle.generation &+= 1 }
                    }
                    Task { @MainActor [weak self] in self?.reconcile(forceClosed: closing) }
                })
            }
            monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
                let consumed: Bool = MainActor.assumeIsolated {
                    guard let self, let window = self.window,
                          window.isVisible, window.isKeyWindow, event.window === window else { return false }
                    return self.owner.keyDown(event)
                }
                return consumed ? nil : event
            }
            // Never mutate SwiftUI state during an NSView update callback.
            Task { @MainActor [weak self] in self?.reconcile() }
        }
        func reconcile(forceClosed: Bool = false) {
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
