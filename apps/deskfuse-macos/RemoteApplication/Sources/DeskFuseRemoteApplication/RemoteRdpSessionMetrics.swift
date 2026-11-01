import Foundation

/// Timing counters for one RDP session attempt. The host connection feeds it
/// connection and frame events, then writes one `rdp_session_closed` diagnostic
/// with the totals instead of logging every frame.
public struct RemoteRdpSessionMetrics: Sendable, Equatable {
    public private(set) var startedAt: Date?
    public private(set) var connectedAt: Date?
    public private(set) var firstFrameAt: Date?
    public private(set) var frames: Int

    public init() {
        frames = 0
    }

    public mutating func begin(at date: Date) {
        startedAt = date
        connectedAt = nil
        firstFrameAt = nil
        frames = 0
    }

    public mutating func markConnected(at date: Date) {
        if connectedAt == nil { connectedAt = date }
    }

    /// Returns true exactly once per session, on the first delivered frame.
    public mutating func markFrame(at date: Date) -> Bool {
        frames += 1
        guard firstFrameAt == nil else { return false }
        firstFrameAt = date
        return true
    }

    public func connectMilliseconds() -> Int? {
        guard let startedAt, let connectedAt else { return nil }
        return max(0, Int(connectedAt.timeIntervalSince(startedAt) * 1000))
    }

    public func firstFrameMilliseconds() -> Int? {
        guard let startedAt, let firstFrameAt else { return nil }
        return max(0, Int(firstFrameAt.timeIntervalSince(startedAt) * 1000))
    }

    public func uptimeMilliseconds(now: Date) -> Int? {
        guard let startedAt else { return nil }
        return max(0, Int(now.timeIntervalSince(startedAt) * 1000))
    }

    public func frameRate(now: Date) -> Double {
        guard let startedAt, frames > 0 else { return 0 }
        let seconds = now.timeIntervalSince(startedAt)
        guard seconds > 0 else { return 0 }
        return Double(frames) / seconds
    }

    public var isActive: Bool { startedAt != nil }
}
