import Foundation
@preconcurrency import VideoToolbox

public final class VideoEncoder: @unchecked Sendable {
    private let queue = DispatchQueue(label: "DeskFuse.remote.encode", qos: .userInteractive)
    private let lock = NSLock()
    private var latest: CapturedVideoFrame?
    private var lastCaptured: CapturedVideoFrame?
    private var lastTimestamp = CMTime.invalid
    private var scheduled = false
    private var stopped = false
    private var session: VTCompressionSession?
    private var forceKeyFrame = true
    private var sequence: UInt64 = 0
    private var inFlight = 0
    private let flowControlled: Bool
    private var outstanding: [UInt64: Double] = [:]
    private var windowLimit = 6
    private var lastKeyRequest = -Double.infinity
    private var currentBitrate: Int
    private var badSamples = 0
    private var goodSamples = 0
    private let surface: SurfaceIdentity
    public let profile: VideoProfile
    private let metrics: MediaMetrics
    private let output: @Sendable (Result<EncodedVideoFrame, RemoteApplicationError>) -> Void

    public init(surface: SurfaceIdentity, profile: VideoProfile, metrics: MediaMetrics, flowControlled: Bool = false,
                output: @escaping @Sendable (Result<EncodedVideoFrame, RemoteApplicationError>) -> Void) throws {
        guard profile.pixels.isValid, profile.framesPerSecond.isFinite, profile.framesPerSecond > 0,
              profile.framesPerSecond <= 120, profile.bitrate > 0 else { throw RemoteApplicationError.invalidGeometry }
        self.surface = surface; self.profile = profile; self.metrics = metrics; self.output = output
        self.flowControlled = flowControlled; self.currentBitrate = profile.bitrate
        var created: VTCompressionSession?
        let specification: [CFString: Any] = [
            kVTVideoEncoderSpecification_RequireHardwareAcceleratedVideoEncoder: true,
            kVTVideoEncoderSpecification_EnableLowLatencyRateControl: true,
        ]
        let status = VTCompressionSessionCreate(allocator: kCFAllocatorDefault,
            width: Int32(profile.pixels.width), height: Int32(profile.pixels.height), codecType: profile.codec.codecType,
            encoderSpecification: specification as CFDictionary, imageBufferAttributes: nil,
            compressedDataAllocator: nil, outputCallback: nil, refcon: nil, compressionSessionOut: &created)
        guard status == noErr, let created else { throw RemoteApplicationError.codec("创建 \(profile.codec.label) 硬件编码器", status) }
        session = created
        do {
            try set(kVTCompressionPropertyKey_RealTime, kCFBooleanTrue)
            try set(kVTCompressionPropertyKey_AllowFrameReordering, kCFBooleanFalse)
            let profileLevel: CFTypeRef = profile.codec == .hevc
                ? kVTProfileLevel_HEVC_Main_AutoLevel
                : kVTProfileLevel_H264_High_AutoLevel
            try set(kVTCompressionPropertyKey_ProfileLevel, profileLevel)
            try set(kVTCompressionPropertyKey_ExpectedFrameRate, NSNumber(value: profile.framesPerSecond))
            try set(kVTCompressionPropertyKey_AverageBitRate, NSNumber(value: profile.bitrate))
            metrics.update { $0.currentBitrate = profile.bitrate }
            try set(kVTCompressionPropertyKey_MaxKeyFrameIntervalDuration, NSNumber(value: 1))
            let ready = VTCompressionSessionPrepareToEncodeFrames(created)
            guard ready == noErr else { throw RemoteApplicationError.codec("准备硬件编码", ready) }
            var value: Unmanaged<CFTypeRef>?
            let read = VTSessionCopyProperty(created, key: kVTCompressionPropertyKey_UsingHardwareAcceleratedVideoEncoder,
                                             allocator: kCFAllocatorDefault, valueOut: &value)
            let property = value?.takeRetainedValue()
            // The macOS 27 low-latency hardware encoder does not expose this
            // optional read property. RequireHardware was passed at creation;
            // VT must fail rather than choose a software encoder. Confirmation
            // is only published after a real encoded frame is returned.
            guard (read == noErr && (property as? NSNumber)?.boolValue == true) || read == kVTPropertyNotSupportedErr else {
                throw RemoteApplicationError.unsupported("目标配置的硬件编码状态未确认（查询状态 \(read)，值 \(String(describing: property))）")
            }
            metrics.update { $0.encoderHardwareEvidence = read == noErr ? "VT UsingHardware=true" : "VT RequireHardware=true; optional status property unsupported" }
        } catch {
            VTCompressionSessionInvalidate(created); session = nil; throw error
        }
    }

    private func set(_ key: CFString, _ value: CFTypeRef) throws {
        guard let session else { return }
        let status = VTSessionSetProperty(session, key: key, value: value)
        guard status == noErr else { throw RemoteApplicationError.codec("配置 \(key)", status) }
    }

