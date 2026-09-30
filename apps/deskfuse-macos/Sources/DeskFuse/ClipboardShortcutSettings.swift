import SwiftUI
import DeskFuseEngine

/// Binding is a draft only; the owner persists it using the library settings API.
struct ClipboardShortcutSettings: View {
    @Bindable var store: AppStore
    @Binding var shortcut: WorkflowKey?
    var onRecordingChange: (Bool) -> Void = { _ in }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("唤起剪贴板").font(.headline)
            Text("默认 ⌥⌘V。面板内双击、回车或 ⌘1–9 粘贴回原应用；移除绑定可关闭全局唤起。")
                .font(.caption).foregroundStyle(.secondary)
            WorkflowShortcutRecorder(store: store, shortcut: $shortcut, editingClipboardShortcut: true,
                onRecordingChange: onRecordingChange)
            Button("恢复默认") { shortcut = WorkflowKey(key_code: 9, modifiers: 0x180000) }
        }
    }

    static func conflict(_ shortcut: WorkflowKey?, store: AppStore) -> String? {
        guard let shortcut else { return nil }
        let key = shortcut.key_code, flags = shortcut.modifiers
        if key == 9 && flags == 0x100000 { return "⌘V 保留给普通粘贴。" }
        if key == 53 && flags & 0x1c0000 == 0x1c0000 { return "这组按键用于紧急收回。" }
        if key == 2 && flags == 0x120000 { return "这组按键用于文件暂存区。" }
        if store.snapshot?.input.settings.shortcuts.contains(where: { $0.keyCode == key && $0.modifiers == flags }) == true {
            return "这组按键已用于键鼠控制。"
        }
        if let other = store.workflowDefinitions.first(where: { $0.enabled && $0.shortcut?.key_code == key && $0.shortcut?.modifiers == flags }) {
            return "这组按键已绑定到“\(other.name)”。"
        }
        return nil
    }
}
