import XCTest
@testable import DeskFuse

final class MouseShortcutTests: XCTestCase {
    func testSpacesEmitsCompleteControlChordAndIntrinsicArrowFlags() {
        var state = MouseShortcutState()
        let shortcut = MouseShortcut(key: 124, modifiers: 0x40000)
        XCTAssertEqual(state.events(for: shortcut, down: true, physicalFlags: 0), [
            .init(key: 59, down: true, flags: 0x40000),
            .init(key: 124, down: true, flags: 0xa40000)
        ])
        XCTAssertEqual(state.events(for: shortcut, down: false, physicalFlags: 0xa40000), [
            .init(key: 124, down: false, flags: 0xa40000),
            .init(key: 59, down: false, flags: 0)
        ])
    }
    func testOverlappingActionsKeepModifierUntilLastRelease() {
        var state = MouseShortcutState()
        let left = MouseShortcut(key: 123, modifiers: 0x40000)
        let right = MouseShortcut(key: 124, modifiers: 0x40000)
        _ = state.events(for: left, down: true, physicalFlags: 0)
        XCTAssertEqual(state.events(for: right, down: true, physicalFlags: 0x40000).count, 1)
        XCTAssertEqual(state.events(for: left, down: false, physicalFlags: 0x40000).count, 1)
        XCTAssertEqual(state.events(for: right, down: false, physicalFlags: 0x40000).last,
                       .init(key: 59, down: false, flags: 0))
        XCTAssertTrue(state.events(for: right, down: false, physicalFlags: 0).isEmpty)
    }
    func testNeverReleasesControlThatWasAlreadyPhysicallyHeld() {
        var state = MouseShortcutState()
        let shortcut = MouseShortcut(key: 124, modifiers: 0x40000)
        XCTAssertEqual(state.events(for: shortcut, down: true, physicalFlags: 0x40000).count, 1)
        XCTAssertEqual(state.events(for: shortcut, down: false, physicalFlags: 0x40000).count, 1)
    }
    func testDisabledSystemShortcutIsNotSilentlyPosted() {
        let shortcut = MouseShortcut(key: 124, modifiers: 0x40000)
        XCTAssertThrowsError(try shortcut.resolved(in: ["81": ["enabled": false]]))
        XCTAssertEqual(try shortcut.resolved(in: [:]), shortcut)
    }
    func testResolvesCustomizedSystemSpacesShortcut() throws {
        let shortcut = MouseShortcut(key: 123, modifiers: 0x40000)
        XCTAssertEqual(try shortcut.resolved(in: ["79": ["enabled": true, "value": [
            "type": "standard", "parameters": [65535, 18, 0x180000]
        ]]]), MouseShortcut(key: 18, modifiers: 0x180000))
        XCTAssertThrowsError(try shortcut.resolved(in: ["79": ["value": [
            "type": "standard", "parameters": [65535, 65535, 0]
        ]]]))
    }
}
