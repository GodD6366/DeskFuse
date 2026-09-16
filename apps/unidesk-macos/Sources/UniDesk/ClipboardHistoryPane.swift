import SwiftUI
import AppKit
import UniDeskEngine

struct ClipboardHistoryPane: View {
    let items: [HistoryItem]
    let sources: [ClipboardLibraryChoice]
    @Binding var selected: UInt64?
    @Binding var search: String
    @Binding var source: String?
    @Binding var displayMode: ClipboardDisplayMode
    let searchRequest: Int
    @FocusState private var searchFocused: Bool
    @FocusState private var historyFocused: Bool
    let searching: Bool
    let historyIsEmpty: Bool
    @Binding var syncEnabled: Bool
    let selectedImage: NSImage?
    let client: EngineClient?
    let appIDs: [UInt64: String]
    let titles: [UInt64: String]
    let metas: [UInt64: ClipboardItemMeta]
    let onRename: (UInt64, String) -> Void
    let previewing: Bool
    let onTogglePreview: () -> Void
    let onDelete: (UInt64) -> Void
    @Binding var listID: String?
    let total: Int
    let memberships: [UInt64: [String]]
    let hasMore: Bool
    let paginationOffset: Int
    let paginationGeneration: UUID
    let loadMore: () -> Void

    var body: some View {
        VStack(spacing: 0) {
            header

            ScrollViewReader { proxy in
                Group {
                    if displayMode == .grid {
                        gridContent(proxy: proxy)
                    } else {
                        listContent(proxy: proxy)
                    }
                }
            }

            Divider().padding(.horizontal, 20)
            HStack(spacing: 8) {
                Text("自动同步剪贴板").font(.callout)
                Spacer(minLength: 4)
                Toggle("自动同步", isOn: $syncEnabled)
                    .labelsHidden().toggleStyle(.switch).controlSize(.small)
                    .help(syncEnabled ? "自动同步已开启" : "自动同步已关闭")
            }
            .padding(.horizontal, 20).padding(.vertical, 16)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(nsColor: .textBackgroundColor))
        .onChange(of: searchRequest) { _, _ in historyFocused = false; searchFocused = true }
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 14) {
            searchField
            ClipboardLibraryTabs(selectedListID: $listID, spacious: true)
            HStack(spacing: 10) {
                Menu {
                    Picker("来源设备", selection: $source) {
                        Text("所有设备").tag(String?.none)
                        ForEach(sources) { Text($0.name).tag(Optional($0.id)) }
                    }
                } label: {
                    HStack(spacing: 5) {
                        Text(sources.first(where: { $0.id == source })?.name ?? source ?? "所有设备").lineLimit(1)
                        Image(systemName: "chevron.down").font(.system(size: 8, weight: .semibold))
                    }
                    .font(.callout)
                }
                .menuStyle(.borderlessButton)
                .menuIndicator(.hidden)
                .fixedSize()
                .accessibilityLabel("来源设备")
                .help("按来源设备筛选历史")
                Spacer(minLength: 8)
                Text("\(total) 条").font(.caption).monospacedDigit().foregroundStyle(.secondary)
                    .accessibilityLabel("共 \(total) 条历史")
                ClipboardDisplayModePicker(mode: $displayMode)
            }
        }
        .padding(.horizontal, 20)
        .padding(.top, 20)
        .padding(.bottom, 10)
    }

    private func listContent(proxy: ScrollViewProxy) -> some View {
        interact(
            List(selection: $selected) {
                ForEach(items, id: \.id) { item in
                    ClipboardHistoryRow(item: item, selectedImage: nil,
                                        client: client, appID: appIDs[item.id], title: titles[item.id],
                                        meta: metas[item.id], ocrHit: ocrHit(item.id),
                                        memberships: memberships[item.id] ?? [],
                                        onRename: { onRename(item.id, $0) },
                                        onDelete: { onDelete(item.id) })
                        .padding(.horizontal, 10)
                        .background(item.id == selected ? Color.accentColor.opacity(0.12) : .clear, in: RoundedRectangle(cornerRadius: 12))
                        .overlay {
                            RoundedRectangle(cornerRadius: 10)
                                .strokeBorder(item.id == selected && historyFocused ? Color.accentColor.opacity(0.5) : .clear, lineWidth: 1)
                        }
                        .background(NativeListSelection())
                        .tag(item.id)
                        .listRowSeparator(.hidden)
                        .listRowInsets(EdgeInsets(top: 2, leading: 0, bottom: 2, trailing: 0))
                }
                if hasMore {
                    ClipboardPaginationTrigger(itemCount: paginationOffset, searching: searching, loadMore: loadMore)
                            .id(paginationGeneration)
                }
            }
            .listStyle(.inset)
            .scrollContentBackground(.hidden)
            .environment(\.defaultMinListRowHeight, 68),
            proxy: proxy
        )
    }

    private func gridContent(proxy: ScrollViewProxy) -> some View {
        interact(
            ClipboardMasonryGrid(items: items, selected: selected, columnCount: 2,
                                 hasMore: hasMore, paginationOffset: paginationOffset, paginationGeneration: paginationGeneration, searching: searching, loadMore: loadMore) { item, isSelected in
                ClipboardHistoryCard(
                    item: item,
                    selected: isSelected,
                    selectedImage: nil,
                    client: client,
                    appID: appIDs[item.id],
                    title: titles[item.id],
                    meta: metas[item.id],
                    ocrHit: ocrHit(item.id),
                    memberships: memberships[item.id] ?? [],
                    onRename: { onRename(item.id, $0) },
                    onSelect: {
                        selected = item.id
                        searchFocused = false
                        historyFocused = true
                    },
                    onDelete: { onDelete(item.id) }
                )
            },
            proxy: proxy
        )
    }

    /// Shared focus, keyboard, and scrolling behavior for both presentations.
    private func interact<V: View>(_ content: V, proxy: ScrollViewProxy) -> some View {
        content
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .overlay { if items.isEmpty { emptyHistory } }
            .focusable()
            .focusEffectDisabled()
            .focused($historyFocused)
            .simultaneousGesture(TapGesture().onEnded {
                searchFocused = false
                historyFocused = true
            })
            .onMoveCommand(perform: moveSelection)
            .onDeleteCommand { if let selected { onDelete(selected) } }
            .onKeyPress(.space) {
                guard !items.isEmpty, selected != nil else { return .ignored }
                onTogglePreview()
                return .handled
            }
            .onKeyPress(.escape) {
                guard previewing else { return .ignored }
                onTogglePreview()
                return .handled
            }
            .onChange(of: selected) { _, id in
                if let id { proxy.scrollTo(id) }
            }
    }

    /// True when the current search matched a record's recognized image text,
    /// so the row can explain why an image without visible text is listed.
    private func ocrHit(_ id: UInt64) -> Bool {
        let query = search.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !query.isEmpty, let text = metas[id]?.ocrText?.lowercased(), !text.isEmpty else { return false }
        return text.contains(query)
    }

    private func moveSelection(_ direction: MoveCommandDirection) {
        guard !items.isEmpty else { return }
        let current = items.firstIndex(where: { $0.id == selected }) ?? 0
        let step: Int
        switch direction {
        case .up: step = displayMode == .grid ? -2 : -1
        case .down: step = displayMode == .grid ? 2 : 1
        case .left: step = -1
        case .right: step = 1
        default: return
        }
        let next = min(max(current + step, 0), items.count - 1)
        selected = items[next].id
    }

    private var searchField: some View {
        HStack(spacing: 8) {
            Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
            TextField("搜索剪贴板", text: $search)
                .textFieldStyle(.plain)
                .focused($searchFocused)
                .onExitCommand { search = ""; searchFocused = false }
            if search.isEmpty {
                Text("⌘ F").font(.caption).foregroundStyle(.tertiary)
            } else {
                Button { search = "" } label: {
                    Image(systemName: "xmark.circle.fill").foregroundStyle(.secondary)
                }
                .buttonStyle(.plain).help("清除搜索")
                .accessibilityLabel("清除搜索")
            }
        }
        .font(.system(size: 13))
        .padding(.horizontal, 12)
        .frame(height: 40)
        .background(Color(nsColor: .quaternaryLabelColor).opacity(0.35), in: RoundedRectangle(cornerRadius: 10))
        .overlay {
            RoundedRectangle(cornerRadius: 10)
                .strokeBorder(searchFocused ? Color.accentColor : .clear, lineWidth: 2)
        }
    }

    private var emptyHistory: some View {
        VStack(spacing: 10) {
            if searching {
                ProgressView().controlSize(.small)
                Text("正在搜索…").font(.callout).foregroundStyle(.secondary)
            } else {
                Image(systemName: historyIsEmpty ? "clipboard" : "magnifyingglass")
                    .font(.system(size: 28)).foregroundStyle(.tertiary)
                Text(historyIsEmpty ? "还没有剪贴板历史" : "没有匹配的内容")
                    .font(.callout.weight(.medium))
                Text(historyIsEmpty ? "复制文字、图片或文件后\n会显示在这里。" : "试试其他关键词或筛选条件。")
                    .font(.caption).foregroundStyle(.secondary).multilineTextAlignment(.center)
            }
        }
        .padding(20)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .allowsHitTesting(false)
    }
}

