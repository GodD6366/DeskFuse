import SwiftUI
import AppKit
import DeskFuseEngine

/// Navigation observes selection; runtime refreshes stay in the device row and footer.
struct MainSidebar: View {
    @Bindable var store: AppStore

    @Environment(\.colorScheme) private var scheme
    private var categorySelection: Binding<SettingsCategory> {
        Binding(get: { store.settingsDestination.category }, set: {
            store.settingsDestination.category = $0
            store.selectedPage = .settings
        })
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                DeskFuseBrandMark()
                Text("DeskFuse").font(DFType.title.weight(.semibold))
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 18).padding(.top, 4).padding(.bottom, 12)

            List(selection: $store.selectedPage) {
                sectionTitle("功能")
                ForEach(Page.allCases.filter { $0 != .devices && $0 != .settings }) { page in
                    DFSidebarRow(title: page.rawValue, symbol: page.symbol,
                        selected: store.selectedPage == page,
                        badge: page.isBeta ? "Beta" : nil)
                        .tag(page)
                        .listRowInsets(EdgeInsets(top: 1, leading: 0, bottom: 1, trailing: 0))
                        .listRowSeparator(.hidden)
                        .listRowBackground(Color.clear)
                }
                sectionTitle("配置")
                SidebarDevicesRow(store: store)
                    .tag(Page.devices)
                    .listRowInsets(EdgeInsets(top: 1, leading: 0, bottom: 1, trailing: 0))
                    .listRowSeparator(.hidden)
                    .listRowBackground(Color.clear)
            }
            .listStyle(.plain)
            .contentMargins(.horizontal, 0, for: .scrollContent)
            .listSectionSeparator(.hidden, edges: .all)
            .scrollContentBackground(.hidden)
            .environment(\.defaultMinListRowHeight, 32)
            // Plain List adds an 8 pt row inset; match the settings button's 10 pt edge.
            .padding(.horizontal, 2)

            VStack(spacing: 4) {
                Button { store.openSettings() } label: {
                    DFSidebarRow(title: "设置", symbol: "gearshape", selected: store.selectedPage == .settings)
                }
                .buttonStyle(.plain)
                .padding(.horizontal, 10)
                if store.selectedPage == .settings {
                    List(selection: categorySelection) {
                        ForEach(SettingsCategory.allCases, id: \.rawValue) { category in
                            DFSidebarRow(title: category.title,
                                            selected: store.settingsDestination.category == category)
                                .tag(category)
                                .listRowInsets(EdgeInsets(top: 0, leading: 0, bottom: 0, trailing: 0))
                                .listRowSeparator(.hidden)
                                .listRowBackground(Color.clear)
                        }
                    }
                    .listStyle(.plain)
                    .scrollContentBackground(.hidden)
                    .environment(\.defaultMinListRowHeight, 32)
                    .frame(height: CGFloat(SettingsCategory.allCases.count) * 32)
                    .padding(.leading, 26).padding(.trailing, 10)
                    .overlay(alignment: .leading) {
                        Rectangle().fill(DFPalette.line(scheme))
                            .frame(width: 1).padding(.leading, 20).padding(.vertical, 4)
                    }
                    .accessibilityLabel("设置分类")
                }
                SidebarHealthStatus(store: store)

            }
            .padding(.top, 8)
            .overlay(alignment: .top) {
                Rectangle().fill(DFPalette.line(scheme)).frame(height: 0.5)
            }
        }
        .dfWorkspace()
        .overlay(alignment: .trailing) {
            Rectangle().fill(DFPalette.line(scheme)).frame(width: 0.5)
        }
    }

    private func sectionTitle(_ title: String) -> some View {
        Text(title)
            .font(DFType.caption)
            .dfSecondaryText()
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 10)
            .padding(.top, 12)
            .padding(.bottom, 4)
            .listRowInsets(EdgeInsets(top: 0, leading: 0, bottom: 0, trailing: 0))
            .listRowSeparator(.hidden)
            .listRowBackground(Color.clear)
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
        .padding(.horizontal, 22).padding(.vertical, 10)
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
                        HStack(spacing: DFSpace.s) {
                            Circle().fill(issue.tone.color).frame(width: 6, height: 6)
                            Text(issue.title).font(DFType.bodyMedium)
                        }
                        if let detail = issue.detail {
                            Text(detail)
                                .font(DFType.detail)
                                .dfSecondaryText()
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        if let actionTitle = issue.actionTitle, let action = issue.action {
                            Button(actionTitle, action: action)
                                .buttonStyle(.dfSecondary)
                                .controlSize(.small)
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
