import Foundation
@preconcurrency import CoreMedia
@preconcurrency import CoreVideo

/// Only this compressed representation may cross the future transport/FFI port.
/// AVCC access units (4-byte NAL lengths), not a serialized native sample buffer.
public struct EncodedVideoFrame: Codable, Sendable {
    public static let maximumBytes = 16 * 1024 * 1024
    public let surface: SurfaceIdentity
    public let codec: VideoCodec
    public let sequence: UInt64
    public let presentationNanoseconds: Int64
    public let keyFrame: Bool
    public let vps: Data
    public let sps: Data
    public let pps: Data
    public let bytes: Data

    public init(surface: SurfaceIdentity, codec: VideoCodec = .h264, sequence: UInt64, presentationNanoseconds: Int64,
                keyFrame: Bool, vps: Data = Data(), sps: Data = Data(), pps: Data = Data(), bytes: Data) {
        self.surface = surface; self.codec = codec; self.sequence = sequence
        self.presentationNanoseconds = presentationNanoseconds; self.keyFrame = keyFrame
        self.vps = vps; self.sps = sps; self.pps = pps; self.bytes = bytes
    }

    public func validate() throws {
        guard sequence > 0, !bytes.isEmpty, bytes.count <= Self.maximumBytes,
              vps.count <= 65536, sps.count <= 65536, pps.count <= 65536 else {
            throw RemoteApplicationError.malformedFrame
        }
        if keyFrame {
            let validParameterSets = codec == .hevc
                ? (!vps.isEmpty && !sps.isEmpty && !pps.isEmpty)
                : (!sps.isEmpty && !pps.isEmpty && vps.isEmpty)
            guard validParameterSets else { throw RemoteApplicationError.malformedFrame }
        } else {
            guard vps.isEmpty && sps.isEmpty && pps.isEmpty else { throw RemoteApplicationError.malformedFrame }
        }
        var offset = 0
        while offset < bytes.count {
            guard bytes.count - offset >= 4 else { throw RemoteApplicationError.malformedFrame }
            let length = bytes[offset..<(offset + 4)].reduce(0) { ($0 << 8) | Int($1) }
            offset += 4
            guard length > 0, length <= bytes.count - offset else { throw RemoteApplicationError.malformedFrame }
            offset += length
        }
    }
}

public enum FrameAdmission: Sendable, Equatable { case accepted, needsKeyFrame, stale }

/// Single-consumer bounded reference chain. If any reference is lost, no later
/// delta frame can be decoded until a fresh key frame restores the chain.
public struct EncodedFrameQueue: Sendable {
    private var frames: [EncodedVideoFrame] = []
    private let capacity: Int
    private var surface: SurfaceIdentity?
    private var codec: VideoCodec?
    private var lastSequence: UInt64 = 0
    private var needsKey = true
    public var count: Int { frames.count }

    public init(capacity: Int = 3) { precondition(capacity > 0); self.capacity = capacity }
    public mutating func reset(to identity: SurfaceIdentity) {
        frames.removeAll(keepingCapacity: true); surface = identity; codec = nil; lastSequence = 0; needsKey = true
    }
    public mutating func invalidate() { frames.removeAll(keepingCapacity: true); needsKey = true }

    public mutating func offer(_ frame: EncodedVideoFrame) -> FrameAdmission {
        guard frame.surface == surface, frame.sequence > lastSequence else { return .stale }
        if let codec, codec != frame.codec {
            invalidate()
            guard frame.keyFrame else { return .needsKeyFrame }
        }
        if frame.keyFrame { codec = frame.codec }
        if (needsKey || (lastSequence != 0 && frame.sequence != lastSequence + 1)) && !frame.keyFrame {
            invalidate(); return .needsKeyFrame
        }
        if frames.count >= capacity {
            invalidate()
            guard frame.keyFrame else { return .needsKeyFrame }
        }
        if frame.keyFrame { needsKey = false }
        frames.append(frame); lastSequence = frame.sequence
        return .accepted
    }
    public mutating func take() -> EncodedVideoFrame? { frames.isEmpty ? nil : frames.removeFirst() }
}

/// Immutable native ownership boxes. Buffers are never mutated after publication.
public struct CapturedVideoFrame: @unchecked Sendable {
    public let surface: SurfaceIdentity
    public let buffer: CVPixelBuffer
    public let timestamp: CMTime
    public init(surface: SurfaceIdentity, buffer: CVPixelBuffer, timestamp: CMTime) {
        self.surface = surface; self.buffer = buffer; self.timestamp = timestamp
    }
}

public struct DecodedVideoFrame: @unchecked Sendable {
    public let surface: SurfaceIdentity
    public let sequence: UInt64
    public let buffer: CVPixelBuffer
    public let timestamp: CMTime
}

