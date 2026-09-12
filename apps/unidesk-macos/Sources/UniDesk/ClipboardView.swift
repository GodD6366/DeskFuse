import SwiftUI
import AppKit
import UniDeskEngine

struct ClipboardView: View {
    @Bindable var store: AppStore
    let snapshot: EngineSnapshot
    let searchRequest: Int
    @State private var selected: UInt64?
    @State private var navigationSelected: UInt64?
    @State private var search = ""
    @State private var kind: ClipboardKind = .all
    @State private var source: String?
    @State private var content: ClipboardContent?
    @State private var media: ClipboardMedia?
    @State private var previewImage: NSImage?
    @State private var loading = false
    @State private var previewError: String?
    @State private var searchMatches: Set<UInt64> = []
    @State private var matchedQuery = ""
    @State private var searching = false

    private var items: [HistoryItem] {
        snapshot.history.filter { item in
            (source == nil || item.source == source) && kind.includes(item) &&
            (search.isEmpty || (matchedQuery == search && searchMatches.contains(item.id)))
        }
    }

    var body: some View {
        HistorySplitView {
            ClipboardHistoryPane(
                items: items, sources: Array(Set(snapshot.history.map(\.source))).sorted(),
                selected: $selected, search: $search, kind: $kind, source: $source,
                searchRequest: searchRequest, searching: searching,
                historyIsEmpty: snapshot.history.isEmpty,
                syncEnabled: Binding(get: { snapshot.clipboardEnabled }, set: { store.send(.setClipboard(enabled: $0)) }),
                selectedImage: content?.id == selected ? previewImage : nil
            )
        } detail: {
            VStack(spacing: 0) {
                if let error = snapshot.clipboardError {
                    Label(error, systemImage: "exclamationmark.triangle")
                        .font(.callout).foregroundStyle(.orange)
                        .frame(maxWidth: .infinity, alignment: .leading).padding(16)
                }
                detail
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
            .frame(minWidth: 340, maxWidth: .infinity, maxHeight: .infinity)
            .background(Color(nsColor: .textBackgroundColor))
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .task(id: HistorySearchKey(query: search, history: snapshot.history.map(\.id))) {
            guard !search.isEmpty else {
                matchedQuery = ""; searchMatches = []; searching = false
                return
            }
            searching = true
            do {
                try await Task.sleep(for: .milliseconds(150))
                guard let client = store.client else { searching = false; return }
                let ids = try await client.search(search, filter: 0)
                guard !Task.isCancelled else { return }
                matchedQuery = search; searchMatches = Set(ids); searching = false
            } catch {
                guard !Task.isCancelled else { return }
                searching = false; store.error = error.userFacingMessage
            }
        }
        .task(id: selected) {
            content = nil; media = nil; previewImage = nil; previewError = nil; loading = false
            guard let id = selected, let client = store.client else { return }
            loading = true
            defer { if selected == id { loading = false } }
            do {
                let loaded = try await client.content(id)
                guard !Task.isCancelled, selected == id else { return }
                let loadedMedia = await ClipboardMediaLoader.shared.load(loaded)
                guard !Task.isCancelled, selected == id else { return }
                content = loaded; media = loadedMedia
                previewImage = loadedMedia.thumbnail.flatMap { NSImage(data: $0) }
            } catch {
                if !Task.isCancelled, selected == id { previewError = error.userFacingMessage }
            }
        }
        .onChange(of: store.clipboardNavigation, initial: true) { _, request in
            guard let request else { return }
            search = request.query; kind = request.kind; source = request.source
            selected = request.selectedID
            navigationSelected = !request.query.isEmpty && matchedQuery != request.query ? request.selectedID : nil
            store.clipboardNavigation = nil
        }
        .onChange(of: ClipboardSelectionKey(ids: items.map(\.id), ready: search.isEmpty || matchedQuery == search), initial: true) { _, state in
            let ids = state.ids
            // A pending full-text query must not erase the menu's target selection.
            guard search.isEmpty || matchedQuery == search else { return }
            if let target = navigationSelected {
                navigationSelected = nil
                if ids.contains(target) { selected = target; return }
            }
            // Selecting a row only reads its preview. It never restores the pasteboard.
            if let selected, ids.contains(selected) { return }
            selected = ids.first
        }
        .onChange(of: snapshot.history.map(\.source)) { _, sources in
            if let source, !sources.contains(source) { self.source = nil }
        }
    }

    @ViewBuilder private var detail: some View {
        if loading {
            ProgressView("正在读取预览…")
        } else if let previewError {
            ContentUnavailableView("无法读取预览", systemImage: "exclamationmark.triangle", description: Text(previewError))
        } else if let content, content.id == selected, let media, let item = snapshot.history.first(where: { $0.id == content.id }) {
            ClipboardPreviewPane(store: store, item: item, content: content, media: media, image: previewImage)
                .id(content.id)
        } else {
            ContentUnavailableView {
                Label(snapshot.history.isEmpty ? "复制内容，从这里接着用" : "选择一条历史", systemImage: "clipboard")
            } description: {
                Text("文字、图片与文件会在可信设备之间同步。")
            }
        }
    }
}

private struct HistorySearchKey: Equatable {
    let query: String
    let history: [UInt64]
}

private struct ClipboardSelectionKey: Equatable { let ids: [UInt64]; let ready: Bool }
