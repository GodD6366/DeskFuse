import Foundation
@preconcurrency import VideoToolbox

public final class VideoDecoder: @unchecked Sendable {
    private let queue = DispatchQueue(label: "DeskFuse.remote.decode", qos: .userInteractive)
    private let lock = NSLock()
    private var pending = EncodedFrameQueue()
    private var scheduled = false
    private var stopped = false
    private var session: VTDecompressionSession?
    private var format: CMVideoFormatDescription?
    private var parameterSets: (codec: VideoCodec, vps: Data, sps: Data, pps: Data)?
    private let metrics: MediaMetrics
    private let expectedPixels: PixelSize
    private let expectedCodec: VideoCodec
    private let output: @Sendable (Result<DecodedVideoFrame, RemoteApplicationError>) -> Void
    private let keyFrameNeeded: @Sendable () -> Void

    public init(surface: SurfaceIdentity, expectedPixels: PixelSize, expectedCodec: VideoCodec = .h264, metrics: MediaMetrics,
                keyFrameNeeded: @escaping @Sendable () -> Void,
                output: @escaping @Sendable (Result<DecodedVideoFrame, RemoteApplicationError>) -> Void) {
        pending.reset(to: surface); self.metrics = metrics; self.expectedPixels = expectedPixels; self.expectedCodec = expectedCodec
        self.keyFrameNeeded = keyFrameNeeded; self.output = output
    }

    public func submit(_ frame: EncodedVideoFrame) {
        do { try frame.validate() } catch { invalidate(); return }
        var requestKey = false
        let wake = lock.withLock { () -> Bool in
            guard !stopped else { return false }
            switch pending.offer(frame) {
            case .stale: return false
            case .needsKeyFrame: requestKey = true; return false
            case .accepted:
                guard !scheduled else { return false }
                scheduled = true; return true
            }
        }
        if requestKey { metrics.update { $0.chainResets += 1 }; keyFrameNeeded() }
        if wake { queue.async { [weak self] in self?.drain() } }
    }

    public func invalidate() {
        lock.withLock { pending.invalidate() }
        metrics.update { $0.chainResets += 1 }; keyFrameNeeded()
    }

    private func drain() {
        while let frame = lock.withLock({ () -> EncodedVideoFrame? in
            guard !stopped, let frame = pending.take() else { scheduled = false; return nil }
            return frame
        }) {
            do { try decode(frame) }
            catch {
                output(.failure(error as? RemoteApplicationError ?? .malformedFrame))
                invalidate()
            }
        }
    }

