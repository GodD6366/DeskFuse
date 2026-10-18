import XCTest
@testable import DeskFuse

final class LogitechHIDProtocolTests: XCTestCase {
    func testOfflineProbeBackoffIsBoundedAndResettable() {
        var retry = LogitechProbeBackoff()
        XCTAssertEqual(retry.nextAttempt, 0)
        for (attempt, delay) in [30.0, 60, 60, 60].enumerated() {
            retry.failed(at: 1_000)
            XCTAssertEqual(retry.nextAttempt, 1_000 + delay)
            XCTAssertEqual(retry.failures, min(attempt + 1, 2))
        }
        retry = LogitechProbeBackoff()
        XCTAssertEqual(retry.nextAttempt, 0)
        XCTAssertEqual(retry.failures, 0)
    }
    func testRejectsMouseVendorInterfaceWithoutHIDPlusPlusReports() {
        let longReport: [UInt8] = [0x85, 0x11, 0x75, 8, 0x95, 19, 0x81, 0, 0x91, 0]
        XCTAssertTrue(LogitechHIDProtocol.hasLongReports(longReport))
        XCTAssertFalse(LogitechHIDProtocol.hasLongReports([0x85, 2, 0x75, 8, 0x95, 8, 0x81, 0, 0x91, 0]))
        XCTAssertFalse(LogitechHIDProtocol.hasLongReports(Array(longReport.dropLast(2))))
        XCTAssertFalse(LogitechHIDProtocol.hasLongReports([0x85]))
        XCTAssertFalse(LogitechHIDProtocol.hasLongReports([0xfe, 20, 0]))
    }
    func testBothProtocolErrorFramesAreMatchedWithoutWaitingForTimeout() {
        XCTAssertEqual(LogitechHIDProtocol.error([1, 0x8f, 0, 0x0f, 9, 0], matching: [1, 0, 0x0f]), 9)
        XCTAssertEqual(LogitechHIDProtocol.error([0, 0xff, 8, 0x31, 2, 0], matching: [0, 8, 0x31]), 2)
        XCTAssertNil(LogitechHIDProtocol.error([1, 0x8f, 0, 0x0e, 9, 0], matching: [1, 0, 0x0f]))
        XCTAssertNil(LogitechHIDProtocol.error([1, 0x8f, 0, 0x0f, 9, 0], matching: [0, 0, 0x0f]))
    }
    func testOnlySupportedDivertablePhysicalControls() {
        XCTAssertEqual(LogitechHIDProtocol.control([0, 0xc3, 0, 0, 0x20, 0, 0, 0]), 195)
        XCTAssertEqual(LogitechHIDProtocol.control([0, 0xc4, 0, 0, 0x60, 0, 0, 0, 1]), 196)
        XCTAssertNil(LogitechHIDProtocol.control([0, 0xc3, 0, 0, 0x10, 0, 0, 0]))
        XCTAssertNil(LogitechHIDProtocol.control([0, 0xc3, 0, 0, 0xa0, 0, 0, 0]))
        XCTAssertNil(LogitechHIDProtocol.control([0, 0x52, 0, 0, 0x20, 0, 0, 0]))
        XCTAssertNil(LogitechHIDProtocol.control([0, 0xc3]))
    }
    func testRestoreUsesValidBitWithoutPersistentOrRawXYChanges() {
        XCTAssertEqual(LogitechHIDProtocol.reporting(195, divert: true), [0, 0xc3, 3, 0, 0])
        XCTAssertEqual(LogitechHIDProtocol.reporting(196, divert: false), [0, 0xc4, 2, 0, 0])
    }
    func testPressedSnapshotIncludesReleaseAndMultipleButtons() {
        XCTAssertEqual(LogitechHIDProtocol.pressed([0, 0xc3, 0, 0xc4, 0, 0, 0, 0]), [195, 196])
        XCTAssertEqual(LogitechHIDProtocol.pressed([UInt8](repeating: 0, count: 16)), [])
        XCTAssertNil(LogitechHIDProtocol.pressed([0, 0xc3]))
    }
    func testReportIDFraming() {
        let body: [UInt8] = [0xff, 9, 0] + Array(repeating: 0, count: 16)
        XCTAssertEqual(LogitechHIDProtocol.report([0x11] + body, reportID: 0x11), body)
        XCTAssertEqual(LogitechHIDProtocol.report(body, reportID: 0x11), body)
        XCTAssertNil(LogitechHIDProtocol.report([0x10] + body, reportID: 0x11))
        XCTAssertNil(LogitechHIDProtocol.report(body, reportID: 2))
        XCTAssertNil(LogitechHIDProtocol.report([0x11, 0xff], reportID: 0x11))
    }
    func testMXModelGateAndSyncedSemanticIDs() throws {
        XCTAssertTrue(LogitechHIDProtocol.supportedModel("MX Master 3S"))
        XCTAssertTrue(LogitechHIDProtocol.supportedModel("MX Master 3 for Mac"))
        XCTAssertFalse(LogitechHIDProtocol.supportedModel("G502 HERO"))
        XCTAssertTrue(LogitechHIDProtocol.supportedModel("G502 LIGHTSPEED Wireless Gaming Mouse"))
        XCTAssertFalse(LogitechHIDProtocol.supportedModel("G502 X LIGHTSPEED"))
        var settings = MouseSettings(enabled: true)
        settings.set(button: 195, modifiers: 0, action: .shortcut(126, 0x40000))
        settings.set(button: 196, modifiers: 0, action: .disabled)
        let decoded = try JSONDecoder().decode(MouseSettings.self, from: JSONEncoder().encode(settings))
        XCTAssertEqual(decoded.action(button: 195, modifiers: 0), .shortcut(126, 0x40000))
        XCTAssertEqual(decoded.action(button: 196, modifiers: 0), .disabled)
        XCTAssertNil(decoded.action(button: 5, modifiers: 0))
    }

