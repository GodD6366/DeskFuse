import Foundation

public struct RemoteMediaDiagnostics: Codable, Sendable {
    public var media: MediaStatistics
    public var sentPackets: UInt64
    public var lostPackets: UInt64
    public var rttMilliseconds: Double
    public var recentLossPercent: Double?
    public var sendWaitMilliseconds: Double?
    public var remoteAddress: String?
    public init(media: MediaStatistics, sentPackets: UInt64, lostPackets: UInt64, rttMilliseconds: Double) {
        self.media = media; self.sentPackets = sentPackets; self.lostPackets = lostPackets
        self.rttMilliseconds = rttMilliseconds
    }
}

public enum RemoteApplicationDisplayMode: String, Codable, Sendable {
    case window, virtualDisplay

    public func captureDisplayID(assigned: UInt32?) throws -> UInt32? {
        if self == .window { return nil }
        guard let assigned, assigned != 0 else {
            throw RemoteApplicationError.unsupported("远程应用专用屏幕不可用，请重新连接")
        }
        return assigned
    }
}

public struct RemoteMessage: Codable, Sendable {
    public enum Kind: String, Codable, Sendable {
        case hello, acquire, controlReady, applications, launch, surface, layout, frame, presented, input, heartbeat, release, keyFrame, preferences, pause, resume, stop, error
    }
    public var version = 2
    public var kind: Kind
    public var applications: [RemoteApplicationInfo]?
    public var application: UUID?
    /// Launch-only selection; absent on older senders means ordinary windows.
    public var displayMode: RemoteApplicationDisplayMode?
    public var surface: CaptureSurface?
    public var identity: SurfaceIdentity?
    public var profile: VideoProfile?
    public var frame: EncodedVideoFrame?
    public var input: RemoteInputEvent?
    public var sequence: UInt64 = 0
    public var preferences: VideoPreferences?
    public var capabilities: MediaCapabilities?
    public var text: String?
    public var diagnostics: RemoteMediaDiagnostics?
    public init(_ kind: Kind) { self.kind = kind }
}

public enum RemoteWire {
    public static let maximum = EncodedVideoFrame.maximumBytes + 1024 * 1024
    public static func encode(_ message: RemoteMessage) throws -> Data {
        let encoder = PropertyListEncoder(); encoder.outputFormat = .binary
        let payload = try encoder.encode(message)
        guard payload.count <= maximum else { throw RemoteApplicationError.malformedFrame }
        var size = UInt32(payload.count).bigEndian
        var data = withUnsafeBytes(of: &size) { Data($0) }; data.append(payload)
        return data
    }
    public static func decode(_ data: Data) throws -> RemoteMessage {
        guard !data.isEmpty, data.count <= maximum else { throw RemoteApplicationError.malformedFrame }
        let message = try PropertyListDecoder().decode(RemoteMessage.self, from: data)
        guard message.version == 2 else { throw RemoteApplicationError.unsupported("两端测试程序版本不一致，请同时更新两端") }
        if let frame = message.frame { try frame.validate() }
        return message
    }
}
