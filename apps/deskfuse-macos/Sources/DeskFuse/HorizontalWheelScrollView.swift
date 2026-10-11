import AppKit
import SwiftUI

/// A compact horizontal scroller that maps a regular mouse wheel to horizontal
/// movement while the pointer is over the category row.
struct HorizontalWheelScrollView<Content: View>: NSViewRepresentable {
    let content: () -> Content

    init(@ViewBuilder content: @escaping () -> Content) {
        self.content = content
    }

    func makeNSView(context: Context) -> WheelScrollView<Content> {
        let view = WheelScrollView<Content>()
        view.updateContent(content())
        return view
    }

    func updateNSView(_ nsView: WheelScrollView<Content>, context: Context) {
        nsView.updateContent(content())
    }
}

final class WheelScrollView<Content: View>: NSScrollView {
    private var hostingView: NSHostingView<Content>?

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        drawsBackground = false
        borderType = .noBorder
        hasVerticalScroller = false
        hasHorizontalScroller = false
        horizontalScrollElasticity = .allowed
        verticalScrollElasticity = .none
        scrollerStyle = .overlay
    }

    required init?(coder: NSCoder) { nil }

    func updateContent(_ content: Content) {
        if let hostingView {
            hostingView.rootView = content
        } else {
            let hostingView = NSHostingView(rootView: content)
            self.hostingView = hostingView
            documentView = hostingView
        }
        needsLayout = true
    }

    override func layout() {
        super.layout()
        guard let hostingView else { return }
        let fitting = hostingView.fittingSize
        let width = max(fitting.width, contentView.bounds.width)
        let height = max(fitting.height, contentView.bounds.height)
        hostingView.frame = NSRect(x: 0, y: 0, width: width, height: height)
    }

    override func scrollWheel(with event: NSEvent) {
        let delta = abs(event.scrollingDeltaX) > 0.01 ? event.scrollingDeltaX : event.scrollingDeltaY
        let scale: CGFloat = event.hasPreciseScrollingDeltas ? 1 : 10
        let clipView = contentView
        var origin = clipView.bounds.origin
        origin.x -= delta * scale
        let maxX = max(0, (documentView?.frame.width ?? 0) - clipView.bounds.width)
        origin.x = min(max(origin.x, 0), maxX)
        clipView.setBoundsOrigin(origin)
    }
}
