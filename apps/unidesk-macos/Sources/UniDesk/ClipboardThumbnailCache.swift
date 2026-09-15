import AppKit
import UniDeskEngine

/// Row and gallery thumbnails are derived preview data, not business state.
/// History records are immutable, so caching by record id and target size keeps
/// scrolling, grid/list switching, and smart-list changes from re-reading and
/// re-decoding the same images through the engine.
@MainActor
final class ClipboardThumbnailCache {
    static let shared = ClipboardThumbnailCache()

    private struct Key: Hashable { let id: UInt64; let size: Int }
    private struct Entry { let image: NSImage? }
    private var entries: [Key: Entry] = [:]
    private var inFlight: [Key: Task<NSImage?, Never>] = [:]
    private var order: [Key] = []
    private let limit = 300

    /// Outer optional: `nil` means "not cached"; inner `nil` means "no thumbnail".
    func cached(_ id: UInt64, maxPixelSize: Int = 160) -> NSImage?? {
        let key = Key(id: id, size: maxPixelSize)
        guard let entry = entries[key] else { return nil }
        touch(key)
        return entry.image
    }

    func thumbnail(for id: UInt64, client: EngineClient, maxPixelSize: Int = 160) async -> NSImage? {
        let key = Key(id: id, size: maxPixelSize)
        if let entry = entries[key] { touch(key); return entry.image }
        if let existing = inFlight[key] { return await existing.value }
        // Deduplicate concurrent requests for the same record and size so a fast
        // tab switch never issues a second engine read.
        let task = Task<NSImage?, Never> { [weak client] in
            guard let client else { return nil }
            do {
                let content = try await client.content(id)
                guard !Task.isCancelled else { return nil }
                let media = await ClipboardMediaLoader.shared.load(content, maxPixelSize: maxPixelSize)
                return media.thumbnail.flatMap { NSImage(data: $0) }
            } catch {
                // A missing thumbnail must not hide the record or block copying.
                return nil
            }
        }
        inFlight[key] = task
        let image = await task.value
        inFlight[key] = nil
        store(image, for: key)
        return image
    }

    private func store(_ image: NSImage?, for key: Key) {
        entries[key] = Entry(image: image)
        touch(key)
        while order.count > limit, let oldest = order.first {
            order.removeFirst()
            entries.removeValue(forKey: oldest)
        }
    }

    private func touch(_ key: Key) {
        if let index = order.firstIndex(of: key) { order.remove(at: index) }
        order.append(key)
    }
}
