import SwiftUI
import DeskFuseEngine

/// Membership comes from the engine. Adding selects a participant, never fabricates screens.
struct InputDevicePicker: View {
    let snapshot: EngineSnapshot
    let adding: Bool
    let error: String?
    let onAdd: (String) -> Void
    let onRemove: (String) -> Void
    let onOpenDevices: () -> Void
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            HStack {
                Text("添加设备屏幕").font(DFType.pageTitle.weight(.bold))
                Spacer()
                Button("完成") { dismiss() }.keyboardShortcut(.defaultAction)
            }
            Text("选择已配对的电脑，添加它的全部屏幕。屏幕尺寸由对端提供，连接就绪后才能跨屏操作。")
                .font(DFType.detail).foregroundStyle(.secondary)
            if let error {
                Label(error, systemImage: "exclamationmark.triangle")
                    .font(DFType.detail).foregroundStyle(DFPalette.warning)
            }
            ScrollView {
                VStack(spacing: 0) {
                    let candidates = snapshot.devices.filter { $0.member && !$0.blocked && $0.id != snapshot.localId }
                    if candidates.isEmpty {
                        ContentUnavailableView("还没有可添加的设备", systemImage: "display",
                                               description: Text("先到设备页配对另一台电脑，再回来添加屏幕。"))
                    }
                    ForEach(candidates, id: \.id) { device in
                        HStack(spacing: 12) {
                            Image(systemName: "display").font(DFType.icon(.medium)).foregroundStyle(.secondary)
                            VStack(alignment: .leading, spacing: 5) {
                                Text(device.name).font(DFType.detail.weight(.semibold))
                                Text(device.inputReason).font(DFType.caption).foregroundStyle(.secondary)
                            }
                            Spacer(minLength: 12)
                            if snapshot.input.settings.participants.contains(device.id) {
                                Button("移除") { onRemove(device.id) }.disabled(adding)
                            } else {
                                Button("添加") { onAdd(device.id) }.disabled(adding)
                            }
                        }.padding(.vertical, 14)
                        Divider()
                    }
                }.frame(maxWidth: .infinity, alignment: .leading)
            }
            HStack {
                Button("前往设备页配对", systemImage: "desktopcomputer") { onOpenDevices() }
                Spacer()
                if adding { ProgressView().controlSize(.small) }
            }
        }.padding(24).frame(width: 500, height: 420)
    }
}
