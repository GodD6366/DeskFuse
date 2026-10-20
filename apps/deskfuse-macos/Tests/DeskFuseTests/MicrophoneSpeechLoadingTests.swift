import Foundation
import XCTest
@testable import DeskFuse

@MainActor final class MicrophoneSpeechLoadingTests: XCTestCase {
    func testSlowSystemPreparationDoesNotBlockMainActor() async {
        let tester = MicrophoneSpeechTester()
        let started = expectation(description: "Background preparation started")
        let release = DispatchSemaphore(value: 0)
        let loading = Task { @MainActor in
            await tester.prepare {
                XCTAssertFalse(Thread.isMainThread, "System initialization must not run on the UI thread")
                started.fulfill()
                _ = release.wait(timeout: .now() + 3)
                return MicrophoneSpeechPreparation(recognizer: nil, inputName: "测试麦克风")
            }
        }
        await fulfillment(of: [started], timeout: 2)
        // This executes while the system query is still blocked.
        XCTAssertEqual(tester.phase, .loading)
        tester.text = "加载期间仍可编辑"
        tester.toggle()
        XCTAssertEqual(tester.phase, .loading)
        release.signal()
        await loading.value
        XCTAssertEqual(tester.phase, .idle)
        XCTAssertEqual(tester.text, "加载期间仍可编辑")
        XCTAssertEqual(tester.inputName, "测试麦克风")
    }

    func testLeavingDuringPreparationIgnoresLateResultAndReusesWorkOnReturn() async {
        let tester = MicrophoneSpeechTester()
        let started = expectation(description: "Background preparation started")
        let release = DispatchSemaphore(value: 0)
        let loading = Task { @MainActor in
            await tester.prepare {
                started.fulfill()
                _ = release.wait(timeout: .now() + 3)
                return MicrophoneSpeechPreparation(recognizer: nil, inputName: "测试麦克风")
            }
        }
        await fulfillment(of: [started], timeout: 2)
        loading.cancel()
        tester.teardown()
        release.signal()
        await loading.value
        XCTAssertEqual(tester.phase, .loading)
        XCTAssertNil(tester.inputName)

        await tester.prepare {
            XCTFail("Returning to the page must reuse the in-flight system query")
            return MicrophoneSpeechPreparation(recognizer: nil, inputName: nil)
        }
        XCTAssertEqual(tester.phase, .idle)
        XCTAssertEqual(tester.inputName, "测试麦克风")
    }
}
