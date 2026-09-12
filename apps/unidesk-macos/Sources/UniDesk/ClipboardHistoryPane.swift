import SwiftUI
import AppKit
import UniDeskEngine

struct ClipboardHistoryPane: View {
    let items: [HistoryItem]
    let sources: [String]
    @Binding var selected: UInt64?
    @Binding var search: String
    @Binding var kind: ClipboardKind
    @Binding var source: String?
    let searchRequest: Int
    @FocusState private var searchFocused: Bool
    @FocusState private var historyFocused: Bool
    let searching: Bool
    let historyIsEmpty: Bool
    @Binding var syncEnabled: Bool
    let selectedImage: NSImage?

    var body: some View {
        VStack(spacing: 0) {
            VStack(alignment: .leading, spacing: 20) {
                VStack(alignment: .leading, spacing: 7) {
                    Text("剪贴板").font(.system(size: 24, weight: .bold))
                    Text("在设备之间，接着用").font(.callout).foregroundStyle(.secondary)
                }
                searchField
                HStack(spacing: 4) {
                    ForEach(ClipboardKind.allCases) { option in
                        Button { kind = option } label: {
                            Text(option.rawValue)
                                .font(.system(size: 12, weight: kind == option ? .semibold : .regular))
                                .foregroundStyle(kind == option ? Color.accentColor : .secondary)
                                .frame(maxWidth: .infinity)
                                .padding(.vertical, 7)
                                .background(kind == option ? Color.accentColor.opacity(0.14) : .clear, in: Capsule())
                                .contentShape(Capsule())
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel("\(option.rawValue)历史")
                        .accessibilityAddTraits(kind == option ? [.isSelected] : [])
                    }
                }
                .accessibilityElement(children: .contain)
                .accessibilityLabel("内容类型")
                HStack {
                    Text("最近").font(.caption).foregroundStyle(.secondary)
                    Text("\(items.count)").font(.caption).monospacedDigit().foregroundStyle(.tertiary)
                    Spacer(minLength: 8)
                    Menu {
                        Picker("来源设备", selection: $source) {
                            Text("全部设备").tag(String?.none)
                            ForEach(sources, id: \.self) { Text($0).tag(Optional($0)) }
                        }
                    } label: {
                        HStack(spacing: 5) {
                            Text(source ?? "全部设备").lineLimit(1)
                            Image(systemName: "chevron.down").font(.system(size: 8, weight: .semibold))
                        }
                        .font(.caption).foregroundStyle(.secondary)
                    }
                    .menuStyle(.borderlessButton)
                    .menuIndicator(.hidden)
                    .fixedSize()
                    .frame(maxWidth: 132, alignment: .trailing)
                    .accessibilityLabel("来源设备")
                    .help("按来源设备筛选历史")
                }
            }
            .padding(.horizontal, 20)
            .padding(.top, 24)
            .padding(.bottom, 10)

            ScrollViewReader { proxy in
                List(selection: $selected) {
                    ForEach(items, id: \.id) { item in
                        ClipboardHistoryRow(item: item, image: item.id == selected ? selectedImage : nil)
                            .padding(.horizontal, 10)
                            .background(item.id == selected ? Color.accentColor.opacity(0.16) : .clear, in: RoundedRectangle(cornerRadius: 10))
                            .overlay {
                                RoundedRectangle(cornerRadius: 10)
                                    .strokeBorder(item.id == selected && historyFocused ? Color.accentColor.opacity(0.5) : .clear, lineWidth: 1)
                            }
                            .background(NativeListSelection())
                            .tag(item.id)
                            .listRowSeparator(.hidden)
                            .listRowInsets(EdgeInsets(top: 2, leading: 0, bottom: 2, trailing: 0))
                    }
                }
                .listStyle(.inset)
                .scrollContentBackground(.hidden)
                .environment(\.defaultMinListRowHeight, 60)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .overlay { if items.isEmpty { emptyHistory } }
                .focusable()
                .focusEffectDisabled()
                .focused($historyFocused)
                .simultaneousGesture(TapGesture().onEnded {
                    searchFocused = false
                    historyFocused = true
                })
                .onMoveCommand { direction in
                    guard direction == .up || direction == .down, !items.isEmpty else { return }
                    let current = items.firstIndex(where: { $0.id == selected }) ?? 0
                    let next = min(max(current + (direction == .down ? 1 : -1), 0), items.count - 1)
                    selected = items[next].id
                }
                .onChange(of: selected) { _, id in
                    if let id { proxy.scrollTo(id) }
                }
            }

            HStack(spacing: 8) {
                Circle().fill(syncEnabled ? Color.green : .secondary).frame(width: 7, height: 7)
                Text(syncEnabled ? "自动同步已开启" : "自动同步已暂停")
                    .font(.caption).foregroundStyle(.secondary)
                Spacer(minLength: 4)
                Toggle("自动同步", isOn: $syncEnabled)
                    .labelsHidden().toggleStyle(.switch).controlSize(.mini)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
            .modifier(NativeGlassSurface(cornerRadius: 20))
            .padding(12)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(nsColor: .underPageBackgroundColor).opacity(0.35))
        .onChange(of: searchRequest) { _, _ in historyFocused = false; searchFocused = true }
    }

    private var searchField: some View {
        HStack(spacing: 8) {
            Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
            TextField("搜索内容", text: $search)
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
        .frame(height: 34)
        .background(Color(nsColor: .textBackgroundColor), in: Capsule())
        .overlay {
            Capsule()
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
    let image: NSImage?

    var body: some View {
        HStack(spacing: 12) {
            Group {
                if let image {
                    Image(nsImage: image).resizable().scaledToFill()
                } else {
                    Image(systemName: item.clipboardSymbol)
                        .font(.system(size: 26, weight: .light))
                        .foregroundStyle(.secondary)
                }
            }
            .frame(width: 40, height: 44)
            .clipShape(RoundedRectangle(cornerRadius: 5))
            VStack(alignment: .leading, spacing: 6) {
                Text(item.clipboardTitle)
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(Color(nsColor: .labelColor)).lineLimit(1).truncationMode(.middle)
                Text("\(item.source) · \(item.fileCount > 0 ? "\(item.fileCount) 项" : item.hasImage ? byteCount(item.length) : item.clipboardKindLabel)")
                    .font(.system(size: 11)).foregroundStyle(.secondary).lineLimit(1)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.vertical, 4)
        .contentShape(Rectangle())
        .help(item.clipboardTitle)
    }
}

enum ClipboardKind: String, CaseIterable, Identifiable {
    case all = "全部", text = "文字", image = "图片", files = "文件"
    var id: Self { self }

    func includes(_ item: HistoryItem) -> Bool {
        switch self {
        case .all: true
        case .text: !item.hasImage && item.fileCount == 0
        case .image: item.hasImage
        case .files: item.fileCount > 0
        }
    }
}

extension HistoryItem {
    var clipboardTitle: String {
        let title = preview.trimmingCharacters(in: .whitespacesAndNewlines)
        if !title.isEmpty { return String(title.prefix(160)) }
        if fileCount > 0 { return "\(fileCount) 个文件" }
        return hasImage ? "剪贴板图片" : "剪贴板文字"
    }

    var clipboardSymbol: String { fileCount > 0 ? "doc.on.doc" : hasImage ? "photo" : "text.alignleft" }
    var clipboardKindLabel: String { fileCount > 0 ? "\(fileCount) 项 · \(byteCount(length))" : hasImage ? byteCount(length) : "文字" }
}
