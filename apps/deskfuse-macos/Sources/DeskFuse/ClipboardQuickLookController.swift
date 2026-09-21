import AppKit
import Observation
import SwiftUI
import DeskFuseEngine

/// A stable AppKit anchor for the selected SwiftUI row. The native popover uses
/// this view's bounds so its arrow points at the record being previewed.
@MainActor final class ClipboardQuickLookAnchor {
    weak var view: NSView?
}

struct ClipboardQuickLookAnchorView: NSViewRepresentable {
    let anchor: ClipboardQuickLookAnchor

    func makeNSView(context: Context) -> ClipboardQuickLookAnchorNSView {
        let view = ClipboardQuickLookAnchorNSView()
        view.anchor = anchor
        anchor.view = view
        return view
    }

    func updateNSView(_ view: ClipboardQuickLookAnchorNSView, context: Context) {
        view.anchor = anchor
        anchor.view = view
    }

    static func dismantleNSView(_ view: ClipboardQuickLookAnchorNSView, coordinator: Void) {
        if view.anchor?.view === view { view.anchor?.view = nil }
    }
}

@MainActor final class ClipboardQuickLookAnchorNSView: NSView {
    weak var anchor: ClipboardQuickLookAnchor?
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}

@MainActor @Observable final class ClipboardQuickLookModel {
    var content: ClipboardContent?
    var image: NSImage?
}

/// Owns the menu-bar clipboard's Space preview. `NSPopover` supplies the native
/// bubble, shadow, and arrow while the selected row remains the source of truth.
@MainActor final class ClipboardQuickLookController: NSObject, NSPopoverDelegate {
    static let shared = ClipboardQuickLookController()
    @ObservationIgnored private let model = ClipboardQuickLookModel()
    @ObservationIgnored private var popover: NSPopover?
    @ObservationIgnored private var hosting: NSHostingController<QuickLookHost>?
    @ObservationIgnored private var dismissHandler: (() -> Void)?
    @ObservationIgnored private var keyMonitor: Any?
    private var presented = false

    var window: NSWindow? {
        guard presented else { return nil }
        return popover?.contentViewController?.view.window
    }

    func present(from anchor: NSView, dismiss: @escaping () -> Void) {
        guard anchor.window != nil else { return }
        dismissHandler = dismiss

        let popover: NSPopover
        if let existing = self.popover {
            popover = existing
        } else {
            let hosting = NSHostingController(rootView: QuickLookHost(model: model))
            let created = NSPopover()
            created.behavior = .transient
            created.animates = true
            created.delegate = self
            created.contentViewController = hosting
            self.hosting = hosting
            self.popover = created
            popover = created
        }

        popover.contentSize = preferredContentSize(content: model.content, image: model.image)
        presented = true
        popover.show(relativeTo: anchor.bounds, of: anchor, preferredEdge: .minX)
        installKeyMonitor()
    }

    func update(content: ClipboardContent?, image: NSImage?) {
        model.content = content
        model.image = image
        guard let popover, presented else { return }
        popover.contentSize = preferredContentSize(content: content, image: image)
    }

    func close() {
        guard presented || popover?.isShown == true else { return }
        presented = false
        dismissHandler = nil
        removeKeyMonitor()
        model.content = nil
        model.image = nil
        popover?.performClose(nil)
    }

    func popoverDidClose(_ notification: Notification) {
        guard presented else { return }
        presented = false
        let dismiss = dismissHandler
        dismissHandler = nil
        removeKeyMonitor()
        model.content = nil
        model.image = nil
        dismiss?()
    }

    private func dismissAndNotify() {
        guard presented else { return }
        presented = false
        let dismiss = dismissHandler
        dismissHandler = nil
        removeKeyMonitor()
        model.content = nil
        model.image = nil
        popover?.performClose(nil)
        dismiss?()
    }

    private func installKeyMonitor() {
        guard keyMonitor == nil else { return }
        keyMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            let handled: Bool = MainActor.assumeIsolated {
                guard let self, self.presented, event.window === self.window,
                      event.keyCode == 49 || event.keyCode == 53 else { return false }
                self.dismissAndNotify()
                return true
            }
            return handled ? nil : event
        }
    }

    private func removeKeyMonitor() {
        if let keyMonitor { NSEvent.removeMonitor(keyMonitor) }
        keyMonitor = nil
    }

    private func preferredContentSize(content: ClipboardContent?, image: NSImage?) -> NSSize {
        if let image {
            let sourceWidth = max(image.size.width, 1)
            let sourceHeight = max(image.size.height, 1)
            let scale = min(1, 696 / sourceWidth, 536 / sourceHeight)
            let width = min(720, max(300, sourceWidth * scale + 24))
            let height = min(560, max(220, sourceHeight * scale + 24))
            return NSSize(width: width, height: height)
        }

        if let text = content?.text, !text.isEmpty {
            let measuredText = String(text.prefix(4_000))
            if ClipboardCodeDocument.detect(text) != nil { return NSSize(width: 640, height: 480) }
            let font = NSFont.systemFont(ofSize: 16)
            let naturalWidth = (measuredText as NSString).size(withAttributes: [.font: font]).width + 36
            let width = min(600, max(360, naturalWidth))
            let bounds = (measuredText as NSString).boundingRect(
                with: NSSize(width: width - 36, height: CGFloat.greatestFiniteMagnitude),
                options: [.usesLineFragmentOrigin, .usesFontLeading],
                attributes: [.font: font]
            )
            return NSSize(width: width, height: min(460, max(160, ceil(bounds.height) * 1.2 + 40)))
        }

        if let paths = content?.filePaths, !paths.isEmpty {
            return NSSize(width: 300, height: 116)
        }

        return NSSize(width: 220, height: 104)
    }
}

private struct QuickLookHost: View {
    @Bindable var model: ClipboardQuickLookModel

    var body: some View {
        ClipboardQuickLook(content: model.content, image: model.image)
    }
}

private struct ClipboardQuickLook: View {
    let content: ClipboardContent?
    let image: NSImage?

    var body: some View {
        Group {
            if let image {
                Image(nsImage: image)
                    .resizable()
                    .scaledToFit()
                    .padding(12)
            } else if let text = content?.text, !text.isEmpty {
                if let document = ClipboardCodeDocument.detect(text) {
                    ClipboardCodePreview(document: document)
                } else {
                ScrollView {
                    Text(text)
                        .font(.system(size: 16))
                        .lineSpacing(3)
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, 18)
                        .padding(.vertical, 14)
                }
                }
            } else if let paths = content?.filePaths, !paths.isEmpty {
                HStack(spacing: 14) {
                    Image(systemName: "doc.on.doc")
                        .font(.system(size: 28))
                        .foregroundStyle(.secondary)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(paths.count == 1 ? URL(fileURLWithPath: paths[0]).lastPathComponent : "\(paths.count) 个文件")
                            .font(.headline)
                            .lineLimit(2)
                            .truncationMode(.middle)
                        if paths.count > 1 {
                            Text(URL(fileURLWithPath: paths[0]).lastPathComponent)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                                .truncationMode(.middle)
                        }
                    }
                    Spacer(minLength: 0)
                }
                .padding(18)
            } else {
                ProgressView()
                    .controlSize(.regular)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(nsColor: .windowBackgroundColor))
    }
}
