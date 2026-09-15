import SwiftUI
import AppKit
import UniDeskEngine

/// Navigation observes selection; runtime refreshes are confined to the footer.
struct MainSidebar: View {
    @Bindable var store: AppStore
    @Binding var visibility: NavigationSplitViewVisibility

    var body: some View {
        List(selection: $store.selectedPage) {
            ForEach(Page.allCases) { page in
                SidebarNavRow(title: page.rawValue, symbol: page.symbol, selected: store.selectedPage == page)
                    .tag(page)
                    .listRowSeparator(.hidden)
                    .listRowBackground(Color.clear)
            }
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .environment(\.defaultMinListRowHeight, 28)
        .safeAreaInset(edge: .top, spacing: 4) {
            HStack(spacing: 8) {
                Image(nsImage: NSApplication.shared.applicationIconImage)
                    .resizable()
                    .renderingMode(.original)
                    .scaledToFit()
                    .frame(width: 28, height: 28)
                    .accessibilityHidden(true)
                Text("uniDesk")
                    .font(.system(size: 16, weight: .bold))
                    .lineLimit(1)
                    .fixedSize()
                Spacer(minLength: 0)
                SidebarVisibilityButton(visibility: $visibility)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 12)
            .padding(.top, 12)
            .padding(.bottom, 4)
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            SidebarDeviceStatus(store: store)
        }
    }
}

private struct SidebarDeviceStatus: View {
    let store: AppStore
    var body: some View {
        HStack(spacing: 8) {
            Button { store.selectedPage = .devices } label: {
                HStack(spacing: 8) {
                    Image(systemName: "laptopcomputer")
                        .font(.system(size: 18, weight: .medium))
                        .symbolRenderingMode(.monochrome)
                        .foregroundStyle(.secondary)
                    VStack(alignment: .leading, spacing: 4) {
                        Text("这台 Mac")
                            .font(.system(size: 13, weight: .medium))
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
            Button { store.openSettings() } label: {
                Image(systemName: "gearshape").font(.system(size: 15, weight: .medium))
                    .frame(width: 24, height: 28)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("设置")
            .help("设置（⌘,）")
        }
        .padding(.horizontal, 12).padding(.vertical, 14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .modifier(NativeGlassSurface(cornerRadius: 14))
        .padding(12)
    }

    private var runtimeColor: Color {
        if store.error != nil || store.snapshot?.error != nil { return .orange }
        return store.snapshot == nil || store.starting ? .secondary : .green
    }
}
