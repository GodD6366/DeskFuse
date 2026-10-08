import Foundation
import Observation
import DeskFuseEngine

enum ClipboardPaging {
    static let pageSize = 20
    /// Keep a small scrolling window in Swift. SQLite remains the source of
    /// truth, so searching or reopening can always fetch older pages again.
    static let maxLoadedItems = 100
}

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
    /// Ordinary history and its frozen manual-list archive share one action.
    var related_ids: [UInt64]?
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
struct ClipboardLibraryCursor: Codable, Equatable {
    var sort_at_ms: UInt64
    var id: UInt64
}
struct ClipboardLibraryPage: Codable {
    var items: [ClipboardLibraryItem]
    var total: Int
    var next_offset: Int?
    var next_cursor: ClipboardLibraryCursor?
    var revision: UInt64?
    var total_exact: Bool?
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
    var counts_revision: UInt64?
    var lists: [ClipboardLibraryList]?
    var settings: ClipboardLibrarySettings?
    var devices: [ClipboardLibraryChoice]?
    var apps: [ClipboardLibraryApp]?
}

/// Backend projections only; selection and editing drafts belong to individual views.
@MainActor @Observable final class ClipboardLibraryStore {
    static let shared = ClipboardLibraryStore()
    private var storedLists: [ClipboardLibraryList] = []
    private var storedListsRevision: UInt64 = 0
    private struct Edit {
        let id: UUID
        let request: [String: Any]
        let items: [ClipboardLibraryItem]
        let originalItems: [ClipboardLibraryItem]
        let originalRevisions: [UInt64: UInt64]
        let deletionIDs: Set<UInt64>
        var committedRevision: UInt64?
        var committedCountsRevision: UInt64?
    }
    private var edits: [Edit] = []
    private(set) var presentationRevision: UInt64 = 0
    private(set) var reloadRevision: UInt64 = 0
    var lists: [ClipboardLibraryList] {
        var result = storedLists
        for edit in edits where edit.committedRevision == nil || (edit.committedRevision ?? 0) > storedListsRevision {
            let request = edit.request
            switch request["op"] as? String {
            case "reorder":
                let ids = request["ids"] as? [String] ?? []
                let positions = Dictionary(uniqueKeysWithValues: ids.enumerated().map { ($0.element, $0.offset) })
                result.sort { (positions[$0.id] ?? $0.position) < (positions[$1.id] ?? $1.position) }
            case "delete_list": result.removeAll { $0.id == request["id"] as? String }
            case "save_list":
                if let raw = request["list"], let data = try? JSONSerialization.data(withJSONObject: raw),
                   let value = try? JSONDecoder().decode(ClipboardLibraryList.self, from: data) {
                    if let index = result.firstIndex(where: { $0.id == value.id }) { result[index] = value }
                    else { result.append(value) }
                }
            default: break
            }
        }
        return result
    }
    private var storedCounts: [String: Int] = [:]
    private var storedCountsRevision: UInt64 = 0
    private(set) var counts: [String: Int] = [:]
    private func rebuildCounts() {
        var result = storedCounts
        var memberships: [UInt64: Set<String>] = [:]
        var metadataRevisions: [UInt64: UInt64] = [:]
        var deleted = Set<UInt64>()
        for edit in edits {
            let needsDelta = (edit.committedCountsRevision ?? .max) > storedCountsRevision
            func identity(_ item: ClipboardLibraryItem) -> UInt64 { (item.related_ids ?? [item.id]).min() ?? item.id }
            for item in edit.originalItems {
                let key = identity(item), sourceRevision = edit.originalRevisions[key] ?? 0
                if memberships[key] == nil || sourceRevision > (metadataRevisions[key] ?? 0) {
                    memberships[key] = Set(item.memberships ?? [])
                    metadataRevisions[key] = sourceRevision
                }
            }
            func adjust(_ key: String, by delta: Int) {
                if needsDelta, let count = result[key] { result[key] = max(0, count + delta) }
            }
            switch edit.request["op"] as? String {
            case "membership":
                if let item = edit.originalItems.first, let list = edit.request["list_id"] as? String,
                   let present = edit.request["present"] as? Bool {
                    let key = identity(item)
                    if let committed = edit.committedRevision, committed <= (metadataRevisions[key] ?? 0) { continue }
                    if memberships[key, default: []].contains(list) != present {
                        adjust(list, by: present ? 1 : -1)
                    }
                    if present { memberships[key, default: []].insert(list) }
                    else { memberships[key, default: []].remove(list) }
                    if let committed = edit.committedRevision { metadataRevisions[key] = committed }
                }
            case "delete_items":
                for item in edit.originalItems where deleted.insert(identity(item)).inserted {
                    adjust("all", by: -1)
                    for list in memberships[identity(item)] ?? [] { adjust(list, by: -1) }
                }
            default: break
            }
        }
        if counts != result { counts = result }
    }
    private var countsRevision: UInt64 = 0
    private var countsKey: String?
    @ObservationIgnored private var countsTask: Task<Void, Never>?
    @ObservationIgnored private var mutationTail: Task<ClipboardLibrarySettings, Error>?
    @ObservationIgnored private var knownItems: [UInt64: ClipboardLibraryItem] = [:]
    @ObservationIgnored private var knownItemRevisions: [UInt64: UInt64] = [:]
    @ObservationIgnored private var refreshTask: Task<Void, Never>?
    @ObservationIgnored private var pages: [PageKey: ClipboardLibraryPage] = [:]
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

