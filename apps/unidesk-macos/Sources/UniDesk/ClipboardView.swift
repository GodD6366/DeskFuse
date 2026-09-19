import SwiftUI
import AppKit
import UniDeskEngine

private struct ClipboardPagePresentation {
    var items: [HistoryItem] = []
    var appIDs: [UInt64: String] = [:]
    var titles: [UInt64: String] = [:]
    var metas: [UInt64: ClipboardItemMeta] = [:]
    var memberships: [UInt64: [String]] = [:]
}

struct ClipboardView: View {
    @Bindable var store: AppStore
    let snapshot: EngineSnapshot
    let searchRequest: Int
    @State private var selected: UInt64?
    @State private var navigationSelected: UInt64?
    @State private var search = ""
    @State private var source: String?
    @State private var content: ClipboardContent?
    @State private var media: ClipboardMedia?
    @State private var previewImage: NSImage?
    @State private var loading = false
    @State private var previewError: String?
    @Bindable private var library = ClipboardLibraryStore.shared
    @State private var listID: String?
    @AppStorage("unidesk.clipboard.displayMode") private var displayMode: ClipboardDisplayMode = .list
    @State private var previewing = false
    @State private var pageItems: [ClipboardLibraryItem] = []
    @State private var presentation = ClipboardPagePresentation()
    @State private var nextOffset: Int?
    @State private var nextCursor: ClipboardLibraryCursor?
    @State private var total = 0
    @State private var searching = false
    @State private var pageRequest = UUID()
    @State private var paginationGeneration = UUID()
    @State private var queryRevision = 0
    @State private var copyToken: String?
    @State private var copyError: String?
    @State private var copiedID: UInt64?

    private var items: [HistoryItem] { presentation.items }
    private var queryKey: String { "\(listID ?? "")|\(search)|\(source ?? "")|\(library.revision)|\(snapshot.history.map(\.id))|\(queryRevision)" }

