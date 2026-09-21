import SwiftUI
import AppKit
import DeskFuseEngine

/// Navigation observes selection; runtime refreshes are confined to the footer.
struct MainSidebar: View {
    @Bindable var store: AppStore
    @Binding var visibility: NavigationSplitViewVisibility

    var body: some View {
        List(selection: $store.selectedPage) {
            sidebarSectionHeader("工作区")
                .listRowSeparator(.hidden, edges: .all)
                .listRowBackground(Color.clear)
            ForEach([Page.clipboard, .input]) { page in
                SidebarNavRow(title: page.rawValue, symbol: page.symbol, selected: store.selectedPage == page)
                    .tag(page)
                    .listRowSeparator(.hidden, edges: .all)
                    .listRowBackground(Color.clear)
            }
            sidebarSectionHeader("连接")
                .listRowSeparator(.hidden, edges: .all)
                .listRowBackground(Color.clear)
            ForEach([Page.devices, .lanTransfer]) { page in
                SidebarNavRow(title: page.rawValue, symbol: page.symbol, selected: store.selectedPage == page)
                    .tag(page)
                    .listRowSeparator(.hidden, edges: .all)
                    .listRowBackground(Color.clear)
            }
        }
        .listStyle(.plain)
        .listSectionSeparator(.hidden, edges: .all)
        .scrollContentBackground(.hidden)
        .environment(\.defaultMinListRowHeight, 44)
        .background(Color(nsColor: .windowBackgroundColor))
        .safeAreaInset(edge: .top, spacing: 0) {
            HStack(alignment: .top, spacing: 10) {
                VStack(alignment: .leading, spacing: 6) {
                    HStack(spacing: 8) {
                        DeskFuseBrandMark()
                        Text("DeskFuse")
                            .font(.system(size: 24, weight: .semibold))
                            .lineLimit(1)
                            .fixedSize()
                    }
                    Text("在所有设备间，自由流转")
                        .font(.system(size: 13, weight: .regular))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .fixedSize(horizontal: true, vertical: false)
                }
                Spacer(minLength: 0)
                SidebarVisibilityButton(visibility: $visibility)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            // The brand lockup sits farther in than the navigation rows in the
            // reference. This keeps the mark from touching the window edge
            // while leaving the selected navigation pill aligned to the grid.
            .padding(.leading, 28)
            .padding(.trailing, 12)
            .padding(.top, 16)
            .padding(.bottom, 14)
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            VStack(spacing: 8) {
                if let update = store.snapshot?.update, update.showsQuickUpdate {
                    SidebarUpdateButton(store: store, update: update)
                }
                SidebarDeviceStatus(store: store)
            }
        }
    }

    private func sidebarSectionHeader(_ title: String) -> some View {
        Text(title)
            .font(.system(size: 12, weight: .regular))
            .foregroundStyle(.secondary)
            .textCase(nil)
            .padding(.top, 13)
            .padding(.bottom, 7)
    }
}

private struct DeskFuseBrandMark: View {
    private var image: NSImage? {
        Bundle.main.url(forResource: "deskfuse-status-icon", withExtension: "png")
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
        .foregroundStyle(.primary)
        .accessibilityHidden(true)
    }
}

private struct SidebarUpdateButton: View {
    let store: AppStore
    let update: UpdateState

    var body: some View {
        WorkspaceChromeButton(title, systemImage: "arrow.down.app", help: update.message, prominent: true) {
            store.send(.downloadAndInstallUpdate)
        }
        .disabled(update.manualRequested || (!update.available && !update.ready))
        .padding(.horizontal, 12)
    }

    private var title: String {
        if update.manualRequested { return update.ready ? "正在准备安装…" : "正在下载更新…" }
        if update.ready { return "立即安装并重启" }
        if let target = update.targetVersion { return "更新到 \(target)" }
        return "立即更新"
    }
}

private struct SidebarDeviceStatus: View {
    let store: AppStore
    var body: some View {
        HStack(spacing: 8) {
            Button { store.showLocalDevice() } label: {
                HStack(spacing: 8) {
                    Image(systemName: "laptopcomputer")
                        .font(.system(size: 23, weight: .regular))
                        .foregroundStyle(.secondary)
                        .frame(width: 34, height: 34)
                    VStack(alignment: .leading, spacing: 4) {
                        Text("这台 Mac")
                            .font(.system(size: 15, weight: .medium))
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
            .help("在设备页查看本机：\(store.snapshot?.localName ?? "这台 Mac")")
            Button { store.openSettings() } label: {
                Image(systemName: "gearshape").font(.system(size: 15, weight: .medium))
                    .frame(width: 24, height: 28)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("设置")
            .help("设置（⌘,）")
        }
        .padding(.horizontal, 14).padding(.vertical, 14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color(nsColor: .controlBackgroundColor).opacity(0.68), in: RoundedRectangle(cornerRadius: 15, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 15, style: .continuous)
                .strokeBorder(Color(nsColor: .separatorColor).opacity(0.42))
        }
        .padding(.horizontal, 12)
        .padding(.bottom, 12)
    }

    private var runtimeColor: Color {
        if store.error != nil || store.snapshot?.error != nil { return .orange }
        return store.snapshot == nil || store.starting ? .secondary : .green
    }
}
