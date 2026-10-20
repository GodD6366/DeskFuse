import SwiftUI
import DeskFuseEngine

struct MicrophoneSettingsView: View {
    let store: AppStore
    @State private var model = MicrophoneSharing.shared
    @State private var driverAction: MicrophoneDriverManager.Action?
    @State private var editingRole = false
    @AppStorage("microphone.role") private var storedRole = ""
    @Environment(\.colorScheme) private var scheme

    private var peers: [Device] {
        (store.snapshot?.devices ?? []).filter {
            $0.trusted && !$0.blocked && $0.id != store.snapshot?.localId
        }
    }

    private func update(_ mutate: (inout MicrophoneSettings) -> Void) {
        guard var settings = model.snapshot?.settings else { return }
        mutate(&settings)
        model.configure(settings)
    }

    var body: some View {
        DFScrollPage(maxWidth: DFMetrics.focusMaxWidth) {
            Group {
                if let state = model.snapshot {
                    let role = resolvedRole(state)
                    if role == nil || editingRole {
                        identityPicker(current: role, state: state)
                            .frame(maxWidth: DFMetrics.focusMaxWidth)
                            .frame(maxWidth: .infinity)
                            .transition(.asymmetric(insertion: .opacity.combined(with: .scale(scale: 0.97)),
                                                    removal: .opacity))
                    } else if let role {
                        configured(state, role: role)
                            .transition(.asymmetric(insertion: .opacity.combined(with: .move(edge: .bottom)),
                                                    removal: .opacity))
                    }
                } else if let connectionError = model.connectionError {
                    VStack(alignment: .leading, spacing: DFSpace.xl) {
                        MicrophoneNoticeCard(
                            symbol: "mic.slash",
                            tone: .warning,
                            title: "无法连接麦克风服务",
                            message: MicrophoneErrorText.friendly(connectionError),
                            detail: MicrophoneErrorText.detail(connectionError),
                            primaryTitle: "重新连接",
                            primaryAction: { model.retryConnection(engine: store.engine) }
                        )
                        privacyNote
                    }
                    .frame(maxWidth: DFMetrics.focusMaxWidth)
                    .frame(maxWidth: .infinity)
                } else {
                    VStack(alignment: .leading, spacing: DFSpace.xl) {
                        MicrophoneLoadingCard()
                        privacyNote
                    }
                    .frame(maxWidth: DFMetrics.focusMaxWidth)
                    .frame(maxWidth: .infinity)
                }
            }
            .animation(DFMotion.settle, value: screenKey)
            if let error = model.error {
                MicrophoneNoticeCard(
                    symbol: "exclamationmark.triangle.fill",
                    tone: .danger,
                    title: "操作未完成",
                    message: MicrophoneErrorText.friendly(error),
                    detail: MicrophoneErrorText.detail(error),
                    onDismiss: { model.error = nil }
                )
                .transition(.move(edge: .top).combined(with: .opacity))
            }
        }
        .animation(DFMotion.snappy, value: model.error)
        .disabled(model.saving || model.managingDriver)
        .onAppear {
            model.ensureStarted(engine: store.engine)
            model.refreshInputs()
        }
        .task(id: automaticSourceCandidates) {
            await selectDefaultSource()
        }
        .onChange(of: model.snapshot?.settings) { _, settings in
            // Backfill the local preference for installations that predate the
            // identity screen; never override a role the user is switching to.
            guard storedRole.isEmpty, let settings else { return }
            if !settings.source.isEmpty {
                storedRole = MicrophoneRole.receiver.rawValue
            } else if settings.enabled || !settings.allowed.isEmpty {
                storedRole = MicrophoneRole.provider.rawValue
            }
        }
        .sheet(item: $driverAction) { action in
            MicrophoneDriverSheet(action: action) { restartAudio in
                driverAction = nil
                model.manageDriver(action, restartAudio: restartAudio)
            }.dfFloating()
        }
    }

    /// Only fill an unset receiver source. Never replace a saved/manual choice.
    private var automaticSourceCandidates: [String] {
        guard let state = model.snapshot, resolvedRole(state) == .receiver,
              state.settings.source.isEmpty, !editingRole,
              !model.saving, !model.managingDriver else { return [] }
        return orderedPeers(state).filter(\.connected).map(\.id)
    }

    private func selectDefaultSource() async {
        guard let engine = store.engine, !automaticSourceCandidates.isEmpty else { return }
        while !Task.isCancelled {
            for id in automaticSourceCandidates {
                let enabled = await Task.detached(priority: .utility) {
                    (try? engine.microphoneSourceEnabled(peerId: id)) == true
                }.value
                // A manual selection or role/connection change may have happened
                // while querying. Recheck before persisting the default.
                guard !Task.isCancelled, !automaticSourceCandidates.isEmpty else { return }
                if enabled && automaticSourceCandidates.contains(id) {
                    update { $0.source = id }
                    return
                }
            }
            do { try await Task.sleep(for: .seconds(3)) }
            catch { return }
        }
    }

    /// 页面处于哪一屏：身份选择、已配置（区分角色）、连接失败或加载中，用于驱动屏间过渡。
    private var screenKey: String {
        guard let state = model.snapshot else { return model.connectionError == nil ? "loading" : "failed" }
        guard let role = resolvedRole(state), !editingRole else { return "identity" }
        return role.rawValue
    }

