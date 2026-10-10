import SwiftUI
import AppKit
import DeskFuseEngine

struct InputView: View {
    @Bindable var store: AppStore
    let snapshot: EngineSnapshot
    @State private var showAddDevice = false
    @State private var showSaveLayout = false
    @State private var layoutName = ""
    @State private var updatingSettings = false
    @State private var retryingInput = false
    @State private var reconnectingDevices: Set<String> = []

    var body: some View {
        GeometryReader { geometry in
            ScrollView {
                if snapshot.input.canConfigure {
                    VStack(alignment: .leading, spacing: 22) {
                        ViewThatFits(in: .horizontal) {
                            HStack(alignment: .center, spacing: 16) {
                                heading
                                Spacer(minLength: 16)
                                headerActions
                            }
                            VStack(alignment: .leading, spacing: 16) {
                                heading
                                HStack { Spacer(minLength: 0); headerActions }
                            }
                        }
                        inputError
                        if snapshot.input.locked { Label("鼠标已锁定在当前设备", systemImage: "lock").foregroundStyle(.orange) }
                        layout(height: max(300, geometry.size.height - 220))
                        participantList
                    }.padding(28).frame(maxWidth: .infinity, alignment: .topLeading)
                } else if !snapshot.input.settings.enabled && !snapshot.input.settings.controller {
                    FeatureGateView(
                        symbol: "rectangle.on.rectangle",
                        symbolColor: .purple,
                        title: "键鼠共享",
                        message: "用一套键鼠控制多台电脑。",
                        bullets: [
                            "鼠标穿越屏幕边缘切换设备",
                            "剪贴板随焦点设备自动跟随",
                            "紧急收回快捷键随时可用",
                        ],
                        permission: .accessibility,
                        actionTitle: "启用键鼠共享",
                        action: { setSharingEnabled(true) },
                        secondaryTitle: "打开共享设置",
                        secondaryAction: { store.openSettings(tab: SettingsCategory.input.rawValue) }
                    )
                    .padding(28)
                    .frame(maxWidth: .infinity, minHeight: max(0, geometry.size.height - 56))
                } else {
                    receiverState
                        .padding(28)
                        .frame(maxWidth: .infinity, minHeight: max(0, geometry.size.height - 56))
                }
            }.background(Color(nsColor: .textBackgroundColor))
        }
        .sheet(isPresented: $showAddDevice) {
            InputDevicePicker(snapshot: snapshot, adding: updatingSettings, error: store.error, onAdd: { id in
                setParticipant(id, included: true)
            }, onOpenDevices: {
                showAddDevice = false
                store.selectedPage = .devices
            })
        }
        .alert("保存布局", isPresented: $showSaveLayout) {
            TextField("布局名称", text: $layoutName)
            Button("保存") { saveLayoutNamed(layoutName) }
            Button("取消", role: .cancel) {}
        } message: {
            Text("独立布局不会绑定 Wi-Fi，可在「设置 → 键鼠共享」中随时应用。")
        }
        .onChange(of: snapshot.input.canConfigure) { _, allowed in
            if !allowed { showAddDevice = false }
        }
        .onChange(of: snapshot.input.displays.filter(\.ready).map(\.deviceId)) { _, ready in
            reconnectingDevices.subtract(Set(ready))
        }
        .task(id: snapshot.input.canConfigure) {
            guard snapshot.input.canConfigure else { return }
            await store.refreshDisplayWallpapers()
        }
        .onDisappear {
            store.clearDisplayWallpapers()
        }
    }

