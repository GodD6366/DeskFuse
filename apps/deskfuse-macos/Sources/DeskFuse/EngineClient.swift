import Foundation
import DeskFuseEngine

/// Native callbacks only enqueue values. Progress snapshots are replaceable; notifications are not.
/// Mutable state is protected by `lock`, and no UI or Rust calls occur while holding it.
final class EventRelay: EngineListener, @unchecked Sendable {
    private let continuation: AsyncStream<Void>.Continuation
    private let lock = NSLock()
    private var latestSnapshot: EngineSnapshot?
    private var notifications: [EngineEvent] = []

    init(_ continuation: AsyncStream<Void>.Continuation) { self.continuation = continuation }

    func onEvent(event: EngineEvent) {
        lock.withLock {
            switch event {
            case .snapshot(let snapshot):
                if snapshot.revision >= (latestSnapshot?.revision ?? 0) {
                    latestSnapshot = snapshot
                }
            default:
                notifications.append(event)
            }
        }
        // The stream buffers only a wake-up, never a stale copy of a snapshot.
        continuation.yield(())
    }

    func takePendingEvents() -> [EngineEvent] {
        lock.withLock {
            var events: [EngineEvent] = []
            events.reserveCapacity(notifications.count + 1)
            // FFI emits the snapshot before its notification. Draining both under the
            // same lock guarantees notifications see that snapshot or a newer one.
            if let latestSnapshot { events.append(.snapshot(snapshot: latestSnapshot)) }
            latestSnapshot = nil
            events.append(contentsOf: notifications)
            notifications.removeAll(keepingCapacity: true)
            return events
        }
    }

    deinit { continuation.finish() }
}

actor EngineClient {
    private let engine: Engine
    private let clipboardMedia: ClipboardMediaClient
    init(engine: Engine) {
        self.engine = engine
        self.clipboardMedia = ClipboardMediaClient(engine: engine)
    }
    func listen(_ relay: EventRelay) throws { try engine.subscribe(listener: relay) }
    func submit(_ command: EngineCommand) throws -> CommandReceipt { try engine.submit(command: command) }
    func search(_ query: String, filter: UInt8) throws -> [UInt64] { try engine.searchHistory(query: query, filter: filter) }
    func libraryQuery(_ request: String) throws -> String {
        try Task.checkCancellation()
        return try engine.libraryQuery(request: request)
    }
    func libraryMutate(_ request: String) throws -> String { try engine.libraryMutate(request: request) }
    func lanWebState() throws -> String { try engine.lanWebState() }
    func configureLanWeb(enabled: Bool, interfaceName: String?, port: UInt16) throws -> String {
        try engine.configureLanWeb(enabled: enabled, interfaceName: interfaceName, port: port)
    }
    func prepareClipboardPaste(_ id: UInt64, session: String) async throws -> Int64 {
        let engine = self.engine
        return try await Task.detached { try engine.prepareClipboardPaste(id: id, session: session) }.value
    }
    func cancelClipboardPreparation(_ session: String) throws { try engine.cancelClipboardPreparation(session: session) }
    func thumbnail(_ id: UInt64, maxPixelSize: Int) async throws -> Data {
        try await clipboardMedia.thumbnail(id, maxPixelSize: maxPixelSize)
    }
    func content(_ id: UInt64) async throws -> ClipboardContent {
        try await clipboardMedia.content(id)
    }
    func exportDiagnostics(_ path: String) throws { try engine.exportDiagnostics(path: path) }
    func shutdown() throws { try engine.shutdown() }
}

/// Keep native image decoding serialized and off the query/command actor.
/// Awaiting this actor releases EngineClient so tabs and paging can respond
/// while a thumbnail, preview or OCR input is being decoded.
private actor ClipboardMediaClient {
    private let engine: Engine
    init(engine: Engine) { self.engine = engine }

    func thumbnail(_ id: UInt64, maxPixelSize: Int) throws -> Data {
        try Task.checkCancellation()
        return try engine.clipboardThumbnail(id: id, maxPixelSize: UInt32(max(64, min(maxPixelSize, 480))))
    }

    func content(_ id: UInt64) throws -> ClipboardContent {
        try Task.checkCancellation()
        return try engine.content(id: id)
    }
}

extension Error {
    var userFacingMessage: String {
        if let engineError = self as? EngineError {
            switch engineError { case .Failed(let message): return message }
        }
        return localizedDescription
    }
}