    private func configured(_ state: MicrophoneSharingSnapshot, role: MicrophoneRole) -> some View {
        VStack(alignment: .leading, spacing: DFSpace.l) {
            DFSectionLabel("当前状态")
            roleStatus(state, role: role)
            if role == .provider { inputPermission(state) }
            runtimeNotices(state, role: role)
            if role == .provider { configurationNotices(state, role: role) }
            MicrophoneSpeechTestCard(receiver: role == .receiver)
            if role == .receiver && (model.driverInstalled || model.legacyDriverInstalled) {
                driverMaintenance
            }
        }
        .frame(maxWidth: DFMetrics.focusMaxWidth)
        .frame(maxWidth: .infinity)
    }

    @ViewBuilder private func configurationNotices(_ state: MicrophoneSharingSnapshot, role: MicrophoneRole) -> some View {
        completionNotice(state, role: role)
    }

    /// 运行期异常：来源不可用、连接中断、后台连接异常。权限与驱动问题由各自的设置区展示。
    @ViewBuilder private func runtimeNotices(_ state: MicrophoneSharingSnapshot, role: MicrophoneRole) -> some View {
        if role == .receiver && !state.settings.source.isEmpty && !peers.isEmpty
            && !peers.contains(where: { $0.id == state.settings.source }) {
            DFNotice(tone: .warning, title: "原来源设备不可用",
                     message: "重新配对来源设备，或点击“声音来源”换一台 Mac。")
        }
        if state.settings.enabled && state.phase == "failed" {
            DFNotice(tone: .warning, title: "连接暂时中断",
                     message: statusText(state, role: role))
        }
        if let connectionError = model.connectionError,
           !connectionErrorHandled(state, role: role) {
            DFNotice(tone: .warning, title: "后台连接暂时异常",
                     message: MicrophoneErrorText.friendly(connectionError))
        }
    }

    // MARK: - 身份

    private func identityPicker(current: MicrophoneRole?, state: MicrophoneSharingSnapshot) -> some View {
        VStack(alignment: .leading, spacing: DFSpace.l) {
            HStack(alignment: .top, spacing: DFSpace.m) {
                VStack(alignment: .leading, spacing: DFSpace.xs) {
                    Text("这台 Mac 要怎么参与？").font(DFType.title)
                    Text("先选择身份，再完成对应的设置。之后可以随时更改。")
                        .font(DFType.detail).dfSecondaryText()
                        .fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: DFSpace.s)
                if current != nil {
                    Button("取消") { editingRole = false }.buttonStyle(.dfPlain)
                }
            }
            ViewThatFits(in: .horizontal) {
                HStack(alignment: .top, spacing: DFSpace.m) {
                    identityCard(.provider, state: state)
                    identityCard(.receiver, state: state)
                }
                VStack(spacing: DFSpace.m) {
                    identityCard(.provider, state: state)
                    identityCard(.receiver, state: state)
                }
            }
            if peers.isEmpty {
                DFNotice(tone: .neutral, title: "先配对另一台 Mac",
                         message: "提供和使用麦克风都需要两台 Mac 已在“设备”中完成配对。",
                         actionTitle: "前往设备", action: { store.selectedPage = .devices })
            } else {
                DFNotice(tone: .neutral, title: "开始前请确认",
                         message: "两台 Mac 需要先完成配对并保持可信。音频只在应用读取麦克风时传输，不会保存。")
            }
        }
    }

    private func identityCard(_ role: MicrophoneRole, state: MicrophoneSharingSnapshot) -> some View {
        MicrophoneIdentityCard(role: role) { choose(role, from: state) }
    }

    private func choose(_ role: MicrophoneRole, from state: MicrophoneSharingSnapshot) {
        let previous = resolvedRole(state)
        storedRole = role.rawValue
        editingRole = false
        guard previous != role, let current = model.snapshot?.settings else { return }
        var settings = current
        // Reconfiguration stops the previous session in the engine; changing
        // identity must not also turn off the page's master switch.
        if role == .provider { settings.source = "" }
        if settings != current { model.configure(settings) }
    }

    private func resolvedRole(_ state: MicrophoneSharingSnapshot) -> MicrophoneRole? {
        // `source` is the engine's source of truth for a configured receiver.
        if !state.settings.source.isEmpty { return .receiver }
        if let role = MicrophoneRole(rawValue: storedRole) { return role }
        if state.settings.enabled || !state.settings.allowed.isEmpty { return .provider }
        return nil
    }

    // MARK: - 角色状态

