import CoreMedia
import CoreVideo
import Foundation
import Testing
@testable import DeskFuseRemoteApplication

// MARK: - Policy

@Test func refinementWaitsForTheScreenToSettle() {
    let policy = PaintOverPolicy(settleDelay: 0.2, burst: 2, bitrateMultiplier: 3, cooldown: 1)
    var planner = PaintOverPlanner(startedAt: 0)
    planner.activity(at: 1)
    // #expect wraps its expression, so a mutating call is hoisted out first.
    // 1.0 + 0.2 is not exactly 1.2 in binary floating point.
    let early = planner.begin(at: 1.19, policy: policy)
    #expect(!early)
    let settled = planner.begin(at: 1.21, policy: policy)
    #expect(settled)
    #expect(planner.remaining == 2)
    let first = planner.takePass(), second = planner.takePass(), third = planner.takePass()
    #expect(first)
    #expect(second)
    #expect(!third)
}

@Test func movingContentCancelsAnUnfinishedBurst() {
    let policy = PaintOverPolicy(settleDelay: 0.1, burst: 5, bitrateMultiplier: 3, cooldown: 0)
    var planner = PaintOverPlanner(startedAt: 0)
    planner.activity(at: 1)
    let began = planner.begin(at: 1.2, policy: policy)
    #expect(began)
    let pass = planner.takePass()
    #expect(pass)
    #expect(planner.remaining == 4)
    planner.activity(at: 1.3)
    #expect(planner.remaining == 0)
    let cancelled = planner.takePass()
    #expect(!cancelled)
    // The settle clock restarted with the new content.
    let restarted = planner.begin(at: 1.35, policy: policy)
    #expect(!restarted)
}

@Test func cooldownStopsAScreenThatFlickersBetweenTwoStills() {
    let policy = PaintOverPolicy(settleDelay: 0.1, burst: 1, bitrateMultiplier: 3, cooldown: 2)
    var planner = PaintOverPlanner(startedAt: 0)
    planner.activity(at: 1)
    let first = planner.begin(at: 1.2, policy: policy)
    #expect(first)
    let pass = planner.takePass()
    #expect(pass)
    planner.activity(at: 1.3)
    let tooSoon = planner.begin(at: 1.5, policy: policy)
    #expect(!tooSoon)
    let later = planner.begin(at: 3.2, policy: policy)
    #expect(later)
}

@Test func disabledPolicyAndDegenerateValuesNeverRefine() {
    var planner = PaintOverPlanner(startedAt: 0)
    planner.activity(at: 0)
    let began = planner.begin(at: 100, policy: .disabled)
    #expect(!began)
    #expect(!PaintOverPolicy(settleDelay: 0.1, burst: 0, bitrateMultiplier: 3, cooldown: 0).isEnabled)
    #expect(!PaintOverPolicy(settleDelay: 0.1, burst: 1, bitrateMultiplier: 1, cooldown: 0).isEnabled)
    #expect(PaintOverPolicy.standard.isEnabled)
}

@Test func refinementRaisesTheTargetButStaysBounded() {
    let policy = PaintOverPolicy(settleDelay: 0.1, burst: 2, bitrateMultiplier: 3, cooldown: 0)
    #expect(policy.refinementBitrate(for: 10_000_000) == 30_000_000)
    // Never lower the live target, whatever the adaptive rate has become.
    #expect(policy.refinementBitrate(for: 0) == 0)
    #expect(policy.refinementBitrate(for: 150_000_000) == 200_000_000)
}

// MARK: - Encoder

private final class CollectedFrames: @unchecked Sendable {
    private let lock = NSLock()
    private var frames: [EncodedVideoFrame] = []
    private var failures: [String] = []
    func remember(_ result: Result<EncodedVideoFrame, RemoteApplicationError>) {
        lock.withLock {
            switch result {
            case .success(let frame): frames.append(frame)
            case .failure(let error): failures.append(error.localizedDescription)
            }
        }
    }
    var counts: (frames: Int, failures: Int) { lock.withLock { (frames.count, failures.count) } }
    var encoded: [EncodedVideoFrame] { lock.withLock { frames } }
}

/// Polls until `condition` holds, so a passing test never waits out its timeout.
private func waitUntil(_ timeout: Double = 3, _ condition: () -> Bool) -> Bool {
    let deadline = ProcessInfo.processInfo.systemUptime + timeout
    while ProcessInfo.processInfo.systemUptime < deadline {
        if condition() { return true }
        Thread.sleep(forTimeInterval: 0.02)
    }
    return condition()
}

