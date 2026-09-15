import SwiftUI
import AppKit
import UniDeskEngine

/// Selection and searches are temporary UI state. Only an explicit restore
/// command changes the pasteboard, and only its applied event dismisses the panel.
struct MenuBarClipboardView: View {
    @Bindable var store: AppStore
    @Environment(\.openWindow) private var openWindow
    @FocusState private var searchFocused: Bool
    @State private var query = ""
    @State private var source: String?
    @State private var selectedID: UInt64?
    @Bindable private var library = ClipboardLibraryStore.shared
    @State private var listID: String?
    @AppStorage("unidesk.clipboard.displayMode") private var displayMode: ClipboardDisplayMode = .list
    @State private var pageItems: [ClipboardLibraryItem] = []
    @State private var commandHeld = false
    @State private var nextOffset: Int?
    @State private var calendarRevision = 0
    @State private var searching = false
    @State private var searchError: String?
    @State private var copyError: String?
    @State private var copying = false
    @State private var preparationToken: String?
    @State private var session = UUID()
    @State private var panelActive = false
    @State private var fileAvailability: [UInt64: Bool] = [:]
    @State private var panel = MenuBarPanelHandle()
    @State private var previewing = false
    @State private var previewContent: ClipboardContent?
    @State private var previewImage: NSImage?

