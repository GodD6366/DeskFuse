import Foundation

/// Windows RemoteApp catalog protocol v2: length-framed JSON over the existing
/// authenticated remote-application QUIC stream. The Windows service sends
/// `RemoteRdpMessage`, the Mac controller sends `RemoteRdpRequest`. This mirrors
/// the schema in `docs/remote-application/WINDOWS_RDP_CATALOG.md`.

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

/// One preflight result reported by the Windows service. The controller shows
/// these verbatim; it never infers a host's readiness from its edition alone.
public struct RemoteRdpHostCheck: Codable, Equatable, Sendable {
    public let name: String
    public let passed: Bool
    public let message: String

    public init(name: String, passed: Bool, message: String) {
        self.name = name
        self.passed = passed
        self.message = message
    }
}

/// RDP listener facts supplied by the paired Windows service. The certificate
/// SHA-256 is the only pin the native bridge accepts; there is no manual trust.
public struct RemoteRdpHost: Codable, Equatable, Sendable {
    public let port: UInt16
    public let certificateSha256: String?
    public let checks: [RemoteRdpHostCheck]

    enum CodingKeys: String, CodingKey {
        case port
        case certificateSha256 = "certificate_sha256"
        case checks
    }

    public init(port: UInt16, certificateSha256: String?, checks: [RemoteRdpHostCheck]) {
        self.port = port
        self.certificateSha256 = certificateSha256
        self.checks = checks
    }

    public var available: Bool {
        port > 0 && Self.isValidFingerprint(certificateSha256) && checks.allSatisfy(\.passed)
    }

    public var failedChecks: [RemoteRdpHostCheck] { checks.filter { !$0.passed } }

    /// 32 SHA-256 bytes rendered as uppercase hex separated by colons, which is
    /// exactly the format FreeRDP compares against.
    public static func isValidFingerprint(_ value: String?) -> Bool {
        guard let value else { return false }
        let bytes = Array(value.utf8)
        guard bytes.count == 95 else { return false }
        for (index, byte) in bytes.enumerated() {
            if index % 3 == 2 {
                guard byte == 0x3a else { return false }
            } else if !((byte >= 0x30 && byte <= 0x39) || (byte >= 0x41 && byte <= 0x46) || (byte >= 0x61 && byte <= 0x66)) {
                return false
            }
        }
        return true
    }
}

/// Thrown before any field is decoded when the peer speaks another catalog
/// version, so callers can tell an upgrade problem from a malformed message.
public enum RemoteRdpProtocolError: Error, Equatable, Sendable {
    case unsupportedVersion(UInt32)
}

public enum RemoteRdpMessage: Codable, Sendable {
    case catalog(version: UInt32, available: Bool, reason: String?, host: RemoteRdpHost, apps: [RemoteRdpApp])
    case ready(version: UInt32, name: String, ticket: Data, rdp: String, host: RemoteRdpHost)
    case error(version: UInt32, code: String, message: String)

    public static let version: UInt32 = 2

    enum CodingKeys: String, CodingKey {
        case type, version, available, reason, host, apps, name, ticket, rdp, code, message
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let type = try container.decode(String.self, forKey: .type)
        let version = try container.decode(UInt32.self, forKey: .version)
        guard version == Self.version else {
            throw RemoteRdpProtocolError.unsupportedVersion(version)
        }
        switch type {
        case "catalog":
            self = .catalog(
                version: Self.version,
                available: try container.decode(Bool.self, forKey: .available),
                reason: try container.decodeIfPresent(String.self, forKey: .reason),
                host: try container.decode(RemoteRdpHost.self, forKey: .host),
                apps: try container.decodeIfPresent([RemoteRdpApp].self, forKey: .apps) ?? [])
        case "ready":
            let ticket = Data(try container.decode([UInt8].self, forKey: .ticket))
            guard ticket.count == 32 else {
                throw DecodingError.dataCorruptedError(forKey: .ticket, in: container,
                    debugDescription: "RDP launch ticket must be 32 bytes")
            }
            self = .ready(
                version: Self.version,
                name: try container.decode(String.self, forKey: .name),
                ticket: ticket,
                rdp: try container.decode(String.self, forKey: .rdp),
                host: try container.decode(RemoteRdpHost.self, forKey: .host))
        case "error":
            self = .error(
                version: Self.version,
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
        case .catalog(let version, let available, let reason, let host, let apps):
            try container.encode("catalog", forKey: .type)
            try container.encode(version, forKey: .version)
            try container.encode(available, forKey: .available)
            try container.encodeIfPresent(reason, forKey: .reason)
            try container.encode(host, forKey: .host)
            try container.encode(apps, forKey: .apps)
        case .ready(let version, let name, let ticket, let rdp, let host):
            try container.encode("ready", forKey: .type)
            try container.encode(version, forKey: .version)
            try container.encode(name, forKey: .name)
            try container.encode([UInt8](ticket), forKey: .ticket)
            try container.encode(rdp, forKey: .rdp)
            try container.encode(host, forKey: .host)
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
    case prepare(version: UInt32, name: String)

    public static let version: UInt32 = 2

    enum CodingKeys: String, CodingKey {
        case type, version, name
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let type = try container.decode(String.self, forKey: .type)
        switch type {
        case "catalog":
            self = .catalog(version: try container.decode(UInt32.self, forKey: .version))
        case "prepare":
            self = .prepare(
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
        case .prepare(let version, let name):
            try container.encode("prepare", forKey: .type)
            try container.encode(version, forKey: .version)
            try container.encode(name, forKey: .name)
        }
    }
}
