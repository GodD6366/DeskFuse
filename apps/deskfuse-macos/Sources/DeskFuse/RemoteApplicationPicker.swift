import SwiftUI
import AppKit
import DeskFuseRemoteApplication

private enum RemoteApplicationCategory: String, CaseIterable, Identifiable {
    case all = "全部", running = "运行中", productivity = "效率与财务", utilities = "工具"
    case developer = "开发者工具", social = "社交", entertainment = "娱乐"
    case information = "信息与阅读", creativity = "创意", other = "其他"
    var id: String { rawValue }
    static func category(for application: RemoteApplicationInfo) -> Self {
        let category = application.category ?? ""
        if category.contains("developer") { return .developer }
        if ["business", "productivity", "finance"].contains(where: category.contains) { return .productivity }
        if category.contains("utilities") { return .utilities }
        if category.contains("social") { return .social }
        if ["graphics", "design", "photography", "video"].contains(where: category.contains) { return .creativity }
        if ["entertainment", "games", "music", "sports"].contains(where: category.contains) { return .entertainment }
        if ["news", "weather", "education", "reference", "books", "medical", "health", "travel"].contains(where: category.contains) { return .information }
        return .other
    }
}

@available(macOS 14.2, *)
struct RemoteApplicationDisplayModeToggle: View {
    @Bindable private var remote = RemoteApplicationManager.shared
    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 7) {
                Toggle("虚拟屏幕模式", isOn: $remote.virtualDisplayEnabled)
                    .toggleStyle(.switch).controlSize(.small)
                FeatureBadge(text: "实验性")
            }
                .help("开启后，每个远程应用使用专属虚拟屏幕；关闭后使用被控 Mac 上的原有应用窗口。不会立即改变已打开的会话。")
            Text("下次打开或重新连接时生效")
                .font(.caption).foregroundStyle(.secondary)
        }
    }
}

/// An independent launcher. Selecting an app creates/focuses its remote window;
/// the catalog connection stays available for opening the next application.
@available(macOS 14.2, *)
struct RemoteApplicationPicker: View {
    @Bindable var session: RemoteApplicationSession
    var embedded = false
    var select: (RemoteApplicationInfo) -> Void
    @Bindable private var remote = RemoteApplicationManager.shared
    @Environment(\.quietWorkspace) private var quiet
    @Environment(\.colorScheme) private var scheme
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    @State private var category: RemoteApplicationCategory = .all
    @State private var selection: UUID?
    @FocusState private var searchFocused: Bool
    @FocusState private var gridFocused: Bool

    private var applications: [RemoteApplicationInfo] {
        session.sortedApplications.filter {
            category == .all || (category == .running && $0.processID != nil)
                || RemoteApplicationCategory.category(for: $0) == category
        }
    }
    private var opened: Set<String> { remote.openedApplications(peerID: session.peerID) }

