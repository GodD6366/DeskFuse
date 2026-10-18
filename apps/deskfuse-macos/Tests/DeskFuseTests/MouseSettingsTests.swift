import XCTest
@testable import DeskFuse

final class MouseSettingsTests: XCTestCase {
    func testExactModifierWinsAndBaseIsFallback() {
        var s = MouseSettings(enabled: true)
        s.set(button: 3, modifiers: 0, action: .shortcut(123, 0x40000))
        s.set(button: 3, modifiers: 0x100000, action: .disabled)
        XCTAssertEqual(s.action(button: 3, modifiers: 0x100000), .disabled)
        XCTAssertEqual(s.action(button: 3, modifiers: 0x20000), .shortcut(123, 0x40000))
        s.enabled = false
        XCTAssertNil(s.action(button: 3, modifiers: 0))
    }
    func testRemoveOverrideAndJSONContract() throws {
        var s = MouseSettings(enabled: true)
        s.set(button: 4, modifiers: 0, action: .mouse(1))
        s.set(button: 4, modifiers: 0x20000, action: .disabled)
        s.set(button: 4, modifiers: 0x20000, action: nil)
        XCTAssertEqual(s.bindings.count, 1)
        let json = try JSONEncoder().encode(s)
        XCTAssertEqual(try JSONDecoder().decode(MouseSettings.self, from: json), s)
        let fixture = Data(#"{"enabled":true,"bindings":[{"button":3,"modifiers":0,"action":{"kind":"shortcut","key_code":123,"modifiers":262144}}]}"#.utf8)
        XCTAssertEqual(try JSONDecoder().decode(MouseSettings.self, from: fixture).action(button: 3, modifiers: 0), .shortcut(123, 0x40000))
    }
}
