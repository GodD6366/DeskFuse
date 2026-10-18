import AppKit
import SwiftUI

/// Window activity is not visibility: a visible, unfocused chart should still
/// update, while an occluded or closed window should not animate.
struct WindowVisibilityObserver: NSViewRepresentable {
    let onChange: (Bool) -> Void

    func makeNSView(context: Context) -> VisibilityView {
        let view = VisibilityView()
        view.onChange = onChange
        return view
    }

    func updateNSView(_ view: VisibilityView, context: Context) {
        view.onChange = onChange
    }

    static func dismantleNSView(_ view: VisibilityView, coordinator: ()) {
        view.onChange = nil
        NotificationCenter.default.removeObserver(view)
    }

    final class VisibilityView: NSView {
        var onChange: ((Bool) -> Void)?
        private var lastVisibility: Bool?

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            NotificationCenter.default.removeObserver(self)
            if let window {
                NotificationCenter.default.addObserver(self,
                    selector: #selector(visibilityChanged),
                    name: NSWindow.didChangeOcclusionStateNotification, object: window)
            }
            visibilityChanged()
        }

        @objc private func visibilityChanged() {
            // AppKit may attach the view during a SwiftUI update transaction.
            Task { @MainActor [weak self] in
                guard let self else { return }
                let visible = window?.occlusionState.contains(.visible) == true
                guard visible != lastVisibility else { return }
                lastVisibility = visible
                onChange?(visible)
            }
        }

        override func hitTest(_ point: NSPoint) -> NSView? { nil }
    }
}
