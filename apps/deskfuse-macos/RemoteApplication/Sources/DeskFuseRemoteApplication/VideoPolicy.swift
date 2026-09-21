import Foundation
@preconcurrency import VideoToolbox

public enum RemoteApplicationError: Error, LocalizedError, Sendable {
    case invalidGeometry
    case unsupported(String)
    case permission(String)
    case codec(String, Int32)
    case noWindow
    case staleSurface
    case malformedFrame

    public var errorDescription: String? {
        switch self {
        case .invalidGeometry: "显示器或捕获区域的几何信息无效"
        case .unsupported(let message): message
        case .permission(let message): message
        case .codec(let operation, let status): "\(operation)失败（\(status)）"
        case .noWindow: "应用仍在运行，但当前显示器上没有可捕获窗口"
        case .staleSurface: "画面已变化，等待新画面确认"
        case .malformedFrame: "视频帧无效或超出限制"
        }
    }
}

public struct PixelSize: Codable, Equatable, Sendable {
    public let width: Int
    public let height: Int
    public init(width: Int, height: Int) { self.width = width; self.height = height }
    public var label: String { "\(width) × \(height)" }
    public var area: Double { Double(width) * Double(height) }
    public var isValid: Bool { width >= 2 && height >= 2 && width <= 32768 && height <= 32768 }

    // Round up for 4:2:0 alignment: never silently reduce the native resolution.
    public func scaled(_ scale: Double) -> PixelSize {
        func even(_ value: Double) -> Int { max(2, Int(ceil(value / 2)) * 2) }
        return PixelSize(width: even(Double(width) * scale), height: even(Double(height) * scale))
    }
}

public enum ResolutionPreference: String, CaseIterable, Codable, Sendable {
    case native, threeQuarters, half, hd, qhd, uhd
    public var label: String {
        switch self {
        case .native: "跟随系统（原生像素）"
        case .threeQuarters: "原生的 75%"
        case .half: "原生的 50%"
        case .hd: "1080p 像素预算"
        case .qhd: "1440p 像素预算"
        case .uhd: "4K 像素预算"
        }
    }
    public func dimensions(for native: PixelSize) -> PixelSize {
        let scale: Double
        switch self {
        case .native: scale = 1
        case .threeQuarters: scale = 0.75
        case .half: scale = 0.5
        case .hd: scale = min(1, sqrt(1920 * 1080 / native.area))
        case .qhd: scale = min(1, sqrt(2560 * 1440 / native.area))
        case .uhd: scale = min(1, sqrt(3840 * 2160 / native.area))
        }
        return native.scaled(scale)
    }
}

public enum RefreshPreference: Int, CaseIterable, Codable, Sendable {
    case system = 0, hz30 = 30, hz60 = 60, hz90 = 90, hz120 = 120
    public var label: String { self == .system ? "跟随系统（最高 120 Hz）" : "\(rawValue) Hz" }
}

public enum VideoCodec: String, CaseIterable, Codable, Sendable {
    case automatic
    case h264
    case hevc

    public var label: String {
        switch self {
        case .automatic: "自动（优先 H.265）"
        case .h264: "H.264"
        case .hevc: "H.265"
        }
    }

    public var codecType: CMVideoCodecType {
        switch self {
        case .h264, .automatic: kCMVideoCodecType_H264
        case .hevc: kCMVideoCodecType_HEVC
        }
    }
}

public struct MediaCapabilities: Codable, Equatable, Sendable {
    public var h264HardwareDecoder: Bool
    public var hevcHardwareDecoder: Bool

    public init(h264HardwareDecoder: Bool = true, hevcHardwareDecoder: Bool = false) {
        self.h264HardwareDecoder = h264HardwareDecoder
        self.hevcHardwareDecoder = hevcHardwareDecoder
    }

    public static var local: MediaCapabilities {
        MediaCapabilities(
            h264HardwareDecoder: VTIsHardwareDecodeSupported(kCMVideoCodecType_H264),
            hevcHardwareDecoder: VTIsHardwareDecodeSupported(kCMVideoCodecType_HEVC))
    }

    public static let h264Only = MediaCapabilities()

    public func supportsDecoder(_ codec: VideoCodec) -> Bool {
        switch codec {
        case .automatic, .h264: h264HardwareDecoder
        case .hevc: hevcHardwareDecoder
        }
    }
}

public enum AdaptationPreference: String, CaseIterable, Codable, Sendable {
    case clarity, fluidity, fixed
    public var label: String {
        switch self { case .clarity: "清晰度优先"; case .fluidity: "流畅度优先"; case .fixed: "固定配置" }
    }
}