    private func roleStatus(_ state: MicrophoneSharingSnapshot, role: MicrophoneRole) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .center, spacing: DFSpace.m) {
                Image(systemName: role.symbol)
                    .font(DFType.icon(.large, weight: .regular))
                    .foregroundStyle(DFPalette.accent)
                    .frame(width: 40, height: 40)
                    .background(DFPalette.control(scheme),
                                in: RoundedRectangle(cornerRadius: DFRadius.control, style: .continuous))
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: DFSpace.xxs) {
                    HStack(spacing: DFSpace.s) {
                        Text(role.title).font(DFType.title)
                        DFBadge(role == .provider ? "提供端" : "接收端", tone: .neutral)
                    }
                    Text(roleStatusDetail(state, role: role))
                        .font(DFType.detail).dfSecondaryText()
                        .fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: DFSpace.s)
                Button("更换身份") { editingRole = true }.buttonStyle(.dfPlain)
            }
            .padding(DFSpace.xl)
            Divider()
            MicrophoneSignalStage(source: routeSource(state, role: role),
                                  destination: routeDestination(state, role: role),
                                  mode: signalMode(state, role: role),
                                  tone: connectionTone(state, role: role),
                                  label: connectionLabel(state, role: role),
                                  level: state.native.level,
                                  sourceMenu: role == .provider ? AnyView(inputPicker(state)) : AnyView(sourcePicker(state)),
                                  sourcePickerTitle: role == .provider ? "选择麦克风" : "选择来源设备",
                                  sourceSelection: role == .provider ? state.settings.input_uid : state.settings.source,
                                  destinationControls: role == .receiver ? AnyView(driverControls(state)) : nil)
                .padding(.horizontal, DFSpace.xl)
                .padding(.vertical, DFSpace.xl + DFSpace.xs)
            // 使用状态区常驻显示：开始/结束采集只切换内容，不插入或移除行，避免卡片高度变化引发整页抖动。
            Divider()
            usageSection(state, role: role)
                .padding(DFSpace.xl)
        }
        .frame(maxHeight: .infinity, alignment: .top)
        .dfCard()
        .animation(DFMotion.settle, value: state.settings.enabled)
    }

    private func roleStatusDetail(_ state: MicrophoneSharingSnapshot, role: MicrophoneRole) -> String {
        switch role {
        case .provider:
            if state.native.permission != "authorized" {
                return state.settings.enabled
                    ? "需要麦克风权限才能提供声音。"
                    : "先允许 DeskFuse 读取此 Mac 的麦克风，再开启共享。"
            }
            if peers.isEmpty {
                return "配对电脑后即可直接使用，不需要逐台授权。"
            }
            if let active = activePeerName(state) {
                let action = state.phase == "streaming" ? "正在使用" : "已连接"
                return "\(peers.count) 台已配对设备 · \(active) \(action)"
            }
            if state.settings.enabled {
                return onlinePeerCount > 0
                    ? "\(peers.count) 台已配对设备 · \(onlinePeerCount) 台在线，可直接使用"
                    : "\(peers.count) 台已配对设备 · 等待上线"
            }
            return "\(peers.count) 台已配对设备可直接使用；打开开关即可。"
        case .receiver:
            guard !state.settings.source.isEmpty else { return "选择提供麦克风的 Mac，再安装虚拟麦克风。" }
            let name = sourcePeerName(state)
            if !model.driverInstalled { return "来源：\(name) · 尚未安装虚拟麦克风" }
            if !state.native.driver { return "来源：\(name) · 等待驱动就绪" }
            guard model.isDefaultInput == true else { return "来源：\(name) · 请设为系统默认输入" }
            guard state.settings.enabled else { return "来源：\(name) · 设置已完成，打开开关即可使用" }
            if model.isDefaultInput == true { return "来源：\(name) · 已是系统默认输入" }
            return "来源：\(name) · 等待语音应用使用"
        }
    }

    private func routeSource(_ state: MicrophoneSharingSnapshot, role: MicrophoneRole) -> MicrophoneSignalEndpoint {
        switch role {
        case .provider:
            MicrophoneSignalEndpoint(symbol: "mic.fill", title: inputDisplayName(state), subtitle: "点击选择麦克风")
        case .receiver:
            MicrophoneSignalEndpoint(symbol: "laptopcomputer", title: sourcePeerName(state), subtitle: "声音来源")
        }
    }

    private func routeDestination(_ state: MicrophoneSharingSnapshot, role: MicrophoneRole) -> MicrophoneSignalEndpoint {
        switch role {
        case .provider:
            let count = peers.count
            let active = activePeerName(state)
            let title = active
                ?? (count == 0 ? "等待配对设备" : (count == 1 ? "1 台已配对电脑" : "\(count) 台已配对电脑"))
            let subtitle: String
            if active != nil {
                subtitle = state.phase == "streaming" ? "正在使用 · 共 \(count) 台" : "已连接 · 共 \(count) 台"
            } else if count == 0 {
                subtitle = "先在设备中配对"
            } else if !state.settings.enabled {
                subtitle = "已配对 · 共享未开启"
            } else {
                subtitle = onlinePeerCount > 0 ? "无需授权 · 按需连接" : "等待设备上线"
            }
            return MicrophoneSignalEndpoint(symbol: "laptopcomputer", title: title, subtitle: subtitle)
        case .receiver:
            return MicrophoneSignalEndpoint(symbol: "waveform",
                                            title: model.driverInstalled ? "DeskFuse 麦克风" : "虚拟麦克风",
                                            subtitle: model.driverInstalled ? "此 Mac 的虚拟输入" : "需要安装驱动")
        }
    }

    /// 把连接状态映射到信号舞台的动效档位，与状态胶囊的文案和色调保持一致。
    private func signalMode(_ state: MicrophoneSharingSnapshot, role: MicrophoneRole) -> MicrophoneSignalMode {
        guard state.settings.enabled else { return .off }
        if state.phase == "streaming" { return .streaming }
        if connectionTone(state, role: role) == .warning { return .attention }
        if isReady(state, role: role) { return .ready }
        return .waiting
    }

    private func connectionTone(_ state: MicrophoneSharingSnapshot, role: MicrophoneRole) -> DFTone {
        guard state.settings.enabled else { return .neutral }
        if state.phase == "streaming" { return .success }
        if role == .provider && state.native.permission != "authorized" { return .warning }
        if role == .receiver && (!model.driverInstalled || !state.native.driver) { return .warning }
        if state.phase == "failed" { return .warning }
        if isReady(state, role: role) { return .accent }
        return .neutral
    }

    private func connectionLabel(_ state: MicrophoneSharingSnapshot, role: MicrophoneRole) -> String {
        guard state.settings.enabled else { return "已关闭" }
        if state.phase == "streaming" { return "传输中" }
        if isReady(state, role: role) { return "按需连接" }
        switch role {
        case .provider:
            if state.native.permission != "authorized" { return "需要权限" }
            if peers.isEmpty { return "等待配对" }
            if onlinePeerCount == 0 { return "等待上线" }
        case .receiver:
            if state.settings.source.isEmpty { return "未选择来源" }
            if !model.driverInstalled { return "未安装驱动" }
            if !state.native.driver { return "驱动未响应" }
        }
        if state.phase == "waiting" { return "连接中" }
        return "未连接"
    }

    private func usageSection(_ state: MicrophoneSharingSnapshot, role: MicrophoneRole) -> some View {
        HStack(alignment: .top, spacing: DFSpace.m) {
            Image(systemName: usageSymbol(state, role: role))
                .font(DFType.title)
                .foregroundStyle(usageTone(state, role: role).color)
                .frame(width: 24, height: 20)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: DFSpace.xs) {
                Text(usageTitle(state, role: role))
                    .font(DFType.title)
                    .lineLimit(1)
                // 固定两行占位：不同状态文案长短不一，也保持区块高度不变。
                Text(usageDetail(state, role: role))
                    .font(DFType.detail).dfSecondaryText()
                    .lineLimit(2, reservesSpace: true)
            }
            Spacer(minLength: DFSpace.s)
            Group {
                if state.phase == "streaming" {
                    MicrophoneLevelView(level: state.native.level)
                        .frame(width: 72, height: 24)
                        .accessibilityLabel("麦克风电平")
                        .accessibilityValue("\(Int(state.native.level * 100))%")
                }
            }
            .frame(minWidth: 72, minHeight: 28)
        }
    }

    private func usageSymbol(_ state: MicrophoneSharingSnapshot, role: MicrophoneRole) -> String {
        if state.phase == "streaming" { return "waveform" }
        if hasIssue(state, role: role) { return "pause.circle" }
        if isReady(state, role: role) { return "moon.zzz" }
        return "mic"
    }

    /// 使用状态区只表达运行状态；异常由对应节点或下方提示展示，
    /// 这里不出现告警色和操作按钮，保证状态卡高度与视觉稳定。
    private func usageTone(_ state: MicrophoneSharingSnapshot, role: MicrophoneRole) -> DFTone {
        state.settings.enabled && state.phase == "streaming" ? .success : .neutral
    }

    /// 需要用户处理才能继续使用的异常：权限、来源、驱动或连接失败。
    private func hasIssue(_ state: MicrophoneSharingSnapshot, role: MicrophoneRole) -> Bool {
        guard state.settings.enabled, state.phase != "streaming" else { return false }
        if state.phase == "failed" { return true }
        switch role {
        case .provider: return state.native.permission != "authorized"
        case .receiver: return state.settings.source.isEmpty || !model.driverInstalled || !state.native.driver
        }
    }

    private func usageTitle(_ state: MicrophoneSharingSnapshot, role: MicrophoneRole) -> String {
        guard state.settings.enabled else { return "共享已关闭" }
        if hasIssue(state, role: role) { return "暂不可用" }
        switch role {
        case .provider:
            if state.phase == "streaming" {
                return "\(activePeerName(state) ?? "另一台 Mac") 正在使用麦克风"
            }
            if peers.isEmpty { return "等待配对设备" }
            if onlinePeerCount == 0 { return "等待已配对设备上线" }
            if isReady(state, role: role) {
                return peers.count == 1
                    ? "已就绪，等待设备使用"
                    : "已就绪，\(peers.count) 台已配对设备可直接使用"
            }
            if state.phase == "waiting" { return "设备正在连接" }
            if state.native.locked { return "已暂停共享" }
            return "等待已配对设备使用"
        case .receiver:
            if state.phase == "streaming" {
                let names = Array(Set(model.readerApps.map(\.name))).sorted()
                if !names.isEmpty { return names.joined(separator: "、") + "正在使用" }
                return "正在使用远端麦克风"
            }
            if isReady(state, role: role) { return "已就绪，等待语音应用使用" }
            if !sourcePeerConnected(state) { return "来源设备离线" }
            if state.phase == "waiting" { return "正在连接来源设备" }
            if state.native.locked { return "已暂停共享" }
            return "等待来源设备"
        }
    }

    private func usageDetail(_ state: MicrophoneSharingSnapshot, role: MicrophoneRole) -> String {
        guard state.settings.enabled else {
            return role == .provider
                ? "开启后，仅在设备使用麦克风时采集和传输。"
                : "开启后，仅在语音应用读取虚拟麦克风时连接来源设备。"
        }
        if hasIssue(state, role: role) {
            return role == .receiver
                ? "需要处理的问题已列在右侧设置步骤中，处理后自动恢复。"
                : "需要处理的问题已列在下方，处理后自动恢复。"
        }
        switch role {
        case .provider:
            if state.phase == "streaming" {
                return peers.count > 1
                    ? "应用停止读取后自动结束。其他已配对设备需等本次使用结束。"
                    : "应用停止读取后自动结束采集与传输。"
            }
            if peers.isEmpty {
                return "先在“设备”中配对另一台电脑；配对后可直接使用，不需要逐台授权。"
            }
            if onlinePeerCount == 0 {
                return "已配对的电脑上线后即可直接使用，不需要逐台授权。"
            }
            if isReady(state, role: role) {
                return peers.count > 1
                    ? "当前没有采集或传输音频。\(peers.count) 台已配对设备中，先开始的设备会建立连接。"
                    : "当前没有采集或传输音频。设备开始使用麦克风时自动连接。"
            }
            return statusText(state, role: role)
        case .receiver:
            if model.readerApps.contains(where: \.isSoundSettings) {
                return "系统声音设置正在检测输入电平。离开该页面后，若没有其他应用使用，传输会自动停止。"
            }
            if state.phase == "streaming" { return "语音应用停止读取后自动结束采集与传输。" }
            if isReady(state, role: role) {
                return "当前没有采集或传输音频。语音应用开始录音时自动连接。"
            }
            return statusText(state, role: role)
        }
    }

    private func statusText(_ state: MicrophoneSharingSnapshot, role: MicrophoneRole) -> String {
        guard state.settings.enabled else { return "共享已关闭" }
        switch role {
        case .provider:
            if state.native.permission != "authorized" {
                return state.native.permission == "denied" ? "未允许麦克风访问" : "需要允许麦克风访问"
            }
            if peers.isEmpty { return "配对电脑后即可直接使用此麦克风" }
        case .receiver:
            if state.settings.source.isEmpty { return "选择提供麦克风的 Mac" }
            if !model.driverInstalled { return "安装虚拟麦克风后即可使用" }
            if !state.native.driver { return "驱动服务未响应，重启音频服务后可重新连接" }
            if !sourcePeerConnected(state) { return "来源设备离线，等待重新连接" }
        }
        return MicrophoneErrorText.friendly(state.detail)
    }

    // MARK: - 节点设置

    private func inputPicker(_ state: MicrophoneSharingSnapshot) -> some View {
        Picker("输入设备", selection: Binding(
            get: { state.settings.input_uid },
            set: { value in update { $0.input_uid = value } }
        )) {
            Text("系统默认麦克风").tag("")
            ForEach(model.inputs) { input in Text(input.name).tag(input.id) }
            if !state.settings.input_uid.isEmpty && !model.inputs.contains(where: { $0.id == state.settings.input_uid }) {
                Text("所选麦克风不可用").tag(state.settings.input_uid)
            }
        }
        .pickerStyle(.inline)
        .onAppear { model.refreshInputs() }
    }

    @ViewBuilder private func inputPermission(_ state: MicrophoneSharingSnapshot) -> some View {
        if state.native.permission != "authorized" {
            DFNotice(tone: state.native.permission == "denied" ? .warning : .accent,
                     title: state.native.permission == "denied" ? "麦克风权限已关闭" : "需要允许麦克风访问",
                     message: "macOS 会询问是否允许 DeskFuse 读取实体麦克风。",
                     actionTitle: state.native.permission == "denied" ? "打开系统设置…" : "允许访问…",
                     action: model.requestPermission)
        }
    }

    private let sourceDeviceRowHeight: CGFloat = 52

    private func sourcePicker(_ state: MicrophoneSharingSnapshot) -> some View {
        VStack(alignment: .leading, spacing: DFSpace.m) {
            Text("选择提供麦克风的 Mac。已配对设备可直接使用。")
                .font(DFType.detail).dfSecondaryText()
            if peers.isEmpty {
                MicrophoneEmptyRow(symbol: "laptopcomputer",
                                   title: "还没有已配对的电脑",
                                   message: "先在“设备”中完成配对；之后无需额外授权。",
                                   actionTitle: "前往设备") { store.selectedPage = .devices }
            } else {
                let ordered = orderedPeers(state)
                ScrollView(.vertical) {
                    VStack(spacing: 0) {
                        ForEach(ordered, id: \.id) { peer in
                            receiverPeerRow(peer, state: state)
                                .frame(height: sourceDeviceRowHeight)
                                .overlay(alignment: .bottom) {
                                    if peer.id != ordered.last?.id {
                                        DFDivider(inset: 30)
                                    }
                                }
                        }
                    }
                }
                .frame(height: sourceDeviceRowHeight * CGFloat(min(ordered.count, 3)))
                .scrollDisabled(ordered.count <= 3)
            }
        }
        .frame(width: 320)
        .disabled(model.saving || model.managingDriver)
    }

    private func receiverPeerRow(_ peer: Device, state: MicrophoneSharingSnapshot) -> some View {
        let selected = state.settings.source == peer.id
        return Button {
            guard !selected else { return }
            update { $0.source = peer.id }
        } label: {
            HStack(spacing: DFSpace.m) {
                Image(systemName: "laptopcomputer")
                    .font(DFType.icon(.regular))
                    .foregroundStyle(selected ? DFPalette.accent : Color.secondary)
                    .frame(width: 18)
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: DFSpace.xxs) {
                    Text(peer.name).font(selected ? DFType.bodyMedium : DFType.body)
                        .lineLimit(1).truncationMode(.middle)
                    Text(peer.connected ? "在线" : "离线")
                        .font(DFType.detail).dfSecondaryText()
                }
                Spacer(minLength: DFSpace.s)
                Image(systemName: selected ? "checkmark.circle.fill" : "circle")
                    .font(DFType.icon(.medium))
                    .foregroundStyle(selected ? DFPalette.accent : Color.secondary.opacity(0.5))
                    .contentTransition(.symbolEffect(.replace))
                    .animation(DFMotion.snappy, value: selected)
                    .accessibilityHidden(true)
            }
            .padding(.vertical, DFSpace.s)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel("\(peer.name)，\(peer.connected ? "在线" : "离线")")
        .accessibilityAddTraits(selected ? .isSelected : [])
    }

    private func driverControls(_ state: MicrophoneSharingSnapshot) -> some View {
        MicrophoneDriverControls(
            installed: model.driverInstalled,
            loaded: state.native.driver,
            legacyInstalled: model.legacyDriverInstalled,
            defaultInput: model.isDefaultInput == true,
            operationTitle: model.managingDriver
                ? (model.driverOperation == .restartAudio ? "正在重启音频服务…" : "正在处理驱动…") : nil,
            install: { driverAction = .install },
            restart: { driverAction = .restartAudio },
            setDefault: model.setDefaultInput
        )
    }

    private var driverMaintenance: some View {
        DFDisclosure("驱动维护", symbol: "wrench.and.screwdriver") {
            VStack(spacing: DFSpace.m) {
                if model.driverInstalled {
                    DFSettingRow(title: "音频服务", detail: "连接异常时先重启，保留驱动与共享设置。") {
                        Button("重启…") { driverAction = .restartAudio }
                    }
                    Divider()
                    DFSettingRow(title: "麦克风驱动", detail: "重启后仍不可用时，可尝试重新安装。") {
                        Button("重新安装…") { driverAction = .install }
                    }
                    Divider()
                }
                DFSettingRow(title: "移除驱动", detail: "移除后，此 Mac 将无法接收远端麦克风。") {
                    Button(model.legacyDriverInstalled ? "卸载驱动（含旧版）…" : "卸载…",
                           role: .destructive) { driverAction = .uninstall }
                }
            }
        }
    }

    // MARK: - 完成与提示

    @ViewBuilder private func completionNotice(_ state: MicrophoneSharingSnapshot, role: MicrophoneRole) -> some View {
        if !state.settings.enabled && canEnable(state, role: role)
            && (role == .provider || model.isDefaultInput == true) {
            DFNotice(tone: .accent, title: "设置已完成",
                     message: role == .provider
                        ? "打开顶部开关后，所有已配对电脑都可在需要时直接连接。"
                        : "打开顶部开关后，语音应用即可使用远端麦克风。",
                     actionTitle: "开启共享", action: { model.setFeatureEnabled(true) })
        }
    }

    private var privacyNote: some View {
        HStack(alignment: .top, spacing: DFSpace.s) {
            Image(systemName: "lock.shield")
                .font(DFType.icon(.small))
                .dfSecondaryText()
                .accessibilityHidden(true)
            Text("仅可信的已配对设备可以使用；DeskFuse 不保存或转写音频，只在应用读取麦克风时采集和传输。")
                .font(DFType.caption)
                .dfSecondaryText()
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.horizontal, DFSpace.xs)
    }

    // MARK: - 状态辅助

    private func canEnable(_ state: MicrophoneSharingSnapshot, role: MicrophoneRole) -> Bool {
        switch role {
        case .provider: state.native.permission == "authorized" && !peers.isEmpty
        case .receiver: !state.settings.source.isEmpty && model.driverInstalled && state.native.driver
        }
    }

    private func isReady(_ state: MicrophoneSharingSnapshot, role: MicrophoneRole) -> Bool {
        guard state.settings.enabled, !state.native.locked, state.phase == "standby" else { return false }
        switch role {
        case .provider:
            return state.native.permission == "authorized" && onlinePeerCount > 0
        case .receiver:
            return model.driverInstalled && state.native.driver && sourcePeerConnected(state)
        }
    }

    private func connectionErrorHandled(_ state: MicrophoneSharingSnapshot, role: MicrophoneRole) -> Bool {
        role == .receiver && model.driverInstalled && !state.native.driver
    }

    private var onlinePeerCount: Int {
        peers.filter(\.connected).count
    }

    private func sourcePeerConnected(_ state: MicrophoneSharingSnapshot) -> Bool {
        peers.contains { $0.id == state.settings.source && $0.connected }
    }

    private func activePeerName(_ state: MicrophoneSharingSnapshot) -> String? {
        guard !state.peer.isEmpty else { return nil }
        return peers.first(where: { $0.id == state.peer })?.name ?? "已连接设备"
    }

    private func sourcePeerName(_ state: MicrophoneSharingSnapshot) -> String {
        guard !state.settings.source.isEmpty else { return "选择来源设备" }
        return peers.first(where: { $0.id == state.settings.source })?.name ?? "原来源设备不可用"
    }

    private func inputDisplayName(_ state: MicrophoneSharingSnapshot) -> String {
        guard !state.settings.input_uid.isEmpty else { return "系统默认麦克风" }
        return model.inputs.first(where: { $0.id == state.settings.input_uid })?.name ?? "所选麦克风不可用"
    }

    private func orderedPeers(_ state: MicrophoneSharingSnapshot) -> [Device] {
        peers.sorted { lhs, rhs in
            let lhsActive = lhs.id == state.peer
            let rhsActive = rhs.id == state.peer
            if lhsActive != rhsActive { return lhsActive }
            if lhs.connected != rhs.connected { return lhs.connected }
            return lhs.name.localizedStandardCompare(rhs.name) == .orderedAscending
        }
    }
}

