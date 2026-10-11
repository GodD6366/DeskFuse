import SwiftUI
import DeskFuseEngine

/// Full-page keyboard/mouse sharing editor. The page replaces the old modal
/// sheet, so the same controls live in 设置 → 键鼠共享 and are one click away
/// from the 鼠标共享 header.
struct InputSharingSettingsView: View {
    @Bindable var store: AppStore
    @State private var draft: InputSettings
    @State private var saved: InputSettings
    @State private var saving = false
    @State private var status: String?
    @State private var failure: String?
    @State private var remember = true
    @State private var selectedController = ""

    init(store: AppStore, initial: InputSettings) {
        self.store = store
        _draft = State(initialValue: initial)
        _saved = State(initialValue: initial)
    }

    private var snapshot: EngineSnapshot? { store.snapshot }
    private var dirty: Bool { draft != saved }

    var body: some View {
        VStack(spacing: 0) {
            Form {
                controllerSection.disabled(snapshot?.input.managed == true)
                switchSection.disabled(snapshot?.input.managed == true)
                shortcutSection.disabled(snapshot?.input.managed == true)
                if snapshot?.input.canConfigure == true {
                    layoutSection
                }
                if snapshot?.input.managed != true {
                    environmentSection
                }
            }
            .formStyle(.grouped)
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
        Section("主控") {
            Toggle("此设备作为主控", isOn: Binding(get: { draft.controller }, set: setController))
            Toggle("共享键鼠", isOn: Binding(get: { draft.enabled }, set: setSharingEnabled))
                .disabled(!draft.controller)
            Text("作为主控时，可以在「鼠标共享」页面添加设备屏幕并分享键鼠。")
                .font(.caption).foregroundStyle(.secondary)
        }
    }

    private var switchSection: some View {
        Section("跨屏方式") {
            Picker("触发方式", selection: $draft.switchMode) {
                Text("立即穿越").tag(SwitchMode.instant)
                Text("停留 250 毫秒").tag(SwitchMode.dwell)
                Text("双触边缘").tag(SwitchMode.doubleTap)
            }
            ForEach(0..<4) { index in
                Toggle(["禁用左上角", "禁用右上角", "禁用左下角", "禁用右下角"][index],
                       isOn: cornerBinding(index))
            }
            Stepper("边角范围：\(draft.cornerSize) pt", value: $draft.cornerSize, in: 0...256)
        }
    }

    private var shortcutSection: some View {
        Section("快捷键") {
            ShortcutEditor(store: store, shortcuts: $draft.shortcuts)
        }
    }

    // MARK: - 布局与 Wi-Fi

    private var layoutSection: some View {
        Section("布局") {
            if let ssid = snapshot?.input.ssid {
                LabeledContent("当前网络", value: ssid)
                Text("屏幕排布改动后会自动保存，并在回到该网络时恢复；只恢复排布，不会自动开启共享。")
                    .font(.caption).foregroundStyle(.secondary)
            } else {
                Text("读取到唯一的 Wi-Fi 名称后，屏幕排布改动会自动按网络保存。")
                    .font(.caption).foregroundStyle(.secondary)
                Button("前往权限设置") { store.openSettings(tab: SettingsCategory.permissions.rawValue) }
                    .buttonStyle(.link)
            }
        }
    }

    private var environmentSection: some View {
        Section("按 Wi-Fi 自动选择主控") {
            Text(snapshot?.input.environmentMessage ?? "等待网络状态")
                .font(.callout).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            Button("Wi-Fi 读取权限…") { store.openSettings(tab: SettingsCategory.permissions.rawValue) }
            Text("权限统一在“设置 → 权限”中管理。匹配网络自动恢复主控，未匹配网络自动暂停；原有布局和共享开关保持保存。")
                .font(.caption).foregroundStyle(.secondary)
            Picker("主控", selection: $selectedController) {
                Text("选择设备").tag("")
                if let snapshot {
                    Text(snapshot.localName).tag(snapshot.localId)
                    ForEach(snapshot.devices.filter(\.member), id: \.id) { Text($0.name).tag($0.id) }
                }
            }
            Toggle("记住当前 Wi-Fi", isOn: $remember)
            Button("使用此主控") {
                store.error = nil
                store.send(.chooseController(id: selectedController, remember: remember))
            }
            .disabled(selectedController.isEmpty)
            ForEach(snapshot?.input.rules ?? [], id: \.ssid) { rule in
                HStack {
                    Text(rule.ssid)
                    Spacer()
                    Text(controllerName(rule.controller)).foregroundStyle(.secondary)
                    Button("删除") { store.send(.forgetEnvironment(ssid: rule.ssid)) }
                }
            }
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

    private func controllerName(_ id: String) -> String {
        if id == snapshot?.localId { return snapshot?.localName ?? "本机" }
        return snapshot?.devices.first { $0.id == id }?.name ?? "离线设备"
    }

    private func cornerBinding(_ index: Int) -> Binding<Bool> {
        Binding(
            get: { draft.disabledCorners & (1 << index) != 0 },
            set: { on in
                if on { draft.disabledCorners |= 1 << index }
                else { draft.disabledCorners &= ~(1 << index) }
            }
        )
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
