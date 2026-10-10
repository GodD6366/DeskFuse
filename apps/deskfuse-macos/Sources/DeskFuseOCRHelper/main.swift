import Foundation
import ImageIO
import Vision
import Darwin

struct OCRHelperRequest: Codable {
    let id: UInt64
    let path: String
}

struct OCRHelperResponse: Codable {
    let id: UInt64
    var text: String?
}

final class IdleState: @unchecked Sendable {
    private let lock = NSLock()
    private var lastActivity = Date()
    private var busy = false

    func begin() {
        lock.lock()
        busy = true
        lastActivity = Date()
        lock.unlock()
    }

    func end() {
        lock.lock()
        busy = false
        lastActivity = Date()
        lock.unlock()
    }

    var shouldExit: Bool {
        lock.lock()
        defer { lock.unlock() }
        return !busy && Date().timeIntervalSince(lastActivity) >= 5
    }
}

let idleState = IdleState()
DispatchQueue.global(qos: .utility).async {
    while true {
        Thread.sleep(forTimeInterval: 1)
        if idleState.shouldExit { exit(0) }
    }
}

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

func writeResponse(_ response: OCRHelperResponse) {
    guard let encoded = try? JSONEncoder().encode(response) else { return }
    FileHandle.standardOutput.write(encoded)
    FileHandle.standardOutput.write(Data([0x0A]))
}

while let line = readLine(strippingNewline: true) {
    idleState.begin()
    defer { idleState.end() }
    guard let request = try? JSONDecoder().decode(OCRHelperRequest.self, from: Data(line.utf8)) else { continue }
    var text: String?
    autoreleasepool {
        text = (try? Data(contentsOf: URL(fileURLWithPath: request.path))).flatMap(recognize)
    }
    try? FileManager.default.removeItem(atPath: request.path)
    writeResponse(OCRHelperResponse(id: request.id, text: text))
}
