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
        DFScrollPage {
            VStack(alignment: .leading, spacing: DFSpace.xl) {
                if let state = model.snapshot {
                    let role = resolvedRole(state)
                    if role == nil || editingRole {
                        identityPicker(current: role, state: state)
                    } else if let role {
                        roleStatus(state, role: role)
                        setupSteps(state, role: role)
                        completionNotice(state, role: role)
                        if role == .receiver, let message = model.driverMessage {
                            DFNotice(tone: .neutral, title: "驱动操作", message: message,
                                     onDismiss: { model.driverMessage = nil })
                        }
                        if let connectionError = model.connectionError,
                           !connectionErrorHandled(state, role: role) {
                            DFNotice(tone: .warning, title: "后台连接暂时异常",
                                     message: MicrophoneErrorText.friendly(connectionError))
                        }
                        privacyNote
                    }
                } else if let connectionError = model.connectionError {
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
                } else {
                    MicrophoneLoadingCard()
                    privacyNote
                }
                if let error = model.error {
                    MicrophoneNoticeCard(
                        symbol: "exclamationmark.triangle.fill",
                        tone: .danger,
                        title: "操作未完成",
                        message: MicrophoneErrorText.friendly(error),
                        detail: MicrophoneErrorText.detail(error),
                        onDismiss: { model.error = nil }
                    )
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .disabled(model.saving || model.managingDriver)
        .onAppear {
            model.ensureStarted(engine: store.engine)
            model.refreshInputs()
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
        settings.enabled = false
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
                sharingToggle(state, role: role)
            }
            .padding(DFSpace.xl)
            Divider()
            routePath(state, role: role).padding(DFSpace.xl)
            if state.settings.enabled && canEnable(state, role: role) {
                Divider()
                usageSection(state, role: role).padding(DFSpace.xl)
            }
        }
        .dfCard()
    }

    private func sharingToggle(_ state: MicrophoneSharingSnapshot, role: MicrophoneRole) -> some View {
        HStack(spacing: DFSpace.s) {
            if model.saving { ProgressView().controlSize(.small) }
            Toggle("麦克风共享", isOn: Binding(
                get: { state.settings.enabled },
                set: { value in
                    guard !value || canEnable(state, role: role) else { return }
                    update { $0.enabled = value }
                }
            ))
            .labelsHidden()
            .toggleStyle(.switch)
            .controlSize(.regular)
            .disabled(!state.settings.enabled && !canEnable(state, role: role))
            .help(state.settings.enabled ? "关闭麦克风共享"
                  : (canEnable(state, role: role) ? "开启麦克风共享" : "完成下方设置后可开启"))
            .accessibilityLabel("麦克风共享")
        }
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
            guard state.settings.enabled else { return "来源：\(name) · 设置已完成，打开开关即可使用" }
            if model.isDefaultInput == true { return "来源：\(name) · 已是系统默认输入" }
            return "来源：\(name) · 等待语音应用使用"
        }
    }

    private func routePath(_ state: MicrophoneSharingSnapshot, role: MicrophoneRole) -> some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: DFSpace.l) {
                routeSource(state, role: role)
                routeConnection(state, role: role)
                routeDestination(state, role: role)
            }
            VStack(spacing: DFSpace.l) {
                routeSource(state, role: role)
                routeConnection(state, role: role)
                routeDestination(state, role: role)
            }
        }
    }

    private func routeSource(_ state: MicrophoneSharingSnapshot, role: MicrophoneRole) -> some View {
        switch role {
        case .provider:
            return endpoint(symbol: "mic.fill", title: inputDisplayName(state), subtitle: "此 Mac 的麦克风")
        case .receiver:
            return endpoint(symbol: "laptopcomputer", title: sourcePeerName(state), subtitle: "声音来源")
        }
    }

    private func routeDestination(_ state: MicrophoneSharingSnapshot, role: MicrophoneRole) -> some View {
        switch role {
        case .provider:
            let count = peers.count
            let active = activePeerName(state)
            let title = active
                ?? (count == 0 ? "等待配对设备" : (count == 1 ? "1 台已配对电脑" : "\(count) 台已配对电脑"))
            let subtitle: String
            if active != nil {
                subtitle = state.phase == "streaming" ? "正在使用 · 共 \(count) 台已配对" : "已连接 · 共 \(count) 台已配对"
            } else if count == 0 {
                subtitle = "先在设备中配对"
            } else if !state.settings.enabled {
                subtitle = "已配对 · 共享未开启"
            } else {
                subtitle = onlinePeerCount > 0 ? "无需授权 · 按需连接" : "等待设备上线"
            }
            return endpoint(symbol: "laptopcomputer", title: title, subtitle: subtitle)
        case .receiver:
            return endpoint(symbol: "desktopcomputer",
                            title: model.driverInstalled ? "DeskFuse 麦克风" : "虚拟麦克风",
                            subtitle: model.driverInstalled ? "此 Mac 的虚拟输入" : "需要安装驱动")
        }
    }

    private func endpoint(symbol: String, title: String, subtitle: String) -> some View {
        HStack(spacing: DFSpace.m) {
            Image(systemName: symbol).font(DFType.pageTitle.weight(.regular))
                .frame(width: 48, height: 48)
                .background(DFPalette.control(scheme), in: RoundedRectangle(cornerRadius: DFRadius.card))
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: DFSpace.xxs) {
                Text(subtitle).font(DFType.detail).dfSecondaryText()
                Text(title).font(DFType.title).lineLimit(2)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .frame(minWidth: 170, maxWidth: .infinity, alignment: .leading)
    }

    private func routeConnection(_ state: MicrophoneSharingSnapshot, role: MicrophoneRole) -> some View {
        let tone = connectionTone(state, role: role)
        return VStack(spacing: DFSpace.s) {
            Text(connectionLabel(state, role: role))
                .font(DFType.caption.weight(.medium))
                .foregroundStyle(tone.color)
                .padding(.horizontal, DFSpace.s + 2)
                .padding(.vertical, DFSpace.xs)
                .background(tone.color.opacity(DFOpacity.soft), in: Capsule())
            HStack(spacing: 0) {
                Rectangle().fill(DFPalette.line(scheme)).frame(height: DFMetrics.hairline)
                Image(systemName: "chevron.right")
                    .font(DFType.caption.weight(.semibold))
                    .foregroundStyle(.tertiary)
            }
            Text("音频不保存").font(DFType.caption).dfSecondaryText()
        }
        .frame(minWidth: 120, maxWidth: 180)
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
                Text(usageTitle(state, role: role)).font(DFType.title)
                Text(usageDetail(state, role: role))
                    .font(DFType.detail).dfSecondaryText()
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: DFSpace.s)
            if state.phase == "streaming" {
                MicrophoneLevelView(level: state.native.level)
                    .frame(width: 72, height: 24)
                    .accessibilityLabel("麦克风电平")
                    .accessibilityValue("\(Int(state.native.level * 100))%")
            } else if role == .receiver && !state.native.driver && model.driverInstalled {
                Button("重启音频服务…") { driverAction = .restartAudio }
            }
        }
    }

    private func usageSymbol(_ state: MicrophoneSharingSnapshot, role: MicrophoneRole) -> String {
        if state.phase == "streaming" { return "waveform" }
        if usageTone(state, role: role) == .warning { return "exclamationmark.triangle" }
        if isReady(state, role: role) { return "moon.zzz" }
        return "mic"
    }

    private func usageTone(_ state: MicrophoneSharingSnapshot, role: MicrophoneRole) -> DFTone {
        guard state.settings.enabled else { return .neutral }
        if state.phase == "streaming" { return .success }
        if state.phase == "failed" { return .warning }
        if role == .provider && state.native.permission != "authorized" { return .warning }
        if role == .receiver && (!model.driverInstalled || !state.native.driver) { return .warning }
        return .neutral
    }

    private func usageTitle(_ state: MicrophoneSharingSnapshot, role: MicrophoneRole) -> String {
        guard state.settings.enabled else { return "共享已关闭" }
        switch role {
        case .provider:
            if state.phase == "streaming" {
                return "\(activePeerName(state) ?? "另一台 Mac") 正在使用麦克风"
            }
            if state.native.permission != "authorized" {
                return state.native.permission == "denied" ? "麦克风权限已关闭" : "需要麦克风权限"
            }
            if peers.isEmpty { return "等待配对设备" }
            if onlinePeerCount == 0 { return "等待已配对设备上线" }
            if isReady(state, role: role) {
                return peers.count == 1
                    ? "已就绪，等待设备使用"
                    : "已就绪，\(peers.count) 台已配对设备可直接使用"
            }
            if state.phase == "waiting" { return "设备正在连接" }
            if state.phase == "failed" { return "连接暂时中断" }
            if state.native.locked { return "已暂停共享" }
            return "等待已配对设备使用"
        case .receiver:
            if state.settings.source.isEmpty { return "尚未选择来源设备" }
            if !model.driverInstalled { return "需要安装虚拟麦克风" }
            if !state.native.driver { return "驱动服务未响应" }
            if state.phase == "streaming" {
                let names = Array(Set(model.readerApps.map(\.name))).sorted()
                if !names.isEmpty { return names.joined(separator: "、") + "正在使用" }
                return "正在使用远端麦克风"
            }
            if isReady(state, role: role) { return "已就绪，等待语音应用使用" }
            if !sourcePeerConnected(state) { return "来源设备离线" }
            if state.phase == "waiting" { return "正在连接来源设备" }
            if state.phase == "failed" { return "连接暂时中断" }
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
        switch role {
        case .provider:
            if state.phase == "streaming" {
                return peers.count > 1
                    ? "应用停止读取后自动结束。其他已配对设备需等本次使用结束。"
                    : "应用停止读取后自动结束采集与传输。"
            }
            if state.native.permission != "authorized" {
                return "macOS 需要授权 DeskFuse 读取实体麦克风。"
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
            if state.settings.source.isEmpty { return "先选择另一台已配对的 Mac。" }
            if !model.driverInstalled { return "安装虚拟麦克风后，语音应用才能接收远端声音。" }
            if !state.native.driver { return "驱动已安装但服务没有响应；可重启音频服务或重新安装驱动。" }
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

    // MARK: - 设置步骤

    @ViewBuilder private func setupSteps(_ state: MicrophoneSharingSnapshot, role: MicrophoneRole) -> some View {
        VStack(alignment: .leading, spacing: DFSpace.m) {
            DFSectionLabel(role == .provider ? "麦克风设置" : "设置步骤")
            switch role {
            case .provider:
                inputStep(state)
                providerDevices(state)
            case .receiver:
                sourceStep(state)
                driverStep(state)
            }
        }
    }

    private func inputStep(_ state: MicrophoneSharingSnapshot) -> some View {
        MicrophoneStepCard(index: 1, title: "选择麦克风",
                           detail: "仅使用此 Mac 的实体输入，不会采集 DeskFuse 虚拟设备。",
                           complete: state.native.permission == "authorized") {
            DFSettingRow(title: "输入设备", detail: "系统默认会跟随 macOS 当前选择。", symbol: "mic") {
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
                .labelsHidden()
                .frame(maxWidth: 240)
            }
            if state.native.permission != "authorized" {
                DFNotice(tone: state.native.permission == "denied" ? .warning : .accent,
                         title: state.native.permission == "denied" ? "麦克风权限已关闭" : "需要允许麦克风访问",
                         message: "macOS 会询问是否允许 DeskFuse 读取实体麦克风。",
                         actionTitle: state.native.permission == "denied" ? "打开系统设置…" : "允许访问…",
                         action: model.requestPermission)
            }
        }
    }

    private func providerDevices(_ state: MicrophoneSharingSnapshot) -> some View {
        DFSection("可直接使用", footer: "可信的已配对电脑不需要逐台授权；同一时间只服务一台设备，其他设备会收到占用中状态。") {
            if peers.isEmpty {
                MicrophoneEmptyRow(symbol: "laptopcomputer",
                                   title: "还没有已配对的电脑",
                                   message: "先在“设备”中完成配对；之后无需授权即可使用此麦克风。",
                                   actionTitle: "前往设备") { store.selectedPage = .devices }
            } else {
                let ordered = orderedPeers(state)
                VStack(spacing: 0) {
                    ForEach(ordered, id: \.id) { peer in
                        providerDeviceRow(peer, state: state)
                        if peer.id != ordered.last?.id {
                            DFDivider(inset: 30)
                        }
                    }
                }
            }
        }
    }

    private func providerDeviceRow(_ peer: Device, state: MicrophoneSharingSnapshot) -> some View {
        HStack(spacing: DFSpace.m) {
            Image(systemName: "laptopcomputer")
                .font(DFType.icon(.regular))
                .foregroundStyle(state.peer == peer.id ? DFPalette.accent : Color.secondary)
                .frame(width: 18)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: DFSpace.xxs) {
                HStack(spacing: DFSpace.s) {
                    Text(peer.name).font(DFType.body)
                        .lineLimit(1).truncationMode(.middle)
                    if state.peer == peer.id { DFBadge("使用中", tone: .success) }
                }
                Text(providerDeviceDetail(peer, state: state))
                    .font(DFType.detail).dfSecondaryText()
            }
            Spacer(minLength: DFSpace.s)
        }
        .padding(.vertical, DFSpace.s)
        .accessibilityElement(children: .combine)
    }

    private func providerDeviceDetail(_ peer: Device, state: MicrophoneSharingSnapshot) -> String {
        if state.peer == peer.id { return peer.connected ? "在线 · 正在使用" : "正在使用" }
        if !state.peer.isEmpty && peer.connected { return "在线 · 当前设备结束后可用" }
        return peer.connected ? "在线 · 可直接使用" : "离线 · 上线后可直接使用"
    }

    private func sourceStep(_ state: MicrophoneSharingSnapshot) -> some View {
        MicrophoneStepCard(index: 1, title: "选择来源设备",
                           detail: "选择提供麦克风的 Mac。已配对设备可直接使用。",
                           complete: !state.settings.source.isEmpty) {
            if peers.isEmpty {
                MicrophoneEmptyRow(symbol: "laptopcomputer",
                                   title: "还没有已配对的电脑",
                                   message: "先在“设备”中完成配对；之后无需额外授权。",
                                   actionTitle: "前往设备") { store.selectedPage = .devices }
            } else {
                let ordered = orderedPeers(state)
                VStack(spacing: 0) {
                    ForEach(ordered, id: \.id) { peer in
                        receiverPeerRow(peer, state: state)
                        if peer.id != ordered.last?.id {
                            DFDivider(inset: 30)
                        }
                    }
                }
                if !state.settings.source.isEmpty && !peers.contains(where: { $0.id == state.settings.source }) {
                    DFNotice(tone: .warning, title: "原来源设备不可用",
                             message: "重新配对来源设备，或在上面选择另一台 Mac。")
                }
            }
        }
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
                    .accessibilityHidden(true)
            }
            .padding(.vertical, DFSpace.s)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel("\(peer.name)，\(peer.connected ? "在线" : "离线")")
        .accessibilityAddTraits(selected ? .isSelected : [])
    }

    private func driverStep(_ state: MicrophoneSharingSnapshot) -> some View {
        MicrophoneStepCard(index: 2, title: "安装虚拟麦克风",
                           detail: "安装后，语音软件才会把远端声音识别为本机输入。",
                           complete: model.driverInstalled && state.native.driver) {
            if model.managingDriver {
                HStack(spacing: DFSpace.s) {
                    ProgressView().controlSize(.small)
                    Text(model.driverOperation == .restartAudio
                         ? "正在重启音频服务，请完成系统管理员授权…"
                         : "正在处理驱动，请完成系统管理员授权…")
                        .font(DFType.detail).dfSecondaryText()
                }
            }
            if !model.driverInstalled {
                MicrophoneStatusRow(symbol: "arrow.down.circle", tone: .accent,
                                    title: "需要安装虚拟麦克风",
                                    detail: "安装需要管理员授权；完成后建议立即重启音频服务。",
                                    actionTitle: "安装…") { driverAction = .install }
            } else {
                if state.native.driver {
                    MicrophoneStatusRow(symbol: "checkmark.circle.fill", tone: .success,
                                        title: "驱动已安装并运行",
                                        detail: "语音应用可以从 DeskFuse 麦克风读取远端声音。")
                } else {
                    MicrophoneStatusRow(symbol: "wrench.and.screwdriver", tone: .warning,
                                        title: "驱动服务未响应",
                                        detail: "重启音频服务通常可以恢复；仍不可用时再重新安装驱动。",
                                        actionTitle: "重启音频服务…") { driverAction = .restartAudio }
                }
                Divider()
                if state.native.driver {
                    if model.isDefaultInput == true {
                        Label("DeskFuse 麦克风已是系统默认输入", systemImage: "checkmark.circle.fill")
                            .font(DFType.body)
                            .foregroundStyle(DFPalette.success)
                    } else {
                        DFSettingRow(title: "系统默认输入",
                                     detail: "使用系统默认输入的应用会自动收到远端声音。") {
                            Button("设为默认", action: model.setDefaultInput)
                        }
                    }
                    Text("也可以直接在微信输入法等应用的麦克风设置中选择“DeskFuse 麦克风（来自 \(sourcePeerName(state))）”。")
                        .font(DFType.detail).dfSecondaryText()
                        .fixedSize(horizontal: false, vertical: true)
                } else {
                    Text("驱动服务恢复后，可把 DeskFuse 麦克风设为系统默认输入。")
                        .font(DFType.detail).dfSecondaryText()
                        .fixedSize(horizontal: false, vertical: true)
                }
                DFDisclosure("驱动维护", symbol: "wrench.and.screwdriver") {
                    maintenanceRows
                }
            }
        }
    }

    private var maintenanceRows: some View {
        VStack(spacing: DFSpace.m) {
            DFSettingRow(title: "音频服务", detail: "连接异常时先重启，保留驱动与共享设置。") {
                Button("重启…") { driverAction = .restartAudio }
            }
            Divider()
            DFSettingRow(title: "麦克风驱动", detail: "重启后仍不可用时，可尝试重新安装。") {
                Button("重新安装…") { driverAction = .install }
            }
            Divider()
            DFSettingRow(title: "移除驱动", detail: "移除后，此 Mac 将无法接收远端麦克风。") {
                Button("卸载…") { driverAction = .uninstall }
            }
        }
    }

    // MARK: - 完成与提示

    @ViewBuilder private func completionNotice(_ state: MicrophoneSharingSnapshot, role: MicrophoneRole) -> some View {
        if !state.settings.enabled && canEnable(state, role: role) {
            DFNotice(tone: .accent, title: "设置已完成",
                     message: role == .provider
                        ? "打开顶部开关后，所有已配对电脑都可在需要时直接连接。"
                        : "打开顶部开关后，语音应用即可使用远端麦克风。",
                     actionTitle: "开启共享", action: { update { $0.enabled = true } })
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
                        .foregroundStyle(DFPalette.accent)
                        .frame(width: 48, height: 48)
                        .background(DFPalette.control(scheme),
                                    in: RoundedRectangle(cornerRadius: DFRadius.card, style: .continuous))
                        .accessibilityHidden(true)
                    Spacer(minLength: DFSpace.s)
                    Image(systemName: "arrow.right")
                        .font(DFType.icon(.small, weight: .semibold))
                        .foregroundStyle(hovering ? DFPalette.accent : Color.secondary)
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
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .animation(DFMotion.snappy, value: hovering)
        .accessibilityLabel(role.title)
        .accessibilityHint(role.identityDetail)
    }
}

// MARK: - 步骤卡片

private struct MicrophoneStepCard<Content: View>: View {
    let index: Int
    let title: String
    let detail: String
    let complete: Bool
    @ViewBuilder var content: () -> Content
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        VStack(alignment: .leading, spacing: DFSpace.m) {
            HStack(alignment: .top, spacing: DFSpace.m) {
                ZStack {
                    Circle().fill(complete ? DFPalette.accent : DFPalette.control(scheme))
                    if complete {
                        Image(systemName: "checkmark")
                            .font(DFType.icon(.small, weight: .semibold))
                            .foregroundStyle(.white)
                    } else {
                        Text("\(index)")
                            .font(DFType.caption.weight(.semibold))
                            .dfSecondaryText()
                    }
                }
                .frame(width: 24, height: 24)
                .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: DFSpace.xxs) {
                    Text(title).font(DFType.title)
                    Text(detail)
                        .font(DFType.detail).dfSecondaryText()
                        .fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: DFSpace.s)
                Text(complete ? "已完成" : "待完成")
                    .font(DFType.caption.weight(.medium))
                    .foregroundStyle(complete ? DFPalette.success : Color.secondary)
            }
            content()
        }
        .padding(DFMetrics.cardPadding)
        .frame(maxWidth: .infinity, alignment: .leading)
        .dfCard()
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

private struct MicrophoneStatusRow: View {
    let symbol: String
    let tone: DFTone
    let title: String
    var detail: String? = nil
    var actionTitle: String? = nil
    var action: (() -> Void)? = nil

    var body: some View {
        HStack(spacing: DFSpace.m) {
            Image(systemName: symbol)
                .font(DFType.icon(.medium, weight: .regular))
                .foregroundStyle(tone.color)
                .frame(width: 32, height: 32)
                .background(tone.color.opacity(DFOpacity.faint),
                            in: RoundedRectangle(cornerRadius: DFRadius.control, style: .continuous))
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: DFSpace.xxs) {
                Text(title).font(DFType.bodyMedium)
                if let detail {
                    Text(detail)
                        .font(DFType.detail).dfSecondaryText()
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer(minLength: DFSpace.s)
            if let actionTitle, let action {
                Button(actionTitle, action: action).buttonStyle(.dfSecondary)
            }
        }
        .padding(DFSpace.m)
        .background(tone.color.opacity(DFOpacity.faint),
                    in: RoundedRectangle(cornerRadius: DFRadius.control, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: DFRadius.control, style: .continuous)
                .strokeBorder(tone.color.opacity(DFOpacity.strong), lineWidth: DFMetrics.hairline)
        }
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
private struct MicrophoneLevelView: View {
    let level: Float
    var body: some View {
        HStack(spacing: 3) {
            ForEach(0..<12) { index in
                Capsule().fill(Double(index) / 12 < min(1, sqrt(Double(max(0, level)))) ? DFPalette.success : Color.primary.opacity(0.08))
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
        VStack(alignment: .leading, spacing: DFSpace.l) {
            Text(action.title).font(DFType.pageTitle)
            if action == .restartAudio {
                Text("这会短暂中断此 Mac 上所有应用的录音和播放，需要管理员授权。")
                Text("保留已安装的驱动和当前共享设置。服务恢复后按需连接，语音应用可能需要重新开始录音。")
                    .font(DFType.detail).dfSecondaryText()
            } else {
                Text("仅操作 DeskFuse 的麦克风驱动，需要在系统弹窗中授权管理员权限。操作前会关闭麦克风共享。")
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