private struct ClipboardHistoryRow: View {
    let item: HistoryItem
    let selectedImage: NSImage?
    let client: EngineClient?
    let appID: String?
    let title: String?
    let meta: ClipboardItemMeta?
    let ocrHit: Bool
    let memberships: [String]
    let onRename: (String) -> Void
    let onDelete: () -> Void
    @State private var thumbnail: NSImage?
    @State private var editingTitle = false

    var body: some View {
        HStack(spacing: 12) {
            // Region 1: the app that produced the copy.
            appIcon
                .frame(width: 30, height: 30)
            // Region 2: the content itself. Images are centered in the row;
            // text and files keep only their short title.
            content
            VStack(alignment: .trailing, spacing: 4) {
                if let date = item.copiedDate {
                    Text(Self.timeFormatter.string(from: date))
                        .font(.system(size: 12)).monospacedDigit().foregroundStyle(.secondary)
                }
                HStack(spacing: 4) {
                    if ocrHit {
                        Image(systemName: "text.viewfinder")
                            .font(.system(size: 11)).foregroundStyle(.secondary)
                            .help("由图片识别文字匹配")
                    }
                    if meta?.remote == true && meta?.available == false {
                        Image(systemName: "arrow.down.circle")
                            .font(.system(size: 11)).foregroundStyle(.secondary)
                            .help("使用时从来源设备获取")
                    }
                }
            }
            .frame(alignment: .trailing)
        }
        .padding(.vertical, 6)
        .contentShape(Rectangle())
        .help(item.preview)
        .clipboardHistoryContextMenu(itemID: item.id, memberships: memberships,
            meta: meta, onEdit: { editingTitle = true }, onDelete: onDelete)
        .sheet(isPresented: $editingTitle) {
            ClipboardTitleEditor(title: title, onSave: onRename)
        }
        .task(id: "\(item.id)-\(item.copiedAtMs ?? 0)") {
            guard item.hasImage || item.fileCount > 0, let client else { return }
            // Reuse a previously decoded thumbnail so switching smart lists does
            // not read and re-decode every visible image again.
            if let cached = ClipboardThumbnailCache.shared.cached(item.id) { thumbnail = cached; return }
            let image = await ClipboardThumbnailCache.shared.thumbnail(for: item.id, client: client, sourceIsImage: item.hasImage)
            guard !Task.isCancelled else { return }
            thumbnail = image
        }
        .onDisappear { thumbnail = nil }
    }

