import SwiftUI
import AppKit
import DeskFuseEngine

/// Navigation observes selection; runtime refreshes stay in the device row and footer.
/// 导航项固定且很少，用按钮栈而不是 List：List 会按表格行样式缩小行内字号，导致与列表外的行不一致。
struct MainSidebar: View {
    @Bindable var store: AppStore

    @Environment(\.colorScheme) private var scheme

    var body: some View {
        VStack(spacing: 0) {
            // 品牌位放在红绿灯右侧的标题栏行内，与内容区面包屑同一基线。
            HStack(spacing: DFSpace.s) {
                DeskFuseBrandMark()
                Text("DeskFuse").font(DFType.title)
                Spacer(minLength: 0)
            }
            .padding(.leading, DFMetrics.trafficLightsInset)
            .frame(height: DFMetrics.titlebarHeight)
            .padding(.bottom, DFSpace.xs)

            ScrollView {
                VStack(spacing: DFSpace.xxs) {
                    // 还没配对任何电脑时，所有功能都用不了：把“设备”放在最上面作为第一步。
                    if needsFirstDevice { devicesSection }
                    ForEach([Page.primary, Page.extended], id: \.first) { pages in
                        sectionTitle(pages[0].section)
                        ForEach(pages) { page in
                            navButton(page) {
                                DFSidebarRow(title: page.rawValue, symbol: page.symbol,
                                    selected: store.selectedPage == page,
                                    badge: page.isBeta ? "Beta" : nil)
                            }
                        }
                    }
                    if !needsFirstDevice { devicesSection }
                }
                .padding(.horizontal, DFSpace.s + 2)
                // 给选中行阴影留出空间，避免被滚动区域裁切。
                .padding(.bottom, DFSpace.xs)
            }
            .scrollIndicators(.never)

            VStack(spacing: DFSpace.xxs) {
                navButton(.settings) {
                    DFSidebarRow(title: Page.settings.rawValue, symbol: Page.settings.symbol,
                                 selected: store.selectedPage == .settings)
                }
                if store.selectedPage == .settings {
                    VStack(spacing: DFSpace.xxs) {
                        ForEach(SettingsCategory.visible(workflowsEnabled: store.workflowsEnabled), id: \.rawValue) { category in
                            Button { store.settingsDestination.category = category } label: {
                                DFSidebarRow(title: category.title,
                                             selected: store.settingsDestination.category == category)
                            }
                            .buttonStyle(.plain)
                        }
                    }
                    .padding(.leading, DFSpace.l)
                    .overlay(alignment: .leading) {
                        Rectangle().fill(DFPalette.line(scheme))
                            .frame(width: DFMetrics.hairline * 2).padding(.leading, DFSpace.s + 2).padding(.vertical, DFSpace.xs)
                    }
                    .accessibilityElement(children: .contain)
                    .accessibilityLabel("设置分类")
                }
                SidebarHealthStatus(store: store)
            }
            .padding(.horizontal, DFSpace.s + 2)
            .padding(.top, DFSpace.s)
            .overlay(alignment: .top) {
                Rectangle().fill(DFPalette.line(scheme)).frame(height: DFMetrics.hairline)
            }
        }
        .dfWorkspace()
        .overlay(alignment: .trailing) {
            Rectangle().fill(DFPalette.line(scheme)).frame(width: DFMetrics.hairline)
        }
    }

    /// 快照未就绪时保持原顺序，避免启动瞬间侧栏跳动。
    private var needsFirstDevice: Bool {
        guard let devices = store.snapshot?.devices else { return false }
        return !devices.contains { $0.trusted && !$0.blocked }
    }

    @ViewBuilder private var devicesSection: some View {
        sectionTitle(Page.devices.section)
        navButton(.devices) { SidebarDevicesRow(store: store) }
    }

    private func navButton<Label: View>(_ page: Page, @ViewBuilder label: () -> Label) -> some View {
        Button {
            if page == .settings { store.openSettings() } else { store.selectedPage = page }
        } label: { label() }
        .buttonStyle(.plain)
    }

    private func sectionTitle(_ title: String) -> some View {
        Text(title)
            .font(DFType.caption)
            .dfSecondaryText()
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, DFSpace.s + 2)
            .padding(.top, DFSpace.m)
            .padding(.bottom, DFSpace.xs)
            .accessibilityAddTraits(.isHeader)
    }
}


private struct SidebarDevicesRow: View {
    let store: AppStore

    var body: some View {
        let paired = store.snapshot?.devices.filter { $0.trusted && !$0.blocked } ?? []
        // 与健康中心同一套措辞：没有配对时直接告诉用户下一步，而不是显示“0 台”。
        DFSidebarRow(title: Page.devices.rawValue, symbol: Page.devices.symbol,
                        selected: store.selectedPage == .devices,
                        trailingText: paired.isEmpty ? "未添加" : "\(paired.filter(\.connected).count) 台在线")
    }
}

