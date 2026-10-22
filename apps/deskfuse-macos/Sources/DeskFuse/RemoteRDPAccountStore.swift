import Foundation
import Observation
import Security

/// 一套 Windows 登录账号。备注名和用户名可公开展示，密码只保存在钥匙串。
struct RemoteRDPAccount: Codable, Identifiable, Equatable, Sendable {
    let id: UUID
    var name: String
    var username: String

    /// 菜单与列表中的标题：备注名与用户名相同时只显示一次。
    var title: String { name == username ? username : "\(name)（\(username)）" }
}

enum RemoteRDPAccountError: LocalizedError, Equatable {
    case emptyUsername
    case emptyPassword
    case keychain(OSStatus)

    var errorDescription: String? {
        switch self {
        case .emptyUsername: "请填写 Windows 用户名。"
        case .emptyPassword: "请填写 Windows 账户密码。"
        case .keychain(let status): "无法保存 Windows 登录密码（钥匙串错误 \(status)）"
        }
    }
}

/// 密码存取边界，测试用内存实现替换钥匙串。
protocol RemoteRDPSecretStore {
    func password(for id: UUID) -> String?
    func setPassword(_ password: String, for id: UUID) throws
    func removePassword(for id: UUID)
    /// 旧版“每台设备一套凭据”的条目，只用于一次性迁移。
    func legacyCredentials() -> [RemoteRDPLegacyCredential]
    func removeLegacyCredential(peerID: String)
}

struct RemoteRDPLegacyCredential: Equatable {
    let peerID: String
    let username: String
    let password: String
}

/// Windows 远程应用的登录账号库。所有 Windows 设备共用同一份账号列表，
/// 每台设备单独记住上次使用的账号；密码从不经过 DeskFuse 目录通道。
@MainActor @Observable final class RemoteRDPAccountStore {
    static let shared = RemoteRDPAccountStore(defaults: .standard, secrets: RemoteRDPKeychain())

    private(set) var accounts: [RemoteRDPAccount] = []
    @ObservationIgnored private let defaults: UserDefaults
    @ObservationIgnored private let secrets: RemoteRDPSecretStore

    private static let accountsKey = "remote-rdp-accounts"
    private static func lastAccountKey(_ peerID: String) -> String { "remote-rdp-last-account.\(peerID)" }

    init(defaults: UserDefaults, secrets: RemoteRDPSecretStore) {
        self.defaults = defaults
        self.secrets = secrets
        if let data = defaults.data(forKey: Self.accountsKey),
           let stored = try? JSONDecoder().decode([RemoteRDPAccount].self, from: data) {
            accounts = stored
        }
        migrateLegacyCredentials()
    }

    func password(for account: RemoteRDPAccount) -> String? {
        secrets.password(for: account.id)
    }

    /// 新建或更新账号。新建时必须提供密码；编辑时密码留空表示保留原密码。
    /// 新建的用户名已存在时更新那一条，避免同一账号重复出现。
    @discardableResult
    func save(id: UUID?, name: String, username: String, password: String) throws -> RemoteRDPAccount {
        let username = username.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !username.isEmpty else { throw RemoteRDPAccountError.emptyUsername }
        let trimmedName = name.trimmingCharacters(in: .whitespacesAndNewlines)
        let existing = id.flatMap { id in accounts.firstIndex { $0.id == id } }
            ?? accounts.firstIndex { $0.username.caseInsensitiveCompare(username) == .orderedSame }
        if existing == nil, password.isEmpty { throw RemoteRDPAccountError.emptyPassword }
        let account = RemoteRDPAccount(id: existing.map { accounts[$0].id } ?? UUID(),
                                       name: trimmedName.isEmpty ? username : trimmedName,
                                       username: username)
        // 先写密码：钥匙串失败时账号列表保持原样，不会留下没有密码的账号。
        if !password.isEmpty { try secrets.setPassword(password, for: account.id) }
        if let existing { accounts[existing] = account } else { accounts.append(account) }
        persist()
        return account
    }

    func remove(_ id: UUID) {
        accounts.removeAll { $0.id == id }
        secrets.removePassword(for: id)
        persist()
    }

