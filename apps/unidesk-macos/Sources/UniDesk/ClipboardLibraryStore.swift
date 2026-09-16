import Foundation
import Observation
import UniDeskEngine

struct ClipboardLibraryRule: Codable, Equatable, Identifiable {
    var id = UUID().uuidString
    var field = "text"
    var op = "contains"
    var value = ""
    var value2 = ""
    enum CodingKeys: String, CodingKey { case field, op, value, value2 }

    // The editor-only ID is regenerated on decoding and is not persisted.
    // Compare rule values so an unchanged poll does not invalidate open menus.
    static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.field == rhs.field && lhs.op == rhs.op && lhs.value == rhs.value && lhs.value2 == rhs.value2
    }
}
struct ClipboardLibraryList: Codable, Equatable, Identifiable {
    var id = UUID().uuidString
    var name = ""
    var kind = "manual"
    var match_mode = "all"
    var position: Int = 0
    var rules: [ClipboardLibraryRule] = []
}
struct ClipboardLibraryChoice: Codable, Equatable, Identifiable { var id: String; var name: String }
struct ClipboardLibraryApp: Codable, Equatable, Identifiable { var bundle_id: String; var name: String; var id: String { bundle_id } }
struct ClipboardLibraryShortcut: Codable, Equatable { var enabled = true; var key_code: UInt16 = 9; var modifiers: UInt64 = 0x180000 }
struct ClipboardLibrarySettings: Codable, Equatable { var retention_days = 30; var excluded_apps: [ClipboardLibraryApp] = []; var shortcut: ClipboardLibraryShortcut?; var sync_index: Bool? }
struct ClipboardLibraryItem: Codable {
    var id: UInt64
    var copied_at_ms: UInt64?
    var mime: String
    var length: UInt64
    var preview: String
    var source: String
    var has_image: Bool
    var file_count: UInt32
    var memberships: [String]?
    var source_device_id: String?
    var source_app_id: String?
    var title: String?
    var ocr_text: String?
    var ocr_version: UInt64?
    var remote: Bool?
    var available: Bool?
    /// Engine page order key: `COALESCE(copied, imported)`. Optional so a page
    /// from an older engine still decodes and falls back to `copied_at_ms`.
    var sort_at_ms: UInt64?
    var native: HistoryItem { HistoryItem(id: id, sourceAppId: source_app_id, sourceDeviceId: source_device_id, copiedAtMs: copied_at_ms, mime: mime, length: length, preview: preview, source: source, hasImage: has_image, fileCount: file_count) }
}
struct ClipboardLibraryPage: Codable {
    var items: [ClipboardLibraryItem]
    var total: Int
    var next_offset: Int?
}
/// Per-record presentation facts for history rows and cards. Derived from the
/// library projection; not persisted Swift business state.
struct ClipboardItemMeta: Equatable {
    var remote = false
    var available = true
    var originDevice: String?
    var ocrText: String?
}
struct ClipboardLibraryState: Codable {
    var revision: UInt64
    var lists: [ClipboardLibraryList]
    var settings: ClipboardLibrarySettings
    var devices: [ClipboardLibraryChoice]
    var apps: [ClipboardLibraryApp]
}

/// Backend projections only; selection and editing drafts belong to individual views.
@MainActor @Observable final class ClipboardLibraryStore {
    static let shared = ClipboardLibraryStore()
    private(set) var lists: [ClipboardLibraryList] = []
    private(set) var settings = ClipboardLibrarySettings()
    private(set) var devices: [ClipboardLibraryChoice] = []
    private(set) var apps: [ClipboardLibraryApp] = []
    private(set) var revision: UInt64 = 0
    var error: String?

    func displayItem(_ item: ClipboardLibraryItem, snapshot: EngineSnapshot?) -> HistoryItem {
        var result = item.native
        result.source = displaySource(item.source, id: item.source_device_id, snapshot: snapshot)
        return result
    }

    private func displaySource(_ name: String, id: String?, snapshot: EngineSnapshot?) -> String {
        let cleanName = name.trimmingCharacters(in: .whitespacesAndNewlines)
        let candidate = id ?? cleanName.replacingOccurrences(of: "远端 ", with: "")
        if let snapshot {
            if candidate == snapshot.localId || cleanName == "本机" { return "这台 Mac" }
            if let device = snapshot.devices.first(where: { $0.id == candidate }), !device.name.isEmpty {
                return device.name
            }
        }
        // Unknown identities remain distinguishable without displaying a full key.
        let rawID = cleanName.replacingOccurrences(of: "远端 ", with: "")
        if rawID.count >= 32 && rawID.allSatisfy(\.isHexDigit) {
            return "远端设备 · \(rawID.prefix(6))"
        }
        return cleanName.isEmpty ? "未知来源" : cleanName
    }

    func sourceChoices(legacyNames: [String], snapshot: EngineSnapshot? = nil) -> [ClipboardLibraryChoice] {
        var result = devices
        let names = Set(devices.map(\.name))
        for name in Set(legacyNames).sorted() where !names.contains(name) {
            result.append(ClipboardLibraryChoice(id: name, name: name.isEmpty ? "未知来源" : name))
        }
        return result.map { ClipboardLibraryChoice(id: $0.id, name: displaySource($0.name, id: $0.id, snapshot: snapshot)) }
    }

