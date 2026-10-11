import SwiftUI
import AppKit
import DeskFuseEngine

struct InputView: View {
    @Bindable var store: AppStore
    let snapshot: EngineSnapshot
    @State private var showAddDevice = false
    @State private var updatingSettings = false
    @State private var retryingInput = false
    @State private var reconnectingDevices: Set<String> = []
    @AppStorage("input.displayLayoutTipDismissed") private var displayLayoutTipDismissed = false

    var body: some View {
        GeometryReader { geometry in
            if snapshot.input.canConfigure {
                VStack(alignment: .leading, spacing: 18) {
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
                    if snapshot.input.locked { Label("鼠标已锁定在当前设备", systemImage: "lock").foregroundStyle(.orange) }
                    layout
                }
                .padding(24)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
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
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                receiverState
                    .padding(28)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .background(Color(nsColor: .textBackgroundColor))
        .sheet(isPresented: $showAddDevice) {
            InputDevicePicker(snapshot: snapshot, adding: updatingSettings, error: store.error, onAdd: { id in
                setParticipant(id, included: true)
            }, onOpenDevices: {
                showAddDevice = false
                store.selectedPage = .devices
            })
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
            }
        }.frame(maxWidth: 520)
    }

    private var heading: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("当前屏幕排布").font(.system(size: 26, weight: .bold)).fixedSize()
            HStack(spacing: 8) {
                Text("本机主控").font(.callout.weight(.medium))
                Toggle("本机主控", isOn: Binding(
                    get: { snapshot.input.settings.controller },
                    set: { enabled in
                        var settings = (store.snapshot ?? snapshot).input.settings
                        settings.controller = enabled
                        if !enabled { settings.enabled = false }
                        applySettings(settings)
                    }
                ))
                .labelsHidden()
                .toggleStyle(.switch)
                .controlSize(.small)
                .disabled(updatingSettings || snapshot.input.managed)
                Image(systemName: "info.circle")
                    .font(.system(size: 13, weight: .medium))
                    .foregroundStyle(.secondary)
                    .help("开启后，这台 Mac 可以管理屏幕排布并控制已添加设备；关闭后只参与被控。")
                    .accessibilityLabel("本机主控说明")
                Text("共享键鼠").font(.callout.weight(.medium))
                Toggle("共享键鼠", isOn: Binding(
                    get: { snapshot.input.settings.enabled },
                    set: { setSharingEnabled($0) }
                ))
                .labelsHidden()
                .toggleStyle(.switch)
                .controlSize(.small)
                .disabled(updatingSettings || snapshot.input.managed || !snapshot.input.settings.controller)
            }
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
            }
            .fixedSize(horizontal: true, vertical: false)
            .disabled(updatingSettings)
        }
    }

    /// 打开共享时同时成为主控，避免提交引擎不接受的 enabled/disabled 组合。
    private func setSharingEnabled(_ enabled: Bool) {
        var settings = (store.snapshot ?? snapshot).input.settings
        settings.enabled = enabled
        // The engine only accepts an enabled controller. The empty-state action
        // is the one place that can enable sharing before the controller switch
        // is visible, so make that transition atomic.
        if enabled { settings.controller = true }
        applySettings(settings)
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

    private var layout: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .center, spacing: 12) {
                Text("\(snapshot.input.displays.count) 个屏幕")
                    .font(.callout.weight(.medium))
                    .foregroundStyle(.secondary)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 12)
            DisplayLayout(store: store, snapshot: snapshot, onRemoveDevice: { id in
                setParticipant(id, included: false)
            }, onReconnectDevice: reconnectDevice, reconnecting: reconnectingDevices,
               editingDisabled: updatingSettings,
               layoutTipVisible: !displayLayoutTipDismissed,
               onLayoutInteraction: { displayLayoutTipDismissed = true })
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .frame(maxWidth: .infinity)
    }

}