    func query(listID: String?, search: String, source: String?, offset: Int = 0, cursor: ClipboardLibraryCursor? = nil) async throws -> ClipboardLibraryPage {
        guard let client = AppStore.shared.client else { throw LibraryError(message: "引擎尚未就绪") }
        var request = itemRequest(listID: listID, search: search, source: source)
        request["offset"] = offset
        if let cursor { request["cursor"] = ["sort_at_ms": cursor.sort_at_ms, "id": cursor.id] }
        let data = try JSONSerialization.data(withJSONObject: request)
        let response = try await client.libraryQuery(String(decoding: data, as: UTF8.self))
        let page = try JSONDecoder().decode(ClipboardLibraryPage.self, from: Data(response.utf8))
        for item in page.items { remember(item, revision: page.revision ?? 0) }
        if offset == 0 {
            if pages.count >= 16 { pages.removeAll(keepingCapacity: true) }
            pages[pageKey(listID, search, source)] = page
        }
        return page
    }
    private struct PageKey: Hashable { let list: String?; let search: String; let source: String? }
    private func pageKey(_ listID: String?, _ search: String, _ source: String?) -> PageKey {
        PageKey(list: listID, search: search, source: source)
    }
    func cachedPage(listID: String?, search: String, source: String?) -> ClipboardLibraryPage? {
        pages[pageKey(listID, search, source)]
    }
    func project(_ items: [ClipboardLibraryItem], revision pageRevision: UInt64?, listID: String? = nil,
                 search: String = "", source: String? = nil) -> [ClipboardLibraryItem] {
        var result = items
        for edit in edits where (edit.committedRevision ?? .max) > (pageRevision ?? 0) {
            let request = edit.request
            switch request["op"] as? String {
            case "delete_items":
                result.removeAll { edit.deletionIDs.contains($0.id) }
            case "set_item_title":
                if let id = request["id"] as? UInt64, let index = result.firstIndex(where: { $0.id == id }) {
                    result[index].title = request["title"] as? String
                }
            case "membership":
                if let id = request["history_id"] as? UInt64, let list = request["list_id"] as? String,
                   let present = request["present"] as? Bool {
                    let aliases = Set(edit.items.first?.related_ids ?? [id])
                    let index = result.firstIndex { $0.id == id || aliases.contains($0.id) }
                    // Show a newly added item when its target list is opened,
                    // including before any first page or durable receipt exists.
                    if index == nil, present, list == listID, search.isEmpty, source == nil,
                       let item = edit.items.first {
                        result.insert(item, at: 0)
                    }
                    guard let index = result.firstIndex(where: { $0.id == id || aliases.contains($0.id) }) else { continue }
                    var members = result[index].memberships ?? []
                    members.removeAll { $0 == list }
                    if present { members.append(list) }
                    result[index].memberships = members
                    if !present && list == listID { result.remove(at: index) }
                }
            default: break
            }
        }
        return result
    }
    func remember(_ item: ClipboardLibraryItem, revision: UInt64 = 0) {
        guard revision >= (knownItemRevisions[item.id] ?? 0) else { return }
        if knownItems.count >= 512, knownItems[item.id] == nil {
            knownItems.removeAll(keepingCapacity: true)
            knownItemRevisions.removeAll(keepingCapacity: true)
        }
        knownItems[item.id] = item
        knownItemRevisions[item.id] = revision
    }
    func remember(_ item: HistoryItem, memberships: [String], title: String?) {
        guard knownItems[item.id] == nil else { return }
        remember(ClipboardLibraryItem(id: item.id, copied_at_ms: item.copiedAtMs,
            mime: item.mime, length: item.length, preview: item.preview, source: item.source,
            has_image: item.hasImage, file_count: item.fileCount, memberships: memberships,
            source_device_id: item.sourceDeviceId, source_app_id: item.sourceAppId, title: title))
    }
    func isLocallyDeleted(_ id: UInt64) -> Bool {
        edits.contains { $0.deletionIDs.contains(id) }
    }
    func queryTotal(listID: String?, search: String, source: String?) async throws -> Int {
        guard let client = AppStore.shared.client else { throw LibraryError(message: "引擎尚未就绪") }
        var request = itemRequest(listID: listID, search: search, source: source)
        request["count_only"] = true
        let data = try JSONSerialization.data(withJSONObject: request)
        let response = try await client.libraryCounts(String(decoding: data, as: UTF8.self))
        struct Count: Decodable { let total: Int }
        return try JSONDecoder().decode(Count.self, from: Data(response.utf8)).total
    }
    func libraryChanged(_ value: UInt64) {
        guard value > revision else { return }
        revision = value
        refreshTask?.cancel()
        refreshTask = Task {
            do { try await Task.sleep(for: .milliseconds(60)) } catch { return }
            await refresh()
        }
    }

