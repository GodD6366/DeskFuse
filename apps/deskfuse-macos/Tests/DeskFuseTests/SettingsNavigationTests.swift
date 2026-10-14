import XCTest
import SwiftUI
import AppKit
@testable import DeskFuse

@MainActor final class SettingsNavigationTests: XCTestCase {
    func testDefaultAndExplicitDestinations() {
        XCTAssertEqual(SettingsDestination(tab: nil, clipboardSegment: nil).category, .general)
        XCTAssertEqual(SettingsDestination(tab: "permissions", clipboardSegment: nil).category, .permissions)
        XCTAssertFalse(SettingsCategory.allCases.map(\.rawValue).contains("input"))
        XCTAssertEqual(SettingsDestination(tab: "removed-category", clipboardSegment: nil).category, .general)
    }

    func testWorkflowDeepLinkAndRepeatedOpening() {
        let workflows = SettingsDestination(tab: "clipboard", clipboardSegment: "workflows")
        XCTAssertEqual(workflows.category, .clipboard)
        XCTAssertEqual(workflows.clipboardSegment, "workflows")
        XCTAssertEqual(SettingsDestination(tab: nil, clipboardSegment: nil, previous: workflows), workflows)
        let history = SettingsDestination(tab: "clipboard", clipboardSegment: "history", previous: workflows)
        XCTAssertEqual(history.clipboardSegment, "history")
        XCTAssertEqual(SettingsDestination(tab: "clipboard", clipboardSegment: "invalid").clipboardSegment, "history")
    }

    func testSettingsNavigationKeepsLastWorkspaceAndUsesMainWindow() {
        _ = NSApplication.shared
        let store = AppStore()
        store.selectedPage = .clipboard
        var opens = 0
        store.showMainWindow = { opens += 1 }
        store.openSettings(tab: "clipboard", clipboardSegment: "workflows")
        XCTAssertEqual(store.selectedPage, .settings)
        XCTAssertEqual(store.workspacePage, .clipboard)
        XCTAssertEqual(store.settingsDestination.clipboardSegment, "workflows")
        // If a main window already exists openMain raises it; otherwise it calls the opener.
        if !NSApp.windows.contains(where: { $0.identifier?.rawValue == "deskfuse-main" }) {
            XCTAssertEqual(opens, 1)
        }
        store.selectedPage = .devices
        store.openSettings(tab: "permissions")
        XCTAssertEqual(store.workspacePage, .devices)
        XCTAssertEqual(store.settingsDestination.category, .permissions)
        store.openSettings(tab: "input")
        XCTAssertEqual(store.selectedPage, .input)
        XCTAssertEqual(store.workspacePage, .input)
        XCTAssertEqual(store.settingsDestination.category, .permissions)
    }
}