    var body: some View {
        HStack(spacing: 0) {
            ClipboardWorkspaceSidebar(snapshot: snapshot, selectedListID: $listID, total: total)
                .frame(minWidth: 180, idealWidth: 190, maxWidth: 220)
            Divider()
            HistorySplitView(expandHistory: displayMode == .grid) {
                ClipboardHistoryPane(
                    items: items, sources: library.sourceChoices(legacyNames: snapshot.history.map(\.source), snapshot: snapshot),
                    selected: $selected, search: $search, source: $source,
                    displayMode: $displayMode,
                    searchRequest: searchRequest, searching: searching,
                    historyIsEmpty: total == 0 && search.isEmpty && listID == nil,
                    syncEnabled: Binding(get: { snapshot.clipboardEnabled }, set: { store.send(.setClipboard(enabled: $0)) }),
                    selectedImage: content?.id == selected ? previewImage : nil,
                    client: store.client,
                    appIDs: presentation.appIDs,
                    titles: presentation.titles,
                    metas: presentation.metas,
                    onRename: { id, title in library.rename(id, title: title) },
                    previewing: previewing,
                    onTogglePreview: { previewing.toggle() },
                    onDelete: { id in
                        library.delete([id])
                        cancelCopy(); copyError = nil; copiedID = nil; previewing = false
                    },
                    listID: $listID, total: total, memberships: presentation.memberships,
                    hasMore: nextOffset != nil, paginationOffset: nextOffset ?? 0, paginationGeneration: paginationGeneration, loadMore: { Task { await loadPage(append: true) } }
                )
            } detail: {
                VStack(spacing: 0) {
                    if let error = copyError ?? library.error ?? snapshot.clipboardError {
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
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .overlay(alignment: .top) { Divider() }
        .task {
            while !Task.isCancelled {
                await library.refresh()
                do { try await Task.sleep(for: .seconds(2)) } catch { return }
            }
        }
        .task(id: queryKey) {
            rebuildPresentation()
            await loadPage(append: false)
        }
        .onChange(of: snapshot.history.map(\.id)) { _, _ in rebuildPresentation() }
        .onReceive(NotificationCenter.default.publisher(for: .NSCalendarDayChanged)) { _ in queryRevision += 1 }
        .onReceive(NotificationCenter.default.publisher(for: .NSSystemTimeZoneDidChange)) { _ in queryRevision += 1 }
        .onChange(of: selected) { _, _ in cancelCopy(); copyError = nil; copiedID = nil; previewing = false }
        // The preview lives in its own screen-sized window, so it is driven here
        // instead of being an overlay capped by this window.
        .onChange(of: previewing) { _, active in
            if active {
                ClipboardQuickLookController.shared.present(over: NSApp.keyWindow ?? NSApp.mainWindow) { previewing = false }
                updateQuickLook()
            } else {
                ClipboardQuickLookController.shared.close()
            }
        }
        .onChange(of: previewImage.map(ObjectIdentifier.init)) { _, _ in updateQuickLook() }
        .onChange(of: content?.id) { _, _ in updateQuickLook() }
        .task(id: copiedID) {
            guard let id = copiedID else { return }
            do { try await Task.sleep(for: .seconds(2)) } catch { return }
            if copiedID == id { copiedID = nil }
        }
        .onDisappear {
            cancelCopy(); ClipboardQuickLookController.shared.close()
            pageRequest = UUID(); searching = false
            content = nil; media = nil; previewImage = nil
            pageItems = []; nextOffset = nil; nextCursor = nil; presentation = ClipboardPagePresentation()
        }
        .onReceive(NotificationCenter.default.publisher(for: NSWindow.willCloseNotification)) { notification in
            if (notification.object as? NSWindow)?.identifier?.rawValue == "unidesk-main" {
                cancelCopy()
                ClipboardQuickLookController.shared.close()
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
                guard !Task.isCancelled, selected == id else { return }
                previewError = error.userFacingMessage
                // A remote index row has no local payload yet. Fetch it on demand
                // and retry briefly while it arrives over the LAN.
                guard pageItems.first(where: { $0.id == id })?.available == false else { return }
                library.wantPayload(id)
                for _ in 0..<20 {
                    try? await Task.sleep(for: .milliseconds(500))
                    guard !Task.isCancelled, selected == id else { return }
                    guard let retried = try? await client.content(id) else { continue }
                    let retriedMedia = await ClipboardMediaLoader.shared.load(retried)
                    guard !Task.isCancelled, selected == id else { return }
                    content = retried; media = retriedMedia
                    previewImage = retriedMedia.thumbnail.flatMap { NSImage(data: $0) }
                    previewError = nil
                    return
                }
            }
        }
        .onChange(of: store.clipboardNavigation, initial: true) { _, request in
            guard let request else { return }
            search = request.query; source = request.source; listID = request.listID
            selected = request.selectedID
            navigationSelected = request.selectedID
            store.clipboardNavigation = nil
        }
        .onChange(of: ClipboardSelectionKey(ids: items.map(\.id), ready: !searching), initial: true) { _, state in
            let ids = state.ids
            // A pending full-text query must not erase the menu's target selection.
            guard state.ready else { return }
            if let target = navigationSelected {
                navigationSelected = nil
                if ids.contains(target) { selected = target; return }
            }
            // Selecting a row only reads its preview. It never restores the pasteboard.
            if let selected, ids.contains(selected) { return }
            selected = ids.first
        }

    }

    private func updateQuickLook() {
        guard previewing else { return }
        ClipboardQuickLookController.shared.update(content: content, image: previewImage)
    }
    private func cancelCopy() {
        guard let token = copyToken else { return }
        copyToken = nil
        if let client = store.client { Task { try? await client.cancelClipboardPreparation(token) } }
    }
    private func prepareCopy(_ id: UInt64) {
        guard copyToken == nil, selected == id, let client = store.client else { return }
        let token = UUID().uuidString
        copyToken = token; copyError = nil; copiedID = nil
        Task { @MainActor in
            do {
                _ = try await client.prepareClipboardPaste(id, session: token)
                guard copyToken == token, selected == id else { return }
                copyToken = nil
                copiedID = id
                // A copy may have fetched remote file references. Refresh the
                // preview's local availability without issuing another paste.
                if let refreshed = try? await client.content(id) {
                    let refreshedMedia = await ClipboardMediaLoader.shared.load(refreshed)
                    guard selected == id, copyToken == nil else { return }
                    content = refreshed; media = refreshedMedia
                    previewImage = refreshedMedia.thumbnail.flatMap { NSImage(data: $0) }
                }
            } catch {
                guard copyToken == token, selected == id else { return }
                copyToken = nil; copyError = error.userFacingMessage
            }
        }
    }

    /// Rebuild the page projection only when the query/page/snapshot changes.
    /// Selection, focus, preview and copy state now reuse these arrays instead
    /// of walking every loaded history row on each SwiftUI body evaluation.
    @MainActor private func rebuildPresentation() {
        var seen = Set<UInt64>()
        var page = pageItems.compactMap { item -> HistoryItem? in
            let display = library.displayItem(item, snapshot: snapshot)
            return seen.insert(display.id).inserted ? display : nil
        }
        var order: [UInt64: UInt64] = [:]
        order.reserveCapacity(pageItems.count + snapshot.history.count)
        for item in pageItems {
            order[item.id] = item.sort_at_ms ?? item.copied_at_ms ?? 0
        }

        // The in-memory engine snapshot keeps the latest local records visible
        // while the first library page is arriving. It is merged only for the
        // unfiltered history view; smart lists and searches remain query-driven.
        if search.isEmpty, source == nil, listID == nil, !snapshot.history.isEmpty {
            let fresh = snapshot.history.filter { seen.insert($0.id).inserted }
            if !fresh.isEmpty {
                for item in fresh { order[item.id] = item.copiedAtMs ?? 0 }
                page.append(contentsOf: fresh)
                page.sort {
                    let left = order[$0.id] ?? 0
                    let right = order[$1.id] ?? 0
                    return left == right ? $0.id > $1.id : left > right
                }
            }
        }

        let unavailable = Set(pageItems.lazy.filter { $0.remote == true && $0.available == false }.map(\.id))
        let visible = page.filter { item in
            item.fileCount == 0 || !unavailable.contains(item.id)
        }
        let deviceNames = Dictionary(library.devices.map { ($0.id, $0.name) }, uniquingKeysWith: { _, latest in latest })
        var appIDs: [UInt64: String] = [:]
        var titles: [UInt64: String] = [:]
        var metas: [UInt64: ClipboardItemMeta] = [:]
        var memberships: [UInt64: [String]] = [:]
        appIDs.reserveCapacity(visible.count)
        titles.reserveCapacity(pageItems.count)
        metas.reserveCapacity(pageItems.count)
        memberships.reserveCapacity(pageItems.count)
        for item in visible {
            if let appID = item.sourceAppId { appIDs[item.id] = appID }
        }
        for item in pageItems {
            if let title = item.title, !title.isEmpty { titles[item.id] = title }
            let originDevice = item.source_device_id.flatMap { deviceNames[$0] }
            metas[item.id] = ClipboardItemMeta(
                remote: item.remote ?? false,
                available: item.available ?? true,
                originDevice: originDevice,
                ocrText: item.ocr_text
            )
            memberships[item.id] = item.memberships ?? []
        }
        presentation = ClipboardPagePresentation(items: visible, appIDs: appIDs, titles: titles, metas: metas, memberships: memberships)
    }

    @MainActor private func loadPage(append: Bool) async {
        let key = queryKey
        if append && (searching || nextOffset == nil) { return }
        let request = UUID(); pageRequest = request
        if !append { paginationGeneration = UUID() }
        searching = true
        defer { if pageRequest == request { searching = false } }
        do {
            // Debounce typing, not window presentation or history refresh.
            if !append && !search.isEmpty { try await Task.sleep(for: .milliseconds(120)) }
            let result = try await library.query(
                listID: listID,
                search: search,
                source: source,
                offset: append ? (nextOffset ?? 0) : 0,
                cursor: append ? nextCursor : nil
            )
            guard !Task.isCancelled, pageRequest == request, key == queryKey else { return }
            if append {
                let existing = Set(pageItems.map(\.id))
                pageItems += result.items.filter { !existing.contains($0.id) }
                if pageItems.count > ClipboardPaging.maxLoadedItems {
                    pageItems.removeFirst(pageItems.count - ClipboardPaging.maxLoadedItems)
                }
            } else { pageItems = result.items }
            total = result.total; nextOffset = result.next_offset; nextCursor = result.next_cursor
            rebuildPresentation()
            scheduleOCR(result.items)
            if let target = navigationSelected, pageItems.contains(where: { $0.id == target }) { selected = target; navigationSelected = nil }
        } catch { if !Task.isCancelled, pageRequest == request, key == queryKey { store.error = error.userFacingMessage } }
    }

    private func scheduleOCR(_ items: [ClipboardLibraryItem]) {
        for item in items where item.has_image && (item.ocr_version ?? 0) < ClipboardOCRIndexer.version {
            ClipboardOCRIndexer.shared.schedule(item.id, client: store.client)
        }
    }

    @ViewBuilder private var detail: some View {
        if loading {
            ProgressView("正在读取预览…")
        } else if let previewError {
            ContentUnavailableView("无法读取预览", systemImage: "exclamationmark.triangle", description: Text(previewError))
        } else if let content, content.id == selected, let media, let item = items.first(where: { $0.id == content.id }) {
            ClipboardPreviewPane(store: store, item: item, content: content, media: media, image: previewImage,
                                 ocrText: pageItems.first(where: { $0.id == content.id })?.ocr_text,
                                 copying: copyToken != nil, copied: copiedID == item.id, copyContent: { prepareCopy(content.id) })
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

/// Space-bar preview: a dimmed, non-destructive enlargement of the selected
/// record. It never writes the pasteboard or changes the selection. Shared by
/// the main window and the menu bar panel, and drawn in its own screen-sized
/// window so it is not capped by the surface that opened it; `content` is
/// optional so the backdrop can appear while the record is still being read.
struct ClipboardQuickLook: View {
    let content: ClipboardContent?
    let image: NSImage?
    let dismiss: () -> Void

    var body: some View {
        ZStack {
            Color.black.opacity(0.72).ignoresSafeArea()
            if let image {
                Image(nsImage: image).resizable().scaledToFit()
                    .padding(44)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if let text = content?.text, !text.isEmpty {
                ScrollView {
                    Text(text)
                        .font(.system(size: 15))
                        .lineSpacing(5)
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(34)
                }
                .frame(maxWidth: 900, maxHeight: .infinity)
                .background(Color(nsColor: .textBackgroundColor), in: RoundedRectangle(cornerRadius: 16))
                .padding(44)
            } else if let paths = content?.filePaths, !paths.isEmpty {
                VStack(spacing: 8) {
                    Image(systemName: "doc.on.doc").font(.system(size: 48)).foregroundStyle(.secondary)
                    Text("\(paths.count) 个文件").font(.title3.weight(.semibold))
                }
                .padding(36)
                .background(Color(nsColor: .textBackgroundColor), in: RoundedRectangle(cornerRadius: 16))
            } else {
                ProgressView().controlSize(.large).tint(.white)
            }
        }
        .contentShape(Rectangle())
        .onTapGesture(perform: dismiss)
        .accessibilityAddTraits(.isModal)
    }
}

private struct ClipboardSelectionKey: Equatable { let ids: [UInt64]; let ready: Bool }
