import AppKit
import SwiftUI

/// Keep List's native selection, focus, accessibility and arrow-key navigation,
/// but let the row supply the reference design's subtle selection background.
/// SwiftUI has no macOS modifier for NSTableView.selectionHighlightStyle.
struct NativeListSelection: NSViewRepresentable {
    final class Marker: NSView {
        override func hitTest(_ point: NSPoint) -> NSView? { nil }
        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            configureTable()
        }
        override func layout() {
            super.layout()
            configureTable()
        }
        private func configureTable() {
            var ancestor = superview
            while let view = ancestor {
                if let table = view as? NSTableView {
                    if table.selectionHighlightStyle != .none {
                        table.selectionHighlightStyle = .none
                    }
                    return
                }
                ancestor = view.superview
            }
        }
    }
    func makeNSView(context: Context) -> Marker { Marker() }
    func updateNSView(_ view: Marker, context: Context) { }
}
