import AppKit

/// Use native overlay scrollers throughout this process without changing the
/// user's system preference. SwiftUI creates/replaces scroll views lazily.
@MainActor final class TransientScrollbars: NSObject {
    static let shared = TransientScrollbars()
    private var installed = false
    private var pending: Set<ObjectIdentifier> = []
    private let indicatorlessViews = NSHashTable<NSScrollView>.weakObjects()

    func hideIndicators(in scroll: NSScrollView) {
        indicatorlessViews.add(scroll)
        if scroll.hasVerticalScroller { scroll.hasVerticalScroller = false }
        if scroll.hasHorizontalScroller { scroll.hasHorizontalScroller = false }
    }

    func install() {
        guard !installed else { return }
        installed = true
        let center = NotificationCenter.default
        center.addObserver(self, selector: #selector(windowUpdated(_:)),
                           name: NSWindow.didUpdateNotification, object: nil)
        center.addObserver(self, selector: #selector(windowShown(_:)),
                           name: NSWindow.didBecomeKeyNotification, object: nil)
        for window in NSApp.windows { configure(window) }
    }

    @objc private func windowShown(_ notification: Notification) {
        guard let window = notification.object as? NSWindow else { return }
        configure(window)
    }

    @objc private func windowUpdated(_ notification: Notification) {
        guard let window = notification.object as? NSWindow,
              window.isVisible, !(window is NSSavePanel) else { return }
        let id = ObjectIdentifier(window)
        guard pending.insert(id).inserted else { return }
        // Coalesce layout/scroll updates; never walk the hierarchy on every frame.
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) { [weak self, weak window] in
            guard let self else { return }
            self.pending.remove(id)
            if let window, window.isVisible { self.configure(window) }
        }
    }

    private func configure(_ window: NSWindow) {
        guard !(window is NSSavePanel), let content = window.contentView else { return }
        configure(content)
    }

    private func configure(_ view: NSView) {
        if let scroll = view as? NSScrollView {
            if indicatorlessViews.contains(scroll) {
                hideIndicators(in: scroll)
            } else {
                if scroll.scrollerStyle != .overlay { scroll.scrollerStyle = .overlay }
                if !scroll.autohidesScrollers { scroll.autohidesScrollers = true }
            }
        }
        for child in view.subviews { configure(child) }
    }
}
