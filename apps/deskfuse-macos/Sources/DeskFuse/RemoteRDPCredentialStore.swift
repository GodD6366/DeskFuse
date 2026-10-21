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

    private static func baseQuery(peerID: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: peerID,
        ]
    }

    struct KeychainError: LocalizedError {
        let status: OSStatus
        var errorDescription: String { "无法保存 RDP 凭据（Keychain 错误 \(status)）" }
    }
}