// MARK: - 身份选择卡片

private enum MicrophoneRole: String {
    case provider
    case receiver

    var title: String {
        switch self {
        case .provider: "我提供麦克风"
        case .receiver: "我使用其他设备的麦克风"
        }
    }

    var symbol: String {
        switch self {
        case .provider: "mic.fill"
        case .receiver: "laptopcomputer"
        }
    }

    var identityDetail: String {
        switch self {
        case .provider: "把此 Mac 连接的麦克风分享给已配对的电脑。可信设备可直接使用，不需要逐台授权。"
        case .receiver: "使用另一台 Mac 的麦克风，在本机听写、通话或录音。"
        }
    }

    var identityFootnote: String {
        switch self {
        case .provider: "无需逐台授权"
        case .receiver: "需要安装虚拟麦克风驱动"
        }
    }

    var footnoteSymbol: String {
        switch self {
        case .provider: "checkmark.circle"
        case .receiver: "arrow.down.circle"
        }
    }

    var footnoteTone: DFTone {
        switch self {
        case .provider: .success
        case .receiver: .warning
        }
    }
}

private struct MicrophoneIdentityCard: View {
    let role: MicrophoneRole
    let action: () -> Void
    @Environment(\.colorScheme) private var scheme
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            VStack(alignment: .leading, spacing: DFSpace.m) {
                HStack(alignment: .top) {
                    Image(systemName: role.symbol)
                        .font(DFType.icon(.large, weight: .regular))
                        .foregroundStyle(hovering ? Color.white : DFPalette.accent)
                        .symbolEffect(.bounce, value: hovering)
                        .frame(width: 48, height: 48)
                        .background(hovering ? DFPalette.accent : DFPalette.control(scheme),
                                    in: RoundedRectangle(cornerRadius: DFRadius.card, style: .continuous))
                        .accessibilityHidden(true)
                    Spacer(minLength: DFSpace.s)
                    Image(systemName: "arrow.right")
                        .font(DFType.icon(.small, weight: .semibold))
                        .foregroundStyle(hovering ? DFPalette.accent : Color.secondary)
                        .offset(x: hovering ? DFSpace.xs : 0)
                        .accessibilityHidden(true)
                }
                VStack(alignment: .leading, spacing: DFSpace.xs) {
                    Text(role.title).font(DFType.title)
                    Text(role.identityDetail)
                        .font(DFType.detail).dfSecondaryText()
                        .fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: DFSpace.s)
                Label(role.identityFootnote, systemImage: role.footnoteSymbol)
                    .font(DFType.caption.weight(.medium))
                    .foregroundStyle(role.footnoteTone.color)
            }
            .padding(DFSpace.xl)
            .frame(maxWidth: .infinity, minHeight: 196, alignment: .topLeading)
            .background(DFPalette.surface(scheme),
                        in: RoundedRectangle(cornerRadius: DFRadius.card, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: DFRadius.card, style: .continuous)
                    .strokeBorder(hovering ? DFPalette.accent.opacity(DFOpacity.strong) : DFPalette.line(scheme),
                                  lineWidth: hovering ? DFMetrics.selectionStroke : DFMetrics.hairline)
            }
            .contentShape(RoundedRectangle(cornerRadius: DFRadius.card, style: .continuous))
            .shadow(color: hovering ? DFShadow.hoverColor : .clear,
                    radius: DFShadow.hoverRadius, y: DFShadow.hoverY)
            .offset(y: hovering ? -DFSpace.xxs : 0)
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .animation(DFMotion.snappy, value: hovering)
        .accessibilityLabel(role.title)
        .accessibilityHint(role.identityDetail)
    }
}

