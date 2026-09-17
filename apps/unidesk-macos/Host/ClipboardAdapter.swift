import AppKit
import Foundation
import os

struct NativeRevision: Equatable {
    var value: Int
}

enum CaptureSource {
    case user
    case remote
    case unknown
}

struct CapturedContent {
    var revision: NativeRevision
    var types: [NSPasteboard.PasteboardType]
    var text: String?
    var source: CaptureSource
}

enum ApplyResult {
    case applied(newRevision: NativeRevision)
    case superseded(currentRevision: NativeRevision)
    case rejected(reason: String)
}

final class MacOSClipboardCapture: @unchecked Sendable {
    private let pasteboard = NSPasteboard.general
    private var lastChangeCount: Int = 0
    private let remoteWrite = OSAllocatedUnfairLock(initialState: false)

    func startMonitoring() async throws {
        lastChangeCount = pasteboard.changeCount
    }

    func stopMonitoring() async throws {}

    func markAsRemoteWrite() {
        remoteWrite.withLock { $0 = true }
    }

    func capture() async throws -> CapturedContent {
        let changeCount = pasteboard.changeCount
        let isRemote = remoteWrite.withLock { flag in
            let current = flag
            flag = false
            return current
        }
        return CapturedContent(
            revision: NativeRevision(value: changeCount),
            types: pasteboard.types ?? [],
            text: pasteboard.string(forType: .string),
            source: isRemote ? .remote : .user
        )
    }
}

final class MacOSClipboardApply: @unchecked Sendable {
    private let pasteboard = NSPasteboard.general
    weak var capture: MacOSClipboardCapture?

    func apply(text: String, expectedRevision: NativeRevision) async throws -> ApplyResult {
        let currentCount = pasteboard.changeCount
        if currentCount != expectedRevision.value {
            return .superseded(currentRevision: NativeRevision(value: currentCount))
        }
        pasteboard.clearContents()
        let ok = pasteboard.setString(text, forType: .string)
        if ok {
            capture?.markAsRemoteWrite()
            return .applied(newRevision: NativeRevision(value: pasteboard.changeCount))
        }
        return .rejected(reason: "NSPasteboard write failed")
    }
}
