import SwiftUI
import AppKit
import DeskFuseEngine

struct MouseSettingsView: View {
    let engine: Engine?
    @State private var controller = MouseButtonController.shared
    @State private var modifiers: UInt64 = 0
    @State private var recording: Int?
    @State private var monitor: Any?
    @State private var reset = false
    @State private var selection = ""
    @State private var contribution = false
    private let choices: [(String, MouseAction?)] = [
        ("保持原始功能", nil), ("停用此按键", .disabled),
        ("主键点击", .mouse(0)), ("副键点击", .mouse(1)), ("中键点击", .mouse(2)),
        ("后退", .shortcut(33, 0x100000)), ("前进", .shortcut(30, 0x100000)),
        ("向左切换桌面", .shortcut(123, 0x40000)), ("向右切换桌面", .shortcut(124, 0x40000)),
        ("Mission Control", .shortcut(126, 0x40000)), ("应用窗口", .shortcut(125, 0x40000)),
        ("显示桌面（F11）", .shortcut(103, 0)),
        ("复制", .shortcut(8, 0x100000)), ("粘贴", .shortcut(9, 0x100000)),
        ("撤销", .shortcut(6, 0x100000)), ("关闭窗口", .shortcut(13, 0x100000))
    ]

    private struct DeviceOption: Identifiable {
        enum Kind {
            case supported(LogitechHIDProtocol.Model)
            case generic
        }
        let id: String
        let title: String
        let kind: Kind
        let devices: [DetectedMouseDevice]

        var model: LogitechHIDProtocol.Model? {
            if case .supported(let model) = kind { return model }
            return nil
        }
        var isSupported: Bool { model != nil }
    }

    private var deviceOptions: [DeviceOption] {
        var options: [DeviceOption] = []
        var seen = Set<LogitechHIDProtocol.Model>()
        var generic: [DetectedMouseDevice] = []
        for device in controller.detectedMice {
            if let model = device.model {
                if seen.insert(model).inserted {
                    options.append(DeviceOption(id: "model:\(model.id)", title: model.displayName,
                                                kind: .supported(model), devices: [device]))
                }
            } else {
                generic.append(device)
            }
        }
        if !generic.isEmpty {
            options.append(DeviceOption(id: "generic", title: "通用鼠标", kind: .generic, devices: generic))
        } else if options.isEmpty {
            options.append(DeviceOption(id: "generic", title: "通用鼠标", kind: .generic, devices: []))
        }
        return options
    }
    private var selectedOption: DeviceOption {
        deviceOptions.first { $0.id == selection } ?? deviceOptions[0]
    }
    private var profile: LogitechHIDProtocol.Model? { selectedOption.model }
    private var buttons: [Int] {
        let base: Set<Int>
        switch profile {
        case .g502Lightspeed: base = LogitechHIDProtocol.g502Buttons
        case .mxMaster3, .mxMaster4, .mxAnywhere3, .mxErgo, .mxVertical, .generic:
            base = Set(0...4).union(LogitechHIDProtocol.mxButtons)
        case nil: base = Set(0...4)
        }
        return base
            .union(controller.settings.bindings.map(\.button))
            .filter {
                switch profile {
                case .g502Lightspeed: LogitechHIDProtocol.g502Buttons.contains($0)
                case .mxMaster3, .mxMaster4, .mxAnywhere3, .mxErgo, .mxVertical, .generic:
                    (0...4).contains($0) || LogitechHIDProtocol.mxButtons.contains($0)
                case nil: (0...4).contains($0)
                }
            }.sorted()
    }
    private var contributionDevice: DetectedMouseDevice? { selectedOption.devices.first }