    var body: some View {
        VStack(spacing: 0) {
            header.padding(.horizontal, 20).padding(.top, 10).padding(.bottom, 12)
            Divider().padding(.horizontal, 20)
            categories.padding(.horizontal, 20).padding(.vertical, 10)
            if !session.applicationsLoaded {
                Spacer()
                VStack(spacing: 14) {
                    if session.active || session.busy { ProgressView() }
                    Text(session.failure ?? session.status).foregroundStyle(.secondary)
                        .multilineTextAlignment(.center).textSelection(.enabled)
                    if !session.active && !session.busy {
                        Button("重新连接") { session.refreshApplications() }.buttonStyle(WorkspaceButtonStyle(prominent: true))
                    }
                }.padding(32)
                Spacer()
            } else if applications.isEmpty {
                ContentUnavailableView.search(text: session.applicationQuery)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                GeometryReader { geometry in
                    let columns = max(1, min(7, Int((geometry.size.width - 40) / 104)))
                    ScrollViewReader { scroll in
                        ScrollView {
                            let running = applications.filter { $0.processID != nil }
                            let installed = applications.filter { $0.processID == nil }
                            VStack(spacing: 12) {
                                if !running.isEmpty { grid(running, columns: columns) }
                                if !running.isEmpty && !installed.isEmpty { Divider().padding(.horizontal, 8) }
                                if !installed.isEmpty { grid(installed, columns: columns) }
                            }
                            .padding(.horizontal, 20).padding(.top, 4).padding(.bottom, 16)
                        }
                        .focusable().focused($gridFocused).focusEffectDisabled()
                        .onMoveCommand { direction in moveSelection(direction, columns: columns) }
                        .onKeyPress(.return) {
                            guard let app = applications.first(where: { $0.id == selection }) else { return .ignored }
                            select(app); return .handled
                        }
                        .onChange(of: selection) { _, selected in
                            if let selected { scroll.scrollTo(selected) }
                        }
                    }
                }
            }
            HStack {
                Text(session.peerName)
                Spacer()
                Text("\(opened.count) 个已打开")
            }
            .font(.caption).foregroundStyle(.secondary)
            .padding(.horizontal, 20).padding(.vertical, 8)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background { panelBackground.ignoresSafeArea() }
        .onAppear { searchFocused = true }
        .onChange(of: category) { _, _ in selection = nil }
        .onChange(of: session.applicationQuery) { _, _ in selection = nil }
    }
    private var header: some View {
        HStack(spacing: 10) {
            Image(systemName: "square.grid.2x2").font(WorkspaceTypography.body).foregroundStyle(.secondary)
            TextField("应用程序", text: $session.applicationQuery)
                .textFieldStyle(.plain).font(WorkspaceTypography.body)
                .focused($searchFocused)
                .onSubmit { if let first = applications.first { select(first) } }
                .onKeyPress(.downArrow) {
                    selection = applications.first?.id; gridFocused = true; return .handled
                }
                .accessibilityLabel("搜索远端应用程序")
            if !session.applicationQuery.isEmpty {
                Button { session.applicationQuery = ""; searchFocused = true } label: {
                    Image(systemName: "xmark.circle.fill").foregroundStyle(.secondary)
                }.buttonStyle(.plain).help("清除搜索")
            }
            Menu {
                Button("刷新应用列表") { session.refreshApplications() }
                Button("搜索应用") { searchFocused = true }.keyboardShortcut("f", modifiers: .command)
            } label: {
                Image(systemName: "ellipsis").font(.system(size: 16))
            }.menuStyle(.borderlessButton).fixedSize().help("应用列表操作")
        }
    }
    private var categories: some View {
        HStack(spacing: 8) {
            ViewThatFits(in: .horizontal) {
                NativeGlassGroup { categoryButtons }
                Picker("分类", selection: $category) {
                    ForEach(RemoteApplicationCategory.allCases) { item in
                        Text(item.rawValue).tag(item)
                    }
                }
                .pickerStyle(.menu)
                .fixedSize()
            }
            Spacer(minLength: 8)
            displayModeControl
        }
        .fixedSize(horizontal: false, vertical: true)
    }
    // 虚拟屏幕模式是高频查看项，直接放在分类行右侧，不收进菜单。
    private var displayModeControl: some View {
        HStack(spacing: 6) {
            Text("虚拟屏幕").font(.system(size: 12))
            FeatureBadge(text: "实验性")
            Toggle("虚拟屏幕模式", isOn: $remote.virtualDisplayEnabled)
                .toggleStyle(.switch).controlSize(.small).labelsHidden()
        }
        .help("开启后，每个远程应用使用专属虚拟屏幕；关闭后使用被控 Mac 上的原有应用窗口。下次打开或重新连接时生效。")
    }
    private var categoryButtons: some View {
        HStack(spacing: 5) {
            ForEach(RemoteApplicationCategory.allCases) { item in
                categoryButton(item)
                    .buttonStyle(WorkspaceButtonStyle(prominent: category == item, compact: true))
            }
        }
    }
    private func categoryButton(_ item: RemoteApplicationCategory) -> some View {
        Button(item.rawValue) { category = item }.buttonBorderShape(.capsule)
            .fixedSize().lineLimit(1)
            .accessibilityAddTraits(category == item ? .isSelected : [])
    }
    @ViewBuilder private var panelBackground: some View {
        if embedded {
            QuietWorkspacePalette.canvas(scheme)
        } else if reduceTransparency {
            Color(nsColor: .windowBackgroundColor)
        } else if #available(macOS 26.0, *) {
            Color.clear.glassEffect(.regular, in: RoundedRectangle(cornerRadius: 28))
        } else { Rectangle().fill(.regularMaterial) }
    }
    private func grid(_ values: [RemoteApplicationInfo], columns: Int) -> some View {
        LazyVGrid(columns: Array(repeating: GridItem(.flexible(minimum: 0), spacing: 10), count: columns), spacing: 12) {
            ForEach(values) { application in
                RemoteApplicationTile(application: application, isOpen: opened.contains(application.applicationKey),
                    isSelected: selection == application.id) {
                    selection = application.id; select(application)
                }
                .id(application.id)
            }
        }
    }
    private func moveSelection(_ direction: MoveCommandDirection, columns: Int) {
        let values = applications
        guard !values.isEmpty else { return }
        let index = values.firstIndex(where: { $0.id == selection }) ?? 0
        // The running/installed divider starts a new grid row.
        let runningCount = values.prefix { $0.processID != nil }.count
        var slots: [UUID?] = values.prefix(runningCount).map { Optional($0.id) }
        if runningCount > 0 && runningCount < values.count {
            slots += Array(repeating: nil, count: (columns - runningCount % columns) % columns)
        }
        slots += values.dropFirst(runningCount).map { Optional($0.id) }
        let slot = slots.firstIndex(where: { $0 == values[index].id }) ?? 0
        let delta: Int
        switch direction { case .left: delta = -1; case .right: delta = 1; case .up: delta = -columns; case .down: delta = columns; default: return }
        var target = min(max(0, slot + delta), slots.count - 1)
        while slots[target] == nil && target > 0 && target < slots.count - 1 { target += delta > 0 ? 1 : -1 }
        selection = slots[target] ?? selection
    }
}

