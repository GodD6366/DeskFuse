import Testing
@testable import DeskFuse

struct InputLatencyTrendTests {
    private func point(_ second: Double, _ rtt: Double, series: Int = 0) -> InputLatencyPoint {
        InputLatencyPoint(second: second, rttMs: rtt, series: series)
    }

    @Test func spikeOnlyAffectsItsThreeSecondWindowAndRawValuesStayUnchanged() {
        let raw = [point(0, 3), point(1, 3), point(2, 30), point(3, 3), point(4, 3), point(5, 3)]
        let trend = InputLatencyTrend.smooth(raw)
        #expect(trend.map(\.rttMs) == [3, 3, 12, 12, 12, 3])
        #expect(raw[2].rttMs == 30)
        #expect(trend.map(\.id) == raw.map(\.id))
    }

    @Test func gapsAndNewSegmentsDoNotBlendUnrelatedReadings() {
        let raw = [point(0, 30), point(1, 30), point(5, 3), point(6, 9, series: 1)]
        #expect(InputLatencyTrend.smooth(raw).map(\.rttMs) == [30, 30, 3, 9])
    }

    @Test func restartedTimelineDoesNotReuseOldWindow() {
        let raw = [point(59, 30), point(60, 30), point(0, 3), point(1, 3)]
        #expect(InputLatencyTrend.smooth(raw).map(\.rttMs) == [30, 30, 3, 3])
        #expect(InputLatencyTrend.smooth([]).isEmpty)
    }
}