    func query(listID: String?, search: String, source: String?, offset: Int = 0) async throws -> ClipboardLibraryPage {
        guard let client = AppStore.shared.client else { throw LibraryError(message: "引擎尚未就绪") }
        let calendar = Calendar.current
        let today = calendar.startOfDay(for: Date())
        let yesterday = calendar.date(byAdding: .day, value: -1, to: today) ?? today
        let tomorrow = calendar.date(byAdding: .day, value: 1, to: today) ?? today
        var request: [String: Any] = ["op": "items", "list_id": listID as Any? ?? NSNull(), "query": search, "kind": "all",
            "source": source as Any? ?? NSNull(), "offset": offset, "limit": 50,
            "day_start_ms": Int64(today.timeIntervalSince1970 * 1000), "yesterday_start_ms": Int64(yesterday.timeIntervalSince1970 * 1000),
            "day_end_ms": Int64(tomorrow.timeIntervalSince1970 * 1000), "timezone": TimeZone.current.identifier]
        var windows: [String: [String: Int64]] = [:]
        if let list = lists.first(where: { $0.id == listID }) {
            let formatter = DateFormatter(); formatter.locale = Locale(identifier: "en_US_POSIX")
            formatter.calendar = calendar; formatter.timeZone = calendar.timeZone; formatter.dateFormat = "yyyy-MM-dd"
            for (index, rule) in list.rules.enumerated() where rule.field == "date" {
                var start: Date?; var end: Date?
                switch rule.op {
                case "today": start = today; end = tomorrow
                case "yesterday": start = yesterday; end = today
                case "last_days":
                    if let days = Int(rule.value), days > 0 { start = calendar.date(byAdding: .day, value: 1 - days, to: today); end = tomorrow }
                case "range":
                    start = formatter.date(from: rule.value)
                    if let last = formatter.date(from: rule.value2) { end = calendar.date(byAdding: .day, value: 1, to: last) }
                default: break
                }
                if let start, let end, start < end { windows[String(index)] = ["start": Int64(start.timeIntervalSince1970 * 1000), "end": Int64(end.timeIntervalSince1970 * 1000)] }
            }
        }
        request["rule_windows"] = windows
        let data = try JSONSerialization.data(withJSONObject: request)
        let response = try await client.libraryQuery(String(decoding: data, as: UTF8.self))
        let page = try JSONDecoder().decode(ClipboardLibraryPage.self, from: Data(response.utf8))
        return page
    }
    func refresh() async {
        do { _ = try await loadSettings() }
        catch { self.error = error.userFacingMessage }
    }
    func loadSettings() async throws -> ClipboardLibrarySettings {
        guard let client = AppStore.shared.client else { throw LibraryError(message: "引擎尚未就绪") }
        let response = try await client.libraryQuery("{\"op\":\"state\"}")
        return try apply(response)
    }
    private func apply(_ response: String) throws -> ClipboardLibrarySettings {
        let state = try JSONDecoder().decode(ClipboardLibraryState.self, from: Data(response.utf8))
        // Only publish changed projections. Replacing observed menu data every
        // two seconds interrupts macOS submenu tracking even without mutations.
        if lists != state.lists { lists = state.lists }
        if settings != state.settings { settings = state.settings }
        if devices != state.devices { devices = state.devices }
        if apps != state.apps { apps = state.apps }
        if error != nil { error = nil }
        if revision != state.revision { revision = state.revision }
        let value = state.settings.shortcut ?? ClipboardLibraryShortcut()
        let key = value.enabled ? WorkflowKey(key_code: value.key_code, modifiers: value.modifiers) : nil
        if ClipboardPanelController.shared.shortcut != key { ClipboardPanelController.shared.configure(key) }
        return state.settings
    }
    @discardableResult
    func mutate(_ request: [String: Any]) async throws -> ClipboardLibrarySettings {
        guard let client = AppStore.shared.client else { throw LibraryError(message: "引擎尚未就绪") }
        let data = try JSONSerialization.data(withJSONObject: request)
        // The mutation returns the persisted state. Do not hide read-back failures
        // behind a separate refresh that swallows errors.
        let response = try await client.libraryMutate(String(decoding: data, as: UTF8.self))
        return try apply(response)
    }
    func save(_ list: ClipboardLibraryList) async throws {
        let object = try JSONSerialization.jsonObject(with: JSONEncoder().encode(list))
        try await mutate(["op": "save_list", "list": object])
    }
    func perform(_ request: [String: Any]) {
        Task { do { try await mutate(request) } catch { self.error = error.userFacingMessage } }
    }
    /// Remove history entries (and any frozen manual-list mirrors) by record id.
    func delete(_ ids: [UInt64]) {
        guard !ids.isEmpty else { return }
        perform(["op": "delete_items", "ids": ids])
    }
    /// Empty titles clear the custom label and fall back to the content preview.
    func rename(_ id: UInt64, title: String) {
        perform(["op": "set_item_title", "id": id, "title": title])
    }
    /// Persist on-device OCR output (or an empty result) for search. The version
    /// stamps which recognizer produced it so a newer one can re-run.
    func setOCR(_ id: UInt64, text: String, version: UInt64) {
        perform(["op": "set_item_ocr", "id": id, "text": text, "version": version])
    }
    /// Ask paired devices for a remote index row's payload on demand.
    func wantPayload(_ id: UInt64) {
        perform(["op": "want_item_payload", "id": id])
    }
    /// Publish/accept the cross-device search index.
    func setIndexSync(_ enabled: Bool) {
        perform(["op": "settings", "settings": ["sync_index": enabled]])
    }
    /// Drop already-received remote index rows (payloads are unaffected).
    func clearRemoteIndex() {
        perform(["op": "clear_remote_index"])
    }
}
private struct LibraryError: LocalizedError { let message: String; var errorDescription: String? { message } }