// MARK: - 加载、故障与状态

private struct MicrophoneLoadingCard: View {
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        HStack(spacing: DFSpace.m) {
            Image(systemName: "mic.fill")
                .font(DFType.icon(.medium, weight: .regular))
                .foregroundStyle(DFPalette.accent)
                .frame(width: 48, height: 48)
                .background(DFPalette.control(scheme),
                            in: RoundedRectangle(cornerRadius: DFRadius.control, style: .continuous))
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: DFSpace.xxs) {
                Text("正在准备麦克风共享").font(DFType.title)
                Text("读取系统输入、已配对设备和驱动状态，首次打开可能需要几秒钟。")
                    .font(DFType.detail).dfSecondaryText()
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: DFSpace.s)
            ProgressView()
                .controlSize(.small)
                .accessibilityLabel("正在加载")
        }
        .padding(DFSpace.xl)
        .dfCard()
        .accessibilityElement(children: .combine)
    }
}

private struct MicrophoneNoticeCard: View {
    let symbol: String
    let tone: DFTone
    let title: String
    let message: String
    var detail: String? = nil
    var primaryTitle: String? = nil
    var primaryAction: (() -> Void)? = nil
    var onDismiss: (() -> Void)? = nil
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        VStack(alignment: .leading, spacing: DFSpace.m) {
            HStack(alignment: .top, spacing: DFSpace.m) {
                Image(systemName: symbol)
                    .font(DFType.icon(.medium, weight: .regular))
                    .foregroundStyle(tone.color)
                    .frame(width: 36, height: 36)
                    .background(tone.color.opacity(DFOpacity.faint),
                                in: RoundedRectangle(cornerRadius: DFRadius.control, style: .continuous))
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: DFSpace.xxs) {
                    Text(title).font(DFType.title)
                    Text(message)
                        .font(DFType.detail).dfSecondaryText()
                        .fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: DFSpace.s)
                if let onDismiss {
                    Button(action: onDismiss) { Image(systemName: "xmark") }
                        .buttonStyle(.dfIcon)
                        .accessibilityLabel("关闭提示")
                }
            }
            if let detail {
                DFDisclosure("技术详情", symbol: "terminal") {
                    Text(detail)
                        .font(DFType.mono)
                        .dfSecondaryText()
                        .textSelection(.enabled)
                        .fixedSize(horizontal: false, vertical: true)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            if let primaryTitle, let primaryAction {
                HStack {
                    Spacer()
                    Button(primaryTitle, action: primaryAction).buttonStyle(.dfPrimary)
                }
            }
        }
        .padding(DFSpace.xl)
        .dfCard()
    }
}