    /// 上次在这台设备上使用的账号；账号已删除时返回 nil。
    func lastAccountID(peerID: String) -> UUID? {
        guard let raw = defaults.string(forKey: Self.lastAccountKey(peerID)),
              let id = UUID(uuidString: raw), accounts.contains(where: { $0.id == id }) else { return nil }
        return id
    }

    func markUsed(_ id: UUID, peerID: String) {
        defaults.set(id.uuidString, forKey: Self.lastAccountKey(peerID))
    }

    private func persist() {
        if let data = try? JSONEncoder().encode(accounts) { defaults.set(data, forKey: Self.accountsKey) }
    }

    private func migrateLegacyCredentials() {
        // 写入新账号库成功后才删除旧条目，钥匙串失败时下次启动重试，不丢密码。
        for legacy in secrets.legacyCredentials() {
            guard let account = try? save(id: nil, name: "", username: legacy.username,
                                           password: legacy.password) else { continue }
            if defaults.string(forKey: Self.lastAccountKey(legacy.peerID)) == nil {
                markUsed(account.id, peerID: legacy.peerID)
            }
            secrets.removeLegacyCredential(peerID: legacy.peerID)
        }
    }
}

/// 钥匙串实现：每个账号一条 generic password，账号字段为账号 UUID。
struct RemoteRDPKeychain: RemoteRDPSecretStore {
    private static let service = "cc.godd.DeskFuse.remote-rdp.account"
    private static let legacyService = "cc.godd.DeskFuse.remote-rdp"
    /// 本应用写入钥匙串的全部服务名；彻底卸载时据此清理。
    static let allServices = [service, legacyService]

    func password(for id: UUID) -> String? {
        var query = Self.query(service: Self.service, account: id.uuidString)
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess,
              let data = result as? Data, let password = String(data: data, encoding: .utf8),
              !password.isEmpty else { return nil }
        return password
    }

    func setPassword(_ password: String, for id: UUID) throws {
        var query = Self.query(service: Self.service, account: id.uuidString)
        let attributes: [String: Any] = [
            kSecValueData as String: Data(password.utf8),
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
        ]
        let status = SecItemUpdate(query as CFDictionary, attributes as CFDictionary)
        if status == errSecItemNotFound {
            query.merge(attributes) { _, new in new }
            let added = SecItemAdd(query as CFDictionary, nil)
            guard added == errSecSuccess else { throw RemoteRDPAccountError.keychain(added) }
        } else if status != errSecSuccess {
            throw RemoteRDPAccountError.keychain(status)
        }
    }

    func removePassword(for id: UUID) {
        _ = SecItemDelete(Self.query(service: Self.service, account: id.uuidString) as CFDictionary)
    }

    func legacyCredentials() -> [RemoteRDPLegacyCredential] {
        // 文件钥匙串不支持 MatchLimitAll 同时返回数据：先列账号名，再逐条读取。
        let list: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: Self.legacyService,
            kSecReturnAttributes as String: true,
            kSecMatchLimit as String: kSecMatchLimitAll,
        ]
        var result: CFTypeRef?
        guard SecItemCopyMatching(list as CFDictionary, &result) == errSecSuccess,
              let items = result as? [[String: Any]] else { return [] }
        struct Payload: Decodable { let username: String; let password: String }
        return items.compactMap { $0[kSecAttrAccount as String] as? String }.compactMap { peerID in
            var query = Self.query(service: Self.legacyService, account: peerID)
            query[kSecReturnData as String] = true
            query[kSecMatchLimit as String] = kSecMatchLimitOne
            var data: CFTypeRef?
            guard SecItemCopyMatching(query as CFDictionary, &data) == errSecSuccess, let data = data as? Data,
                  let payload = try? JSONDecoder().decode(Payload.self, from: data),
                  !payload.username.isEmpty, !payload.password.isEmpty else { return nil }
            return RemoteRDPLegacyCredential(peerID: peerID, username: payload.username, password: payload.password)
        }
    }

    func removeLegacyCredential(peerID: String) {
        _ = SecItemDelete(Self.query(service: Self.legacyService, account: peerID) as CFDictionary)
    }

    private static func query(service: String, account: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
    }
}
