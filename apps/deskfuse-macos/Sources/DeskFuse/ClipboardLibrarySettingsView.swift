import SwiftUI

struct ClipboardLibrarySettingsView: View {
    @Bindable private var library = ClipboardLibraryStore.shared
    @State private var draft = ClipboardLibrarySettings()
    @State private var original: ClipboardLibrarySettings?
    @State private var loading = false
    @State private var saving = false
    @State private var recording = false
    @State private var status: String?
    @State private var failure: String?
    @AppStorage(wrappedValue: ClipboardOCRIndexer.defaultAutomaticIndexing,
                 ClipboardOCRIndexer.automaticIndexingKey)
    private var automaticOCR: Bool
    @State private var draftAutomaticOCR = ClipboardOCRIndexer.defaultAutomaticIndexing
    @State private var originalAutomaticOCR = ClipboardOCRIndexer.defaultAutomaticIndexing
    private var dirty: Bool { original != nil && (draft != original || draftAutomaticOCR != originalAutomaticOCR) }
    private var shortcut: WorkflowKey? {
        let value = draft.shortcut ?? ClipboardLibraryShortcut()
        return value.enabled ? WorkflowKey(key_code: value.key_code, modifiers: value.modifiers) : nil
    }

    var body: some View {
        VStack(spacing: 0) {
            SystemSettingsFormPage {
                SystemSettingsFormBlock(title: "快捷键") {
                    ClipboardShortcutSettings(store: AppStore.shared, shortcut: Binding(
                        get: { shortcut },
                        set: { key in
                            draft.shortcut = ClipboardLibraryShortcut(enabled: key != nil,
                                key_code: key?.key_code ?? 9, modifiers: key?.modifiers ?? 0x180000)
                        }), onRecordingChange: { recording = $0 })
                }
                SystemSettingsFormBlock(title: "普通历史") {
                    HStack {
                        Text("保留天数")
                        TextField("天数", value: $draft.retention_days, format: .number).frame(width: 80)
                    }
                    Text("默认保留 30 天，不限制条数。手动列表成员长期保留；智能列表不会延长历史保留期。")
                        .font(.caption).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                SystemSettingsFormBlock(title: "跨设备搜索") {
                    Toggle("同步标题与识别文字到其他设备", isOn: Binding(
                        get: { draft.sync_index ?? true },
                        set: { draft.sync_index = $0 }))
                    Text("只同步标题、识别文字和元数据这类小体积索引，方便在其它设备搜索；图片/文件正文仍在需要时按需获取。关闭后不再发布也不再接收远端索引。")
                        .font(.caption).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                    Button("清除已同步的远端索引") { library.clearRemoteIndex() }
                    Text("清除其它设备同步过来的索引行，不影响本机历史、手动列表或已获取的正文。")
                        .font(.caption).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                SystemSettingsFormBlock(title: "图片文字识别") {
                    Toggle("自动识别图片文字", isOn: $draftAutomaticOCR)
                    Text("默认开启，识别在可自动退出的独立进程中运行。关闭后保留已有识别结果；图片预览和复制始终可用。")
                        .font(.caption).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                SystemSettingsFormBlock(title: "排除 App") {
                    ForEach(draft.excluded_apps) { app in
                        HStack {
                            Text(app.name).textSelection(.enabled)
                            Spacer()
                            Button("移除", role: .destructive) {
                                draft.excluded_apps.removeAll { $0.bundle_id == app.bundle_id }
                            }
                        }
                    }
                    Button("添加应用…") {
                        guard let id = ClipboardLibraryAppPicker.choose(),
                              !draft.excluded_apps.contains(where: { $0.bundle_id == id }) else { return }
                        draft.excluded_apps.append(ClipboardLibraryApp(bundle_id: id,
                            name: library.apps.first(where: { $0.bundle_id == id })?.name ?? id))
                    }
                    Text("新复制仍正常实时同步，但发送端和接收端不保存历史。不会删除已有历史或手动列表内容；此配置仅属于当前设备。")
                        .font(.caption).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                Text("本页修改统一在点击底部“保存”后生效。")
                    .font(.caption).foregroundStyle(.secondary)
                    .padding(.horizontal, 4)
            }
            .disabled(original == nil || saving)
            if original == nil {
                HStack {
                    if loading { ProgressView().controlSize(.small); Text("正在读取设置…") }
                    else { Button("重新读取设置") { Task { await load() } } }
                }.padding(12)
            }
            SettingsSaveFooter(dirty: dirty, saving: saving,
                canSave: original != nil && !recording && (dirty || failure != nil),
                canRevert: dirty && !recording, status: status, error: failure,
                revert: {
                    if let original { draft = original }
                    draftAutomaticOCR = originalAutomaticOCR
                    failure = nil; status = nil
                },
                save: save)
        }
        .task { if original == nil { await load() } }
        .onChange(of: draft) { _, _ in status = nil; failure = nil }
        .onChange(of: draftAutomaticOCR) { _, _ in status = nil; failure = nil }
    }

    private func load() async {
        loading = true; failure = nil
        defer { loading = false }
        do {
            var settings = try await library.loadSettings()
            settings.shortcut = settings.shortcut ?? ClipboardLibraryShortcut()
            settings.sync_index = settings.sync_index ?? true
            draft = settings; original = settings
            draftAutomaticOCR = automaticOCR; originalAutomaticOCR = automaticOCR
        } catch { failure = error.userFacingMessage }
    }

    private func save() {
        guard !saving, !recording, original != nil else { return }
        guard draft.retention_days > 0, draft.retention_days <= Int(UInt32.max) else {
            failure = "请输入有效的保留天数（1–4294967295）。"; return
        }
        if let conflict = ClipboardShortcutSettings.conflict(shortcut, store: AppStore.shared) {
            failure = conflict; return
        }
        let submitted = draft
        let submittedAutomaticOCR = draftAutomaticOCR
        saving = true; failure = nil; status = nil
        Task {
            defer { saving = false }
            do {
                let object = try JSONSerialization.jsonObject(with: JSONEncoder().encode(submitted))
                var saved = try await library.mutate(["op": "settings", "settings": object])
                saved.shortcut = saved.shortcut ?? ClipboardLibraryShortcut()
                // Preserve the local ordering; the backend normalizes excluded apps.
                original = submitted
                automaticOCR = submittedAutomaticOCR
                originalAutomaticOCR = submittedAutomaticOCR
                if !submittedAutomaticOCR { ClipboardOCRIndexer.shared.cancelPending() }
                let value = saved.shortcut ?? ClipboardLibraryShortcut()
                ClipboardPanelController.shared.configure(value.enabled
                    ? WorkflowKey(key_code: value.key_code, modifiers: value.modifiers) : nil)
                if let error = ClipboardPanelController.shared.error {
                    failure = "设置已保存，但\(error)"
                } else {
                    status = value.enabled ? "已保存，快捷键已生效" : "已保存，快捷键已关闭"
                }
            } catch { failure = error.userFacingMessage }
        }
    }
}
