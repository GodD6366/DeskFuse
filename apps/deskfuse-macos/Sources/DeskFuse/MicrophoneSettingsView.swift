import SwiftUI
import DeskFuseEngine

struct MicrophoneSettingsView: View {
    let store: AppStore
    @State private var model = MicrophoneSharing.shared
    @State private var driverAction: MicrophoneDriverManager.Action?
    @Environment(\.colorScheme) private var scheme
    private var peers: [Device] { (store.snapshot?.devices ?? []).filter { $0.trusted && !$0.blocked && $0.id != store.snapshot?.localId } }
    private func update(_ mutate: (inout MicrophoneSettings) -> Void) {
        guard var settings = model.snapshot?.settings else { return }
        mutate(&settings); model.configure(settings)
    }
    var body: some View {
        SystemSettingsFormPage {
            VStack(alignment: .leading, spacing: 24) {
                if let state = model.snapshot {
                    routeSurface(state)
                    configuration(state)
                    if model.driverInstalled { maintenance(state) }
                    if model.managingDriver { ProgressView("正在处理驱动，请完成系统管理员授权…") }
                    if let message = model.driverMessage { Text(message).font(.callout).workspaceSecondaryText() }
                } else if let connectionError = model.connectionError {
                    SystemSettingsFormBlock {
                        Label("无法连接麦克风服务", systemImage: "exclamationmark.triangle")
                        Text(connectionError).font(.callout).workspaceSecondaryText()
                        Button("重试") { model.retryConnection(engine: store.engine) }
                    }
                } else {
                    ProgressView("正在连接麦克风服务…")
                }
                if model.snapshot != nil, let connectionError = model.connectionError { Text(connectionError).foregroundStyle(.red) }
                if let error = model.error { Text(error).foregroundStyle(.red) }
            }
            .frame(maxWidth: 820, alignment: .leading)
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

    private func routeSurface(_ state: MicrophoneSharingSnapshot) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .center, spacing: 16) {
                VStack(alignment: .leading, spacing: 6) {
                    Text("跨设备麦克风").font(WorkspaceTypography.section)
                    Text(state.settings.source.isEmpty ? "把此 Mac 的麦克风提供给其他设备。" : "用另一台 Mac 的麦克风，在此 Mac 听写或通话。")
                        .font(WorkspaceTypography.detail).workspaceSecondaryText()
                }
                Spacer(minLength: 8)
                Toggle("麦克风共享", isOn: Binding(get: { state.settings.enabled }, set: { value in update { $0.enabled = value } }))
                    .labelsHidden().toggleStyle(.switch).controlSize(.regular)
                    .accessibilityLabel("麦克风共享")
            }
            .padding(24)
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 20) {
                    endpoint(symbol: "mic.fill", title: sourceName(state), subtitle: "声音来源")
                    routeConnection(state).frame(minWidth: 120, maxWidth: 180)
                    endpoint(symbol: "desktopcomputer", title: destinationName(state), subtitle: "使用麦克风")
                }
                VStack(spacing: 20) {
                    endpoint(symbol: "mic.fill", title: sourceName(state), subtitle: "声音来源")
                    routeConnection(state).frame(width: 180)
                    endpoint(symbol: "desktopcomputer", title: destinationName(state), subtitle: "使用麦克风")
                }
                .frame(maxWidth: .infinity)
            }
            .padding(.horizontal, 24).padding(.top, 8).padding(.bottom, 32)
            Divider().padding(.horizontal, 24)
            HStack(alignment: .top, spacing: 12) {
                Image(systemName: state.phase == "streaming" ? "waveform" : (isReady(state) ? "moon.zzz" : "mic"))
                    .font(.system(size: 16)).foregroundStyle(statusColor(state)).frame(width: 24, height: 20)
                VStack(alignment: .leading, spacing: 5) {
                    Text(usageTitle(state)).font(WorkspaceTypography.heading)
                    Text(usageDetail(state)).font(WorkspaceTypography.detail).workspaceSecondaryText()
                        .fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: 0)
                if state.phase == "streaming" {
                    MicrophoneLevelView(level: state.native.level)
                        .frame(width: 72, height: 24)
                        .accessibilityLabel("麦克风电平")
                        .accessibilityValue("\(Int(state.native.level * 100))%")
                }
            }
            .padding(24)
        }
        .background(QuietWorkspacePalette.surface(scheme), in: RoundedRectangle(cornerRadius: 12))
    }
    private func endpoint(symbol: String, title: String, subtitle: String) -> some View {
        VStack(spacing: 12) {
            Image(systemName: symbol).font(.system(size: 26, weight: .light))
                .frame(width: 64, height: 64)
                .background(QuietWorkspacePalette.control(scheme), in: RoundedRectangle(cornerRadius: 16))
            VStack(spacing: 5) {
                Text(title).font(WorkspaceTypography.heading).lineLimit(2).multilineTextAlignment(.center)
                Text(subtitle).font(WorkspaceTypography.detail).workspaceSecondaryText()
            }
        }
        .frame(minWidth: 140, maxWidth: .infinity)
    }
    private func routeConnection(_ state: MicrophoneSharingSnapshot) -> some View {
        VStack(spacing: 10) {
            Text(connectionLabel(state))
                .font(.system(size: 11, weight: .medium)).foregroundStyle(statusColor(state))
                .padding(.horizontal, 10).padding(.vertical, 5)
                .background(statusColor(state).opacity(0.08), in: Capsule())
            HStack(spacing: 0) {
                Rectangle().fill(QuietWorkspacePalette.line(scheme)).frame(height: 1)
                Image(systemName: "chevron.right").font(.system(size: 9, weight: .semibold)).foregroundStyle(.tertiary)
            }
            Text("音频不保存").font(.system(size: 11)).workspaceSecondaryText()
        }
    }
    private func sourceName(_ state: MicrophoneSharingSnapshot) -> String {
        state.settings.source.isEmpty ? "此 Mac" : state.source_name
    }
    private func destinationName(_ state: MicrophoneSharingSnapshot) -> String {
        guard state.settings.source.isEmpty else { return "此 Mac" }
        if let peer = peers.first(where: { $0.id == state.peer }) { return peer.name }
        return state.settings.allowed.count == 1 ? (peers.first(where: { state.settings.allowed.contains($0.id) })?.name ?? "已允许的 Mac") : "已允许的 Mac"
    }
    private func statusColor(_ state: MicrophoneSharingSnapshot) -> Color {
        state.phase == "streaming" ? .green : .secondary
    }
    private func isReady(_ state: MicrophoneSharingSnapshot) -> Bool {
        guard state.settings.enabled, !state.native.locked, state.phase == "standby" else { return false }
        if state.settings.source.isEmpty {
            return state.native.permission == "authorized" && !state.settings.allowed.isEmpty
        }
        return state.native.driver && peers.contains(where: { $0.id == state.settings.source && $0.connected })
    }
    private func connectionLabel(_ state: MicrophoneSharingSnapshot) -> String {
        if !state.settings.enabled { return "已关闭" }
        if state.phase == "streaming" { return "传输中" }
        if isReady(state) { return "按需连接" }
        if state.phase == "waiting" { return "连接中" }
        return "未连接"
    }
    private func usageTitle(_ state: MicrophoneSharingSnapshot) -> String {
        guard state.settings.enabled else { return "共享已关闭" }
        if state.phase == "streaming" {
            let names = Array(Set(model.readerApps.map(\.name))).sorted()
            if !names.isEmpty { return names.joined(separator: "、") + "正在使用" }
            return state.role == "sender" ? "正在向另一台 Mac 提供声音" : "应用正在使用麦克风"
        }
        if isReady(state) { return "已就绪，等待使用" }
        if !state.settings.source.isEmpty && !state.native.driver { return model.driverInstalled ? "麦克风暂时不可用" : "完成安装后即可使用" }
        if state.settings.source.isEmpty && state.native.permission != "authorized" { return "需要麦克风权限" }
        if state.settings.source.isEmpty && state.settings.allowed.isEmpty { return "尚未允许其他设备" }
        if state.phase == "failed" { return "连接暂时中断" }
        if state.phase == "waiting" { return "正在连接麦克风" }
        if state.native.locked { return "已暂停共享" }
        return "等待来源设备上线"
    }
    private func usageDetail(_ state: MicrophoneSharingSnapshot) -> String {
        guard state.settings.enabled else { return "开启后，仅在应用读取麦克风时采集和传输。" }
        if model.readerApps.contains(where: \.isSoundSettings) {
            return "系统声音设置正在检测输入电平。离开该页面后，若没有其他应用使用，传输会自动停止。"
        }
        if state.phase == "streaming" { return "应用停止读取后自动结束采集与传输。" }
        if isReady(state) {
            return "当前没有采集或传输音频。应用开始录音时自动连接。"
        }
        return statusText(state)
    }
    private func statusText(_ state: MicrophoneSharingSnapshot) -> String {
        guard state.settings.enabled else { return "共享已关闭" }
        if !state.settings.source.isEmpty && !state.native.driver {
            return model.driverInstalled ? "驱动服务未响应，请在“驱动维护”中恢复" : "安装虚拟麦克风后即可使用"
        }
        if state.settings.source.isEmpty && state.native.permission != "authorized" {
            return state.native.permission == "denied" ? "未允许麦克风访问" : "需要允许麦克风访问"
        }
        if state.settings.source.isEmpty && state.settings.allowed.isEmpty { return "选择允许使用此麦克风的设备" }
        if !state.settings.source.isEmpty && !peers.contains(where: { $0.id == state.settings.source && $0.connected }) { return "来源设备离线，等待重新连接" }
        return state.detail
    }
    @ViewBuilder private func configuration(_ state: MicrophoneSharingSnapshot) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("连接设置").font(WorkspaceTypography.heading).padding(.bottom, 12)
            VStack(alignment: .leading, spacing: 0) {
                settingsRow("声音来源", detail: "选择提供麦克风的 Mac") {
                    Picker("声音来源", selection: Binding(get: { state.settings.source }, set: { value in update { $0.source = value } })) {
                        Text("此 Mac").tag("")
                        ForEach(peers, id: \.id) { peer in Text(peer.name + (peer.connected ? "" : "（离线）")).tag(peer.id) }
                        if !state.settings.source.isEmpty && !peers.contains(where: { $0.id == state.settings.source }) {
                            Text("原来源设备不可用").tag(state.settings.source)
                        }
                    }.labelsHidden().frame(maxWidth: 240)
                }
                Divider().padding(.horizontal, 20)
                if state.settings.source.isEmpty { sourceSettings(state) } else { receiverSettings(state) }
            }
            .background(QuietWorkspacePalette.surface(scheme), in: RoundedRectangle(cornerRadius: 12))
            if !state.settings.source.isEmpty {
                Label("直接使用微信输入法等应用的录音按钮或快捷键。", systemImage: "info.circle")
                    .font(WorkspaceTypography.detail).workspaceSecondaryText().padding(.top, 12)
            }
        }
    }
    private func settingsRow<Control: View>(_ title: String, detail: String, @ViewBuilder control: () -> Control) -> some View {
        HStack(spacing: 20) {
            VStack(alignment: .leading, spacing: 5) {
                Text(title).font(WorkspaceTypography.body)
                Text(detail).font(WorkspaceTypography.detail).workspaceSecondaryText().fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 8)
            control()
        }.padding(20)
    }
    @ViewBuilder private func sourceSettings(_ state: MicrophoneSharingSnapshot) -> some View {
        settingsRow("输入设备", detail: "仅使用此 Mac 的实体麦克风") {
            Picker("输入设备", selection: Binding(get: { state.settings.input_uid }, set: { value in update { $0.input_uid = value } })) {
                Text("系统默认麦克风").tag("")
                ForEach(model.inputs) { input in Text(input.name).tag(input.id) }
                if !state.settings.input_uid.isEmpty && !model.inputs.contains(where: { $0.id == state.settings.input_uid }) {
                    Text("所选麦克风不可用").tag(state.settings.input_uid)
                }
            }.labelsHidden().frame(maxWidth: 240)
        }
        if state.native.permission != "authorized" {
            Button(state.native.permission == "denied" ? "打开麦克风权限设置…" : "允许麦克风访问…", action: model.requestPermission)
                .padding(.horizontal, 20).padding(.bottom, 16)
        }
        Divider().padding(.horizontal, 20)
        VStack(alignment: .leading, spacing: 16) {
            Text("允许使用的设备").font(WorkspaceTypography.body)
            ForEach(peers, id: \.id) { peer in
                Toggle(isOn: Binding(get: { state.settings.allowed.contains(peer.id) }, set: { value in
                    update { settings in settings.allowed.removeAll { $0 == peer.id }; if value { settings.allowed.append(peer.id) } }
                })) {
                    Label(peer.name, systemImage: "laptopcomputer")
                }.toggleStyle(.switch)
            }
            if peers.isEmpty { Text("先在“设备”中配对另一台 Mac。").workspaceSecondaryText() }
            Text("同一时间可供一台设备使用。").font(WorkspaceTypography.detail).workspaceSecondaryText()
        }.padding(20)
    }
    @ViewBuilder private func receiverSettings(_ state: MicrophoneSharingSnapshot) -> some View {
        if !model.driverInstalled {
            settingsRow("虚拟麦克风", detail: "安装后，语音应用即可接收远端声音") {
                Button("安装…") { driverAction = .install }
            }
        } else {
            settingsRow("系统输入", detail: model.isDefaultInput == true ? "DeskFuse 麦克风已设为默认输入" : "让使用系统默认输入的应用接收远端声音") {
                if model.isDefaultInput == true {
                    Label("已设为默认", systemImage: "checkmark.circle.fill").foregroundStyle(.secondary)
                } else {
                    Button("设为默认", action: model.setDefaultInput)
                        .disabled(!state.settings.enabled || !state.native.driver)
                }
            }
        }
    }
    private func maintenance(_ state: MicrophoneSharingSnapshot) -> some View {
        DisclosureGroup {
            VStack(alignment: .leading, spacing: 12) {
                Text(state.native.driver ? "驱动已安装，连接正常。" : "驱动已安装，但服务未响应。可重新安装并选择立即重启音频服务。")
                    .font(WorkspaceTypography.detail).workspaceSecondaryText()
                HStack {
                    Button("重新安装…") { driverAction = .install }
                    Button("卸载…", role: .destructive) { driverAction = .uninstall }
                }
            }.padding(.top, 12)
        } label: {
            Label("驱动维护", systemImage: "wrench.and.screwdriver").font(WorkspaceTypography.detail).workspaceSecondaryText()
        }
    }
}

/// A compact level indicator, not a progress bar. It only reflects real samples.
private struct MicrophoneLevelView: View {
    let level: Float
    var body: some View {
        HStack(spacing: 3) {
            ForEach(0..<12) { index in
                Capsule().fill(Double(index) / 12 < min(1, sqrt(Double(max(0, level)))) ? Color.green : Color.primary.opacity(0.08))
                    .frame(width: 3, height: CGFloat(8 + (index % 4) * 4))
            }
        }.accessibilityElement(children: .ignore)
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
