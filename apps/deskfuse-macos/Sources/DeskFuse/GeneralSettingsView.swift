import SwiftUI

/// 通用设置：功能开关、开机启动、Dock 图标、保持唤醒。
/// 功能在各自页面的引导页里开启，关闭统一放在这里，避免页面里误触关掉正在用的功能。
struct GeneralSettingsView: View {
    let store: AppStore
    @Binding var hideDockIcon: Bool
    @Binding var keepAwake: Bool
    @State private var saving: Set<Page> = []

    var body: some View {
        DFScrollPage {
            DFSection("功能", footer: "关闭后停止相关后台任务；在功能页可随时重新启用。") {
                let pages = (Page.primary + Page.extended).filter { store.featureEnabled(for: $0) != nil }
                ForEach(pages) { page in
                    if page != pages.first { Divider() }
                    featureRow(page)
                }
            }
            DFSection("启动") {
                LoginItemSettingsView()
                Divider()
                DFToggleRow("隐藏 Dock 图标",
                    detail: "隐藏后只保留菜单栏图标；仍可从菜单栏打开主窗口和设置。",
                    isOn: $hideDockIcon)
            }
            DFSection("保持可用") {
                DFToggleRow("保持唤醒（避免作为被控端时休眠离线）",
                    detail: "阻止系统和显示器自动休眠，退出 DeskFuse 后自动恢复。",
                    isOn: $keepAwake)
            }
        }
    }

    private func featureRow(_ page: Page) -> some View {
        DFToggleRow(page.rawValue, symbol: page.symbol, badge: page.isBeta ? "Beta" : nil,
                    isOn: Binding(
                        get: { store.featureEnabled(for: page) ?? false },
                        set: { value in
                            saving.insert(page)
                            store.setFeatureEnabled(value, for: page) { saving.remove(page) }
                        }))
            .disabled(saving.contains(page) || store.client == nil
                      || (page == .microphone && MicrophoneSharing.shared.saving)
                      || (page == .mouse && MouseButtonController.shared.saving))
    }
}
