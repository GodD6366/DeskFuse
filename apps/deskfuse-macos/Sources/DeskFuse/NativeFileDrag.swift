import SwiftUI
import AppKit
import QuickLookThumbnailing
import DeskFuseEngine

/// AppKit reports the final drag operation. A separate engine lease outlives the shelf itself.
struct NativeFileDrag: View {
    let urls: [URL]
    let store: AppStore
    let onSuccess: ([URL]) -> Void
    let onClick: () -> Void
    var iconMaxSize: CGFloat = 80

    var body: some View {
        ZStack {
            FileStackArtwork(urls: urls, iconMaxSize: iconMaxSize)
                .allowsHitTesting(false)
            // Keep one clear native hit surface throughout the visual animation.
            // Finder drag receipts and export leases still belong to AppKit.
            NativeFileDragSurface(urls: urls, store: store, onSuccess: onSuccess, onClick: onClick)
        }
    }
}

private struct FileStackArtwork: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let urls: [URL]
    let iconMaxSize: CGFloat
    private var visibleURLs: [URL] { Array(urls.suffix(3)) }

    var body: some View {
        GeometryReader { geometry in
            let isFan = visibleURLs.count > 1
            let size = max(0, min(iconMaxSize, min(
                (geometry.size.width - 16) / (isFan ? 1.95 : 1),
                (geometry.size.height - 16) / (isFan ? 1.35 : 1)
            )))
            ZStack {
                // Identity is the URL, not the slot: existing files move smoothly
                // into their new places instead of becoming replacement images.
                ForEach(Array(visibleURLs.enumerated()), id: \.element) { index, url in
                    let isFront = index == visibleURLs.count - 1
                    let side: CGFloat = isFront ? 0 : (index == 0 ? -1 : 1)
                    let iconSize = isFront ? size : size * 0.94
                    let lift: CGFloat = isFan ? (isFront ? size * 0.04 : -size * 0.05) : 0
                    FileIconView(
                        url: url,
                        shadowOpacity: isFan ? (isFront ? 0.16 : 0.10) : 0.14,
                        shadowRadius: isFront ? 4 : 2
                    )
                    .frame(width: iconSize, height: iconSize)
                    .rotationEffect(.degrees(Double(side * 18)), anchor: .bottom)
                    .offset(x: side * size * 0.14, y: -lift)
                    .transition(fileTransition)
                    .zIndex(Double(index + 1))
                }
            }
            .frame(width: geometry.size.width, height: geometry.size.height)
            .animation(reduceMotion ? nil : .spring(response: 0.36, dampingFraction: 0.82), value: visibleURLs)
        }
    }

    private var fileTransition: AnyTransition {
        guard !reduceMotion else { return .identity }
        return .asymmetric(
            insertion: .offset(y: 14).combined(with: .scale(scale: 0.84, anchor: .bottom)).combined(with: .opacity),
            removal: .offset(y: 8).combined(with: .scale(scale: 0.90)).combined(with: .opacity)
        )
    }
}

/// The workspace icon renders synchronously but has no shadow of its own, so a
/// plain-white generic icon disappears into the glass. The fallback gets an
/// explicit shadow; the QuickLook representation (real previews, or the type
/// icon with a baked-in shadow) fades in over it once ready.
private struct FileIconView: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let url: URL
    var shadowOpacity: Double = 0.14
    var shadowRadius: CGFloat = 3
    @State private var thumbnail: NSImage?

    var body: some View {
        ZStack {
            Image(nsImage: NSWorkspace.shared.icon(forFile: url.path))
                .resizable()
                .scaledToFit()
                .shadow(color: .black.opacity(shadowOpacity), radius: shadowRadius, y: 1)
            if let thumbnail {
                Image(nsImage: thumbnail)
                    .resizable()
                    .scaledToFit()
                    .transition(.opacity)
            }
        }
        .task { await load() }
    }

    private func load() async {
        let key = FileThumbnailCache.key(for: url)
        if let cached = FileThumbnailCache.shared.image(forKey: key) {
            thumbnail = cached
            return
        }
        // .all lets the system pick the best representation: a real content
        // preview when the file has one, otherwise the shadowed type icon.
        let request = QLThumbnailGenerator.Request(
            fileAt: url,
            size: CGSize(width: 256, height: 256),
            scale: 2,
            representationTypes: .all
        )
        guard let representation = try? await QLThumbnailGenerator.shared.generateBestRepresentation(for: request) else { return }
        let image = representation.nsImage
        FileThumbnailCache.shared.set(image, forKey: key)
        withAnimation(reduceMotion ? nil : .easeOut(duration: 0.18)) {
            thumbnail = image
        }
    }
}

