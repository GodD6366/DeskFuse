import AppKit
import Observation
import SwiftUI
import DeskFuseEngine

extension NSView {
    /// On macOS with unclipped views, visibleRect can extend beyond bounds.
    /// Anchoring/hit testing must use only the visible part of this row.
    var clipboardPreviewVisibleRect: NSRect { visibleRect.intersection(bounds) }
}

/// A stable AppKit anchor for the selected SwiftUI row. The native popover uses
/// this view's bounds so its arrow points at the record being previewed.
@MainActor final class ClipboardQuickLookAnchor {
    weak var view: NSView?
}

struct ClipboardQuickLookAnchorView: NSViewRepresentable {
    let anchor: ClipboardQuickLookAnchor
    /// The record this row renders. Re-anchoring must wait until the incoming
    /// row's view exists, never present onto the outgoing row's leftover view.
    var itemID: UInt64? = nil

    func makeNSView(context: Context) -> ClipboardQuickLookAnchorNSView {
        let view = ClipboardQuickLookAnchorNSView()
        view.anchor = anchor
        view.itemID = itemID
        anchor.view = view
        return view
    }

    func updateNSView(_ view: ClipboardQuickLookAnchorNSView, context: Context) {
        view.anchor = anchor
        view.itemID = itemID
        anchor.view = view
    }

    static func dismantleNSView(_ view: ClipboardQuickLookAnchorNSView, coordinator: Void) {
        if view.anchor?.view === view { view.anchor?.view = nil }
    }
}

@MainActor final class ClipboardQuickLookAnchorNSView: NSView {
    weak var anchor: ClipboardQuickLookAnchor?
    var itemID: UInt64?
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
    /// Arrow keys land in the popover's window, not the panel's, so the open
    /// bubble routes them back to the list for record-to-record switching.
    @ObservationIgnored private var navigateHandler: ((UInt16) -> Void)?
    @ObservationIgnored private var keyMonitor: Any?
    /// 同一条记录的后续内容更新不再改气泡尺寸，避免加载完成后撑开并盖住光标。
    @ObservationIgnored private var lockedSize = false
    @ObservationIgnored private var closeRequested = false
    @ObservationIgnored private weak var positioningView: NSView?
    private var presented = false
    private var presentedItemID: UInt64?

    var window: NSWindow? {
        guard presented else { return nil }
        return popover?.contentViewController?.view.window
    }

    var isShown: Bool { presented && popover?.isShown == true }

    func isAnchored(to view: NSView) -> Bool { isShown && positioningView === view }

    /// Re-calling `show` on a visible popover repositions it onto the new
    /// anchor without closing, so switching records never blinks the bubble.
    func present(itemID: UInt64, from anchor: NSView, lockSize: Bool = true,
                 navigate: ((UInt16) -> Void)? = nil,
                 dismiss: @escaping () -> Void) {
        guard let parent = anchor.window, let container = parent.contentView else { return }
        dismissHandler = dismiss
        navigateHandler = navigate

        let popover: NSPopover
        if let existing = self.popover {
            popover = existing
        } else {
            let hosting = NSHostingController(rootView: QuickLookHost(model: model))
            // 尺寸只由 preferredContentSize 决定。SwiftUI 的无限 frame 不能把气泡撑成空白大卡。
            hosting.sizingOptions = []
            let created = NSPopover()
            // 面板本身已是 popover。再用 transient 会在悬停移动时被系统关掉并立刻重开。
            created.behavior = .applicationDefined
            created.animates = false
            created.delegate = self
            created.contentViewController = hosting
            self.hosting = hosting
            self.popover = created
            popover = created
        }

        guard Self.displayable(content: model.content, image: model.image) else { return }
        let switchingAnchor = presentedItemID != itemID || positioningView !== anchor || !popover.isShown
        if switchingAnchor { lockedSize = false }
        if !lockedSize {
            applyContentSize(preferredContentSize(content: model.content, image: model.image), to: popover)
            lockedSize = lockSize
        }
        positioningView = anchor
        presentedItemID = itemID
        presented = true
        // SwiftUI can reuse the same NSView for a different selected record.
        // Position must update even when the view identity has not changed.
        if !popover.isShown {
            let parentWasKey = parent.isKeyWindow
            // 行锚点会在选择变化 / LazyVStack 回收时销毁，不能作为原生气泡的
            // positioningView。用稳定的面板根视图承载，仅把箭头矩形换到当前行。
            let rect = anchor.convert(anchor.clipboardPreviewVisibleRect, to: container)
            popover.show(relativeTo: rect, of: container, preferredEdge: .minX)
            // 气泡不要抢走面板的键焦点，否则悬停跟踪和方向键都会跳到另一个窗口。
            if parentWasKey { parent.makeKey() }
        }
        refreshPosition()
        installKeyMonitor()
    }

