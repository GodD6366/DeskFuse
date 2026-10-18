import SwiftUI
import DeskFuseEngine

struct MicrophoneSettingsView: View {
    let store: AppStore
    @State private var model = MicrophoneSharing.shared
    @State private var driverAction: MicrophoneDriverManager.Action?
    private var peers: [Device] { (store.snapshot?.devices ?? []).filter { $0.trusted && !$0.blocked && $0.id != store.snapshot?.localId } }
    private func update(_ mutate: (inout MicrophoneSettings) -> Void) {
        guard var settings = model.snapshot?.settings else { return }
        mutate(&settings); model.configure(settings)
    }
    var body: some View {
        SystemSettingsFormPage {
            if let state = model.snapshot {
                SystemSettingsFormBlock(footer: "应用使用麦克风时自动传输，结束使用后停止。音频不保存。") {
                    Toggle("麦克风共享", isOn: Binding(get: { state.settings.enabled }, set: { value in update { $0.enabled = value } }))
                    Picker("声音来自", selection: Binding(get: { state.settings.source }, set: { value in update { $0.source = value } })) {
                        Text("此 Mac").tag("")
                        ForEach(peers, id: \.id) { peer in Text(peer.name + (peer.connected ? "" : "（离线）")).tag(peer.id) }
                        if !state.settings.source.isEmpty && !peers.contains(where: { $0.id == state.settings.source }) {
                            Text("原来源设备不可用").tag(state.settings.source)
                        }
                    }
                    Divider()
                    HStack(alignment: .top, spacing: 10) {
                        Image(systemName: state.phase == "streaming" ? "waveform" : "mic")
                            .foregroundStyle(state.phase == "streaming" ? Color.green : Color.secondary)
                        VStack(alignment: .leading, spacing: 4) {
                            Text(statusText(state)).font(.callout)
                            if !state.peer.isEmpty {
                                Text((state.role == "sender" ? "正在提供给 " : "正在接收自 ") + (peers.first(where: { $0.id == state.peer })?.name ?? "已配对设备"))
                                    .font(.caption).foregroundStyle(.secondary)
                            }
                        }
                        Spacer(minLength: 0)
                    }
                    if state.phase == "streaming" {
                        ProgressView(value: Double(state.native.level), total: 1)
                            .accessibilityLabel("麦克风电平")
                    }
                }
                if state.settings.source.isEmpty {
                    sourceSettings(state)
                } else {
                    receiverSettings(state)
                }
                if model.driverInstalled {
                    SystemSettingsFormBlock {
                        DisclosureGroup("驱动维护") {
                            LabeledContent("驱动状态", value: state.native.driver ? "已安装，可连接" : "已安装，服务未响应")
                            Text("仅接收端需要驱动。服务未响应时可重新安装，并选择立即重启音频服务。")
                                .font(.caption).foregroundStyle(.secondary)
                            HStack {
                                Button("重新安装…") { driverAction = .install }
                                Spacer()
                                Button("卸载…", role: .destructive) { driverAction = .uninstall }
                            }
                        }
                    }
                }
                if model.managingDriver { ProgressView("正在处理驱动，请完成系统管理员授权…") }
                if let message = model.driverMessage { Text(message).font(.callout).foregroundStyle(.secondary) }
            } else if let connectionError = model.connectionError {
                SystemSettingsFormBlock {
                    Label("无法连接麦克风服务", systemImage: "exclamationmark.triangle")
                    Text(connectionError).font(.callout).foregroundStyle(.secondary)
                    Button("重试") { model.retryConnection(engine: store.engine) }
                }
            } else {
                ProgressView("正在连接麦克风服务…")
            }
            if model.snapshot != nil, let connectionError = model.connectionError { Text(connectionError).foregroundStyle(.red) }
            if let error = model.error { Text(error).foregroundStyle(.red) }
        }
        .disabled(model.saving || model.managingDriver)
        .onAppear { model.ensureStarted(engine: store.engine); model.refreshInputs() }
        .sheet(item: $driverAction) { action in
            MicrophoneDriverSheet(action: action) { restartAudio in
                driverAction = nil
                model.manageDriver(action, restartAudio: restartAudio)
            }.floatingWorkspaceStyle()
        }
    }
    private func statusText(_ state: MicrophoneSharingSnapshot) -> String {
        guard state.settings.enabled else { return "共享已关闭" }
        if !state.settings.source.isEmpty && !state.native.driver {
            return model.driverInstalled ? "驱动服务未响应，请在“驱动维护”中恢复" : "安装虚拟麦克风后即可使用"
        }
        if state.settings.source.isEmpty && state.native.permission != "authorized" {
            return state.native.permission == "denied" ? "未允许麦克风访问" : "需要允许麦克风访问"
        }
        if state.settings.source.isEmpty && state.settings.allowed.isEmpty { return "选择下方设备，允许它使用此麦克风" }
        return state.detail
    }
    @ViewBuilder private func sourceSettings(_ state: MicrophoneSharingSnapshot) -> some View {
        SystemSettingsFormBlock(title: "提供此 Mac 的麦克风", footer: "在另一台 Mac 开启麦克风共享，并将声音来源选为此 Mac。同一时间可供一台设备使用。") {
            Picker("输入设备", selection: Binding(get: { state.settings.input_uid }, set: { value in update { $0.input_uid = value } })) {
                Text("系统默认麦克风").tag("")
                ForEach(model.inputs) { input in Text(input.name).tag(input.id) }
                if !state.settings.input_uid.isEmpty && !model.inputs.contains(where: { $0.id == state.settings.input_uid }) {
                    Text("所选麦克风不可用").tag(state.settings.input_uid)
                }
            }
            if state.native.permission != "authorized" {
                Button(state.native.permission == "denied" ? "打开麦克风权限设置…" : "允许麦克风访问…", action: model.requestPermission)
            }
            Divider()
            Text("允许使用的设备").font(.callout.weight(.medium))
            ForEach(peers, id: \.id) { peer in
                Toggle(peer.name, isOn: Binding(get: { state.settings.allowed.contains(peer.id) }, set: { value in
                    update { settings in settings.allowed.removeAll { $0 == peer.id }; if value { settings.allowed.append(peer.id) } }
                }))
            }
            if peers.isEmpty { Text("请先在“设备”中配对另一台 Mac。").foregroundStyle(.secondary) }
        }
    }
    @ViewBuilder private func receiverSettings(_ state: MicrophoneSharingSnapshot) -> some View {
        SystemSettingsFormBlock(title: "在此 Mac 使用", footer: "直接使用微信输入法等语音软件的录音按钮或快捷键，无需在 DeskFuse 额外配置。") {
            if !model.driverInstalled {
                Text("先安装虚拟麦克风，让语音软件接收远端声音。")
                Button("安装虚拟麦克风…") { driverAction = .install }
            } else {
                LabeledContent("应用中的输入设备", value: "DeskFuse 麦克风（来自 \(state.source_name)）")
                Text("在语音软件中选择上述麦克风；使用系统默认输入的软件，可点击下方按钮。")
                    .font(.caption).foregroundStyle(.secondary)
                Button("设为系统默认麦克风", action: model.setDefaultInput)
                    .disabled(!state.settings.enabled || !state.native.driver)
            }
            Text("来源 Mac 需开启共享，并允许此 Mac 使用。").font(.caption).foregroundStyle(.secondary)
        }
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
            Text("仅操作 DeskFuse 的麦克风驱动，需要在系统弹窗中授权管理员权限。操作前会关闭麦克风共享。")
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
