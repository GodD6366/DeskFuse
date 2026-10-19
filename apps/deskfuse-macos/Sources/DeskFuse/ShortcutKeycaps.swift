import SwiftUI
import DeskFuseEngine

/// The same visual vocabulary is used by read-only bindings and recorder buttons.
struct ShortcutKeycaps: View {
    let shortcut: Shortcut
    var body: some View {
        HStack(spacing: 4) {
            ForEach(Array(shortcutParts(shortcut).enumerated()), id: \.offset) { _, key in
                Text(verbatim: key)
                    .font(DFType.keycap)
                    .frame(minWidth: 18, minHeight: 24)
                    .padding(.horizontal, 5)
                    .background(.quaternary.opacity(0.55), in: RoundedRectangle(cornerRadius: DFRadius.control))
                    .overlay { RoundedRectangle(cornerRadius: DFRadius.control).strokeBorder(.primary.opacity(0.12)) }
            }
        }
        .fixedSize()
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(shortcutLabel(shortcut))
    }
}

func shortcutParts(_ shortcut: Shortcut) -> [String] {
    var parts: [String] = []
    for (flag, symbol) in [(UInt64(1 << 18), "⌃"), (UInt64(1 << 19), "⌥"), (UInt64(1 << 17), "⇧"), (UInt64(1 << 20), "⌘")] where shortcut.modifiers & flag != 0 {
        parts.append(symbol)
    }
    let keys: [UInt16: String] = [
        123: "←", 124: "→", 125: "↓", 126: "↑", 53: "Esc", 49: "Space", 36: "↩", 48: "Tab", 51: "⌫", 117: "⌦",
        0: "A", 1: "S", 2: "D", 3: "F", 4: "H", 5: "G", 6: "Z", 7: "X", 8: "C", 9: "V", 11: "B", 12: "Q", 13: "W", 14: "E", 15: "R", 16: "Y", 17: "T", 31: "O", 32: "U", 34: "I", 35: "P", 37: "L", 38: "J", 40: "K", 45: "N", 46: "M",
        18: "1", 19: "2", 20: "3", 21: "4", 23: "5", 22: "6", 26: "7", 28: "8", 25: "9", 29: "0",
        24: "=", 27: "−", 30: "]", 33: "[", 39: "'", 41: ";", 42: "\\", 43: ",", 44: "/", 47: ".", 50: "`",
        122: "F1", 120: "F2", 99: "F3", 118: "F4", 96: "F5", 97: "F6", 98: "F7", 100: "F8", 101: "F9", 109: "F10", 103: "F11", 111: "F12"
    ]
    parts.append(keys[shortcut.keyCode] ?? "Key \(shortcut.keyCode)")
    return parts
}
