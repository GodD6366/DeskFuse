import AppKit
import SwiftUI

/// Shown while a Command-V is held for remote clipboard content in flight.
/// The panel never activates DeskFuse, so the paste target stays frontmost
/// and the replayed Command-V lands where the user pressed it.
@MainActor
@Observable
final class RemotePasteHUD {
    static let shared = RemotePasteHUD()

    private(set) var source = ""
    private(set) var received: UInt64 = 0
    private(set) var total: UInt64 = 0
    var onCancel: (() -> Void)?

    @ObservationIgnored private var panel: NSPanel?

    var fraction: Double { total > 0 ? min(1, Double(received) / Double(total)) : 0 }

    func update(source: String, received: UInt64, total: UInt64) {
        self.source = source
        self.received = received
        self.total = total
        present()
    }

    func dismiss() {
        panel?.orderOut(nil)
    }

    func cancel() {
        dismiss()
        onCancel?()
    }

    private func present() {
        let panel = panel ?? makePanel()
        self.panel = panel
        guard !panel.isVisible else { return }
        panel.setFrameOrigin(Self.origin(for: panel.frame.size, near: Self.pasteAnchor()))
        panel.orderFrontRegardless()
    }

    /// Where the held paste will land, in Cocoa screen coordinates: the text
    /// caret of the frontmost app, else its focused field, else the pointer.
    /// The panel never activates, so the frontmost app is the paste target.
    private static func pasteAnchor() -> NSRect {
        let mouse = NSEvent.mouseLocation
        let pointer = NSRect(x: mouse.x, y: mouse.y, width: 0, height: 0)
        guard let pid = NSWorkspace.shared.frontmostApplication?.processIdentifier else { return pointer }
        let app = AXUIElementCreateApplication(pid)
        // A hung target must not stall the main thread.
        AXUIElementSetMessagingTimeout(app, 0.2)
        var focused: CFTypeRef?
        guard AXUIElementCopyAttributeValue(app, kAXFocusedUIElementAttribute as CFString, &focused) == .success,
              let focused, CFGetTypeID(focused) == AXUIElementGetTypeID() else { return pointer }
        let element = focused as! AXUIElement
        if let caret = caretRect(element), caret.height > 0 { return cocoa(caret) }
        // A large focused element (document, canvas) says little about the
        // caret; only anchor to field-sized elements.
        if let field = frame(element), field.height > 0, field.height <= 120 { return cocoa(field) }
        return pointer
    }

    private static func caretRect(_ element: AXUIElement) -> CGRect? {
        var range: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, kAXSelectedTextRangeAttribute as CFString, &range) == .success,
              let range else { return nil }
        var bounds: CFTypeRef?
        guard AXUIElementCopyParameterizedAttributeValue(
            element, kAXBoundsForRangeParameterizedAttribute as CFString, range, &bounds) == .success,
              let bounds, CFGetTypeID(bounds) == AXValueGetTypeID() else { return nil }
        var rect = CGRect.zero
        return AXValueGetValue(bounds as! AXValue, .cgRect, &rect) ? rect : nil
    }

    private static func frame(_ element: AXUIElement) -> CGRect? {
        var position: CFTypeRef?
        var size: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, kAXPositionAttribute as CFString, &position) == .success,
              AXUIElementCopyAttributeValue(element, kAXSizeAttribute as CFString, &size) == .success,
              let position, let size,
              CFGetTypeID(position) == AXValueGetTypeID(), CFGetTypeID(size) == AXValueGetTypeID() else { return nil }
        var point = CGPoint.zero
        var extent = CGSize.zero
        guard AXValueGetValue(position as! AXValue, .cgPoint, &point),
              AXValueGetValue(size as! AXValue, .cgSize, &extent) else { return nil }
        return CGRect(origin: point, size: extent)
    }

    /// Accessibility uses a top-left origin on the primary display.
    private static func cocoa(_ rect: CGRect) -> NSRect {
        let primary = NSScreen.screens.first?.frame.height ?? 0
        return NSRect(x: rect.minX, y: primary - rect.maxY, width: rect.width, height: rect.height)
    }

    /// Just below the anchor, left-aligned with it; above it when the screen
    /// edge leaves no room. Always fully on the anchor's screen.
    private static func origin(for size: NSSize, near anchor: NSRect) -> NSPoint {
        let center = NSPoint(x: anchor.midX, y: anchor.midY)
        let screen = NSScreen.screens.first { $0.frame.contains(center) } ?? NSScreen.main
        let gap: CGFloat = 8
        var origin = NSPoint(x: anchor.minX, y: anchor.minY - gap - size.height)
        guard let visible = screen?.visibleFrame else { return origin }
        if origin.y < visible.minY { origin.y = anchor.maxY + gap }
        origin.x = min(max(origin.x, visible.minX + gap), visible.maxX - size.width - gap)
        origin.y = min(max(origin.y, visible.minY + gap), visible.maxY - size.height - gap)
        return origin
    }

    private func makePanel() -> NSPanel {
        let panel = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 300, height: 92),
                            styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: true)
        panel.isFloatingPanel = true
        panel.level = .statusBar
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .transient]
        panel.isReleasedWhenClosed = false
        panel.hidesOnDeactivate = false
        panel.backgroundColor = .clear
        panel.isOpaque = false
        panel.hasShadow = true
        let host = NSHostingView(rootView: RemotePasteHUDView(hud: self))
        host.frame = panel.contentRect(forFrameRect: panel.frame)
        panel.contentView = host
        return panel
    }
}

private struct RemotePasteHUDView: View {
    let hud: RemotePasteHUD

    var body: some View {
        VStack(alignment: .leading, spacing: DFSpace.s) {
            HStack(spacing: DFSpace.s) {
                Image(systemName: "doc.on.clipboard").font(DFType.icon(.regular))
                    .foregroundStyle(.secondary)
                Text(hud.source.isEmpty ? "正在从其他设备粘贴" : "正在从“\(hud.source)”粘贴")
                    .font(DFType.bodyMedium).lineLimit(1)
                Spacer(minLength: 0)
                Button("取消") { hud.cancel() }
                    .buttonStyle(.dfPlain).controlSize(.small)
                    .keyboardShortcut(.cancelAction)
            }
            // Announced but no bytes yet: the size is unknown until data starts.
            if hud.total == 0 {
                ProgressView().progressViewStyle(.linear)
                Text("等待对方发送…").font(DFType.caption).foregroundStyle(.secondary)
            } else {
                ProgressView(value: hud.fraction).progressViewStyle(.linear)
                Text(verbatim: "\(byteCount(hud.received)) / \(byteCount(hud.total))")
                    .font(DFType.caption).monospacedDigit().foregroundStyle(.secondary)
            }
        }
        .padding(DFSpace.l)
        .frame(width: 300, height: 92)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: DFRadius.panel, style: .continuous))
    }

    private func byteCount(_ value: UInt64) -> String {
        ByteCountFormatter.string(fromByteCount: Int64(clamping: value), countStyle: .file)
    }
}
