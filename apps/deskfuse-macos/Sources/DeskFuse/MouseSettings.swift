import Foundation

struct MouseSettings: Codable, Equatable, Sendable {
    var enabled = false
    var bindings: [MouseBinding] = []

    func action(button: Int, modifiers: UInt64) -> MouseAction? {
        guard enabled else { return nil }
        return bindings.first { $0.button == button && $0.modifiers == modifiers }?.action
            ?? bindings.first { $0.button == button && $0.modifiers == 0 }?.action
    }
    mutating func set(button: Int, modifiers: UInt64, action: MouseAction?) {
        bindings.removeAll { $0.button == button && $0.modifiers == modifiers }
        if let action { bindings.append(MouseBinding(button: button, modifiers: modifiers, action: action)) }
        bindings.sort { ($0.button, $0.modifiers) < ($1.button, $1.modifiers) }
    }
    static func buttonName(_ button: Int) -> String {
        switch button {
        case 0: "左键"
        case 1: "右键"
        case 2: "中键 / 滚轮按下"
        case 3: "后退侧键（按钮 4）"
        case 4: "前进侧键（按钮 5）"
        case 195: "拇指手势键（MX Master）"
        case 196: "滚轮模式切换键（MX Master）"
        case 208...223: "G502 实体键 \(button - 207)"
        default: "按钮 \(button + 1)"
        }
    }
}
struct MouseBinding: Codable, Equatable, Sendable {
    var button: Int
    var modifiers: UInt64
    var action: MouseAction
}
struct MouseAction: Codable, Equatable, Sendable {
    var kind: String
    var button: Int?
    var key_code: UInt16?
    var modifiers: UInt64?

    static let disabled = MouseAction(kind: "disabled")
    static func mouse(_ button: Int) -> Self { Self(kind: "mouse", button: button) }
    static func shortcut(_ key: UInt16, _ modifiers: UInt64) -> Self {
        Self(kind: "shortcut", key_code: key, modifiers: modifiers)
    }
}