private func makeFrame(surface: SurfaceIdentity, seed: UInt8, at seconds: Double,
                       size: PixelSize = PixelSize(width: 320, height: 180)) throws -> CapturedVideoFrame {
    var buffer: CVPixelBuffer?
    let created = CVPixelBufferCreate(kCFAllocatorDefault, size.width, size.height,
        kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange, nil, &buffer)
    #expect(created == kCVReturnSuccess)
    let image = try #require(buffer)
    CVPixelBufferLockBaseAddress(image, [])
    defer { CVPixelBufferUnlockBaseAddress(image, []) }
    let luma = try #require(CVPixelBufferGetBaseAddressOfPlane(image, 0)).assumingMemoryBound(to: UInt8.self)
    let lumaStride = CVPixelBufferGetBytesPerRowOfPlane(image, 0)
    for row in 0..<CVPixelBufferGetHeightOfPlane(image, 0) {
        for column in 0..<size.width { luma[row * lumaStride + column] = UInt8((column &+ Int(seed)) % 251) }
    }
    if let chroma = CVPixelBufferGetBaseAddressOfPlane(image, 1)?.assumingMemoryBound(to: UInt8.self) {
        let stride = CVPixelBufferGetBytesPerRowOfPlane(image, 1)
        for row in 0..<CVPixelBufferGetHeightOfPlane(image, 1) {
            for column in 0..<size.width { chroma[row * stride + column] = 128 }
        }
    }
    return CapturedVideoFrame(surface: surface, buffer: image,
                              timestamp: CMTime(value: CMTimeValue(seconds * 600), timescale: 600))
}

private func makeEncoder(surface: SurfaceIdentity, metrics: MediaMetrics, policy: PaintOverPolicy,
                         collected: CollectedFrames) -> VideoEncoder? {
    try? VideoEncoder(surface: surface,
        profile: VideoProfile(codec: .h264, pixels: PixelSize(width: 320, height: 180),
                              framesPerSecond: 30, bitrate: 1_000_000),
        metrics: metrics, flowControlled: false, paintOver: policy) { collected.remember($0) }
}

@Test func settledPictureIsEncodedAgainWithoutANewCapture() throws {
    let surface = SurfaceIdentity(session: UUID(), epoch: 1, generation: 1, configuration: 1)
    let metrics = MediaMetrics(), collected = CollectedFrames()
    let policy = PaintOverPolicy(settleDelay: 0.15, burst: 2, bitrateMultiplier: 3, cooldown: 5)
    // A machine without a hardware encoder never serves this path either.
    guard let encoder = makeEncoder(surface: surface, metrics: metrics, policy: policy, collected: collected) else { return }
    defer { encoder.stop() }

    encoder.submit(try makeFrame(surface: surface, seed: 0, at: 0))
    encoder.submit(try makeFrame(surface: surface, seed: 7, at: 0.033))
    let submitted = waitUntil { collected.counts.frames >= 1 }
    #expect(submitted)
    let beforeRefinement = collected.counts.frames
    // Nothing is submitted from here: every further frame is a still pass.
    #expect(waitUntil { collected.counts.frames >= beforeRefinement + 2 })

    let stats = metrics.snapshot()
    #expect((stats.paintOverFrames ?? 0) >= 2)
    #expect(collected.counts.failures == 0)
    let frames = collected.encoded
    #expect(frames.allSatisfy { $0.surface == surface })
    let ordered = zip(frames, frames.dropFirst()).allSatisfy { $0.sequence < $1.sequence }
    #expect(ordered)
    // A refinement restates the current picture; it must not spend a key frame.
    #expect(!frames.dropFirst().contains { $0.keyFrame })
}

@Test func refinementStopsAfterItsBurstAndYieldsToNewContent() throws {
    let surface = SurfaceIdentity(session: UUID(), epoch: 2, generation: 1, configuration: 1)
    let metrics = MediaMetrics(), collected = CollectedFrames()
    let policy = PaintOverPolicy(settleDelay: 0.1, burst: 2, bitrateMultiplier: 2, cooldown: 30)
    guard let encoder = makeEncoder(surface: surface, metrics: metrics, policy: policy, collected: collected) else { return }
    defer { encoder.stop() }

    encoder.submit(try makeFrame(surface: surface, seed: 0, at: 0))
    #expect(waitUntil { (metrics.snapshot().paintOverFrames ?? 0) == 2 })
    let settled = collected.counts.frames
    // Cooldown is 30 s here: the burst is over and nothing more may follow.
    Thread.sleep(forTimeInterval: 0.4)
    #expect(collected.counts.frames == settled)
    #expect((metrics.snapshot().paintOverFrames ?? 0) == 2)

    // New content is still encoded once, in order.
    encoder.submit(try makeFrame(surface: surface, seed: 90, at: 0.5))
    #expect(waitUntil { collected.counts.frames == settled + 1 })
    #expect(collected.counts.failures == 0)
}

@Test func movingScreenIsNeverRefined() throws {
    let surface = SurfaceIdentity(session: UUID(), epoch: 3, generation: 1, configuration: 1)
    let metrics = MediaMetrics(), collected = CollectedFrames()
    let policy = PaintOverPolicy(settleDelay: 0.15, burst: 3, bitrateMultiplier: 3, cooldown: 1)
    guard let encoder = makeEncoder(surface: surface, metrics: metrics, policy: policy, collected: collected) else { return }
    defer { encoder.stop() }

    // 40 ms capture interval is well under the settle delay, so no capture may
    // be mistaken for a still picture.
    for index in 0..<10 {
        encoder.submit(try makeFrame(surface: surface, seed: UInt8(index &* 17), at: Double(index) * 0.04))
        Thread.sleep(forTimeInterval: 0.04)
        #expect((metrics.snapshot().paintOverFrames ?? 0) == 0)
    }
    #expect(collected.counts.frames >= 1)
    #expect(collected.counts.failures == 0)
}