    private func configure(_ frame: EncodedVideoFrame) throws {
        guard frame.keyFrame else { throw RemoteApplicationError.malformedFrame }
        guard frame.codec == expectedCodec else { throw RemoteApplicationError.unsupported("收到的编码格式与协商结果不一致") }
        if let parameterSets, parameterSets.codec == frame.codec,
           parameterSets.vps == frame.vps, parameterSets.sps == frame.sps,
           parameterSets.pps == frame.pps, session != nil { return }
        if let session { VTDecompressionSessionInvalidate(session) }
        session = nil; format = nil
        var description: CMVideoFormatDescription?
        let status: OSStatus
        if frame.codec == .hevc {
            status = frame.vps.withUnsafeBytes { (vps: UnsafeRawBufferPointer) in
                frame.sps.withUnsafeBytes { (sps: UnsafeRawBufferPointer) in
                    frame.pps.withUnsafeBytes { (pps: UnsafeRawBufferPointer) in
                        let pointers = [vps.baseAddress!.assumingMemoryBound(to: UInt8.self),
                                        sps.baseAddress!.assumingMemoryBound(to: UInt8.self),
                                        pps.baseAddress!.assumingMemoryBound(to: UInt8.self)]
                        let sizes = [vps.count, sps.count, pps.count]
                        return CMVideoFormatDescriptionCreateFromHEVCParameterSets(allocator: kCFAllocatorDefault,
                            parameterSetCount: 3, parameterSetPointers: pointers, parameterSetSizes: sizes,
                            nalUnitHeaderLength: 4, extensions: nil, formatDescriptionOut: &description)
                    }
                }
            }
        } else {
            status = frame.sps.withUnsafeBytes { (sps: UnsafeRawBufferPointer) in
                frame.pps.withUnsafeBytes { (pps: UnsafeRawBufferPointer) in
                    let pointers = [sps.baseAddress!.assumingMemoryBound(to: UInt8.self),
                                    pps.baseAddress!.assumingMemoryBound(to: UInt8.self)]
                    let sizes = [sps.count, pps.count]
                    return CMVideoFormatDescriptionCreateFromH264ParameterSets(allocator: kCFAllocatorDefault,
                        parameterSetCount: 2, parameterSetPointers: pointers, parameterSetSizes: sizes,
                            nalUnitHeaderLength: 4, formatDescriptionOut: &description)
                }
            }
        }
        guard status == noErr, let description else { throw RemoteApplicationError.codec("读取 \(frame.codec.label) 配置", status) }
        let dimensions = CMVideoFormatDescriptionGetDimensions(description)
        guard expectedPixels.isValid, Int(dimensions.width) == expectedPixels.width,
              Int(dimensions.height) == expectedPixels.height else { throw RemoteApplicationError.invalidGeometry }
        let specification = [kVTVideoDecoderSpecification_RequireHardwareAcceleratedVideoDecoder: true] as CFDictionary
        let attributes: [CFString: Any] = [
            kCVPixelBufferPixelFormatTypeKey: kCVPixelFormatType_32BGRA,
            kCVPixelBufferMetalCompatibilityKey: true,
            kCVPixelBufferIOSurfacePropertiesKey: [:] as [String: Any],
        ]
        var created: VTDecompressionSession?
        let result = VTDecompressionSessionCreate(allocator: kCFAllocatorDefault, formatDescription: description,
            decoderSpecification: specification, imageBufferAttributes: attributes as CFDictionary,
            outputCallback: nil, decompressionSessionOut: &created)
        guard result == noErr, let created else { throw RemoteApplicationError.codec("创建 \(frame.codec.label) 硬件解码器", result) }
        var hardware: Unmanaged<CFTypeRef>?
        let read = VTSessionCopyProperty(created, key: kVTDecompressionPropertyKey_UsingHardwareAcceleratedVideoDecoder,
                                        allocator: kCFAllocatorDefault, valueOut: &hardware)
        let property = hardware?.takeRetainedValue()
        guard (read == noErr && (property as? NSNumber)?.boolValue == true) || read == kVTPropertyNotSupportedErr else {
            VTDecompressionSessionInvalidate(created)
            throw RemoteApplicationError.unsupported("当前配置无法使用硬件解码")
        }
        let realTime = VTSessionSetProperty(created, key: kVTDecompressionPropertyKey_RealTime, value: kCFBooleanTrue)
        guard realTime == noErr else { VTDecompressionSessionInvalidate(created); throw RemoteApplicationError.codec("配置实时解码", realTime) }
        session = created; format = description
        parameterSets = (frame.codec, frame.vps, frame.sps, frame.pps)
        metrics.update { $0.decoderHardwareEvidence = read == noErr ? "VT UsingHardware=true" : "VT RequireHardware=true; optional status property unsupported" }
    }

