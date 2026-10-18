struct InputLatencyPoint: Identifiable, Equatable {
    let second: Double
    let rttMs: Double
    let series: Int
    var id: Double { second }
}

enum InputLatencyTrend {
    /// A trailing three-second mean is presentation only. Do not average across
    /// missing data or a new connection segment, and keep raw metrics separate.
    static func smooth(_ points: [InputLatencyPoint]) -> [InputLatencyPoint] {
        var window: [InputLatencyPoint] = []
        return points.map { point in
            if let previous = window.last,
               previous.series != point.series || point.second - previous.second > 2 || point.second <= previous.second {
                window.removeAll(keepingCapacity: true)
            }
            window.removeAll { point.second - $0.second >= 3 }
            window.append(point)
            return InputLatencyPoint(second: point.second,
                rttMs: window.reduce(0) { $0 + $1.rttMs } / Double(window.count), series: point.series)
        }
    }
}
