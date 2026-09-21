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
    private var measuredPresentationRtt: Double = 0
    private var measuredNetworkRtt = 0.05
    private var lastKeyRequest = -Double.infinity
    private var currentBitrate: Int
    private let bitrateFloor: Int
    /// Connection-local delay floor for congestion detection. It follows lower
    /// samples immediately and is allowed to rise only a few milliseconds per
    /// sample, so a steady link never reads as congestion while a deepening
    /// queue still does. Modelled on RustDesk's in-band TestDelay baseline.
    private var delayBaseline = 0.05
    private var hasDelayBaseline = false
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
        // A bitrate-targeted encoder answers a starved target by dropping
        // frames, so keep a floor that still carries motion. High-resolution
        // profiles bottom out near a tenth of their target, matching RustDesk's
        // BR_MIN_HIGH_RESOLUTION ratio.
        self.bitrateFloor = max(750_000, profile.bitrate / 10)
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
            guard let self else { return }
            let started = self.outstanding[sequence]
            // Heartbeats carry a cumulative presented sequence. A late or
            // skipped ACK must still retire every frame at or below it;
            // otherwise one missing dictionary entry permanently occupies
            // the flow-control window and the encoder self-locks.
            self.outstanding = self.outstanding.filter { $0.key > sequence }
            if let started {
                let elapsed = (ProcessInfo.processInfo.systemUptime - started) * 1000
                self.measuredPresentationRtt = max(0.001, elapsed / 1000)
                self.refreshWindowLimit()
                self.metrics.update { $0.presentationRoundTripMs = elapsed }
            }
            self.metrics.update { $0.inFlightFrames = self.outstanding.count }
            self.drain()
        }
    }

    /// Once-per-second feedback. Congestion is an excess over this connection's
    /// own learned delay floor rather than an absolute latency, and a bitrate
    /// target is lowered before anything else because a starved encoder answers
    /// by dropping frames. RustDesk reaches the same conclusions with its
    /// in-band TestDelay probe and video_qos ratio control.
    public func updateNetwork(rttMilliseconds: Double, sendWaitMilliseconds: Double, lossPercent: Double?) {
        queue.async { [weak self] in
            guard let self, self.flowControlled, self.session != nil,
                  !self.lock.withLock({ self.stopped }), rttMilliseconds.isFinite else { return }
            let rtt = min(2, max(0.001, rttMilliseconds / 1000))
            self.measuredNetworkRtt = rtt
            self.refreshWindowLimit()
            let now = ProcessInfo.processInfo.systemUptime
            let stalled = self.expireOutstanding(now: now)

            // Queue delay in front of the newest acknowledged frame. With no
            // acknowledgement yet, fall back to the path RTT instead of a stale
            // sample. Comparing against the learned floor keeps a link whose
            // baseline latency is high but stable out of the congestion path;
            // the earlier absolute `rtt * 2 + 0.05` test saw a 312 ms
            // presentation round trip as congestion on every sample and walked
            // the bitrate from ~48 Mbps down to 2.29 Mbps.
            let observed = self.measuredPresentationRtt > 0 ? self.measuredPresentationRtt : rtt
            if self.hasDelayBaseline {
                if observed < self.delayBaseline { self.delayBaseline = observed }
                else { self.delayBaseline += min(0.005, (observed - self.delayBaseline) * 0.2) }
            } else {
                self.delayBaseline = observed; self.hasDelayBaseline = true
            }
            let excess = max(0, observed - self.delayBaseline)
            // 150 ms of queue growth is the same bar RustDesk uses
            // (DELAY_THRESHOLD_150MS). A single slow write is noise; only a
            // write that itself takes 150 ms is treated as a pressure signal.
            let congested = excess > 0.15 || sendWaitMilliseconds > 150 || (lossPercent ?? 0) > 2 || stalled
            // A full second of excess, or a frame that died waiting for its
            // acknowledgement, cannot wait for three confirmations.
            self.badSamples = congested ? self.badSamples + (excess > 1 || stalled ? 3 : 1) : 0
            self.goodSamples = congested ? 0 : self.goodSamples + 1
            var bitrate = self.currentBitrate
            if self.badSamples >= 3 {
                bitrate = max(self.bitrateFloor, Int(Double(bitrate) * 0.8))
                self.badSamples = 0; self.goodSamples = 0
            } else if self.goodSamples >= 3 {
                // Climb proportionally to what is still missing so a cut is
                // undone in seconds rather than minutes, then settle on target.
                let deficit = max(0, self.profile.bitrate - bitrate)
                bitrate = min(self.profile.bitrate, bitrate + max(250_000, deficit / 4))
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

    /// Bounds queueing to roughly one round trip of frames. Derived from the
    /// path RTT alone: letting the presentation delay enlarge its own budget
    /// meant a growing queue widened the window that fed it.
    private func refreshWindowLimit() {
        windowLimit = min(24, max(6, Int(ceil(profile.framesPerSecond * (measuredNetworkRtt + 0.05))) + 2))
    }

    /// Retires frames whose presentation acknowledgement never arrived and
    /// reports whether any of them sat past a second. A viewer that stops
    /// acknowledging is congestion the delay sample cannot see, so the caller
    /// treats it as pressure instead of letting the window quietly refill.
    @discardableResult
    private func expireOutstanding(now: Double) -> Bool {
        guard flowControlled, !outstanding.isEmpty else { return false }
        let latency = max(0.05, measuredPresentationRtt, measuredNetworkRtt)
        let timeout = min(2, max(0.3, latency * 2.5))
        let expired = outstanding.filter { now - $0.value > timeout }
        guard !expired.isEmpty else { return false }
        let stalled = expired.contains { now - $0.value > 1 }
        for sequence in expired.keys { outstanding.removeValue(forKey: sequence) }
        metrics.update { $0.inFlightFrames = outstanding.count }
        return stalled
    }

    private func drain() {
        guard let session else { return }
        expireOutstanding(now: ProcessInfo.processInfo.systemUptime)
        while inFlight < 2 {
            if flowControlled && outstanding.count >= windowLimit {
                // Re-arm submit(). Returning with scheduled=true used to leave
                // the encoder permanently asleep after the window filled.
                lock.withLock { scheduled = false }
                return
            }
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
