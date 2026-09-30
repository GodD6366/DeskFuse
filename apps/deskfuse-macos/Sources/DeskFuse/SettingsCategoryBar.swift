import SwiftUI

enum SettingsCategory: String, CaseIterable {
    case general, input, permissions, storage, clipboard, workflows, shortcuts, updates

    var title: String {
        switch self {
        case .general: "通用"
        case .input: "键鼠共享"
        case .permissions: "权限"
        case .storage: "传输与存储"
        case .clipboard: "剪贴板"
        case .workflows: "工作流"
        case .shortcuts: "快捷键"
        case .updates: "更新"
        }
    }

    var symbol: String {
        switch self {
        case .general: "gearshape"
        case .input: "rectangle.on.rectangle"
        case .permissions: "shield"
        case .storage: "tray"
        case .clipboard: "clipboard"
        case .workflows: "person.2"
        case .shortcuts: "keyboard"
        case .updates: "arrow.clockwise"
        }
    }
}

struct SettingsCategoryBar: View {
    @Binding var selection: String
    var body: some View {
        List(selection: $selection) {
            ForEach(SettingsCategory.allCases, id: \.rawValue) { category in
                SidebarNavRow(
                    title: category.title,
                    symbol: category.symbol,
                    selected: selection == category.rawValue
                )
                .tag(category.rawValue)
                .listRowSeparator(.hidden)
                .listRowBackground(Color.clear)
            }
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .environment(\.defaultMinListRowHeight, 28)
        .padding(.top, 12)
        .accessibilityLabel("设置分类")
    }
}
