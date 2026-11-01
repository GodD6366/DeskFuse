import Foundation

/// Backoff and deadline for re-establishing an RDP session after the QUIC
/// tunnel or FreeRDP connection drops. The goal is to ride out a short network
/// or sleep/wake interruption without asking the user to reopen every app.
public struct RemoteRdpReconnectPolicy: Sendable, Equatable {
    /// Total time to keep retrying, measured from the first failure.
    public let deadline: TimeInterval
    /// Delay before attempt 1, 2, …; the last value repeats.
    public let retryDelays: [TimeInterval]

    public init(deadline: TimeInterval = 60, retryDelays: [TimeInterval] = [1, 2, 4, 8]) {
        self.deadline = deadline
        self.retryDelays = retryDelays
    }

    /// 1-based attempt number.
    public func delay(afterAttempt attempt: Int) -> TimeInterval {
        guard let last = retryDelays.last else { return 0 }
        guard attempt > 0 else { return retryDelays.first ?? last }
        return attempt <= retryDelays.count ? retryDelays[attempt - 1] : last
    }

    public func mayRetry(elapsed: TimeInterval) -> Bool {
        elapsed < deadline
    }
}