private struct MicrophoneEmptyRow: View {
    let symbol: String
    let title: String
    let message: String
    let actionTitle: String
    let action: () -> Void
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        HStack(spacing: DFSpace.m) {
            Image(systemName: symbol)
                .font(DFType.icon(.medium, weight: .regular))
                .dfSecondaryText()
                .frame(width: 32, height: 32)
                .background(DFPalette.control(scheme),
                            in: RoundedRectangle(cornerRadius: DFRadius.control, style: .continuous))
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: DFSpace.xxs) {
                Text(title).font(DFType.bodyMedium)
                Text(message)
                    .font(DFType.detail).dfSecondaryText()
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: DFSpace.s)
            Button(actionTitle, action: action).buttonStyle(.dfSecondary)
        }
        .padding(DFSpace.m)
        .background(DFPalette.control(scheme),
                    in: RoundedRectangle(cornerRadius: DFRadius.control, style: .continuous))
    }
}

private enum MicrophoneErrorText {
    static func friendly(_ raw: String?) -> String {
        guard let text = raw?.trimmingCharacters(in: .whitespacesAndNewlines), !text.isEmpty else {
            return "麦克风服务暂时不可用，请稍后重试。"
        }
        let lower = text.lowercased()
        if lower.contains("deadline") || lower.contains("timed out") || lower.contains("timeout") {
            return "麦克风服务响应超时，请稍后重试。"
        }
        if lower.contains("connection refused") || lower.contains("econnrefused") {
            return "无法连接麦克风服务，请确认后台服务正在运行。"
        }
        if lower.contains("connection reset") || lower.contains("econnreset") || lower.contains("broken pipe") {
            return "麦克风服务连接已断开，请重试。"
        }
        if lower.contains("not installed") {
            return "虚拟麦克风驱动尚未安装。"
        }
        if lower.contains("permission") || lower.contains("denied") {
            return "麦克风权限不可用，请在系统设置中检查。"
        }
        if text.unicodeScalars.contains(where: { $0.value >= 0x4E00 && $0.value <= 0x9FFF }) {
            return text
        }
        return "麦克风服务返回了错误，请重试。"
    }

