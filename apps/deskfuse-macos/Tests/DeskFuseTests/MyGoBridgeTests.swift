import XCTest
@testable import DeskFuse

private final class FrameCollector: @unchecked Sendable {
    private let lock = NSLock()
    private var frames: [[String: Any]] = []
    private(set) var closed = false

    func add(_ framed: Data) {
        var reader = MyGoFrameReader()
        reader.append(framed)
        guard let payload = try? reader.nextFrame(),
              let object = (try? JSONSerialization.jsonObject(with: payload)) as? [String: Any] else { return }
        lock.withLock { frames.append(object) }
    }
    func markClosed() { lock.withLock { closed = true } }
    var isClosed: Bool { lock.withLock { closed } }
    var all: [[String: Any]] { lock.withLock { frames } }

    func next(count: Int = 1, timeout: Duration = .seconds(5)) async throws -> [[String: Any]] {
        let deadline = ContinuousClock.now + timeout
        while all.count < count, ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(5)) }
        return all
    }
}

private struct FakeEngine: MyGoLibraryQuerying {
    var handler: @Sendable (String) async throws -> String
    func libraryQuery(_ request: String) async throws -> String { try await handler(request) }
}

private struct EngineFailure: LocalizedError { var errorDescription: String? { "数据库忙" } }

private func frame(_ object: [String: Any]) -> Data {
    MyGoBridgeFrame.encode(try! JSONSerialization.data(withJSONObject: object))
}

final class MyGoBridgeTests: XCTestCase {
    private func makeServer(_ collector: FrameCollector, engine: (any MyGoLibraryQuerying)?) -> MyGoBridgeServer {
        MyGoBridgeServer(engine: { engine }, send: { collector.add($0) }, onClose: { collector.markClosed() })
    }

    // MARK: framing

