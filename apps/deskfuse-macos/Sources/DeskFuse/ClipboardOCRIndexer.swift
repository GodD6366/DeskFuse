import Foundation
import UniformTypeIdentifiers
import DeskFuseEngine

struct ClipboardOCRDiagnostics: Codable, Sendable {
    let queued: Int
    let running: Bool
    let helperPID: Int32?
    let helperRunning: Bool
    let generation: UInt64
}

/// Queues image records for OCR without loading Vision into the main app.
/// The helper owns the Vision model and exits after a short idle period.
@MainActor
final class ClipboardOCRIndexer {
    static let shared = ClipboardOCRIndexer()
    static let version: UInt64 = 1
    static let automaticIndexingKey = "deskfuse.clipboard.ocr.helperAutomaticIndexing"
    static let defaultAutomaticIndexing = true

    private var pending: [UInt64] = []
    private var queued = Set<UInt64>()
    private var running = false
    private var generation: UInt64 = 0
    private var helper: OCRHelperClient?

    /// Queue a record for recognition. The engine's `ocr_version` keeps this
    /// idempotent across launches; `queued` avoids duplicate work in one run.
    func schedule(_ id: UInt64, client: EngineClient?) {
        guard let client, !queued.contains(id) else { return }
        queued.insert(id)
        pending.append(id)
        guard !running else { return }
        running = true
        Task { await drain(client: client) }
    }

    /// Stop queued work and release the helper immediately when auto indexing
    /// is disabled. The generation guard prevents an in-flight result from
    /// being written after the user cancelled the queue.
    func cancelPending() {
        generation &+= 1
        pending.removeAll(keepingCapacity: false)
        queued.removeAll(keepingCapacity: false)
        helper?.stop()
        helper = nil
    }

    func diagnostics() -> ClipboardOCRDiagnostics {
        ClipboardOCRDiagnostics(
            queued: pending.count,
            running: running,
            helperPID: helper?.processIdentifier,
            helperRunning: helper?.isRunning == true,
            generation: generation
        )
    }

    private func drain(client: EngineClient) async {
        defer { running = false }
        while !pending.isEmpty {
            let id = pending.removeFirst()
            queued.remove(id)
            let currentGeneration = generation
            guard let path = await stageImage(for: id, client: client) else { continue }
            guard currentGeneration == generation else {
                try? FileManager.default.removeItem(at: path)
                continue
            }
            let text = await recognize(id: id, path: path)
            guard currentGeneration == generation else { continue }
            // Empty text is still a finished result: record it so the record is
            // not re-recognized, while keeping the search index clean.
            ClipboardLibraryStore.shared.setOCR(id, text: text ?? "", version: Self.version)
        }
    }

    private func recognize(id: UInt64, path: URL) async -> String? {
        if helper == nil || helper?.isRunning != true {
            helper?.stop()
            guard let executableURL = Self.helperExecutableURL() else { return nil }
            helper = OCRHelperClient(executableURL: executableURL)
        }
        guard let helper else { return nil }
        let result = await helper.recognize(id: id, path: path)
        if !helper.isRunning { self.helper = nil }
        return result
    }

    private func stageImage(for id: UInt64, client: EngineClient) async -> URL? {
        guard let content = try? await client.content(id) else { return nil }
        let data: Data
        if !content.imagePng.isEmpty {
            data = content.imagePng
        } else {
            guard content.filePaths.count == 1, let path = content.filePaths.first else { return nil }
            let url = URL(fileURLWithPath: path)
            guard FileManager.default.fileExists(atPath: path),
                  UTType(filenameExtension: url.pathExtension)?.conforms(to: .image) == true else { return nil }
            guard let fileData = try? Data(contentsOf: url) else { return nil }
            data = fileData
        }
        let staged = FileManager.default.temporaryDirectory
            .appendingPathComponent("deskfuse-ocr-\(UUID().uuidString).image")
        do {
            try await Task.detached(priority: .utility) {
                try data.write(to: staged, options: .atomic)
            }.value
            return staged
        } catch {
            try? FileManager.default.removeItem(at: staged)
            return nil
        }
    }

    private static func helperExecutableURL() -> URL? {
        let executableDirectory = URL(fileURLWithPath: CommandLine.arguments[0])
            .deletingLastPathComponent()
        let candidates = [
            Bundle.main.bundleURL.appendingPathComponent("Contents/MacOS/deskfuse-ocr-helper"),
            executableDirectory.appendingPathComponent("deskfuse-ocr-helper"),
        ]
        return candidates.first { FileManager.default.isExecutableFile(atPath: $0.path) }
    }
}

private struct OCRHelperRequest: Codable {
    let id: UInt64
    let path: String
}

private struct OCRHelperResponse: Codable {
    let id: UInt64
    let text: String?
}

/// A single-request-at-a-time client. ClipboardOCRIndexer is serial, so a
/// line reader is enough and avoids retaining a response queue in the app.
private final class OCRHelperClient: @unchecked Sendable {
    private let process: Process
    private let input: FileHandle
    private let output: FileHandle
    private let stateLock = NSLock()
    private var stopped = false

    var processIdentifier: Int32? { process.isRunning ? process.processIdentifier : nil }

    var isRunning: Bool {
        stateLock.lock()
        defer { stateLock.unlock() }
        return !stopped && process.isRunning
    }

    init?(executableURL: URL) {
        let process = Process()
        let input = Pipe()
        let output = Pipe()
        process.executableURL = executableURL
        process.standardInput = input
        process.standardOutput = output
        process.standardError = FileHandle.nullDevice
        do {
            try process.run()
        } catch {
            return nil
        }
        self.process = process
        self.input = input.fileHandleForWriting
        self.output = output.fileHandleForReading
        process.terminationHandler = { [weak self] _ in self?.markStopped() }
    }

    func recognize(id: UInt64, path: URL) async -> String? {
        guard isRunning else { return nil }
        defer { try? FileManager.default.removeItem(at: path) }
        do {
            let request = try JSONEncoder().encode(OCRHelperRequest(id: id, path: path.path))
            input.write(request)
            input.write(Data([0x0A]))
        } catch {
            stop()
            return nil
        }
        guard let line = await readResponseLine(),
              let response = try? JSONDecoder().decode(OCRHelperResponse.self, from: line),
              response.id == id else {
            return nil
        }
        return response.text
    }

    func stop() {
        stateLock.lock()
        let shouldStop = !stopped
        stopped = true
        stateLock.unlock()
        guard shouldStop else { return }
        if process.isRunning { process.terminate() }
        try? input.close()
        try? output.close()
    }

    private func markStopped() {
        stateLock.lock()
        stopped = true
        stateLock.unlock()
    }

    private func readResponseLine() async -> Data? {
        await withCheckedContinuation { continuation in
            DispatchQueue.global(qos: .utility).async { [output] in
                var line = Data()
                while true {
                    let chunk = output.readData(ofLength: 4096)
                    if chunk.isEmpty {
                        continuation.resume(returning: line.isEmpty ? nil : line)
                        return
                    }
                    if let newline = chunk.firstIndex(of: 0x0A) {
                        line.append(chunk.prefix(upTo: newline))
                        continuation.resume(returning: line)
                        return
                    }
                    line.append(chunk)
                }
            }
        }
    }
}