    @ViewBuilder private var appIcon: some View {
        if let icon = ClipboardAppInfoCache.shared.icon(for: appID) {
            Image(nsImage: icon).resizable().scaledToFit()
                .clipShape(RoundedRectangle(cornerRadius: 7))
                .accessibilityHidden(true)
        } else {
            Image(systemName: item.clipboardSymbol)
                .font(.system(size: 22, weight: .regular))
                .foregroundStyle(.secondary)
        }
    }

    @ViewBuilder private var content: some View {
        if item.hasImage {
            VStack(alignment: .leading, spacing: 4) {
                Group {
                    if let image = thumbnail {
                        Image(nsImage: image).resizable().scaledToFit()
                    } else {
                        RoundedRectangle(cornerRadius: 8)
                            .fill(Color(nsColor: .quaternaryLabelColor).opacity(0.25))
                            .overlay { Image(systemName: "photo").foregroundStyle(.secondary) }
                    }
                }
                    .frame(width: 76, height: 76)
                    .clipShape(RoundedRectangle(cornerRadius: 8))
                    .frame(maxWidth: .infinity, alignment: .center)
                if title?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false {
                    titleLabel
                }
            }
        } else {
            titleLabel
        }
    }

    private var titleLabel: some View {
        ClipboardTitleLabel(
            title: title,
            fallback: item.clipboardTitle,
            editing: $editingTitle,
            font: .system(size: 14, weight: .semibold)
        )
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private static let timeFormatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.dateFormat = "HH:mm"
        return formatter
    }()
}

extension HistoryItem {
    var copiedDate: Date? {
        copiedAtMs.map { Date(timeIntervalSince1970: Double($0) / 1000) }
    }

    var clipboardTitle: String {
        let title = preview.trimmingCharacters(in: .whitespacesAndNewlines)
        if hasImage && ["", "图像", "图片"].contains(title) {
            return "剪贴板图片"
        }
        if let url = URL(string: title), ["http", "https"].contains(url.scheme?.lowercased() ?? ""),
           let host = url.host, !title.contains(where: \.isWhitespace) {
            let path = url.path.removingPercentEncoding ?? url.path
            return String((host + (path == "/" ? "" : path)).prefix(160))
        }
        if !title.isEmpty { return String(title.prefix(160)) }
        if fileCount > 0 { return "\(fileCount) 个文件" }
        return hasImage ? "剪贴板图片" : "剪贴板文字"
    }

    var clipboardSymbol: String { fileCount > 0 ? "doc.on.doc" : hasImage ? "photo" : "text.alignleft" }
    var clipboardKindLabel: String { fileCount > 0 ? "\(fileCount) 项 · \(byteCount(length))" : hasImage ? byteCount(length) : "文字" }
}