    func testG502BitmapUsesBothBytesAndPhysicalNamespace() throws {
        // Real receiver: slot 1, feature 0x0a, 11 buttons. Feature indices are discovered.
        XCTAssertEqual(LogitechHIDProtocol.spyButtons(count: 11), Set(208...218))
        XCTAssertEqual(LogitechHIDProtocol.spyPressed([0, 4], count: 11), [210])
        XCTAssertEqual(LogitechHIDProtocol.spyPressed([5, 0x84], count: 11), [210, 215, 216, 218])
        XCTAssertEqual(LogitechHIDProtocol.spyPressed([0, 0], count: 11), [])
        XCTAssertEqual(LogitechHIDProtocol.spyPressed([0xff, 0xff], count: 11), Set(208...218))
        XCTAssertNil(LogitechHIDProtocol.spyPressed([0], count: 11))
        XCTAssertNil(LogitechHIDProtocol.spyPressed([0, 1], count: 0))
        XCTAssertNil(LogitechHIDProtocol.spyButtons(count: 17))
        XCTAssertTrue(LogitechHIDProtocol.g502Buttons.isDisjoint(with: LogitechHIDProtocol.mxButtons))
        XCTAssertTrue(LogitechHIDProtocol.g502Buttons.isDisjoint(with: Set(0...31)))
        XCTAssertNil(LogitechHIDProtocol.control([0, 210, 0, 0, 0x20, 0, 0, 0]))
        var settings = MouseSettings(enabled: true)
        settings.set(button: 210, modifiers: 0, action: .shortcut(124, 0x40000))
        let decoded = try JSONDecoder().decode(MouseSettings.self, from: JSONEncoder().encode(settings))
        XCTAssertEqual(decoded.action(button: 210, modifiers: 0), .shortcut(124, 0x40000))
        XCTAssertNil(decoded.action(button: 2, modifiers: 0))
    }
}
