import SwiftUI
import AppKit

/// 指针没移动时，不把「行滑到光标下面」或「气泡出现导致的悬停回放」当成新的悬停。
/// 键盘切换和气泡自身布局都会制造这种假悬停，若当真就会和预览互相抢。
@MainActor enum ClipboardPreviewHoverGate {
    private static var suppressedLocation: NSPoint?

    static func suppressCurrentPointer() {
        suppressedLocation = NSEvent.mouseLocation
    }

    static func allows(_ location: NSPoint) -> Bool {
        guard let suppressedLocation else { return true }
        let dx = location.x - suppressedLocation.x
        let dy = location.y - suppressedLocation.y
        return dx * dx + dy * dy > 1
    }

    static func pointerIsInside(_ view: NSView) -> Bool {
        guard let window = view.window, view.bounds.width > 1, view.bounds.height > 1 else { return false }
        let inWindow = window.convertPoint(fromScreen: NSEvent.mouseLocation)
        let local = view.convert(inWindow, from: nil)
        return view.bounds.contains(local)
    }
}

/// Each row owns its anchor and cancellable dwell timer; hovering never changes
/// the keyboard/paste selection. Leaving the row cancels only a pending open,
/// so the pointer can move into the displayed bubble to scroll or select text.
struct ClipboardHoverPreview: ViewModifier {
    let session: UUID
    let active: Bool
    let open: (NSView) -> Void
    @State private var anchor = ClipboardQuickLookAnchor()
    @State private var pending: Task<Void, Never>?

    func body(content: Content) -> some View {
        content
            .background { ClipboardQuickLookAnchorView(anchor: anchor) }
            .onHover { inside in
                cancel()
                guard inside, active else { return }
                let location = NSEvent.mouseLocation
                guard ClipboardPreviewHoverGate.allows(location) else { return }
                pending = Task { @MainActor in
                    do { try await Task.sleep(for: .seconds(1)) } catch { return }
                    guard !Task.isCancelled, ClipboardPreviewHoverGate.allows(location) else { return }
                    guard let view = anchor.view, ClipboardPreviewHoverGate.pointerIsInside(view) else { return }
                    open(view)
                }
            }
            .onChange(of: session) { _, _ in cancel() }
            .onChange(of: active) { _, _ in cancel() }
            .onDisappear { cancel() }
    }

    private func cancel() { pending?.cancel(); pending = nil }
}
