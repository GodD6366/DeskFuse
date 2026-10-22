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
                    sectionTitle(Page.devices.section)
                    navButton(.devices) { SidebarDevicesRow(store: store) }
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
                        ForEach(SettingsCategory.allCases, id: \.rawValue) { category in
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
        let count = store.snapshot?.devices.filter(\.connected).count ?? 0
        DFSidebarRow(title: Page.devices.rawValue, symbol: Page.devices.symbol,
                        selected: store.selectedPage == .devices,
                        trailingText: "\(count) 台已连接")
    }
}

/// 侧栏底部：只显示一句整体状态，点开才展开需要处理的问题清单。
private struct SidebarHealthStatus: View {
    let store: AppStore
    @State private var showingIssues = false

    var body: some View {
        let issues = HealthCenter.issues(store)
        Button { showingIssues = true } label: {
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
        .padding(.horizontal, DFSpace.m).padding(.vertical, DFSpace.s + 2)
        .frame(maxWidth: .infinity, alignment: .leading)
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
            if UpdateInstallationChannel.current.isInternal {
                DFValueRow(title: "版本", value: SidebarHealthStatus.versionLabel)
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

extension SidebarHealthStatus {
    static let versionLabel: String = {
        let info = Bundle.main.infoDictionary ?? [:]
        let version = info["CFBundleShortVersionString"] as? String ?? "—"
        let channel = Bundle.main.url(forResource: "update-channel", withExtension: nil)
            .flatMap { try? String(contentsOf: $0, encoding: .utf8) }
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
        if channel != "github", let build = info["CFBundleVersion"] as? String, !build.isEmpty {
            return "v\(version) · build \(build)"
        }
        return "v\(version)"
    }()
}