    private func decode(_ frame: EncodedVideoFrame) throws {
        if frame.keyFrame { try configure(frame) }
        guard let session, let format else { throw RemoteApplicationError.malformedFrame }
        var block: CMBlockBuffer?
        let allocation = CMBlockBufferCreateWithMemoryBlock(allocator: kCFAllocatorDefault, memoryBlock: nil,
            blockLength: frame.bytes.count, blockAllocator: kCFAllocatorDefault, customBlockSource: nil,
            offsetToData: 0, dataLength: frame.bytes.count, flags: 0, blockBufferOut: &block)
        guard allocation == noErr, let block else { throw RemoteApplicationError.codec("分配解码缓冲", allocation) }
        let copied = frame.bytes.withUnsafeBytes {
            CMBlockBufferReplaceDataBytes(with: $0.baseAddress!, blockBuffer: block, offsetIntoDestination: 0, dataLength: $0.count)
        }
        guard copied == noErr else { throw RemoteApplicationError.codec("复制压缩数据", copied) }
        var timing = CMSampleTimingInfo(duration: .invalid,
            presentationTimeStamp: CMTime(value: frame.presentationNanoseconds, timescale: 1_000_000_000), decodeTimeStamp: .invalid)
        var size = frame.bytes.count
        var sample: CMSampleBuffer?
        let prepared = CMSampleBufferCreateReady(allocator: kCFAllocatorDefault, dataBuffer: block,
            formatDescription: format, sampleCount: 1, sampleTimingEntryCount: 1, sampleTimingArray: &timing,
            sampleSizeEntryCount: 1, sampleSizeArray: &size, sampleBufferOut: &sample)
        guard prepared == noErr, let sample else { throw RemoteApplicationError.codec("准备解码帧", prepared) }
        let started = ProcessInfo.processInfo.systemUptime
        // Synchronous decode on the dedicated worker bounds native in-flight
        // surfaces. The UI thread never waits for VideoToolbox.
        let result = VTDecompressionSessionDecodeFrame(session, sampleBuffer: sample,
            flags: [], infoFlagsOut: nil) { [weak self] status, _, image, timestamp, _ in
                guard let self, !self.lock.withLock({ self.stopped }) else { return }
                guard status == noErr, let image else { self.output(.failure(.codec("解码视频帧", status))); self.invalidate(); return }
                self.metrics.update { $0.hardwareDecoder = true }
                self.metrics.decode(duration: ProcessInfo.processInfo.systemUptime - started)
                self.output(.success(DecodedVideoFrame(surface: frame.surface, sequence: frame.sequence, buffer: image, timestamp: timestamp)))
            }
        guard result == noErr else { throw RemoteApplicationError.codec("提交解码帧", result) }
    }

    public func stop() {
        lock.withLock { stopped = true; pending.invalidate() }
        queue.sync {
            if let session { VTDecompressionSessionWaitForAsynchronousFrames(session); VTDecompressionSessionInvalidate(session) }
            session = nil; format = nil
        }
    }
    deinit { if let session { VTDecompressionSessionInvalidate(session) } }
}

/// P0 local loopback. The compressed callback is the future authenticated QUIC
/// boundary; this class does NOT claim to be a remote connection.
public final class LocalMediaPipeline: @unchecked Sendable {
    public let metrics: MediaMetrics
    public let latest: LatestVideoFrame
    private var encoder: VideoEncoder?
    private var decoder: VideoDecoder?

    public init(surface: SurfaceIdentity, profile: VideoProfile, metrics: MediaMetrics = MediaMetrics(),
                latest: LatestVideoFrame = LatestVideoFrame(),
                onError: @escaping @Sendable (RemoteApplicationError) -> Void) throws {
        self.metrics = metrics; self.latest = latest
        latest.reset(surface)
        decoder = VideoDecoder(surface: surface, expectedPixels: profile.pixels, expectedCodec: profile.codec, metrics: metrics, keyFrameNeeded: { [weak self] in
            self?.encoder?.requestKeyFrame()
        }, output: { result in
            switch result {
            case .success(let frame): latest.publish(frame)
            case .failure(let error): onError(error)
            }
        })
        encoder = try VideoEncoder(surface: surface, profile: profile, metrics: metrics) { [weak self] result in
            switch result {
            case .success(let frame): self?.decoder?.submit(frame)
            case .failure(let error): self?.decoder?.invalidate(); onError(error)
            }
        }
    }
    public func submit(_ frame: CapturedVideoFrame) { encoder?.submit(frame) }
    public func stop() {
        encoder?.stop(); decoder?.stop(); latest.reset(nil)
    }
}
