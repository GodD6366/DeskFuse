/// Keep a live chart comparable between samples. A large spike can expand the
/// range, but its expiry must not rescale every remaining point on the page.
struct InputLatencyChartScale {
    private(set) var ceiling: Double = 50

    static func requiredCeiling(for peak: Double) -> Double {
        guard peak.isFinite, peak > 50 else { return 50 }
        let steps: [Double] = [100, 200, 500, 1_000, 2_000, 5_000, 10_000]
        return steps.first { peak <= $0 } ?? peak
    }

    mutating func include(peak: Double) {
        ceiling = max(ceiling, Self.requiredCeiling(for: peak))
    }
}