    private func itemRequest(listID: String?, search: String = "", source: String? = nil) -> [String: Any] {
        let calendar = Calendar.current
        let today = calendar.startOfDay(for: Date())
        let yesterday = calendar.date(byAdding: .day, value: -1, to: today) ?? today
        let tomorrow = calendar.date(byAdding: .day, value: 1, to: today) ?? today
        var request: [String: Any] = ["op": "items", "list_id": listID as Any? ?? NSNull(), "query": search, "kind": "all",
            "source": source as Any? ?? NSNull(), "limit": ClipboardPaging.pageSize, "include_total": false,
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
        return request
    }

    private func refreshCounts() async throws {
        guard let client = AppStore.shared.client else { throw LibraryError(message: "引擎尚未就绪") }
        let key = "\(countsRevision)|\(Calendar.current.startOfDay(for: Date()))|\(TimeZone.current.identifier)"
        guard countsKey != key else { return }
        let expectedRevision = countsRevision
        let requests = [itemRequest(listID: nil)] + lists.map { itemRequest(listID: $0.id) }
        let data = try JSONSerialization.data(withJSONObject: ["op": "counts", "requests": requests])
        let response = try await client.libraryCounts(String(decoding: data, as: UTF8.self))
        let updated = try JSONDecoder().decode([String: Int].self, from: Data(response.utf8))
        guard countsRevision == expectedRevision else { return }
        applyCounts(updated, revision: expectedRevision)
        countsKey = key
    }
    func applyCounts(_ updated: [String: Int], revision: UInt64) {
        guard revision >= storedCountsRevision else { return }
        if storedCounts != updated { storedCounts = updated }
        storedCountsRevision = revision
        rebuildCounts()
    }
    func refresh() async {
        do { _ = try await loadSettings() }
        catch { self.error = error.userFacingMessage }
    }
    private func scheduleCounts() {
        countsTask?.cancel()
        countsTask = Task(priority: .utility) {
            do {
                try await Task.sleep(for: .milliseconds(250))
                try await refreshCounts()
            } catch is CancellationError { }
            catch { self.error = error.userFacingMessage }
        }
    }
    func loadSettings() async throws -> ClipboardLibrarySettings {
        guard let client = AppStore.shared.client else { throw LibraryError(message: "引擎尚未就绪") }
        let response = try await client.libraryQuery("{\"op\":\"state\"}")
        let settings = try apply(response)
        scheduleCounts()
        return settings
    }
    private func apply(_ response: String) throws -> ClipboardLibrarySettings {
        let state = try JSONDecoder().decode(ClipboardLibraryState.self, from: Data(response.utf8))
        // A poll already in flight must not roll back a completed mutation.
        guard state.revision >= revision else { return settings }
        // Only publish changed projections. Replacing observed menu data every
        // two seconds interrupts macOS submenu tracking even without mutations.
        if let value = state.lists {
            if storedLists != value { storedLists = value }
            storedListsRevision = state.revision
        }
        if let value = state.settings, settings != value { settings = value }
        if let value = state.devices, devices != value { devices = value }
        if let value = state.apps, apps != value { apps = value }
        if error != nil { error = nil }
        if let value = state.counts_revision { countsRevision = value }
        if revision != state.revision { revision = state.revision }
        let value = settings.shortcut ?? ClipboardLibraryShortcut()
        let key = value.enabled ? WorkflowKey(key_code: value.key_code, modifiers: value.modifiers) : nil
        if ClipboardPanelController.shared.shortcut != key { ClipboardPanelController.shared.configure(key) }
        return settings
    }
    @discardableResult
    func mutate(_ request: [String: Any]) async throws -> ClipboardLibrarySettings {
        try await enqueueMutation(request, operation: beginMutation(request)).value
    }
    private func enqueueMutation(_ request: [String: Any], operation: UUID) -> Task<ClipboardLibrarySettings, Error> {
        let previous = mutationTail
        let invocation = Task(priority: .userInitiated) {
            _ = try? await previous?.value
            do {
                guard let client = AppStore.shared.client else { throw LibraryError(message: "引擎尚未就绪") }
                var request = request
                request["compact"] = true
                let data = try JSONSerialization.data(withJSONObject: request)
                let encoded = String(decoding: data, as: UTF8.self)
                let response = try await client.libraryMutate(encoded)
                let settings = try apply(response)
                try acknowledgeMutation(operation, response: response)
                scheduleCounts()
                return settings
            } catch {
                rollbackMutation(operation)
                self.error = error.userFacingMessage
                throw error
            }
        }
        mutationTail = invocation
        return invocation
    }
    @discardableResult
    func beginMutation(_ request: [String: Any]) -> UUID {
        let operation = UUID()
        var request = request
        let ids = (request["ids"] as? [UInt64]) ?? (request["history_id"] as? UInt64).map { [$0] } ?? []
        var originalSeen = Set<UInt64>()
        let originalItems = ids.compactMap { knownItems[$0] }.filter {
            originalSeen.insert(($0.related_ids ?? [$0.id]).min() ?? $0.id).inserted
        }
        let originalRevisions = Dictionary(uniqueKeysWithValues: originalItems.map {
            (($0.related_ids ?? [$0.id]).min() ?? $0.id, knownItemRevisions[$0.id] ?? 0)
        })
        let projected = ids.flatMap { id -> [ClipboardLibraryItem] in
            guard let item = knownItems[id] else { return [] }
            return project([item], revision: knownItemRevisions[id])
        }
        var seen = Set<UInt64>()
        let items = projected.filter { seen.insert(($0.related_ids ?? [$0.id]).min() ?? $0.id).inserted }
        if request["op"] as? String == "delete_items" {
            request["ids"] = Array(Set(ids + items.flatMap { $0.related_ids ?? [$0.id] }))
        }
        let deletionIDs = request["op"] as? String == "delete_items" ? Set(request["ids"] as? [UInt64] ?? []) : []
        edits.append(Edit(id: operation, request: request, items: items, originalItems: originalItems, originalRevisions: originalRevisions, deletionIDs: deletionIDs))
        rebuildCounts()
        presentationRevision &+= 1
        return operation
    }
    func acknowledgeMutation(_ operation: UUID, response: String) throws {
        let state = try JSONDecoder().decode(ClipboardLibraryState.self, from: Data(response.utf8))
        if let index = edits.firstIndex(where: { $0.id == operation }) {
            edits[index].committedRevision = state.revision
            edits[index].committedCountsRevision = state.counts_revision ?? state.revision
        }
        // The receipt can be older than a simultaneous background event. Keep
        // its overlay until a page/state at that revision actually arrives.
        presentationRevision &+= 1
        compactEdits()
        rebuildCounts()
    }
    func rollbackMutation(_ operation: UUID) {
        edits.removeAll { $0.id == operation }
        rebuildCounts()
        presentationRevision &+= 1
    }
    private func compactEdits() {
        guard edits.count > 512 else { return }
        pages.removeAll(keepingCapacity: true)
        // Keep recent receipts plus deletions still present in the bounded hot
        // history. Older receipts trigger a fresh authoritative page.
        let hotIDs = Set(AppStore.shared.snapshot?.history.map(\.id) ?? [])
        let recent = Set(edits.suffix(256).map(\.id))
        edits.removeAll { edit in
            guard edit.committedRevision != nil, !recent.contains(edit.id) else { return false }
            return !(edit.request["op"] as? String == "delete_items" &&
                (edit.request["ids"] as? [UInt64] ?? []).contains(where: hotIDs.contains))
        }
        reloadRevision &+= 1
    }
    func save(_ list: ClipboardLibraryList) async throws {
        let object = try JSONSerialization.jsonObject(with: JSONEncoder().encode(list))
        try await mutate(["op": "save_list", "list": object])
    }
    func perform(_ request: [String: Any]) {
        // Publish in the action callback, before creating or waiting for a task.
        _ = enqueueMutation(request, operation: beginMutation(request))
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
