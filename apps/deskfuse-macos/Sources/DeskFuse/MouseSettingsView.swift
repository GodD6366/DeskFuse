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
    @State private var source = 1
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
    private var buttons: [Int] {
        Set(0...4).union(controller.availableNativeButtons)
            .union(controller.settings.bindings.map(\.button))
            .filter {
                switch source {
                case 1: LogitechHIDProtocol.g502Buttons.contains($0)
                case 2: (0...4).contains($0) || LogitechHIDProtocol.mxButtons.contains($0)
                default: (0...4).contains($0)
                }
            }.sorted()
    }
    private var mappedCount: Int { buttons.filter { selected($0) != nil }.count }

    var body: some View {
        DFScrollPage {
            DFSection {
                HStack(spacing: 16) {
                    Image(systemName: "computermouse.fill")
                        .font(DFType.metric.weight(.light))
                        .frame(width: 52, height: 56)
                        .foregroundStyle(.secondary)
                    VStack(alignment: .leading, spacing: 5) {
                        Text(source == 1 ? "G502 LIGHTSPEED" : source == 2 ? "MX Master 3" : "通用鼠标").font(DFType.title.weight(.semibold))
                        Text(controller.settings.enabled ? "自定义已启用 · 修改自动保存" : "先配置按键，再开启自定义")
                            .font(DFType.detail).foregroundStyle(.secondary)
                    }
                    Spacer()
                    Toggle("启用鼠标自定义", isOn: Binding(get: { controller.settings.enabled }, set: {
                        var value = controller.settings; value.enabled = $0; controller.update(value)
                    })).toggleStyle(.switch).labelsHidden()
                        .accessibilityLabel("启用鼠标按键自定义")
                }
                if controller.settings.enabled && !controller.tapReady {
                    Divider()
                    HStack {
                        Label("需要辅助功能与输入监控权限", systemImage: "exclamationmark.triangle")
                            .foregroundStyle(DFPalette.warning)
                        Spacer()
                        Button("检查权限") { controller.openInputMonitoringSettings() }
                    }.padding(.top, 6)
                }
                if let error = controller.error {
                    Label(error, systemImage: "exclamationmark.circle").font(DFType.caption).foregroundStyle(DFPalette.danger)
                }
            }

            VStack(alignment: .leading, spacing: 12) {
                HStack {
                    Text("按键分配").font(DFType.title.weight(.semibold))
                    Text("已设置 \(mappedCount) 项").font(DFType.caption).foregroundStyle(.secondary)
                    Spacer()
                    Picker("鼠标类型", selection: $source) {
                        Text("G502 LIGHTSPEED").tag(1)
                        Text("通用鼠标").tag(0)
                        Text("MX Master 3").tag(2)
                    }.labelsHidden().frame(width: 180)
                }
                if source == 1 {
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
                } else if source == 2 {
                    MXMasterMouseEditor(controller: controller, onSelection: stopRecording) { button in
                        actionMenu(button)
                    }
                } else {
                    DFSection {
                        ForEach(buttons, id: \.self) { button in
                            if button != buttons.first { Divider() }
                            mappingRow(button)
                        }
                    }
                    Text("支持左键、右键、中键及前进、后退键。侧键需鼠标具备对应按键并发送标准鼠标事件。")
                        .font(DFType.caption).foregroundStyle(.secondary)
                }
                if source == 0 {
                    DisclosureGroup("组合键设置") {
                        HStack {
                            Text("按住这些键时使用独立映射").foregroundStyle(.secondary)
                            Spacer()
                            modifier("⌘", 0x100000); modifier("⇧", 0x20000)
                            modifier("⌥", 0x80000); modifier("⌃", 0x40000)
                        }.padding(.vertical, 8)
                        Text("未设置的组合沿用默认映射。").font(DFType.caption).foregroundStyle(.secondary)
                    }.font(DFType.detail)
                }
            }

            DFSection {
                DisclosureGroup("设备状态与使用说明") {
                    VStack(alignment: .leading, spacing: 12) {
                        Text(controller.hidStatus)
                        Text("G502 的原有 DPI、配置切换或 G HUB 动作会继续执行。实体键编号按实际识别为准；不要同时绑定同一按键的通用事件和实体事件。机械滚轮解锁键无法识别。")
                        Text("切换桌面需要对应方向存在桌面，且系统键盘快捷键已启用。MX 与 G502 原生按键暂不支持修饰键分层。")
                        Button("打开输入监控设置") { controller.openInputMonitoringSettings() }
                    }.font(DFType.caption).foregroundStyle(.secondary).padding(.top, 10)
                }
                Divider().padding(.vertical, 4)
                DisclosureGroup("Mac 间自动同步") {
                    Text("配置自动同步到已配对、已连接且支持此功能的 Mac。离线修改在重连后按逻辑版本同步整套配置，不逐个合并按键；权限需在每台 Mac 单独授予。Windows 不参与。")
                        .font(DFType.caption).foregroundStyle(.secondary).padding(.top, 8)
                }
            }
            HStack {
                Label("更改自动保存", systemImage: "checkmark.circle").foregroundStyle(.secondary)
                Spacer()
                Button("恢复默认…", role: .destructive) { reset = true }.buttonStyle(.borderless)
            }.font(DFType.caption).padding(.horizontal, 4)
        }
        .disabled(engine == nil)
        .onChange(of: source) { _, _ in stopRecording(); modifiers = 0 }
        .onDisappear { stopRecording(); controller.endDetection() }
        .confirmationDialog("恢复默认并同步到其他 Mac？", isPresented: $reset) {
            Button("恢复默认", role: .destructive) { controller.update(MouseSettings()) }
        }
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
        if source == 1 {
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