/// QuickLook generation goes through a system service; keep the results so the
/// stack, the expanded grid and re-shown panels don't re-request. Keys carry
/// the modification date because shelved files can still be edited in place.
@MainActor private final class FileThumbnailCache {
    static let shared = FileThumbnailCache()
    private let cache = NSCache<NSString, NSImage>()
    private init() { cache.countLimit = 256 }
    static func key(for url: URL) -> String {
        let modified = (try? url.resourceValues(forKeys: [.contentModificationDateKey]))?.contentModificationDate?.timeIntervalSince1970 ?? 0
        return "\(url.path)#\(modified)"
    }
    func image(forKey key: String) -> NSImage? { cache.object(forKey: key as NSString) }
    func set(_ image: NSImage, forKey key: String) { cache.setObject(image, forKey: key as NSString) }
}

private struct NativeFileDragSurface: NSViewRepresentable {
    let urls: [URL]
    let store: AppStore
    let onSuccess: ([URL]) -> Void
    let onClick: () -> Void
    func makeNSView(context: Context) -> FileDragView { FileDragView() }
    func updateNSView(_ view: FileDragView, context: Context) { view.urls = urls; view.store = store; view.onSuccess = onSuccess; view.onClick = onClick }
}
@MainActor final class FileDragView: NSView, NSDraggingSource {
    var urls: [URL] = []
    weak var store: AppStore?
    var onSuccess: (([URL]) -> Void)?
    var onClick: (() -> Void)?
    private var down: NSEvent?
    private var lease: ExportLease?
    private var dragged: [URL] = []
    private var moving = false
    /// The shelf panel is movable by its background; this surface must start a
    /// file drag instead of dragging the whole window out from under the pointer.
    override var mouseDownCanMoveWindow: Bool { false }
    override func mouseDown(with event: NSEvent) { down = event }
    override func mouseUp(with event: NSEvent) { if down != nil { onClick?() }; down = nil }
    override func mouseDragged(with event: NSEvent) {
        guard let down, let engine = store?.engine else { return }
        guard hypot(event.locationInWindow.x - down.locationInWindow.x, event.locationInWindow.y - down.locationInWindow.y) > 4 else { return }
        self.down = nil
        do {
            lease = try engine.acquireExport(paths: urls.map(\.path)); dragged = urls
            moving = event.modifierFlags.contains(.option)
            let items = urls.map { url in
                let item = NSDraggingItem(pasteboardWriter: url as NSURL)
                let point = convert(event.locationInWindow, from: nil)
                item.setDraggingFrame(NSRect(x: point.x - 24, y: point.y - 24, width: 48, height: 48), contents: NSWorkspace.shared.icon(forFile: url.path))
                return item
            }
            let session = beginDraggingSession(with: items, event: event, source: self)
            session.animatesToStartingPositionsOnCancelOrFail = true
        } catch { store?.error = error.userFacingMessage }
    }
    func draggingSession(_ session: NSDraggingSession, sourceOperationMaskFor context: NSDraggingContext) -> NSDragOperation { moving ? .move : .copy }
    func ignoreModifierKeys(for session: NSDraggingSession) -> Bool { true }
    func draggingSession(_ session: NSDraggingSession, endedAt screenPoint: NSPoint, operation: NSDragOperation) {
        // Releasing the stack back over the shelf is not an external copy; keep
        // the files instead of treating the self-drop as a successful drag-out.
        let droppedOnShelf = window?.frame.contains(screenPoint) ?? false
        let succeeded = !droppedOnShelf && (operation.contains(.copy) || operation.contains(.move))
        if let lease { try? store?.engine?.releaseExport(id: lease.id, outcome: succeeded ? (operation.contains(.move) ? .moved : .copied) : .cancelled) }
        lease = nil
        if succeeded { onSuccess?(dragged) }
        dragged = []
    }
}
