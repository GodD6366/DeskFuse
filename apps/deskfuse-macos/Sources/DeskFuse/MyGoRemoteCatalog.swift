import Foundation
import DeskFuseEngine
import DeskFuseRemoteApplication

@available(macOS 14.2, *)
@MainActor
enum MyGoRemoteCatalog {
    private static var applicationSessions: [String: RemoteApplicationSession] = [:]
    private static var rdpSessions: [String: RemoteRdpSession] = [:]
    private static var tasks: [String: Task<Void, Never>] = [:]

    static func catalog(peerID: String, refresh: Bool = false) async throws -> Data {
        guard let device = device(peerID) else { throw error("unknown_peer", "未找到已配对设备") }
        let manager = RemoteApplicationManager.shared
        if device.remoteAppRdp {
            guard let session = rdpSessions[peerID] ?? manager.makeWindowsCatalog(for: device) else {
                throw error("catalog_unavailable", "Windows 应用目录不可用")
            }
            rdpSessions[peerID] = session
            if refresh { session.refresh() }
            else if session.phase == .idle { await session.connect() }
            do { try await waitRDP(session) } catch { return try payload(applications: [], status: "error", loading: false, rdp: true, accounts: accounts(), error: message(error), favorites: []) }
            return try payload(applications: session.apps.map(rdpMetadata), status: status(session), loading: false,
                               rdp: true, accounts: accounts(), error: session.unavailableReason,
                               favorites: rdpFavoriteIDs(peerID: peerID, apps: session.apps))
        }

        guard let session = applicationSessions[peerID] ?? manager.makeCatalog(for: device) else {
            throw error("catalog_unavailable", "远程应用目录不可用")
        }
        applicationSessions[peerID] = session
        if refresh { session.refreshApplications() }
        else if !session.connected && !session.busy { tasks[peerID]?.cancel(); tasks[peerID] = Task { await session.start() } }
        do { try await waitApplications(session) } catch { return try payload(applications: [], status: "error", loading: false, rdp: false, accounts: accounts(), error: message(error), favorites: []) }
        let opened = RemoteApplicationManager.shared.openedApplications(peerID: peerID)
        return try payload(applications: session.applications.map { applicationMetadata($0, opened: opened) },
                           status: session.failure == nil ? "ready" : "error",
                           loading: !session.applicationsLoaded, rdp: false, accounts: accounts(), error: session.failure,
                           favorites: [])
    }

    static func close(peerID: String) async {
        tasks[peerID]?.cancel(); tasks.removeValue(forKey: peerID)
        if let session = applicationSessions.removeValue(forKey: peerID) { await RemoteApplicationManager.shared.closeCatalog(session) }
        if let session = rdpSessions.removeValue(forKey: peerID) { session.stop() }
    }

    static func open(peerID: String, applicationID: String, accountID: String? = nil,
                     username: String? = nil, password: String? = nil,
                     saveAccount: Bool = false, accountName: String = "") async throws -> Data {
        guard let device = device(peerID) else { throw error("unknown_peer", "未找到已配对设备") }
        let manager = RemoteApplicationManager.shared
        if device.remoteAppRdp {
            guard let session = rdpSessions[peerID] ?? manager.makeWindowsCatalog(for: device) else { throw error("catalog_unavailable", "Windows 应用目录不可用") }
            rdpSessions[peerID] = session
            if session.phase == .idle { await session.connect() }
            try await waitRDP(session)
            guard let app = session.apps.first(where: { rdpID($0) == applicationID }) else { throw error("unknown_application", "未知的远程应用") }
            // 一次性账号：与原生“使用其他账号”一致，保存失败不阻止本次打开。
            if let username, !username.isEmpty, let password, !password.isEmpty {
                var accountError: String?
                if saveAccount {
                    // 与原生一致：保存失败不阻止本次登录，只提示下次需要重新输入。
                    do {
                        _ = try RemoteRDPAccountStore.shared.save(id: nil, name: accountName, username: username, password: password)
                    } catch {
                        accountError = error.localizedDescription
                    }
                }
                manager.openWindowsApplication(app, session: session, username: username, password: password)
                var payload: [String: Any] = ["opened": true, "application_id": applicationID]
                if let accountError { payload["account_error"] = accountError }
                return try JSONSerialization.data(withJSONObject: payload)
            }
            let account = try selectedAccount(accountID, peerID: peerID)
            guard let secret = account.password else { throw error("account_secret_missing", "Windows 账户密码不可用") }
            manager.openWindowsApplication(app, session: session, username: account.account.username, password: secret)
            return try JSONSerialization.data(withJSONObject: ["opened": true, "application_id": applicationID])
        }
        guard let session = applicationSessions[peerID] else { throw error("catalog_required", "请先读取应用目录") }
        guard let app = session.applications.first(where: { $0.id.uuidString == applicationID || $0.applicationKey == applicationID }) else { throw error("unknown_application", "未知的远程应用") }
        manager.openApplication(app, from: session)
        return try JSONSerialization.data(withJSONObject: ["opened": true, "application_id": applicationID])
    }

    /// 收藏与原生菜单栏选择器共用 UserDefaults 键（按应用名保存）。
    static func setVirtualDisplay(enabled: Bool) {
        RemoteApplicationManager.shared.virtualDisplayEnabled = enabled
    }

