import Foundation
import Security

/// Stores an RDP username/password only when the user explicitly opts in.
/// The password never crosses the QUIC catalog channel and is read only by the
/// local RDP client at launch time.
enum RemoteRDPCredentialStore {
    struct Credential: Equatable {
        let username: String
        let password: String
    }

    private static let service = "cc.godd.DeskFuse.remote-rdp"
    private static let certificateService = "cc.godd.DeskFuse.remote-rdp-certificate"

    private struct Payload: Codable {
        let username: String
        let password: String
    }

    static func load(peerID: String) -> Credential? {
        var query = baseQuery(peerID: peerID)
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess,
              let data = result as? Data,
              let payload = try? JSONDecoder().decode(Payload.self, from: data),
              !payload.username.isEmpty, !payload.password.isEmpty else { return nil }
        return Credential(username: payload.username, password: payload.password)
    }

    static func save(_ credential: Credential, peerID: String) throws {
        let data = try JSONEncoder().encode(Payload(username: credential.username, password: credential.password))
        var query = baseQuery(peerID: peerID)
        let attributes: [String: Any] = [
            kSecValueData as String: data,
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
        ]
        let status = SecItemUpdate(query as CFDictionary, attributes as CFDictionary)
        if status == errSecItemNotFound {
            query.merge(attributes) { _, new in new }
            let addStatus = SecItemAdd(query as CFDictionary, nil)
            guard addStatus == errSecSuccess else { throw KeychainError(status: addStatus) }
        } else if status != errSecSuccess {
            throw KeychainError(status: status)
        }
    }

    static func remove(peerID: String) {
        _ = SecItemDelete(baseQuery(peerID: peerID) as CFDictionary)
    }

    /// RDP certificate trust is stored separately from credentials so turning
    /// off "remember password" never persists the Windows password as a side
    /// effect of trusting a server certificate.
    static func loadCertificateFingerprint(peerID: String) -> String? {
        var query = certificateQuery(peerID: peerID)
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess,
              let data = result as? Data,
              let value = String(data: data, encoding: .utf8),
              isValidFingerprint(value) else { return nil }
        return value
    }

    static func saveCertificateFingerprint(_ fingerprint: String, peerID: String) throws {
        guard isValidFingerprint(fingerprint) else { throw CertificateKeychainError(status: errSecParam) }
        let data = Data(fingerprint.utf8)
        var query = certificateQuery(peerID: peerID)
        let attributes: [String: Any] = [
            kSecValueData as String: data,
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
        ]
        let status = SecItemUpdate(query as CFDictionary, attributes as CFDictionary)
        if status == errSecItemNotFound {
            query.merge(attributes) { _, new in new }
            let addStatus = SecItemAdd(query as CFDictionary, nil)
            guard addStatus == errSecSuccess else { throw CertificateKeychainError(status: addStatus) }
        } else if status != errSecSuccess {
            throw CertificateKeychainError(status: status)
        }
    }

    static func removeCertificateFingerprint(peerID: String) {
        _ = SecItemDelete(certificateQuery(peerID: peerID) as CFDictionary)
    }

    private static func baseQuery(peerID: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: peerID,
        ]
    }

    private static func certificateQuery(peerID: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: certificateService,
            kSecAttrAccount as String: peerID,
        ]
    }

    private static func isValidFingerprint(_ value: String) -> Bool {
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

    struct KeychainError: LocalizedError {
        let status: OSStatus
        var errorDescription: String { "无法保存 RDP 凭据（Keychain 错误 \(status)）" }
    }

    struct CertificateKeychainError: LocalizedError {
        let status: OSStatus
        var errorDescription: String { "无法保存 RDP 证书信任（Keychain 错误 \(status)）" }
    }
}
