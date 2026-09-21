import SwiftUI
import AppKit
import DeskFuseEngine

struct ShortcutEditor: View {
    @Bindable var store: AppStore
    @Binding var shortcuts: [Shortcut]
    @State private var recording: Int?
    @State private var monitor: Any?
    @State private var error: String?
    var body: some View {
        ForEach(Array(shortcuts.indices), id: \.self) { index in
            HStack { Text(["向左切换", "向右切换", "向上切换", "向下切换", "锁定当前设备"][index]); Spacer(); Button { begin(index) } label: {
                if recording == index { Label("按下组合键…", systemImage: "record.circle").foregroundStyle(Color.accentColor) }
                else { ShortcutKeycaps(shortcut: shortcuts[index]) }
            }.buttonStyle(.borderless).help("更改快捷键") }
        }
        if let error { Text(error).font(.caption).foregroundStyle(.red) }
        Text("紧急收回固定为 ⌃⌥⌘ Esc。").font(.caption).foregroundStyle(.secondary)
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
