import Foundation
import UniDeskEngine

/// No callback performs UI work or calls back into Rust. AsyncStream preserves callback order.
final class EventRelay: EngineListener, Sendable {
    let continuation: AsyncStream<EngineEvent>.Continuation
    init(_ continuation: AsyncStream<EngineEvent>.Continuation) { self.continuation = continuation }
    func onEvent(event: EngineEvent) { continuation.yield(event) }
}

actor EngineClient {
    private let engine: Engine
    init(engine: Engine) { self.engine = engine }
    func listen(_ relay: EventRelay) throws { try engine.subscribe(listener: relay) }
    func submit(_ command: EngineCommand) throws -> CommandReceipt { try engine.submit(command: command) }
    func search(_ query: String, filter: UInt8) throws -> [UInt64] { try engine.searchHistory(query: query, filter: filter) }
    func content(_ id: UInt64) throws -> ClipboardContent { try engine.content(id: id) }
    func exportDiagnostics(_ path: String) throws { try engine.exportDiagnostics(path: path) }
    func shutdown() throws { try engine.shutdown() }
}

extension Error {
    var userFacingMessage: String {
        if let engineError = self as? EngineError {
            switch engineError { case .Failed(let message): return message }
        }
        return localizedDescription
    }
}
