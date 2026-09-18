import Foundation

public struct RemoteMessage: Codable, Sendable {
    public enum Kind: String, Codable, Sendable {
        case hello, acquire, controlReady, applications, launch, surface, layout, frame, presented, input, heartbeat, release, keyFrame, preferences, pause, resume, stop, error
    }
    public var version = 1
    public var kind: Kind
    public var applications: [RemoteApplicationInfo]?
    public var application: UUID?
    public var surface: CaptureSurface?
    public var identity: SurfaceIdentity?
    public var profile: VideoProfile?
    public var frame: EncodedVideoFrame?
    public var input: RemoteInputEvent?
    public var sequence: UInt64 = 0
    public var preferences: VideoPreferences?
    public var text: String?
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
        guard message.version == 1 else { throw RemoteApplicationError.unsupported("两端测试程序版本不一致") }
        if let frame = message.frame { try frame.validate() }
        return message
    }
}
