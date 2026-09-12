import SwiftUI
import UniDeskEngine

struct RootView: View {
    @Bindable var store: AppStore
    @State private var visibility: NavigationSplitViewVisibility = .all
    @State private var historySearchRequest = 0
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        NavigationSplitView(columnVisibility: $visibility) {
            sidebar
                .navigationSplitViewColumnWidth(min: 172, ideal: 180, max: 220)
        } detail: {
            VStack(spacing: 0) {
                if let error = store.error ?? store.snapshot?.error {
                    HStack(alignment: .top, spacing: 10) {
                        Image(systemName: "exclamationmark.triangle")
                        Text(error).font(.callout).textSelection(.enabled)
                        Spacer(minLength: 8)
                        if store.error != nil {
                            Button("关闭") { store.error = nil }
                        }
                    }
                    .padding(12)
                    .background(.yellow.opacity(0.12))
                }
                pageContent
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .navigationTitle("")
        }
        .navigationSplitViewStyle(.balanced)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .toolbar {
            ToolbarItemGroup(placement: .automatic) {
                if store.selectedPage == .clipboard {
                    Button(action: { historySearchRequest += 1 }) {
                        Image(systemName: "magnifyingglass")
                    }
                    .help("搜索历史")
                    .keyboardShortcut("f", modifiers: .command)
                }
                if store.selectedPage == .clipboard || store.selectedPage == .transfers {
                    Button(action: { store.shelf?.show() }) {
                        Image(systemName: "tray")
                    }
                    .help("打开文件暂存区（⇧⌘D）")
                }
            }
            if #available(macOS 26.0, *) { ToolbarSpacer(.fixed, placement: .automatic) }
            ToolbarItem(placement: .automatic) {
                SettingsLink {
                    Image(systemName: "gearshape")
                }
                .help("设置")
            }
        }
        .onAppear { store.showMainWindow = { openWindow(id: "main") } }
    }

    private var sidebar: some View {
        List(selection: $store.selectedPage) {
            Section("工作空间") {
                ForEach(Page.allCases) { page in
                    let isSelected = store.selectedPage == page
                    let textColor = isSelected ? Color.accentColor : Color(nsColor: .labelColor)
                    let iconColor = isSelected ? Color.accentColor : .secondary

                    Label {
                        Text(page.rawValue)
                            .font(.system(size: 13, weight: .medium))
                            .foregroundStyle(textColor)
                    } icon: {
                        Image(systemName: page.symbol)
                            .font(.system(size: 15))
                            .foregroundStyle(iconColor)
                            .frame(width: 20)
                    }
                    .padding(.vertical, 9)
                    .padding(.horizontal, 10)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .modifier(NavigationSelection(selected: isSelected))
                    .background(NativeListSelection())
                    .listRowInsets(EdgeInsets(top: 3, leading: 0, bottom: 3, trailing: 0))
                    .tag(page)
                }
            }
        }
        .listStyle(.sidebar)
        .safeAreaInset(edge: .top, spacing: 8) {
            HStack(spacing: 8) {
                Image(systemName: "square.grid.2x2")
                    .font(.system(size: 18, weight: .semibold))
                    .foregroundStyle(Color.accentColor)
                Text("uniDesk")
                    .font(.system(size: 19, weight: .bold))
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 20)
            .padding(.top, 20)
            .padding(.bottom, 8)
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            Button {
                store.selectedPage = .devices
            } label: {
                HStack(spacing: 10) {
                    Image(systemName: "laptopcomputer")
                        .font(.system(size: 23))
                        .foregroundStyle(.secondary)
                    VStack(alignment: .leading, spacing: 5) {
                        Text("这台 Mac")
                            .font(.system(size: 12, weight: .medium))
                            .lineLimit(1)
                        HStack(spacing: 5) {
                            Circle().fill(runtimeColor).frame(width: 6, height: 6)
                            Text(store.snapshot?.runtimeStatus ?? "正在启动")
                                .font(.caption2)
                                .foregroundStyle(.secondary)
                                .lineLimit(2)
                        }
                    }
                    Spacer(minLength: 0)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help("查看设备详情：\(store.snapshot?.localName ?? "这台 Mac")")
            .padding(12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .modifier(NativeGlassSurface(cornerRadius: 16))
            .padding(10)
        }
    }

    private var runtimeColor: Color {
        if store.error != nil || store.snapshot?.error != nil { return .orange }
        return store.snapshot == nil || store.starting ? .secondary : .green
    }

    @ViewBuilder private var pageContent: some View {
        if store.selfTest {
            ContentUnavailableView("正在确认更新启动", systemImage: "checkmark.shield", description: Text("初始化原生界面与本地存储"))
                .onAppear { store.confirmSelfTest() }
        } else if store.starting {
            ProgressView("正在启动 uniDesk…")
        } else if let snapshot = store.snapshot {
            switch store.selectedPage {
            case .clipboard: ClipboardView(store: store, snapshot: snapshot, searchRequest: historySearchRequest)
            case .input: InputView(store: store, snapshot: snapshot)
            case .devices: DevicesView(store: store, snapshot: snapshot)
            case .transfers: TransfersView(store: store, transfers: snapshot.transfers)
            }
        } else {
            ContentUnavailableView("无法启动", systemImage: "exclamationmark.triangle", description: Text(store.error ?? "引擎未就绪"))
        }
    }
}

/// Only the active navigation item receives glass; the sidebar itself remains
/// the standard NavigationSplitView material.
private struct NavigationSelection: ViewModifier {
    let selected: Bool
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    func body(content: Content) -> some View {
        if selected {
            if #available(macOS 26.0, *), !reduceTransparency {
                content.glassEffect(.regular.tint(.accentColor.opacity(0.12)).interactive(), in: RoundedRectangle(cornerRadius: 12))
            } else {
                content.background(Color.accentColor.opacity(0.16), in: RoundedRectangle(cornerRadius: 12))
            }
        } else {
            content
        }
    }
}