    private var receiverState: some View {
        VStack(spacing: 20) {
            Image(systemName: snapshot.input.managedPaused ? "pause.circle" : "keyboard")
                .font(.system(size: 44, weight: .light)).foregroundStyle(.secondary)
                .accessibilityHidden(true)
            Text(snapshot.input.managedPaused ? "已停止被控" : snapshot.input.managed ? "被控状态" : snapshot.input.settings.controller ? "主控已自动暂停" : "等待主控")
                .font(.system(size: 26, weight: .bold))
            Text(snapshot.input.summary).font(.callout).foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            inputError
            if snapshot.input.managed {
                Button("停止被控") { store.reclaim() }
                    .modifier(NativeGlassAction())
                    .disabled(snapshot.input.managedPaused)
            } else {
                if snapshot.input.settings.controller {
                    Text(snapshot.input.environmentMessage).font(.callout).foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                }
                Button(snapshot.input.settings.controller ? "本次使用此 Mac 作为主控" : "在此 Mac 启用主控") { enableAsController() }
                    .modifier(NativeGlassAction())
                Button("共享设置") { store.openSettings(tab: SettingsCategory.input.rawValue) }
                    .modifier(NativeGlassAction())
                Button("快捷键设置") { store.openSettings(tab: "shortcuts") }
                    .buttonStyle(.link)
            }
        }.frame(maxWidth: 520)
    }

    @ViewBuilder
    private var inputError: some View {
        if let error = snapshot.input.error {
            VStack(alignment: .leading, spacing: 10) {
                Label(error, systemImage: "exclamationmark.triangle")
                    .font(.callout).foregroundStyle(.orange)
                    .fixedSize(horizontal: false, vertical: true)
                Button(retryingInput ? "正在重试…" : "重试输入检查", systemImage: "arrow.clockwise") {
                    retryingInput = true
                }
                .disabled(retryingInput)
                .help("重新检查原生输入状态，必要时恢复输入服务；不会自动开启键鼠共享。")
            }
            .task(id: retryingInput) {
                guard retryingInput else { return }
                store.send(.refreshPermissions)
                // Command acceptance is not native readiness; retain the real error
                // until a fresh engine snapshot clears it. Only debounce the button.
                try? await Task.sleep(for: .seconds(2))
                retryingInput = false
            }
        }
    }