    func refreshPosition() {
        guard let popover, popover.isShown, let anchor = positioningView,
              let container = anchor.window?.contentView, !anchor.clipboardPreviewVisibleRect.isEmpty else { return }
        popover.positioningRect = anchor.convert(anchor.clipboardPreviewVisibleRect, to: container)
    }

    func update(content: ClipboardContent?, image: NSImage?) {
        guard Self.displayable(content: content, image: image) else { return }
        model.content = content
        model.image = image
        guard let popover, presented, !lockedSize else { return }
        applyContentSize(preferredContentSize(content: content, image: image), to: popover)
        lockedSize = true
    }

    /// 气泡尺寸以这里为准。Hosting 视图若按 SwiftUI 理想尺寸自己长，会盖住正在悬停的行。
    private func applyContentSize(_ size: NSSize, to popover: NSPopover) {
        popover.contentSize = size
        hosting?.view.frame.size = size
    }

    func close() {
        guard presented || popover?.isShown == true else { return }
        relinquish()
        hidePopover()
        clearModel()
    }

    /// `close()` 仍会询问代理。悬停和点按列表不能把气泡关掉，只有这里放行。
    func popoverShouldClose(_ popover: NSPopover) -> Bool { closeRequested }

    private func hidePopover() {
        guard let popover else { return }
        closeRequested = true
        popover.close()
        closeRequested = false
    }

    func popoverDidClose(_ notification: Notification) {
        guard presented else { return }
        let dismiss = dismissHandler
        relinquish()
        clearModel()
        dismiss?()
    }

    private func relinquish() {
        presented = false
        lockedSize = false
        positioningView = nil
        presentedItemID = nil
        dismissHandler = nil
        navigateHandler = nil
        removeKeyMonitor()
    }

    private func clearModel() {
        model.content = nil
        model.image = nil
    }

    private static func displayable(content: ClipboardContent?, image: NSImage?) -> Bool {
        if image != nil { return true }
        if let text = content?.text, !text.isEmpty { return true }
        if let paths = content?.filePaths, !paths.isEmpty { return true }
        return false
    }

    private func dismissAndNotify() {
        guard presented else { return }
        let dismiss = dismissHandler
        relinquish()
        hidePopover()
        clearModel()
        dismiss?()
    }

    private func installKeyMonitor() {
        guard keyMonitor == nil else { return }
        keyMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            let handled: Bool = MainActor.assumeIsolated {
                guard let self, self.presented, event.window === self.window else { return false }
                switch event.keyCode {
                case 49:
                    // 空格打开的预览再按空格关闭。
                    self.dismissAndNotify()
                    return true
                case 53:
                    self.dismissAndNotify()
                    return true
                case 123, 124, 125, 126:
                    // Modified arrows still belong to the preview's own content.
                    guard let navigate = self.navigateHandler,
                          event.modifierFlags.intersection([.command, .control, .option, .shift]).isEmpty else { return false }
                    navigate(event.keyCode)
                    return true
                default:
                    return false
                }
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
                        .font(DFType.title)
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
                        .font(DFType.metric)
                        .foregroundStyle(.secondary)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(paths.count == 1 ? URL(fileURLWithPath: paths[0]).lastPathComponent : "\(paths.count) 个文件")
                            .font(DFType.section)
                            .lineLimit(2)
                            .truncationMode(.middle)
                        if paths.count > 1 {
                            Text(URL(fileURLWithPath: paths[0]).lastPathComponent)
                                .font(DFType.caption)
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                                .truncationMode(.middle)
                        }
                    }
                    Spacer(minLength: 0)
                }
                .padding(18)
            } else {
                Color.clear
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(nsColor: .windowBackgroundColor))
    }
}
