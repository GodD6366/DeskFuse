import AppKit
import SwiftUI
import DeskFuseEngine

/// Voice bindings accept a single key, including a side-specific modifier.
/// Workflow/clipboard recorders keep their existing combination-key policy.
struct MicrophoneShortcutKeycaps: View {
    let key: WorkflowKey
    var body: some View {
        if key.modifiers == 0, let name = microphoneModifierName(key.key_code) {
            Text(name).font(.body.monospaced())
        } else {
            ShortcutKeycaps(shortcut: key.native)
        }
    }
}

private func microphoneModifierName(_ code: UInt16) -> String? {
    [54: "右 ⌘", 55: "左 ⌘", 56: "左 ⇧", 60: "右 ⇧",
     58: "左 ⌥", 61: "右 ⌥", 59: "左 ⌃", 62: "右 ⌃", 63: "Fn / 🌐"][code]
}

private func microphoneModifierDown(_ event: NSEvent) -> Bool {
    let masks: [UInt16: (UInt, UInt, UInt)] = [
        54: (1 << 20, 0x10, 0x8), 55: (1 << 20, 0x8, 0x10),
        56: (1 << 17, 0x2, 0x4), 60: (1 << 17, 0x4, 0x2),
        58: (1 << 19, 0x20, 0x40), 61: (1 << 19, 0x40, 0x20),
        59: (1 << 18, 0x1, 0x2000), 62: (1 << 18, 0x2000, 0x1),
        63: (1 << 23, 0, 0)
    ]
    guard let (aggregate, side, opposite) = masks[event.keyCode] else { return false }
    let flags = event.modifierFlags.rawValue
    return flags & aggregate != 0 && (side == 0 || flags & side != 0 || flags & (side | opposite) == 0)
}

struct MicrophoneShortcutRecorder: View {
    let store: AppStore
    @Binding var shortcut: WorkflowKey?
    var onRecordingChange: (Bool) -> Void
    @State private var recording = false
    @State private var monitor: Any?
    @State private var timeout: Task<Void, Never>?
    @State private var message: String?
    @State private var candidate: WorkflowKey?
    @State private var heldModifiers: Set<UInt16> = []
    @State private var modifierCandidate: UInt16?
    @State private var modifierChord = false
    @State private var receivedEvent = false

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Button(recording ? "正在录制…" : "录制按键") { begin() }.disabled(recording)
                if let shortcut { MicrophoneShortcutKeycaps(key: shortcut) }
                Spacer()
                if recording { Button("取消录制") { stop(); message = nil } }
                else { Button("使用 Fn / 🌐") { shortcut = WorkflowKey(key_code: 63, modifiers: 0); message = nil } }
            }
            Text(recording ? "按下并松开一个键，或按下组合键。点击“取消录制”可保留原绑定。" : "支持单键（如 F6、右 ⌥）、Fn / 🌐，也支持组合键。")
                .font(.callout).foregroundStyle(.secondary)
            if let message { Text(message).font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true) }
            Text("右侧 Fn 若未被系统上报，无法录制或跨设备转发；可在这里录制其它单键，并在输入法中设置同一按键。")
                .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
        }
        .onDisappear(perform: stop)
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didResignActiveNotification)) { _ in stop() }
    }

    private func begin() {
        guard store.engine != nil else { message = "引擎尚未就绪，请稍后重试"; return }
        stop()
        candidate = nil; heldModifiers = []; modifierCandidate = nil; modifierChord = false; receivedEvent = false
        recording = true; message = nil; onRecordingChange(true)
        ClipboardPanelController.shared.suspendShortcut(true)
        store.engine?.setRecording(enabled: true)
        monitor = NSEvent.addLocalMonitorForEvents(matching: [.keyDown, .keyUp, .flagsChanged]) { event in
            MainActor.assumeIsolated { consume(event) }
            return nil
        }
        timeout = Task { @MainActor in
            do { try await Task.sleep(for: .seconds(5)) } catch { return }
            if recording && !receivedEvent { message = "尚未收到按键事件。请试按 F6 或右 ⌥；如果只有右 Fn 没有反应，它可能由键盘内部处理。" }
        }
    }

    private func consume(_ event: NSEvent) {
        receivedEvent = true
        if event.keyCode > 127 || event.keyCode == 57 {
            message = "这个按键不支持按住／松开录制，请换一个键（如 F6 或右 ⌥）。"
            return
        }
        let flags = UInt64(event.modifierFlags.intersection([.shift, .control, .option, .command]).rawValue)
        if event.type == .flagsChanged {
            guard microphoneModifierName(event.keyCode) != nil else { return }
            if microphoneModifierDown(event) {
                if heldModifiers.isEmpty && candidate == nil { modifierCandidate = event.keyCode; modifierChord = false }
                else if !heldModifiers.contains(event.keyCode) { modifierChord = true }
                heldModifiers.insert(event.keyCode)
                if candidate == nil { message = "已检测到 \(microphoneModifierName(event.keyCode) ?? "修饰键")，松开可绑定单键，也可继续按其它键组成组合键。" }
            } else {
                let wasHeld = heldModifiers.remove(event.keyCode) != nil
                if wasHeld && candidate == nil && !modifierChord && modifierCandidate == event.keyCode {
                    accept(WorkflowKey(key_code: event.keyCode, modifiers: 0))
                }
                if heldModifiers.isEmpty { modifierCandidate = nil; modifierChord = false }
            }
        } else if event.type == .keyDown && !event.isARepeat {
            guard candidate == nil else { return }
            let key = WorkflowKey(key_code: event.keyCode, modifiers: flags)
            if let conflict = conflict(key) { message = conflict; modifierChord = true; return }
            candidate = key
            message = "已检测到 \(shortcutLabel(key.native))，松开该键完成录制。"
        } else if event.type == .keyUp, let candidate, candidate.key_code == event.keyCode {
            accept(candidate)
        }
    }

    private func accept(_ key: WorkflowKey) {
        if let conflict = conflict(key) { message = conflict; candidate = nil; return }
        shortcut = key
        stop()
        message = "已识别按键，保存后生效。"
    }
    private func conflict(_ key: WorkflowKey) -> String? {
        if key.key_code == 53 && key.modifiers & 0x1c0000 == 0x1c0000 { return "此组合用于紧急收回，请换一个按键。" }
        if key.key_code == 9 && key.modifiers == 0x100000 { return "⌘ V 保留给普通粘贴。" }
        if key.key_code == 2 && key.modifiers == 0x120000 { return "此组合已用于文件暂存区。" }
        if let clipboard = ClipboardPanelController.shared.shortcut, clipboard.key_code == key.key_code && clipboard.modifiers == key.modifiers { return "此按键已用于唤起剪贴板。" }
        if store.snapshot?.input.settings.shortcuts.contains(where: { $0.keyCode == key.key_code && $0.modifiers == key.modifiers }) == true { return "此按键已用于键鼠控制。" }
        if let flow = store.workflowDefinitions.first(where: { $0.enabled && $0.shortcut == key }) { return "此按键已用于工作流“\(flow.name)”。" }
        return nil
    }
    private func stop() {
        timeout?.cancel(); timeout = nil
        if let monitor { NSEvent.removeMonitor(monitor) }
        monitor = nil
        if recording {
            recording = false
            ClipboardPanelController.shared.suspendShortcut(false)
            store.engine?.setRecording(enabled: false)
            onRecordingChange(false)
        }
    }
}
