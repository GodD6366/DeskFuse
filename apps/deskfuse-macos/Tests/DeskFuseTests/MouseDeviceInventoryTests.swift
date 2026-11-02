import XCTest
@testable import DeskFuse

final class MouseDeviceInventoryTests: XCTestCase {
    func testInternalKeyboardAndTrackpadAreNotMice() {
        XCTAssertTrue(MouseDeviceInventory.isBuiltInPointer(name: "Apple Internal Keyboard / Trackpad", builtIn: false))
        XCTAssertTrue(MouseDeviceInventory.isBuiltInPointer(name: "Apple Internal Keyboard / Trackpad (ANSI)", builtIn: false))
        XCTAssertTrue(MouseDeviceInventory.isBuiltInPointer(name: "Apple Internal Trackpad", builtIn: true))
    }

    func testBuiltInFlagAloneExcludesDevice() {
        XCTAssertTrue(MouseDeviceInventory.isBuiltInPointer(name: "Trackpad", builtIn: true))
    }

    func testExternalMiceStayListed() {
        XCTAssertFalse(MouseDeviceInventory.isBuiltInPointer(name: "G502 LIGHTSPEED", builtIn: false))
        XCTAssertFalse(MouseDeviceInventory.isBuiltInPointer(name: "MX Master 3", builtIn: false))
        XCTAssertFalse(MouseDeviceInventory.isBuiltInPointer(name: "USB Receiver", builtIn: false))
        XCTAssertFalse(MouseDeviceInventory.isBuiltInPointer(name: "ROG STRIX SCOPE II 96 WIRELESS", builtIn: false))
    }
}
