import Foundation

/// Windows RemoteApp catalog protocol: length-framed JSON over the existing
/// authenticated remote-application QUIC stream. The Windows service sends
/// `RemoteRdpMessage`, the Mac controller sends `RemoteRdpRequest`. This mirrors
/// the schema documented in `docs/remote-application/WINDOWS_RDP_CATALOG.md`.

public struct RemoteRdpApp: Codable, Equatable, Sendable {
    public let name: String
    public let displayName: String
    public let path: String
    public let commandLine: String
    public let iconPath: String
    public let iconIndex: UInt32

    enum CodingKeys: String, CodingKey {
        case name
        case displayName = "display_name"
        case path
        case commandLine = "command_line"
        case iconPath = "icon_path"
        case iconIndex = "icon_index"
    }
}

public enum RemoteRdpMessage: Codable, Sendable {
    case catalog(version: UInt32, available: Bool, reason: String?, apps: [RemoteRdpApp])
    case rdp(version: UInt32, rdp: String)
    case error(version: UInt32, code: String, message: String)

    enum CodingKeys: String, CodingKey {
        case type, version, available, reason, apps, rdp, code, message
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let type = try container.decode(String.self, forKey: .type)
        switch type {
        case "catalog":
            self = .catalog(
                version: try container.decode(UInt32.self, forKey: .version),
                available: try container.decode(Bool.self, forKey: .available),
                reason: try container.decodeIfPresent(String.self, forKey: .reason),
                apps: try container.decodeIfPresent([RemoteRdpApp].self, forKey: .apps) ?? [])
        case "rdp":
            self = .rdp(
                version: try container.decode(UInt32.self, forKey: .version),
                rdp: try container.decode(String.self, forKey: .rdp))
        case "error":
            self = .error(
                version: try container.decode(UInt32.self, forKey: .version),
                code: try container.decode(String.self, forKey: .code),
                message: try container.decode(String.self, forKey: .message))
        default:
            throw DecodingError.dataCorruptedError(
                forKey: .type, in: container,
                debugDescription: "unknown remote-app message type \(type)")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .catalog(let version, let available, let reason, let apps):
            try container.encode("catalog", forKey: .type)
            try container.encode(version, forKey: .version)
            try container.encode(available, forKey: .available)
            try container.encodeIfPresent(reason, forKey: .reason)
            try container.encode(apps, forKey: .apps)
        case .rdp(let version, let rdp):
            try container.encode("rdp", forKey: .type)
            try container.encode(version, forKey: .version)
            try container.encode(rdp, forKey: .rdp)
        case .error(let version, let code, let message):
            try container.encode("error", forKey: .type)
            try container.encode(version, forKey: .version)
            try container.encode(code, forKey: .code)
            try container.encode(message, forKey: .message)
        }
    }
}

public enum RemoteRdpRequest: Codable, Sendable {
    case catalog(version: UInt32)
    case launch(version: UInt32, name: String)

    enum CodingKeys: String, CodingKey {
        case type, version, name
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let type = try container.decode(String.self, forKey: .type)
        switch type {
        case "catalog":
            self = .catalog(version: try container.decode(UInt32.self, forKey: .version))
        case "launch":
            self = .launch(
                version: try container.decode(UInt32.self, forKey: .version),
                name: try container.decode(String.self, forKey: .name))
        default:
            throw DecodingError.dataCorruptedError(
                forKey: .type, in: container,
                debugDescription: "unknown remote-app request type \(type)")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .catalog(let version):
            try container.encode("catalog", forKey: .type)
            try container.encode(version, forKey: .version)
        case .launch(let version, let name):
            try container.encode("launch", forKey: .type)
            try container.encode(version, forKey: .version)
            try container.encode(name, forKey: .name)
        }
    }
}
