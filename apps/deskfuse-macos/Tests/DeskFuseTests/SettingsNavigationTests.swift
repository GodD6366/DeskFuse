import XCTest
import SwiftUI
import AppKit
@testable import DeskFuse

@MainActor final class SettingsNavigationTests: XCTestCase {
    func testDefaultAndExplicitDestinations() {
        XCTAssertEqual(SettingsDestination(tab: nil).category, .general)
        XCTAssertEqual(SettingsDestination(tab: "permissions").category, .permissions)
        XCTAssertFalse(SettingsCategory.allCases.map(\.rawValue).contains("input"))
        XCTAssertEqual(SettingsDestination(tab: "removed-category").category, .general)
        XCTAssertEqual(SettingsDestination(tab: "storage").category, .files)
    }

    func testWorkflowsIsItsOwnCategoryAndHiddenWhenDisabled() {
        let workflows = SettingsDestination(tab: "workflows")
        XCTAssertEqual(workflows.category, .workflows)
        XCTAssertEqual(SettingsDestination(tab: nil, previous: workflows), workflows)
        XCTAssertTrue(SettingsCategory.visible(workflowsEnabled: true).contains(.workflows))
        XCTAssertFalse(SettingsCategory.visible(workflowsEnabled: false).contains(.workflows))
        XCTAssertEqual(SettingsCategory.visible(workflowsEnabled: false).count, SettingsCategory.allCases.count - 1)
    }

    func testSettingsNavigationKeepsLastWorkspaceAndUsesMainWindow() {
        _ = NSApplication.shared
        let store = AppStore()
        store.selectedPage = .clipboard
        var opens = 0
        store.showMainWindow = { opens += 1 }
        store.openSettings(tab: "workflows")
        XCTAssertEqual(store.selectedPage, .settings)
        XCTAssertEqual(store.workspacePage, .clipboard)
        XCTAssertEqual(store.settingsDestination.category, .workflows)
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
