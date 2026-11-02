import XCTest
import SwiftUI
import AppKit
@testable import DeskFuse

@MainActor final class SettingsNavigationTests: XCTestCase {
    /// 轻量模式恢复时，宿主用 `page` 或 `page:category` 告诉界面进程落到哪里；
    /// Go 侧按第一个冒号切分。
    func testLightweightResumeRoutes() {
        XCTAssertEqual(MyGoUIHost.route(page: "clipboard"), "clipboard")
        XCTAssertEqual(MyGoUIHost.route(page: "settings", category: "general"), "settings:general")
        XCTAssertEqual(MyGoUIHost.route(page: "settings", category: ""), "settings")
        XCTAssertEqual(MyGoUIHost.lightweightIdleSeconds, 600)
        XCTAssertEqual(MyGoUIHost.idlePollSeconds, 30)
    }

    /// DEBUG 构建没有随包界面进程时，从仓库产物里找 `deskfuse-mygo`；
    /// 逐层向上必须真的走到仓库根（SwiftPM 的产物目录距根 6 层）。
    func testDevelopmentUILookupWalksUpToRepositoryRoot() {
        let repo = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()   // DeskFuseTests
            .deletingLastPathComponent()   // Tests
            .deletingLastPathComponent()   // deskfuse-macos
            .deletingLastPathComponent()   // apps
        let product = repo.appendingPathComponent("apps/deskfuse-macos/.build/out/Products/Debug")
        let found = MyGoUIOptions.devExecutable(startingAt: product)
        guard let found else {
            XCTFail("expected to find a development UI binary from \(product.path)")
            return
        }
        XCTAssertTrue(["deskfuse", "deskfuse-mygo"].contains(found.lastPathComponent), found.path)
        XCTAssertTrue(FileManager.default.isExecutableFile(atPath: found.path))
    }

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

    /// 设置页由 WebView 客户端提供：原生导航只更新目标分类，不再切到本地设置页，
    /// 也不抢主窗口。
    func testSettingsNavigationKeepsLastWorkspaceAndUsesMainWindow() {
        _ = NSApplication.shared
        let store = AppStore()
        store.selectedPage = .clipboard
        store.openSettings(tab: "workflows")
        XCTAssertEqual(store.selectedPage, .clipboard)
        XCTAssertEqual(store.workspacePage, .clipboard)
        XCTAssertEqual(store.settingsDestination.category, .workflows)
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
