import SwiftUI
import AppKit
import UniDeskEngine

struct InputView: View {
    @Bindable var store: AppStore
    let snapshot: EngineSnapshot
    @State private var showSettings = false
    @State private var showAddDevice = false
    @State private var updatingSettings = false

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
                        if let error = snapshot.input.error { Label(error, systemImage: "exclamationmark.triangle").foregroundStyle(.orange) }
                        if snapshot.input.locked { Label("鼠标已锁定在当前设备", systemImage: "lock").foregroundStyle(.orange) }
                        layout(height: max(300, geometry.size.height - 260))
                        participantList
                        reclaimBar
                    }.padding(28).frame(maxWidth: .infinity, alignment: .topLeading)
                } else {
                    receiverState
                        .padding(28)
                        .frame(maxWidth: .infinity, minHeight: max(0, geometry.size.height - 56))
                }
            }.background(Color(nsColor: .textBackgroundColor))
        }
        .sheet(isPresented: $showSettings) {
            if snapshot.input.canConfigure {
                InputSettingsEditor(store: store, initial: snapshot.input.settings).frame(width: 640, height: 610)
            }
        }
        .sheet(isPresented: $showAddDevice) {
            InputDevicePicker(snapshot: snapshot, adding: updatingSettings, error: store.error, onAdd: { id in
                setParticipant(id, included: true)
            }, onOpenDevices: {
                showAddDevice = false
                store.selectedPage = .devices
            })
        }
        .onChange(of: snapshot.input.canConfigure) { _, allowed in
            if !allowed { showSettings = false; showAddDevice = false }
        }
    }

    private var receiverState: some View {
        VStack(spacing: 20) {
            Image(systemName: snapshot.input.managedPaused ? "pause.circle" : "keyboard")
                .font(.system(size: 44, weight: .light)).foregroundStyle(.secondary)
                .accessibilityHidden(true)
            Text(snapshot.input.managedPaused ? "已停止被控" : snapshot.input.managed ? "被控状态" : "等待主控")
                .font(.system(size: 26, weight: .bold))
            Text(snapshot.input.summary).font(.callout).foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            if let error = snapshot.input.error {
                Label(error, systemImage: "exclamationmark.triangle")
                    .font(.callout).foregroundStyle(.orange)
            }
            if snapshot.input.managed {
                Button("停止被控") { store.reclaim() }
                    .modifier(NativeGlassAction())
                    .disabled(snapshot.input.managedPaused)
            } else {
                Button("在此 Mac 启用主控") { setControllerEnabled(true) }
                    .modifier(NativeGlassAction())
            }
        }.frame(maxWidth: 520)
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
                WorkspaceChromeButton("共享设置", systemImage: "slider.horizontal.3") {
                    showSettings = true
                }
                sharingControl
            }
            .fixedSize(horizontal: true, vertical: false)
            .disabled(updatingSettings)
        }
    }

    private var sharingControl: some View {
        Toggle("共享键鼠", isOn: Binding(get: { snapshot.input.settings.enabled }, set: setControllerEnabled))
            .toggleStyle(.switch).controlSize(.small).fixedSize()
            .modifier(WorkspaceControlLabel())
            .modifier(NativeGlassBackdrop(cornerRadius: WorkspaceControlMetrics.cornerRadius))
    }

    private func setControllerEnabled(_ enabled: Bool) {
        var settings = (store.snapshot ?? snapshot).input.settings
        settings.enabled = enabled
        settings.controller = true
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
            HStack {
                Text("\(snapshot.input.displays.count) 个屏幕").font(.callout).foregroundStyle(.secondary)
                Spacer()
                WorkspaceChromeButton("添加设备屏幕", systemImage: "plus", prominent: true) { showAddDevice = true }
                    .disabled(updatingSettings)
            }
            DisplayLayout(store: store, snapshot: snapshot, onRemoveDevice: { id in
                setParticipant(id, included: false)
            }, editingDisabled: updatingSettings).frame(height: height)
            Text("拖动屏幕调整位置，松手自动贴合。添加或移除按整台设备生效，不解除配对。")
                .font(.caption).foregroundStyle(.secondary)
        }.frame(maxWidth: .infinity)
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
                }
                Button("移除", systemImage: "minus.circle") { setParticipant(id, included: false) }
                    .disabled(updatingSettings)
                    .help("从排布移除 \(name) 的全部屏幕，停止向该设备共享键鼠；不会解除配对。")
            }
        }.padding(.vertical, 12)
    }

    private var reclaimBar: some View {
        HStack(spacing: 12) {
            Text("紧急收回").font(.callout.weight(.semibold))
            Text("⌃⌥⌘ Esc").font(.caption.monospaced()).foregroundStyle(.secondary)
            Spacer()
            Button("现在收回") { store.reclaim() }
        }.padding(14).modifier(NativeGlassSurface(cornerRadius: 12))
    }
}

