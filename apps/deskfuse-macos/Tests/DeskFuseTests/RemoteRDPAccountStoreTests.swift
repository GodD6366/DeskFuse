import XCTest
@testable import DeskFuse

/// 内存密码存储：验证账号库逻辑，不触碰真实钥匙串。
private final class MemorySecrets: RemoteRDPSecretStore {
    var passwords: [UUID: String] = [:]
    var legacy: [RemoteRDPLegacyCredential] = []
    var failWrites = false

    func password(for id: UUID) -> String? { passwords[id] }
    func setPassword(_ password: String, for id: UUID) throws {
        if failWrites { throw RemoteRDPAccountError.keychain(-25299) }
        passwords[id] = password
    }
    func removePassword(for id: UUID) { passwords[id] = nil }
    func legacyCredentials() -> [RemoteRDPLegacyCredential] { legacy }
    func removeLegacyCredential(peerID: String) { legacy.removeAll { $0.peerID == peerID } }
}

@MainActor
final class RemoteRDPAccountStoreTests: XCTestCase {
    private var suite = ""
    private var defaults: UserDefaults!

    override func setUp() {
        suite = "RemoteRDPAccountStoreTests.\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: suite)
    }

    override func tearDown() {
        defaults.removePersistentDomain(forName: suite)
    }

    func testSavedAccountsPersistAndPasswordStaysInSecretStore() throws {
        let secrets = MemorySecrets()
        let store = RemoteRDPAccountStore(defaults: defaults, secrets: secrets)
        let account = try store.save(id: nil, name: "工作", username: " user@example.com ", password: "pw")
        XCTAssertEqual(account.username, "user@example.com")
        XCTAssertEqual(account.title, "工作（user@example.com）")
        XCTAssertEqual(store.password(for: account), "pw")

        let reloaded = RemoteRDPAccountStore(defaults: defaults, secrets: secrets)
        XCTAssertEqual(reloaded.accounts, [account])
        // 账号资料里不能出现密码。
        let raw = String(decoding: defaults.data(forKey: "remote-rdp-accounts") ?? Data(), as: UTF8.self)
        XCTAssertFalse(raw.contains("pw"))
    }

    func testEmptyNameFallsBackToUsername() throws {
        let store = RemoteRDPAccountStore(defaults: defaults, secrets: MemorySecrets())
        let account = try store.save(id: nil, name: "  ", username: "alice", password: "pw")
        XCTAssertEqual(account.name, "alice")
        XCTAssertEqual(account.title, "alice")
    }

    func testNewAccountRequiresUsernameAndPassword() {
        let store = RemoteRDPAccountStore(defaults: defaults, secrets: MemorySecrets())
        XCTAssertThrowsError(try store.save(id: nil, name: "", username: " ", password: "pw")) {
            XCTAssertEqual($0 as? RemoteRDPAccountError, .emptyUsername)
        }
        XCTAssertThrowsError(try store.save(id: nil, name: "", username: "alice", password: "")) {
            XCTAssertEqual($0 as? RemoteRDPAccountError, .emptyPassword)
        }
        XCTAssertTrue(store.accounts.isEmpty)
    }

    func testEditingWithEmptyPasswordKeepsExistingPassword() throws {
        let store = RemoteRDPAccountStore(defaults: defaults, secrets: MemorySecrets())
        let account = try store.save(id: nil, name: "", username: "alice", password: "old")
        let edited = try store.save(id: account.id, name: "家里", username: "alice", password: "")
        XCTAssertEqual(edited.id, account.id)
        XCTAssertEqual(store.password(for: edited), "old")
        XCTAssertEqual(store.accounts.map(\.name), ["家里"])
    }

    func testSavingExistingUsernameUpdatesInsteadOfDuplicating() throws {
        let store = RemoteRDPAccountStore(defaults: defaults, secrets: MemorySecrets())
        let first = try store.save(id: nil, name: "", username: "Alice", password: "old")
        let second = try store.save(id: nil, name: "", username: "alice", password: "new")
        XCTAssertEqual(first.id, second.id)
        XCTAssertEqual(store.accounts.count, 1)
        XCTAssertEqual(store.password(for: second), "new")
    }

    func testKeychainFailureLeavesAccountListUnchanged() {
        let secrets = MemorySecrets()
        secrets.failWrites = true
        let store = RemoteRDPAccountStore(defaults: defaults, secrets: secrets)
        XCTAssertThrowsError(try store.save(id: nil, name: "", username: "alice", password: "pw"))
        XCTAssertTrue(store.accounts.isEmpty)
    }

    func testRemoveDeletesPasswordAndForgetsLastAccount() throws {
        let secrets = MemorySecrets()
        let store = RemoteRDPAccountStore(defaults: defaults, secrets: secrets)
        let account = try store.save(id: nil, name: "", username: "alice", password: "pw")
        store.markUsed(account.id, peerID: "peer")
        XCTAssertEqual(store.lastAccountID(peerID: "peer"), account.id)
        store.remove(account.id)
        XCTAssertNil(secrets.passwords[account.id])
        XCTAssertNil(store.lastAccountID(peerID: "peer"))
    }

    func testLegacyCredentialsMigrateOnceAndRememberPeer() {
        let secrets = MemorySecrets()
        secrets.legacy = [
            RemoteRDPLegacyCredential(peerID: "win-a", username: "alice", password: "pw"),
            RemoteRDPLegacyCredential(peerID: "win-b", username: "alice", password: "pw"),
        ]
        let store = RemoteRDPAccountStore(defaults: defaults, secrets: secrets)
        XCTAssertEqual(store.accounts.count, 1)
        let id = store.accounts[0].id
        XCTAssertEqual(store.lastAccountID(peerID: "win-a"), id)
        XCTAssertEqual(store.lastAccountID(peerID: "win-b"), id)
        XCTAssertTrue(secrets.legacy.isEmpty)
    }

    func testFailedLegacyMigrationKeepsOldCredentialForRetry() {
        let secrets = MemorySecrets()
        secrets.failWrites = true
        secrets.legacy = [RemoteRDPLegacyCredential(peerID: "win-a", username: "alice", password: "pw")]
        let store = RemoteRDPAccountStore(defaults: defaults, secrets: secrets)
        XCTAssertTrue(store.accounts.isEmpty)
        XCTAssertEqual(secrets.legacy.count, 1)
    }
}
