import Foundation
import AppKit
import Vision
import ImageIO
import UniformTypeIdentifiers
import DeskFuseEngine

/// Runs on-device Vision OCR for image records and writes the result back to the
/// engine for search. One task per record; results never merge or deduplicate
/// records — records are still deduplicated only by clipboard content digest.
@MainActor
final class ClipboardOCRIndexer {
    static let shared = ClipboardOCRIndexer()
    /// Bump when the recognizer changes so older results are re-run.
    static let version: UInt64 = 1

    private var pending: [UInt64] = []
    private var queued = Set<UInt64>()
    private var running = false

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

    private func drain(client: EngineClient) async {
        defer { running = false }
        while !pending.isEmpty {
            let id = pending.removeFirst()
            queued.remove(id)
            guard let data = await imageData(for: id, client: client) else { continue }
            let text = await ClipboardOCRRecognizer.shared.recognize(data)
            // Empty text is still a finished result: record it so the record is
            // not re-recognized, while keeping the search index clean.
            ClipboardLibraryStore.shared.setOCR(id, text: text ?? "", version: Self.version)
        }
    }

    private func imageData(for id: UInt64, client: EngineClient) async -> Data? {
        guard let content = try? await client.content(id) else { return nil }
        if !content.imagePng.isEmpty { return content.imagePng }
        guard content.filePaths.count == 1, let path = content.filePaths.first else { return nil }
        let url = URL(fileURLWithPath: path)
        guard FileManager.default.fileExists(atPath: path),
              UTType(filenameExtension: url.pathExtension)?.conforms(to: .image) == true else { return nil }
        return try? Data(contentsOf: url)
    }
}

/// Vision text recognition, serialized off the main actor. Fails closed: an
/// unusable image returns nil and is retried on a later pass.
actor ClipboardOCRRecognizer {
    static let shared = ClipboardOCRRecognizer()

    func recognize(_ data: Data) -> String? {
        guard !data.isEmpty,
              let source = CGImageSourceCreateWithData(data as CFData, [kCGImageSourceShouldCache: false] as CFDictionary),
              let image = CGImageSourceCreateImageAtIndex(source, 0, nil) else { return nil }
        let request = VNRecognizeTextRequest()
        request.recognitionLevel = .accurate
        request.usesLanguageCorrection = true
        request.recognitionLanguages = ["zh-Hans", "en-US"]
        let handler = VNImageRequestHandler(cgImage: image, options: [:])
        do {
            try handler.perform([request])
        } catch {
            return nil
        }
        let lines = (request.results ?? []).compactMap { $0.topCandidates(1).first?.string }
        return lines.joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines)
    }
}
