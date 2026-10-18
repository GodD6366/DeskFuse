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
                Text("DeskFuse").font(.system(size: 16, weight: .semibold))
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 18).padding(.top, 4).padding(.bottom, 12)

            List(selection: $store.selectedPage) {
                sectionTitle("功能")
                ForEach(Page.allCases.filter { $0 != .devices && $0 != .settings }) { page in
                    QuietSidebarRow(title: page.rawValue, symbol: page.symbol,
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
                    QuietSidebarRow(title: "设置", symbol: "gearshape", selected: store.selectedPage == .settings)
                }
                .buttonStyle(.plain)
                .padding(.horizontal, 10)
                if store.selectedPage == .settings {
                    List(selection: categorySelection) {
                        ForEach(SettingsCategory.allCases, id: \.rawValue) { category in
                            QuietSidebarRow(title: category.title,
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
                        Rectangle().fill(QuietWorkspacePalette.line(scheme))
                            .frame(width: 1).padding(.leading, 20).padding(.vertical, 4)
                    }
                    .accessibilityLabel("设置分类")
                }
                SidebarDeviceStatus(store: store)

            }
            .padding(.top, 8)
            .overlay(alignment: .top) {
                Rectangle().fill(QuietWorkspacePalette.line(scheme)).frame(height: 0.5)
            }
        }
        .quietWorkspaceStyle()
        .overlay(alignment: .trailing) {
            Rectangle().fill(QuietWorkspacePalette.line(scheme)).frame(width: 0.5)
        }
    }

    private func sectionTitle(_ title: String) -> some View {
        Text(title)
            .font(.system(size: 11))
            .workspaceSecondaryText()
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


private struct DeskFuseBrandMark: View {
    @Environment(\.colorScheme) private var colorScheme

    private var image: NSImage? {
        let name = colorScheme == .dark ? "DeskFuse-icon-1024" : "DeskFuse-icon-light-1024"
        return Bundle.main.url(forResource: name, withExtension: "png")
            .flatMap { NSImage(contentsOf: $0) }
    }

    var body: some View {
        Group {
            if let image {
                Image(nsImage: image)
                    .resizable()
                    .interpolation(.high)
                    .scaledToFit()
            } else {
                Image(systemName: "rectangle.on.rectangle")
                    .resizable()
                    .scaledToFit()
            }
        }
        .frame(width: 22, height: 22)
        .accessibilityHidden(true)
    }
}

private struct SidebarDevicesRow: View {
    let store: AppStore

    var body: some View {
        let count = store.snapshot?.devices.filter(\.connected).count ?? 0
        QuietSidebarRow(title: Page.devices.rawValue, symbol: Page.devices.symbol,
                        selected: store.selectedPage == .devices,
                        trailingText: "\(count) 台已连接")
    }
}

private struct SidebarDeviceStatus: View {
    let store: AppStore

    var body: some View {
        HStack(spacing: 10) {
            Button { store.showLocalDevice() } label: {
                VStack(alignment: .leading, spacing: 3) {
                    HStack(spacing: 8) {
                        Circle().fill(runtimeColor).frame(width: 7, height: 7)
                        Text(store.snapshot?.runtimeStatus ?? "正在启动")
                            .font(.system(size: 12, weight: .medium))
                            .lineLimit(1)
                    }
                    Text(Self.versionLabel)
                        .font(.system(size: 12, weight: .regular))
                        .workspaceSecondaryText()
                        .lineLimit(1)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help("查看本机运行状态：\(store.snapshot?.localName ?? "这台 Mac")")
            Spacer(minLength: 0)
            Button { store.checkForUpdates() } label: {
                Group {
                    if store.isCheckingForUpdates {
                        ProgressView().controlSize(.small)
                    } else {
                        Image(systemName: "arrow.clockwise")
                            .font(.system(size: 13, weight: .semibold))
                    }
                }
                .frame(width: 26, height: 26)
            }
            .buttonStyle(.plain)
            .disabled(store.client == nil || store.isCheckingForUpdates)
            .accessibilityLabel(store.isCheckingForUpdates ? "正在检查更新" : "检查更新")
            .help(store.isCheckingForUpdates ? "正在检查更新…" : "检查更新")
        }
        .padding(.horizontal, 12).padding(.vertical, 8)
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 10)
        .padding(.bottom, 10)
    }

    private static let versionLabel: String = {
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

    private var runtimeColor: Color {
        if store.error != nil || store.snapshot?.error != nil { return .orange }
        return store.snapshot == nil || store.starting ? .secondary : .green
    }

}
