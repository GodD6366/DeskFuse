import SwiftUI
import UniDeskEngine

struct MicrophoneSettingsView: View {
    let store: AppStore
    @State private var model = MicrophoneSharing.shared
    @State private var editingShortcut = false
    @State private var driverAction: MicrophoneDriverManager.Action?
    private var peers: [Device] { (store.snapshot?.devices ?? []).filter { $0.trusted && !$0.blocked && $0.id != store.snapshot?.localId } }
    private func update(_ mutate: (inout MicrophoneSettings) -> Void) {
        guard var settings = model.snapshot?.settings else { return }
        mutate(&settings); model.configure(settings)
    }
    var body: some View {
        Form {
            if let state = model.snapshot {
                Section {
                    Toggle("跨设备麦克风共享", isOn: Binding(get: { state.settings.enabled }, set: { value in update { $0.enabled = value } }))
                    Picker("麦克风来源", selection: Binding(get: { state.settings.source }, set: { value in update { $0.source = value } })) {
                        Text("此 Mac 的实体麦克风").tag("")
                        ForEach(peers, id: \.id) { peer in Text(peer.name + (peer.connected ? "" : "（离线）")).tag(peer.id) }
                        if !state.settings.source.isEmpty && !peers.contains(where: { $0.id == state.settings.source }) {
                            Text("原来源设备不可用").tag(state.settings.source)
                        }
                    }
                    Text("在有麦克风的 A 上选择“此 Mac”，在 B 上选择 A。两端均须启用，并在 A 允许 B 使用麦克风。").font(.caption).foregroundStyle(.secondary)
                } header: { Label("麦克风共享", systemImage: "mic") }
                if state.settings.source.isEmpty {
                    Section("实体麦克风") {
                        Picker("输入设备", selection: Binding(get: { state.settings.input_uid }, set: { value in update { $0.input_uid = value } })) {
                            Text("系统默认实体麦克风").tag("")
                            ForEach(model.inputs) { input in Text(input.name).tag(input.id) }
                            if !state.settings.input_uid.isEmpty && !model.inputs.contains(where: { $0.id == state.settings.input_uid }) {
                                Text("所选麦克风不可用").tag(state.settings.input_uid)
                            }
                        }
                        Button("刷新麦克风列表", action: model.refreshInputs)
                        if state.native.permission != "authorized" { Button("授权麦克风访问…", action: model.requestPermission) }
                        Text("只在目标应用读取虚拟麦克风时采集；锁屏、断线或关闭功能后停止。音频不保存到磁盘。").font(.caption).foregroundStyle(.secondary)
                    }
                    Section("允许使用此麦克风的设备") {
                        ForEach(peers, id: \.id) { peer in
                            Toggle(peer.name, isOn: Binding(get: { state.settings.allowed.contains(peer.id) }, set: { value in
                                update { settings in settings.allowed.removeAll { $0 == peer.id }; if value { settings.allowed.append(peer.id) } }
                            }))
                        }
                        if peers.isEmpty { Text("请先在设备页配对另一台 Mac。").foregroundStyle(.secondary) }
                        Text("同一时间仅允许一个目标设备使用。").font(.caption).foregroundStyle(.secondary)
                    }
                }
                Section("虚拟麦克风驱动") {
                    LabeledContent("安装状态", value: model.driverInstalled ? "已安装" : "未安装")
                    if model.driverInstalled {
                        LabeledContent("运行状态", value: state.native.driver ? "可连接" : "等待音频服务加载，请重启 Mac")
                    }
                    HStack {
                        Button(model.driverInstalled ? "重新安装驱动…" : "安装驱动…") { driverAction = .install }
                        if model.driverInstalled {
                            Button("卸载驱动…", role: .destructive) { driverAction = .uninstall }
                        }
                    }
                    if !state.settings.source.isEmpty {
                        Button("设为系统默认输入", action: model.setDefaultInput)
                            .disabled(!state.settings.enabled || !state.native.driver || !model.driverInstalled)
                        Text("在 B 的语音软件中选择“uniDesk 麦克风（来自 \(state.source_name)）”。仅使用系统默认输入的软件，可点击上方按钮。")
                            .font(.caption).foregroundStyle(.secondary)
                    } else {
                        Text("仅接收其它 Mac 麦克风的设备需要安装驱动。")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    if model.managingDriver { ProgressView("正在处理驱动，请完成系统管理员授权…") }
                    if let message = model.driverMessage { Text(message).font(.callout).foregroundStyle(.secondary) }
                }
                Section("语音快捷键") {
                    HStack {
                        Text("按住说话")
                        Spacer()
                        MicrophoneShortcutKeycaps(key: WorkflowKey(key_code: state.settings.shortcut_key, modifiers: state.settings.shortcut_modifiers))
                        Button("更改…") { editingShortcut = true }
                    }
                    Text("两端配置需与目标语音软件一致。按住与松开都交给按下时的目标；快捷键不裁切音频，避免丢失尾音。共享按键需要现有键鼠共享及辅助功能权限。").font(.caption).foregroundStyle(.secondary)
                }
                Section("连接状态") {
                    LabeledContent("状态", value: state.detail)
                    if !state.peer.isEmpty { LabeledContent("连接设备", value: peers.first(where: { $0.id == state.peer })?.name ?? "已配对设备") }
                    if !state.role.isEmpty {
                        ProgressView(value: Double(state.native.level), total: 1) { Text("麦克风电平") }
                    }
                    if state.native.readers { Text("目标应用正在读取虚拟麦克风").font(.caption).foregroundStyle(.secondary) }
                }
            } else if let connectionError = model.connectionError {
                Section {
                    Label("无法连接麦克风服务", systemImage: "exclamationmark.triangle")
                    Text(connectionError).font(.callout).foregroundStyle(.secondary)
                    Button("重试") { model.retryConnection(engine: store.engine) }
                }
            } else {
                ProgressView("正在连接麦克风服务…（最多 5 秒）")
            }
            if let error = model.error { Text(error).foregroundStyle(.red) }
        }
        .formStyle(.grouped)
        .disabled(model.saving || model.managingDriver)
        .onAppear { model.ensureStarted(engine: store.engine); model.refreshInputs() }
        .sheet(item: $driverAction) { action in
            MicrophoneDriverSheet(action: action) { restartAudio in
                driverAction = nil
                model.manageDriver(action, restartAudio: restartAudio)
            }
        }
        .sheet(isPresented: $editingShortcut) {
            if let settings = model.snapshot?.settings { MicrophoneShortcutSheet(store: store, settings: settings) { editingShortcut = false } }
        }
    }
}
private struct MicrophoneShortcutSheet: View {
    let store: AppStore
    let settings: MicrophoneSettings
    let close: () -> Void
    @State private var key: WorkflowKey?
    @State private var recording = false
    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text("语音快捷键").font(.title2.bold())
            MicrophoneShortcutRecorder(store: store, shortcut: $key, onRecordingChange: { recording = $0 })
            Text("请在目标设备的语音软件中设置相同按键。").font(.callout).foregroundStyle(.secondary)
            HStack {
                Spacer()
                Button("取消", action: close)
                Button("保存") {
                    var edited = settings
                    edited.shortcut_key = key?.key_code ?? 63
                    edited.shortcut_modifiers = key?.modifiers ?? 0
                    MicrophoneSharing.shared.configure(edited); close()
                }.buttonStyle(.borderedProminent).disabled(recording || key == nil)
            }
        }.padding(24).frame(width: 500)
        .onAppear { key = WorkflowKey(key_code: settings.shortcut_key, modifiers: settings.shortcut_modifiers) }
    }
}

private struct MicrophoneDriverSheet: View {
    let action: MicrophoneDriverManager.Action
    let confirm: (Bool) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var restartAudio = false

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text(action.title).font(.title2.bold())
            Text("仅操作 uniDesk 的麦克风驱动，需要在系统弹窗中授权管理员权限。操作前会关闭麦克风共享。")
            Toggle("完成后立即重启音频服务", isOn: $restartAudio)
            Text(restartAudio ? "这会短暂中断此 Mac 上所有应用的录音和播放。" : "稍后重启 Mac 后生效。")
                .font(.callout).foregroundStyle(.secondary)
            HStack {
                Spacer()
                Button("取消") { dismiss() }.keyboardShortcut(.cancelAction)
                Button(action.title, role: action == .uninstall ? .destructive : nil) { confirm(restartAudio) }
            }
        }
        .padding(24).frame(width: 460)
    }
}