    var body: some View {
        DFScrollPage(maxWidth: nil) {
            // 宽窗口：左侧设备与按键编辑，右侧已支持列表与说明；窄窗口自动上下堆叠。
            DFAdaptiveColumns {
                VStack(alignment: .leading, spacing: DFSpace.xl) {
                    statusNotices
                    assignments
                }
                VStack(alignment: .leading, spacing: DFSpace.l) {
                    supportedMice
                    notes
                    footer
                }
            }
        }
        .disabled(engine == nil)
        .onAppear { controller.beginDeviceScan() }
        .onChange(of: selectedOption.id) { _, _ in stopRecording(); modifiers = 0 }
        .onDisappear { stopRecording(); controller.endDeviceScan() }
        .confirmationDialog("恢复默认并同步到其他 Mac？", isPresented: $reset) {
            Button("恢复默认", role: .destructive) {
                var defaults = MouseSettings()
                defaults.enabled = controller.isFeatureEnabled
                controller.update(defaults)
            }
        }
        .sheet(isPresented: $contribution) {
            MouseDataContributionView(controller: controller, initialDevice: contributionDevice)
        }
    }

    @ViewBuilder
    private var statusNotices: some View {
        if controller.settings.enabled && !controller.tapReady {
            DFNotice(tone: .warning, title: "需要辅助功能与输入监控权限",
                     actionTitle: "检查权限", action: { controller.openInputMonitoringSettings() })
        }
        if let error = controller.error {
            DFNotice(tone: .danger, title: friendlyError(error))
        }
    }

    private func battery(_ model: LogitechHIDProtocol.Model) -> LogitechHIDManager.BatteryInfo? {
        controller.batteryInfo.first { LogitechHIDProtocol.model($0.name) == model }
    }

    private func wheel(_ model: LogitechHIDProtocol.Model) -> LogitechHIDManager.WheelInfo? {
        controller.wheelInfo.first { LogitechHIDProtocol.model($0.name) == model }
    }

    private func connectionPill(_ model: LogitechHIDProtocol.Model) -> DFStatusPill {
        guard isConnected(model) else { return DFStatusPill(text: "未连接", tone: .neutral) }
        guard let pct = battery(model)?.percent else { return DFStatusPill(text: "已连接", tone: .success) }
        let text = battery(model)?.charging == true ? "充电中 \(pct)%" : "电量 \(pct)%"
        return DFStatusPill(text: text, tone: pct <= 15 ? .danger : pct <= 30 ? .warning : .success)
    }

    private func smartShiftRow(_ info: LogitechHIDManager.WheelInfo) -> some View {
        HStack(spacing: 12) {
            Image(systemName: "scroll.fill")
                .font(DFType.body)
                .frame(width: 24)
                .dfSecondaryText()
            Text("滚轮模式")
                .frame(width: 62, alignment: .leading)
            Spacer(minLength: 0)
            if let freespin = info.freespin {
                Picker("滚轮模式", selection: Binding(
                    get: { freespin },
                    set: { value in
                        controller.setWheelMode(name: info.name, freespin: value)
                    })) {
                    Text("自由滚动").tag(true)
                    Text("棘轮").tag(false)
                }
                .pickerStyle(.segmented)
                .frame(width: 200)
                .help("SmartShift：切换滚轮的自动解锁模式")
            } else {
                Text("不支持")
                    .font(DFType.detail)
                    .dfSecondaryText()
            }
        }
        .font(DFType.detail)
        .padding(.vertical, 6)
    }

    private var assignments: some View {
        VStack(alignment: .leading, spacing: 12) {
            if deviceOptions.count > 1 {
                Picker("鼠标类型", selection: Binding(
                    get: { selectedOption.id },
                    set: { selection = $0 })) {
                    ForEach(deviceOptions) { option in
                        Text(option.title).tag(option.id)
                    }
                }
                .labelsHidden()
                .frame(width: 200)
                .frame(maxWidth: .infinity, alignment: .trailing)
                .help("只显示当前识别到的鼠标；未适配型号使用通用映射")
            }
            switch selectedOption.kind {
            case .supported(.g502Lightspeed):
                G502MouseEditor(controller: controller, onSelection: stopRecording) { button in
                    VStack(alignment: .leading, spacing: 10) {
                        actionMenu(button)
                        Text(controller.saveStatus)
                            .font(DFType.caption).foregroundStyle(.secondary)
                        Text(controller.lastExecution).font(DFType.caption).foregroundStyle(.secondary)
                        Text("自动清除该位置的通用按键映射，避免重复执行动作。")
                            .font(DFType.caption).foregroundStyle(.secondary)
                    }
                }
            case .supported(.mxMaster3), .supported(.mxMaster4):
                MXMasterMouseEditor(controller: controller, onSelection: stopRecording) { button in
                    actionMenu(button)
                }
            case .supported(.mxAnywhere3), .supported(.mxErgo), .supported(.mxVertical),
                 .supported(.generic), .generic:
                genericAssignments
            }
        }
    }

