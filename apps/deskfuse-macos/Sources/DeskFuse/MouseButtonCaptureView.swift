import SwiftUI

/// Captures observed Quartz buttons or supported HID++ control IDs.
/// Vendor controls remain semantic IDs, never guessed Quartz button numbers.
struct MouseButtonCaptureView: View {
    let controller: MouseButtonController
    let modifiers: UInt64
    let initialG502: Bool
    let onApplied: (Int) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var attempted = false
    @State private var selection = 0
    @State private var applied = false
    @State private var g502 = false
    @State private var deadline = Date()
    private let actions: [(String, MouseAction)] = [
        ("后退", .shortcut(33, 0x100000)),
        ("前进", .shortcut(30, 0x100000)),
        ("向左切换桌面", .shortcut(123, 0x40000)),
        ("向右切换桌面", .shortcut(124, 0x40000)),
        ("Mission Control", .shortcut(126, 0x40000)),
        ("应用窗口", .shortcut(125, 0x40000)),
        ("复制", .shortcut(8, 0x100000)),
        ("粘贴", .shortcut(9, 0x100000)),
        ("中键点击", .mouse(2)),
        ("停用此按键", .disabled)
    ]
    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            HStack(alignment: .top) {
                VStack(alignment: .leading, spacing: 6) {
                    Text("添加按键动作").font(.system(size: 20, weight: .semibold))
                    Text("按一下鼠标，告诉 DeskFuse 你想设置哪个键。")
                        .font(.callout).foregroundStyle(.secondary)
                }
                Spacer()
                Button { dismiss() } label: { Image(systemName: "xmark") }
                    .buttonStyle(.borderless).keyboardShortcut(.cancelAction)
                    .accessibilityLabel("关闭按键设置")
            }
            HStack(spacing: 8) {
                step("1", "识别按键", complete: controller.lastButton != nil)
                Rectangle().fill(.quaternary).frame(height: 1)
                step("2", "选择动作", complete: applied)
            }
            Picker("采集来源", selection: $g502) {
                Text("通用 / MX 按键").tag(false)
                Text("G502 实体键").tag(true)
            }.pickerStyle(.segmented).disabled(controller.detecting)

