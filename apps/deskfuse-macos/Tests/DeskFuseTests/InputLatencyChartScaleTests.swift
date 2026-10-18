import Testing
@testable import DeskFuse

struct InputLatencyChartScaleTests {
    @Test func smallFluctuationsKeepTheSameRange() {
        var scale = InputLatencyChartScale()
        for peak in [0.2, 1.6, 2.1, 8.1, 35.0, 2.0, 49.0] {
            scale.include(peak: peak)
            #expect(scale.ceiling == 50)
        }
    }

    @Test func largeSpikesExpandWithoutBouncingBackOnExpiry() {
        var scale = InputLatencyChartScale()
        scale.include(peak: 80)
        #expect(scale.ceiling == 100)
        scale.include(peak: 2)
        #expect(scale.ceiling == 100)
        scale.include(peak: 430)
        #expect(scale.ceiling == 500)
        scale.include(peak: 35)
        #expect(scale.ceiling == 500)
    }

    @Test func invalidSamplesCannotCorruptTheRange() {
        var scale = InputLatencyChartScale()
        for peak in [Double.nan, Double.infinity, -2] {
            scale.include(peak: peak)
            #expect(scale.ceiling == 50)
        }
        scale.include(peak: 12_000)
        #expect(scale.ceiling.isFinite)
        #expect(scale.ceiling >= 12_000)
    }
}
