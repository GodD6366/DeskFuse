import SwiftUI

/// 通用设置只保留三件与日常使用直接相关的事：开机启动、Dock 图标、保持唤醒。
/// 功能开关都在各自功能页的页头，不再在设置里重复一份。
struct GeneralSettingsView: View {
    @Binding var hideDockIcon: Bool
    @Binding var keepAwake: Bool

    var body: some View {
        DFScrollPage {
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
}
