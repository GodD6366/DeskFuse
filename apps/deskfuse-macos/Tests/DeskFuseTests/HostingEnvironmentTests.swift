import SwiftUI
import XCTest
@testable import DeskFuse

private final class EnvironmentLifetimeToken: Sendable {}
private struct LifetimeKey: EnvironmentKey {
    static let defaultValue: EnvironmentLifetimeToken? = nil
}

@MainActor final class HostingEnvironmentTests: XCTestCase {
    func testSnapshotDoesNotRetainUnrelatedParentEnvironment() {
        weak var retainedToken: EnvironmentLifetimeToken?
        let snapshot: HostingEnvironment = {
            let token = EnvironmentLifetimeToken()
            retainedToken = token
            var parent = EnvironmentValues()
            parent[LifetimeKey.self] = token
            return HostingEnvironment(parent)
        }()
        withExtendedLifetime(snapshot) {
            XCTAssertNil(retainedToken, "The hosting boundary must not retain the parent property list")
        }
    }

    func testPresentationValuesCrossBoundary() {
        var parent = EnvironmentValues()
        parent.dfOpaque = true
        parent.colorScheme = .dark
        parent.controlSize = .small
        parent.font = .system(size: 13)
        parent.isEnabled = false
        parent.layoutDirection = .rightToLeft
        parent.locale = Locale(identifier: "zh_CN")
        parent.displayScale = 2
        parent.dynamicTypeSize = .accessibility1
        parent.scenePhase = .inactive
        let snapshot = HostingEnvironment(parent)
        XCTAssertTrue(snapshot.quietWorkspace)
        XCTAssertEqual(snapshot.colorScheme, .dark)
        XCTAssertEqual(snapshot.controlSize, .small)
        XCTAssertEqual(snapshot.font, .system(size: 13))
        XCTAssertFalse(snapshot.isEnabled)
        XCTAssertEqual(snapshot.layoutDirection, .rightToLeft)
        XCTAssertEqual(snapshot.locale.identifier, "zh_CN")
        XCTAssertEqual(snapshot.displayScale, 2)
        XCTAssertEqual(snapshot.dynamicTypeSize, .accessibility1)
        XCTAssertEqual(snapshot.scenePhase, .inactive)
    }
}