public struct VideoPreferences: Codable, Equatable, Sendable {
    public var codec: VideoCodec = .automatic
    public var resolution: ResolutionPreference = .hd
    public var refresh: RefreshPreference = .hz30
    public var adaptation: AdaptationPreference = .clarity
    public init() {}
}

public struct VideoProfile: Codable, Equatable, Sendable {
    public let codec: VideoCodec
    public let pixels: PixelSize
    public let framesPerSecond: Double
    public var bitrate: Int
    public let reason: String?

    public init(codec: VideoCodec = .h264, pixels: PixelSize, framesPerSecond: Double, bitrate: Int? = nil, reason: String? = nil) {
        self.codec = codec
        self.pixels = pixels
        self.framesPerSecond = framesPerSecond
        self.bitrate = bitrate ?? Int(min(200_000_000, max(2_000_000, pixels.area * framesPerSecond * 0.14)))
        self.reason = reason
    }

    public static func requested(native: PixelSize, sourceHz: Double?, preferences: VideoPreferences,
                                 codec: VideoCodec? = nil) throws -> VideoProfile {
        guard native.isValid else { throw RemoteApplicationError.invalidGeometry }
        if let sourceHz, (!sourceHz.isFinite || sourceHz <= 0) { throw RemoteApplicationError.invalidGeometry }
        let available = min(sourceHz ?? 60, 120)
        let requested = preferences.refresh == .system ? available : Double(preferences.refresh.rawValue)
        let effective = min(requested, available)
        let reason: String? = sourceHz == nil ? "系统未报告刷新率，暂用 60 Hz 上限" :
            (effective < requested ? "远端显示器最多更新 \(Int(available)) Hz" : nil)
        let selectedCodec = codec ?? (preferences.codec == .hevc ? .hevc : .h264)
        return VideoProfile(codec: selectedCodec, pixels: preferences.resolution.dimensions(for: native),
                            framesPerSecond: effective, reason: reason)
    }
}

/// No network-interface type is an input to this policy. A sample is one second
/// of measured pressure, not a count of unchanged/static source frames.
public struct VideoAdaptation: Sendable {
    public let preferred: VideoProfile
    public let preference: AdaptationPreference
    public private(set) var current: VideoProfile
    private var history: [VideoProfile] = []
    private var bad = 0
    private var good = 0

    public init(profile: VideoProfile, preference: AdaptationPreference) {
        preferred = profile; current = profile; self.preference = preference
    }

    /// Three pressured samples to degrade, fifteen healthy samples to recover.
    /// Fixed mode reports pressure to its caller but never changes configuration.
    public mutating func observe(congested: Bool) -> VideoProfile? {
        bad = congested ? bad + 1 : 0
        good = congested ? 0 : good + 1
        guard preference != .fixed else { return nil }
        if good >= 15, let restored = history.popLast() {
            current = restored; good = 0; bad = 0; return current
        }
        guard bad >= 3 else { return nil }
        bad = 0; good = 0
        let next = downgrade()
        guard next != current else { return nil }
        history.append(current); current = next
        return next
    }

    private func downgrade() -> VideoProfile {
        let floor = max(1_000_000, preferred.bitrate / 2)
        if current.bitrate > floor {
            var value = current
            value.bitrate = max(floor, current.bitrate * 3 / 4)
            return value
        }
        let minimumHz = min(60, preferred.framesPerSecond)
        func lowerHz(_ minimum: Double) -> VideoProfile? {
            guard let hz = [120.0, 90, 60, 30].first(where: { $0 < current.framesPerSecond && $0 >= minimum }) else { return nil }
            return VideoProfile(codec: current.codec, pixels: current.pixels, framesPerSecond: hz,
                                bitrate: current.bitrate, reason: "持续性能压力，已降低刷新率")
        }
        func lowerSize() -> VideoProfile? {
            for scale in [0.75, 0.5] {
                let size = preferred.pixels.scaled(scale)
                if size.area < current.pixels.area {
                    return VideoProfile(codec: current.codec, pixels: size, framesPerSecond: current.framesPerSecond,
                                        bitrate: current.bitrate, reason: "持续性能压力，已降低分辨率")
                }
            }
            return nil
        }
        // Clarity reduces >60 Hz first; both automatic modes preserve 60 Hz
        // before falling back below it. The preferred native profile is retained.
        let next = preference == .clarity ? (lowerHz(minimumHz) ?? lowerSize()) : (lowerSize() ?? lowerHz(minimumHz))
        return next ?? lowerHz(30) ?? current
    }
}
