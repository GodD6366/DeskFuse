import SwiftUI

enum SettingsCategory: String, CaseIterable {
    case general, permissions, storage, clipboard, shortcuts, updates, about

    var title: String {
        switch self {
        case .general: "通用"
        case .permissions: "权限"
        case .storage: "传输与存储"
        case .clipboard: "剪贴板"
        case .shortcuts: "快捷键"
        case .updates: "更新"
        case .about: "关于"
        }
    }

    var symbol: String {
        switch self {
        case .general: "gearshape"
        case .permissions: "shield"
        case .storage: "tray"
        case .clipboard: "clipboard"
        case .shortcuts: "keyboard"
        case .updates: "arrow.clockwise"
        case .about: "info.circle"
        }
    }

    /// 侧栏彩色方块图标的识别色，与主窗口同名功能保持同色。
    var color: Color {
        switch self {
        case .general: .gray
        case .permissions: .red
        case .storage: .teal
        case .clipboard: .blue
        case .shortcuts: .indigo
        case .updates: .green
        case .about: .gray
        }
    }
}
