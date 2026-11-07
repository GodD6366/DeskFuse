import Foundation

/// Re-encodes the last captured picture once the screen stops changing.
///
/// Motion and stills share one adaptive bitrate, and that rate is chosen for
/// the worst case: a scrolling window or a drag spends the budget on change. A
/// still screen carries no change at all, so leaving the encoder on its motion
/// target freezes text at the coarse quantization of the last moving frame for
/// as long as the user reads it. After `settleDelay` of quiet capture, the last
/// frame is encoded again with a raised target. The encoder's reference is its
/// own lossy reconstruction while the source is clean, so those extra bits
/// remove quantization error instead of inventing detail: the still picture
/// converges back toward the source and then stops.
///
/// Borrowed from voxl's `--paintover-*` behaviour (tembo/voxl), which sharpens
/// the X11/macOS stream when it settles. It is the only part of that pipeline
/// DeskFuse's media path was missing; damage-driven capture, latest-wins
/// frame coalescing and acknowledgement-based flow control already exist here.
public struct PaintOverPolicy: Sendable, Equatable {
    /// Quiet capture time before a still picture is refined.
    public var settleDelay: TimeInterval
    /// Frames spent refining one still picture. More passes converge further
    /// because each one sees the previous pass's reconstruction as reference.
    public var burst: Int
    /// Multiple of the current adaptive bitrate a refinement may spend.
    public var bitrateMultiplier: Double
    /// Shortest gap between two refinements, so a screen that alternates
    /// between two still pictures cannot refine on every settle.
    public var cooldown: TimeInterval

    public init(settleDelay: TimeInterval = 0.18, burst: Int = 3,
                bitrateMultiplier: Double = 3, cooldown: TimeInterval = 1) {
        self.settleDelay = settleDelay; self.burst = burst
        self.bitrateMultiplier = bitrateMultiplier; self.cooldown = cooldown
    }

    public static let standard = PaintOverPolicy()
    public static let disabled = PaintOverPolicy(settleDelay: 0, burst: 0, bitrateMultiplier: 1, cooldown: 0)

    public var isEnabled: Bool { settleDelay > 0 && burst > 0 && bitrateMultiplier > 1 }

    /// A refinement may only ever raise the target. VideoToolbox answers a
    /// higher `AverageBitRate` with a finer quantizer; clamping keeps a burst
    /// bounded even when the adaptive rate has already climbed to its ceiling.
    public func refinementBitrate(for base: Int) -> Int {
        let raised = Int(Double(base) * bitrateMultiplier)
        return min(200_000_000, max(base, raised))
    }
}

/// Decides when a still picture is worth re-encoding. Pure timing state, so the
/// encoder's queue owns all of it and no capture, codec or transport knowledge
/// leaks in.
public struct PaintOverPlanner: Sendable {
    public private(set) var remaining = 0
    private var lastActivity: Double
    private var lastBurst: Double = -.infinity

    public init(startedAt: Double = 0) { lastActivity = startedAt }

    /// Moving content supersedes a refinement: its remaining passes would spend
    /// bits on a picture the viewer has already left.
    public mutating func activity(at now: Double) {
        lastActivity = now
        remaining = 0
    }

    /// Starts a refinement when the picture has been still long enough. The
    /// caller submits every pass, including the first, through `takePass()`.
    public mutating func begin(at now: Double, policy: PaintOverPolicy) -> Bool {
        guard policy.isEnabled, remaining == 0,
              now - lastActivity >= policy.settleDelay,
              now - lastBurst >= policy.cooldown else { return false }
        remaining = policy.burst
        lastBurst = now
        return true
    }

    /// Consumes one pass; `false` means the burst is over.
    public mutating func takePass() -> Bool {
        guard remaining > 0 else { return false }
        remaining -= 1
        return true
    }
}
