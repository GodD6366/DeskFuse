import XCTest
@testable import DeskFuse

final class G502ButtonLayoutTests: XCTestCase {
    func testFactoryLayoutFollowsGKeyOrder() {
        let locations = G502ButtonLocations()
        XCTAssertEqual(locations.button(at: .left), 208)
        XCTAssertEqual(locations.button(at: .back), 211)
        XCTAssertEqual(locations.button(at: .forward), 212)
        XCTAssertEqual(locations.button(at: .dpiDown), 214)
        XCTAssertEqual(locations.button(at: .tiltRight), 218)
        XCTAssertEqual(Set(G502ButtonPosition.allCases.map(locations.button(at:))), Set(208...218))
    }

    func testRelocatingSwapsWithDisplacedPosition() {
        var locations = G502ButtonLocations()
        locations.locate(213, at: .back)
        XCTAssertEqual(locations.button(at: .back), 213)
        XCTAssertEqual(locations.button(at: .thumb), 211)
        XCTAssertEqual(G502ButtonLocations(json: locations.json), locations)
    }

    func testInvalidAndForeignEventsAreIgnored() {
        let locations = G502ButtonLocations(json: "{\"left\":209,\"right\":209,\"middle\":195,\"thumb\":219,\"unknown\":211}")
        XCTAssertEqual(locations.button(at: .left), 209)
        XCTAssertEqual(locations.button(at: .right), 208)
        XCTAssertEqual(locations.button(at: .middle), 210)
        XCTAssertEqual(locations.button(at: .thumb), 213)
        XCTAssertEqual(G502ButtonLocations(json: "invalid"), G502ButtonLocations())
        var unchanged = G502ButtonLocations()
        unchanged.locate(3, at: .middle)
        XCTAssertEqual(unchanged, G502ButtonLocations())
    }
}