    public func submit(_ frame: CapturedVideoFrame) {
        guard frame.surface == surface else { return }
        let wake = lock.withLock { () -> Bool in
            guard !stopped else { return false }
            if latest != nil { metrics.update { $0.replacedBeforeEncode += 1 } }
            latest = frame
            lastCaptured = frame
            guard !scheduled else { return false }
            scheduled = true; return true
        }
        if wake { queue.async { [weak self] in self?.drain() } }
    }

    public func requestKeyFrame() {
        queue.async { [weak self] in
            guard let self, !self.lock.withLock({ self.stopped }) else { return }
            let now = ProcessInfo.processInfo.systemUptime
            guard now - self.lastKeyRequest >= 0.5 else { return }
            self.lastKeyRequest = now
            self.forceKeyFrame = true
            self.lock.withLock {
                if self.latest == nil { self.latest = self.lastCaptured }
            }
            // A broken reference chain cannot produce presentation ACKs. Start
            // a fresh bounded window for the requested key frame.
            self.outstanding.removeAll()
            self.drain()
        }
    }

    public func acknowledge(surface: SurfaceIdentity, sequence: UInt64) {
        guard surface == self.surface else { return }
        queue.async { [weak self] in
            guard let self, let started = self.outstanding[sequence] else { return }
            let elapsed = (ProcessInfo.processInfo.systemUptime - started) * 1000
            self.outstanding = self.outstanding.filter { $0.key > sequence }
            self.metrics.update {
                $0.presentationRoundTripMs = elapsed
                $0.inFlightFrames = self.outstanding.count
            }
            self.drain()
        }
    }

    /// Once-per-second feedback. Lower bitrate before dropping frame rate; the
    /// capture mailbox keeps only the latest raw frame while the window is full.
    public func updateNetwork(rttMilliseconds: Double, sendWaitMilliseconds: Double, lossPercent: Double?) {
        queue.async { [weak self] in
            guard let self, self.flowControlled, self.session != nil,
                  !self.lock.withLock({ self.stopped }), rttMilliseconds.isFinite else { return }
            let rtt = min(2, max(0.001, rttMilliseconds / 1000))
            self.windowLimit = min(12, max(3, Int(ceil(self.profile.framesPerSecond * (rtt + 0.05))) + 2))
            let oldest = self.outstanding.values.min().map { ProcessInfo.processInfo.systemUptime - $0 } ?? 0
            let congested = oldest > max(0.2, rtt * 2 + 0.05) || sendWaitMilliseconds > 30 || (lossPercent ?? 0) > 2
            self.badSamples = congested ? self.badSamples + 1 : 0
            self.goodSamples = congested ? 0 : self.goodSamples + 1
            var bitrate = self.currentBitrate
            if self.badSamples >= 3 {
                bitrate = max(min(500_000, self.profile.bitrate), Int(Double(bitrate) * 0.8))
                self.badSamples = 0
            } else if self.goodSamples >= 5 {
                bitrate = min(self.profile.bitrate, Int(Double(bitrate) * 1.05))
                self.goodSamples = 0
            }
            if bitrate != self.currentBitrate {
                do {
                    try self.set(kVTCompressionPropertyKey_AverageBitRate, NSNumber(value: bitrate))
                    self.currentBitrate = bitrate
                    self.metrics.update { $0.currentBitrate = bitrate }
                } catch {
                    self.metrics.update { $0.bitrateUpdateFailures = ($0.bitrateUpdateFailures ?? 0) + 1 }
                }
            }
            self.drain()
        }
    }

