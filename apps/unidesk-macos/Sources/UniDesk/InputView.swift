import SwiftUI
import AppKit
import UniDeskEngine

struct InputView: View {
    @Bindable var store: AppStore
    let snapshot: EngineSnapshot
    @State private var showSettings = false

    var body: some View {
        GeometryReader { geometry in
            ScrollView {
                if snapshot.input.canConfigure {
                    VStack(alignment: .leading, spacing: 22) {
                        HStack(alignment: .top, spacing: 16) {
                            VStack(alignment: .leading, spacing: 10) {
                                Text("让操作自然跨过屏幕").font(.system(size: 26, weight: .bold))
                                Text(snapshot.input.summary).font(.callout).foregroundStyle(.secondary)
                            }
                            Spacer(minLength: 0)
                            sharingControl
                        }
                        if let error = snapshot.input.error { Label(error, systemImage: "exclamationmark.triangle").foregroundStyle(.orange) }
                        if snapshot.input.locked { Label("鼠标已锁定在当前设备", systemImage: "lock").foregroundStyle(.orange) }
                        if geometry.size.width >= 920 {
                            HStack(alignment: .top, spacing: 20) {
                                layout(height: max(360, geometry.size.height - 190))
                                inspector(minHeight: max(360, geometry.size.height - 190)).frame(width: 250)
                            }
                        } else {
                            layout(height: max(330, geometry.size.height - 210))
                            inspector(minHeight: 0)
                        }
                    }.padding(28).frame(maxWidth: .infinity, alignment: .topLeading)
                } else {
                    receiverState
                        .padding(28)
                        .frame(maxWidth: .infinity, minHeight: max(0, geometry.size.height - 56))
                }
            }.background(Color(nsColor: .textBackgroundColor))
        }
        .toolbar {
            if snapshot.input.canConfigure {
                Button("共享设置", systemImage: "slider.horizontal.3") { showSettings = true }
            }
        }
        .sheet(isPresented: $showSettings) {
            if snapshot.input.canConfigure {
                InputSettingsEditor(store: store, initial: snapshot.input.settings).frame(width: 640, height: 610)
            }
        }
        .onChange(of: snapshot.input.canConfigure) { _, allowed in
            if !allowed { showSettings = false }
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

    private var sharingControl: some View {
        Toggle("共享键鼠", isOn: Binding(get: { snapshot.input.settings.enabled }, set: setControllerEnabled))
            .toggleStyle(.switch).fixedSize().padding(12)
            .modifier(NativeGlassSurface(cornerRadius: 16))
    }

    private func setControllerEnabled(_ enabled: Bool) {
        var settings = snapshot.input.settings
        settings.enabled = enabled
        settings.controller = true
        store.send(.setInput(settings: settings))
    }

    private func layout(height: CGFloat) -> some View {
        VStack(alignment: .leading, spacing: 14) {
            DisplayLayout(store: store, snapshot: snapshot).frame(height: height)
            ForEach(Array(snapshot.input.displays.sorted { $0.layoutID < $1.layoutID }.enumerated()), id: \.element.layoutID) { index, display in
                HStack(alignment: .top, spacing: 8) {
                    Text(String(format: "%02d", index + 1)).monospacedDigit().foregroundStyle(DisplayPalette.color(for: display.deviceId, displays: snapshot.input.displays))
                    Text("\(display.name) · \(display.layoutSizeLabel)\(display.local ? " · 本机" : "")")
                    Spacer(minLength: 0)
                    Text(display.ready ? "已就绪" : "未就绪").foregroundStyle(.secondary)
                }.font(.caption)
            }
            Text("拖动自动吸附，松手贴合相邻屏幕；同色屏幕属于同一设备并整体移动。尺寸单位为 pt，调整位置不会开启或关闭共享。")
                .font(.caption).foregroundStyle(.secondary)
        }.frame(maxWidth: .infinity)
    }

    private func inspector(minHeight: CGFloat) -> some View {
        VStack(alignment: .leading, spacing: 22) {
            Text("设备可用性").font(.headline)
            ForEach(snapshot.devices.filter { $0.member }, id: \.id) { device in
                VStack(alignment: .leading, spacing: 10) {
                    Text(device.name).font(.callout.weight(.semibold))
                    Text(device.inputReason).font(.caption).foregroundStyle(.secondary)
                }.frame(maxWidth: .infinity, alignment: .leading)
                Divider()
            }
            if !snapshot.devices.contains(where: { $0.member }) {
                Text("先在设备页完成配对，再选择要共享的设备。").font(.callout).foregroundStyle(.secondary)
            }
            Text("共享开关只表示你的选择，实际可用性以设备状态为准。")
                .font(.caption).foregroundStyle(.secondary)
            Spacer(minLength: 0)
            VStack(alignment: .leading, spacing: 12) {
                Text("紧急收回").font(.headline)
                Text("随时收回键鼠控制权到这台 Mac。").font(.caption).foregroundStyle(.secondary)
                HStack {
                    Text("⌃⌥⌘ Esc").font(.callout.monospaced())
                    Spacer()
                    Button("现在收回") { store.reclaim() }
                }
            }.padding(16).modifier(NativeGlassSurface(cornerRadius: 16))
        }
        .padding(20).frame(maxWidth: .infinity, minHeight: minHeight, alignment: .topLeading)
        .background(Color(nsColor: .controlBackgroundColor), in: RoundedRectangle(cornerRadius: 16))
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
                Section("主控与参与设备") {
                    Toggle("此设备作为主控", isOn: $draft.controller)
                    ForEach(store.snapshot?.devices.filter { $0.member } ?? [], id: \.id) { device in
                        Toggle(device.name, isOn: Binding(get: { draft.participants.contains(device.id) }, set: { enabled in if enabled { if !draft.participants.contains(device.id) { draft.participants.append(device.id) } } else { draft.participants.removeAll { $0 == device.id } } }))
                    }
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
