import Foundation
import Testing
@testable import DeskFuse

struct InputLatencyTransitionTests {
    private let start = Date(timeIntervalSince1970: 1_000)
    private func point(_ value: Double, series: Int = 0) -> [InputLatencyPoint] {
        [InputLatencyPoint(second: 60, rttMs: value, series: series)]
    }

    @Test func interpolatesOneCurveWithoutOvershootingAndReachesTheTarget() {
        var transition = InputLatencyTransition()
        transition.snap(to: point(3))
        transition.retarget(point(15), at: start, duration: 1)
        #expect(transition.points(at: start)[0].rttMs == 3)
        #expect(transition.points(at: start.addingTimeInterval(0.5))[0].rttMs == 9)
        #expect(transition.points(at: start.addingTimeInterval(1))[0].rttMs == 15)
        #expect(transition.points(at: start.addingTimeInterval(2))[0].rttMs == 15)
    }

    @Test func interruptionContinuesFromTheDisplayedValue() {
        var transition = InputLatencyTransition()
        transition.snap(to: point(0))
        transition.retarget(point(30), at: start, duration: 1)
        let interrupted = start.addingTimeInterval(0.5)
        transition.retarget(point(3), at: interrupted, duration: 1)
        #expect(transition.points(at: interrupted)[0].rttMs == 15)
        #expect(transition.points(at: interrupted.addingTimeInterval(0.5))[0].rttMs == 9)
        #expect(transition.points(at: interrupted.addingTimeInterval(1))[0].rttMs == 3)
    }

    @Test func snappingAndConnectionChangesNeverBlendOldData() {
        var transition = InputLatencyTransition()
        transition.snap(to: point(30))
        transition.retarget(point(3, series: 1), at: start, duration: 1)
        #expect(transition.points(at: start)[0].rttMs == 3)
        transition.snap(to: point(7))
        #expect(transition.points(at: start)[0].rttMs == 7)
        transition.snap(to: [])
        #expect(transition.points(at: start).isEmpty)
    }
}