    func testFrameReaderReassemblesSplitAndCoalescedFrames() throws {
        let one = frame(["a": 1]), two = frame(["b": 2])
        var reader = MyGoFrameReader()
        reader.append(one.prefix(3))
        XCTAssertNil(try reader.nextFrame())
        reader.append(one.dropFirst(3) + two)
        XCTAssertEqual(try reader.nextFrame().flatMap { String(data: $0, encoding: .utf8) }, #"{"a":1}"#)
        XCTAssertEqual(try reader.nextFrame().flatMap { String(data: $0, encoding: .utf8) }, #"{"b":2}"#)
        XCTAssertNil(try reader.nextFrame())
    }

    func testFrameReaderRejectsEmptyAndOversizedHeaders() {
        var empty = MyGoFrameReader()
        empty.append(Data([0, 0, 0, 0]))
        XCTAssertThrowsError(try empty.nextFrame()) { XCTAssertEqual(($0 as? MyGoBridgeError)?.code, "bad_frame") }
        var huge = MyGoFrameReader()
        // Rejected from the header alone; the payload is never buffered.
        huge.append(Data([0x7f, 0xff, 0xff, 0xff]))
        XCTAssertThrowsError(try huge.nextFrame()) { XCTAssertEqual(($0 as? MyGoBridgeError)?.code, "frame_too_large") }
    }

    // MARK: parsing

    private func parse(_ object: [String: Any]) throws -> MyGoBridgeRequest {
        try MyGoBridgeRequestParser.parse(JSONSerialization.data(withJSONObject: object))
    }

    func testLibraryQueryStripsCallerQueryIDAndKeepsParameters() throws {
        let request = try parse(["v": 1, "id": "r1", "op": "library.query",
                                 "params": ["op": "items", "query": "hello", "limit": 50, "query_id": "attacker"]])
        guard case .libraryQuery(let json) = request.command else { return XCTFail("wrong command") }
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
        XCTAssertNil(object["query_id"])
        XCTAssertEqual(object["op"] as? String, "items")
        XCTAssertEqual(object["limit"] as? Int, 50)
    }

    func testParserRejectsInvalidRequestsWithStructuredCodes() {
        func code(_ object: [String: Any]) -> String? {
            do { _ = try parse(object); return nil } catch { return (error as? MyGoBridgeError)?.code }
        }
        XCTAssertEqual(code(["v": 2, "id": "a", "op": "hello"]), "unsupported_version")
        XCTAssertEqual(code(["v": 1, "id": "", "op": "hello"]), "invalid_request")
        XCTAssertEqual(code(["v": 1, "id": String(repeating: "x", count: 65), "op": "hello"]), "invalid_request")
        XCTAssertEqual(code(["v": 1, "id": "a", "op": "library.mutate", "params": [:]]), "invalid_params")
        XCTAssertEqual(code(["v": 1, "id": "a", "op": "library.query", "params": ["op": "cancel_query"]]), "invalid_params")
        XCTAssertEqual(code(["v": 1, "id": "a", "op": "library.query", "params": ["op": "settings"]]), "invalid_params")
        XCTAssertEqual(code(["v": 1, "id": "a", "op": "library.query"]), "invalid_params")
        XCTAssertEqual(code(["v": 1, "id": "a", "op": "library.query",
                             "params": ["op": "items", "query": String(repeating: "x", count: 1025)]]), "invalid_params")
        XCTAssertThrowsError(try MyGoBridgeRequestParser.parse(Data("[1]".utf8))) {
            XCTAssertEqual(($0 as? MyGoBridgeError)?.code, "bad_json")
        }
    }

    func testMutationsUseRealOpsRequireConfirmationAndPreserveUInt64IDs() throws {
        XCTAssertThrowsError(try parse(["v": 1, "id": "delete", "op": "library.mutate", "params": ["op": "clear_history"]])) {
            XCTAssertEqual(($0 as? MyGoBridgeError)?.code, "confirmation_required")
        }
        let request = try parse(["v": 1, "id": "edit", "op": "library.mutate",
                                 "params": ["op": "set_item_title", "id": "18446744073709551615", "title": "测试"]])
        guard case .libraryMutate(let json) = request.command else { return XCTFail("wrong command") }
        XCTAssertTrue(json.contains("18446744073709551615"))
        XCTAssertTrue(json.contains("set_item_title"))
    }

    func testInvocationCarriesNestedJSONWithoutDroppingSettings() throws {
        let request = try parse(["v": 1, "id": "invoke", "op": "app.invoke",
                                 "params": ["action": "input.setSettings", "params": ["settings": ["enabled": true]]]])
        guard case .appInvoke(let action, let data) = request.command else { return XCTFail("wrong command") }
        XCTAssertEqual(action, "input.setSettings")
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual((object["settings"] as? [String: Any])?["enabled"] as? Bool, true)
    }

    func testParserKeepsRequestIDOnFailureForCorrelation() {
        XCTAssertThrowsError(try parse(["v": 1, "id": "keep-me", "op": "nope"])) {
            XCTAssertEqual(($0 as? MyGoBridgeError)?.id, "keep-me")
        }
    }

    // MARK: server

    func testHelloAdvertisesProtocolAndLimits() async throws {
        let collector = FrameCollector()
        let server = makeServer(collector, engine: nil)
        await server.receive(frame(["v": 1, "id": "h", "op": "hello"]))
        let reply = try await collector.next()[0]
        XCTAssertEqual(reply["ok"] as? Bool, true)
        XCTAssertEqual(reply["id"] as? String, "h")
        let result = try XCTUnwrap(reply["result"] as? [String: Any])
        XCTAssertEqual(result["protocol"] as? String, "deskfuse.mygo.bridge")
        XCTAssertEqual(result["max_request_bytes"] as? Int, 262_144)
    }

    func testLibraryQueryReturnsEngineJSONVerbatim() async throws {
        let collector = FrameCollector()
        let seen = LockedBox<String>()
        let server = makeServer(collector, engine: FakeEngine { request in
            seen.value = request
            return #"{"items":[{"id":7}],"revision":3,"next_offset":null}"#
        })
        await server.receive(frame(["v": 1, "id": "q1", "op": "library.query", "params": ["op": "items", "limit": 20]]))
        let reply = try await collector.next()[0]
        XCTAssertEqual(reply["ok"] as? Bool, true)
        XCTAssertEqual(reply["id"] as? String, "q1")
        let result = try XCTUnwrap(reply["result"] as? [String: Any])
        XCTAssertEqual(result["revision"] as? Int, 3)
        XCTAssertEqual((result["items"] as? [[String: Any]])?.first?["id"] as? Int, 7)
        XCTAssertTrue(try XCTUnwrap(seen.value).contains(#""op":"items""#))
    }

    func testEngineFailureAndUnavailableEngineAreStructured() async throws {
        let failing = FrameCollector()
        let server = makeServer(failing, engine: FakeEngine { _ in throw EngineFailure() })
        await server.receive(frame(["v": 1, "id": "e1", "op": "library.query", "params": ["op": "state"]]))
        let failed = try await failing.next()[0]
        var error = try XCTUnwrap(failed["error"] as? [String: Any])
        XCTAssertEqual(error["code"] as? String, "engine_error")
        XCTAssertEqual(error["message"] as? String, "数据库忙")
        XCTAssertEqual(error["retryable"] as? Bool, false)

        let missing = FrameCollector()
        let early = makeServer(missing, engine: nil)
        await early.receive(frame(["v": 1, "id": "e2", "op": "library.query", "params": ["op": "state"]]))
        let unavailable = try await missing.next()[0]
        error = try XCTUnwrap(unavailable["error"] as? [String: Any])
        XCTAssertEqual(error["code"] as? String, "engine_unavailable")
        XCTAssertEqual(error["retryable"] as? Bool, true)
    }

    func testCancelInterruptsInFlightQuery() async throws {
        let collector = FrameCollector()
        let server = makeServer(collector, engine: FakeEngine { _ in
            try await Task.sleep(for: .seconds(30))
            return "{}"
        })
        await server.receive(frame(["v": 1, "id": "slow", "op": "library.query", "params": ["op": "items"]]))
        await server.receive(frame(["v": 1, "id": "c", "op": "cancel", "params": ["id": "slow"]]))
        let replies = try await collector.next(count: 2)
        let byID = Dictionary(uniqueKeysWithValues: replies.map { ($0["id"] as! String, $0) })
        XCTAssertEqual((byID["c"]?["result"] as? [String: Any])?["cancelled"] as? Bool, true)
        XCTAssertEqual((byID["slow"]?["error"] as? [String: Any])?["code"] as? String, "cancelled")
    }

    func testDuplicateIDAndConcurrencyLimitAreRejected() async throws {
        let collector = FrameCollector()
        let server = makeServer(collector, engine: FakeEngine { _ in
            try await Task.sleep(for: .seconds(30))
            return "{}"
        })
        for index in 0..<MyGoBridge.maxInFlight {
            await server.receive(frame(["v": 1, "id": "r\(index)", "op": "library.query", "params": ["op": "items"]]))
        }
        await server.receive(frame(["v": 1, "id": "r0", "op": "library.query", "params": ["op": "items"]]))
        await server.receive(frame(["v": 1, "id": "extra", "op": "library.query", "params": ["op": "items"]]))
        let replies = try await collector.next(count: 2)
        let codes = replies.compactMap { ($0["error"] as? [String: Any])?["code"] as? String }
        XCTAssertEqual(codes, ["duplicate_id", "busy"])
        await server.close()
        XCTAssertTrue(collector.isClosed)
    }

    func testOversizedFrameIsFatalAndClosesChannel() async throws {
        let collector = FrameCollector()
        let server = makeServer(collector, engine: nil)
        await server.receive(Data([0x00, 0x10, 0x00, 0x00]))
        let reply = try await collector.next()[0]
        XCTAssertEqual(reply["ok"] as? Bool, false)
        XCTAssertTrue(reply["id"] is NSNull)
        XCTAssertEqual((reply["error"] as? [String: Any])?["code"] as? String, "frame_too_large")
        XCTAssertTrue(collector.isClosed)
        await server.receive(frame(["v": 1, "id": "late", "op": "hello"]))
        XCTAssertEqual(collector.all.count, 1)
    }

    func testReplyRejectsNonObjectEngineResult() {
        XCTAssertThrowsError(try MyGoBridgeReply.success(id: "x", result: Data("[1]".utf8))) {
            XCTAssertEqual(($0 as? MyGoBridgeError)?.code, "internal")
        }
    }

    // MARK: opt-in activation

    func testNoFlagMeansNoBridge() throws {
        XCTAssertNil(try MyGoUIOptions.executableURL(arguments: ["/Applications/DeskFuse.app/Contents/MacOS/DeskFuse"]))
    }

    func testFlagValidation() throws {
        XCTAssertThrowsError(try MyGoUIOptions.executableURL(arguments: ["app", "--mygo-ui"])) {
            XCTAssertEqual($0 as? MyGoUIOptions.Failure, .missingPath)
        }
        XCTAssertThrowsError(try MyGoUIOptions.executableURL(arguments: ["app", "--mygo-ui", "relative/ui"])) {
            XCTAssertEqual($0 as? MyGoUIOptions.Failure, .notAbsolute("relative/ui"))
        }
        XCTAssertThrowsError(try MyGoUIOptions.executableURL(arguments: ["app", "--mygo-ui", "/nonexistent/mygo"])) {
            XCTAssertEqual($0 as? MyGoUIOptions.Failure, .notExecutable("/nonexistent/mygo"))
        }
        XCTAssertEqual(try MyGoUIOptions.executableURL(arguments: ["app", "--mygo-ui", "/usr/bin/true"])?.path, "/usr/bin/true")
        XCTAssertEqual(try MyGoUIOptions.executableURL(arguments: ["app", "--mygo-ui=/usr/bin/true"])?.path, "/usr/bin/true")
    }
}

private final class LockedBox<T>: @unchecked Sendable {
    private let lock = NSLock()
    private var stored: T?
    var value: T? {
        get { lock.withLock { stored } }
        set { lock.withLock { stored = newValue } }
    }
}