    private var history: [HistoryItem] { store.snapshot?.history ?? [] }
    private var items: [HistoryItem] {
        let page = pageItems.map { library.displayItem($0, snapshot: store.snapshot) }
        // `pageItems` is only refreshed while the panel is open, so a record copied
        // while it was closed would wait for the library query to appear. The engine
        // snapshot history is already in memory and current; surface those records at
        // the top immediately. Active searches and list filters keep the query as the
        // source of truth because their ordering and membership differ.
        guard query.isEmpty, source == nil, listID == nil, !history.isEmpty else { return page }
        let known = Set(page.map(\.id))
        let fresh = history.filter { !known.contains($0.id) }
        guard !fresh.isEmpty else { return page }
        return fresh + page
    }
    private var titles: [UInt64: String] {
        Dictionary(uniqueKeysWithValues: pageItems.compactMap { item in
            (item.title?.isEmpty == false) ? (item.id, item.title!) : nil
        })
    }
    private var selectedItem: HistoryItem? { items.first { $0.id == selectedID } }
    private var canCopy: Bool {
        selectedItem != nil && store.client != nil && !copying &&
        !searching && searchError == nil &&
        selectedID.map { id in pageItems.first(where: { $0.id == id })?.available != false } == true &&
        selectedID.map { fileAvailability[$0] != false } == true
    }
    private var searchKey: String {
        "\(listID ?? "")|\(query)|\(source ?? "")|\(library.revision)|\(panelActive)|\(history.map(\.id))|\(calendarRevision)"
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            historyList
            Divider()
            footer
        }
        .frame(width: 440, height: ClipboardPanelController.shared.panelHeight)
        .background(Color(nsColor: .textBackgroundColor))
        .clipShape(RoundedRectangle(cornerRadius: 14))
        .background(MenuBarPanelBridge(handle: panel, opened: beginSession, closed: endSession, modifiersChanged: { commandHeld = $0.contains(.command) }, keyDown: handleKey))
        .task(id: searchKey) { if panelActive { await searchHistory() } }
        .task(id: panelActive) {
            guard panelActive else { return }
            while !Task.isCancelled {
                await library.refresh()
                do { try await Task.sleep(for: .seconds(2)) } catch { return }
            }
        }
        .onChange(of: items.map(\.id), initial: true) { _, ids in
            guard !searching else { return }
            if selectedID.map(ids.contains) != true { selectedID = ids.first }
            fileAvailability = fileAvailability.filter { ids.contains($0.key) }
        }
        // The preview belongs to one record: navigating away closes it.
        .onChange(of: selectedID) { _, _ in dismissPreview() }
        .onReceive(NotificationCenter.default.publisher(for: .NSCalendarDayChanged)) { _ in calendarRevision += 1 }
        .onReceive(NotificationCenter.default.publisher(for: .NSSystemTimeZoneDidChange)) { _ in calendarRevision += 1 }
        .onDisappear { endSession() }
    }

    private var header: some View {
        NativeGlassGroup {
            VStack(spacing: 10) {
                HStack(spacing: 10) {
                    Image(systemName: "rectangle.on.rectangle").font(.system(size: 19))
                    Text("最近剪贴板").font(.system(size: 16, weight: .semibold))
                    Spacer(minLength: 4)
                    ClipboardDisplayModePicker(mode: $displayMode)
                    sourceMenu
                    Button { closePanel(); store.shelf?.show() } label: {
                        Image(systemName: "tray").frame(width: 22, height: 22)
                    }
                    .buttonStyle(ClipboardHeaderButtonStyle())
                    .help("打开文件暂存区")
                    .accessibilityLabel("文件暂存区")
                    moreMenu
                }
                HStack(spacing: 8) {
                    Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                    TextField("搜索最近内容", text: $query)
                        .textFieldStyle(.plain).focused($searchFocused)
                    if query.isEmpty {
                        Text("⌘ F").font(.caption).foregroundStyle(.tertiary)
                            .opacity(commandHeld ? 1 : 0).accessibilityHidden(!commandHeld)
                    } else {
                        Button { query = "" } label: { Image(systemName: "xmark.circle.fill") }
                            .buttonStyle(.plain).foregroundStyle(.secondary).accessibilityLabel("清除搜索")
                    }
                }
                .font(.system(size: 13)).padding(.horizontal, 11).frame(height: 34)
                .background(Color(nsColor: .textBackgroundColor), in: RoundedRectangle(cornerRadius: 9))
                .overlay(RoundedRectangle(cornerRadius: 9).strokeBorder(searchFocused ? Color.accentColor : Color(nsColor: .separatorColor), lineWidth: 1))
                ClipboardLibraryTabs(selectedListID: $listID)


            }
            .padding(16)
            .modifier(NativeGlassBackdrop(cornerRadius: 0))
        }
    }

    private var sourceMenu: some View {
        Menu {
            Picker("来源设备", selection: $source) {
                Text("全部设备").tag(String?.none)
                ForEach(library.sourceChoices(legacyNames: history.map(\.source), snapshot: store.snapshot)) { value in
                    Text(value.name).tag(Optional(value.id))
                }
            }
        } label: {
            Text(library.devices.first(where: { $0.id == source })?.name ?? source ?? "全部设备")
                .font(.system(size: 11)).lineLimit(1).truncationMode(.tail)
                .frame(maxWidth: 100, alignment: .trailing)
        }
        .menuStyle(.borderlessButton)
        .accessibilityLabel("来源设备")
        .help("筛选来源设备")
    }

    private var moreMenu: some View {
        Menu {
            Text(store.snapshot?.runtimeStatus ?? (store.starting ? "正在启动" : "引擎不可用"))
            Button("打开 uniDesk") { closePanel(); openMainWindow() }
            Button("文件暂存区") { closePanel(); store.shelf?.show() }
            Button("立即收回键鼠") { store.reclaim() }
            Divider()
            Button("仅复制选中内容") { copySelected(paste: false) }.disabled(!canCopy)
            Divider()
            Button("设置…") { closePanel(); store.openSettings() }
            Button("退出 uniDesk") { NSApp.terminate(nil) }
        } label: { Image(systemName: "ellipsis.circle").font(.system(size: 18)).frame(width: 24, height: 24) }
        .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize()
        .accessibilityLabel("更多操作")
    }

    private var historyList: some View {
        // Resolve per-card metadata once per body evaluation. Scanning
        // `pageItems`/`items` inside the card builder made every selection change
        // O(cards x items) on the main thread, which is what made clicking a card
        // feel sticky once the page held enough rows.
        let rows = Dictionary(uniqueKeysWithValues: pageItems.map { ($0.id, $0) })
        let positions = Dictionary(uniqueKeysWithValues: items.enumerated().map { ($0.element.id, $0.offset) })
        let needle = query.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return ScrollViewReader { proxy in
            Group {
                if displayMode == .grid {
                    if items.isEmpty {
                        ScrollView { emptyState }
                            .frame(maxWidth: .infinity, maxHeight: .infinity)
                    } else {
                        ClipboardMasonryGrid(items: items, selected: selectedID, columnCount: 2,
                                             hasMore: nextOffset != nil, searching: searching,
                                             loadMore: { Task { await searchHistory(append: true) } }) { item, isSelected in
                            MenuBarClipboardCard(
                                item: item,
                                selected: isSelected,
                                client: store.client,
                                appID: rows[item.id]?.source_app_id,
                                memberships: rows[item.id]?.memberships ?? [],
                                title: titles[item.id],
                                index: positions[item.id] ?? 99,
                                showShortcut: commandHeld,
                                remoteUnavailable: isRemoteUnavailable(rows[item.id]),
                                ocrHit: isOCRHit(rows[item.id], needle: needle),
                                presentation: session,
                                active: panelActive,
                                availability: { fileAvailability[item.id] = $0 },
                                onSelect: { selectedID = item.id; searchFocused = false; copyError = nil; requestPayloadIfNeeded(item.id) },
                                onPaste: { copySelected(paste: true) },
                                onCopy: { copySelected(paste: false) },
                                onRename: { library.rename(item.id, title: $0) },
                                onDelete: { deleteItem(item.id) }
                            )
                        }
                    }
                } else {
                    ScrollView {
                        LazyVStack(spacing: 3) {
                            if items.isEmpty {
                                emptyState
                            } else {
                                ForEach(Array(items.enumerated()), id: \.element.id) { index, item in
                                    MenuBarClipboardRow(item: item, index: index, showShortcuts: commandHeld, selected: selectedID == item.id,
                                        client: store.client, appID: rows[item.id]?.source_app_id,
                                        title: titles[item.id],
                                        memberships: rows[item.id]?.memberships ?? [],
                                        remoteUnavailable: isRemoteUnavailable(rows[item.id]),
                                        ocrHit: isOCRHit(rows[item.id], needle: needle),
                                        presentation: session, active: panelActive, availability: { available in
                                            fileAvailability[item.id] = available
                                        }, select: { selectedID = item.id; searchFocused = false; copyError = nil; requestPayloadIfNeeded(item.id) },
                                        onCopy: { selectedID = item.id; copySelected(paste: false) },
                                        onRename: { library.rename(item.id, title: $0) },
                                        onDelete: { deleteItem(item.id) })
                                    .simultaneousGesture(TapGesture(count: 2).onEnded { selectedID = item.id; copySelected(paste: true) })
                                    .id(item.id)
                                }
                            }
                        }
                        .padding(.horizontal, 12).padding(.vertical, 10)
                        if nextOffset != nil { Button("加载更多") { Task { await searchHistory(append: true) } }.disabled(searching).padding(.bottom, 10) }
                    }
                    .scrollIndicators(.automatic)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .onChange(of: selectedID) { _, id in
                if let id { proxy.scrollTo(id) }
            }
        }
    }

    @ViewBuilder private var emptyState: some View {
        if store.starting || searching {
            ProgressView(store.starting ? "正在启动…" : "正在搜索…")
                .frame(maxWidth: .infinity).padding(.vertical, 65)
        } else {
            VStack(spacing: 10) {
                Image(systemName: history.isEmpty ? "clipboard" : "magnifyingglass")
                    .font(.system(size: 28)).foregroundStyle(.tertiary)
                Text(store.client == nil ? "引擎尚未就绪" : searchError != nil ? "搜索失败" : history.isEmpty ? "还没有剪贴板记录" : "没有匹配内容")
                    .font(.headline)
                Text(store.client == nil ? (store.error ?? "请稍后重试") : searchError ?? (history.isEmpty ? "复制文字、图片或文件后，会显示在这里" : "试试其他关键词，或清除筛选"))
                    .font(.callout).foregroundStyle(.secondary).multilineTextAlignment(.center)
                if !history.isEmpty && (!query.isEmpty || source != nil || listID != nil) {
                    Button("清除筛选") { query = ""; source = nil; listID = nil; searchError = nil }
                }
            }
            .frame(maxWidth: .infinity).padding(.horizontal, 15).padding(.vertical, 45)
        }
    }

    private var footer: some View {
        NativeGlassGroup {
            VStack(spacing: 10) {
                if let message = copyError ?? library.error ?? ClipboardPanelController.shared.error {
                    Label(message, systemImage: "exclamationmark.triangle")
                        .font(.caption).foregroundStyle(.orange)
                        .lineLimit(3).frame(maxWidth: .infinity, alignment: .leading)
                        .accessibilityLabel("剪贴板提示：\(message)")
                }
                HStack(spacing: 8) {
                    Button { showAll() } label: { Label("查看全部", systemImage: "arrow.up.right") }
                        .buttonStyle(.borderless).fixedSize()
                    Spacer(minLength: 0)
                    if copying { ProgressView().controlSize(.mini).help("正在准备剪贴板内容") }
                    if let item = selectedItem {
                        WorkflowMenu(store: store, historyID: item.id,
                                     contentType: item.fileCount > 0 ? .files : (item.hasImage ? .image : .text))
                        WorkflowShareMenu(store: store, historyID: item.id,
                                          contentType: item.fileCount > 0 ? .files : (item.hasImage ? .image : .text))
                    }
                }
                .font(.system(size: 11)).controlSize(.small)
                .disabled(copying || store.client == nil)
                WorkflowRunStatusView(compact: true)
                if let error = store.error { Text(error).font(.caption).foregroundStyle(.red).lineLimit(3) }
            }
            .padding(.horizontal, 16).padding(.vertical, 13)
            .modifier(NativeGlassBackdrop(cornerRadius: 0))
        }
    }

    @MainActor private func searchHistory(append: Bool = false) async {
        if append && searching { return }
        let requested = searchKey
        searching = true; searchError = nil
        defer { if requested == searchKey { searching = false } }
        do {
            if !append { try await Task.sleep(for: .milliseconds(120)) }
            let result = try await library.query(listID: listID, search: query, source: source, offset: append ? (nextOffset ?? 0) : 0)
            guard !Task.isCancelled, requested == searchKey, panelActive else { return }
            if append {
                let existing = Set(pageItems.map(\.id)); pageItems += result.items.filter { !existing.contains($0.id) }
            } else { pageItems = result.items }
            nextOffset = result.next_offset
            for item in result.items where item.has_image && (item.ocr_version ?? 0) < ClipboardOCRIndexer.version {
                ClipboardOCRIndexer.shared.schedule(item.id, client: store.client)
            }
            if selectedID.map({ id in items.contains(where: { $0.id == id }) }) != true { selectedID = items.first?.id }
        } catch {
            guard !Task.isCancelled, requested == searchKey else { return }
            searchError = error.userFacingMessage
        }
    }

    private func beginSession() {
        commandHeld = NSEvent.modifierFlags.contains(.command)
        session = UUID(); panelActive = true; copying = false; copyError = nil; searchFocused = true
        selectedID = items.first?.id
    }
    private func endSession() {
        dismissPreview()
        commandHeld = false
        panelActive = false; session = UUID(); copying = false
        cancelPreparation()
    }

    /// Read-only preview of the selected record: it never prepares a paste and
    /// never changes the selection. It is drawn in its own screen-sized window so
    /// a 440 pt panel is not the ceiling for an image; the dimmed backdrop appears
    /// immediately and the payload follows, so the keypress always has feedback.
    private func togglePreview() {
        guard selectedID != nil else { return }
        if previewing { dismissPreview() } else { previewSelected() }
    }
    private func dismissPreview() {
        previewing = false
        previewContent = nil
        previewImage = nil
        ClipboardQuickLookController.shared.close()
    }
    private func previewSelected() {
        guard let id = selectedID, items.contains(where: { $0.id == id }), store.client != nil else { return }
        previewing = true
        previewContent = nil
        previewImage = nil
        ClipboardQuickLookController.shared.present(over: panel.window) { dismissPreview() }
        Task { @MainActor in
            guard let client = store.client else { return }
            var content = try? await client.content(id)
            // A remote index row has no local payload yet. Ask the source device
            // and retry briefly, the same way the main window's preview does.
            if content == nil, pageItems.first(where: { $0.id == id })?.available == false {
                library.wantPayload(id)
                for _ in 0..<10 {
                    try? await Task.sleep(for: .milliseconds(400))
                    guard !Task.isCancelled, previewing, selectedID == id else { return }
                    if let retried = try? await client.content(id) { content = retried; break }
                }
            }
            guard !Task.isCancelled, previewing, selectedID == id else { return }
            // Unreadable record: never leave the spinner covering the screen.
            guard let content else { dismissPreview(); return }
            let media = await ClipboardMediaLoader.shared.load(content)
            guard !Task.isCancelled, previewing, selectedID == id else { return }
            previewContent = content
            previewImage = media.thumbnail.flatMap { NSImage(data: $0) }
            ClipboardQuickLookController.shared.update(content: content, image: previewImage)
        }
    }
    private func cancelPreparation() {
        guard let token = preparationToken else { return }
        preparationToken = nil
        if let client = store.client { Task { try? await client.cancelClipboardPreparation(token) } }
    }
    private func closePanel() { endSession(); ClipboardPanelController.shared.close() }
    private func deleteItem(_ id: UInt64) {
        fileAvailability[id] = nil
        if selectedID == id { selectedID = nil }
        library.delete([id])
    }
    /// Remote index rows have no local payload; ask the source device for it.
    private func requestPayloadIfNeeded(_ id: UInt64) {
        guard let item = pageItems.first(where: { $0.id == id }), item.available == false else { return }
        library.wantPayload(id)
    }
    private func isRemoteUnavailable(_ row: ClipboardLibraryItem?) -> Bool {
        row?.remote == true && row?.available == false
    }
    private func isOCRHit(_ row: ClipboardLibraryItem?, needle: String) -> Bool {
        guard !needle.isEmpty, let text = row?.ocr_text?.lowercased() else { return false }
        return text.contains(needle)
    }
    private func copySelected(paste: Bool) {
        guard canCopy, let item = selectedItem, let client = store.client else { return }
        let operation = session
        let nativeSession = ClipboardPanelController.shared.session
        let token = nativeSession.uuidString
        let generation = panel.generation
        preparationToken = token
        copying = true; copyError = nil
        Task { @MainActor in
            do {
                let changeCount = try await client.prepareClipboardPaste(item.id, session: token)
                guard panelActive, session == operation, panel.generation == generation,
                      preparationToken == token, ClipboardPanelController.shared.session == nativeSession else { return }
                if paste {
                    try await ClipboardPanelController.shared.pasteRestoredContent(expectedChangeCount: changeCount, in: nativeSession)
                } else {
                    copying = false; closePanel()
                }
                if session == operation { copying = false; preparationToken = nil }
            } catch {
                guard ClipboardPanelController.shared.session == nativeSession else { return }
                preparationToken = nil; copying = false; copyError = error.userFacingMessage
                ClipboardPanelController.shared.reportPasteFailure(error.userFacingMessage)
                store.error = error.userFacingMessage
            }
        }
    }

    private func showAll() {
        store.clipboardNavigation = ClipboardNavigationRequest(query: query, source: source, selectedID: selectedID, listID: listID)
        store.selectedPage = .clipboard
        closePanel(); openMainWindow()
    }
    private func openMainWindow() {
        if NSApp.windows.contains(where: { $0.identifier?.rawValue == "unidesk-main" }) { store.openMain() }
        else { openWindow(id: "main"); NSApp.activate(ignoringOtherApps: true) }
    }
    private func handleKey(_ event: NSEvent) -> Bool {
        let modifiers = event.modifierFlags.intersection([.command, .control, .option, .shift])
        let fieldEditor = panel.window?.firstResponder as? NSTextView
        if fieldEditor?.hasMarkedText() == true { return false }
        let editingText = fieldEditor != nil
        if modifiers == .command {
            let character = event.charactersIgnoringModifiers?.lowercased() ?? ""
            if character == "f" { searchFocused = true; return true }
            if let digit = Int(character), (1...9).contains(digit), items.indices.contains(digit - 1) {
                guard !copying else { return true }
                selectedID = items[digit - 1].id; copySelected(paste: true); return true
            }
        }
        guard modifiers.isEmpty else { return false }
        // Space opens the read-only quick look for the selected record. While the
        // search field has focus the space belongs to the field editor, so it is
        // only claimed once nothing is being typed into.
        if event.keyCode == 49 {
            guard !editingText else { return false }
            togglePreview()
            return true
        }
        // While a record's title is being edited (not the search field), let the
        // field editor own Return/Esc/arrows/Delete.
        if editingText && !searchFocused { return false }
        switch event.keyCode {
        case 53:
            // Esc leaves the preview before it reaches the panel itself.
            if previewing { dismissPreview() } else { closePanel() }
            return true
        case 36, 76: copySelected(paste: true); return true
        case 51, 117:
            // Backspace/Forward Delete belong to the focused search field while
            // the user is editing; only delete the selected record otherwise.
            if editingText { return false }
            guard let id = selectedID else { return true }
            deleteItem(id); return true
        case 125, 126:
            guard !items.isEmpty else { return true }
            let old = items.firstIndex { $0.id == selectedID } ?? 0
            let next = min(items.count - 1, max(0, old + (event.keyCode == 125 ? 1 : -1)))
            selectedID = items[next].id; searchFocused = false; copyError = nil; return true
        default: return false
        }
    }
}
