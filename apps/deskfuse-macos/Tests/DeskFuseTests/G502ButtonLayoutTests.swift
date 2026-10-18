import XCTest
@testable import DeskFuse

final class G502ButtonLayoutTests: XCTestCase {
    func testConfirmedPositionsRoundTrip() {
        var locations = G502ButtonLocations()
        locations.locate(210, at: .middle)
        locations.locate(212, at: .thumb)
        XCTAssertEqual(G502ButtonLocations(json: locations.json), locations)
        XCTAssertEqual(locations.button(at: .middle), 210)
    }

    func testMovingEventDoesNotDuplicateLocation() {
        var locations = G502ButtonLocations()
        locations.locate(212, at: .back)
        locations.locate(212, at: .thumb)
        XCTAssertNil(locations.button(at: .back))
        XCTAssertEqual(locations.position(for: 212), .thumb)
    }

    func testInvalidAndForeignEventsAreIgnored() {
        let locations = G502ButtonLocations(json: "{\"left\":208,\"right\":208,\"middle\":195,\"thumb\":219,\"unknown\":211}")
        XCTAssertEqual(locations.buttons, ["left": 208])
        XCTAssertTrue(G502ButtonLocations(json: "invalid").buttons.isEmpty)
        var empty = G502ButtonLocations()
        empty.locate(3, at: .middle)
        XCTAssertTrue(empty.buttons.isEmpty)
    }
}