    private func drain() {
        guard let session else { return }
        while inFlight < 2 {
            if flowControlled && outstanding.count >= windowLimit { return }
            guard let frame = lock.withLock({ () -> CapturedVideoFrame? in
                guard !stopped else { latest = nil; scheduled = false; return nil }
                guard let frame = latest else { scheduled = false; return nil }
                latest = nil; return frame
            }) else { return }
            guard CVPixelBufferGetWidth(frame.buffer) == profile.pixels.width,
                  CVPixelBufferGetHeight(frame.buffer) == profile.pixels.height else {
                output(.failure(.invalidGeometry)); continue
            }
            sequence += 1; let frameSequence = sequence
            if flowControlled {
                outstanding[frameSequence] = ProcessInfo.processInfo.systemUptime
                metrics.update { $0.inFlightFrames = outstanding.count }
            }
            let properties = forceKeyFrame ? [kVTEncodeFrameOptionKey_ForceKeyFrame: true] as CFDictionary : nil
            forceKeyFrame = false; inFlight += 1
            metrics.update { $0.encoderSubmitted += 1 }
            let started = ProcessInfo.processInfo.systemUptime
            let duration = CMTime(seconds: 1 / profile.framesPerSecond, preferredTimescale: 1_000_000)
            let timestamp = lastTimestamp.isValid && CMTimeCompare(frame.timestamp, lastTimestamp) <= 0
                ? CMTimeAdd(lastTimestamp, duration) : frame.timestamp
            lastTimestamp = timestamp
            let status = VTCompressionSessionEncodeFrame(session, imageBuffer: frame.buffer,
                presentationTimeStamp: timestamp, duration: duration,
                frameProperties: properties, infoFlagsOut: nil) { [weak self] status, flags, sample in
                    guard let self else { return }
                    // Real-time VT encoding may successfully drop a frame. This
                    // is not a codec failure and must not disconnect the viewer.
                    // Submission sequence numbers now have a gap, so resume with
                    // a key frame for the receiver's reference-chain validation.
                    let dropped = status == noErr && (flags.contains(.frameDropped) || sample == nil)
                    self.metrics.update {
                        $0.encoderCallbacks += 1
                        if dropped { $0.encoderDropped += 1 }
                    }
                    if status != noErr {
                        self.output(.failure(.codec("编码视频帧", status)))
                    } else if !dropped, let sample {
                        do {
                            let encoded = try Self.extract(sample, codec: self.profile.codec,
                                                           surface: self.surface, sequence: frameSequence)
                            self.metrics.update { $0.hardwareEncoder = true }
                            self.metrics.encode(duration: ProcessInfo.processInfo.systemUptime - started, bytes: encoded.bytes.count)
                            if !self.lock.withLock({ self.stopped }) { self.output(.success(encoded)) }
                        } catch { self.output(.failure(error as? RemoteApplicationError ?? .malformedFrame)) }
                    }
                    self.queue.async { [weak self] in
                        guard let self else { return }
                        if dropped { self.forceKeyFrame = true }
                        if dropped || status != noErr { self.outstanding.removeValue(forKey: frameSequence) }
                        self.inFlight = max(0, self.inFlight - 1); self.drain()
                    }
                }
            if status != noErr {
                outstanding.removeValue(forKey: frameSequence)
                inFlight -= 1; forceKeyFrame = true
                output(.failure(.codec("提交编码帧", status)))
            }
        }
    }

    public func stop() {
        lock.withLock { stopped = true; latest = nil; lastCaptured = nil }
        queue.sync {
            if let session {
                VTCompressionSessionCompleteFrames(session, untilPresentationTimeStamp: .invalid)
                VTCompressionSessionInvalidate(session)
            }
            session = nil
        }
    }

    deinit { if let session { VTCompressionSessionInvalidate(session) } }

    private static func extract(_ sample: CMSampleBuffer, codec: VideoCodec,
                                surface: SurfaceIdentity, sequence: UInt64) throws -> EncodedVideoFrame {
        guard CMSampleBufferDataIsReady(sample), let block = CMSampleBufferGetDataBuffer(sample),
              let format = CMSampleBufferGetFormatDescription(sample) else { throw RemoteApplicationError.malformedFrame }
        let size = CMBlockBufferGetDataLength(block)
        guard size > 0, size <= EncodedVideoFrame.maximumBytes else { throw RemoteApplicationError.malformedFrame }
        var bytes = Data(count: size)
        let copied = bytes.withUnsafeMutableBytes { CMBlockBufferCopyDataBytes(block, atOffset: 0, dataLength: size, destination: $0.baseAddress!) }
        guard copied == noErr else { throw RemoteApplicationError.codec("读取编码帧", copied) }
        let attachments = CMSampleBufferGetSampleAttachmentsArray(sample, createIfNecessary: false) as? [[CFString: Any]]
        let key = !(attachments?.first?[kCMSampleAttachmentKey_NotSync] as? Bool ?? false)
        var vps = Data(), sps = Data(), pps = Data()
        if key {
            func parameter(_ index: Int) throws -> Data {
                var pointer: UnsafePointer<UInt8>?
                var length = 0, count = 0
                var header: Int32 = 0
                let result: OSStatus
                if codec == .hevc {
                    result = CMVideoFormatDescriptionGetHEVCParameterSetAtIndex(format, parameterSetIndex: index,
                        parameterSetPointerOut: &pointer, parameterSetSizeOut: &length, parameterSetCountOut: &count,
                        nalUnitHeaderLengthOut: &header)
                } else {
                    result = CMVideoFormatDescriptionGetH264ParameterSetAtIndex(format, parameterSetIndex: index,
                        parameterSetPointerOut: &pointer, parameterSetSizeOut: &length, parameterSetCountOut: &count,
                        nalUnitHeaderLengthOut: &header)
                }
                guard result == noErr, let pointer, length > 0, length <= 65536, header == 4 else { throw RemoteApplicationError.malformedFrame }
                return Data(bytes: pointer, count: length)
            }
            if codec == .hevc {
                vps = try parameter(0); sps = try parameter(1); pps = try parameter(2)
            } else {
                sps = try parameter(0); pps = try parameter(1)
            }
        }
        let pts = CMTimeConvertScale(CMSampleBufferGetPresentationTimeStamp(sample), timescale: 1_000_000_000, method: .default)
        let frame = EncodedVideoFrame(surface: surface, codec: codec, sequence: sequence,
                                      presentationNanoseconds: pts.value, keyFrame: key,
                                      vps: vps, sps: sps, pps: pps, bytes: bytes)
        try frame.validate()
        return frame
    }
}
