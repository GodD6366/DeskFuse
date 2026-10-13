import SwiftUI
import AppKit
import DeskFuseEngine

/// Navigation observes selection; runtime refreshes are confined to the footer.
struct MainSidebar: View {
    @Bindable var store: AppStore

    var body: some View {
        List(selection: $store.selectedPage) {
            Section("功能") {
                ForEach(Page.allCases.filter { $0 != .devices }) { page in
                    SystemSettingsSidebarRow(
                        title: page.rawValue,
                        symbol: page.symbol,
                        color: page.color,
                        selected: store.selectedPage == page,
                        badge: page == .remoteApplications ? "Beta" : nil
                    )
                    .tag(page)
                    .listRowInsets(SystemSettingsMetrics.rowInsets)
                    .listRowBackground(Color.clear)
                }
            }
            Section("配置") {
                SystemSettingsSidebarRow(
                    title: Page.devices.rawValue,
                    symbol: Page.devices.symbol,
                    color: Page.devices.color,
                    selected: store.selectedPage == .devices
                )
                .tag(Page.devices)
                .listRowInsets(SystemSettingsMetrics.rowInsets)
                .listRowBackground(Color.clear)
                Button { store.openSettings() } label: {
                    SystemSettingsSidebarRow(title: "设置", symbol: "gearshape", color: .gray, selected: false)
                }
                .buttonStyle(.plain)
                .listRowInsets(SystemSettingsMetrics.rowInsets)
                .listRowBackground(Color.clear)
            }
        }
        .listStyle(.sidebar)
        .listSectionSeparator(.hidden, edges: .all)
        .scrollContentBackground(.hidden)
        .environment(\.defaultMinListRowHeight, SystemSettingsMetrics.rowHeight)
        .safeAreaInset(edge: .top, spacing: 0) {
            HStack(alignment: .top, spacing: 10) {
                VStack(alignment: .leading, spacing: 6) {
                    HStack(spacing: 8) {
                        DeskFuseBrandMark()
                        Text("DeskFuse")
                            .font(.system(size: 20, weight: .semibold))
                            .lineLimit(1)
                            .fixedSize()
                    }
                    Text("跨设备工作区")
                        .font(.system(size: 12, weight: .regular))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .fixedSize(horizontal: true, vertical: false)
                }
                Spacer(minLength: 0)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.leading, 20)
            .padding(.trailing, 12)
            .padding(.top, 16)
            .padding(.bottom, 12)
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            SidebarDeviceStatus(store: store)
        }
        .systemSettingsSidebarBackground()
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
        .frame(width: 34, height: 34)
        .modifier(BrandGlow())
        .accessibilityHidden(true)
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
                    Text(versionText)
                        .font(.system(size: 10, weight: .regular))
                        .foregroundStyle(.tertiary)
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
        .systemSettingsBlock()
        .padding(.horizontal, 10)
        .padding(.bottom, 10)
    }

    private var runtimeColor: Color {
        if store.error != nil || store.snapshot?.error != nil { return .orange }
        return store.snapshot == nil || store.starting ? .secondary : .green
    }

    private var versionText: String {
        let info = Bundle.main.infoDictionary ?? [:]
        let version = info["CFBundleShortVersionString"] as? String ?? "—"
        let build = info["CFBundleVersion"] as? String
        return build.map { "v\(version) (\($0))" } ?? "v\(version)"
    }
}
