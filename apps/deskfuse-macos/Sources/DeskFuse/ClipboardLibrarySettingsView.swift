import SwiftUI

/// 剪贴板历史设置。唤起面板的快捷键在「设置 → 快捷键」统一管理，这里不再重复。
struct ClipboardLibrarySettingsView: View {
    @Bindable private var library = ClipboardLibraryStore.shared
    @State private var draft = ClipboardLibrarySettings()
    @State private var original: ClipboardLibrarySettings?
    @State private var loading = false
    @State private var saving = false
    @State private var status: String?
    @State private var failure: String?
    @AppStorage(wrappedValue: ClipboardOCRIndexer.defaultAutomaticIndexing,
                 ClipboardOCRIndexer.automaticIndexingKey)
    private var automaticOCR: Bool
    @State private var draftAutomaticOCR = ClipboardOCRIndexer.defaultAutomaticIndexing
    @State private var originalAutomaticOCR = ClipboardOCRIndexer.defaultAutomaticIndexing
    /// 快捷键不在本页编辑，比较时忽略它，避免快捷键页的修改让本页显示“未保存”。
    private var dirty: Bool {
        guard var original else { return false }
        original.shortcut = draft.shortcut
        return draft != original || draftAutomaticOCR != originalAutomaticOCR
    }

    var body: some View {
        VStack(spacing: 0) {
            DFScrollPage {
                DFSection("普通历史") {
                    DFSettingRow(title: "保留天数",
                        detail: "默认保留 30 天，不限制条数。手动列表成员长期保留；智能列表不会延长历史保留期。") {
                        TextField("天数", value: $draft.retention_days, format: .number).frame(width: 80)
                    }
                }
                DFSection("跨设备搜索") {
                    DFSettingRow(title: "同步标题与识别文字到其他设备",
                        detail: "始终开启：只同步标题、识别文字和元数据，方便在其他设备搜索；图片和文件正文按需获取。") { EmptyView() }
                    Divider()
                    DFSettingRow(title: "已同步的远端索引",
                        detail: "清除其他设备同步过来的索引行，不影响本机历史、手动列表或已获取的正文。") {
                        Button("清除远端索引") { library.clearRemoteIndex() }
                    }
                }
                DFSection("图片文字识别") {
                    DFToggleRow("自动识别图片文字",
                        detail: "默认开启，识别在可自动退出的独立进程中运行。关闭后保留已有识别结果；图片预览和复制始终可用。", isOn: $draftAutomaticOCR)
                }
                DFSection("排除 App") {
                    ForEach(draft.excluded_apps) { app in
                        HStack {
                            Text(app.name).textSelection(.enabled)
                            Spacer()
                            Button("移除", role: .destructive) {
                                draft.excluded_apps.removeAll { $0.bundle_id == app.bundle_id }
                            }
                        }
                    }
                    DFSettingRow(title: "不保存历史的应用",
                        detail: "新复制仍正常实时同步，但发送端和接收端不保存历史。不会删除已有历史或手动列表内容；此配置仅属于当前设备。") {
                        Button("添加应用…") {
                            guard let id = ClipboardLibraryAppPicker.choose(),
                                  !draft.excluded_apps.contains(where: { $0.bundle_id == id }) else { return }
                            draft.excluded_apps.append(ClipboardLibraryApp(bundle_id: id,
                                name: library.apps.first(where: { $0.bundle_id == id })?.name ?? id))
                        }
                    }
                }
                Text("本页修改统一在点击底部“保存”后生效。")
                    .font(DFType.caption).foregroundStyle(.secondary)
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
                canSave: original != nil && (dirty || failure != nil),
                canRevert: dirty, status: status, error: failure,
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
            draft = settings; original = settings
            draftAutomaticOCR = automaticOCR; originalAutomaticOCR = automaticOCR
        } catch { failure = error.userFacingMessage }
    }

    private func save() {
        guard !saving, original != nil else { return }
        guard draft.retention_days > 0, draft.retention_days <= Int(UInt32.max) else {
            failure = "请输入有效的保留天数（1–4294967295）。"; return
        }
        var submitted = draft
        let submittedAutomaticOCR = draftAutomaticOCR
        saving = true; failure = nil; status = nil
        Task {
            defer { saving = false }
            do {
                // 快捷键可能已在快捷键页单独修改：沿用引擎中的最新值，避免本页的旧草稿把它覆盖回去。
                submitted.shortcut = try await library.loadSettings().shortcut
                let object = try JSONSerialization.jsonObject(with: JSONEncoder().encode(submitted))
                _ = try await library.mutate(["op": "settings", "settings": object])
                // Preserve the local ordering; the backend normalizes excluded apps.
                original = submitted
                automaticOCR = submittedAutomaticOCR
                originalAutomaticOCR = submittedAutomaticOCR
                if !submittedAutomaticOCR { ClipboardOCRIndexer.shared.cancelPending() }
                status = "已保存"
            } catch { failure = error.userFacingMessage }
        }
    }
}
