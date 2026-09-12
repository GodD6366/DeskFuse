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
    @State private var kind: ClipboardKind = .all
    @State private var source: String?
    @State private var selectedID: UInt64?
    @State private var matchedQuery = ""
    @State private var searchMatches: Set<UInt64> = []
    @State private var searching = false
    @State private var searchError: String?
    @State private var copyError: String?
    @State private var copying = false
    @State private var session = UUID()
    @State private var panelActive = false
    @State private var fileAvailability: [UInt64: Bool] = [:]
    @State private var panel = MenuBarPanelHandle()
    @State private var panelHeight: CGFloat = 660

    private var history: [HistoryItem] { store.snapshot?.history ?? [] }
    private var items: [HistoryItem] {
        Array(history.filter { item in
            kind.includes(item) && (source == nil || source == item.source) &&
            (query.isEmpty || (matchedQuery == query && searchMatches.contains(item.id)))
        }.prefix(20))
    }
    private var selectedItem: HistoryItem? { items.first { $0.id == selectedID } }
    private var canCopy: Bool {
        selectedItem != nil && store.client != nil && !copying &&
        !searching && (query.isEmpty || matchedQuery == query) &&
        selectedID.map { fileAvailability[$0] != false } == true
    }
    private var searchKey: MenuBarHistorySearchKey {
        MenuBarHistorySearchKey(query: query, history: history.map { MenuBarHistoryVersion(id: $0.id, copiedAt: $0.copiedAtMs) })
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            historyList
            Divider()
            footer
        }
        .frame(width: 440, height: panelHeight)
        .background(Color(nsColor: .textBackgroundColor))
        .background(MenuBarPanelBridge(handle: panel, opened: beginSession, closed: endSession, keyDown: handleKey))
        .task(id: searchKey) { await searchHistory() }
        .onChange(of: items.map(\.id), initial: true) { _, ids in
            guard query.isEmpty || matchedQuery == query else { return }
            if selectedID.map(ids.contains) != true { selectedID = ids.first }
            fileAvailability = fileAvailability.filter { ids.contains($0.key) }
        }
        .onChange(of: history.map(\.source)) { _, sources in
            if let source, !sources.contains(source) { self.source = nil }
        }
        .onDisappear { endSession() }
    }

    private var header: some View {
        NativeGlassGroup {
            VStack(spacing: 12) {
                HStack(spacing: 10) {
                    Image(systemName: "rectangle.on.rectangle").font(.system(size: 19))
                    Text("最近剪贴板").font(.system(size: 16, weight: .semibold))
                    Spacer()
                    Button { closePanel(); store.shelf?.show() } label: {
                        Image(systemName: "tray").frame(width: 22, height: 22)
                    }
                    .buttonStyle(.borderless)
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
                    } else {
                        Button { query = "" } label: { Image(systemName: "xmark.circle.fill") }
                            .buttonStyle(.plain).foregroundStyle(.secondary).accessibilityLabel("清除搜索")
                    }
                }
                .font(.system(size: 13)).padding(.horizontal, 11).frame(height: 34)
                .background(Color(nsColor: .textBackgroundColor), in: RoundedRectangle(cornerRadius: 9))
                .overlay(RoundedRectangle(cornerRadius: 9).strokeBorder(searchFocused ? Color.accentColor : Color(nsColor: .separatorColor), lineWidth: 1))
                Picker("内容类型", selection: $kind) {
                    ForEach(ClipboardKind.allCases) { Text($0.rawValue).tag($0) }
                }
                .pickerStyle(.segmented).labelsHidden()
                HStack {
                    Text(query.isEmpty && source == nil && kind == .all ? "最近 \(items.count) 条" : "匹配 \(items.count) 条")
                    if searching { ProgressView().controlSize(.mini) }
                    Spacer()
                    Menu {
                        Picker("来源设备", selection: $source) {
                            Text("全部设备").tag(String?.none)
                            ForEach(Array(Set(history.map(\.source))).sorted(), id: \.self) { value in
                                Text(value.isEmpty ? "本机" : value).tag(Optional(value))
                            }
                        }
                    } label: { Text(source ?? "全部设备").lineLimit(1) }
                    .menuStyle(.borderlessButton).fixedSize().frame(maxWidth: 160, alignment: .trailing)
                    .accessibilityLabel("来源设备")
                }
                .font(.system(size: 11)).foregroundStyle(.secondary)
            }
            .padding(16)
            .modifier(NativeGlassBackdrop(cornerRadius: 0))
        }
    }

    private var moreMenu: some View {
        Menu {
            Text(store.snapshot?.runtimeStatus ?? (store.starting ? "正在启动" : "引擎不可用"))
            Button("打开 uniDesk") { closePanel(); openMainWindow() }
            Button("文件暂存区") { closePanel(); store.shelf?.show() }
            Button("立即收回键鼠") { store.reclaim() }
            Divider()
            SettingsLink { Text("设置…") }
            Button("退出 uniDesk") { NSApp.terminate(nil) }
        } label: { Image(systemName: "ellipsis.circle").font(.system(size: 18)).frame(width: 24, height: 24) }
        .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize()
        .accessibilityLabel("更多操作")
    }

    private var historyList: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(spacing: 3) {
                    if items.isEmpty {
                        emptyState
                    } else {
                        ForEach(Array(items.enumerated()), id: \.element.id) { index, item in
                            MenuBarClipboardRow(item: item, index: index, selected: selectedID == item.id,
                                client: store.client, presentation: session, active: panelActive, availability: { available in
                                    fileAvailability[item.id] = available
                                }, select: { selectedID = item.id; searchFocused = false; copyError = nil })
                            .id(item.id)
                        }
                    }
                }
                .padding(.horizontal, 12).padding(.vertical, 10)
            }
            .scrollIndicators(.automatic)
            .onChange(of: selectedID) { _, id in
                if let id { proxy.scrollTo(id) }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
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
                if !history.isEmpty && (!query.isEmpty || source != nil || kind != .all) {
                    Button("清除筛选") { query = ""; kind = .all; source = nil; searchError = nil }
                }
            }
            .frame(maxWidth: .infinity).padding(.horizontal, 15).padding(.vertical, 45)
        }
    }

    private var footer: some View {
        NativeGlassGroup {
            VStack(spacing: 10) {
                if let message = copyError {
                    Label(message, systemImage: "exclamationmark.triangle")
                        .font(.caption).foregroundStyle(.orange)
                        .lineLimit(3).frame(maxWidth: .infinity, alignment: .leading)
                        .accessibilityLabel("复制失败：\(message)")
                }
                HStack {
                    Button { showAll() } label: { Label("查看全部", systemImage: "arrow.up.right") }
                        .buttonStyle(.borderless)
                    Spacer()
                    Button { copySelected() } label: {
                        HStack(spacing: 7) {
                            if copying { ProgressView().controlSize(.mini) }
                            else { Image(systemName: "doc.on.doc") }
                            Text(copying ? "正在复制" : "复制")
                            Text("↩").foregroundStyle(.secondary)
                        }.padding(.horizontal, 9).padding(.vertical, 3)
                    }
                    .modifier(NativeGlassAction()).disabled(!canCopy)
                    .help(selectedID.map { fileAvailability[$0] == false } == true ? "历史文件已清理或移动，请从源设备重新复制" : "复制选中历史（回车）")
                }
                if let item = selectedItem, item.hasImage || item.fileCount == 1 {
                    ImageWorkflowMenu(store: store, historyID: item.id)
                        .disabled(copying || store.client == nil)
                }
                WorkflowRunStatusView(compact: true)
                if let error = store.error { Text(error).font(.caption).foregroundStyle(.red).lineLimit(3) }
                Text("↑↓ 选择   ⌘1–9 快速复制   esc 关闭")
                    .font(.system(size: 11)).foregroundStyle(.secondary)
            }
            .padding(.horizontal, 16).padding(.vertical, 13)
            .modifier(NativeGlassBackdrop(cornerRadius: 0))
        }
    }

    @MainActor private func searchHistory() async {
        guard !query.isEmpty else { matchedQuery = ""; searchMatches = []; searching = false; searchError = nil; return }
        let requested = query
        searching = true; searchError = nil
        do {
            try await Task.sleep(for: .milliseconds(150))
            guard let client = store.client else { searching = false; return }
            let result = try await client.search(requested, filter: 0)
            guard !Task.isCancelled, requested == query else { return }
            matchedQuery = requested; searchMatches = Set(result); searching = false
        } catch {
            guard !Task.isCancelled, requested == query else { return }
            searchMatches = []; matchedQuery = requested
            searchError = error.userFacingMessage; searching = false
        }
    }

    private func beginSession() {
        session = UUID(); panelActive = true; copying = false; copyError = nil
        selectedID = items.first?.id
        if let screen = panel.window?.screen {
            panelHeight = min(660, max(360, screen.visibleFrame.height - 24))
        }
    }
    private func endSession() { panelActive = false; session = UUID(); copying = false }
    private func closePanel() { endSession(); panel.window?.orderOut(nil) }
    private func copySelected() {
        guard canCopy, let item = selectedItem else { return }
        let operation = session
        let generation = panel.generation
        copying = true; copyError = nil
        store.send(.restoreHistory(id: item.id), applied: {
            guard panelActive, session == operation, panel.generation == generation,
                  panel.window?.isVisible == true, panel.window?.isKeyWindow == true else { return }
            copying = false; closePanel()
        }, failed: { message in
            guard panelActive, session == operation, panel.generation == generation,
                  panel.window?.isVisible == true, panel.window?.isKeyWindow == true else { return }
            copying = false; copyError = message
        })
    }
    private func showAll() {
        store.clipboardNavigation = ClipboardNavigationRequest(query: query, kind: kind, source: source, selectedID: selectedID)
        store.selectedPage = .clipboard
        closePanel(); openMainWindow()
    }
    private func openMainWindow() {
        if NSApp.windows.contains(where: { $0.identifier?.rawValue == "unidesk-main" }) { store.openMain() }
        else { openWindow(id: "main"); NSApp.activate(ignoringOtherApps: true) }
    }
    private func handleKey(_ event: NSEvent) -> Bool {
        let modifiers = event.modifierFlags.intersection([.command, .control, .option, .shift])
        let hasMarkedText = (panel.window?.firstResponder as? NSTextView)?.hasMarkedText() == true
        if hasMarkedText { return false }
        if modifiers == .command {
            let character = event.charactersIgnoringModifiers?.lowercased() ?? ""
            if character == "f" { searchFocused = true; return true }
            if let digit = Int(character), (1...9).contains(digit), items.indices.contains(digit - 1) {
                guard !copying else { return true }
                selectedID = items[digit - 1].id; copySelected(); return true
            }
        }
        guard modifiers.isEmpty else { return false }
        switch event.keyCode {
        case 53: closePanel(); return true
        case 36, 76: copySelected(); return true
        case 125, 126:
            guard !items.isEmpty else { return true }
            let old = items.firstIndex { $0.id == selectedID } ?? 0
            let next = min(items.count - 1, max(0, old + (event.keyCode == 125 ? 1 : -1)))
            selectedID = items[next].id; searchFocused = false; copyError = nil; return true
        default: return false
        }
    }
}

private struct MenuBarHistoryVersion: Equatable { let id: UInt64; let copiedAt: UInt64? }
private struct MenuBarHistorySearchKey: Equatable { let query: String; let history: [MenuBarHistoryVersion] }