            VStack(spacing: 12) {
                Image(systemName: controller.lastButton == nil ? "computermouse" : "checkmark.circle.fill")
                    .font(.system(size: 34, weight: .light))
                    .foregroundStyle(controller.lastButton == nil ? Color.accentColor : Color.green)
                Text(status).font(.system(size: 16, weight: .semibold))
                    .accessibilityAddTraits(.updatesFrequently)
                Text(detail).font(.callout).foregroundStyle(.secondary)
                    .multilineTextAlignment(.center).frame(height: 38)
                if controller.detecting {
                    TimelineView(.periodic(from: .now, by: 1)) { context in
                        let remaining = max(0, Int(ceil(deadline.timeIntervalSince(context.date))))
                        HStack(spacing: 12) {
                            ProgressView().controlSize(.small)
                            Text("正在监听 · 剩余 \(remaining) 秒").monospacedDigit()
                            Button("取消") { controller.endDetection() }.buttonStyle(.borderless)
                        }.frame(height: 36)
                    }
                } else {
                    Button(controller.lastButton == nil ? "开始识别" : "重新识别") {
                        attempted = true; applied = false
                        deadline = Date().addingTimeInterval(10)
                        controller.beginDetection(g502: g502)
                    }.buttonStyle(.borderedProminent).frame(height: 36)
                }
            }
            .frame(maxWidth: .infinity).padding(.vertical, 22).padding(.horizontal, 16)
            .background(Color.accentColor.opacity(0.045), in: RoundedRectangle(cornerRadius: 12))
            .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(Color.accentColor.opacity(0.12)))

            VStack(alignment: .leading, spacing: 10) {
                HStack {
                    Text("按键动作").font(.headline)
                    Spacer()
                    Picker("按键动作", selection: $selection) {
                        ForEach(actions.indices.filter {
                            (!LogitechHIDProtocol.buttons.contains(controller.lastButton ?? -1) || actions[$0].1.kind != "mouse")
                                && (!g502 || actions[$0].1.kind != "disabled" && actions[$0].1.kind != "mouse")
                        }, id: \.self) { index in
                            Text(actions[index].0).tag(index)
                        }
                    }.labelsHidden().frame(width: 220)
                        .disabled(controller.lastButton == nil || !controller.captureReleased)
                }
                Text(g502 ? "只附加快捷键，保留鼠标原有 DPI 和配置功能。" : "识别完成后选择动作；更多动作可在按键列表中修改。")
                    .font(.caption).foregroundStyle(.secondary)
            }
            if let error = controller.error {
                Label(error, systemImage: "exclamationmark.circle").font(.caption).foregroundStyle(.red)
            }
            DisclosureGroup("没有识别到按键？") {
                VStack(alignment: .leading, spacing: 8) {
                    Text(controller.hidStatus)
                    Text(g502 ? "仅支持 G502 LIGHTSPEED。实体键编号按实际识别为准，机械滚轮解锁键不产生事件。" : "支持系统鼠标按钮，以及 MX Master 3 / 3S 的手势键与模式键。首次发现设备可能需要数秒，请唤醒鼠标后重试。")
                    Button("检查输入监控权限") { controller.openInputMonitoringSettings() }
                }.font(.caption).foregroundStyle(.secondary).padding(.top, 6)
            }.font(.caption)
            Divider()
            HStack {
                if applied {
                    Label("已保存并启用", systemImage: "checkmark.circle.fill").foregroundStyle(.green)
                } else {
                    Text("识别完成并松开按键后即可保存").foregroundStyle(.secondary)
                }
                Spacer()
                if applied {
                    Button("完成") { dismiss() }.buttonStyle(.borderedProminent)
                } else {
                    Button("保存并启用") { apply() }.buttonStyle(.borderedProminent)
                        .disabled(controller.lastButton == nil || !controller.captureReleased)
                }
            }.font(.callout)
        }
        .padding(24).frame(width: 540)
        .onAppear { g502 = initialG502; controller.clearCapture(); attempted = false }
        .onDisappear { controller.endDetection() }
        .onChange(of: controller.lastButton) { _, _ in selection = 0 }
        .onChange(of: selection) { _, _ in applied = false }
        .onChange(of: g502) { _, _ in controller.clearCapture(); attempted = false; applied = false; selection = 0 }
    }
    private func step(_ number: String, _ title: String, complete: Bool) -> some View {
        HStack(spacing: 8) {
            Text(complete ? "✓" : number).font(.system(size: 11, weight: .semibold))
                .frame(width: 22, height: 22)
                .background(complete ? Color.green.opacity(0.12) : Color.primary.opacity(0.06), in: Circle())
            Text(title).font(.system(size: 12, weight: .medium))
        }.fixedSize()
    }
    private var status: String {
        if controller.detecting { return "现在，请按下要设置的鼠标键" }
        if let button = controller.lastButton { return "已采集：\(MouseSettings.buttonName(button))" }
        return attempted ? "未采集到按键事件" : "等待开始采集"
    }
    private var detail: String {
        if attempted && !controller.tapReady && controller.lastButton == nil {
            return "普通按键监听尚未就绪，请检查辅助功能授权；HID++ 接口状态见下方。"
        }
        if controller.detecting { return g502 ? "请按目标实体键；原有鼠标功能仍会执行。也可按 Esc 退出。" : "本次采集会拦截下一次鼠标点击；也可按 Esc 退出。" }
        if let button = controller.lastButton {
            let source = LogitechHIDProtocol.g502Buttons.contains(button) ? "G502 实体编号 \(button - 207)"
                : LogitechHIDProtocol.mxButtons.contains(button) ? String(format: "HID++ CID 0x%04X", button) : "系统编号 \(button)"
            return "\(source) · \(controller.captureReleased ? "按下与松开均已收到" : "等待松开按键")"
        }
        return "点击开始后，再按目标鼠标按钮。超时后可重新采集。"
    }
    private func apply() {
        guard let button = controller.lastButton, controller.captureReleased else { return }
        guard !LogitechHIDProtocol.buttons.contains(button) || actions[selection].1.kind != "mouse" else { return }
        guard !LogitechHIDProtocol.g502Buttons.contains(button) || actions[selection].1.kind == "shortcut" else { return }
        var settings = controller.settings
        settings.set(button: button, modifiers: LogitechHIDProtocol.buttons.contains(button) ? 0 : modifiers, action: actions[selection].1)
        settings.enabled = true
        controller.update(settings)
        onApplied(button)
        applied = true
    }
}
