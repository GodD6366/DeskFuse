import AppKit
import DeskFuseEngine

/// Derived images only. Bound both decoded bytes and count, and serialize reads
/// through decode so rapid scrolling cannot retain a queue of full-size payloads.
@MainActor
final class ClipboardThumbnailCache {
    static let shared = ClipboardThumbnailCache()
    private struct Key: Hashable { let id: UInt64; let size: Int }
    private struct Entry { let image: NSImage?; let cost: Int }
    private struct Request {
        let client: EngineClient
        let sourceIsImage: Bool
        var waiters: [UUID: CheckedContinuation<NSImage?, Never>]
    }
    private var entries: [Key: Entry] = [:]
    private var order: [Key] = []
    private var cost = 0
    private let countLimit = 96
    private let costLimit = 16 * 1024 * 1024
    private var requests: [Key: Request] = [:]
    private var pending: [Key] = []
    private var worker: Task<Void, Never>?
    private var pressure: DispatchSourceMemoryPressure?

    private init() {
        let source = DispatchSource.makeMemoryPressureSource(eventMask: [.warning, .critical], queue: .main)
        source.setEventHandler { [weak self] in
            Task { @MainActor [weak self] in self?.removeCachedImages() }
        }
        source.resume()
        pressure = source
    }

    /// Outer nil means not cached; inner nil means no image exists.
    func cached(_ id: UInt64, maxPixelSize: Int = 160) -> NSImage?? {
        let key = Key(id: id, size: maxPixelSize)
        guard let entry = entries[key] else { return nil }
        touch(key)
        return .some(entry.image)
    }

    func thumbnail(for id: UInt64, client: EngineClient, maxPixelSize: Int = 160, sourceIsImage: Bool = true) async -> NSImage? {
        guard !Task.isCancelled else { return nil }
        let key = Key(id: id, size: maxPixelSize)
        if let entry = entries[key] { touch(key); return entry.image }
        let waiter = UUID()
        return await withTaskCancellationHandler {
            await withCheckedContinuation { continuation in
                guard !Task.isCancelled else { continuation.resume(returning: nil); return }
                if requests[key] != nil {
                    requests[key]?.waiters[waiter] = continuation
                } else {
                    guard requests.count < 128 else { continuation.resume(returning: nil); return }
                    requests[key] = Request(client: client, sourceIsImage: sourceIsImage, waiters: [waiter: continuation])
                    pending.append(key)
                }
                if worker == nil { worker = Task { await self.drain() } }
            }
        } onCancel: {
            Task { @MainActor in self.cancel(key, waiter: waiter) }
        }
    }

    private func cancel(_ key: Key, waiter: UUID) {
        requests[key]?.waiters.removeValue(forKey: waiter)?.resume(returning: nil)
        if requests[key]?.waiters.isEmpty == true {
            requests.removeValue(forKey: key)
            pending.removeAll { $0 == key }
        }
    }

    private func drain() async {
        defer { worker = nil }
        while !pending.isEmpty {
            let key = pending.removeFirst()
            guard let client = requests[key]?.client else { continue }
            do {
                let data: Data?
                if requests[key]?.sourceIsImage == true {
                    data = try await client.thumbnail(key.id, maxPixelSize: key.size)
                } else {
                    let content = try await client.content(key.id)
                    guard requests[key] != nil else { continue }
                    data = await ClipboardMediaLoader.shared.load(content, maxPixelSize: key.size).thumbnail
                }
                guard requests[key] != nil else { continue }
                let image = data.flatMap { NSImage(data: $0) }
                store(image, for: key)
                finish(key, image: image)
            } catch {
                // Do not permanently cache transient engine/content failures.
                finish(key, image: nil)
            }
        }
    }

    private func finish(_ key: Key, image: NSImage?) {
        guard let request = requests.removeValue(forKey: key) else { return }
        pending.removeAll { $0 == key }
        for continuation in request.waiters.values { continuation.resume(returning: image) }
    }

    private func store(_ image: NSImage?, for key: Key) {
        let bytes = image.map { image in
            image.representations.reduce(0) { $0 + max(0, $1.pixelsWide) * max(0, $1.pixelsHigh) * 4 }
        } ?? 0
        cost -= entries[key]?.cost ?? 0
        entries[key] = Entry(image: image, cost: bytes)
        cost += bytes
        touch(key)
        while order.count > countLimit || cost > costLimit, let oldest = order.first {
            order.removeFirst()
            cost -= entries.removeValue(forKey: oldest)?.cost ?? 0
        }
    }

    private func touch(_ key: Key) {
        if let index = order.firstIndex(of: key) { order.remove(at: index) }
        order.append(key)
    }

    private func removeCachedImages() {
        entries.removeAll(); order.removeAll(); cost = 0
    }
}