struct InputSettingsEditor: View {
    @Bindable var store: AppStore
    @State var draft: InputSettings
    @State private var sceneName = ""
    @State private var renameID: String?
    @State private var remember = true
    @State private var selectedController = ""
    @Environment(\.dismiss) private var dismiss
    init(store: AppStore, initial: InputSettings) { self.store = store; _draft = State(initialValue: initial) }
    var body: some View {
        VStack(spacing: 0) {
            HStack { Text("键鼠共享设置").font(.title2.bold()); Spacer(); Button("取消") { dismiss() }; Button("保存") { store.send(.setInput(settings: draft)); dismiss() }.keyboardShortcut(.defaultAction) }.padding()
            Form {
                Section("主控") {
                    Toggle("此设备作为主控", isOn: $draft.controller)
                    Text("在键鼠共享页面添加或移除设备屏幕。").font(.caption).foregroundStyle(.secondary)
                }
                Section("跨屏方式") {
                    Picker("触发方式", selection: $draft.switchMode) { Text("立即穿越").tag(SwitchMode.instant); Text("停留 250 毫秒").tag(SwitchMode.dwell); Text("双触边缘").tag(SwitchMode.doubleTap) }
                    ForEach(0..<4) { index in Toggle(["禁用左上角", "禁用右上角", "禁用左下角", "禁用右下角"][index], isOn: Binding(get: { draft.disabledCorners & (1 << index) != 0 }, set: { enabled in if enabled { draft.disabledCorners |= 1 << index } else { draft.disabledCorners &= ~(1 << index) } })) }
                    Stepper("边角范围：\(draft.cornerSize) pt", value: $draft.cornerSize, in: 0...256)
                }
                Section("快捷键") { ShortcutEditor(store: store, shortcuts: $draft.shortcuts) }
                Section("场景") {
                    HStack { TextField("场景名称", text: $sceneName); Button(renameID == nil ? "保存当前布局" : "重命名") { if let id = renameID { store.send(.renameScene(id: id, name: sceneName)); renameID = nil } else { store.send(.saveScene(name: sceneName)) }; sceneName = "" }.disabled(sceneName.trimmingCharacters(in: .whitespaces).isEmpty) }
                    Text("保存场景使用已应用的设置；请先保存上方配置。").font(.caption).foregroundStyle(.secondary)
                    ForEach(store.snapshot?.input.scenes ?? [], id: \.id) { scene in HStack { Text(scene.name); Spacer(); Button("应用") { store.send(.applyScene(id: scene.id)); dismiss() }; Button("重命名") { renameID = scene.id; sceneName = scene.name }; Button("删除", role: .destructive) { store.send(.deleteScene(id: scene.id)) } } }
                }
                Section("按 Wi-Fi 自动选择主控") {
                    Text(store.snapshot?.input.environmentMessage ?? "等待网络状态").foregroundStyle(.secondary)
                    Button("允许读取当前 Wi-Fi") { requestWifiAccess() }
                    Picker("主控", selection: $selectedController) { Text("选择设备").tag(""); if let snapshot = store.snapshot { Text(snapshot.localName).tag(snapshot.localId); ForEach(snapshot.devices.filter { $0.member }, id: \.id) { Text($0.name).tag($0.id) } } }
                    Toggle("记住当前 Wi-Fi", isOn: $remember)
                    Button("使用此主控") { store.send(.chooseController(id: selectedController, remember: remember)) }.disabled(selectedController.isEmpty)
                    ForEach(store.snapshot?.input.rules ?? [], id: \.ssid) { rule in HStack { Text(rule.ssid); Spacer(); Text(controllerName(rule.controller)).foregroundStyle(.secondary); Button("删除") { store.send(.forgetEnvironment(ssid: rule.ssid)) } } }
                }
            }.formStyle(.grouped)
        }
        .onChange(of: store.snapshot?.input.canConfigure) { _, allowed in
            if allowed != true { dismiss() }
        }
    }
    private func controllerName(_ id: String) -> String { if id == store.snapshot?.localId { return store.snapshot?.localName ?? "本机" }; return store.snapshot?.devices.first { $0.id == id }?.name ?? "离线设备" }
}