    static func setFavorite(peerID: String, applicationID: String, favorite: Bool) throws -> Data {
        guard let device = device(peerID), device.remoteAppRdp else {
            throw error("unsupported_action", "只有 Windows 远程应用支持收藏")
        }
        guard let session = rdpSessions[peerID] else { throw error("catalog_required", "请先读取应用目录") }
        guard let app = session.apps.first(where: { rdpID($0) == applicationID }) else { throw error("unknown_application", "未知的远程应用") }
        var names = rdpFavoriteNames(peerID)
        if favorite { names.insert(app.name) } else { names.remove(app.name) }
        UserDefaults.standard.set(Array(names).sorted(), forKey: favoritesKey(peerID))
        return try JSONSerialization.data(withJSONObject: ["favorites": session.apps.filter { names.contains($0.name) }.map(rdpID)])
    }

    static func saveAccount(id: UUID? = nil, name: String, username: String, password: String) throws -> Data {
        let account = try RemoteRDPAccountStore.shared.save(id: id, name: name, username: username, password: password)
        return try JSONSerialization.data(withJSONObject: ["id": account.id.uuidString, "name": account.name, "username": account.username])
    }

    static func removeAccount(id: UUID) { RemoteRDPAccountStore.shared.remove(id) }

    private static func device(_ id: String) -> Device? { AppStore.shared.snapshot?.devices.first { $0.id == id } }
    private static func waitApplications(_ session: RemoteApplicationSession) async throws {
        try await bounded { session.applicationsLoaded || session.failure != nil }
        if let failure = session.failure { throw error("catalog_failed", failure) }
    }
    private static func waitRDP(_ session: RemoteRdpSession) async throws {
        try await bounded { if case .ready = session.phase { return true }; if case .failed = session.phase { return true }; return false }
        if case .failed(let reason) = session.phase { throw error("catalog_failed", reason) }
    }
    private static func bounded(_ condition: @escaping () -> Bool) async throws {
        let deadline = ContinuousClock.now + .seconds(10)
        while !condition() { if ContinuousClock.now >= deadline { throw error("catalog_timeout", "读取远程应用目录超时") }; try await Task.sleep(for: .milliseconds(50)) }
    }
    private static func applicationMetadata(_ app: RemoteApplicationInfo, opened: Set<String>) -> [String: Any] {
        var meta: [String: Any] = ["id": app.id.uuidString, "name": app.name,
            "bundle_id": app.bundleIdentifier.map { $0 as Any } ?? NSNull(),
            "category": app.category.map { $0 as Any } ?? NSNull(),
            "running": app.processID != nil,
            "opened": opened.contains(app.applicationKey)]
        if let icon = app.iconPNG { meta["icon"] = "data:image/png;base64," + icon.base64EncodedString() }
        return meta
    }
    private static func rdpMetadata(_ app: RemoteRdpApp) -> [String: Any] { ["id": rdpID(app), "name": app.displayName.isEmpty ? app.name : app.displayName, "bundle_id": NSNull(), "category": "windows"] }
    private static func favoritesKey(_ peerID: String) -> String { "remote-rdp-favorites.\(peerID)" }
    private static func rdpFavoriteNames(_ peerID: String) -> Set<String> { Set(UserDefaults.standard.stringArray(forKey: favoritesKey(peerID)) ?? []) }
    private static func rdpFavoriteIDs(peerID: String, apps: [RemoteRdpApp]) -> [String] {
        let names = rdpFavoriteNames(peerID)
        return apps.filter { names.contains($0.name) }.map(rdpID)
    }
    private static func rdpID(_ app: RemoteRdpApp) -> String { app.path.isEmpty ? app.name : app.path }
    private static func accounts() -> [[String: Any]] { RemoteRDPAccountStore.shared.accounts.map { ["id": $0.id.uuidString, "name": $0.name, "username": $0.username] } }
    private static func payload(applications: [[String: Any]], status: String, loading: Bool, rdp: Bool, accounts: [[String: Any]], error: String?, favorites: [String]) throws -> Data {
        var o: [String: Any] = ["applications": applications, "status": status, "loading": loading, "rdp": rdp,
                                "accounts": accounts, "favorites": favorites,
                                "virtual_display": RemoteApplicationManager.shared.virtualDisplayEnabled]
        if let error { o["error"] = error }
        return try JSONSerialization.data(withJSONObject: o)
    }
    private static func status(_ session: RemoteRdpSession) -> String { if case .ready = session.phase { return "ready" }; if case .failed = session.phase { return "error" }; return "loading" }
    private static func selectedAccount(_ id: String?, peerID: String) throws -> (account: RemoteRDPAccount, password: String?) { let store = RemoteRDPAccountStore.shared; let account = id.flatMap { UUID(uuidString: $0) }.flatMap { id in store.accounts.first { $0.id == id } } ?? (store.lastAccountID(peerID: peerID).flatMap { id in store.accounts.first { $0.id == id } }); guard let account else { throw error("unknown_account", "未知的 Windows 账户") }; return (account, store.password(for: account)) }
    private static func message(_ error: Error) -> String { (error as? MyGoBridgeError)?.message ?? error.localizedDescription }
    private static func error(_ code: String, _ message: String) -> MyGoBridgeError { MyGoBridgeError(code: code, message: message) }
}
