import AppKit
import DeskFuseEngine
import XCTest
@testable import DeskFuse

@MainActor final class ClipboardPreviewStabilityTests: XCTestCase {
    private func pane(store: AppStore, text: String = "Preview", copying: Bool = false,
                      copied: Bool = false, title: String? = nil, ocrText: String? = nil,
                      localPaths: Set<String> = [], image: NSImage? = nil,
                      copy: @escaping () -> Void = {}) -> ClipboardPreviewPane {
        ClipboardPreviewPane(
            store: store,
            item: HistoryItem(id: 1, sourceAppId: nil, sourceDeviceId: nil,
                              copiedAtMs: 1, mime: "text/plain", length: 7,
                              preview: "Preview", source: "Local", hasImage: false, fileCount: 0),
            title: title,
            content: ClipboardContent(canRequestFiles: false, id: 1, text: text,
                                      truncated: false, imagePng: Data(), filePaths: [], rtf: Data()),
            media: ClipboardMedia(thumbnail: nil, localPaths: localPaths,
                                  pixelWidth: nil, pixelHeight: nil),
            image: image, ocrText: ocrText, copying: copying, copied: copied, copyContent: copy)
    }

    func testUnrelatedStoreUpdatesAndNewCopyClosureKeepPreviewEqual() {
        let store = AppStore()
        let original = pane(store: store)
        store.starting = false
        store.deviceSelectionRequest += 1
        XCTAssertEqual(original, pane(store: store, copy: { _ = store.deviceSelectionRequest }))
    }

    func testContentAndVisibleStateChangesStillInvalidatePreview() {
        let store = AppStore()
        let original = pane(store: store)
        XCTAssertNotEqual(original, pane(store: store, text: "Updated"))
        XCTAssertNotEqual(original, pane(store: store, copying: true))
        XCTAssertNotEqual(original, pane(store: store, copied: true))
        XCTAssertNotEqual(original, pane(store: store, title: "Renamed"))
        XCTAssertNotEqual(original, pane(store: store, ocrText: "Recognized"))
        XCTAssertNotEqual(original, pane(store: store, localPaths: ["/tmp/available"]))
        XCTAssertNotEqual(original, pane(store: store, image: NSImage(size: NSSize(width: 1, height: 1))))
        XCTAssertNotEqual(original, pane(store: AppStore()))
    }
}
