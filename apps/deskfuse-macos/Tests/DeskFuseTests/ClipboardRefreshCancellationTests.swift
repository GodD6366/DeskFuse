import Foundation
import XCTest
@testable import DeskFuse

@MainActor final class ClipboardRefreshCancellationTests: XCTestCase {
    private struct RefreshFailure: LocalizedError {
        var errorDescription: String? { "读取剪贴板失败" }
    }

    func testCancellationDoesNotReplaceExistingError() async {
        let store = ClipboardLibraryStore()
        store.error = "已有错误"
        await store.refresh { throw CancellationError() }
        XCTAssertEqual(store.error, "已有错误")
    }

    func testCancelledRefreshDoesNotPublishLateFailure() async {
        let store = ClipboardLibraryStore()
        let task = Task { @MainActor in
            await store.refresh {
                withUnsafeCurrentTask { $0?.cancel() }
                throw RefreshFailure()
            }
        }
        await task.value
        XCTAssertNil(store.error)
    }

    func testRealRefreshFailureRemainsVisible() async {
        let store = ClipboardLibraryStore()
        await store.refresh { throw RefreshFailure() }
        XCTAssertEqual(store.error, "读取剪贴板失败")
    }
}