    static func detail(_ raw: String?) -> String? {
        guard let text = raw?.trimmingCharacters(in: .whitespacesAndNewlines), !text.isEmpty else { return nil }
        return text == friendly(text) ? nil : text
    }
}

/// A compact level indicator, not a progress bar. It only reflects real samples.
struct MicrophoneLevelView: View {
    let level: Float
    var body: some View {
        HStack(spacing: 3) {
            ForEach(0..<12) { index in
                Capsule().fill(Double(index) / 12 < min(1, sqrt(Double(max(0, level)))) ? DFPalette.success : Color.primary.opacity(0.08))
                    .frame(width: 3, height: CGFloat(8 + (index % 4) * 4))
            }
        }
        .animation(.easeOut(duration: 0.12), value: level)
        .accessibilityElement(children: .ignore)
    }
}

private struct MicrophoneDriverSheet: View {
    let action: MicrophoneDriverManager.Action
    let confirm: (Bool) -> Void
    @Environment(\.dismiss) private var dismiss
    // 新驱动只有在 coreaudiod 重新加载 HAL 插件后才会监听控制端口；
    // 安装默认立即重启，否则文件已就位但服务不可达，界面会显示“驱动未响应”。
    @State private var restartAudio: Bool

    init(action: MicrophoneDriverManager.Action, confirm: @escaping (Bool) -> Void) {
        self.action = action
        self.confirm = confirm
        _restartAudio = State(initialValue: action == .install)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: DFSpace.l) {
            Text(action.title).font(DFType.pageTitle)
            if action == .restartAudio {
                Text("这会短暂中断此 Mac 上所有应用的录音和播放，需要管理员授权。")
                Text("保留已安装的驱动和当前共享设置。服务恢复后按需连接，语音应用可能需要重新开始录音。")
                    .font(DFType.detail).dfSecondaryText()
            } else {
                Text("仅操作 DeskFuse 的麦克风驱动（包括旧版名称遗留的驱动），需要在系统弹窗中授权管理员权限。操作前会关闭麦克风共享。")
                    .fixedSize(horizontal: false, vertical: true)
                Toggle("完成后立即重启音频服务", isOn: $restartAudio).font(DFType.body)
                Text(restartAudio ? "这会短暂中断此 Mac 上所有应用的录音和播放。" : "稍后重启 Mac 后生效。")
                    .font(DFType.detail).dfSecondaryText()
            }
            HStack(spacing: DFSpace.s) {
                Spacer()
                Button("取消") { dismiss() }.keyboardShortcut(.cancelAction)
                Button(action.title, role: action == .uninstall ? .destructive : nil) {
                    confirm(action == .restartAudio || restartAudio)
                }
            }
        }
        .padding(DFSpace.xl)
        .frame(width: 460)
    }
}