/// 侧栏底部：只显示一句整体状态，点开才展开需要处理的问题清单；
/// 右侧的问号是全局固定的求助入口，任何页面都在同一个位置。
private struct SidebarHealthStatus: View {
    let store: AppStore
    @State private var showingIssues = false

    var body: some View {
        HStack(spacing: DFSpace.xs) {
            status
            SupportMenu(store: store)
        }
        .padding(.leading, DFSpace.m).padding(.trailing, DFSpace.xs).padding(.vertical, DFSpace.s + 2)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var status: some View {
        let issues = HealthCenter.issues(store)
        return Button { showingIssues = true } label: {
            HStack(spacing: DFSpace.s) {
                Circle().fill(HealthCenter.tone(of: issues).color).frame(width: 7, height: 7)
                Text(HealthCenter.summary(of: issues))
                    .font(DFType.detail)
                    .lineLimit(1)
                Spacer(minLength: 0)
                if !issues.isEmpty {
                    Image(systemName: "chevron.right")
                        .font(DFType.icon(.small, weight: .semibold))
                        .dfSecondaryText()
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help("查看运行状态与需要处理的问题")
        .accessibilityLabel("运行状态：\(HealthCenter.summary(of: issues))")
        .popover(isPresented: $showingIssues, arrowEdge: .top) {
            KeyboardIssueList(store: store, issues: issues)
                .dfFloating()
        }
    }
}

/// 侧栏底部的求助菜单：与菜单栏「帮助」的条目一致。
private struct SupportMenu: View {
    let store: AppStore

    var body: some View {
        Menu {
            Button("反馈问题…") { SupportCenter.reportProblem(store) }
            Button("复制版本与环境信息") { SupportCenter.copyEnvironment(store) }
            Divider()
            Button("已报告的问题") { SupportCenter.open(SupportCenter.issuesURL) }
            Button("发行说明") { SupportCenter.open(SupportCenter.releasesURL) }
        } label: {
            Image(systemName: "questionmark.circle")
                .font(DFType.icon(.regular))
                .dfSecondaryText()
                .frame(width: 24, height: 24)
                .contentShape(Rectangle())
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .fixedSize()
        .help("帮助与反馈")
        .accessibilityLabel("帮助与反馈")
    }
}

/// 健康问题清单：每条都有可直接执行的下一步。
struct KeyboardIssueList: View {
    let store: AppStore
    let issues: [HealthIssue]

    var body: some View {
        VStack(alignment: .leading, spacing: DFSpace.m) {
            HStack(spacing: DFSpace.s) {
                Text("运行状态").font(DFType.title)
                Spacer(minLength: DFSpace.s)
                Button { store.send(.refreshDevices) } label: {
                    Image(systemName: "arrow.clockwise").font(DFType.icon(.small, weight: .semibold))
                }
                .buttonStyle(.dfIcon)
                .help("刷新设备")
            }
            if issues.isEmpty {
                Text("DeskFuse 正在正常工作。")
                    .font(DFType.detail)
                    .dfSecondaryText()
            } else {
                ForEach(issues) { issue in
                    VStack(alignment: .leading, spacing: DFSpace.xs) {
                        // 动作紧跟标题，右边距内联显示；标题过长时才回退成上下两行。
                        ViewThatFits(in: .horizontal) {
                            HStack(spacing: DFSpace.s) {
                                title(for: issue)
                                Spacer(minLength: DFSpace.s)
                                action(for: issue)
                            }
                            VStack(alignment: .leading, spacing: DFSpace.xs) {
                                title(for: issue)
                                action(for: issue)
                            }
                        }
                        if let detail = issue.detail {
                            Text(detail)
                                .font(DFType.detail)
                                .dfSecondaryText()
                                .fixedSize(horizontal: false, vertical: true)
                        }
                    }
                    if issue.id != issues.last?.id { Divider() }
                }
            }
            Divider()
            DFValueRow(title: "本机", value: store.snapshot?.localName ?? "这台 Mac")
            DFValueRow(title: "版本", value: SupportCenter.versionLabel)
            Divider()
            HStack(spacing: DFSpace.s) {
                Text("遇到问题？").font(DFType.detail).dfSecondaryText()
                Spacer(minLength: DFSpace.s)
                Button("反馈问题…") { SupportCenter.reportProblem(store) }
                    .buttonStyle(.dfSecondary)
                    .controlSize(.small)
            }
        }
        .padding(DFSpace.l)
        .frame(width: 320, alignment: .leading)
    }

    private func title(for issue: HealthIssue) -> some View {
        HStack(spacing: DFSpace.s) {
            Circle().fill(issue.tone.color).frame(width: 6, height: 6)
            Text(issue.title).font(DFType.bodyMedium)
        }
    }

    @ViewBuilder
    private func action(for issue: HealthIssue) -> some View {
        if let actionTitle = issue.actionTitle, let action = issue.action {
            Button(actionTitle, action: action)
                .buttonStyle(.dfSecondary)
                .controlSize(.small)
                .fixedSize()
        }
    }
}
