import SwiftUI
import AppKit
import DeskFuseEngine

struct WorkflowShortcutRecorder: View {
    @Bindable var store: AppStore
    @Binding var shortcut: WorkflowKey?
    var workflowID: String? = nil
    var editingClipboardShortcut = false
    var onRecordingChange: (Bool) -> Void = { _ in }
    @State private var recording = false
    @State private var monitor: Any?
    @State private var message: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text("全局快捷键").font(DFType.section)
                Spacer()
                if recording {
                    Button("取消录制") { stop(); message = nil }
                } else if shortcut != nil {
                    Button("移除绑定", role: .destructive) { shortcut = nil; message = nil }
                        .buttonStyle(.borderless)
                }
            }
            Button(action: begin) {
                HStack(spacing: 12) {
                    Image(systemName: recording ? "record.circle" : "keyboard")
                        .font(DFType.icon(.medium)).foregroundStyle(recording ? DFPalette.accent : .secondary)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(recording ? "现在按下组合键" : shortcut == nil ? "点击录制快捷键" : "已设置快捷键")
                            .font(DFType.bodyMedium)
                        Text(recording ? "按 Esc 取消，原绑定保持不变" : shortcut == nil ? "例如 ⌃ ⌥ V，运行整个工作流" : "点击此处重新录制")
                            .font(DFType.caption).foregroundStyle(.secondary)
                    }
                    Spacer(minLength: 8)
                    if !recording, let shortcut { ShortcutKeycaps(shortcut: shortcut.native) }
                }
                .padding(14)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(recording ? DFPalette.accent.opacity(0.08) : Color.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: DFRadius.control))
                .overlay { RoundedRectangle(cornerRadius: DFRadius.control).strokeBorder(recording ? DFPalette.accent : Color.primary.opacity(0.12), lineWidth: recording ? 2 : 1) }
                .contentShape(RoundedRectangle(cornerRadius: DFRadius.control))
            }
            .buttonStyle(.plain)
            .accessibilityLabel(recording ? "正在录制快捷键，按 Esc 取消" : "录制全局快捷键")
            if let message {
                Label(message, systemImage: "exclamationmark.circle")
                    .foregroundStyle(DFPalette.danger).font(DFType.caption).fixedSize(horizontal: false, vertical: true)
            } else {
                Text("包含 ⌘、⌥ 或 ⌃ 中至少一个修饰键。绑定在保存后生效。")
                    .foregroundStyle(.secondary).font(DFType.caption).fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(.vertical, 4)
        .onDisappear(perform: stop)
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didResignActiveNotification)) { _ in stop() }
    }
    private func begin() {
        guard store.engine != nil else { message = "引擎尚未就绪，请稍后重试"; return }
        stop(); ClipboardPanelController.shared.suspendShortcut(true); recording = true; message = nil; onRecordingChange(true)
        store.engine?.setRecording(enabled: true)
        monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
            MainActor.assumeIsolated {
                let flags = UInt64(event.modifierFlags.intersection([.shift, .control, .option, .command]).rawValue)
                if event.keyCode == 53 && flags == 0 { stop(); message = nil; return }
                guard flags & 0x1c0000 != 0 else { message = "请同时按住 ⌘、⌥ 或 ⌃"; return }
                if let conflict = conflict(key: event.keyCode, flags: flags) { message = conflict; return }
                shortcut = WorkflowKey(key_code: event.keyCode, modifiers: flags)
                message = nil; stop()
            }
            return nil
        }
    }
    private func conflict(key: UInt16, flags: UInt64) -> String? {
        if !editingClipboardShortcut, let clipboard = ClipboardPanelController.shared.shortcut,
           clipboard.key_code == key && clipboard.modifiers == flags {
            return "这组按键已用于唤起剪贴板，请换一组按键"
        }
        if key == 9 && flags == 0x100000 { return "⌘ V 保留给普通粘贴，请换一组按键" }
        if key == 53 && flags & 0x1c0000 == 0x1c0000 { return "这组按键用于紧急收回，请换一组按键" }
        if key == 2 && flags == 0x120000 { return "这组按键用于文件暂存区，请换一组按键" }
        if store.snapshot?.input.settings.shortcuts.contains(where: { $0.keyCode == key && $0.modifiers == flags }) == true {
            return "这组按键已用于键鼠控制，请换一组按键"
        }
        if let other = store.workflowDefinitions.first(where: { $0.id != workflowID && $0.enabled && $0.shortcut?.key_code == key && $0.shortcut?.modifiers == flags }) {
            return "已绑定到“\(other.name)”，请换一组按键"
        }
        return nil
    }
    private func stop() {
        ClipboardPanelController.shared.suspendShortcut(false)
        if let monitor { NSEvent.removeMonitor(monitor) }
        monitor = nil
        if recording { recording = false; store.engine?.setRecording(enabled: false); onRecordingChange(false) }
    }
}

