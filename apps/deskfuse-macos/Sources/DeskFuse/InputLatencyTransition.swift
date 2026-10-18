import Foundation
import SwiftUI

struct InputLatencyTransition {
    private(set) var source: [InputLatencyPoint] = []
    private(set) var target: [InputLatencyPoint] = []
    private(set) var startedAt = Date.distantPast
    private(set) var duration: TimeInterval = 1

    func points(at time: Date) -> [InputLatencyPoint] {
        guard !source.isEmpty, source != target else { return target }
        let progress = min(1, max(0, time.timeIntervalSince(startedAt) / duration))
        let previous = Dictionary(uniqueKeysWithValues: source.map { ($0.id, $0) })
        return target.map { point in
            guard let old = previous[point.id], old.series == point.series else { return point }
            return InputLatencyPoint(second: point.second,
                rttMs: old.rttMs + (point.rttMs - old.rttMs) * progress, series: point.series)
        }
    }

    mutating func retarget(_ points: [InputLatencyPoint], at time: Date, duration: TimeInterval) {
        source = self.points(at: time)
        target = points
        startedAt = time
        self.duration = max(0.001, duration)
    }

    mutating func snap(to points: [InputLatencyPoint]) {
        source = points
        target = points
    }
}

/// Animate values in one existing curve. Swift Charts' implicit series
/// transitions crossfade whole paths, leaving ghosts and a detached endpoint.
struct InputLatencyTransitionView<Content: View>: View {
    let points: [InputLatencyPoint]
    let active: Bool
    @ViewBuilder let content: ([InputLatencyPoint]) -> Content
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var transition = InputLatencyTransition()
    @State private var moving = false

    var body: some View {
        TimelineView(.animation(minimumInterval: 1.0 / 60, paused: !moving || !active || reduceMotion)) { context in
            content(transition.target.isEmpty ? points : transition.points(at: context.date))
                .transaction { transaction in
                    transaction.animation = nil
                    transaction.disablesAnimations = true
                }
        }
        .onAppear { transition.snap(to: points) }
        .onChange(of: points) { _, updated in
            if !active || reduceMotion {
                transition.snap(to: updated)
                moving = false
            } else {
                transition.retarget(updated, at: .now, duration: Motion.telemetryDuration)
                moving = transition.source != transition.target
            }
        }
        .onChange(of: active) { _, _ in transition.snap(to: points); moving = false }
        .onChange(of: reduceMotion) { _, reduced in
            if reduced { transition.snap(to: points); moving = false }
        }
        .onDisappear { moving = false }
        .task(id: transition.startedAt) {
            guard moving, active, !reduceMotion else { return }
            let token = transition.startedAt
            do { try await Task.sleep(for: .seconds(Motion.telemetryDuration)) } catch { return }
            guard !Task.isCancelled, moving, transition.startedAt == token else { return }
            transition.snap(to: points)
            moving = false
        }
    }
}