public final class LatestVideoFrame: @unchecked Sendable {
    private let lock = NSLock()
    private var identity: SurfaceIdentity?
    private var frame: DecodedVideoFrame?
    private var pendingLayout: CaptureSurface?
    private var visibleLayout: CaptureSurface?
    public init() {}
    public func reset(_ identity: SurfaceIdentity?) {
        lock.withLock { self.identity = identity; frame = nil; pendingLayout = nil; visibleLayout = nil }
    }
    /// Keep one previous image visible while the new decoder waits for its IDR.
    /// The expected identity changes immediately, so old decoder output cannot
    /// overwrite it and the input layer cannot acknowledge it as the new surface.
    public func transition(to identity: SurfaceIdentity, layout: CaptureSurface? = nil) {
        lock.withLock { self.identity = identity; pendingLayout = layout }
    }
    public func updateLayout(_ layout: CaptureSurface) {
        lock.withLock {
            guard layout.identity == identity else { return }
            pendingLayout = layout
            if frame?.surface == identity { visibleLayout = layout }
        }
    }
    public func isCurrent(_ surface: SurfaceIdentity) -> Bool { lock.withLock { surface == identity } }
    public func publish(_ value: DecodedVideoFrame) {
        lock.withLock {
            guard value.surface == identity,
                  frame?.surface != value.surface || value.sequence > (frame?.sequence ?? 0) else { return }
            frame = value; visibleLayout = pendingLayout
        }
    }
    public func snapshot() -> DecodedVideoFrame? { lock.withLock { frame } }
    public func presentation() -> (frame: DecodedVideoFrame, layout: CaptureSurface?)? {
        lock.withLock { frame.map { ($0, visibleLayout) } }
    }
}

public struct MediaStatistics: Codable, Sendable {
    public var captured: UInt64 = 0
    /// Non-complete samples ScreenCaptureKit delivered, kept as buckets so a
    /// stream that never produces `.complete` frames is visible in statistics.
    public var capturedIdle: UInt64 = 0
    public var capturedBlank: UInt64 = 0
    public var capturedStarted: UInt64 = 0
    public var capturedOther: UInt64 = 0
    public var replacedBeforeEncode: UInt64 = 0
    public var encoderSubmitted: UInt64 = 0
    public var encoderCallbacks: UInt64 = 0
    public var encoderDropped: UInt64 = 0
    public var encoded: UInt64 = 0
    public var decoded: UInt64 = 0
    public var presented: UInt64 = 0
    public var chainResets: UInt64 = 0
    public var encodedBytes: UInt64 = 0
    public var encodeP95Milliseconds: Double = 0
    public var decodeP95Milliseconds: Double = 0
    public var hardwareEncoder = false
    public var hardwareDecoder = false
    public var encoderHardwareEvidence = "not initialized"
    public var decoderHardwareEvidence = "not initialized"
    public init() {}
}

public final class MediaMetrics: @unchecked Sendable {
    private let lock = NSLock()
    private var counters = MediaStatistics()
    private var encodeTimes: [Double] = []
    private var decodeTimes: [Double] = []
    public init() {}
    public func update(_ body: (inout MediaStatistics) -> Void) { lock.withLock { body(&counters) } }
    public func resetMeasurement() {
        lock.withLock {
            let old = counters
            counters = MediaStatistics()
            counters.hardwareEncoder = old.hardwareEncoder; counters.hardwareDecoder = old.hardwareDecoder
            counters.encoderHardwareEvidence = old.encoderHardwareEvidence
            counters.decoderHardwareEvidence = old.decoderHardwareEvidence
            encodeTimes.removeAll(keepingCapacity: true); decodeTimes.removeAll(keepingCapacity: true)
        }
    }
    public func encode(duration: Double, bytes: Int) {
        lock.withLock {
            counters.encoded += 1; counters.encodedBytes += UInt64(bytes)
            append(duration, to: &encodeTimes)
        }
    }
    public func decode(duration: Double) {
        lock.withLock { counters.decoded += 1; append(duration, to: &decodeTimes) }
    }
    private func append(_ duration: Double, to values: inout [Double]) {
        if values.count >= 600 { values.removeFirst() }
        values.append(duration * 1000)
    }
    public func snapshot() -> MediaStatistics {
        lock.withLock {
            var value = counters
            func percentile(_ times: [Double]) -> Double {
                guard !times.isEmpty else { return 0 }
                return times.sorted()[min(times.count - 1, Int(ceil(Double(times.count) * 0.95)) - 1)]
            }
            value.encodeP95Milliseconds = percentile(encodeTimes)
            value.decodeP95Milliseconds = percentile(decodeTimes)
            return value
        }
    }
}
