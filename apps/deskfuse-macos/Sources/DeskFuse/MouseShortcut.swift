import CoreGraphics
import Foundation

/// Keep event planning testable without posting input into the user's session.
struct MouseShortcut: Equatable, Hashable {
    var key: UInt16
    var modifiers: UInt64

    static let modifierKeys: [(UInt64, UInt16)] = [
        (0x40000, 59), (0x80000, 58), (0x20000, 56), (0x100000, 55), (0x800000, 63)
    ]
    var eventFlags: UInt64 {
        // Arrow keys carry both Fn and NumericPad even when Fn isn't held.
        modifiers | ((123...126).contains(key) ? 0xa00000 : 0)
    }
    var systemHotKey: (id: String, name: String)? {
        switch (key, modifiers) {
        case (123, 0x40000): ("79", "向左切换桌面")
        case (124, 0x40000): ("81", "向右切换桌面")
        case (126, 0x40000): ("32", "Mission Control")
        case (125, 0x40000): ("33", "应用程序窗口")
        case (103, 0): ("36", "显示桌面")
        default: nil
        }
    }
    func resolved(in hotKeys: [String: Any]) throws -> Self {
        guard let systemHotKey, let entry = hotKeys[systemHotKey.id] as? [String: Any] else { return self }
        if (entry["enabled"] as? NSNumber)?.boolValue == false {
            throw ShortcutError(message: "系统的“\(systemHotKey.name)”快捷键已关闭，请在系统设置 → 键盘 → 键盘快捷键 → Mission Control 中启用。")
        }
        guard let value = entry["value"] as? [String: Any] else { return self }
        guard value["type"] as? String == "standard",
              let parameters = value["parameters"] as? [NSNumber], parameters.count == 3,
              (0...127).contains(parameters[1].intValue) else {
            throw ShortcutError(message: "系统的“\(systemHotKey.name)”没有可用的键盘快捷键，请先在系统设置中分配。")
        }
        return Self(key: parameters[1].uint16Value, modifiers: parameters[2].uint64Value & 0x9e0000)
    }
    struct ShortcutError: LocalizedError {
        var message: String
        var errorDescription: String? { message }
    }
}

/// Reference-count generated modifiers so simultaneous mouse holds cannot release
/// one another's shortcut. Modifiers physically held before a press aren't released.
struct MouseShortcutState {
    struct Event: Equatable {
        var key: UInt16
        var down: Bool
        var flags: UInt64
    }
    private var holds: [MouseShortcut: Int] = [:]
    private var references: [UInt64: Int] = [:]
    private var generated: UInt64 = 0
    private var initialPhysicalFlags: UInt64 = 0

    mutating func events(for shortcut: MouseShortcut, down: Bool, physicalFlags: UInt64) -> [Event] {
        var events: [Event] = []
        if holds.isEmpty { initialPhysicalFlags = physicalFlags }
        // Fn on a posted arrow is intrinsic, not a physically held Fn key.
        let intrinsicFn = initialPhysicalFlags & 0x800000 == 0
            && (holds.keys.contains { (123...126).contains($0.key) } || (123...126).contains(shortcut.key))
        let ambient = physicalFlags & 0x9e0000 & (intrinsicFn ? ~UInt64(0x800000) : ~UInt64(0))
        if down {
            holds[shortcut, default: 0] += 1
            guard holds[shortcut] == 1 else { return [] }
            var flags = ambient | generated
            for (bit, key) in MouseShortcut.modifierKeys where shortcut.modifiers & bit != 0 {
                references[bit, default: 0] += 1
                if references[bit] == 1 && flags & bit == 0 {
                    generated |= bit; flags |= bit
                    events.append(Event(key: key, down: true, flags: flags))
                }
            }
            events.append(Event(key: shortcut.key, down: true, flags: shortcut.eventFlags))
        } else {
            guard let count = holds[shortcut] else { return [] }
            if count > 1 { holds[shortcut] = count - 1; return [] }
            holds.removeValue(forKey: shortcut)
            events.append(Event(key: shortcut.key, down: false, flags: shortcut.eventFlags))
            var flags = ambient | generated
            for (bit, key) in MouseShortcut.modifierKeys.reversed() where shortcut.modifiers & bit != 0 {
                let count = max(0, (references[bit] ?? 0) - 1)
                references[bit] = count
                if count == 0 && generated & bit != 0 {
                    generated &= ~bit; flags &= ~bit
                    events.append(Event(key: key, down: false, flags: flags))
                }
            }
        }
        return events
    }
}
