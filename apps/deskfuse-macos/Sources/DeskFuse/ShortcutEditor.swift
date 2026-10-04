import SwiftUI
import AppKit
import DeskFuseEngine

struct ShortcutEditor: View {
    @Bindable var store: AppStore
    @Binding var shortcuts: [Shortcut]
    static let titles = ["向左切换", "向右切换", "向上切换", "向下切换", "锁定当前设备", "紧急收回"]
    @State private var recording: Int?
    @State private var monitor: Any?
    @State private var error: String?
    var body: some View {
        ForEach(Array(shortcuts.indices), id: \.self) { index in
            HStack { Text(Self.titles[index]); Spacer(); Button { begin(index) } label: {
                if recording == index { Label("按下组合键…", systemImage: "record.circle").foregroundStyle(Color.accentColor) }
                else { ShortcutKeycaps(shortcut: shortcuts[index]) }
            }.buttonStyle(.borderless).help("更改快捷键") }
        }
        if shortcuts.count > 5 {
            Button("恢复默认紧急收回快捷键") {
                stop(); error = nil
                shortcuts[5] = Shortcut(keyCode: 53, modifiers: 0x1c0000)
            }.font(.caption)
        }
        if let error { Text(error).font(.caption).foregroundStyle(.red) }
        Text("紧急收回支持自定义；⌃⌥⌘ Esc 始终保留为保底快捷键，无需找到鼠标。").font(.caption).foregroundStyle(.secondary)
            .onDisappear(perform: stop)
    }
    private func begin(_ index: Int) {
        stop(); ClipboardPanelController.shared.suspendShortcut(true); recording = index; store.engine?.setRecording(enabled: true)
        monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
            let handled = MainActor.assumeIsolated {
                guard store.snapshot?.input.recording == true else { return false }
                let flags = event.modifierFlags.intersection([.shift, .control, .option, .command]).rawValue
                guard flags != 0 else { if event.keyCode == 53 { stop() }; return true }
                if let clipboard = ClipboardPanelController.shared.shortcut,
                   clipboard.key_code == event.keyCode && clipboard.modifiers == UInt64(flags) {
                    error = "这组按键已用于唤起剪贴板，请换一组按键"
                    return true
                }
                if (index != 5 && event.keyCode == 53 && flags == 0x1c0000)
                    || (event.keyCode == 2 && flags == 0x120000) {
                    error = "这组按键是 DeskFuse 保留快捷键，请换一组按键"; return true
                }
                if index == 5 && flags & 0x1c0000 == 0 {
                    error = "紧急收回需包含 Command、Option 或 Control"; return true
                }
                if shortcuts.enumerated().contains(where: { $0.offset != index && $0.element.keyCode == event.keyCode && $0.element.modifiers == UInt64(flags) }) {
                    error = "这组按键已用于其他键鼠操作，请换一组按键"; return true
                }
                error = nil
                shortcuts[index] = Shortcut(keyCode: event.keyCode, modifiers: UInt64(flags))
                stop(); return true
            }
            return handled ? nil : event
        }
    }
    private func stop() { ClipboardPanelController.shared.suspendShortcut(false); if let monitor { NSEvent.removeMonitor(monitor) }; monitor = nil; recording = nil; store.engine?.setRecording(enabled: false) }
}
func shortcutLabel(_ shortcut: Shortcut) -> String {
    shortcutParts(shortcut).joined(separator: " ")
}
