import Foundation
@preconcurrency import Security
import CryptoKit

/// Per-sharing-session TLS certificate, separate from the app's stable code
/// signature. The connection code pins its DER hash and contains a client token.
public final class RemoteTLSIdentity: @unchecked Sendable {
    public let identity: sec_identity_t
    public let code: String
    private var temporaryKeychain: SecKeychain?
    private let directory: URL
    public init() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("deskfuse-tls-" + UUID().uuidString)
        self.directory = directory
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false,
                                               attributes: [.posixPermissions: 0o700])
        do {
            func openssl(_ arguments: [String], input: String? = nil) throws {
                let process = Process(); process.executableURL = URL(fileURLWithPath: "/usr/bin/openssl")
                process.arguments = arguments; process.currentDirectoryURL = directory
                process.standardOutput = FileHandle.nullDevice; process.standardError = FileHandle.nullDevice
                let pipe = Pipe(); process.standardInput = pipe
                try process.run()
                if let input { try pipe.fileHandleForWriting.write(contentsOf: Data((input + "\n").utf8)) }
                try pipe.fileHandleForWriting.close(); process.waitUntilExit()
                guard process.terminationStatus == 0 else { throw RemoteApplicationError.unsupported("创建临时 QUIC 证书失败") }
            }
            try openssl(["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "2", "-subj", "/CN=DeskFuse-P0",
                         "-keyout", "key.pem", "-out", "cert.pem"])
            let password = RemoteWire.key()
            try openssl(["pkcs12", "-export", "-inkey", "key.pem", "-in", "cert.pem", "-out", "identity.p12", "-passout", "stdin"], input: password)
            var options: [String: Any] = [kSecImportExportPassphrase as String: password]
            if #available(macOS 15, *) {
                options[kSecImportToMemoryOnly as String] = true
            } else {
                // macOS 14 fallback uses a disposable keychain, never the user's
                // login keychain, and removes it when the listener is released.
                var keychain: SecKeychain?
                let path = directory.appendingPathComponent("temporary.keychain").path
                let result = password.withCString { bytes in
                    SecKeychainCreate(path, UInt32(password.utf8.count), bytes, false, nil, &keychain)
                }
                guard result == errSecSuccess, let keychain else { throw RemoteApplicationError.codec("创建临时钥匙串", result) }
                temporaryKeychain = keychain; options[kSecImportExportKeychain as String] = keychain
            }
            var imported: CFArray?
            let data = try Data(contentsOf: directory.appendingPathComponent("identity.p12"))
            let result = SecPKCS12Import(data as CFData, options as CFDictionary, &imported)
            guard result == errSecSuccess, let item = (imported as? [[String: Any]])?.first,
                  let raw = item[kSecImportItemIdentity as String] else { throw RemoteApplicationError.codec("载入临时 QUIC 证书", result) }
            let native = raw as! SecIdentity
            guard let identity = sec_identity_create(native) else { throw RemoteApplicationError.unsupported("创建 TLS 身份失败") }
            var certificate: SecCertificate?
            guard SecIdentityCopyCertificate(native, &certificate) == errSecSuccess, let certificate else {
                throw RemoteApplicationError.unsupported("读取 TLS 证书失败")
            }
            self.identity = identity
            let fingerprint = Data(SHA256.hash(data: SecCertificateCopyData(certificate) as Data))
            guard let token = Data(base64Encoded: RemoteWire.key()) else { throw RemoteApplicationError.unsupported("无法生成连接密钥") }
            code = (fingerprint + token).base64EncodedString()
            // Certificate/private key now live in Security objects only.
            for filename in ["key.pem", "cert.pem", "identity.p12"] {
                try? FileManager.default.removeItem(at: directory.appendingPathComponent(filename))
            }
        } catch {
            if let temporaryKeychain { SecKeychainDelete(temporaryKeychain) }
            try? FileManager.default.removeItem(at: directory)
            throw error
        }
    }
    deinit {
        if let temporaryKeychain { SecKeychainDelete(temporaryKeychain) }
        try? FileManager.default.removeItem(at: directory)
    }
    public static func token(code: String) throws -> String {
        guard let bytes = Data(base64Encoded: code.trimmingCharacters(in: .whitespacesAndNewlines)), bytes.count == 64 else {
            throw RemoteApplicationError.unsupported("请完整粘贴 B 端的连接密钥")
        }
        return bytes.suffix(32).base64EncodedString()
    }
    public static func matches(_ first: String, _ second: String) -> Bool {
        guard let a = Data(base64Encoded: first), let b = Data(base64Encoded: second), a.count == b.count else { return false }
        return zip(a, b).reduce(UInt8(0)) { $0 | ($1.0 ^ $1.1) } == 0
    }
}