    private var genericAssignments: some View {
        VStack(alignment: .leading, spacing: 12) {
            DFSection {
                ForEach(buttons, id: \.self) { button in
                    if button != buttons.first { Divider() }
                    mappingRow(button)
                }
            }
            Text("支持左键、右键、中键及前进、后退键。侧键需鼠标具备对应按键并发送标准鼠标事件。")
                .font(DFType.caption).foregroundStyle(.secondary)
            DFDisclosure("组合键设置") {
                VStack(alignment: .leading, spacing: 12) {
                    HStack {
                        Text("按住这些键时使用独立映射").foregroundStyle(.secondary)
                        Spacer()
                        modifier("⌘", 0x100000); modifier("⇧", 0x20000)
                        modifier("⌥", 0x80000); modifier("⌃", 0x40000)
                    }
                    Text("未设置的组合沿用默认映射。").font(DFType.caption).foregroundStyle(.secondary)
                }.font(DFType.detail)
            }
        }
    }

    private var supportedMice: some View {
        DFSection("已支持鼠标", symbol: "checkmark.seal") {
            ForEach(Array(LogitechHIDProtocol.Model.allCases.filter { $0 != .generic }.enumerated()), id: \.offset) { index, model in
                if index > 0 { Divider() }
                HStack(spacing: DFSpace.m) {
                    Image(systemName: "computermouse.fill")
                        .font(DFType.icon(.regular))
                        .dfSecondaryText()
                        .frame(width: 20)
                    VStack(alignment: .leading, spacing: DFSpace.xxs) {
                        Text(model.displayName).font(DFType.bodyMedium)
                        Text(model.supportSummary).font(DFType.caption).dfSecondaryText()
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    Spacer(minLength: DFSpace.s)
                    connectionPill(model)
                }
                .frame(minHeight: 38)
                if let wheel = wheel(model) {
                    smartShiftRow(wheel)
                }
            }
            Divider().padding(.vertical, 4)
            VStack(alignment: .leading, spacing: DFSpace.s) {
                Text("没有找到你的鼠标？").font(DFType.bodyMedium)
                Text("记录型号和按键编号，生成 Markdown 数据；后续可直接用于适配 Issue。")
                    .font(DFType.caption).dfSecondaryText()
                    .fixedSize(horizontal: false, vertical: true)
                Button("我要提供鼠标数据…") { contribution = true }
                    .buttonStyle(.dfPrimary)
            }
        }
    }

    private var notes: some View {
        DFSection {
            DFDisclosure("设备状态与使用说明") {
                VStack(alignment: .leading, spacing: 12) {
                    Text(controller.hidStatus)
                    Text("列表只展示当前识别到的鼠标；未适配型号自动使用通用映射。如已连接但未显示，请检查输入监控权限。")
                    Text("G502 的原有 DPI、配置切换或 G HUB 动作会继续执行。实体键编号按实际识别为准；不要同时绑定同一按键的通用事件和实体事件。机械滚轮解锁键无法识别。")
                    Text("切换桌面需要对应方向存在桌面，且系统键盘快捷键已启用。MX 与 G502 原生按键暂不支持修饰键分层。")
                    Button("打开输入监控设置") { controller.openInputMonitoringSettings() }
                }.font(DFType.caption).foregroundStyle(.secondary)
            }
            Divider().padding(.vertical, 4)
            DFDisclosure("Mac 间自动同步") {
                Text("配置自动同步到已配对、已连接且支持此功能的 Mac。离线修改在重连后按逻辑版本同步整套配置，不逐个合并按键；权限需在每台 Mac 单独授予。Windows 不参与。")
                    .font(DFType.caption).foregroundStyle(.secondary)
            }
        }
    }

    private var footer: some View {
        HStack {
            Label("更改自动保存", systemImage: "checkmark.circle").foregroundStyle(.secondary)
            Spacer()
            Button("恢复默认…", role: .destructive) { reset = true }.buttonStyle(.borderless)
        }.font(DFType.caption).padding(.horizontal, 4)
    }

    private func isConnected(_ model: LogitechHIDProtocol.Model) -> Bool {
        controller.detectedMice.contains { $0.model == model }
    }

    private func mappingRow(_ button: Int) -> some View {
        HStack(spacing: 12) {
            Image(systemName: selected(button) == nil ? "computermouse" : "cursorarrow.click")
                .foregroundStyle(selected(button) == nil ? Color.secondary : DFPalette.accent)
                .frame(width: 24)
            Text(MouseSettings.buttonName(button)).lineLimit(2)
            Spacer(minLength: 12)
            actionMenu(button)
        }.frame(minHeight: 42)
    }
    private func actionMenu(_ button: Int) -> some View {
            Menu {
                ForEach(choices.indices.filter {
                    (!LogitechHIDProtocol.buttons.contains(button) || choices[$0].1?.kind != "mouse")
                        && (!LogitechHIDProtocol.g502Buttons.contains(button) || choices[$0].1?.kind != "disabled")
                }, id: \.self) { index in
                    Button(choices[index].0) { set(button, choices[index].1) }
                }
                Divider()
                Button("录制快捷键…") { beginRecording(button) }
                if modifiers != 0 { Button("此组合保持原始按钮") { set(button, .mouse(button)) } }
            } label: {
                HStack(spacing: 8) {
                    Text(recording == button ? "请按快捷键…" : label(button)).lineLimit(2)
                    Image(systemName: "chevron.up.chevron.down").font(DFType.caption.weight(.semibold))
                }.foregroundStyle(selected(button) == nil ? Color.secondary : Color.primary)
                    .frame(minWidth: 150, alignment: .trailing)
                    .padding(.vertical, 8)
            }.menuStyle(.borderlessButton).menuIndicator(.hidden)
                .fixedSize(horizontal: true, vertical: false)
                .disabled(recording != nil)
                .accessibilityLabel("\(MouseSettings.buttonName(button))的动作")
    }
    private func modifier(_ title: String, _ bit: UInt64) -> some View {
        Toggle(title, isOn: Binding(get: { modifiers & bit != 0 }, set: {
            stopRecording()
            if $0 { modifiers |= bit } else { modifiers &= ~bit }
        })).toggleStyle(.button).help("为此修饰键组合单独设置映射")
    }
    private func selected(_ button: Int) -> MouseAction? {
        return controller.settings.bindings.first { $0.button == button && $0.modifiers == modifiers }?.action
    }
    private func label(_ button: Int) -> String {
        let action = selected(button)
        if action == nil && modifiers != 0 { return "沿用默认映射" }
        if let index = choices.firstIndex(where: { $0.1 == action }) { return choices[index].0 }
        if let action, let key = action.key_code {
            return shortcutLabel(Shortcut(keyCode: key, modifiers: action.modifiers ?? 0))
        }
        return "原始按钮"
    }
    private func set(_ button: Int, _ action: MouseAction?) {
        var value = controller.settings
        value.set(button: button, modifiers: modifiers, action: action)
        if profile == .g502Lightspeed {
            let locations = G502ButtonLocations(json: UserDefaults.standard.string(forKey: "mouse.g502.physicalLocations.v1") ?? "{}")
            if let generic = locations.position(for: button)?.genericButton {
                value.bindings.removeAll { $0.button == generic }
            }
        }
        controller.update(value)
    }
    private func beginRecording(_ button: Int) {
        stopRecording()
        recording = button
        engine?.setRecording(enabled: true)
        ClipboardPanelController.shared.suspendShortcut(true)
        monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
            MainActor.assumeIsolated {
                if event.keyCode != 53 {
                    set(button, .shortcut(event.keyCode, UInt64(event.modifierFlags.rawValue) & 0x1e0000))
                }
                stopRecording()
            }
            return nil
        }
    }
    private func stopRecording() {
        guard recording != nil else { return }
        if let monitor { NSEvent.removeMonitor(monitor) }
        monitor = nil; recording = nil
        ClipboardPanelController.shared.suspendShortcut(false)
        engine?.setRecording(enabled: false)
    }
}