struct WorkflowShortcutSettings: View {
    @Bindable var store: AppStore
    let manageWorkflows: () -> Void
    @State private var editing: WorkflowDefinition?
    var body: some View {
        DFSection(
            "工作流",
            symbol: "photo.on.rectangle",
            footer: "点击右侧组合键即可修改。停用的流程不会响应快捷键；全局触发需要辅助功能权限。"
        ) {
            ForEach(store.workflowDefinitions) { flow in
                HStack(spacing: 12) {
                    Image(systemName: "square.stack.3d.up").foregroundStyle(.secondary).frame(width: 24)
                    VStack(alignment: .leading, spacing: 4) {
                        HStack(spacing: 8) {
                            Text(flow.name).fontWeight(.medium).lineLimit(1)
                            if !flow.enabled { Text("已停用").font(DFType.caption).foregroundStyle(.secondary) }
                        }
                        Text(flow.steps.map { $0.kind.title }.joined(separator: " → "))
                            .font(DFType.caption).foregroundStyle(.secondary).lineLimit(2)
                    }
                    Spacer(minLength: 12)
                    Button { editing = flow } label: {
                        if let shortcut = flow.shortcut { ShortcutKeycaps(shortcut: shortcut.native) }
                        else { Label("设置快捷键", systemImage: "plus").font(DFType.detail) }
                    }.buttonStyle(.borderless).help("编辑“\(flow.name)”的快捷键")
                }.padding(.vertical, 5)
            }
            if store.workflowDefinitions.isEmpty {
                VStack(alignment: .leading, spacing: 6) {
                    Text("还没有工作流").fontWeight(.medium)
                    Text("先组合压缩、转格式和粘贴，再为整个流程设置快捷键。")
                        .font(DFType.detail).foregroundStyle(.secondary)
                }.padding(.vertical, 6)
            }
            Button("管理工作流…", action: manageWorkflows)
        }
        .sheet(item: $editing) { flow in WorkflowShortcutSheet(store: store, initial: flow) { editing = nil } }
    }
}
private struct WorkflowShortcutSheet: View {
    @Bindable var store: AppStore
    @State var initial: WorkflowDefinition
    let close: () -> Void
    @State private var recording = false
    @State private var saving = false
    @State private var failure: String?
    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            VStack(alignment: .leading, spacing: 6) {
                Text("设置快捷键").font(DFType.pageTitle.weight(.bold))
                Text(initial.name).font(DFType.detail).foregroundStyle(.secondary).lineLimit(2)
            }
            WorkflowShortcutRecorder(store: store, shortcut: $initial.shortcut,
                workflowID: initial.id, onRecordingChange: { recording = $0 })
                .disabled(saving)
            if !initial.enabled { Label("流程已停用；保存绑定不会自动启用流程。", systemImage: "pause.circle").font(DFType.caption).foregroundStyle(.secondary) }
            if let failure { Label(failure, systemImage: "exclamationmark.circle").font(DFType.caption).foregroundStyle(DFPalette.danger).fixedSize(horizontal: false, vertical: true) }
            Divider()
            HStack {
                Spacer()
                Button("取消", action: close).disabled(saving)
                Button(saving ? "保存中…" : "保存绑定") {
                    saving = true; failure = nil
                    Task {
                        do { try await WorkflowStore.shared.save(initial); close() }
                        catch { failure = error.userFacingMessage; saving = false }
                    }
                }.buttonStyle(.dfPrimary).disabled(recording || saving)
            }
        }.padding(24).frame(width: 500)
        .interactiveDismissDisabled(saving)
    }
}

/// 工作流默认关闭：关闭时不渲染任何入口，也不注册全局快捷键。
struct WorkflowMenu: View {
    @Bindable var store: AppStore
    let historyID: UInt64
    var contentType: WorkflowContent = .any
    private var eligible: [WorkflowDefinition] {
        store.workflowDefinitions.filter { $0.enabled && $0.accepts(contentType) }
    }
    var body: some View {
        if store.workflowsEnabled {
            Menu("使用工作流…", systemImage: "square.stack.3d.up") {
                ForEach(eligible) { flow in
                    Button(flow.name) { store.runWorkflow(flow.id, historyID: historyID) }
                }
                if eligible.isEmpty { Text("没有适合此内容的已启用工作流") }
            }
        }
    }
}

/// Share menus also serve the file shelf; each target starts from the original input.
/// 统一的「发送到设备」菜单：直接发送；工作流启用后才追加处理选项。
struct WorkflowShareMenu: View {
    @Bindable var store: AppStore
    var historyID: UInt64? = nil
    var paths: [String] = []
    var contentType: WorkflowContent = .any
    private var peers: [Device] { (store.snapshot?.devices ?? []).filter { $0.trusted && !$0.blocked } }
    private var eligible: [WorkflowDefinition] {
        store.workflowDefinitions.filter { $0.enabled && $0.accepts(contentType) }
    }
    var body: some View {
        Menu("发送到设备", systemImage: "paperplane") {
            ForEach(peers, id: \.id) { peer in
                Menu(peer.name) {
                    Button("直接发送") {
                        store.shareWorkflow(nil, historyID: historyID, paths: paths, peerID: peer.id, bypass: true)
                    }
                    if store.workflowsEnabled {
                        let flows = eligible
                        if !flows.isEmpty {
                            Divider()
                            ForEach(flows) { flow in
                                Button("使用「\(flow.name)」发送") {
                                    store.shareWorkflow(flow.id, historyID: historyID, paths: paths, peerID: peer.id)
                                }
                            }
                        }
                    }
                }.disabled(!peer.connected)
            }
            if peers.isEmpty { Text("请先配对一台设备") }
        }
    }
}
