import SwiftUI

/// 设置分类按「先常用、后进阶」排列；高级里只放少数人需要的开关与诊断。
enum SettingsCategory: String, CaseIterable {
    case general, clipboard, shortcuts, files, permissions, about, advanced

    var title: String {
        switch self {
        case .general: "通用"
        case .clipboard: "剪贴板"
        case .shortcuts: "快捷键"
        case .files: "文件"
        case .permissions: "权限"
        case .about: "关于与更新"
        case .advanced: "高级"
        }
    }

    var symbol: String {
        switch self {
        case .general: "gearshape"
        case .clipboard: "doc.on.clipboard"
        case .shortcuts: "command"
        case .files: "folder"
        case .permissions: "lock.shield"
        case .about: "info.circle"
        case .advanced: "slider.horizontal.3"
        }
    }

    /// 旧版深链接使用的分类名，继续指向合并后的新分类。
    init?(legacy rawValue: String) {
        switch rawValue {
        case "storage": self = .files
        case "updates": self = .about
        default: self.init(rawValue: rawValue)
        }
    }
}
