import SwiftUI
import AppKit

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
                pending = Task { @MainActor in
                    do { try await Task.sleep(for: .seconds(1)) } catch { return }
                    guard !Task.isCancelled, let view = anchor.view, view.window != nil else { return }
                    open(view)
                }
            }
            .onChange(of: session) { _, _ in cancel() }
            .onChange(of: active) { _, _ in cancel() }
            .onDisappear { cancel() }
    }

    private func cancel() { pending?.cancel(); pending = nil }
}