@available(macOS 14.2, *)
private struct RemoteApplicationTile: View {
    let application: RemoteApplicationInfo
    let isOpen: Bool
    let isSelected: Bool
    let action: () -> Void
    @State private var icon: NSImage?
    @State private var hovering = false
    var body: some View {
        Button(action: action) {
            VStack(spacing: 7) {
                Group {
                    if let icon { Image(nsImage: icon).resizable().interpolation(.high) }
                    else { Image(systemName: "app.fill").resizable().foregroundStyle(.secondary) }
                }
                .scaledToFit().frame(width: 64, height: 64)
                Text(application.name).font(.system(size: 12)).lineLimit(1).truncationMode(.tail)
                    .frame(maxWidth: .infinity)
                HStack(spacing: 4) {
                    if isOpen || application.processID != nil { Circle().fill(isOpen ? Color.accentColor : .secondary).frame(width: 5, height: 5) }
                    Text(isOpen ? "已打开" : application.processID != nil ? "运行中" : " ")
                }.font(.system(size: 10)).foregroundStyle(.secondary)
            }
            .padding(.horizontal, 4).padding(.vertical, 8)
            .frame(maxWidth: .infinity)
            .background((isSelected ? DeskFuseBrand.selectionFill : Color.primary.opacity(hovering ? 0.06 : 0)),
                        in: RoundedRectangle(cornerRadius: 12))
            .contentShape(RoundedRectangle(cornerRadius: 12))
        }
        .buttonStyle(.plain).focusable(false)
        .onHover { hovering = $0 }
        .help(application.name + (isOpen ? " · 显示已打开窗口" : " · 打开远程窗口"))
        .accessibilityLabel(application.name + (isOpen ? "，已打开" : ""))
        .task(id: application.iconPNG) { icon = application.iconPNG.flatMap { NSImage(data: $0) } }
    }
}
