import SwiftUI
import DeskFuseEngine

/// Full-page keyboard/mouse sharing editor. The page replaces the old modal
/// sheet, so the same controls live in 设置 → 键鼠共享 and are one click away
/// from the 鼠标共享 header. 跨屏方式、快捷键、布局与 Wi-Fi 主控选择暂不
/// 暴露设置项：底层默认值与快捷键注册保持生效，主控按 Wi-Fi 的记忆
/// 由引擎静默完成。
struct InputSharingSettingsView: View {
    @Bindable var store: AppStore
    @State private var draft: InputSettings
    @State private var saved: InputSettings
    @State private var saving = false
    @State private var status: String?
    @State private var failure: String?

    init(store: AppStore, initial: InputSettings) {
        self.store = store
        _draft = State(initialValue: initial)
        _saved = State(initialValue: initial)
    }

    private var snapshot: EngineSnapshot? { store.snapshot }
    private var dirty: Bool { draft != saved }

    var body: some View {
        VStack(spacing: 0) {
            SystemSettingsFormPage {
                controllerSection.disabled(snapshot?.input.managed == true)
            }
            SettingsSaveFooter(dirty: dirty, saving: saving, canSave: dirty && snapshot?.input.managed != true, canRevert: dirty,
                status: status, error: failure,
                revert: { draft = saved; failure = nil; status = nil }, save: save)
        }
        .onChange(of: snapshot?.input.settings, initial: true) { _, current in
            guard let current else { return }
            let hadUnsavedChanges = dirty
            saved = current
            if !hadUnsavedChanges || snapshot?.input.managed == true { draft = current }
        }
    }

    // MARK: - 主控

    private var controllerSection: some View {
        SystemSettingsFormBlock(title: "主控") {
            Toggle("此设备作为主控", isOn: Binding(get: { draft.controller }, set: setController))
            Toggle("共享键鼠", isOn: Binding(get: { draft.enabled }, set: setSharingEnabled))
                .disabled(!draft.controller)
            Text("作为主控时，可以在「鼠标共享」页面添加设备屏幕并分享键鼠。当前 Wi-Fi 会自动记住生效的主控：切换主控或关闭主控也会自动更新保存的记忆；回到该 Wi-Fi 时自动恢复主控与屏幕排布，未匹配的网络自动暂停，其余设置保持保存。")
                .font(.caption).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    // MARK: - Actions

    // 关闭主控也停止主动共享；被控状态由对端激活管理。
    private func setController(_ enabled: Bool) {
        draft.controller = enabled
        if !enabled { draft.enabled = false }
    }

    private func setSharingEnabled(_ enabled: Bool) {
        draft.enabled = enabled
        if enabled { draft.controller = true }
    }

    private func save() {
        guard !saving, snapshot?.input.managed != true else { return }
        var submitted = draft
        // Display arrangement belongs to the layout page; keep what is live so
        // a stale form can never rewind a drag that happened meanwhile.
        if let live = snapshot?.input.settings.placements { submitted.placements = live }
        saving = true
        failure = nil
        status = nil
        store.error = nil
        store.send(.setInput(settings: submitted), applied: {
            draft = submitted
            saved = submitted
            saving = false
            status = "已保存"
        }, failed: { message in
            saving = false
            failure = message
        })
    }
}