    private var heading: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("当前屏幕排布").font(.system(size: 26, weight: .bold)).fixedSize()
            Text(snapshot.input.summary).font(.callout).foregroundStyle(.secondary)
        }
    }

    private var headerActions: some View {
        NativeGlassGroup {
            HStack(alignment: .center, spacing: 12) {
                WorkspaceChromeButton("添加设备屏幕", systemImage: "plus", prominent: true) {
                    showAddDevice = true
                }
                WorkspaceChromeButton("共享设置", systemImage: "slider.horizontal.3") {
                    store.openSettings(tab: SettingsCategory.input.rawValue)
                }
                WorkspaceChromeButton("快捷键设置", systemImage: "keyboard", help: "配置紧急收回、方向切换和锁定快捷键") {
                    store.openSettings(tab: "shortcuts")
                }
            }
            .fixedSize(horizontal: true, vertical: false)
            .disabled(updatingSettings)
        }
    }

    /// 只打开共享开关，不强制成为主控：用户可以只作为被控参与。
    private func setSharingEnabled(_ enabled: Bool) {
        var settings = (store.snapshot ?? snapshot).input.settings
        settings.enabled = enabled
        applySettings(settings)
    }

    private func saveLayoutNamed(_ rawName: String) {
        let name = rawName.trimmingCharacters(in: .whitespaces)
        store.error = nil
        store.send(.saveScene(name: name.isEmpty ? "布局 \(snapshot.input.scenes.count + 1)" : name))
    }

    /// 成为主控但不自动打开共享，避免新设备默认开始注入键鼠。
    private func enableAsController() {
        let current = store.snapshot ?? snapshot
        if current.input.settings.controller {
            store.send(.chooseController(id: current.localId, remember: false))
            return
        }
        var settings = current.input.settings
        settings.controller = true
        settings.enabled = false
        applySettings(settings)
    }

    /// Starts one fresh enable cycle for a device that reclaimed input locally.
    /// Toggling the whole switch is not needed for a single device.
    private func restartSharing(_ id: String) {
        guard !updatingSettings else { return }
        updatingSettings = true
        store.error = nil
        store.send(.restartInputActivation(peerId: id), applied: { updatingSettings = false },
                   failed: { _ in updatingSettings = false })
    }

    /// Dial the peer again and start a new input activation so the receiver can prepare.
    private func reconnectDevice(_ id: String) {
        guard !reconnectingDevices.contains(id) else { return }
        reconnectingDevices.insert(id)
        store.error = nil
        store.send(.connect(peerId: id))
        store.send(.restartInputActivation(peerId: id))
        Task { @MainActor in
            try? await Task.sleep(for: .seconds(12))
            reconnectingDevices.remove(id)
        }
    }

    private func applySettings(_ settings: InputSettings) {
        guard !updatingSettings else { return }
        updatingSettings = true
        store.error = nil
        store.send(.setInput(settings: settings), applied: { updatingSettings = false },
                   failed: { _ in updatingSettings = false })
    }

    private func setParticipant(_ id: String, included: Bool) {
        let current = store.snapshot ?? snapshot
        guard current.input.canConfigure, id != current.localId else { return }
        var settings = current.input.settings
        if included {
            guard current.devices.contains(where: { $0.id == id && $0.member && !$0.blocked }),
                  !settings.participants.contains(id) else { return }
            settings.participants.append(id)
        } else {
            settings.participants.removeAll { $0 == id }
        }
        // Keep saved placement and pairing so adding this device again restores its position.
        applySettings(settings)
    }

    private func layout(height: CGFloat) -> some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(alignment: .center, spacing: 12) {
                Text("\(snapshot.input.displays.count) 个屏幕").font(.callout).foregroundStyle(.secondary)
                Spacer(minLength: 12)
                saveEnvironmentLayoutButton
                layoutMenu
            }
            DisplayLayout(store: store, snapshot: snapshot, onRemoveDevice: { id in
                setParticipant(id, included: false)
            }, onReconnectDevice: reconnectDevice, reconnecting: reconnectingDevices,
               editingDisabled: updatingSettings).frame(height: height)
            Text("拖动屏幕调整位置，松手自动贴合。添加或移除按整台设备生效，不解除配对。")
                .font(.caption).foregroundStyle(.secondary)
        }.frame(maxWidth: .infinity)
    }

    /// The Wi-Fi-bound layout is a frequent action, so keep it beside the
    /// canvas instead of requiring a trip through Settings or a secondary menu.
    @ViewBuilder
    private var saveEnvironmentLayoutButton: some View {
        if let ssid = snapshot.input.ssid {
            WorkspaceChromeButton(
                boundScene == nil ? "保存到当前 Wi-Fi" : "更新当前 Wi-Fi 布局",
                systemImage: "wifi",
                help: boundScene == nil
                    ? "把当前屏幕排布保存到「\(ssid)」；以后回到这个网络时自动恢复。"
                    : "更新「\(ssid)」保存的屏幕排布。",
                prominent: false
            ) {
                store.error = nil
                store.send(.saveEnvironmentLayout(name: nil))
            }
        } else {
            WorkspaceChromeButton(
                "保存到当前 Wi-Fi",
                systemImage: "wifi.slash",
                help: "尚未取得唯一的 Wi-Fi 名称，暂时无法按网络保存布局。"
            ) {}
            .disabled(true)
        }
    }

    /// 布局既能随 Wi-Fi 自动保存/恢复，也能另存为独立布局。
    private var layoutMenu: some View {
        Menu {
            Button("另存为新布局…") { layoutName = ""; showSaveLayout = true }
            if !snapshot.input.scenes.isEmpty {
                Divider()
                ForEach(snapshot.input.scenes, id: \.id) { scene in
                    Button(scene.name) { store.send(.applyScene(id: scene.id)) }
                }
            }
        } label: {
            Label("布局", systemImage: "square.on.square")
                .modifier(WorkspaceControlLabel())
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .fixedSize()
        .frame(minHeight: WorkspaceControlMetrics.height)
        .modifier(NativeGlassBackdrop(cornerRadius: WorkspaceControlMetrics.cornerRadius))
        .disabled(updatingSettings)
        .help("保存或应用屏幕布局；绑定 Wi-Fi 的布局会在回到该网络时自动恢复。")
    }

    private var boundScene: DeskFuseEngine.Scene? {
        guard let ssid = snapshot.input.ssid else { return nil }
        return snapshot.input.scenes.first { $0.ssid == ssid }
    }

    private var participantList: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("排布中的设备").font(.headline).padding(.bottom, 8)
            participantRow(snapshot.localId, local: true)
            ForEach(Array(Set(snapshot.input.settings.participants)).sorted(), id: \.self) { id in
                if id != snapshot.localId {
                    Divider()
                    participantRow(id, local: false)
                }
            }
            if snapshot.input.settings.participants.isEmpty {
                Text("还没有添加其他设备。点击“添加设备屏幕”，选择已配对的电脑。")
                    .font(.callout).foregroundStyle(.secondary).padding(.top, 8)
            }
        }
    }

    private func participantRow(_ id: String, local: Bool) -> some View {
        let device = snapshot.devices.first { $0.id == id }
        let screens = snapshot.input.displays.sorted { $0.layoutID < $1.layoutID }
        let deviceScreens = screens.enumerated().filter { $0.element.deviceId == id }
        let name = local ? snapshot.localName : device?.name ?? "设备 \(id.prefix(8))"
        let screensUnready = !deviceScreens.isEmpty && !deviceScreens.allSatisfy { $0.element.ready }
        let canReconnect = screensUnready && (snapshot.input.settings.enabled || device?.connected != true)
        let reconnecting = reconnectingDevices.contains(id)
        return HStack(alignment: .center, spacing: 14) {
            Image(systemName: local ? "laptopcomputer" : "display")
                .font(.title3)
                .foregroundStyle(DisplayPalette.color(for: id, displays: screens))
                .frame(width: 28)
            VStack(alignment: .leading, spacing: 5) {
                Text(name + (local ? "（本机）" : "")).font(.callout.weight(.semibold))
                if deviceScreens.isEmpty {
                    Text(local ? "正在读取本机屏幕信息" : "已添加，等待屏幕信息")
                        .font(.caption).foregroundStyle(.secondary)
                } else {
                    Text(deviceScreens.map { String(format: "%02d", $0.offset + 1) + "  " + $0.element.layoutSizeLabel }.joined(separator: "  /  "))
                        .font(.caption).foregroundStyle(.secondary)
                }
                if !local {
                    Text(!deviceScreens.isEmpty && deviceScreens.allSatisfy { $0.element.ready }
                         ? "跨屏已就绪" : device?.inputReason ?? "设备暂不可用，请前往设备页检查连接。")
                        .font(.caption).foregroundStyle(.secondary)
                    if deviceScreens.isEmpty {
                        Button("检查设备连接") { store.selectedPage = .devices }
                            .font(.caption).buttonStyle(.link)
                    }
                }
            }
            Spacer(minLength: 8)
            if local {
                Text("本机屏幕").font(.caption).foregroundStyle(.secondary)
            } else {
                if device?.inputRestartRequired == true {
                    Button("重新启用共享", systemImage: "arrow.clockwise") { restartSharing(id) }
                        .font(.caption)
                        .disabled(updatingSettings)
                        .help("对端已在本机收回输入；重新启用会为这台设备开始一次新的共享周期。")
                } else if canReconnect {
                    Button(reconnecting ? "正在连接" : "重连", systemImage: "arrow.clockwise") { reconnectDevice(id) }
                        .font(.caption)
                        .disabled(updatingSettings || reconnecting)
                        .help("重新连接此设备，并通知对端准备键鼠共享。")
                }
                Button("移除", systemImage: "minus.circle") { setParticipant(id, included: false) }
                    .disabled(updatingSettings)
                    .help("从排布移除 \(name) 的全部屏幕，停止向该设备共享键鼠；不会解除配对。")
            }
        }.padding(.vertical, 12)
    }
}
